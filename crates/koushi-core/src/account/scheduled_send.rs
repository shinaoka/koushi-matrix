//! `scheduled_send` ownership for AccountActor.

use koushi_protocol::SessionKeyId;
use koushi_sdk::MatrixClientSession;
use koushi_state::{
    AppAction, ComposerDraftRevision, ScheduledSendCapability, ScheduledSendHandle,
    ScheduledSendItem,
};
use tokio::sync::mpsc;

use crate::composer_draft_lifecycle::ForwardedComposerDraftPermit;
use crate::timeline::composer::build_room_message_content_from_composer_body;
use koushi_protocol::failure::{CoreFailure, TimelineFailureKind};
use koushi_protocol::ids::RequestId;

use super::actor::AccountActor;

fn scheduled_dispatch_targets_active_session(
    active_session_key: Option<&SessionKeyId>,
    origin_session_key: &SessionKeyId,
) -> bool {
    active_session_key == Some(origin_session_key)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AuthoritativeRoomEncryption {
    Unknown,
    Unencrypted,
    Encrypted,
}

fn user_content_is_admitted(encryption: AuthoritativeRoomEncryption) -> bool {
    match encryption {
        AuthoritativeRoomEncryption::Unknown => false,
        AuthoritativeRoomEncryption::Unencrypted => true,
        AuthoritativeRoomEncryption::Encrypted => true,
    }
}

fn server_delayed_events_are_safe(encryption: AuthoritativeRoomEncryption) -> bool {
    encryption == AuthoritativeRoomEncryption::Unencrypted
}

pub(super) async fn admit_secure_backup_user_content(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<(matrix_sdk::Room, AuthoritativeRoomEncryption), TimelineFailureKind> {
    let room_id = matrix_sdk::ruma::RoomId::parse(room_id)
        .map_err(|_| TimelineFailureKind::SecureBackupRequired)?;
    let room = session
        .client()
        .get_room(&room_id)
        .ok_or(TimelineFailureKind::SecureBackupRequired)?;
    let encryption = match room.latest_encryption_state().await {
        Ok(state) if state.is_encrypted() => AuthoritativeRoomEncryption::Encrypted,
        Ok(_) => AuthoritativeRoomEncryption::Unencrypted,
        Err(_) => AuthoritativeRoomEncryption::Unknown,
    };
    user_content_is_admitted(encryption)
        .then_some((room, encryption))
        .ok_or(TimelineFailureKind::SecureBackupRequired)
}

fn build_scheduled_message_content(
    body: &str,
    thread_root_event_id: Option<&str>,
) -> Result<matrix_sdk::ruma::events::room::message::RoomMessageEventContent, TimelineFailureKind> {
    use matrix_sdk::ruma::events::{relation::Thread, room::message::Relation};

    let mut content = build_room_message_content_from_composer_body(
        body,
        koushi_state::MentionIntent::default(),
    )?;
    if let Some(root_event_id) = thread_root_event_id {
        let root_event_id = matrix_sdk::ruma::EventId::parse(root_event_id)
            .map_err(|_| TimelineFailureKind::Sdk)?;
        content.relates_to = Some(Relation::Thread(Thread::plain(
            root_event_id.clone(),
            root_event_id,
        )));
    }
    Ok(content)
}

async fn send_scheduled_acceptance_actions(
    action_tx: &mpsc::Sender<Vec<AppAction>>,
    actions: Vec<AppAction>,
    mut composer_permit: ForwardedComposerDraftPermit,
) {
    composer_permit.acceptance_projection_reached();
    if action_tx.send(actions).await.is_ok() {
        composer_permit.acceptance_enqueued();
    }
}

impl AccountActor {
    pub(super) async fn handle_schedule_server_delayed_send(
        &self,
        request_id: RequestId,
        expected_account: SessionKeyId,
        scheduled_id: String,
        room_id: String,
        thread_root_event_id: Option<String>,
        body: String,
        send_at_ms: u64,
        draft_revision: ComposerDraftRevision,
        composer_permit: ForwardedComposerDraftPermit,
    ) {
        if self.session_key_id.as_ref() != Some(&expected_account) {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        }
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        let (_, encryption) = match admit_secure_backup_user_content(session, &room_id).await {
            Ok(admitted) => admitted,
            Err(kind) => {
                self.emit_failure(request_id, CoreFailure::TimelineOperationFailed { kind });
                return;
            }
        };

        let capability = crate::scheduled_send::detect_capability(&session.client()).await;
        // The delayed-event endpoint accepts event content directly; a homeserver
        // cannot perform client-side room encryption. Encrypted rooms therefore
        // always use the local scheduler, whose eventual Room::send crosses the
        // exact-session secure-backup fence.
        if server_delayed_events_are_safe(encryption)
            && capability == ScheduledSendCapability::ServerDelayedEvents
        {
            match self
                .send_server_delayed_message(
                    session,
                    &room_id,
                    thread_root_event_id.as_deref(),
                    &body,
                    send_at_ms,
                )
                .await
            {
                Ok(delay_id) => {
                    send_scheduled_acceptance_actions(
                        &self.action_tx,
                        vec![
                            AppAction::ScheduledSendCapabilityChanged {
                                capability: ScheduledSendCapability::ServerDelayedEvents,
                            },
                            AppAction::ScheduledSendCreatedAtRevision {
                                item: ScheduledSendItem {
                                    scheduled_id,
                                    room_id,
                                    thread_root_event_id,
                                    body,
                                    send_at_ms,
                                    handle: ScheduledSendHandle::Server { delay_id },
                                    is_dispatching: false,
                                },
                                draft_revision,
                            },
                        ],
                        composer_permit,
                    )
                    .await;
                    return;
                }
                Err(()) => {}
            }
        }

        send_scheduled_acceptance_actions(
            &self.action_tx,
            vec![
                AppAction::ScheduledSendCapabilityChanged {
                    capability: ScheduledSendCapability::LocalFallback,
                },
                AppAction::ScheduledSendCreatedAtRevision {
                    item: ScheduledSendItem {
                        scheduled_id,
                        room_id,
                        thread_root_event_id,
                        body,
                        send_at_ms,
                        handle: ScheduledSendHandle::Local,
                        is_dispatching: false,
                    },
                    draft_revision,
                },
            ],
            composer_permit,
        )
        .await;
    }

    pub(super) async fn handle_dispatch_local_scheduled_send(
        &self,
        request_id: RequestId,
        origin_session_key: SessionKeyId,
        scheduled_id: String,
        room_id: String,
        thread_root_event_id: Option<String>,
        body: String,
    ) {
        let retry_at_ms = crate::scheduled_send::local_scheduled_send_retry_at_ms();
        if !scheduled_dispatch_targets_active_session(
            self.session_key_id.as_ref(),
            &origin_session_key,
        ) {
            self.retry_local_scheduled_send(scheduled_id, retry_at_ms)
                .await;
            return;
        }
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            self.retry_local_scheduled_send(scheduled_id, retry_at_ms)
                .await;
            return;
        };
        let room = match admit_secure_backup_user_content(session, &room_id).await {
            Ok((room, _)) => room,
            Err(_) => {
                self.retry_local_scheduled_send(scheduled_id, retry_at_ms)
                    .await;
                return;
            }
        };
        let content = match build_scheduled_message_content(&body, thread_root_event_id.as_deref())
        {
            Ok(content) => content,
            Err(kind) => {
                self.emit_failure(request_id, CoreFailure::TimelineOperationFailed { kind });
                self.retry_local_scheduled_send(scheduled_id, retry_at_ms)
                    .await;
                return;
            }
        };
        let transaction_id = matrix_sdk::ruma::OwnedTransactionId::from(
            crate::scheduled_send::scheduled_send_transaction_id(&scheduled_id),
        );
        match room.send(content).with_transaction_id(transaction_id).await {
            Ok(_) => {
                self.send_actions(vec![AppAction::ScheduledSendDispatched { scheduled_id }])
                    .await;
            }
            Err(_) => {
                self.emit_failure(
                    request_id,
                    CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::Sdk,
                    },
                );
                self.retry_local_scheduled_send(scheduled_id, retry_at_ms)
                    .await;
            }
        }
    }

    async fn retry_local_scheduled_send(&self, scheduled_id: String, retry_at_ms: u64) {
        self.send_actions(vec![AppAction::ScheduledSendDispatchFailed {
            scheduled_id,
            retry_at_ms,
        }])
        .await;
    }

    pub(super) async fn handle_cancel_server_delayed_send(
        &self,
        request_id: RequestId,
        scheduled_id: String,
        delay_id: String,
    ) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        match self
            .update_server_delayed_event(
                session,
                delay_id,
                matrix_sdk::ruma::api::client::delayed_events::update_delayed_event::UpdateAction::Cancel,
            )
            .await
        {
            Ok(()) => {
                self.send_actions(vec![AppAction::ScheduledSendCancelled { scheduled_id }])
                    .await;
            }
            Err(()) => self.emit_failure(
                request_id,
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::Sdk,
                },
            ),
        }
    }

    pub(super) async fn handle_reschedule_server_delayed_send(
        &self,
        request_id: RequestId,
        scheduled_id: String,
        room_id: String,
        thread_root_event_id: Option<String>,
        body: String,
        delay_id: String,
        send_at_ms: u64,
    ) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        let (_, encryption) = match admit_secure_backup_user_content(session, &room_id).await {
            Ok(admitted) => admitted,
            Err(kind) => {
                self.emit_failure(request_id, CoreFailure::TimelineOperationFailed { kind });
                return;
            }
        };

        if self
            .update_server_delayed_event(
                session,
                delay_id,
                matrix_sdk::ruma::api::client::delayed_events::update_delayed_event::UpdateAction::Cancel,
            )
            .await
            .is_err()
        {
            self.emit_failure(
                request_id,
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::Sdk,
                },
            );
            return;
        }

        if encryption == AuthoritativeRoomEncryption::Encrypted {
            self.send_actions(vec![
                AppAction::ScheduledSendCapabilityChanged {
                    capability: ScheduledSendCapability::LocalFallback,
                },
                AppAction::ScheduledSendRescheduled {
                    scheduled_id,
                    body,
                    send_at_ms,
                    handle: ScheduledSendHandle::Local,
                },
            ])
            .await;
            return;
        }

        match self
            .send_server_delayed_message(
                session,
                &room_id,
                thread_root_event_id.as_deref(),
                &body,
                send_at_ms,
            )
            .await
        {
            Ok(delay_id) => {
                self.send_actions(vec![AppAction::ScheduledSendRescheduled {
                    scheduled_id,
                    body,
                    send_at_ms,
                    handle: ScheduledSendHandle::Server { delay_id },
                }])
                .await;
            }
            Err(()) => {
                self.send_actions(vec![
                    AppAction::ScheduledSendCapabilityChanged {
                        capability: ScheduledSendCapability::LocalFallback,
                    },
                    AppAction::ScheduledSendRescheduled {
                        scheduled_id,
                        body,
                        send_at_ms,
                        handle: ScheduledSendHandle::Local,
                    },
                ])
                .await;
            }
        }
    }

    async fn send_server_delayed_message(
        &self,
        session: &MatrixClientSession,
        room_id: &str,
        thread_root_event_id: Option<&str>,
        body: &str,
        send_at_ms: u64,
    ) -> Result<String, ()> {
        use matrix_sdk::ruma::TransactionId;
        use matrix_sdk::ruma::api::client::delayed_events::{
            DelayParameters, delayed_message_event,
        };

        let room_id = matrix_sdk::ruma::RoomId::parse(room_id).map_err(|_| ())?;
        let content =
            build_scheduled_message_content(body, thread_root_event_id).map_err(|_| ())?;
        let request = delayed_message_event::unstable::Request::new(
            room_id,
            TransactionId::new(),
            DelayParameters::Timeout {
                timeout: crate::scheduled_send::server_delay_timeout(
                    send_at_ms,
                    crate::time::current_epoch_ms(),
                ),
            },
            &content,
        )
        .map_err(|_| ())?;

        session
            .client()
            .send(request)
            .await
            .map(|response| response.delay_id)
            .map_err(|_| ())
    }

    async fn update_server_delayed_event(
        &self,
        session: &MatrixClientSession,
        delay_id: String,
        action: matrix_sdk::ruma::api::client::delayed_events::update_delayed_event::UpdateAction,
    ) -> Result<(), ()> {
        let request =
            matrix_sdk::ruma::api::client::delayed_events::update_delayed_event::unstable_v1::Request::new(
                delay_id, action,
            );
        session
            .client()
            .send(request)
            .await
            .map(|_| ())
            .map_err(|_| ())
    }
}

#[cfg(test)]
mod tests;
