use super::RecoveryFailureKind;
use super::config::{EVENT_TIMEOUT, SYNC_TIMEOUT};
use super::credentials::RealCredentials;
use super::event_source::{QaEventDeadline, QaEventSource, QaSnapshotEventSource};
use super::{
    AccountCommand, AccountEvent, AccountKey, AppState, CoreCommand, CoreConnection, CoreEvent,
    CoreFailure, RecoveryRequest, RequestId, RoomEvent, SearchCommand, SearchEvent, SearchScope,
    SessionState, SyncEvent, TimelineEvent, TimelineKey,
};
use std::time::Duration;

#[cfg(any(debug_assertions, test))]
pub(super) enum RecoveryOutcome {
    Completed,
    Failed(RecoveryFailureKind),
}

// ---------------------------------------------------------------------------
// Event waiter helpers
// ---------------------------------------------------------------------------

#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_logged_in(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<AccountKey, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for LoggedIn event"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Account(AccountEvent::LoggedIn {
                request_id: ev_id,
                account_key,
            }) if ev_id == request_id => {
                return Ok(account_key);
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_recovery_outcome(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<RecoveryOutcome, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| {
                format!("{label}: timed out waiting for RecoveryCompleted or RecoveryFailed")
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Account(AccountEvent::RecoveryCompleted {
                request_id: ev_id, ..
            }) if ev_id == request_id => {
                return Ok(RecoveryOutcome::Completed);
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure: CoreFailure::RecoveryFailed { kind },
            } if ev_id == request_id => {
                return Ok(RecoveryOutcome::Failed(kind));
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: unexpected failure: {failure:?}"));
            }
            _ => continue,
        }
    }
}

#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_recovery_outcome_until<S: QaEventSource + ?Sized>(
    conn: &mut S,
    request_id: RequestId,
    label: &str,
    deadline: QaEventDeadline,
) -> Result<RecoveryOutcome, String> {
    loop {
        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| {
                format!("{label}: timed out waiting for RecoveryCompleted or RecoveryFailed")
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Account(AccountEvent::RecoveryCompleted {
                request_id: ev_id, ..
            }) if ev_id == request_id => {
                return Ok(RecoveryOutcome::Completed);
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure: CoreFailure::RecoveryFailed { kind },
            } if ev_id == request_id => {
                return Ok(RecoveryOutcome::Failed(kind));
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: unexpected failure: {failure:?}"));
            }
            _ => continue,
        }
    }
}

/// Wait for the authoritative recovery-required account event.
#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_recovery_required_after_sync<S: QaEventSource + ?Sized>(
    conn: &mut S,
    label: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("{label}: timed out waiting for RecoveryRequired"));
        }

        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let event = tokio::time::timeout(remaining, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RecoveryRequired"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Account(AccountEvent::RecoveryRequired { .. }) => {
                return Ok(());
            }
            _ => continue,
        }
    }
}

#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_ready_snapshot(
    conn: &mut CoreConnection,
    label: &str,
) -> Result<(), String> {
    if matches!(conn.snapshot().session, SessionState::Ready(_)) {
        return Ok(());
    }
    loop {
        let snapshot = tokio::time::timeout(EVENT_TIMEOUT, conn.next_versioned_snapshot())
            .await
            .map_err(|_| format!("{label}: timed out waiting for Ready snapshot"))?
            .ok_or_else(|| format!("{label}: snapshot stream closed"))?;
        if matches!(snapshot.state.session, SessionState::Ready(_)) {
            return Ok(());
        }
    }
}

/// Wait for the post-login `Ready` snapshot before starting sync.
/// `LoggedIn` can arrive before the reducer has processed `LoginSucceeded`,
/// so this gate closes the action-channel race before `SyncCommand::Start`.
#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_post_login_ready_snapshot(
    conn: &mut CoreConnection,
    label: &str,
) -> Result<(), String> {
    wait_for_ready_snapshot(conn, label).await
}

#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_sync_started(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
    timeout: Duration,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(timeout, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for SyncEvent::Started"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Sync(SyncEvent::Started {
                request_id: Some(ev_id),
            }) if ev_id == request_id => return Ok(()),
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_sync_running(
    conn: &mut CoreConnection,
    label: &str,
    timeout: Duration,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(timeout, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for SyncEvent::Running"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        if matches!(event, CoreEvent::Sync(SyncEvent::Running)) {
            return Ok(());
        }
        if matches!(event, CoreEvent::Sync(SyncEvent::Failed)) {
            return Err(format!(
                "{label}: SyncEvent::Failed received before Running"
            ));
        }
    }
}

#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_sync_stopped(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for SyncEvent::Stopped"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        if matches!(
            event,
            CoreEvent::Sync(SyncEvent::Stopped { request_id: Some(ev_id) })
            if ev_id == request_id
        ) {
            return Ok(());
        }
        if matches!(
            event,
            CoreEvent::Sync(SyncEvent::Stopped { request_id: None })
        ) {
            return Ok(());
        }
        if let CoreEvent::OperationFailed {
            request_id: ev_id,
            failure,
        } = event
        {
            if ev_id == request_id {
                return Err(format!("{label} failed: {failure:?}"));
            }
        }
    }
}

#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_non_empty_room_list(
    conn: &mut CoreConnection,
    label: &str,
    timeout: Duration,
) -> Result<AppState, String> {
    let snapshot = conn.snapshot();
    if !snapshot.rooms.is_empty() || !snapshot.spaces.is_empty() {
        return Ok(snapshot);
    }

    loop {
        let event = tokio::time::timeout(timeout, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for non-empty room list"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomListUpdated) | CoreEvent::StateDelta(_) => {
                let snapshot = conn.snapshot();
                if !snapshot.rooms.is_empty() || !snapshot.spaces.is_empty() {
                    return Ok(snapshot);
                }
            }
            _ => continue,
        }
    }
}

#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_room_created(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<String, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomEvent::RoomCreated"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomCreated {
                request_id: ev_id,
                room_id,
            }) if ev_id == request_id => {
                return Ok(room_id);
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_room_left(
    conn: &mut CoreConnection,
    request_id: RequestId,
    expected_room_id: &str,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomEvent::RoomLeft"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomLeft {
                request_id: ev_id,
                room_id,
            }) if ev_id == request_id => {
                if room_id == expected_room_id {
                    return Ok(());
                }
                return Err(format!("{label}: unexpected room id (redacted)"));
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_room_forgotten(
    conn: &mut CoreConnection,
    request_id: RequestId,
    expected_room_id: &str,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomEvent::RoomForgotten"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomForgotten {
                request_id: ev_id,
                room_id,
            }) if ev_id == request_id => {
                if room_id == expected_room_id {
                    return Ok(());
                }
                return Err(format!("{label}: unexpected room id (redacted)"));
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_space_created(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<String, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomEvent::SpaceCreated"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::SpaceCreated {
                request_id: ev_id,
                space_id,
            }) if ev_id == request_id => {
                return Ok(space_id);
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_space_child_set(
    conn: &mut CoreConnection,
    request_id: RequestId,
    space_id: &str,
    child_room_id: &str,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomEvent::SpaceChildSet"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::SpaceChildSet {
                request_id: ev_id,
                space_id: ev_space,
                child_room_id: ev_child,
            }) if ev_id == request_id => {
                if ev_space != space_id || ev_child != child_room_id {
                    return Err(format!("{label}: SpaceChildSet IDs mismatch (redacted)"));
                }
                return Ok(());
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_room_list_space_child(
    conn: &mut CoreConnection,
    space_id: &str,
    child_room_id: &str,
    label: &str,
    timeout: Duration,
) -> Result<AppState, String> {
    let contains_expected = |snapshot: &AppState| {
        snapshot.spaces.iter().any(|space| {
            space.space_id == space_id
                && space
                    .child_room_ids
                    .iter()
                    .any(|room_id| room_id == child_room_id)
        }) || snapshot.rooms.iter().any(|room| {
            room.room_id == child_room_id && room.parent_space_ids.iter().any(|id| id == space_id)
        })
    };

    let snapshot = conn.snapshot();
    if contains_expected(&snapshot) {
        return Ok(snapshot);
    }

    loop {
        let event = tokio::time::timeout(timeout, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for space-child projection"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomListUpdated) | CoreEvent::StateDelta(_) => {
                let snapshot = conn.snapshot();
                if contains_expected(&snapshot) {
                    return Ok(snapshot);
                }
            }
            _ => continue,
        }
    }
}

#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_initial_items(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    request_id: RequestId,
    label: &str,
) -> Result<Vec<koushi_protocol::event::TimelineItem>, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for TimelineEvent::InitialItems"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                cause_request_id: Some(ev_id),
                key: ref ev_key,
                items,
                ..
            }) if ev_id == request_id && ev_key == key => {
                return Ok(items);
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_send_completed(
    conn: &mut CoreConnection,
    request_id: RequestId,
    key: &TimelineKey,
    label: &str,
) -> Result<(String, String), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for TimelineEvent::SendCompleted"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::SendCompleted {
                request_id: ev_id,
                key: ref ev_key,
                transaction_id,
                event_id,
            }) if ev_id == request_id && ev_key == key => {
                return Ok((transaction_id, event_id));
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

/// Wait for a Set diff whose body contains `edited_body` or whose event_id
/// matches, signalling that the edit was received.
#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_edit_diff(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    request_id: RequestId,
    event_id: &str,
    edited_body: &str,
    label: &str,
    timeout: Duration,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(timeout, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for edit Set diff"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                ref diffs,
                ..
            }) if ev_key == key => {
                for diff in diffs {
                    if let koushi_protocol::event::TimelineDiff::Set { item, .. } = diff {
                        let body_ok = item.body.as_deref().unwrap_or("").contains(edited_body);
                        let eid_ok = matches!(
                            &item.id,
                            koushi_protocol::event::TimelineItemId::Event { event_id: id }
                            if id == event_id
                        );
                        if body_ok || eid_ok {
                            return Ok(());
                        }
                    }
                }
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: edit operation failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

/// Wait for an ItemsUpdated diff that signals a redaction (Remove or body-cleared Set).
#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_redact_diff(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    request_id: RequestId,
    label: &str,
    timeout: Duration,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(timeout, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for redact diff"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                ref diffs,
                ..
            }) if ev_key == key => {
                for diff in diffs {
                    match diff {
                        koushi_protocol::event::TimelineDiff::Remove { .. } => return Ok(()),
                        koushi_protocol::event::TimelineDiff::Set { item, .. } => {
                            // A redacted item typically has no body.
                            if item.body.is_none() || item.body.as_deref() == Some("") {
                                return Ok(());
                            }
                        }
                        _ => {}
                    }
                }
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: redact operation failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

#[cfg(any(debug_assertions, test))]
#[path = "../common/pagination_waiter.rs"]
mod pagination_waiter;

/// Drive correlated pagination until EndReached, respecting gap-repair admission.
#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_paginate_end_reached(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    first_request_id: RequestId,
    label: &str,
    timeout: Duration,
) -> Result<String, String> {
    pagination_waiter::wait_for_end_reached(
        conn,
        key,
        first_request_id,
        label,
        10,
        tokio::time::Instant::now() + timeout,
    )
    .await
}

/// Wait for a session restore, handling an optional recovery requirement.
#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_session_restored_with_recovery(
    conn: &mut CoreConnection,
    request_id: RequestId,
    expected_account_key: &AccountKey,
    creds: &RealCredentials,
    transcript: &mut Vec<String>,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| {
                format!("{label}: timed out waiting for SessionRestored or RecoveryRequired")
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Account(AccountEvent::SessionRestored {
                request_id: ev_id,
                account_key,
            }) if ev_id == request_id => {
                ensure_session_restored_account_key(&account_key, expected_account_key, label)?;
                return Ok(());
            }
            CoreEvent::Account(AccountEvent::RecoveryRequired { .. }) => {
                let line = "restore_recovery=required".to_owned();
                transcript.push(line.clone());
                println!("{line}");

                let submit_id = conn.next_request_id();
                conn.command(CoreCommand::Account(AccountCommand::SubmitRecovery {
                    request_id: submit_id,
                    request: RecoveryRequest {
                        secret: creds.recovery_key.clone(),
                    },
                }))
                .await
                .map_err(|e| format!("restore recovery submit failed: {e}"))?;

                match wait_for_recovery_outcome(conn, submit_id, "restore recovery").await? {
                    RecoveryOutcome::Completed => {
                        let line2 = "restore_recovery=completed".to_owned();
                        transcript.push(line2.clone());
                        println!("{line2}");
                    }
                    RecoveryOutcome::Failed(kind) => {
                        return Err(format!("restore recovery failed with kind {kind:?}"));
                    }
                }
                // Continue looping to receive SessionRestored.
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

#[cfg(any(debug_assertions, test))]
pub(super) fn ensure_session_restored_account_key(
    actual: &AccountKey,
    expected: &AccountKey,
    label: &str,
) -> Result<(), String> {
    if actual != expected {
        return Err(format!("{label}: SessionRestored account_key mismatch"));
    }
    Ok(())
}

/// Wait for a `Ready` session snapshot. If recovery is required first, submit
/// the recovery key once and keep waiting. On the restore path the session
/// normally reaches Ready directly without recovery.
#[cfg(any(debug_assertions, test))]
#[derive(Debug)]
pub(super) enum ReadyRecoveryWaitOutcome {
    Ready,
    RecoveryRequired,
}

#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_ready_or_recovery_required<S: QaSnapshotEventSource + ?Sized>(
    conn: &mut S,
    deadline: QaEventDeadline,
    label: &str,
) -> Result<ReadyRecoveryWaitOutcome, String> {
    if matches!(conn.snapshot().session, SessionState::Ready(_)) {
        return Ok(ReadyRecoveryWaitOutcome::Ready);
    }

    loop {
        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| format!("{label}: timed out waiting for Ready snapshot"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        if matches!(conn.snapshot().session, SessionState::Ready(_)) {
            return Ok(ReadyRecoveryWaitOutcome::Ready);
        }
        match event {
            CoreEvent::Account(AccountEvent::RecoveryRequired { .. }) => {
                return Ok(ReadyRecoveryWaitOutcome::RecoveryRequired);
            }
            _ => {}
        }
    }
}

#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_ready_handling_recovery(
    conn: &mut CoreConnection,
    creds: &RealCredentials,
    transcript: &mut Vec<String>,
    label: &str,
) -> Result<(), String> {
    let deadline = QaEventDeadline::after(SYNC_TIMEOUT);
    let mut recovery_submitted = false;
    loop {
        match wait_for_ready_or_recovery_required(conn, deadline, label).await? {
            ReadyRecoveryWaitOutcome::Ready => return Ok(()),
            ReadyRecoveryWaitOutcome::RecoveryRequired if !recovery_submitted => {
                recovery_submitted = true;
                submit_startup_lat_recovery(conn, creds, transcript, label, deadline).await?;
            }
            ReadyRecoveryWaitOutcome::RecoveryRequired => {}
        }
    }
}

#[cfg(any(debug_assertions, test))]
pub(super) async fn submit_startup_lat_recovery(
    conn: &mut CoreConnection,
    creds: &RealCredentials,
    transcript: &mut Vec<String>,
    label: &str,
    deadline: QaEventDeadline,
) -> Result<(), String> {
    let line = "startup_lat recovery=required".to_owned();
    transcript.push(line.clone());
    println!("{line}");
    let submit_id = conn.next_request_id();
    tokio::time::timeout_at(
        deadline.instant,
        conn.command(CoreCommand::Account(AccountCommand::SubmitRecovery {
            request_id: submit_id,
            request: RecoveryRequest {
                secret: creds.recovery_key.clone(),
            },
        })),
    )
    .await
    .map_err(|_| format!("{label}: timed out submitting recovery"))?
    .map_err(|e| format!("{label} recovery submit failed: {e}"))?;
    match wait_for_recovery_outcome_until(conn, submit_id, label, deadline).await? {
        RecoveryOutcome::Completed => {
            let l = "startup_lat recovery=completed".to_owned();
            transcript.push(l.clone());
            println!("{l}");
            Ok(())
        }
        // Coarse, no Debug (consistent with the codex finding-2 fix).
        RecoveryOutcome::Failed(_) => Err(format!("{label}: recovery failed")),
    }
}

#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_logged_out<S: QaSnapshotEventSource + ?Sized>(
    conn: &mut S,
    request_id: RequestId,
    expected_account_key: &AccountKey,
    label: &str,
) -> Result<(), String> {
    let deadline = QaEventDeadline::after(EVENT_TIMEOUT);
    let mut saw_logged_out = false;
    loop {
        if saw_logged_out && matches!(conn.snapshot().session, SessionState::SignedOut) {
            return Ok(());
        }

        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| format!("{label}: timed out waiting for LoggedOut event"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Account(AccountEvent::LoggedOut {
                request_id: ev_id,
                account_key,
            }) if ev_id == request_id => {
                if account_key != *expected_account_key {
                    return Err(format!("{label}: LoggedOut account_key mismatch"));
                }
                saw_logged_out = true;
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_operation_failed<S: QaEventSource + ?Sized>(
    conn: &mut S,
    request_id: RequestId,
    label: &str,
) -> Result<CoreFailure, String> {
    let deadline = QaEventDeadline::after(EVENT_TIMEOUT);
    loop {
        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| format!("{label}: timed out waiting for OperationFailed event"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Ok(failure);
            }
            CoreEvent::Account(account_event) => {
                let matches_id = match &account_event {
                    AccountEvent::LoggedIn { request_id: id, .. }
                    | AccountEvent::SessionRestored { request_id: id, .. }
                    | AccountEvent::SavedSessionsListed { request_id: id, .. }
                    | AccountEvent::RecoveryCompleted { request_id: id, .. }
                    | AccountEvent::ProfileUpdated { request_id: id, .. }
                    | AccountEvent::AvatarThumbnailDownloaded { request_id: id, .. }
                    | AccountEvent::ReportCompleted { request_id: id, .. }
                    | AccountEvent::LoggedOut { request_id: id, .. }
                    | AccountEvent::AccountSwitched { request_id: id, .. } => *id == request_id,
                    AccountEvent::OidcAuthorizationCreated { .. }
                    | AccountEvent::AuthDiscoveryChanged { .. }
                    | AccountEvent::RecoveryRequired { .. } => false,
                };
                if matches_id {
                    return Err(format!(
                        "{label}: expected OperationFailed but the operation succeeded"
                    ));
                }
            }
            _ => continue,
        }
    }
}

#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_operation_failed_and_signed_out<S: QaSnapshotEventSource + ?Sized>(
    conn: &mut S,
    request_id: RequestId,
    label: &str,
) -> Result<CoreFailure, String> {
    let deadline = QaEventDeadline::after(EVENT_TIMEOUT);
    let mut operation_failure = None;
    loop {
        if matches!(conn.snapshot().session, SessionState::SignedOut) {
            if let Some(failure) = operation_failure.take() {
                return Ok(failure);
            }
        }

        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| {
                format!("{label}: timed out waiting for OperationFailed and SignedOut state")
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                operation_failure = Some(failure);
            }
            CoreEvent::Account(account_event) => {
                let matches_request = match &account_event {
                    AccountEvent::LoggedIn { request_id: id, .. }
                    | AccountEvent::SessionRestored { request_id: id, .. }
                    | AccountEvent::SavedSessionsListed { request_id: id, .. }
                    | AccountEvent::RecoveryCompleted { request_id: id, .. }
                    | AccountEvent::ProfileUpdated { request_id: id, .. }
                    | AccountEvent::AvatarThumbnailDownloaded { request_id: id, .. }
                    | AccountEvent::ReportCompleted { request_id: id, .. }
                    | AccountEvent::LoggedOut { request_id: id, .. }
                    | AccountEvent::AccountSwitched { request_id: id, .. } => *id == request_id,
                    AccountEvent::OidcAuthorizationCreated { .. }
                    | AccountEvent::AuthDiscoveryChanged { .. }
                    | AccountEvent::RecoveryRequired { .. } => false,
                };
                if matches_request {
                    return Err(format!(
                        "{label}: expected OperationFailed but the operation succeeded"
                    ));
                }
            }
            _ => continue,
        }
    }
}

/// Wait for any timeline diff in `key` containing `body_substring` in any item body.
#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_body_substring_in_timeline(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    body_substring: &str,
    label: &str,
    timeout: Duration,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(timeout, conn.recv_event())
            .await
            .map_err(|_| {
                format!(
                    "{label}: timed out waiting for item with body containing '{body_substring}'"
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        let found = match &event {
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ev_key, diffs, ..
            }) if ev_key == key => diffs.iter().any(|diff| {
                let item_opt = match diff {
                    koushi_protocol::event::TimelineDiff::PushBack { item }
                    | koushi_protocol::event::TimelineDiff::PushFront { item }
                    | koushi_protocol::event::TimelineDiff::Insert { item, .. }
                    | koushi_protocol::event::TimelineDiff::Set { item, .. } => Some(item),
                    koushi_protocol::event::TimelineDiff::Reset { items } => {
                        return items
                            .iter()
                            .any(|it| it.body.as_deref().unwrap_or("").contains(body_substring));
                    }
                    _ => None,
                };
                item_opt.map_or(false, |it| {
                    it.body.as_deref().unwrap_or("").contains(body_substring)
                })
            }),
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ev_key, items, ..
            }) if ev_key == key => items
                .iter()
                .any(|it| it.body.as_deref().unwrap_or("").contains(body_substring)),
            _ => false,
        };

        if found {
            return Ok(());
        }
    }
}

/// Poll search until the expected event appears in results or the deadline is exceeded.
#[cfg(any(debug_assertions, test))]
pub(super) async fn poll_search_until_found_or_timeout(
    conn: &mut CoreConnection,
    query: &str,
    expected_event_id: &str,
    room_id: &str,
    label: &str,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "{label}: expected event not found in search results after {timeout:?}"
            ));
        }

        let rid = conn.next_request_id();
        conn.command(CoreCommand::Search(SearchCommand::Query {
            request_id: rid,
            query: query.to_owned(),
            scope: SearchScope::CurrentRoom {
                room_id: room_id.to_owned(),
            },
            room_filter: koushi_state::SearchRoomFilter::AllRooms,
        }))
        .await
        .map_err(|e| format!("{label}: submit search query failed: {e}"))?;

        let found = wait_for_search_results(conn, rid, expected_event_id, label).await?;
        if found {
            return Ok(());
        }

        // Wake on the next search index mutation rather than blindly sleeping:
        // wait (bounded) for a `SearchEvent::IndexUpdated`, then retry the
        // query. If no index event arrives within the bound, fall through to a
        // plain retry so a missing event can never deadlock the loop. The
        // overall `deadline` still bounds the whole poll.
        wait_for_index_update_or_idle(conn, deadline).await;
    }
}

/// Wait until the search index reports an `IndexUpdated`, the per-iteration
/// idle bound elapses, or the overall `deadline` passes. Other events on the
/// interleaved stream are ignored. Always returns (never errors): a missing
/// index event simply means the caller retries its query.
#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_index_update_or_idle(
    conn: &mut CoreConnection,
    deadline: tokio::time::Instant,
) {
    // Bound a single idle wait so the retry cadence matches the prior sleep
    // when the index is quiet, while still waking immediately on indexing.
    const IDLE_WAIT: Duration = Duration::from_millis(1000);
    let now = tokio::time::Instant::now();
    if now >= deadline {
        return;
    }
    let wait_until = (now + IDLE_WAIT).min(deadline);

    loop {
        let remaining = wait_until.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return;
        }
        match tokio::time::timeout(remaining, conn.recv_event()).await {
            // Timed out waiting for an index event — fall through to retry.
            Err(_) => return,
            // Stream lagged or closed — let the caller resync via its next query.
            Ok(Err(_)) => return,
            Ok(Ok(CoreEvent::Search(SearchEvent::IndexUpdated { .. }))) => return,
            // Any other event: keep waiting for an index update (or the bound).
            Ok(Ok(_)) => continue,
        }
    }
}

#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_search_results(
    conn: &mut CoreConnection,
    request_id: RequestId,
    expected_event_id: &str,
    label: &str,
) -> Result<bool, String> {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(10), conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for SearchEvent::Results"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Search(SearchEvent::Results {
                request_id: ev_id,
                results,
            }) if ev_id == request_id => {
                return Ok(results.iter().any(|r| r.event_id == expected_event_id));
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: search query failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}
