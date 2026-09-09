use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;
use matrix_sdk_ui::timeline::{
    EncryptedMessage, TimelineItem as SdkTimelineItem, TimelineItemKind,
};
use tokio::sync::mpsc;
#[cfg(test)]
use tokio::sync::oneshot;

use crate::executor;
use koushi_protocol::event::{CoreEvent, TimelineEvent};
use koushi_protocol::failure::TimelineFailureKind;
use koushi_protocol::ids::RequestId;

// BEGIN GENERATED SIBLING IMPORTS
use super::actor::{TimelineActor, TimelineActorMessage};
use super::diagnostics::{
    decrypt_retry_backup_result_for_error, decrypt_retry_failure_for_room_operation,
    record_decrypt_retry_backup_lookup, record_decrypt_retry_device_request,
    record_decrypt_retry_request, record_decrypt_retry_settled, record_room_key_requester_stage,
};
use super::item_projection::{
    decrypt_retry_reason_from_content, key_request_stage_token, key_request_withheld_code_token,
    unable_to_decrypt_from_content,
};
// END GENERATED SIBLING IMPORTS

pub(super) const DECRYPT_RETRY_TIMEOUT: Duration = Duration::from_secs(30);

fn spawn_delayed_timeline_message<T: Send + 'static>(
    tx: mpsc::Sender<T>,
    delay: Duration,
    message: T,
) -> executor::JoinHandle<()> {
    executor::spawn(async move {
        executor::sleep(delay).await;
        let _ = tx.send(message).await;
    })
}

#[derive(Clone, Copy)]
pub(super) enum DecryptRetryReason {
    MissingRoomKey,
    Withheld,
    Malformed,
    Unknown,
}

impl DecryptRetryReason {
    pub(super) fn token(self) -> &'static str {
        match self {
            Self::MissingRoomKey => "missing_room_key",
            Self::Withheld => "withheld",
            Self::Malformed => "malformed",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum DecryptRetryBackupState {
    Available,
    Unknown,
}

impl DecryptRetryBackupState {
    pub(super) fn token(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Unknown => "unknown",
        }
    }
}

fn decrypt_retry_backup_state_for(
    backup: koushi_sdk::MatrixSecureBackupLocalState,
    recovery: koushi_sdk::MatrixSecureBackupRecoveryState,
) -> DecryptRetryBackupState {
    if backup == koushi_sdk::MatrixSecureBackupLocalState::Enabled
        && recovery == koushi_sdk::MatrixSecureBackupRecoveryState::Enabled
    {
        DecryptRetryBackupState::Available
    } else {
        DecryptRetryBackupState::Unknown
    }
}

#[derive(Clone, Copy)]
pub(super) enum DecryptRetryBackupResult {
    Found,
    NotFound,
    Network,
    Forbidden,
    InvalidBackup,
    Timeout,
    Sdk,
}

impl DecryptRetryBackupResult {
    pub(super) fn token(self) -> &'static str {
        match self {
            Self::Found => "found",
            Self::NotFound => "not_found",
            Self::Network => "network",
            Self::Forbidden => "forbidden",
            Self::InvalidBackup => "invalid_backup",
            Self::Timeout => "timeout",
            Self::Sdk => "sdk",
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum DecryptRetryDeviceResult {
    Sent,
    Failed,
}

impl DecryptRetryDeviceResult {
    pub(super) fn token(self) -> &'static str {
        match self {
            Self::Sent => "sent",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum DecryptRetryFailure {
    Network,
    Forbidden,
    Timeout,
    Sdk,
}

impl DecryptRetryFailure {
    pub(super) fn token(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Forbidden => "forbidden",
            Self::Timeout => "timeout",
            Self::Sdk => "sdk",
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy)]
pub(super) enum DecryptRetrySettledResult {
    Decrypted,
    StillMissing,
    Withheld,
    Malformed,
    Timeout,
    Superseded,
}

impl DecryptRetrySettledResult {
    pub(super) fn token(self) -> &'static str {
        match self {
            Self::Decrypted => "decrypted",
            Self::StillMissing => "still_missing",
            Self::Withheld => "withheld",
            Self::Malformed => "malformed",
            Self::Timeout => "timeout",
            Self::Superseded => "superseded",
        }
    }
}

pub(super) fn decrypt_retry_diff_settlement(
    diff: &eyeball_im::VectorDiff<Arc<SdkTimelineItem>>,
    event_id: &str,
) -> Option<DecryptRetrySettledResult> {
    let settlement = |item: &Arc<SdkTimelineItem>| {
        let TimelineItemKind::Event(event_item) = item.kind() else {
            return None;
        };
        if !event_item
            .event_id()
            .is_some_and(|candidate| candidate.as_str() == event_id)
        {
            return None;
        }
        if !event_item.content().is_unable_to_decrypt() {
            Some(DecryptRetrySettledResult::Decrypted)
        } else if matches!(
            decrypt_retry_reason_from_content(event_item.content()),
            DecryptRetryReason::Withheld
        ) {
            Some(DecryptRetrySettledResult::Withheld)
        } else {
            None
        }
    };
    match diff {
        eyeball_im::VectorDiff::PushFront { value }
        | eyeball_im::VectorDiff::PushBack { value }
        | eyeball_im::VectorDiff::Insert { value, .. }
        | eyeball_im::VectorDiff::Set { value, .. } => settlement(value),
        eyeball_im::VectorDiff::Reset { values } => values.iter().find_map(settlement),
        eyeball_im::VectorDiff::Append { values } => values.iter().find_map(settlement),
        eyeball_im::VectorDiff::Clear
        | eyeball_im::VectorDiff::PopFront
        | eyeball_im::VectorDiff::PopBack
        | eyeball_im::VectorDiff::Remove { .. }
        | eyeball_im::VectorDiff::Truncate { .. } => None,
    }
}

static NEXT_DECRYPT_RETRY_OPERATION: AtomicU64 = AtomicU64::new(1);

fn next_decrypt_retry_operation() -> u64 {
    NEXT_DECRYPT_RETRY_OPERATION.fetch_add(1, Ordering::Relaxed)
}

/// Presentation state of a user/automatic room-key request (issue #460).
/// Rust-owned; React renders the closed tokens only.
#[derive(Clone, Debug)]
pub(super) struct KeyRequestUiState {
    pub(super) stage: &'static str,
    pub(super) withheld_code: Option<&'static str>,
    pub(super) session_id: Option<String>,
    /// Command correlation for externally issued requests (both origins;
    /// issue #460); None only for actor-internal automatic work.
    request_id: Option<RequestId>,
}

#[derive(Clone)]
pub(super) struct PendingDecryptRetry {
    pub(super) event_id: String,
    operation: u64,
    attempt: u8,
    actor_generation: u64,
    started_at: executor::Instant,
    deadline: executor::Instant,
}

struct DecryptRetrySettlement {
    pending: PendingDecryptRetry,
    result: DecryptRetrySettledResult,
}

#[derive(Default)]
pub(super) struct DecryptRetryController {
    pub(super) pending: Option<PendingDecryptRetry>,
}

impl DecryptRetryController {
    fn admit(
        &mut self,
        event_id: &str,
        actor_generation: u64,
        started_at: executor::Instant,
    ) -> (PendingDecryptRetry, Option<PendingDecryptRetry>, bool) {
        if let Some(current) = self.pending.as_ref().filter(|pending| {
            pending.event_id == event_id && pending.actor_generation == actor_generation
        }) {
            return (current.clone(), None, true);
        }
        let previous = self.pending.take();
        let attempt = previous
            .as_ref()
            .filter(|pending| pending.event_id == event_id)
            .map_or(1, |pending| pending.attempt.saturating_add(1));
        let pending = PendingDecryptRetry {
            event_id: event_id.to_owned(),
            operation: next_decrypt_retry_operation(),
            attempt,
            actor_generation,
            started_at,
            deadline: started_at + DECRYPT_RETRY_TIMEOUT,
        };
        self.pending = Some(pending.clone());
        (pending, previous, false)
    }

    pub(super) fn is_current(&self, operation: u64, actor_generation: u64) -> bool {
        self.pending.as_ref().is_some_and(|pending| {
            pending.operation == operation && pending.actor_generation == actor_generation
        })
    }

    fn settle_if_current(
        &mut self,
        operation: u64,
        actor_generation: u64,
        result: DecryptRetrySettledResult,
    ) -> Option<DecryptRetrySettlement> {
        if !self.is_current(operation, actor_generation) {
            return None;
        }
        Some(DecryptRetrySettlement {
            pending: self.pending.take().expect("current retry is retained"),
            result,
        })
    }

    fn settle_timeout_if_current(
        &mut self,
        operation: u64,
        actor_generation: u64,
    ) -> Option<DecryptRetrySettlement> {
        self.settle_if_current(
            operation,
            actor_generation,
            DecryptRetrySettledResult::Timeout,
        )
    }
}

pub(super) fn decrypt_retry_settlement_operation(
    controller: &DecryptRetryController,
    actor_generation: u64,
    event_id: &str,
) -> Option<u64> {
    controller.pending.as_ref().and_then(|pending| {
        (pending.actor_generation == actor_generation && pending.event_id == event_id)
            .then_some(pending.operation)
    })
}

impl TimelineActor {
    fn begin_decrypt_retry(
        &mut self,
        event_id: &str,
        reason: DecryptRetryReason,
        backup_state: DecryptRetryBackupState,
    ) -> Option<PendingDecryptRetry> {
        let (pending, previous, coalesced) =
            self.decrypt_retry
                .admit(event_id, self.actor_generation, executor::Instant::now());
        if coalesced {
            record_room_key_requester_stage(
                pending.operation,
                "awaiting",
                "none",
                pending.started_at.elapsed(),
            );
            return None;
        }
        if let Some(previous) = previous {
            record_decrypt_retry_settled(
                previous.operation,
                DecryptRetrySettledResult::Superseded,
                previous.started_at.elapsed(),
            );
            if let Some(task) = self.decrypt_retry_timeout_task.take() {
                task.abort();
            }
            // Issue #460: the superseded request's operational window ends and
            // its timeout task is gone — move its presentation to
            // still_waiting (the Matrix request is still outstanding; a late
            // key or withheld observation settles it further).
            if let Some(state) = self.key_request_states.get_mut(&previous.event_id) {
                if !matches!(
                    state.stage,
                    "withheld" | "decryption_recovered" | "send_failed"
                ) {
                    state.stage = "still_waiting";
                }
            }
            if let Some(state) = self.key_request_states.get(&previous.event_id) {
                self.publish_key_request_state(&previous.event_id, state);
            }
        }
        record_decrypt_retry_request(
            pending.operation,
            pending.attempt,
            reason,
            backup_state,
            Duration::ZERO,
        );
        Some(pending)
    }
    fn schedule_decrypt_retry_timeout(&mut self, pending: &PendingDecryptRetry) {
        if let Some(task) = self.decrypt_retry_timeout_task.take() {
            task.abort();
        }
        self.decrypt_retry_timeout_task = Some(spawn_delayed_timeline_message(
            self.msg_tx.clone(),
            pending
                .deadline
                .saturating_duration_since(executor::Instant::now()),
            TimelineActorMessage::DecryptRetryTimeout {
                operation: pending.operation,
                actor_generation: pending.actor_generation,
            },
        ));
    }
    pub(super) fn settle_decrypt_retry(
        &mut self,
        operation: u64,
        result: DecryptRetrySettledResult,
    ) {
        let Some(settlement) = (match result {
            DecryptRetrySettledResult::Timeout => self
                .decrypt_retry
                .settle_timeout_if_current(operation, self.actor_generation),
            result => {
                self.decrypt_retry
                    .settle_if_current(operation, self.actor_generation, result)
            }
        }) else {
            return;
        };
        if let Some(task) = self.decrypt_retry_timeout_task.take() {
            task.abort();
        }
        record_decrypt_retry_settled(
            settlement.pending.operation,
            settlement.result,
            settlement.pending.started_at.elapsed(),
        );
        // Issue #460: update the presentation state for the affected event.
        let event_id = settlement.pending.event_id.clone();
        let stage = match settlement.result {
            DecryptRetrySettledResult::Decrypted => Some("decryption_recovered"),
            DecryptRetrySettledResult::Withheld => Some("withheld"),
            DecryptRetrySettledResult::Timeout => Some("still_waiting"),
            // Request enqueue/SDK failures settle as still-missing; surface
            // them as a terminal send failure so the UI is not stuck waiting.
            DecryptRetrySettledResult::StillMissing => Some("send_failed"),
            _ => None,
        };
        if let Some(stage) = stage {
            // Resolve a closed withheld code from the observed to-device
            // `m.room_key.withheld` for this event's session, if known.
            let existing_session = self
                .key_request_states
                .get(&event_id)
                .and_then(|state| state.session_id.clone());
            let withheld_code = existing_session.as_deref().and_then(|session| {
                self.withheld_codes
                    .get(&(self.key.room_id().to_owned(), session.to_owned()))
                    .copied()
            });
            self.key_request_states
                .entry(event_id.clone())
                .and_modify(|state| {
                    state.stage = stage;
                    state.withheld_code = withheld_code;
                })
                .or_insert(KeyRequestUiState {
                    stage,
                    withheld_code,
                    session_id: None,
                    request_id: None,
                });
            // Issue #460: every settle transition is published so the UI can
            // reflect withheld / still-waiting / send-failure even when no
            // timeline diff follows (static timeline, to-device withheld).
            if let Some(state) = self.key_request_states.get(&event_id) {
                self.publish_key_request_state(&event_id, state);
            }
        }
    }
    pub(super) fn publish_key_request_state(&self, event_id: &str, state: &KeyRequestUiState) {
        // Generation fence: a replaced actor must not publish outcomes for a
        // batch the UI has already discarded (same gate as timeline events).
        let Some(_lease) = self
            .timeline_actor_generations
            .try_acquire(&self.key, self.actor_generation)
        else {
            return;
        };
        let _ = self.event_tx.send(CoreEvent::Room(
            koushi_protocol::event::RoomEvent::RoomKeyRequestStateChanged {
                key: self.key.clone(),
                event_id: event_id.to_owned(),
                request_id: state.request_id.clone(),
                stage: key_request_stage_token(state.stage),
                withheld_code: state
                    .withheld_code
                    .and_then(key_request_withheld_code_token),
            },
        ));
    }
    /// Issue #460: queue automatic key-request messages for events, retaining
    /// candidates that hit a full mailbox for a later retry (never blocks the
    /// projection on the actor's own bounded mailbox).
    pub(super) fn dispatch_auto_key_requests(&mut self, event_ids: Vec<String>) {
        for event_id in event_ids {
            if self.key_request_states.contains_key(&event_id) {
                continue;
            }
            match self.msg_tx.try_send(TimelineActorMessage::RequestRoomKey {
                request_id: None,
                event_id,
                origin: koushi_protocol::command::KeyRequestOrigin::Automatic,
            }) {
                Ok(()) => {}
                Err(tokio::sync::mpsc::error::TrySendError::Full(message)) => {
                    match message {
                        TimelineActorMessage::RequestRoomKey { event_id, .. }
                            if !self.pending_auto_key_requests.contains(&event_id) =>
                        {
                            // Dedup: repeated Reset batches re-scan the same
                            // events; the pending set stays bounded by the
                            // number of distinct requestable events.
                            self.pending_auto_key_requests.push(event_id);
                        }
                        _ => {}
                    }
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {}
            }
        }
    }
    pub(super) async fn handle_request_room_key(
        &mut self,
        request_id: Option<RequestId>,
        event_id: String,
        origin: koushi_protocol::command::KeyRequestOrigin,
    ) {
        let requested_event_id = event_id.clone();
        let event_id = match matrix_sdk::ruma::EventId::parse(&event_id) {
            Ok(event_id) => event_id,
            Err(_) => {
                if let Some(request_id) = request_id {
                    self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendTarget);
                }
                return;
            }
        };
        let Some(event_item) = self.timeline.item_by_event_id(&event_id).await else {
            if let Some(request_id) = request_id {
                self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendTarget);
            }
            return;
        };
        if !event_item.content().is_unable_to_decrypt() {
            if let Some(request_id) = request_id {
                self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendState);
            }
            return;
        }
        let retry_reason = decrypt_retry_reason_from_content(event_item.content());
        let backup_observation = self.session.observe_secure_backup_state();
        // Issue #460: automatic (thread auto-recovery) requests are one-shot
        // per event — once a request state exists, repeats are refused so a
        // settled still-waiting/withheld event is not re-spammed on every
        // render. Externally issued commands still retain correlation: when a
        // concrete request id is present, republish the in-flight state with
        // it (silent return only for actor-internal None).
        if origin == koushi_protocol::command::KeyRequestOrigin::Automatic
            && self.key_request_states.contains_key(&requested_event_id)
        {
            if let Some(request_id) = request_id
                && let Some(state) = self.key_request_states.get(&requested_event_id)
            {
                let correlated = KeyRequestUiState {
                    stage: state.stage,
                    withheld_code: state.withheld_code,
                    session_id: state.session_id.clone(),
                    request_id: Some(request_id),
                };
                self.publish_key_request_state(&requested_event_id, &correlated);
            }
            return;
        }
        let Some(pending) = self.begin_decrypt_retry(
            &requested_event_id,
            retry_reason,
            decrypt_retry_backup_state_for(
                backup_observation.current.backup,
                backup_observation.current.recovery,
            ),
        ) else {
            // Issue #460: coalesced duplicate — the request for this event is
            // already in flight. Republish the current state correlated to
            // this accepted command so every accepted command retains its
            // request_id (the frontend apply is idempotent).
            if let Some(state) = self.key_request_states.get(&requested_event_id) {
                let correlated = KeyRequestUiState {
                    stage: state.stage,
                    withheld_code: state.withheld_code,
                    session_id: state.session_id.clone(),
                    request_id: request_id.clone(),
                };
                self.publish_key_request_state(&requested_event_id, &correlated);
            }
            return;
        };
        let Some(original_json) = event_item.original_json().cloned() else {
            self.settle_decrypt_retry(pending.operation, DecryptRetrySettledResult::Malformed);
            if let Some(request_id) = request_id {
                self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendTarget);
            }
            return;
        };
        let room_id = match matrix_sdk::ruma::RoomId::parse(self.key.room_id()) {
            Ok(room_id) => room_id,
            Err(_) => {
                self.settle_decrypt_retry(pending.operation, DecryptRetrySettledResult::Malformed);
                if let Some(request_id) = request_id {
                    self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendTarget);
                }
                return;
            }
        };
        let session_id =
            unable_to_decrypt_from_content(event_item.content()).and_then(|utd| utd.session_id);
        // Issue #460: publish the request state so the UI can show progress.
        self.key_request_states.insert(
            requested_event_id.clone(),
            KeyRequestUiState {
                stage: if origin == koushi_protocol::command::KeyRequestOrigin::User {
                    "sent"
                } else {
                    "automatic"
                },
                withheld_code: None,
                session_id: session_id.clone(),
                request_id: request_id.clone(),
            },
        );
        // Issue #460: publish the accepted initial state so the UI can move
        // off its local optimistic marker onto Rust-owned waiting copy.
        if let Some(state) = self.key_request_states.get(&requested_event_id) {
            self.publish_key_request_state(&requested_event_id, state);
        }
        let Some(session_id) = session_id else {
            let result = executor::timeout_at(
                pending.deadline,
                koushi_sdk::request_room_key_for_event(
                    &self.session,
                    room_id.as_str(),
                    &original_json,
                ),
            )
            .await;
            match result {
                Ok(Ok(())) => record_decrypt_retry_device_request(
                    pending.operation,
                    DecryptRetryDeviceResult::Sent,
                    None,
                    pending.started_at.elapsed(),
                ),
                Ok(Err(error)) => {
                    record_decrypt_retry_device_request(
                        pending.operation,
                        DecryptRetryDeviceResult::Failed,
                        Some(decrypt_retry_failure_for_room_operation(&error)),
                        pending.started_at.elapsed(),
                    );
                    self.settle_decrypt_retry(
                        pending.operation,
                        DecryptRetrySettledResult::StillMissing,
                    );
                    if let Some(request_id) = request_id {
                        self.emit_timeline_failure(request_id, TimelineFailureKind::Sdk);
                    }
                    return;
                }
                Err(_) => {
                    record_decrypt_retry_device_request(
                        pending.operation,
                        DecryptRetryDeviceResult::Failed,
                        Some(DecryptRetryFailure::Timeout),
                        pending.started_at.elapsed(),
                    );
                    self.settle_decrypt_retry(
                        pending.operation,
                        DecryptRetrySettledResult::Timeout,
                    );
                    return;
                }
            }
            self.schedule_decrypt_retry_timeout(&pending);
            return;
        };
        match executor::timeout_at(
            pending.deadline,
            koushi_sdk::download_room_key_from_backup(&self.session, room_id.as_str(), &session_id),
        )
        .await
        {
            Ok(Ok(true)) => {
                record_decrypt_retry_backup_lookup(
                    pending.operation,
                    DecryptRetryBackupResult::Found,
                    pending.started_at.elapsed(),
                );
                if executor::timeout_at(
                    pending.deadline,
                    self.timeline.retry_decryption([session_id]),
                )
                .await
                .is_err()
                {
                    self.settle_decrypt_retry(
                        pending.operation,
                        DecryptRetrySettledResult::Timeout,
                    );
                    return;
                }
            }
            Ok(Ok(false)) => {
                record_decrypt_retry_backup_lookup(
                    pending.operation,
                    DecryptRetryBackupResult::NotFound,
                    pending.started_at.elapsed(),
                );
                let result = executor::timeout_at(
                    pending.deadline,
                    koushi_sdk::request_room_key_for_event(
                        &self.session,
                        room_id.as_str(),
                        &original_json,
                    ),
                )
                .await;
                match result {
                    Ok(Ok(())) => record_decrypt_retry_device_request(
                        pending.operation,
                        DecryptRetryDeviceResult::Sent,
                        None,
                        pending.started_at.elapsed(),
                    ),
                    Ok(Err(error)) => {
                        record_decrypt_retry_device_request(
                            pending.operation,
                            DecryptRetryDeviceResult::Failed,
                            Some(decrypt_retry_failure_for_room_operation(&error)),
                            pending.started_at.elapsed(),
                        );
                        self.settle_decrypt_retry(
                            pending.operation,
                            DecryptRetrySettledResult::StillMissing,
                        );
                        if let Some(request_id) = request_id {
                            self.emit_timeline_failure(request_id, TimelineFailureKind::Sdk);
                        }
                        return;
                    }
                    Err(_) => {
                        record_decrypt_retry_device_request(
                            pending.operation,
                            DecryptRetryDeviceResult::Failed,
                            Some(DecryptRetryFailure::Timeout),
                            pending.started_at.elapsed(),
                        );
                        self.settle_decrypt_retry(
                            pending.operation,
                            DecryptRetrySettledResult::Timeout,
                        );
                        return;
                    }
                }
            }
            Ok(Err(error)) => {
                record_decrypt_retry_backup_lookup(
                    pending.operation,
                    decrypt_retry_backup_result_for_error(&error),
                    pending.started_at.elapsed(),
                );
                let result = executor::timeout_at(
                    pending.deadline,
                    koushi_sdk::request_room_key_for_event(
                        &self.session,
                        room_id.as_str(),
                        &original_json,
                    ),
                )
                .await;
                match result {
                    Ok(Ok(())) => record_decrypt_retry_device_request(
                        pending.operation,
                        DecryptRetryDeviceResult::Sent,
                        None,
                        pending.started_at.elapsed(),
                    ),
                    Ok(Err(error)) => {
                        record_decrypt_retry_device_request(
                            pending.operation,
                            DecryptRetryDeviceResult::Failed,
                            Some(decrypt_retry_failure_for_room_operation(&error)),
                            pending.started_at.elapsed(),
                        );
                        self.settle_decrypt_retry(
                            pending.operation,
                            DecryptRetrySettledResult::StillMissing,
                        );
                        if let Some(request_id) = request_id {
                            self.emit_timeline_failure(request_id, TimelineFailureKind::Sdk);
                        }
                        return;
                    }
                    Err(_) => {
                        record_decrypt_retry_device_request(
                            pending.operation,
                            DecryptRetryDeviceResult::Failed,
                            Some(DecryptRetryFailure::Timeout),
                            pending.started_at.elapsed(),
                        );
                        self.settle_decrypt_retry(
                            pending.operation,
                            DecryptRetrySettledResult::Timeout,
                        );
                        return;
                    }
                }
            }
            Err(_) => {
                record_decrypt_retry_backup_lookup(
                    pending.operation,
                    DecryptRetryBackupResult::Timeout,
                    pending.started_at.elapsed(),
                );
                self.settle_decrypt_retry(pending.operation, DecryptRetrySettledResult::Timeout);
                return;
            }
        }
        self.schedule_decrypt_retry_timeout(&pending);
    }
    pub(super) async fn handle_request_late_decryption(
        &mut self,
        request_id: Option<RequestId>,
        trigger: &'static str,
    ) {
        // Consolidated receive-side summary: transport/Olm, merge, and
        // late-decryption groups plus event-cache health (#476).
        let diagnostics = koushi_sdk::room_key_receive_diagnostics(&self.session).await;
        crate::room_key_receive::record_room_key_receive_summary(&diagnostics, trigger);

        let items: Vec<_> = self.timeline.items().await.iter().cloned().collect();
        let session_ids = crate::room_key_receive::collect_visible_utd_sessions(&items);
        let requested = !session_ids.is_empty();
        if requested {
            koushi_sdk::request_late_decryption(
                &self.session,
                self.key.room_id(),
                session_ids.iter().cloned(),
            );
        }
        crate::room_key_receive::record_late_decryption_retry(session_ids.len(), requested);
        if let Some(request_id) = request_id {
            if !requested {
                self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendState);
            }
        }
    }
    /// Start or join the standard-only recovery operation for a missing-session
    /// UTD (issue #478). Only `MissingMegolmSession` UTDs are eligible.
    pub(super) fn ensure_room_key_recovery(&mut self, session_id: &str) {
        use super::recovery_model::{RecoveryOperation, RecoveryStage};

        let resume = self.load_recovery_resume(session_id);
        let should_begin = {
            let op = self
                .room_key_recovery
                .entry(session_id.to_owned())
                .or_insert_with(|| {
                    self.next_session_alias += 1;
                    let mut op = RecoveryOperation::new(self.next_session_alias);
                    // Resume the bounded sequence from a persisted record so a
                    // restart does not duplicate requests or reset the backoff.
                    if let Some(record) = resume {
                        op.resume(record);
                    }
                    op
                });
            op.stage() == RecoveryStage::Detected && op.attempts() == 0 && op.begin_attempt()
        };
        if should_begin {
            let attempts = self
                .room_key_recovery
                .get(session_id)
                .map(|op| op.attempts())
                .unwrap_or(0);
            self.schedule_recovery_tick(session_id.to_owned(), attempts);
        }
        self.persist_recovery_state();
    }
    /// Path of the per-account recovery resume file.
    fn recovery_resume_path(&self) -> Option<std::path::PathBuf> {
        let data_dir = self.data_dir.as_ref()?;
        Some(data_dir.join(format!("recovery-{}.json", self.key.account_key.0)))
    }
    /// Persist the minimal safe resume records for the current operations.
    fn persist_recovery_state(&mut self) {
        let Some(path) = self.recovery_resume_path() else {
            return;
        };
        let records: std::collections::BTreeMap<
            String,
            super::recovery_model::RecoveryResumeRecord,
        > = self
            .room_key_recovery
            .iter()
            .filter(|(_, op)| !op.is_terminal())
            .map(|(session, op)| (session.clone(), op.resume_record()))
            .collect();
        let Ok(payload) = serde_json::to_vec(&records) else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, payload);
    }
    /// Load the resume record for a session, if any.
    fn load_recovery_resume(
        &self,
        session_id: &str,
    ) -> Option<super::recovery_model::RecoveryResumeRecord> {
        let path = self.recovery_resume_path()?;
        let bytes = std::fs::read(&path).ok()?;
        let records: std::collections::BTreeMap<
            String,
            super::recovery_model::RecoveryResumeRecord,
        > = serde_json::from_slice(&bytes).ok()?;
        records.get(session_id).copied()
    }
    fn schedule_recovery_tick(&mut self, session_id: String, attempt: u32) {
        use super::recovery_model::RECOVERY_BACKOFF;
        if let Some(task) = self.recovery_tick_tasks.remove(&session_id) {
            task.abort();
        }
        let task = spawn_delayed_timeline_message(
            self.msg_tx.clone(),
            RECOVERY_BACKOFF,
            TimelineActorMessage::RoomKeyRecoveryTick {
                session_id: session_id.clone(),
                attempt,
                actor_generation: self.actor_generation,
            },
        );
        self.recovery_tick_tasks.insert(session_id, task);
    }
    /// Drive one automatic recovery step for a missing Megolm session
    /// (issue #478): local store, then trusted backup, then own verified
    /// devices, then a bounded wait for sender re-sharing; finally local
    /// redecryption. Never requests keys from peers and never re-broadcasts.
    pub(super) async fn handle_room_key_recovery_tick(
        &mut self,
        session_id: String,
        attempt: u32,
        actor_generation: u64,
    ) {
        use super::recovery_model::{RecoveryStage, RecoveryStepOutcome as Outcome};

        if self.actor_generation != actor_generation {
            return;
        }
        let stage = match self.room_key_recovery.get(&session_id) {
            Some(op) if attempt == op.attempts() && !op.is_terminal() => op.stage(),
            _ => return,
        };
        let room_id = self.key.room_id().to_owned();
        let outcome = match stage {
            RecoveryStage::CheckingLocal => {
                match koushi_sdk::has_inbound_group_session(&self.session, &room_id, &session_id)
                    .await
                {
                    Ok(true) => Outcome::LocalFound,
                    Ok(false) | Err(_) => Outcome::LocalAbsent,
                }
            }
            RecoveryStage::CheckingBackup => {
                match koushi_sdk::download_room_key_from_backup(
                    &self.session,
                    &room_id,
                    &session_id,
                )
                .await
                {
                    Ok(true) => Outcome::BackupImported,
                    Ok(false) => Outcome::BackupAbsent,
                    Err(_) => Outcome::BackupUnavailable,
                }
            }
            RecoveryStage::RequestingOwnDevices => {
                // Request from own verified devices via the standard
                // m.room_key_request path using a matching UTD event.
                let raw = self
                    .timeline
                    .items()
                    .await
                    .iter()
                    .find_map(|item| {
                        let event = item.as_event()?;
                        let content = event.content();
                        let utd = content.as_unable_to_decrypt()?;
                        let EncryptedMessage::MegolmV1AesSha2 {
                            session_id: sid, ..
                        } = utd
                        else {
                            return None;
                        };
                        (sid.as_str() == session_id).then(|| event.original_json().cloned())
                    })
                    .flatten();
                match raw {
                    Some(raw) => {
                        match koushi_sdk::request_room_key_for_event(&self.session, &room_id, &raw)
                            .await
                        {
                            Ok(()) => Outcome::OwnDeviceRequestQueued,
                            Err(_) => Outcome::OwnDeviceRequestFailed,
                        }
                    }
                    None => Outcome::OwnDeviceRequestFailed,
                }
            }
            RecoveryStage::RepairingOlm => {
                // The standard Olm unwedge work (one-time-key claim and
                // m.dummy) is flushed by the SDK's outgoing request pump on
                // the next sync; record the closed stage and transition to
                // waiting.
                Outcome::OlmRepairFlushed
            }
            RecoveryStage::WaitingForKey => {
                // The key may have arrived (sender re-sharing incl. #477).
                match koushi_sdk::has_inbound_group_session(&self.session, &room_id, &session_id)
                    .await
                {
                    Ok(true) => Outcome::KeyArrived,
                    _ => Outcome::OwnDeviceRequestFailed,
                }
            }
            RecoveryStage::KeyReceived | RecoveryStage::RetryingDecryption => {
                // Key is stored: bounded local redecryption only.
                koushi_sdk::request_late_decryption(&self.session, &room_id, [session_id.clone()]);
                Outcome::RedecryptionRequested
            }
            _ => {
                // Other stages are not driver steps.
                return;
            }
        };
        let next = {
            let Some(op) = self.room_key_recovery.get_mut(&session_id) else {
                return;
            };
            op.observe(outcome)
        };
        self.persist_recovery_state();
        match next {
            RecoveryStage::Recovered => {
                super::recovery_model::record_recovery_settled(RecoveryStage::Recovered);
            }
            RecoveryStage::AutomaticPathsExhausted | RecoveryStage::UnrecoverableNoKnownHolder => {
                super::recovery_model::record_recovery_settled(next);
            }
            RecoveryStage::TemporarilyFailed => {
                // Bounded retry: schedule the next attempt if allowed.
                let can_retry = {
                    let Some(op) = self.room_key_recovery.get_mut(&session_id) else {
                        return;
                    };
                    op.begin_attempt()
                };
                if can_retry {
                    let attempts = self
                        .room_key_recovery
                        .get(&session_id)
                        .map(|op| op.attempts())
                        .unwrap_or(0);
                    self.schedule_recovery_tick(session_id, attempts);
                } else {
                    super::recovery_model::record_recovery_settled(
                        RecoveryStage::AutomaticPathsExhausted,
                    );
                }
            }
            _ => {
                let attempts = self
                    .room_key_recovery
                    .get(&session_id)
                    .map(|op| op.attempts())
                    .unwrap_or(0);
                self.schedule_recovery_tick(session_id, attempts);
            }
        }
    }
    pub(super) async fn handle_forward_message(
        &mut self,
        request_id: RequestId,
        source_event_id: String,
        destination_room_id: String,
        transaction_id: String,
    ) {
        let Some(source) = self
            .project_message_source_for_event(&source_event_id)
            .await
        else {
            self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendTarget);
            return;
        };
        let Some(body) = source
            .body
            .as_deref()
            .filter(|body| !body.trim().is_empty())
        else {
            self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendState);
            return;
        };
        if source.is_redacted {
            self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendState);
            return;
        }

        let destination_room_id_parsed = match matrix_sdk::ruma::RoomId::parse(&destination_room_id)
        {
            Ok(room_id) => room_id,
            Err(_) => {
                self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendTarget);
                return;
            }
        };
        let Some(destination_room) = self.session.client().get_room(&destination_room_id_parsed)
        else {
            self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendTarget);
            return;
        };

        let txn_id = matrix_sdk::ruma::OwnedTransactionId::from(transaction_id.clone());
        let content = RoomMessageEventContent::text_plain(body);
        match destination_room
            .send(content)
            .with_transaction_id(txn_id)
            .await
        {
            Ok(result) => {
                self.emit(CoreEvent::Timeline(TimelineEvent::MessageForwarded {
                    request_id,
                    key: self.key.clone(),
                    destination_room_id,
                    transaction_id,
                    event_id: result.response.event_id.to_string(),
                }));
            }
            Err(_) => {
                self.emit_timeline_failure(request_id, TimelineFailureKind::Sdk);
            }
        }
    }
}

#[cfg(test)]
mod tests;
