//! `routing` ownership for AccountActor.

use std::time::Duration;

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_protocol::SessionKeyId;
use koushi_state::{AppAction, OperationFailureKind};

use crate::command_policy::{
    search_scope_to_state, space_member_forward_failure_action, timeline_composer_account_fence,
};
use crate::composer_draft_lifecycle::ForwardedComposerDraftPermit;
use crate::room::RoomMessage;
use crate::sync::SyncMessage;
use crate::timeline::TimelineMessage;
use koushi_protocol::command::{
    RoomCommand, SearchCommand, SyncCommand, ThreadsListCommand, TimelineCommand,
};
use koushi_protocol::event::{CoreEvent, TimelineEvent};
#[cfg(any(test, feature = "test-hooks"))]
use koushi_protocol::failure::SyncFailureKind;
use koushi_protocol::failure::{CoreFailure, RoomFailureKind, TimelineFailureKind};
use koushi_protocol::ids::{RequestId, TimelineKey, TimelineKind};

use super::RoomEventLookupResult;
use super::actor::{AccountActor, trace_restore};
use super::scheduled_send::admit_secure_backup_user_content;

const SEARCH_UNAVAILABLE_MESSAGE: &str = "search unavailable";
const ROOM_EVENT_CACHE_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
const ROOM_EVENT_CACHE_TEST_TIMEOUT: Duration = Duration::from_millis(25);

fn composer_timeline_command_targets_active_session(
    active_session_key: Option<&SessionKeyId>,
    command: &TimelineCommand,
) -> bool {
    timeline_composer_account_fence(command)
        .is_none_or(|(_, expected_account)| active_session_key == Some(expected_account))
}

fn trace_room_route(stage: &'static str, command: &RoomCommand) {
    match command {
        RoomCommand::CreateRoom { request_id, .. } => {
            trace_room_route_event(stage, "create_room", *request_id);
        }
        RoomCommand::CreateSpace { request_id, .. } => {
            trace_room_route_event(stage, "create_space", *request_id);
        }
        RoomCommand::SetSpaceChild { request_id, .. } => {
            trace_room_route_event(stage, "set_space_child", *request_id);
        }
        RoomCommand::InviteUser { request_id, .. } => {
            trace_room_route_event(stage, "invite_user", *request_id);
        }
        RoomCommand::AcceptInvite { request_id, .. } => {
            trace_room_route_event(stage, "accept_invite", *request_id);
        }
        _ => {}
    }
}

fn trace_room_route_event(stage: &'static str, kind: &'static str, request_id: RequestId) {
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "core.account", stage)
            .field(DiagnosticField::token("operation", kind))
            .field(DiagnosticField::request_id(
                "request_id",
                request_id.connection_id.0,
                request_id.sequence,
            )),
    );
}

fn trace_room_route_closed() {
    record(DiagnosticEvent::new(
        DiagnosticLevel::Debug,
        "core.account",
        "closed",
    ));
}

struct EncryptedUserContentTarget<'a> {
    request_id: RequestId,
    room_id: &'a str,
    submission: Option<(&'a TimelineKey, &'a koushi_state::SubmissionId)>,
}

fn encrypted_user_content_target(
    command: &TimelineCommand,
) -> Option<EncryptedUserContentTarget<'_>> {
    match command {
        TimelineCommand::SendText {
            request_id, key, ..
        }
        | TimelineCommand::SendReply {
            request_id, key, ..
        }
        | TimelineCommand::EditText {
            request_id, key, ..
        }
        | TimelineCommand::RetrySend {
            request_id, key, ..
        }
        | TimelineCommand::UploadAndSendMedia {
            request_id, key, ..
        } => Some(EncryptedUserContentTarget {
            request_id: *request_id,
            room_id: key.room_id(),
            submission: None,
        }),
        TimelineCommand::SubmitText {
            request_id,
            key,
            submission_id,
            ..
        }
        | TimelineCommand::SubmitReply {
            request_id,
            key,
            submission_id,
            ..
        } => Some(EncryptedUserContentTarget {
            request_id: *request_id,
            room_id: key.room_id(),
            submission: Some((key, submission_id)),
        }),
        TimelineCommand::ForwardMessage {
            request_id,
            destination_room_id,
            ..
        } => Some(EncryptedUserContentTarget {
            request_id: *request_id,
            room_id: destination_room_id,
            submission: None,
        }),
        _ => None,
    }
}

impl AccountActor {
    /// Route a RoomCommand to the RoomActor. The RoomActor handles the
    /// SessionRequired check internally (it holds the session ref after
    /// SyncStarted).
    pub(super) async fn route_room_command(&self, command: RoomCommand) {
        trace_room_route("send", &command);
        let forward_failure = space_member_forward_failure_action(&command);
        let sent = self.room_actor.send(RoomMessage::Command(command)).await;
        if !sent {
            trace_room_route_closed();
            if let Some((request_id, failure_action)) = forward_failure {
                let _ = self.send_actions(vec![failure_action]).await;
                self.emit_failure(
                    request_id,
                    CoreFailure::RoomOperationFailed {
                        kind: RoomFailureKind::Sdk,
                    },
                );
            }
        }
    }

    /// Route a TimelineCommand to the TimelineManagerActor.
    /// Composer-affecting sends are revalidated here as a second ordered
    /// account-owner barrier. AppActor's check alone cannot cover a switch
    /// already queued ahead of this message in the AccountActor mailbox.
    pub(super) async fn route_timeline_command(&mut self, command: TimelineCommand) {
        self.route_timeline_command_with_permit(command, None).await;
    }

    pub(super) async fn route_timeline_command_with_formatting_options(
        &mut self,
        command: TimelineCommand,
        formatting_options: koushi_state::ComposerFormattingOptions,
    ) {
        self.route_timeline_command_with_permit_and_formatting_options(
            command,
            None,
            Some(formatting_options),
        )
        .await;
    }

    pub(super) async fn route_leased_timeline_command(
        &mut self,
        command: TimelineCommand,
        composer_permit: ForwardedComposerDraftPermit,
    ) {
        self.route_timeline_command_with_permit(command, Some(composer_permit))
            .await;
    }

    pub(super) async fn route_leased_timeline_command_with_formatting_options(
        &mut self,
        command: TimelineCommand,
        composer_permit: ForwardedComposerDraftPermit,
        formatting_options: koushi_state::ComposerFormattingOptions,
    ) {
        self.route_timeline_command_with_permit_and_formatting_options(
            command,
            Some(composer_permit),
            Some(formatting_options),
        )
        .await;
    }

    async fn route_timeline_command_with_permit(
        &mut self,
        command: TimelineCommand,
        composer_permit: Option<ForwardedComposerDraftPermit>,
    ) {
        self.route_timeline_command_with_permit_and_formatting_options(
            command,
            composer_permit,
            None,
        )
        .await;
    }

    async fn route_timeline_command_with_permit_and_formatting_options(
        &mut self,
        command: TimelineCommand,
        composer_permit: Option<ForwardedComposerDraftPermit>,
        formatting_options: Option<koushi_state::ComposerFormattingOptions>,
    ) {
        if !composer_timeline_command_targets_active_session(self.session_key_id.as_ref(), &command)
            && let Some((request_id, _)) = timeline_composer_account_fence(&command)
        {
            record(
                DiagnosticEvent::new(
                    DiagnosticLevel::Warn,
                    "core.timeline_send_admission",
                    "rejected",
                )
                .field(DiagnosticField::token("reason", "account_mismatch")),
            );
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        }
        if let Some(target) = encrypted_user_content_target(&command) {
            let Some(session) = self.session.as_deref().filter(|_| self.session_promoted) else {
                record(
                    DiagnosticEvent::new(
                        DiagnosticLevel::Warn,
                        "core.timeline_send_admission",
                        "rejected",
                    )
                    .field(DiagnosticField::token("reason", "session_unpromoted")),
                );
                self.emit_failure(target.request_id, CoreFailure::SessionRequired);
                return;
            };
            if let Err(kind) = admit_secure_backup_user_content(session, target.room_id).await {
                if let Some((key, submission_id)) = target.submission {
                    let _ = self.event_tx.send(CoreEvent::Timeline(
                        TimelineEvent::SubmissionRejected {
                            request_id: target.request_id,
                            key: key.clone(),
                            submission_id: submission_id.clone(),
                            kind,
                        },
                    ));
                } else {
                    self.emit_failure(
                        target.request_id,
                        CoreFailure::TimelineOperationFailed { kind },
                    );
                }
                return;
            }
        }
        if let TimelineCommand::BroadcastLinkPreviewPolicy {
            unencrypted_global_enabled,
            encrypted_global_enabled,
            room_overrides,
        } = &command
        {
            self.link_preview_policy.unencrypted_global_enabled = *unencrypted_global_enabled;
            self.link_preview_policy.encrypted_global_enabled = *encrypted_global_enabled;
            self.link_preview_policy.room_overrides = room_overrides.clone();
        }
        let message = match (composer_permit, formatting_options) {
            (Some(composer_permit), Some(formatting_options)) => {
                TimelineMessage::LeasedCommandWithComposerFormatting {
                    command,
                    composer_permit,
                    formatting_options,
                }
            }
            (Some(composer_permit), None) => TimelineMessage::LeasedCommand {
                command,
                composer_permit,
            },
            (None, Some(formatting_options)) => TimelineMessage::CommandWithComposerFormatting {
                command,
                formatting_options,
            },
            (None, None) => TimelineMessage::Command(command),
        };
        let _ = self.timeline_manager.send(message).await;
    }

    pub(super) fn flush_pending_crawler_notification(&mut self) {
        let Some(handle) = &self.search_actor else {
            return;
        };
        let Some((room_ids, settings)) = self.pending_crawler_notification.take() else {
            return;
        };
        if let Err((room_ids, settings)) = handle.try_notify_rooms_available(room_ids, settings) {
            self.pending_crawler_notification = Some((room_ids, settings));
        }
    }

    /// Route a SearchCommand to the SearchActor. Emit SessionRequired if no
    /// search actor is active.
    pub(super) async fn route_search_command(&self, command: SearchCommand) {
        let request_id = match &command {
            SearchCommand::Query { request_id, .. }
            | SearchCommand::Attachments { request_id, .. }
            | SearchCommand::StartHistoryCrawl { request_id, .. }
            | SearchCommand::StopHistoryCrawl { request_id, .. } => *request_id,
        };
        let query_context = match &command {
            SearchCommand::Query {
                request_id,
                query,
                scope,
                ..
            } => Some((*request_id, query.clone(), scope.clone())),
            _ => None,
        };
        match &self.search_actor {
            Some(handle) => {
                if !handle.send_command(command).await {
                    if let Some((request_id, query, scope)) = query_context.as_ref() {
                        self.emit_search_failed(
                            *request_id,
                            query,
                            scope,
                            SEARCH_UNAVAILABLE_MESSAGE,
                        )
                        .await;
                    }
                    self.emit_failure(request_id, CoreFailure::SessionRequired);
                }
            }
            None => {
                if let Some((request_id, query, scope)) = query_context.as_ref() {
                    self.emit_search_failed(*request_id, query, scope, SEARCH_UNAVAILABLE_MESSAGE)
                        .await;
                }
                self.emit_failure(request_id, CoreFailure::SessionRequired);
            }
        }
    }

    /// Route a ThreadsListCommand to the ThreadsListActor. Spawns the actor
    /// on `Open` when a session is present; drops it on `Close`.
    pub(super) async fn route_threads_list_command(&mut self, command: ThreadsListCommand) {
        match command {
            ThreadsListCommand::Open {
                request_id,
                scope,
                room_ids,
            } => {
                let Some(session) = self.session.clone() else {
                    self.emit_threads_list_failed(request_id, scope.scope_key())
                        .await;
                    self.emit_failure(request_id, CoreFailure::SessionRequired);
                    return;
                };
                if self.threads_list_actor.is_none() {
                    self.threads_list_actor = Some(crate::threads_list::ThreadsListActor::spawn(
                        session,
                        self.action_tx.clone(),
                        self.event_tx.clone(),
                    ));
                }
                if let Some(handle) = &self.threads_list_actor {
                    let _ = handle.open(request_id, scope, room_ids).await;
                }
            }
            ThreadsListCommand::Close { request_id } => {
                if let Some(handle) = self.threads_list_actor.take() {
                    let _ = handle.close(request_id).await;
                }
            }
            ThreadsListCommand::Paginate {
                request_id,
                scope: _,
            } => {
                if let Some(handle) = &self.threads_list_actor {
                    let _ = handle.paginate(request_id).await;
                }
            }
        }
    }

    async fn emit_search_failed(
        &self,
        request_id: RequestId,
        query: &str,
        scope: &koushi_protocol::command::SearchScope,
        message: &str,
    ) {
        let _ = self
            .action_tx
            .send(vec![AppAction::SearchFailed {
                request_id: request_id.sequence,
                query: query.to_owned(),
                scope: search_scope_to_state(scope),
                message: message.to_owned(),
            }])
            .await;
    }

    async fn emit_threads_list_failed(&self, request_id: RequestId, room_id: String) {
        let _ = self
            .action_tx
            .send(vec![AppAction::ThreadsListFailed {
                request_id: request_id.sequence,
                room_id,
                failure_kind: OperationFailureKind::Network,
            }])
            .await;
    }

    fn record_event_cache_repair(
        request_id: RequestId,
        stage: &'static str,
        outcome: &'static str,
        reason: &'static str,
    ) {
        record(
            DiagnosticEvent::new(DiagnosticLevel::Debug, "core.event_cache_repair", stage)
                .field(DiagnosticField::request_id(
                    "request_id",
                    request_id.connection_id.0,
                    request_id.sequence,
                ))
                .field(DiagnosticField::token("outcome", outcome))
                .field(DiagnosticField::token("reason", reason)),
        );
    }

    pub(super) async fn handle_ensure_room_event_cached(
        &mut self,
        request_id: RequestId,
        room_id: String,
        event_id: String,
    ) -> crate::account::RoomEventLookupResult {
        #[cfg(test)]
        if let Some(fetch) = self.event_cache_fetch_override.take() {
            return match crate::executor::timeout(ROOM_EVENT_CACHE_TEST_TIMEOUT, fetch).await {
                Ok(Ok(result)) => {
                    Self::record_event_cache_repair(request_id, "done", "succeeded", "loaded");
                    result
                }
                Ok(Err(_)) => {
                    Self::record_event_cache_repair(request_id, "failed", "failed", "sdk");
                    crate::account::RoomEventLookupResult::Failed
                }
                Err(_) => {
                    Self::record_event_cache_repair(request_id, "failed", "failed", "timeout");
                    crate::account::RoomEventLookupResult::Failed
                }
            };
        }

        let Some(session) = &self.session else {
            Self::record_event_cache_repair(request_id, "failed", "failed", "no_session");
            return crate::account::RoomEventLookupResult::Failed;
        };
        let Ok(parsed_room_id) = matrix_sdk::ruma::RoomId::parse(room_id.as_str()) else {
            Self::record_event_cache_repair(request_id, "failed", "failed", "invalid_room");
            return crate::account::RoomEventLookupResult::Failed;
        };
        let Ok(parsed_event_id) = matrix_sdk::ruma::EventId::parse(event_id.as_str()) else {
            Self::record_event_cache_repair(request_id, "failed", "failed", "invalid_event");
            return crate::account::RoomEventLookupResult::Failed;
        };
        let Some(room) = session.client().get_room(&parsed_room_id) else {
            Self::record_event_cache_repair(request_id, "failed", "failed", "room_missing");
            return crate::account::RoomEventLookupResult::Failed;
        };

        match crate::executor::timeout(
            ROOM_EVENT_CACHE_TIMEOUT,
            room.load_or_fetch_event(&parsed_event_id, None),
        )
        .await
        {
            Ok(Ok(_)) => {
                Self::record_event_cache_repair(request_id, "done", "succeeded", "loaded");
                if let Some(account_key) = self.active_account_key() {
                    self.route_timeline_command(TimelineCommand::RepairGaps {
                        request_id,
                        key: TimelineKey {
                            account_key,
                            kind: TimelineKind::Room { room_id },
                        },
                    })
                    .await;
                }
                crate::account::RoomEventLookupResult::Located
            }
            Ok(Err(error)) => {
                let result = classify_room_event_lookup_error(error.client_api_error_kind());
                match result {
                    crate::account::RoomEventLookupResult::Missing => {
                        Self::record_event_cache_repair(
                            request_id,
                            "failed",
                            "missing",
                            "not_found",
                        );
                    }
                    crate::account::RoomEventLookupResult::Failed => {
                        Self::record_event_cache_repair(request_id, "failed", "failed", "sdk");
                    }
                    crate::account::RoomEventLookupResult::Located => unreachable!(),
                }
                result
            }
            Err(_) => {
                Self::record_event_cache_repair(request_id, "failed", "failed", "timeout");
                crate::account::RoomEventLookupResult::Failed
            }
        }
    }

    pub(super) async fn handle_open_timeline_at_timestamp(
        &mut self,
        request_id: RequestId,
        room_id: String,
        timestamp_ms: u64,
    ) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        let Some(account_key) = self.active_account_key() else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        let parsed_room_id = match matrix_sdk::ruma::RoomId::parse(room_id.as_str()) {
            Ok(room_id) => room_id,
            Err(_) => {
                self.emit_failure(
                    request_id,
                    CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::Sdk,
                    },
                );
                return;
            }
        };

        let request = Self::timeline_event_by_timestamp_request(parsed_room_id, timestamp_ms);
        let response = match session.client().send(request).await {
            Ok(response) => response,
            Err(_) => {
                self.emit_failure(
                    request_id,
                    CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::Sdk,
                    },
                );
                return;
            }
        };
        let event_id = response.event_id.to_string();
        // #161: jump-to-date renders the focused timeline in the MAIN pane
        // (marked by `main_timeline_anchor`), reusing the focused-context
        // subscription lifecycle; it must not open the right panel.
        let _ = self
            .action_tx
            .send(vec![
                AppAction::OpenFocusedContext {
                    room_id: room_id.clone(),
                    event_id: event_id.clone(),
                },
                AppAction::EnterAnchoredTimeline {
                    room_id: room_id.clone(),
                    event_id: event_id.clone(),
                },
            ])
            .await;
        self.route_timeline_command(TimelineCommand::Subscribe {
            request_id,
            key: TimelineKey {
                account_key,
                kind: TimelineKind::Focused { room_id, event_id },
            },
            initial_backfill: koushi_protocol::command::InitialBackfillPolicy::Disabled,
        })
        .await;
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) async fn route_sync_once_for_qa(&mut self, request_id: RequestId) {
        if let Some(handle) = &self.sync_actor {
            if !handle.sync_once_for_qa(request_id).await {
                self.emit_failure(
                    request_id,
                    CoreFailure::SyncFailed {
                        kind: SyncFailureKind::Internal,
                    },
                );
            }
        } else {
            self.emit_failure(
                request_id,
                CoreFailure::SyncFailed {
                    kind: SyncFailureKind::Internal,
                },
            );
        }
    }

    /// Route a SyncCommand to the SyncActor, or emit SessionRequired if no
    /// store-backed session is active yet.
    pub(super) async fn route_sync_command(&mut self, command: SyncCommand) {
        let (command_kind, request_id) = match &command {
            SyncCommand::Start { request_id } => ("start", *request_id),
            SyncCommand::Stop { request_id } => ("stop", *request_id),
            SyncCommand::Restart { request_id } => ("restart", *request_id),
        };
        trace_restore!(
            "route_sync_command",
            [
                DiagnosticField::request_id(
                    "request_id",
                    request_id.connection_id.0,
                    request_id.sequence
                ),
                DiagnosticField::token("kind", command_kind),
                DiagnosticField::boolean("session", self.session.is_some()),
                DiagnosticField::boolean("sync_actor", self.sync_actor.is_some()),
                DiagnosticField::token("action", "begin"),
            ],
            "request_id={} kind={} session={} sync_actor={} action=begin",
            request_id_trace_label(request_id),
            command_kind,
            if self.session.is_some() { "yes" } else { "no" },
            if self.sync_actor.is_some() {
                "yes"
            } else {
                "no"
            }
        );

        if self.sync_actor.is_none()
            && !matches!(command, SyncCommand::Stop { .. })
            && let Some(session) = &self.session
        {
            trace_restore!(
                "route_sync_command",
                [
                    DiagnosticField::request_id(
                        "request_id",
                        request_id.connection_id.0,
                        request_id.sequence
                    ),
                    DiagnosticField::token("kind", command_kind),
                    DiagnosticField::token("action", "spawn_sync_actor"),
                ],
                "request_id={} kind={} action=spawn_sync_actor",
                request_id_trace_label(request_id),
                command_kind
            );
            self.spawn_sync_actor(session.clone()).await;
        }

        match &self.sync_actor {
            Some(handle) => {
                trace_restore!(
                    "route_sync_command",
                    [
                        DiagnosticField::request_id(
                            "request_id",
                            request_id.connection_id.0,
                            request_id.sequence
                        ),
                        DiagnosticField::token("kind", command_kind),
                        DiagnosticField::token("action", "send_to_sync_actor"),
                    ],
                    "request_id={} kind={} action=send_to_sync_actor",
                    request_id_trace_label(request_id),
                    command_kind
                );
                // The SyncActor notifies the RoomActor itself on start/stop/
                // restart: only it knows the selected backend and owns the
                // live RoomListService (canon, overview.md RoomActor bullet).
                let _ = handle.send(SyncMessage::Command(command)).await;
            }
            None if self.session.is_none() => {
                trace_restore!(
                    "route_sync_command",
                    [
                        DiagnosticField::request_id(
                            "request_id",
                            request_id.connection_id.0,
                            request_id.sequence
                        ),
                        DiagnosticField::token("kind", command_kind),
                        DiagnosticField::token("action", "session_required"),
                    ],
                    "request_id={} kind={} action=session_required",
                    request_id_trace_label(request_id),
                    command_kind
                );
                // Session not yet ready — gate is enforced in AppActor but be
                // defensive here too.
                self.emit_failure(request_id, CoreFailure::SessionRequired);
            }
            None => {
                trace_restore!(
                    "route_sync_command",
                    [
                        DiagnosticField::request_id(
                            "request_id",
                            request_id.connection_id.0,
                            request_id.sequence
                        ),
                        DiagnosticField::token("kind", command_kind),
                        DiagnosticField::token("action", "no_sync_actor"),
                    ],
                    "request_id={} kind={} action=no_sync_actor",
                    request_id_trace_label(request_id),
                    command_kind
                );
            }
        }
    }

    fn timeline_event_by_timestamp_request(
        room_id: matrix_sdk::ruma::OwnedRoomId,
        timestamp_ms: u64,
    ) -> matrix_sdk::ruma::api::client::room::get_event_by_timestamp::v1::Request {
        use matrix_sdk::ruma::{MilliSecondsSinceUnixEpoch, UInt};

        matrix_sdk::ruma::api::client::room::get_event_by_timestamp::v1::Request::since(
            room_id,
            MilliSecondsSinceUnixEpoch(UInt::new_saturating(timestamp_ms)),
        )
    }
}

fn classify_room_event_lookup_error(
    kind: Option<&matrix_sdk::ruma::api::error::ErrorKind>,
) -> crate::account::RoomEventLookupResult {
    if matches!(
        kind,
        Some(matrix_sdk::ruma::api::error::ErrorKind::NotFound)
    ) {
        crate::account::RoomEventLookupResult::Missing
    } else {
        crate::account::RoomEventLookupResult::Failed
    }
}

#[cfg(test)]
mod tests {

    use koushi_protocol::SessionKeyId;

    use std::time::Duration;

    use matrix_sdk::ruma::api::error::ErrorKind;
    use tokio::sync::oneshot;

    use super::{
        classify_room_event_lookup_error, composer_timeline_command_targets_active_session,
    };
    use crate::account::RoomEventLookupResult;
    use crate::account::actor::AccountMessage;
    use crate::account::test_support::spawn_actor_with_dirs;
    use crate::executor;
    use koushi_protocol::command::TimelineCommand;

    use koushi_protocol::ids::{AccountKey, RequestId, RuntimeConnectionId, TimelineKey};

    use tempfile::tempdir;

    #[test]
    fn composer_timeline_command_rechecks_full_session_owner_before_account_routing() {
        let active = SessionKeyId {
            homeserver: "https://active.example.test".to_owned(),
            user_id: "@same-user:example.test".to_owned(),
            device_id: "ACTIVE".to_owned(),
        };
        let stale = SessionKeyId {
            homeserver: "https://stale.example.test".to_owned(),
            user_id: active.user_id.clone(),
            device_id: "STALE".to_owned(),
        };
        let command = TimelineCommand::SubmitText {
            request_id: RequestId {
                connection_id: RuntimeConnectionId(1),
                sequence: 1,
            },
            expected_account: stale.clone(),
            submission_id: koushi_state::SubmissionId::new("submission-owner-fence"),
            key: TimelineKey::room(AccountKey(active.user_id.clone()), "!room:example.test"),
            transaction_id: "transaction-owner-fence".to_owned(),
            document: koushi_state::ComposerDocument::from_plain_text("synthetic body"),
            draft_revision: 1.into(),
        };

        assert!(!composer_timeline_command_targets_active_session(
            Some(&active),
            &command
        ));
        assert!(composer_timeline_command_targets_active_session(
            Some(&stale),
            &command
        ));
    }

    #[tokio::test]
    async fn pending_event_cache_fetch_times_out_and_releases_account_actor() {
        let cred_dir = tempdir().expect("credential tempdir");
        let data_dir = tempdir().expect("data tempdir");
        let (handle, _action_rx, _event_rx) =
            crate::account::test_support::spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let (_fetch_tx, fetch_rx) = oneshot::channel::<RoomEventLookupResult>();
        assert!(
            handle
                .send(AccountMessage::ConfigureEventCacheFetchForTesting { fetch: fetch_rx })
                .await
        );

        let request_id = RequestId {
            connection_id: RuntimeConnectionId(7),
            sequence: 23,
        };
        let (response_tx, response_rx) = oneshot::channel();
        assert!(
            handle
                .send(AccountMessage::EnsureRoomEventCached {
                    request_id,
                    room_id: "!synthetic-room:example.invalid".to_owned(),
                    event_id: "$synthetic-event:example.invalid".to_owned(),
                    response_tx,
                })
                .await
        );
        let (acknowledged, completion) = oneshot::channel();
        assert!(
            handle
                .send(AccountMessage::ShutdownWithAck { acknowledged })
                .await
        );

        assert_eq!(
            executor::timeout(Duration::from_secs(1), response_rx)
                .await
                .expect("bounded event-cache response")
                .expect("event-cache response channel"),
            RoomEventLookupResult::Failed
        );
        executor::timeout(Duration::from_secs(1), completion)
            .await
            .expect("account actor should process the following shutdown")
            .expect("account shutdown acknowledgement");
    }

    #[test]
    fn event_cache_repair_diagnostic_runs_without_trace_environment() {
        let child = std::process::Command::new(
            std::env::current_exe().expect("current test executable should be available"),
        )
        .arg("--exact")
        .arg(concat!(
            "account::routing::tests::",
            "event_cache_repair_diagnostic_records_without_trace_environment"
        ))
        .arg("--ignored")
        .arg("--nocapture")
        .env_remove("KOUSHI_TIMELINE_ITEM_TRACE")
        .env_remove("KOUSHI_SUBSCRIBE_TRACE")
        .status()
        .expect("env-unset event-cache-repair child should start");
        assert!(
            child.success(),
            "env-unset event-cache-repair child failed: {child}"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn event_cache_repair_diagnostic_records_without_trace_environment() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        assert!(std::env::var_os("KOUSHI_TIMELINE_ITEM_TRACE").is_none());
        assert!(std::env::var_os("KOUSHI_SUBSCRIBE_TRACE").is_none());

        let cred_dir = tempdir().expect("credential tempdir");
        let data_dir = tempdir().expect("data tempdir");
        let (handle, _action_rx, _event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let synthetic_room_id = "!synthetic-room:example.invalid";
        let synthetic_event_id = "$synthetic-event:example.invalid";
        let request_id = RequestId {
            connection_id: RuntimeConnectionId(17),
            sequence: 23,
        };
        let (response_tx, response_rx) = oneshot::channel();
        assert!(
            handle
                .send(AccountMessage::EnsureRoomEventCached {
                    request_id,
                    room_id: synthetic_room_id.to_owned(),
                    event_id: synthetic_event_id.to_owned(),
                    response_tx,
                })
                .await
        );
        assert_eq!(
            response_rx.await.expect("cache-repair response"),
            RoomEventLookupResult::Failed,
            "a cache miss must not be reported as successful"
        );

        let records = koushi_diagnostics::test_support::detail_snapshot().records;
        let repair = records
            .iter()
            .rev()
            .find(|record| {
                record.event.source == "core.event_cache_repair"
                    && record.event.stage == "failed"
                    && record.event.fields.iter().any(|field| {
                        field.key == "reason"
                            && field.value
                                == koushi_diagnostics::DiagnosticValue::Token("no_session")
                    })
            })
            .expect("event-cache repair should be collected without trace environment");
        assert_eq!(repair.event.source, "core.event_cache_repair");
        assert_eq!(repair.event.stage, "failed");
        assert_eq!(
            repair.event.fields,
            vec![
                koushi_diagnostics::DiagnosticField::request_id("request_id", 17, 23),
                koushi_diagnostics::DiagnosticField::token("outcome", "failed"),
                koushi_diagnostics::DiagnosticField::token("reason", "no_session"),
            ]
        );

        let serialized = serde_json::to_string(&repair.event)
            .expect("event-cache repair event should serialize for privacy assertions");
        for forbidden in [
            synthetic_room_id,
            synthetic_event_id,
            "synthetic-body-value",
            "https://example.invalid/synthetic",
            "/tmp/synthetic-path",
            "raw sdk error: synthetic",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "serialized event must not contain forbidden diagnostic data: {forbidden}"
            );
        }
    }

    #[test]
    fn classify_room_event_lookup_error_only_treats_not_found_as_missing() {
        assert_eq!(
            classify_room_event_lookup_error(Some(&ErrorKind::NotFound)),
            RoomEventLookupResult::Missing
        );
        assert_eq!(
            classify_room_event_lookup_error(Some(&ErrorKind::Forbidden)),
            RoomEventLookupResult::Failed
        );
        assert_eq!(
            classify_room_event_lookup_error(None),
            RoomEventLookupResult::Failed
        );
    }
}
