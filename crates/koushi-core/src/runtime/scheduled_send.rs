use std::time::Duration;

use koushi_state::{
    AppAction, AppState, ScheduledSendItem, ScheduledSendStore, SessionState, reduce,
};

use super::composer_draft_session_key;

use crate::account::AccountMessage;
use crate::executor;
use koushi_protocol::event::CoreEvent;
use koushi_protocol::failure::CoreFailure;

pub(super) enum DeferredScheduledSendPersist {
    ClearLoadedMarker,
    Persist {
        key_id: koushi_protocol::SessionKeyId,
        scheduled_sends: ScheduledSendStore,
    },
}

pub(super) fn scheduled_send_id() -> String {
    format!("scheduled-{}", matrix_sdk::ruma::TransactionId::new())
}

impl super::AppActor {
    pub(super) async fn load_scheduled_sends_for_current_session(&mut self) {
        let Some(key_id) = scheduled_send_session_key(&self.state) else {
            self.scheduled_sends_loaded_for = None;
            return;
        };
        if self.scheduled_sends_loaded_for.as_ref() == Some(&key_id) {
            return;
        }

        let store = self.composer_draft_store_actor.clone();
        let load_key_id = key_id.clone();
        let scheduled_sends = executor::spawn_blocking(move || {
            store.load_scheduled_sends(&load_key_id).unwrap_or_default()
        })
        .await
        .unwrap_or_default();
        let effects = reduce(
            &mut self.state,
            AppAction::ScheduledSendsLoaded { scheduled_sends },
        );
        self.scheduled_sends_loaded_for = Some(key_id);
        self.handle_ui_event_effects(&effects).await;
    }

    pub(super) async fn persist_scheduled_sends(
        &mut self,
        key_id: koushi_protocol::SessionKeyId,
        scheduled_sends: ScheduledSendStore,
    ) {
        let store = self.composer_draft_store_actor.clone();
        let save_key_id = key_id.clone();
        let saved = executor::spawn_blocking(move || {
            store.save_scheduled_sends(&save_key_id, &scheduled_sends)
        })
        .await;
        if scheduled_send_session_key(&self.state).as_ref() != Some(&key_id) {
            // The account changed while the write was in flight. This outcome
            // belongs to a retired session and must not set or clear the
            // replacement account's user-visible state (#1159).
            return;
        }
        // #1159: acceptance is not durability. A dropped result left an
        // unpersisted local reservation looking saved, so the outcome is now
        // reduced into user-visible state. The store error and the account key
        // are never surfaced (no identifiers, no paths).
        let action = match saved {
            Ok(Ok(())) => AppAction::ScheduledSendPersisted,
            Err(_) | Ok(Err(_)) => AppAction::ScheduledSendPersistenceFailed {
                message: "scheduled sends could not be saved on this device".to_owned(),
            },
        };
        let effects = reduce(&mut self.state, action);
        self.handle_ui_event_effects(&effects).await;
    }

    pub(super) fn scheduled_send_delay(&self) -> Option<Duration> {
        if !matches!(self.state.session, SessionState::Ready(_)) {
            return None;
        }
        let next_send_at_ms = self.state.scheduled_sends.next_local_send_at_ms()?;
        let now_ms = crate::time::current_epoch_ms();
        Some(Duration::from_millis(
            next_send_at_ms.saturating_sub(now_ms),
        ))
    }

    pub(super) async fn dispatch_due_scheduled_send(&mut self) -> bool {
        if !matches!(self.state.session, SessionState::Ready(_)) {
            return false;
        }
        let Some(item) = self
            .state
            .scheduled_sends
            .next_local_due(crate::time::current_epoch_ms())
        else {
            return false;
        };
        self.dispatch_scheduled_send(item).await
    }

    async fn dispatch_scheduled_send(&mut self, item: ScheduledSendItem) -> bool {
        let Some(origin_session_key) = scheduled_send_session_key(&self.state) else {
            return false;
        };
        let scheduled_id = item.scheduled_id.clone();
        let effects = self
            .reduce_app_action(AppAction::ScheduledSendDispatchStarted {
                scheduled_id: scheduled_id.clone(),
            })
            .await;
        self.handle_ui_event_effects(&effects).await;

        let request_id = self.next_internal_request_id();
        if !self
            .account_actor
            .send(AccountMessage::DispatchLocalScheduledSend {
                request_id,
                origin_session_key,
                scheduled_id: scheduled_id.clone(),
                room_id: item.room_id,
                thread_root_event_id: item.thread_root_event_id,
                body: item.body,
            })
            .await
        {
            self.emit(CoreEvent::OperationFailed {
                request_id,
                failure: CoreFailure::ShutdownFailed,
            });
            let retry_effects = self
                .reduce_app_action(AppAction::ScheduledSendDispatchFailed {
                    scheduled_id,
                    retry_at_ms: crate::scheduled_send::local_scheduled_send_retry_at_ms(),
                })
                .await;
            self.handle_ui_event_effects(&retry_effects).await;
        }
        true
    }
}

pub(super) fn scheduled_send_session_key(
    state: &AppState,
) -> Option<koushi_protocol::SessionKeyId> {
    composer_draft_session_key(state)
}
