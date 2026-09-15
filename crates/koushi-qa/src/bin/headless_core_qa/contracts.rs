use super::event_wait::{
    QaEventFuture, QaEventSource, QaSnapshotEventSource, projection_timeline_item,
};
use super::participants::{
    QaE2eeLogoutBarrier, QaOwnedE2eeCleanupOperations,
    ensure_incoming_verification_receiver_sync_not_stopped,
};
use super::registry::{
    QaScenario, QaStage, SEND_QUEUE_EVENT_TIMEOUT, TIMELINE_RECONNECT_EXPECTED_BODY_COUNT,
    should_run_focused_send_queue_route, tokens_for_stage,
};
use super::scenario_timeline::assert_zero_display_projection_reset_fallback_delta;
use super::{
    AccountEvent, AccountKey, AppState, Arc, CoreEvent, CoreFailure, Duration, EventStreamLag,
    Mutex, RequestId, SessionState, SyncEvent, TimelineDiff, TimelineEvent, TimelineItem,
    TimelineItemId, TimelineKey, TimelineMessageActions,
};
use koushi_protocol::event::ThreadSummaryDto;

pub(super) fn reconnect_test_bodies() -> Vec<String> {
    (0..TIMELINE_RECONNECT_EXPECTED_BODY_COUNT)
        .map(|index| format!("synthetic body {index:02}"))
        .collect()
}

pub(super) fn reconnect_test_items(indices: impl IntoIterator<Item = usize>) -> Vec<TimelineItem> {
    let bodies = reconnect_test_bodies();
    indices
        .into_iter()
        .map(|index| {
            synthetic_timeline_item(
                &format!("$synthetic-{index:02}:example.invalid"),
                Some(&bodies[index]),
                None,
                None,
                None,
            )
        })
        .collect()
}

pub(super) fn reconnect_test_request(sequence: u64) -> RequestId {
    RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
        sequence,
    }
}

pub(super) fn synthetic_timeline_item(
    event_id: &str,
    body: Option<&str>,
    in_reply_to_event_id: Option<&str>,
    thread_root: Option<&str>,
    thread_summary: Option<ThreadSummaryDto>,
) -> TimelineItem {
    TimelineItem {
        request_state: None,
        id: TimelineItemId::Event {
            event_id: event_id.to_owned(),
        },
        sender: Some("@member:test".to_owned()),
        sender_label: None,
        sender_avatar: None,
        body: body.map(str::to_owned),
        notice_i18n: None,
        message_kind: Default::default(),
        spoiler_spans: Vec::new(),
        timestamp_ms: None,
        in_reply_to_event_id: in_reply_to_event_id.map(str::to_owned),
        formatted: None,
        reply_quote: None,
        thread_root: thread_root.map(str::to_owned),
        thread_summary,
        media: None,
        link_previews: None,
        link_ranges: Vec::new(),
        mentioned_user_ids: Vec::new(),
        reactions: Vec::new(),
        can_react: false,
        is_redacted: false,
        is_hidden: false,
        can_redact: false,
        is_edited: false,
        can_edit: false,
        actions: TimelineMessageActions::default(),
        send_state: None,
        display_metadata: None,
        unable_to_decrypt: None,
    }
}

#[test]
fn send_queue_display_projection_fallback_gate_requires_zero_counter_delta() {
    assert_eq!(
        assert_zero_display_projection_reset_fallback_delta(41, 41),
        Ok(())
    );
    assert!(assert_zero_display_projection_reset_fallback_delta(41, 42).is_err());
}

#[test]
fn send_queue_alone_uses_the_focused_early_route() {
    assert!(should_run_focused_send_queue_route(QaScenario::SendQueue));

    for scenario in [
        QaScenario::All,
        QaScenario::LoginSync,
        QaScenario::RoomSpace,
        QaScenario::Timeline,
        QaScenario::E2eeTrust,
    ] {
        assert!(
            !should_run_focused_send_queue_route(scenario),
            "{scenario:?} must retain its existing route"
        );
    }
}

#[test]
fn all_flow_retains_the_primary_recovery_secret_for_its_send_queue_stage() {
    assert!(QaScenario::All.should_run_stage(QaStage::SendQueue));
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum RecordedOwnedE2eeCleanupOperation {
    StopSync,
    Logout(QaE2eeLogoutBarrier),
    AuthoritativeLogoutBarrier(QaE2eeLogoutBarrier),
    DropConnection,
    ShutdownRuntime,
}

pub(super) struct RecordingOwnedE2eeCleanupOperations {
    pub(super) participant: &'static str,
    pub(super) operations:
        std::sync::Arc<std::sync::Mutex<Vec<(&'static str, RecordedOwnedE2eeCleanupOperation)>>>,
    pub(super) fail_authoritative_barrier: bool,
}

impl RecordingOwnedE2eeCleanupOperations {
    pub(super) fn record(&self, operation: RecordedOwnedE2eeCleanupOperation) {
        self.operations
            .lock()
            .expect("cleanup observation lock")
            .push((self.participant, operation));
    }
}

impl QaOwnedE2eeCleanupOperations for RecordingOwnedE2eeCleanupOperations {
    async fn stop_sync(&mut self, _label: &str) -> Result<(), String> {
        self.record(RecordedOwnedE2eeCleanupOperation::StopSync);
        Ok(())
    }

    async fn submit_logout(
        &mut self,
        barrier: &QaE2eeLogoutBarrier,
        _label: &str,
    ) -> Result<(), String> {
        self.record(RecordedOwnedE2eeCleanupOperation::Logout(barrier.clone()));
        Ok(())
    }

    async fn wait_for_authoritative_logout(
        &mut self,
        barrier: &QaE2eeLogoutBarrier,
        _label: &str,
    ) -> Result<(), String> {
        self.record(RecordedOwnedE2eeCleanupOperation::AuthoritativeLogoutBarrier(barrier.clone()));
        if self.fail_authoritative_barrier {
            Err("injected authoritative logout barrier failure".to_owned())
        } else {
            Ok(())
        }
    }

    fn drop_connection(&mut self) {
        self.record(RecordedOwnedE2eeCleanupOperation::DropConnection);
    }

    async fn shutdown_runtime(&mut self) {
        self.record(RecordedOwnedE2eeCleanupOperation::ShutdownRuntime);
    }
}

pub(super) fn recording_owned_e2ee_cleanup_operations(
    participant: &'static str,
    fail_authoritative_barrier: bool,
    operations: &std::sync::Arc<
        std::sync::Mutex<Vec<(&'static str, RecordedOwnedE2eeCleanupOperation)>>,
    >,
) -> RecordingOwnedE2eeCleanupOperations {
    RecordingOwnedE2eeCleanupOperations {
        participant,
        operations: operations.clone(),
        fail_authoritative_barrier,
    }
}

pub(super) struct ScriptedQaEventSource {
    pub(super) events: std::collections::VecDeque<CoreEvent>,
}

impl QaEventSource for ScriptedQaEventSource {
    fn recv_event(
        &mut self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<CoreEvent, koushi_core::runtime::EventStreamLag>,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            match self.events.pop_front() {
                Some(event) => Ok(event),
                None => std::future::pending().await,
            }
        })
    }
}

pub(super) fn withheld_projection_test_item(event_id: &str, body: &str) -> TimelineItem {
    let mut item = projection_timeline_item(event_id, false);
    item.body = Some(body.to_owned());
    item
}

pub(super) fn withheld_projection_items_updated(key: TimelineKey, item: TimelineItem) -> CoreEvent {
    CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
        key,
        generation: koushi_protocol::ids::TimelineGeneration(0),
        batch_id: koushi_protocol::ids::TimelineBatchId(1),
        diffs: vec![TimelineDiff::PushBack { item }],
    })
}

pub(super) struct ScriptedQaSnapshotEventSource {
    pub(super) events: std::collections::VecDeque<(CoreEvent, SessionState)>,
    pub(super) snapshot: AppState,
    pub(super) received: usize,
}

impl QaEventSource for ScriptedQaSnapshotEventSource {
    fn recv_event(
        &mut self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<CoreEvent, koushi_core::runtime::EventStreamLag>,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            match self.events.pop_front() {
                Some((event, session)) => {
                    self.snapshot.session = session;
                    self.received += 1;
                    Ok(event)
                }
                None => std::future::pending().await,
            }
        })
    }
}

impl QaSnapshotEventSource for ScriptedQaSnapshotEventSource {
    fn snapshot(&self) -> AppState {
        self.snapshot.clone()
    }
}

pub(super) struct IntervalQaEventSource {
    pub(super) interval: tokio::time::Interval,
}

impl QaEventSource for IntervalQaEventSource {
    fn recv_event(
        &mut self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<CoreEvent, koushi_core::runtime::EventStreamLag>,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            self.interval.tick().await;
            Ok(CoreEvent::Sync(SyncEvent::Running))
        })
    }
}

pub(super) struct IntervalQaSnapshotEventSource {
    pub(super) interval: tokio::time::Interval,
    pub(super) snapshot: AppState,
    pub(super) first_event: Option<CoreEvent>,
}

impl QaEventSource for IntervalQaSnapshotEventSource {
    fn recv_event(
        &mut self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<CoreEvent, koushi_core::runtime::EventStreamLag>,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            if let Some(event) = self.first_event.take() {
                return Ok(event);
            }
            self.interval.tick().await;
            Ok(CoreEvent::Sync(SyncEvent::Running))
        })
    }
}

impl QaSnapshotEventSource for IntervalQaSnapshotEventSource {
    fn snapshot(&self) -> AppState {
        self.snapshot.clone()
    }
}

pub(super) struct SharedSnapshotPendingEventSource {
    pub(super) snapshot: Arc<Mutex<AppState>>,
}

impl QaEventSource for SharedSnapshotPendingEventSource {
    fn recv_event(&mut self) -> QaEventFuture<'_> {
        Box::pin(std::future::pending())
    }
}

impl QaSnapshotEventSource for SharedSnapshotPendingEventSource {
    fn snapshot(&self) -> AppState {
        self.snapshot
            .lock()
            .expect("shared QA snapshot lock should not be poisoned")
            .clone()
    }
}

pub(super) struct FirstEventSharedSnapshotPendingSource {
    pub(super) first_event: Option<CoreEvent>,
    pub(super) snapshot: Arc<Mutex<AppState>>,
}

impl QaEventSource for FirstEventSharedSnapshotPendingSource {
    fn recv_event(&mut self) -> QaEventFuture<'_> {
        if let Some(event) = self.first_event.take() {
            return Box::pin(async move { Ok(event) });
        }
        Box::pin(std::future::pending())
    }
}

impl QaSnapshotEventSource for FirstEventSharedSnapshotPendingSource {
    fn snapshot(&self) -> AppState {
        self.snapshot
            .lock()
            .expect("shared QA snapshot lock should not be poisoned")
            .clone()
    }
}

pub(super) struct FirstEventThenTerminalLagSource {
    pub(super) first_event: Option<CoreEvent>,
    pub(super) snapshot: AppState,
    pub(super) skipped: u64,
}

impl QaEventSource for FirstEventThenTerminalLagSource {
    fn recv_event(&mut self) -> QaEventFuture<'_> {
        Box::pin(async move {
            if let Some(event) = self.first_event.take() {
                return Ok(event);
            }
            self.snapshot.session = SessionState::SignedOut;
            Err(EventStreamLag {
                skipped: self.skipped,
            })
        })
    }
}

impl QaSnapshotEventSource for FirstEventThenTerminalLagSource {
    fn snapshot(&self) -> AppState {
        self.snapshot.clone()
    }
}

pub(super) fn qa_state_with_session(session: SessionState) -> AppState {
    AppState {
        session,
        ..AppState::default()
    }
}

pub(super) fn qa_logged_out_event(request_id: RequestId, account_key: AccountKey) -> CoreEvent {
    CoreEvent::Account(AccountEvent::LoggedOut {
        request_id,
        account_key,
    })
}

pub(super) fn qa_operation_failed_event(request_id: RequestId) -> CoreEvent {
    CoreEvent::OperationFailed {
        request_id,
        failure: CoreFailure::SessionNotFound,
    }
}

pub(super) fn qa_state_delta_event() -> CoreEvent {
    CoreEvent::StateDelta(koushi_core::StateDelta {
        generation: 1,
        changed: koushi_core::StateDeltaChangedSlices::default(),
    })
}

pub(super) fn strict_e2ee_waiter_inventory() -> &'static [(&'static str, &'static str)] {
    &[
        (
            "wait_for_existing_identity_gate",
            "\nasync fn wait_for_recovery_gate",
        ),
        (
            "wait_for_room_in_room_list",
            "\nasync fn wait_for_space_in_space_list",
        ),
        (
            "wait_for_sync_started_and_running",
            "\nasync fn wait_for_sync_started",
        ),
        ("wait_for_ready_snapshot", "\nasync fn wait_for_logged_in"),
        ("wait_for_logged_in", "\nasync fn wait_for_session_restored"),
        (
            "subscribe_active_timeline_projection_for_qa",
            "\nfn thread_initial_items_need_paginate_backfill",
        ),
        (
            "wait_for_verification_requested_event_only",
            "\nfn requested_verification_flow_id",
        ),
        (
            "wait_for_verification_accepted",
            "\nfn verification_state_is_at_least_accepted",
        ),
        (
            "wait_for_initial_items_from_source",
            "\n#[derive(Default)]\nstruct InitialItemsWaitDiagnostics",
        ),
        (
            "wait_for_send_flow_completion_with_timeout",
            "\nasync fn send_text_expect_local_echo",
        ),
        (
            "wait_for_item_with_body_or_decryption_failure",
            "\nasync fn wait_for_withheld_event_projection_from_source",
        ),
        (
            "wait_for_withheld_event_projection_from_source",
            "\n/// Wait until all `expected_bodies` are found",
        ),
    ]
}

pub(super) fn strict_e2ee_waiter_body(source: &str, waiter: &str, end_declaration: &str) -> String {
    let source = source.replace("pub(super) ", "");
    source
        .split(&format!("async fn {waiter}"))
        .nth(1)
        .unwrap_or_else(|| panic!("missing strict E2EE waiter {waiter}"))
        .split(end_declaration)
        .next()
        .unwrap_or_else(|| panic!("missing end declaration for strict E2EE waiter {waiter}"))
        .to_owned()
}

#[test]
fn incoming_verification_waiter_rejects_stopped_receiver_sync_at_entry() {
    let label = "incoming verification receiver";
    assert_eq!(
        ensure_incoming_verification_receiver_sync_not_stopped(
            &koushi_state::SyncState::Stopped,
            label,
        ),
        Err(format!(
            "{label}: receiver sync is stopped; cannot await an incoming verification request"
        ))
    );
    for sync in [
        koushi_state::SyncState::Running,
        koushi_state::SyncState::Starting,
        koushi_state::SyncState::Failed {
            reason: "synthetic failure detail".to_owned(),
        },
        koushi_state::SyncState::Reconnecting {
            reason: "synthetic reconnect detail".to_owned(),
        },
    ] {
        assert_eq!(
            ensure_incoming_verification_receiver_sync_not_stopped(&sync, label),
            Ok(())
        );
    }
}
