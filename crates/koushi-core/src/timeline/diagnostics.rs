use std::time::Duration;

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_sdk::{
    MatrixCommittedRoomTimelineCheckpoint as MatrixRoomSubscriptionCheckpoint,
    MatrixLiveTailRefreshDiagnostics,
};
use koushi_state::AppAction;

use crate::causal_projection::{CausalProjectionDomain, CausalProjectionOperationId};
use crate::live_catchup::LiveCatchupGate;
use crate::live_tail_freshness::{LiveTailFreshnessState, LiveTailSchedulerAction};
use crate::read_state::{ReadAdmissionDiagnostic, ReadCompletionDiagnostic, ReadStateKey};
use koushi_protocol::event::{PaginationDirection, TimelineDiff, TimelineItem, TimelineItemId};
use koushi_protocol::ids::{
    RequestId, TimelineBatchId, TimelineGeneration, TimelineKey, TimelineKind,
};
use koushi_sdk::MatrixLiveTailRefreshOutcome as LiveTailRefreshOutcome;

// BEGIN GENERATED SIBLING IMPORTS
use super::gap_repair::GapBoundaryPresenceCounts;
use super::item_projection::timeline_formatted_body_is_renderable;
use super::read_state::{ReadCommandKind, ReadRetrySource};
use super::room_key_recovery::{
    DECRYPT_RETRY_TIMEOUT, DecryptRetryBackupResult, DecryptRetryBackupState,
    DecryptRetryDeviceResult, DecryptRetryFailure, DecryptRetryReason, DecryptRetrySettledResult,
};
// END GENERATED SIBLING IMPORTS

fn read_state_kind_token(key: &ReadStateKey) -> &'static str {
    match key {
        ReadStateKey::PublicUnthreaded { .. } => "public_unthreaded",
        ReadStateKey::ThreadRead { .. } => "thread",
        ReadStateKey::FullyReadAndPrivateUnthreaded { .. } => "fully_read_private",
    }
}

pub(super) fn record_read_admission(key: &ReadStateKey, diagnostic: ReadAdmissionDiagnostic) {
    let (outcome, candidate_count, waiter_count, superseded_operation_count) = match diagnostic {
        ReadAdmissionDiagnostic::Accepted {
            candidate_count,
            waiter_count,
            superseded_operation_count,
        } => (
            "accepted",
            candidate_count,
            waiter_count,
            superseded_operation_count,
        ),
        ReadAdmissionDiagnostic::Coalesced {
            candidate_count,
            waiter_count,
            superseded_operation_count,
        } => (
            "coalesced",
            candidate_count,
            waiter_count,
            superseded_operation_count,
        ),
        ReadAdmissionDiagnostic::Rejected {
            candidate_count,
            waiter_count,
            ..
        } => ("rejected", candidate_count, waiter_count, 0),
    };
    record_read_state_diagnostic(
        "admission",
        key,
        outcome,
        candidate_count,
        waiter_count,
        superseded_operation_count,
        None,
        None,
    );
}

fn decrypt_retry_elapsed_bucket(elapsed: Duration) -> &'static str {
    match elapsed {
        elapsed if elapsed < Duration::from_secs(1) => "under_1s",
        elapsed if elapsed < Duration::from_secs(5) => "under_5s",
        elapsed if elapsed < DECRYPT_RETRY_TIMEOUT => "under_30s",
        _ => "over_30s",
    }
}

fn decrypt_retry_event(stage: &'static str, operation: u64, elapsed: Duration) -> DiagnosticEvent {
    DiagnosticEvent::new(DiagnosticLevel::Info, "core.decrypt_retry", stage)
        .field(DiagnosticField::correlation("operation", operation))
        .field(DiagnosticField::token(
            "elapsed_bucket",
            decrypt_retry_elapsed_bucket(elapsed),
        ))
}

pub(super) fn record_room_key_requester_stage(
    operation: u64,
    stage: &'static str,
    withheld_code: &'static str,
    elapsed: Duration,
) {
    record(
        DiagnosticEvent::new(DiagnosticLevel::Info, "core.room_key_requester", stage)
            .field(DiagnosticField::ordinal_alias(
                "request_alias",
                "request",
                operation,
            ))
            .field(DiagnosticField::token("withheld_code", withheld_code))
            .field(DiagnosticField::token("response_source", "unknown"))
            .field(DiagnosticField::milliseconds(
                "elapsed_ms",
                elapsed.as_millis(),
            )),
    );
    koushi_diagnostics::increment_counter(match stage {
        "send_started" => "requester_send_started",
        "sent" => "requester_sent",
        "awaiting" => "requester_awaiting",
        "still_waiting" => "requester_still_waiting",
        "withheld_received" => "requester_withheld",
        "key_received" => "requester_key_received",
        "decryption_recovered" => "requester_decryption_recovered",
        "send_failed" => "requester_send_failed",
        _ => "requester_unknown",
    });
}

pub(super) fn record_decrypt_retry_request(
    operation: u64,
    attempt: u8,
    reason: DecryptRetryReason,
    backup_state: DecryptRetryBackupState,
    elapsed: Duration,
) {
    record_room_key_requester_stage(operation, "send_started", "none", elapsed);
    record(
        decrypt_retry_event("request", operation, elapsed)
            .field(DiagnosticField::token("reason", reason.token()))
            .field(DiagnosticField::count("attempt", u64::from(attempt)))
            .field(DiagnosticField::token("backup_state", backup_state.token())),
    );
}

pub(super) fn record_decrypt_retry_backup_lookup(
    operation: u64,
    result: DecryptRetryBackupResult,
    elapsed: Duration,
) {
    record(
        decrypt_retry_event("backup_lookup", operation, elapsed)
            .field(DiagnosticField::token("result", result.token())),
    );
}

pub(super) fn record_decrypt_retry_device_request(
    operation: u64,
    result: DecryptRetryDeviceResult,
    failure: Option<DecryptRetryFailure>,
    elapsed: Duration,
) {
    let mut event = decrypt_retry_event("device_request", operation, elapsed)
        .field(DiagnosticField::token("result", result.token()));
    if let Some(failure) = failure {
        event = event.field(DiagnosticField::token("failure", failure.token()));
    }
    record(event);
    match result {
        DecryptRetryDeviceResult::Sent => {
            record_room_key_requester_stage(operation, "sent", "none", elapsed);
            record_room_key_requester_stage(operation, "awaiting", "none", elapsed);
        }
        DecryptRetryDeviceResult::Failed => {
            record_room_key_requester_stage(operation, "send_failed", "none", elapsed);
        }
    }
}

pub(super) fn record_decrypt_retry_settled(
    operation: u64,
    result: DecryptRetrySettledResult,
    elapsed: Duration,
) {
    match result {
        DecryptRetrySettledResult::Decrypted => {
            record_room_key_requester_stage(operation, "key_received", "none", elapsed);
            record_room_key_requester_stage(operation, "decryption_recovered", "none", elapsed);
        }
        DecryptRetrySettledResult::Withheld => {
            record_room_key_requester_stage(operation, "withheld_received", "custom", elapsed);
        }
        DecryptRetrySettledResult::Timeout => {
            record_room_key_requester_stage(operation, "still_waiting", "none", elapsed);
        }
        _ => {}
    }
    record(
        decrypt_retry_event("settled", operation, elapsed)
            .field(DiagnosticField::token("result", result.token())),
    );
}

pub(super) fn decrypt_retry_backup_result_for_error(
    error: &koushi_sdk::E2eeTrustError,
) -> DecryptRetryBackupResult {
    match error {
        koushi_sdk::E2eeTrustError::Classified(kind) => match kind {
            koushi_sdk::E2eeTrustFailureKind::Network => DecryptRetryBackupResult::Network,
            koushi_sdk::E2eeTrustFailureKind::Forbidden => DecryptRetryBackupResult::Forbidden,
            koushi_sdk::E2eeTrustFailureKind::InvalidBackup => {
                DecryptRetryBackupResult::InvalidBackup
            }
            koushi_sdk::E2eeTrustFailureKind::Timeout => DecryptRetryBackupResult::Timeout,
            koushi_sdk::E2eeTrustFailureKind::Sdk => DecryptRetryBackupResult::Sdk,
        },
        koushi_sdk::E2eeTrustError::NoOlmMachine
        | koushi_sdk::E2eeTrustError::SecureBackupInspectionInconclusive
        | koushi_sdk::E2eeTrustError::SecureBackupAlreadyExists
        | koushi_sdk::E2eeTrustError::SecureBackupReenableConfirmationRequired
        | koushi_sdk::E2eeTrustError::SecureBackupUploadFailed
        | koushi_sdk::E2eeTrustError::SecureBackupRecoveryKeyDeliveryFailed
        | koushi_sdk::E2eeTrustError::Sdk(_) => DecryptRetryBackupResult::Sdk,
    }
}

pub(super) fn decrypt_retry_failure_for_room_operation(
    error: &koushi_sdk::MatrixRoomOperationError,
) -> DecryptRetryFailure {
    match error.failure_kind() {
        Some(koushi_sdk::MatrixRoomOperationFailureKind::Forbidden)
        | Some(koushi_sdk::MatrixRoomOperationFailureKind::AuthenticationRequired) => {
            DecryptRetryFailure::Forbidden
        }
        Some(koushi_sdk::MatrixRoomOperationFailureKind::Http) => DecryptRetryFailure::Network,
        _ => DecryptRetryFailure::Sdk,
    }
}

pub(super) fn record_read_completion(key: &ReadStateKey, diagnostic: ReadCompletionDiagnostic) {
    let (outcome, settled, candidates, waiters, failure_kind) = match diagnostic {
        ReadCompletionDiagnostic::Succeeded {
            settled_waiter_count,
            remaining_candidate_count,
            remaining_waiter_count,
        } => (
            "succeeded",
            settled_waiter_count,
            remaining_candidate_count,
            remaining_waiter_count,
            None,
        ),
        ReadCompletionDiagnostic::Failed {
            settled_waiter_count,
            remaining_candidate_count,
            remaining_waiter_count,
            failure_kind,
        } => (
            "failed",
            settled_waiter_count,
            remaining_candidate_count,
            remaining_waiter_count,
            Some(failure_kind),
        ),
        ReadCompletionDiagnostic::TimedOut {
            settled_waiter_count,
            remaining_candidate_count,
            remaining_waiter_count,
            failure_kind,
        } => (
            "timed_out",
            settled_waiter_count,
            remaining_candidate_count,
            remaining_waiter_count,
            Some(failure_kind),
        ),
        ReadCompletionDiagnostic::StaleDiscarded {
            remaining_candidate_count,
            remaining_waiter_count,
        } => (
            "stale_discarded",
            0,
            remaining_candidate_count,
            remaining_waiter_count,
            None,
        ),
    };
    record_read_state_diagnostic(
        "completion",
        key,
        outcome,
        candidates,
        waiters,
        settled,
        None,
        failure_kind,
    );
}

pub(super) fn record_read_retry_scheduled(
    key: &ReadStateKey,
    attempt: u32,
    queued_count: usize,
    active_count: usize,
    delay: std::time::Duration,
) {
    let delay_bucket = match delay.as_secs() {
        0 => "subsecond",
        1..=4 => "1_4s",
        5..=29 => "5_29s",
        30..=59 => "30_59s",
        _ => "ge_60s",
    };
    koushi_diagnostics::record(
        DiagnosticEvent::new(DiagnosticLevel::Error, "core.read_state", "retry_scheduled")
            .field(DiagnosticField::token("kind", read_state_kind_token(key)))
            .field(DiagnosticField::count("attempt", u64::from(attempt)))
            .field(DiagnosticField::count(
                "queued_count",
                queued_count.try_into().unwrap_or(u64::MAX),
            ))
            .field(DiagnosticField::count(
                "active_count",
                active_count.try_into().unwrap_or(u64::MAX),
            ))
            .field(DiagnosticField::token("delay_bucket", delay_bucket)),
    );
}

pub(super) fn record_read_retry(
    key: &ReadStateKey,
    source: ReadRetrySource,
    candidate_count: usize,
    waiter_count: usize,
) {
    record_read_state_diagnostic(
        "retry_wake",
        key,
        "woken",
        candidate_count,
        waiter_count,
        0,
        Some(source.token()),
        None,
    );
}

fn record_read_state_diagnostic(
    stage: &'static str,
    key: &ReadStateKey,
    outcome: &'static str,
    candidate_count: usize,
    waiter_count: usize,
    related_count: usize,
    source: Option<&'static str>,
    failure_kind: Option<koushi_protocol::failure::ReadStateFailureKind>,
) {
    let level = if failure_kind.is_some() {
        DiagnosticLevel::Error
    } else {
        DiagnosticLevel::Debug
    };
    let mut event = DiagnosticEvent::new(level, "core.read_state", stage)
        .field(DiagnosticField::token("kind", read_state_kind_token(key)))
        .field(DiagnosticField::token("outcome", outcome))
        .field(DiagnosticField::count(
            "candidate_count",
            candidate_count.try_into().unwrap_or(u64::MAX),
        ))
        .field(DiagnosticField::count(
            "waiter_count",
            waiter_count.try_into().unwrap_or(u64::MAX),
        ))
        .field(DiagnosticField::count(
            "related_count",
            related_count.try_into().unwrap_or(u64::MAX),
        ));
    if let Some(source) = source {
        event = event.field(DiagnosticField::token("source", source));
    }
    if let Some(failure_kind) = failure_kind {
        event = event.field(DiagnosticField::token("failure_kind", failure_kind.token()));
    }
    koushi_diagnostics::record(event);
}

pub(super) fn read_state_key_for_command(key: &TimelineKey, kind: ReadCommandKind) -> ReadStateKey {
    match (kind, &key.kind) {
        (
            ReadCommandKind::Receipt,
            TimelineKind::Thread {
                room_id,
                root_event_id,
            },
        ) => ReadStateKey::ThreadRead {
            room_id: room_id.clone(),
            root_event_id: root_event_id.clone(),
        },
        (ReadCommandKind::Receipt, TimelineKind::Room { room_id })
        | (ReadCommandKind::Receipt, TimelineKind::Focused { room_id, .. }) => {
            ReadStateKey::PublicUnthreaded {
                room_id: room_id.clone(),
            }
        }
        (ReadCommandKind::FullyRead, kind) => ReadStateKey::FullyReadAndPrivateUnthreaded {
            room_id: match kind {
                TimelineKind::Room { room_id }
                | TimelineKind::Thread { room_id, .. }
                | TimelineKind::Focused { room_id, .. } => room_id.clone(),
            },
        },
    }
}

pub(super) fn read_state_room_id(key: &ReadStateKey) -> &str {
    match key {
        ReadStateKey::PublicUnthreaded { room_id }
        | ReadStateKey::ThreadRead { room_id, .. }
        | ReadStateKey::FullyReadAndPrivateUnthreaded { room_id } => room_id,
    }
}

pub(super) fn timeline_key_matches_read_state_key(
    key: &TimelineKey,
    read_key: &ReadStateKey,
) -> bool {
    match (read_key, &key.kind) {
        (
            ReadStateKey::PublicUnthreaded { room_id: desired },
            TimelineKind::Room { room_id } | TimelineKind::Focused { room_id, .. },
        ) => room_id == desired,
        (
            ReadStateKey::ThreadRead {
                room_id: desired_room,
                root_event_id: desired_root,
            },
            TimelineKind::Thread {
                room_id,
                root_event_id,
            },
        ) => room_id == desired_room && root_event_id == desired_root,
        (
            ReadStateKey::FullyReadAndPrivateUnthreaded { room_id: desired },
            TimelineKind::Room { room_id }
            | TimelineKind::Thread { room_id, .. }
            | TimelineKind::Focused { room_id, .. },
        ) => room_id == desired,
        _ => false,
    }
}

pub(super) fn timeline_subscription_failed_action(key: &TimelineKey) -> Option<AppAction> {
    match &key.kind {
        TimelineKind::Room { .. } => None,
        TimelineKind::Thread {
            room_id,
            root_event_id,
        } => Some(AppAction::ThreadSubscriptionFailed {
            room_id: room_id.clone(),
            root_event_id: root_event_id.clone(),
            message: "timeline subscription failed".to_owned(),
        }),
        TimelineKind::Focused { room_id, event_id } => {
            Some(AppAction::FocusedContextSubscriptionFailed {
                room_id: room_id.clone(),
                event_id: event_id.clone(),
                message: "timeline subscription failed".to_owned(),
            })
        }
    }
}

pub(super) fn timeline_key_trace_kind(key: &TimelineKey) -> &'static str {
    match &key.kind {
        TimelineKind::Room { .. } => "room",
        TimelineKind::Thread { .. } => "thread",
        TimelineKind::Focused { .. } => "focused",
    }
}

fn timeline_stage_token(value: &str) -> &'static str {
    match value {
        "actor_start" => "actor_start",
        "actor_finish" => "actor_finish",
        "sdk_done" => "sdk_done",
        "target_scan" => "target_scan",
        "manager_received" => "manager_received",
        "sdk_finish" => "sdk_finish",
        "gate_acquired" => "gate_acquired",
        "start" => "start",
        "complete" => "complete",
        "lookup_miss" => "lookup_miss",
        "no_previews" => "no_previews",
        "cache_diff" => "cache_diff",
        "initial" => "initial",
        "cache_initial" => "cache_initial",
        "cache_update" => "cache_update",
        "diff_batch" => "diff_batch",
        "replay_initial" => "replay_initial",
        "send_queue_lagged_initial" => "send_queue_lagged_initial",
        "overflow_initial" => "overflow_initial",
        "initial_hydrate_gate_acquired" => "initial_hydrate_gate_acquired",
        "initial_hydrate_sdk_finish" => "initial_hydrate_sdk_finish",
        "actor_paginate_start" => "actor_paginate_start",
        "actor_paginate_skip" => "actor_paginate_skip",
        "cancelled" => "cancelled",
        "sync_started_existing_rooms" => "sync_started_existing_rooms",
        "replay_initial_skipped" => "replay_initial_skipped",
        "replay_initial_failed" => "replay_initial_failed",
        "subscribed_done" => "subscribed_done",
        "subscribe_rooms_begin" => "subscribe_rooms_begin",
        "subscribe_rooms_done" => "subscribe_rooms_done",
        "build_begin" => "build_begin",
        "build_done" => "build_done",
        "initial_backfill_projection_wait" => "initial_backfill_projection_wait",
        "initial_backfill_projection_closed" => "initial_backfill_projection_closed",
        "initial_backfill_projection_deadline" => "initial_backfill_projection_deadline",
        "initial_backfill_sdk_failed" => "initial_backfill_sdk_failed",

        "spawn_begin" => "spawn_begin",
        "spawn_done" => "spawn_done",
        "initial_emitted" => "initial_emitted",
        "replay_initial_emitted" => "replay_initial_emitted",
        "legacy_response_commit" => "legacy_response_commit",
        _ => "other",
    }
}

fn timeline_operation_token(value: &str) -> &'static str {
    match value {
        "send_reaction" => "send_reaction",
        "redact_reaction" => "redact_reaction",
        "send_read_receipt" => "send_read_receipt",
        "set_fully_read" => "set_fully_read",
        "paginate" => "paginate",
        "link_preview" => "link_preview",
        "subscribe" => "subscribe",
        "ensure_subscribed" => "ensure_subscribed",
        "unsubscribe" => "unsubscribe",
        "cancel_pagination" => "cancel_pagination",
        "cancel_link_previews" => "cancel_link_previews",
        "load_link_previews" => "load_link_previews",
        _ => "other",
    }
}

fn timeline_outcome_token(value: &str) -> &'static str {
    match value {
        "pending" => "pending",
        "success" => "success",
        "invalid_target" => "invalid_target",
        "target_missing" => "target_missing",
        "invalid_state" => "invalid_state",
        "sent" => "sent",
        "sdk_error" => "sdk_error",
        "cancelled" => "cancelled",
        "loaded" => "loaded",
        "missing" => "missing",
        "ready" => "ready",
        "failed" => "failed",
        "end_reached" => "end_reached",
        "idle" => "idle",
        "in_flight" => "in_flight",
        "invalid_event" => "invalid_event",
        "invalid_private_receipt" => "invalid_private_receipt",
        "invalid_thread_root" => "invalid_thread_root",
        "redacted" => "redacted",
        "unchanged" => "unchanged",
        "discarded" => "discarded",
        "updated" => "updated",
        "lookup_miss" => "lookup_miss",
        "no_previews" => "no_previews",
        _ => "other",
    }
}

fn timeline_diff_token(value: &str) -> &'static str {
    match value {
        "push_front" => "push_front",
        "push_back" => "push_back",
        "insert" => "insert",
        "set" => "set",
        "append" => "append",
        "append_item" => "append_item",
        "reset" => "reset",
        "reset_item" => "reset_item",
        "remove" => "remove",
        "truncate" => "truncate",
        "clear" => "clear",
        "pop_front" => "pop_front",
        "pop_back" => "pop_back",
        "item" => "item",
        _ => "other",
    }
}

fn record_timeline_event(stage: &str, kind: &str, fields: Vec<DiagnosticField>) {
    let mut event = DiagnosticEvent::new(
        DiagnosticLevel::Debug,
        "core.timeline",
        timeline_stage_token(stage),
    )
    .field(DiagnosticField::token(
        "kind",
        timeline_operation_token(kind),
    ));
    for field in fields {
        event = event.field(field);
    }
    koushi_diagnostics::record(event);
}

pub(super) fn record_subscribe_stage(stage: &str, count: Option<usize>) {
    let mut fields = Vec::new();
    if let Some(count) = count {
        fields.push(DiagnosticField::count("count", count as u64));
    }
    #[cfg(not(any(test, feature = "test-hooks")))]
    record_timeline_event(stage, "subscribe", fields);
    #[cfg(any(test, feature = "test-hooks"))]
    {
        let mut event = DiagnosticEvent::new(
            DiagnosticLevel::Debug,
            "core.timeline",
            timeline_stage_token(stage),
        )
        .field(DiagnosticField::token("kind", "subscribe"));
        for field in fields {
            event = event.field(field);
        }
        koushi_diagnostics::record_and_stderr(event);
    }
}

/// Record one closed-token subscription reconciliation (issue #518). Counts
/// and the no-op flag are bucketed/counted as aggregates that survive
/// detail-ring eviction; no room identifiers are exported.
pub(super) fn record_subscription_room_coverage(
    room_ordinal: u64,
    key: &'static str,
    token: &'static str,
) {
    koushi_diagnostics::record(
        DiagnosticEvent::new(DiagnosticLevel::Info, "core.subscription", "room")
            .field(DiagnosticField::ordinal_alias(
                "room_alias",
                "room",
                room_ordinal,
            ))
            .field(DiagnosticField::token(key, token)),
    );
}

pub(super) fn subscription_count_bucket(count: usize) -> u64 {
    match count {
        0 => 0,
        1 => 1,
        2..=5 => 2,
        6..=20 => 3,
        _ => 4,
    }
}

pub(super) fn record_residency_intent(
    source: &'static str,
    outcome: &'static str,
    accepted: usize,
    rejected: usize,
) {
    record(
        DiagnosticEvent::new(DiagnosticLevel::Info, "core.subscription", "intent")
            .field(DiagnosticField::token("source", source))
            .field(DiagnosticField::token("outcome", outcome))
            .field(DiagnosticField::count(
                "accepted_bucket",
                subscription_count_bucket(accepted),
            ))
            .field(DiagnosticField::count(
                "rejected_bucket",
                subscription_count_bucket(rejected),
            )),
    );
}

pub(super) fn record_subscription_reconcile(
    trigger_token: &'static str,
    previous_active_count: usize,
    desired_count: usize,
    generation_before: u64,
    result: &matrix_sdk_ui::room_list_service::RoomSubscriptionReconcile,
) {
    koushi_diagnostics::increment_counter(if result.noop {
        "subscription_reconcile_noop"
    } else {
        "subscription_reconcile_changed"
    });
    match trigger_token {
        "opened" => koushi_diagnostics::increment_counter("subscription_reconcile_trigger_opened"),
        "visible_range" => {
            koushi_diagnostics::increment_counter("subscription_reconcile_trigger_visible_range")
        }
        "restore" => {
            koushi_diagnostics::increment_counter("subscription_reconcile_trigger_restore")
        }
        "room_left" => {
            koushi_diagnostics::increment_counter("subscription_reconcile_trigger_room_left")
        }
        "room_rejoined" => {
            koushi_diagnostics::increment_counter("subscription_reconcile_trigger_room_rejoined")
        }
        "membership" => {
            koushi_diagnostics::increment_counter("subscription_reconcile_trigger_membership")
        }
        "session_restart" => {
            koushi_diagnostics::increment_counter("subscription_reconcile_trigger_session_restart")
        }
        _ => {}
    }
    if !result.noop {
        koushi_diagnostics::increment_counter("subscription_reconcile_added");
        koushi_diagnostics::increment_counter("subscription_reconcile_removed");
        koushi_diagnostics::increment_counter("subscription_reconcile_retained");
    }
    let event = DiagnosticEvent::new(DiagnosticLevel::Info, "core.subscription", "reconcile")
        .field(DiagnosticField::token("trigger", trigger_token))
        .field(DiagnosticField::boolean("exact_set_noop", result.noop))
        .field(DiagnosticField::count(
            "previous_bucket",
            subscription_count_bucket(previous_active_count),
        ))
        .field(DiagnosticField::count(
            "desired_bucket",
            subscription_count_bucket(desired_count),
        ))
        .field(DiagnosticField::count(
            "added_bucket",
            subscription_count_bucket(result.added),
        ))
        .field(DiagnosticField::count(
            "removed_bucket",
            subscription_count_bucket(result.removed),
        ))
        .field(DiagnosticField::count(
            "retained_bucket",
            subscription_count_bucket(result.retained),
        ))
        .field(DiagnosticField::count(
            "generation_before",
            generation_before,
        ))
        .field(DiagnosticField::count(
            "generation_after",
            result.generation.get(),
        ))
        .field(DiagnosticField::boolean(
            "checkpoints_retained",
            result.checkpoints_retained,
        ));
    koushi_diagnostics::record(event);
}

pub(crate) fn record_thread_summary_reconciliation(
    (room_ordinal, root_ordinal): (u64, u64),
    source: &'static str,
    identity_relation: &'static str,
    decision: &'static str,
    merge_reason: &'static str,
    count_before: u32,
    count_after: u32,
    dto_changed: bool,
) {
    record(
        DiagnosticEvent::new(DiagnosticLevel::Info, "core.thread_summary", "reconciled")
            .field(DiagnosticField::ordinal_alias(
                "room_ordinal",
                "room",
                room_ordinal,
            ))
            .field(DiagnosticField::ordinal_alias(
                "root_ordinal",
                "root",
                root_ordinal,
            ))
            .field(DiagnosticField::token("source", source))
            .field(DiagnosticField::token(
                "identity_relation",
                identity_relation,
            ))
            .field(DiagnosticField::token("decision", decision))
            .field(DiagnosticField::token("merge_reason", merge_reason))
            .field(DiagnosticField::count(
                "count_before",
                u64::from(count_before),
            ))
            .field(DiagnosticField::count(
                "count_after",
                u64::from(count_after),
            ))
            .field(DiagnosticField::boolean("dto_changed", dto_changed)),
    );
}

pub(super) fn record_thread_projection(
    key: &TimelineKey,
    actor_generation: u64,
    timeline_generation: TimelineGeneration,
    batch_id: TimelineBatchId,
    input_diff_count: usize,
    projected_diff_count: usize,
    projected_item_count: usize,
) {
    if !matches!(key.kind, TimelineKind::Thread { .. }) {
        return;
    }
    koushi_diagnostics::record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "core.thread_timeline", "projected")
            .field(DiagnosticField::count("actor_generation", actor_generation))
            .field(DiagnosticField::count(
                "timeline_generation",
                timeline_generation.0,
            ))
            .field(DiagnosticField::count("batch_id", batch_id.0))
            .field(DiagnosticField::count(
                "input_diffs",
                input_diff_count as u64,
            ))
            .field(DiagnosticField::count(
                "projected_diffs",
                projected_diff_count as u64,
            ))
            .field(DiagnosticField::count("items", projected_item_count as u64)),
    );
}

pub(super) fn record_timeline_gap_repair(
    stage: &'static str,
    trigger: &'static str,
    generation: u64,
    gap_count: u32,
    batches_processed: u32,
    outcome: &'static str,
) {
    koushi_diagnostics::record_and_stderr(
        DiagnosticEvent::new(DiagnosticLevel::Info, "core.timeline_gap_repair", stage)
            .field(DiagnosticField::token("trigger", trigger))
            .field(DiagnosticField::count("generation", generation))
            .field(DiagnosticField::count("gap_count", gap_count.into()))
            .field(DiagnosticField::count(
                "batches_processed",
                batches_processed.into(),
            ))
            .field(DiagnosticField::token("outcome", outcome)),
    );
}

fn live_tail_refresh_outcome_token(outcome: LiveTailRefreshOutcome) -> &'static str {
    match outcome {
        LiveTailRefreshOutcome::Cancelled => "cancelled",
        LiveTailRefreshOutcome::Unchanged => "unchanged",
        LiveTailRefreshOutcome::Advanced { .. } => "advanced",
        LiveTailRefreshOutcome::Detached { .. } => "detached",
        LiveTailRefreshOutcome::Stale => "stale",
        LiveTailRefreshOutcome::Failed => "failed",
    }
}

fn live_tail_freshness_token(state: Option<LiveTailFreshnessState>) -> &'static str {
    match state {
        None => "none",
        Some(LiveTailFreshnessState::Unproven { .. }) => "unproven",
        Some(LiveTailFreshnessState::Refreshing { .. }) => "refreshing",
        Some(LiveTailFreshnessState::Fresh { .. }) => "fresh",
        Some(LiveTailFreshnessState::Deferred { .. }) => "deferred",
        Some(LiveTailFreshnessState::Retryable { .. }) => "retryable",
    }
}

pub(super) fn record_live_tail_state(
    from: Option<LiveTailFreshnessState>,
    to: Option<LiveTailFreshnessState>,
    sync_epoch: u64,
) {
    if from == to {
        return;
    }
    koushi_diagnostics::record(
        DiagnosticEvent::new(
            DiagnosticLevel::Info,
            "core.timeline",
            "timeline_live_tail_state",
        )
        .field(DiagnosticField::token(
            "from",
            live_tail_freshness_token(from),
        ))
        .field(DiagnosticField::token("to", live_tail_freshness_token(to)))
        .field(DiagnosticField::count("sync_epoch", sync_epoch)),
    );
}

pub(super) fn record_live_tail_queue(
    priority: &'static str,
    actions: &[LiveTailSchedulerAction<TimelineKey>],
) {
    if actions.is_empty() {
        return;
    }
    let queue_depth = actions
        .iter()
        .filter(|action| matches!(action, LiveTailSchedulerAction::Start { .. }))
        .count();
    let preempted = actions
        .iter()
        .any(|action| matches!(action, LiveTailSchedulerAction::CancelNetwork { .. }));
    koushi_diagnostics::record(
        DiagnosticEvent::new(
            DiagnosticLevel::Info,
            "core.timeline",
            "timeline_live_tail_queue",
        )
        .field(DiagnosticField::token("priority", priority))
        .field(DiagnosticField::count(
            "queue_depth",
            queue_depth.try_into().unwrap_or(u64::MAX),
        ))
        .field(DiagnosticField::boolean("preempted", preempted)),
    );
}

pub(super) fn record_live_tail_cancellation(
    outcome: &'static str,
    operation_generation: u64,
    duration_ms: u128,
) {
    koushi_diagnostics::record(
        DiagnosticEvent::new(
            DiagnosticLevel::Info,
            "core.timeline",
            "timeline_live_tail_cancellation",
        )
        .field(DiagnosticField::token("outcome", outcome))
        .field(DiagnosticField::count(
            "operation_generation",
            operation_generation,
        ))
        .field(DiagnosticField::milliseconds("duration_ms", duration_ms)),
    );
}

pub(super) fn record_live_tail_refresh(
    outcome: LiveTailRefreshOutcome,
    requested_limit: u16,
    returned_events: usize,
    historical_gap_remaining: bool,
    operation_generation: u64,
    duration_ms: u128,
) {
    koushi_diagnostics::record(
        DiagnosticEvent::new(
            DiagnosticLevel::Info,
            "core.timeline",
            "timeline_live_tail_refresh",
        )
        .field(DiagnosticField::token(
            "outcome",
            live_tail_refresh_outcome_token(outcome),
        ))
        .field(DiagnosticField::count(
            "requested_limit",
            requested_limit.into(),
        ))
        .field(DiagnosticField::count(
            "returned_events",
            returned_events.try_into().unwrap_or(u64::MAX),
        ))
        .field(DiagnosticField::boolean(
            "historical_gap_remaining",
            historical_gap_remaining,
        ))
        .field(DiagnosticField::count(
            "operation_generation",
            operation_generation,
        ))
        .field(DiagnosticField::milliseconds("duration_ms", duration_ms)),
    );
}

pub(super) fn record_live_tail_reconciliation(
    diagnostics: MatrixLiveTailRefreshDiagnostics,
    operation_generation: u64,
) {
    let optional_index = |key, value: Option<usize>| {
        DiagnosticField::count(
            key,
            value
                .and_then(|index| u64::try_from(index).ok())
                .unwrap_or(u64::MAX),
        )
    };
    koushi_diagnostics::record(
        DiagnosticEvent::new(
            DiagnosticLevel::Info,
            "core.timeline",
            "timeline_live_tail_reconciliation",
        )
        .field(DiagnosticField::count(
            "cached_suffix_events",
            diagnostics
                .cached_suffix_events
                .try_into()
                .unwrap_or(u64::MAX),
        ))
        .field(DiagnosticField::count(
            "response_events_with_ids",
            diagnostics
                .response_events_with_ids
                .try_into()
                .unwrap_or(u64::MAX),
        ))
        .field(optional_index(
            "newest_cached_response_index",
            diagnostics.newest_cached_response_index,
        ))
        .field(optional_index(
            "older_anchor_response_index",
            diagnostics.older_anchor_response_index,
        ))
        .field(DiagnosticField::count(
            "in_memory_duplicates",
            diagnostics
                .in_memory_duplicates
                .try_into()
                .unwrap_or(u64::MAX),
        ))
        .field(DiagnosticField::count(
            "in_store_duplicates",
            diagnostics
                .in_store_duplicates
                .try_into()
                .unwrap_or(u64::MAX),
        ))
        .field(DiagnosticField::count(
            "new_events",
            diagnostics.new_events.try_into().unwrap_or(u64::MAX),
        ))
        .field(DiagnosticField::count(
            "operation_generation",
            operation_generation,
        )),
    );
}

pub(super) fn record_live_tail_commit(phase: &'static str, operation_generation: u64) {
    koushi_diagnostics::record(
        DiagnosticEvent::new(
            DiagnosticLevel::Debug,
            "core.timeline",
            "timeline_live_tail_commit",
        )
        .field(DiagnosticField::token("phase", phase))
        .field(DiagnosticField::count(
            "operation_generation",
            operation_generation,
        )),
    );
}

pub(super) fn record_live_catchup_gate(
    gate: LiveCatchupGate,
    expected_generation: Option<u64>,
    checkpoint: Option<&MatrixRoomSubscriptionCheckpoint>,
    scheduler_phase: &'static str,
    batches_processed: u32,
) {
    let checkpoint_origin = checkpoint.map_or("none", |_| "room_update");
    let candidate = match gate {
        LiveCatchupGate::RepairCheckpointGap => "exact_response_gap",
        LiveCatchupGate::AwaitingCheckpoint
        | LiveCatchupGate::Stale
        | LiveCatchupGate::NoTimelineUpdate
        | LiveCatchupGate::NoGap => "none",
        LiveCatchupGate::InspectCommittedLiveEdge => "global_commit",
    };
    koushi_diagnostics::record(
        DiagnosticEvent::new(DiagnosticLevel::Info, "core.live_catchup", "checkpoint")
            .field(DiagnosticField::token("decision", gate.token()))
            .field(DiagnosticField::token(
                "checkpoint_origin",
                checkpoint_origin,
            ))
            .field(DiagnosticField::token("candidate", candidate))
            .field(DiagnosticField::token("scheduler_phase", scheduler_phase))
            .field(DiagnosticField::count(
                "batches_processed",
                batches_processed.into(),
            ))
            .field(DiagnosticField::boolean("supported_backend", true))
            .field(DiagnosticField::count(
                "subscription_generation",
                expected_generation.unwrap_or_default(),
            ))
            .field(DiagnosticField::count(
                "checkpoint_generation",
                checkpoint.map_or(0, MatrixRoomSubscriptionCheckpoint::generation),
            ))
            .field(DiagnosticField::boolean(
                "timeline_update",
                checkpoint.is_some_and(|checkpoint| checkpoint.has_timeline_update()),
            ))
            .field(DiagnosticField::boolean(
                "checkpoint_gap",
                checkpoint.is_some_and(|checkpoint| checkpoint.has_inserted_gap()),
            ))
            .field(DiagnosticField::count("event_count", 0)),
    );
}

pub(super) fn record_timeline_gap_repair_evaluation(
    decision: &'static str,
    projected_gap_count: usize,
    visible_gap_count: usize,
    visible_gap_validated: bool,
    candidate_changed: bool,
    scheduler_phase: &'static str,
) {
    koushi_diagnostics::record(
        DiagnosticEvent::new(
            DiagnosticLevel::Debug,
            "core.timeline_gap_repair",
            "evaluation",
        )
        .field(DiagnosticField::token("trigger", "viewport"))
        .field(DiagnosticField::token("decision", decision))
        .field(DiagnosticField::count(
            "projected_gap_count",
            projected_gap_count.try_into().unwrap_or(u64::MAX),
        ))
        .field(DiagnosticField::count(
            "visible_gap_count",
            visible_gap_count.try_into().unwrap_or(u64::MAX),
        ))
        .field(DiagnosticField::boolean(
            "visible_gap_validated",
            visible_gap_validated,
        ))
        .field(DiagnosticField::boolean(
            "candidate_changed",
            candidate_changed,
        ))
        .field(DiagnosticField::token("scheduler_phase", scheduler_phase)),
    );
}

pub(super) fn record_timeline_gap_projection(
    gap_count: usize,
    counts: GapBoundaryPresenceCounts,
    navigation_event_count: usize,
    foreground_demand_active: bool,
    foreground_demand_epoch: u64,
    scheduler_phase: &'static str,
) {
    koushi_diagnostics::record(
        DiagnosticEvent::new(
            DiagnosticLevel::Info,
            "core.timeline_gap_projection",
            "inspection",
        )
        .field(DiagnosticField::count(
            "gap_count",
            gap_count.try_into().unwrap_or(u64::MAX),
        ))
        .field(DiagnosticField::count(
            "projected_count",
            counts.projected.try_into().unwrap_or(u64::MAX),
        ))
        .field(DiagnosticField::count(
            "boundary_both_count",
            counts.both.try_into().unwrap_or(u64::MAX),
        ))
        .field(DiagnosticField::count(
            "boundary_one_count",
            counts.one.try_into().unwrap_or(u64::MAX),
        ))
        .field(DiagnosticField::count(
            "boundary_none_count",
            counts.none.try_into().unwrap_or(u64::MAX),
        ))
        .field(DiagnosticField::count(
            "navigation_event_count",
            navigation_event_count.try_into().unwrap_or(u64::MAX),
        ))
        .field(DiagnosticField::boolean(
            "foreground_demand_active",
            foreground_demand_active,
        ))
        .field(DiagnosticField::count(
            "foreground_demand_epoch",
            foreground_demand_epoch,
        ))
        .field(DiagnosticField::token("scheduler_phase", scheduler_phase)),
    );
}

pub(super) fn record_timeline_gap_projection_boundary(
    stage: &'static str,
    outcome: &'static str,
    actor_generation: u64,
    timeline_generation: TimelineGeneration,
    operation: CausalProjectionOperationId,
    projection_batch: Option<u32>,
    timeline_batch_id: Option<TimelineBatchId>,
    expected_projection_batch: Option<u32>,
    observed_projection_count: usize,
) {
    let domain = match operation.domain {
        CausalProjectionDomain::HistoricalGap => "historical_gap",
        CausalProjectionDomain::LiveTail => "live_tail",
    };
    koushi_diagnostics::record_and_stderr(
        DiagnosticEvent::new(DiagnosticLevel::Info, "core.timeline_gap_projection", stage)
            .field(DiagnosticField::token("outcome", outcome))
            .field(DiagnosticField::token("domain", domain))
            .field(DiagnosticField::count("actor_generation", actor_generation))
            .field(DiagnosticField::count(
                "timeline_generation",
                timeline_generation.0,
            ))
            .field(DiagnosticField::count(
                "operation_generation",
                operation.serial,
            ))
            .field(DiagnosticField::count(
                "projection_batch",
                projection_batch.map_or(u64::MAX, u64::from),
            ))
            .field(DiagnosticField::count(
                "timeline_batch_id",
                timeline_batch_id.map_or(u64::MAX, |batch_id| batch_id.0),
            ))
            .field(DiagnosticField::count(
                "expected_projection_batch",
                expected_projection_batch.map_or(u64::MAX, u64::from),
            ))
            .field(DiagnosticField::count(
                "observed_projection_count",
                observed_projection_count.try_into().unwrap_or(u64::MAX),
            )),
    );
}

pub(super) fn record_timeline_gap_demand(
    foreground_demand_epoch: u64,
    projected_gap_count: usize,
    visible_gap_count: usize,
    inspection_requested: bool,
    reason: &'static str,
    scheduler_phase: &'static str,
) {
    koushi_diagnostics::record(
        DiagnosticEvent::new(
            DiagnosticLevel::Info,
            "core.timeline_gap_demand",
            "activate",
        )
        .field(DiagnosticField::count(
            "foreground_demand_epoch",
            foreground_demand_epoch,
        ))
        .field(DiagnosticField::boolean("foreground_demand_active", true))
        .field(DiagnosticField::count(
            "projected_gap_count",
            projected_gap_count.try_into().unwrap_or(u64::MAX),
        ))
        .field(DiagnosticField::count(
            "visible_gap_count",
            visible_gap_count.try_into().unwrap_or(u64::MAX),
        ))
        .field(DiagnosticField::boolean(
            "inspection_requested",
            inspection_requested,
        ))
        .field(DiagnosticField::token("reason", reason))
        .field(DiagnosticField::token("scheduler_phase", scheduler_phase)),
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TimelineGapSelectionDiagnostic {
    pub(super) trigger: &'static str,
    pub(super) decision: &'static str,
    pub(super) repair_started: bool,
    pub(super) gap_count: usize,
    pub(super) projected_gap_count: usize,
    pub(super) visible_gap_count: usize,
    pub(super) foreground_demand_active: bool,
    pub(super) foreground_demand_epoch: u64,
    pub(super) has_live_edge_target: bool,
    pub(super) scheduler_phase: &'static str,
}

pub(super) fn record_timeline_gap_selection(diagnostic: TimelineGapSelectionDiagnostic) {
    koushi_diagnostics::record(
        DiagnosticEvent::new(
            DiagnosticLevel::Info,
            "core.timeline_gap_selection",
            "evaluation",
        )
        .field(DiagnosticField::token("trigger", diagnostic.trigger))
        .field(DiagnosticField::token("decision", diagnostic.decision))
        .field(DiagnosticField::boolean(
            "repair_started",
            diagnostic.repair_started,
        ))
        .field(DiagnosticField::count(
            "gap_count",
            diagnostic.gap_count.try_into().unwrap_or(u64::MAX),
        ))
        .field(DiagnosticField::count(
            "projected_gap_count",
            diagnostic
                .projected_gap_count
                .try_into()
                .unwrap_or(u64::MAX),
        ))
        .field(DiagnosticField::count(
            "visible_gap_count",
            diagnostic.visible_gap_count.try_into().unwrap_or(u64::MAX),
        ))
        .field(DiagnosticField::boolean(
            "foreground_demand_active",
            diagnostic.foreground_demand_active,
        ))
        .field(DiagnosticField::count(
            "foreground_demand_epoch",
            diagnostic.foreground_demand_epoch,
        ))
        .field(DiagnosticField::boolean(
            "has_live_edge_target",
            diagnostic.has_live_edge_target,
        ))
        .field(DiagnosticField::token(
            "scheduler_phase",
            diagnostic.scheduler_phase,
        )),
    );
}

pub(super) fn trace_timeline_actor_operation(
    stage: &str,
    kind: &str,
    request_id: RequestId,
    key: &TimelineKey,
    elapsed_ms: Option<u128>,
    outcome: Option<&str>,
) {
    record_timeline_event(
        stage,
        kind,
        vec![
            DiagnosticField::token("timeline", timeline_key_trace_kind(key)),
            DiagnosticField::request_id(
                "request_id",
                request_id.connection_id.0,
                request_id.sequence,
            ),
            DiagnosticField::milliseconds("duration", elapsed_ms.unwrap_or(0)),
            DiagnosticField::token(
                "outcome",
                outcome.map(timeline_outcome_token).unwrap_or("pending"),
            ),
        ],
    );
}

pub(super) fn trace_timeline_actor_scan(
    stage: &str,
    kind: &str,
    request_id: RequestId,
    key: &TimelineKey,
    item_count: usize,
    elapsed_ms: u128,
    found: bool,
) {
    record_timeline_event(
        stage,
        kind,
        vec![
            DiagnosticField::token("timeline", timeline_key_trace_kind(key)),
            DiagnosticField::request_id(
                "request_id",
                request_id.connection_id.0,
                request_id.sequence,
            ),
            DiagnosticField::count("count", item_count as u64),
            DiagnosticField::milliseconds("duration", elapsed_ms),
            DiagnosticField::boolean("found", found),
        ],
    );
}

fn timeline_item_id_for_trace(item: &TimelineItem) -> (&'static str, Option<&str>) {
    match &item.id {
        TimelineItemId::Event { event_id } => ("event", Some(event_id.as_str())),
        TimelineItemId::Transaction { transaction_id } => {
            ("transaction", Some(transaction_id.as_str()))
        }
        TimelineItemId::Synthetic { synthetic_id } => ("synthetic", Some(synthetic_id.as_str())),
    }
}

pub(super) fn timeline_item_diagnostic_event(
    stage: &str,
    key: &TimelineKey,
    op: &str,
    index: Option<usize>,
    item: &TimelineItem,
) -> DiagnosticEvent {
    let (id_kind, _) = timeline_item_id_for_trace(item);
    let sender_present = item
        .sender
        .as_deref()
        .is_some_and(|sender| !sender.trim().is_empty());
    let thread_root_present = item
        .thread_root
        .as_deref()
        .is_some_and(|thread_root| !thread_root.trim().is_empty());
    let reply_present = item
        .in_reply_to_event_id
        .as_deref()
        .is_some_and(|event_id| !event_id.trim().is_empty());
    let body_present = item
        .body
        .as_deref()
        .is_some_and(|body| !body.trim().is_empty());
    let formatted_present = item
        .formatted
        .as_ref()
        .is_some_and(timeline_formatted_body_is_renderable);

    DiagnosticEvent::new(
        DiagnosticLevel::Debug,
        "core.timeline_item",
        timeline_stage_token(stage),
    )
    .field(DiagnosticField::token("kind", timeline_diff_token(op)))
    .field(DiagnosticField::token(
        "timeline",
        timeline_key_trace_kind(key),
    ))
    .field(DiagnosticField::token("id_kind", id_kind))
    .field(DiagnosticField::count("count", 1))
    .field(DiagnosticField::count("index", index.unwrap_or(0) as u64))
    .field(DiagnosticField::boolean("index_present", index.is_some()))
    .field(DiagnosticField::count(
        "timestamp_minute",
        item.timestamp_ms.unwrap_or(0) / 60_000,
    ))
    .field(DiagnosticField::boolean(
        "timestamp_present",
        item.timestamp_ms.is_some(),
    ))
    .field(DiagnosticField::boolean("sender_present", sender_present))
    .field(DiagnosticField::boolean("hidden", item.is_hidden))
    .field(DiagnosticField::boolean(
        "thread_root_present",
        thread_root_present,
    ))
    .field(DiagnosticField::boolean("reply_present", reply_present))
    .field(DiagnosticField::boolean("body_present", body_present))
    .field(DiagnosticField::boolean(
        "formatted_present",
        formatted_present,
    ))
    .field(DiagnosticField::boolean(
        "media_present",
        item.media.is_some(),
    ))
    .field(DiagnosticField::boolean("redacted", item.is_redacted))
    .field(DiagnosticField::boolean(
        "unable_to_decrypt",
        item.unable_to_decrypt.is_some(),
    ))
    .field(DiagnosticField::boolean(
        "send_state_present",
        item.send_state.is_some(),
    ))
}

#[allow(dead_code)]
fn record_timeline_item(
    stage: &str,
    key: &TimelineKey,
    op: &str,
    index: Option<usize>,
    item: &TimelineItem,
) {
    koushi_diagnostics::record(timeline_item_diagnostic_event(stage, key, op, index, item));
}

pub(super) fn trace_timeline_items(stage: &str, key: &TimelineKey, items: &[TimelineItem]) {
    let hidden = items.iter().filter(|item| item.is_hidden).count();
    let mut events = Vec::with_capacity(1);
    events.push(
        DiagnosticEvent::new(
            DiagnosticLevel::Debug,
            "core.timeline_item",
            timeline_stage_token(stage),
        )
        .field(DiagnosticField::token("kind", "batch"))
        .field(DiagnosticField::token(
            "timeline",
            timeline_key_trace_kind(key),
        ))
        .field(DiagnosticField::count("count", items.len() as u64))
        .field(DiagnosticField::count("hidden", hidden as u64)),
    );
    koushi_diagnostics::record_batch(events);
}

pub(super) fn trace_timeline_diffs(stage: &str, key: &TimelineKey, diffs: &[TimelineDiff]) {
    koushi_diagnostics::record(timeline_diff_batch_diagnostic_event(stage, key, diffs));
}

fn timeline_diff_batch_diagnostic_event(
    stage: &str,
    key: &TimelineKey,
    diffs: &[TimelineDiff],
) -> DiagnosticEvent {
    let mut push_front_count = 0_u64;
    let mut push_back_count = 0_u64;
    let mut insert_count = 0_u64;
    let mut set_count = 0_u64;
    let mut remove_count = 0_u64;
    let mut truncate_count = 0_u64;
    let mut clear_count = 0_u64;
    let mut reset_count = 0_u64;
    let mut reset_item_count = 0_u64;
    for diff in diffs {
        match diff {
            TimelineDiff::PushFront { .. } => push_front_count += 1,
            TimelineDiff::PushBack { .. } => push_back_count += 1,
            TimelineDiff::Insert { .. } => insert_count += 1,
            TimelineDiff::Set { .. } => set_count += 1,
            TimelineDiff::Remove { .. } => remove_count += 1,
            TimelineDiff::Truncate { .. } => truncate_count += 1,
            TimelineDiff::Clear => clear_count += 1,
            TimelineDiff::Reset { items } => {
                reset_count += 1;
                reset_item_count = reset_item_count.saturating_add(items.len() as u64);
            }
        }
    }
    DiagnosticEvent::new(
        DiagnosticLevel::Debug,
        "core.timeline_item",
        timeline_stage_token(stage),
    )
    .field(DiagnosticField::token("kind", "batch"))
    .field(DiagnosticField::token(
        "timeline",
        timeline_key_trace_kind(key),
    ))
    .field(DiagnosticField::count("count", diffs.len() as u64))
    .field(DiagnosticField::count("push_front_count", push_front_count))
    .field(DiagnosticField::count("push_back_count", push_back_count))
    .field(DiagnosticField::count("insert_count", insert_count))
    .field(DiagnosticField::count("set_count", set_count))
    .field(DiagnosticField::count("remove_count", remove_count))
    .field(DiagnosticField::count("truncate_count", truncate_count))
    .field(DiagnosticField::count("clear_count", clear_count))
    .field(DiagnosticField::count("reset_count", reset_count))
    .field(DiagnosticField::count("reset_item_count", reset_item_count))
}

#[derive(Default)]
struct EventCacheRelationTrace {
    relates_to_present: bool,
    rel_type: &'static str,
    relation_event_id: Option<String>,
    reply_event_id: Option<String>,
    thread_root_event_id: Option<String>,
}

fn event_cache_relation_trace(
    event: &matrix_sdk_base::event_cache::Event,
) -> EventCacheRelationTrace {
    let content = event
        .raw()
        .get_field::<serde_json::Value>("content")
        .ok()
        .flatten();
    let Some(relates_to) = content
        .as_ref()
        .and_then(|content| content.get("m.relates_to"))
    else {
        return EventCacheRelationTrace {
            rel_type: "none",
            ..EventCacheRelationTrace::default()
        };
    };

    let rel_type_raw = relates_to
        .get("rel_type")
        .and_then(serde_json::Value::as_str);
    let relation_event_id = relates_to
        .get("event_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let reply_event_id = relates_to
        .get("m.in_reply_to")
        .and_then(|reply| reply.get("event_id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let thread_root_event_id = matches!(rel_type_raw, Some("m.thread"))
        .then(|| relation_event_id.clone())
        .flatten();

    EventCacheRelationTrace {
        relates_to_present: true,
        rel_type: relation_type_trace_token(rel_type_raw),
        relation_event_id,
        reply_event_id,
        thread_root_event_id,
    }
}

fn relation_type_trace_token(rel_type: Option<&str>) -> &'static str {
    match rel_type {
        Some("m.thread") => "m.thread",
        Some("m.replace") => "m.replace",
        Some("m.annotation") => "m.annotation",
        Some("m.reference") => "m.reference",
        Some(_) => "other",
        None => "none",
    }
}

#[derive(Clone, Copy)]
pub(super) struct FullyReadReceiptContext<'a> {
    pub(super) visible_event_id: &'a str,
    pub(super) latest_event_id: Option<&'a str>,
    pub(super) latest_event_relation_type: Option<&'a str>,
    pub(super) unread_messages: u64,
    pub(super) notification_count: u64,
}

struct RoomLatestReceiptContext {
    event_id: Option<String>,
    relation_type: Option<String>,
    unread_messages: u64,
    notification_count: u64,
}

pub(super) fn private_read_receipt_event_id_for_fully_read<'a>(
    context: FullyReadReceiptContext<'a>,
) -> &'a str {
    if context.unread_messages == 0
        && context.notification_count > 0
        && context.latest_event_relation_type == Some("m.replace")
        && let Some(latest_event_id) = context.latest_event_id
        && !latest_event_id.trim().is_empty()
    {
        latest_event_id
    } else {
        context.visible_event_id
    }
}

pub(super) fn private_read_receipt_event_id_from_room_for_fully_read(
    room: &matrix_sdk::Room,
    visible_event_id: &str,
) -> String {
    let latest = room_latest_receipt_context(room);
    private_read_receipt_event_id_for_fully_read(FullyReadReceiptContext {
        visible_event_id,
        latest_event_id: latest.event_id.as_deref(),
        latest_event_relation_type: latest.relation_type.as_deref(),
        unread_messages: latest.unread_messages,
        notification_count: latest.notification_count,
    })
    .to_owned()
}

fn room_latest_receipt_context(room: &matrix_sdk::Room) -> RoomLatestReceiptContext {
    let (event_id, relation_type) = match room.latest_event() {
        matrix_sdk::latest_events::LatestEventValue::Remote(timeline_event) => (
            timeline_event
                .event_id()
                .map(|event_id| event_id.to_string()),
            timeline_event_relation_type(&timeline_event),
        ),
        _ => (None, None),
    };

    RoomLatestReceiptContext {
        event_id,
        relation_type,
        unread_messages: room.num_unread_messages(),
        notification_count: room.num_unread_notifications(),
    }
}

fn timeline_event_relation_type(
    timeline_event: &matrix_sdk::deserialized_responses::TimelineEvent,
) -> Option<String> {
    let content = timeline_event
        .raw()
        .get_field::<serde_json::Value>("content")
        .ok()
        .flatten()?;
    content
        .get("m.relates_to")?
        .get("rel_type")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

fn event_cache_item_diagnostic_event(
    stage: &str,
    key: &TimelineKey,
    op: &str,
    index: Option<usize>,
    item: &matrix_sdk_base::event_cache::Event,
) -> DiagnosticEvent {
    let event_id_present = item.event_id().is_some();
    let sender_present = item
        .sender()
        .is_some_and(|sender| !sender.as_str().trim().is_empty());
    let timestamp_ms = item.timestamp().map(|timestamp| timestamp.0.into());
    let relation = event_cache_relation_trace(item);
    let relation_event_present = relation
        .relation_event_id
        .as_deref()
        .is_some_and(|event_id| !event_id.trim().is_empty());
    let reply_present = relation
        .reply_event_id
        .as_deref()
        .is_some_and(|event_id| !event_id.trim().is_empty());
    let thread_root_present = relation
        .thread_root_event_id
        .as_deref()
        .is_some_and(|event_id| !event_id.trim().is_empty());

    DiagnosticEvent::new(
        DiagnosticLevel::Debug,
        "core.event_cache",
        timeline_stage_token(stage),
    )
    .field(DiagnosticField::token("kind", timeline_diff_token(op)))
    .field(DiagnosticField::token(
        "timeline",
        timeline_key_trace_kind(key),
    ))
    .field(DiagnosticField::count("count", 1))
    .field(DiagnosticField::count("index", index.unwrap_or(0) as u64))
    .field(DiagnosticField::boolean("index_present", index.is_some()))
    .field(DiagnosticField::boolean(
        "event_id_present",
        event_id_present,
    ))
    .field(DiagnosticField::boolean("sender_present", sender_present))
    .field(DiagnosticField::boolean(
        "redacted",
        item.raw().deserialize().is_ok_and(|event| event.is_redacted()),
    ))
    .field(DiagnosticField::count(
        "timestamp_minute",
        timestamp_ms.unwrap_or(0) / 60_000,
    ))
    .field(DiagnosticField::boolean(
        "timestamp_present",
        timestamp_ms.is_some(),
    ))
    .field(DiagnosticField::boolean(
        "push_actions_present",
        item.push_actions().is_some(),
    ))
    .field(DiagnosticField::boolean(
        "push_notify",
        item.push_actions().is_some_and(|actions| actions.iter().any(|action| action.should_notify())),
    ))
    .field(DiagnosticField::boolean(
        "push_highlight",
        item.push_actions().is_some_and(|actions| actions.iter().any(|action| action.is_highlight())),
    ))
    .field(DiagnosticField::token("relation", relation.rel_type))
    .field(DiagnosticField::boolean(
        "relates_to_present",
        relation.relates_to_present,
    ))
    .field(DiagnosticField::boolean(
        "relation_event_present",
        relation_event_present,
    ))
    .field(DiagnosticField::boolean("reply_present", reply_present))
    .field(DiagnosticField::boolean(
        "thread_root_present",
        thread_root_present,
    ))
}

#[allow(dead_code)]
fn record_event_cache_item(
    stage: &str,
    key: &TimelineKey,
    op: &str,
    index: Option<usize>,
    item: &matrix_sdk_base::event_cache::Event,
) {
    koushi_diagnostics::record(event_cache_item_diagnostic_event(
        stage, key, op, index, item,
    ));
}

pub(super) fn trace_room_receipt_cache(
    key: &TimelineKey,
    room: &matrix_sdk::Room,
    items: &[matrix_sdk_base::event_cache::Event],
) {
    let receipts = room.read_receipts();
    let active_index = receipts.latest_active.as_ref().and_then(|active|
        items.iter().rposition(|item| item.event_id() == Some(active.event_id.as_ref())));
    koushi_diagnostics::record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "core.room_receipt_cache", "snapshot")
            .field(DiagnosticField::token("timeline", timeline_key_trace_kind(key)))
            .field(DiagnosticField::count("items", items.len() as u64))
            .field(DiagnosticField::boolean("active_present", receipts.latest_active.is_some()))
            .field(DiagnosticField::boolean("active_in_cache", active_index.is_some()))
            .field(DiagnosticField::count("active_index", active_index.unwrap_or(0) as u64))
            .field(DiagnosticField::count("unread", receipts.num_unread))
            .field(DiagnosticField::count("notifications", receipts.num_notifications))
            .field(DiagnosticField::count("mentions", receipts.num_mentions))
    );
}

pub(super) fn trace_event_cache_items(
    stage: &str,
    key: &TimelineKey,
    items: &[matrix_sdk_base::event_cache::Event],
) {
    let mut events = Vec::with_capacity(items.len().saturating_add(1));
    events.push(
        DiagnosticEvent::new(
            DiagnosticLevel::Debug,
            "core.event_cache",
            timeline_stage_token(stage),
        )
        .field(DiagnosticField::token("kind", "batch"))
        .field(DiagnosticField::token(
            "timeline",
            timeline_key_trace_kind(key),
        ))
        .field(DiagnosticField::count("count", items.len() as u64)),
    );
    let positions: std::collections::HashMap<_, _> = items.iter().enumerate()
        .filter_map(|(index, item)| item.event_id().map(|id| (id.as_str(), index)))
        .collect();
    for (index, item) in items.iter().enumerate() {
        let relation = event_cache_relation_trace(item);
        let target_index = relation.relation_event_id.as_deref()
            .and_then(|id| positions.get(id).copied());
        events.push(event_cache_item_diagnostic_event(
            stage,
            key,
            "item",
            Some(index),
            item,
        )
        .field(DiagnosticField::boolean("relation_target_in_cache", target_index.is_some()))
        .field(DiagnosticField::count("relation_target_index", target_index.unwrap_or(0) as u64)));
    }
    koushi_diagnostics::record_batch(events);
}

fn event_cache_diff_without_item_diagnostic_event(
    stage: &str,
    key: &TimelineKey,
    op: &str,
    index: Option<usize>,
    length: Option<usize>,
) -> DiagnosticEvent {
    DiagnosticEvent::new(
        DiagnosticLevel::Debug,
        "core.event_cache",
        timeline_stage_token(stage),
    )
    .field(DiagnosticField::token("kind", timeline_diff_token(op)))
    .field(DiagnosticField::token(
        "timeline",
        timeline_key_trace_kind(key),
    ))
    .field(DiagnosticField::count("count", length.unwrap_or(0) as u64))
    .field(DiagnosticField::count("index", index.unwrap_or(0) as u64))
    .field(DiagnosticField::boolean("index_present", index.is_some()))
}

fn record_event_cache_diff_without_item(
    stage: &str,
    key: &TimelineKey,
    op: &str,
    index: Option<usize>,
    length: Option<usize>,
) {
    koushi_diagnostics::record(event_cache_diff_without_item_diagnostic_event(
        stage, key, op, index, length,
    ));
}

#[cfg_attr(not(test), allow(dead_code))]
fn trace_event_cache_diff_without_item(
    stage: &str,
    key: &TimelineKey,
    op: &str,
    index: Option<usize>,
    length: Option<usize>,
) {
    record_event_cache_diff_without_item(stage, key, op, index, length);
}

pub(super) fn event_cache_origin_trace_token(
    origin: &matrix_sdk::event_cache::EventsOrigin,
) -> &'static str {
    match origin {
        matrix_sdk::event_cache::EventsOrigin::Sync => "sync",
        matrix_sdk::event_cache::EventsOrigin::Pagination => "network",
        matrix_sdk::event_cache::EventsOrigin::Cache => "cache",
        matrix_sdk::event_cache::EventsOrigin::GapRepair { .. } => "gap_repair",
    }
}

pub(super) fn trace_event_cache_diffs(
    stage: &str,
    key: &TimelineKey,
    origin: &matrix_sdk::event_cache::EventsOrigin,
    diffs: &[eyeball_im::VectorDiff<matrix_sdk_base::event_cache::Event>],
) {
    koushi_diagnostics::record(event_cache_diff_batch_diagnostic_event(
        stage, key, origin, diffs,
    ));
}

fn event_cache_diff_batch_diagnostic_event(
    stage: &str,
    key: &TimelineKey,
    origin: &matrix_sdk::event_cache::EventsOrigin,
    diffs: &[eyeball_im::VectorDiff<matrix_sdk_base::event_cache::Event>],
) -> DiagnosticEvent {
    let mut push_front_count = 0_u64;
    let mut push_back_count = 0_u64;
    let mut insert_count = 0_u64;
    let mut set_count = 0_u64;
    let mut append_count = 0_u64;
    let mut append_item_count = 0_u64;
    let mut reset_count = 0_u64;
    let mut reset_item_count = 0_u64;
    let mut remove_count = 0_u64;
    let mut truncate_count = 0_u64;
    let mut clear_count = 0_u64;
    let mut pop_front_count = 0_u64;
    let mut pop_back_count = 0_u64;
    for diff in diffs {
        match diff {
            eyeball_im::VectorDiff::PushFront { .. } => push_front_count += 1,
            eyeball_im::VectorDiff::PushBack { .. } => push_back_count += 1,
            eyeball_im::VectorDiff::Insert { .. } => insert_count += 1,
            eyeball_im::VectorDiff::Set { .. } => set_count += 1,
            eyeball_im::VectorDiff::Append { values } => {
                append_count += 1;
                append_item_count = append_item_count.saturating_add(values.len() as u64);
            }
            eyeball_im::VectorDiff::Reset { values } => {
                reset_count += 1;
                reset_item_count = reset_item_count.saturating_add(values.len() as u64);
            }
            eyeball_im::VectorDiff::Remove { .. } => remove_count += 1,
            eyeball_im::VectorDiff::Truncate { .. } => truncate_count += 1,
            eyeball_im::VectorDiff::Clear => clear_count += 1,
            eyeball_im::VectorDiff::PopFront => pop_front_count += 1,
            eyeball_im::VectorDiff::PopBack => pop_back_count += 1,
        }
    }
    DiagnosticEvent::new(
        DiagnosticLevel::Debug,
        "core.event_cache",
        timeline_stage_token(stage),
    )
    .field(DiagnosticField::token("kind", "batch"))
    .field(DiagnosticField::token(
        "timeline",
        timeline_key_trace_kind(key),
    ))
    .field(DiagnosticField::token(
        "origin",
        event_cache_origin_trace_token(origin),
    ))
    .field(DiagnosticField::count("count", diffs.len() as u64))
    .field(DiagnosticField::count("push_front_count", push_front_count))
    .field(DiagnosticField::count("push_back_count", push_back_count))
    .field(DiagnosticField::count("insert_count", insert_count))
    .field(DiagnosticField::count("set_count", set_count))
    .field(DiagnosticField::count("append_count", append_count))
    .field(DiagnosticField::count(
        "append_item_count",
        append_item_count,
    ))
    .field(DiagnosticField::count("reset_count", reset_count))
    .field(DiagnosticField::count("reset_item_count", reset_item_count))
    .field(DiagnosticField::count("remove_count", remove_count))
    .field(DiagnosticField::count("truncate_count", truncate_count))
    .field(DiagnosticField::count("clear_count", clear_count))
    .field(DiagnosticField::count("pop_front_count", pop_front_count))
    .field(DiagnosticField::count("pop_back_count", pop_back_count))
}

fn pagination_direction_trace_token(direction: PaginationDirection) -> &'static str {
    match direction {
        PaginationDirection::Backward => "backward",
        PaginationDirection::Forward => "forward",
    }
}

pub(super) fn trace_timeline_route(
    stage: &str,
    kind: &str,
    request_id: RequestId,
    key: &TimelineKey,
) {
    record_timeline_event(
        stage,
        kind,
        vec![
            DiagnosticField::token("timeline", timeline_key_trace_kind(key)),
            DiagnosticField::request_id(
                "request_id",
                request_id.connection_id.0,
                request_id.sequence,
            ),
        ],
    );
}

pub(super) fn trace_timeline_paginate(
    stage: &str,
    request_id: RequestId,
    key: &TimelineKey,
    direction: PaginationDirection,
    event_count: u16,
    elapsed_ms: Option<u128>,
    gate_ms: Option<u128>,
    outcome: Option<&'static str>,
) {
    record_timeline_event(
        stage,
        "paginate",
        vec![
            DiagnosticField::token("timeline", timeline_key_trace_kind(key)),
            DiagnosticField::token("direction", pagination_direction_trace_token(direction)),
            DiagnosticField::count("count", event_count as u64),
            DiagnosticField::request_id(
                "request_id",
                request_id.connection_id.0,
                request_id.sequence,
            ),
            DiagnosticField::milliseconds("duration", elapsed_ms.unwrap_or(0)),
            DiagnosticField::milliseconds("gate_wait", gate_ms.unwrap_or(0)),
            DiagnosticField::token(
                "outcome",
                outcome.map(timeline_outcome_token).unwrap_or("pending"),
            ),
        ],
    );
}

pub(super) fn trace_timeline_link_preview(
    stage: &str,
    request_id: RequestId,
    key: &TimelineKey,
    pending_count: usize,
    ready_count: usize,
    failed_count: usize,
    elapsed_ms: Option<u128>,
    outcome: Option<&'static str>,
) {
    record_timeline_event(
        stage,
        "link_preview",
        vec![
            DiagnosticField::token("timeline", timeline_key_trace_kind(key)),
            DiagnosticField::count("pending", pending_count as u64),
            DiagnosticField::count("ready", ready_count as u64),
            DiagnosticField::count("failed", failed_count as u64),
            DiagnosticField::request_id(
                "request_id",
                request_id.connection_id.0,
                request_id.sequence,
            ),
            DiagnosticField::milliseconds("duration", elapsed_ms.unwrap_or(0)),
            DiagnosticField::token(
                "outcome",
                outcome.map(timeline_outcome_token).unwrap_or("pending"),
            ),
        ],
    );
}

#[cfg(test)]
mod tests;
