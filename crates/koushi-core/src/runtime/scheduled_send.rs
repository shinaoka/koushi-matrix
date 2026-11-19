use std::time::Duration;

use koushi_state::{
    AppAction, AppState, ScheduledSendHandle, ScheduledSendItem, ScheduledSendStore, SessionState,
    reduce,
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
        // #1737: an outbox write is a send, so a read-only session arms no
        // dispatch timer at all (a due item that cannot dispatch would otherwise
        // re-arm a zero delay and spin). The main loop re-observes this on the
        // next state change, which is also when admission can open.
        if !self.state.conversation_access().send_admitted {
            return None;
        }
        // #1973: a server-handle item is woken for a delivery check, a grace
        // period after its due time; it is never sent from here.
        let next_server_check_at_ms = self
            .state
            .scheduled_sends
            .next_server_send_at_ms()
            .map(|send_at_ms| send_at_ms.saturating_add(server_check_grace_ms()));
        let next_wake_at_ms = [
            self.state.scheduled_sends.next_local_send_at_ms(),
            next_server_check_at_ms,
        ]
        .into_iter()
        .flatten()
        .min()?;
        let now_ms = crate::time::current_epoch_ms();
        Some(Duration::from_millis(
            next_wake_at_ms.saturating_sub(now_ms),
        ))
    }

    pub(super) async fn dispatch_due_scheduled_send(&mut self) -> bool {
        if !matches!(self.state.session, SessionState::Ready(_)) {
            return false;
        }
        // #1737: a due scheduled send is an ordinary room send, so it waits for
        // the same Secure Backup admission as the composer. Nothing is released
        // from the outbox while the restored session is read-only.
        if !self.state.conversation_access().send_admitted {
            return false;
        }
        let now_ms = crate::time::current_epoch_ms();
        if let Some(item) = self.state.scheduled_sends.next_local_due(now_ms) {
            return self.dispatch_scheduled_send(item).await;
        }
        let Some(item) = self
            .state
            .scheduled_sends
            .next_server_check_due(now_ms.saturating_sub(server_check_grace_ms()))
        else {
            return false;
        };
        self.check_server_scheduled_send(item).await
    }

    /// #1973: ask the account actor whether the homeserver has sent a due
    /// server-handle item. The local timer never sends it.
    async fn check_server_scheduled_send(&mut self, item: ScheduledSendItem) -> bool {
        let ScheduledSendHandle::Server { delay_id } = item.handle else {
            return false;
        };
        let Some(origin_session_key) = scheduled_send_session_key(&self.state) else {
            return false;
        };
        let scheduled_id = item.scheduled_id;
        let effects = self
            .reduce_app_action(AppAction::ScheduledSendServerCheckStarted {
                scheduled_id: scheduled_id.clone(),
                delay_id: delay_id.clone(),
            })
            .await;
        self.handle_ui_event_effects(&effects).await;

        if !self
            .account_actor
            .send(AccountMessage::CheckServerScheduledSend {
                origin_session_key,
                scheduled_id: scheduled_id.clone(),
                delay_id: delay_id.clone(),
            })
            .await
        {
            let defer_effects = self
                .reduce_app_action(AppAction::ScheduledSendServerCheckDeferred {
                    scheduled_id,
                    delay_id,
                    recheck_at_ms: crate::scheduled_send::server_scheduled_send_recheck_at_ms(),
                })
                .await;
            self.handle_ui_event_effects(&defer_effects).await;
        }
        true
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
                // #1348: a local fallback dispatch is an ordinary room send, so it
                // uses the composer formatting in force at dispatch time.
                formatting_options: self.state.settings.values.composer.formatting_options(),
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

fn server_check_grace_ms() -> u64 {
    crate::scheduled_send::SERVER_SCHEDULED_SEND_CHECK_GRACE.as_millis() as u64
}

pub(super) fn scheduled_send_session_key(
    state: &AppState,
) -> Option<koushi_protocol::SessionKeyId> {
    composer_draft_session_key(state)
}
