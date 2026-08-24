use super::{
    AppState, Arc, AtomicBool, AtomicUsize, Duration, EventStreamLag, JoinHandle, Mutex, Ordering,
    SessionState, Shutdown, SocketAddr, SyncEvent, TcpListener, TcpStream, VerificationFlowState,
    io,
};
use crate::ToSocketAddrs;
use crate::thread;
use std::sync::Condvar;

pub(super) fn verification_event_stream_error(
    label: &str,
    participant: &str,
    lag: EventStreamLag,
) -> String {
    if lag.skipped == 0 {
        format!("{label}: {participant} event stream closed")
    } else {
        format!(
            "{label}: {participant} event stream lagged; skipped={}",
            lag.skipped
        )
    }
}

pub(super) fn verification_closed_summary(
    state: &VerificationFlowState,
    expected_flow_id: u64,
) -> (&'static str, bool, usize) {
    let phase = match state {
        VerificationFlowState::Idle => "idle",
        VerificationFlowState::Requested { .. } => "requested",
        VerificationFlowState::Accepted { .. } => "accepted",
        VerificationFlowState::SasPresented { .. } => "presented",
        VerificationFlowState::Confirming { .. } => "confirming",
        VerificationFlowState::Done { .. } => "done",
        VerificationFlowState::Failed { .. } => "failed",
    };
    let matches = verification_state_flow_id(state) == Some(expected_flow_id);
    let count = match state {
        VerificationFlowState::SasPresented { emojis, .. }
        | VerificationFlowState::Confirming { emojis, .. } => emojis.len(),
        _ => 0,
    };
    (phase, matches, count)
}

pub(super) fn session_gate_closed_summary(
    state: &SessionState,
    expected_flow_id: u64,
) -> (&'static str, bool, usize) {
    match state {
        SessionState::Verifying {
            flow_id,
            sas_emojis,
            ..
        } => ("verifying", *flow_id == expected_flow_id, sas_emojis.len()),
        SessionState::AwaitingVerification { .. } => ("awaiting_verification", false, 0),
        SessionState::Provisional { .. } => ("provisional", false, 0),
        SessionState::AwaitingBootstrapConfirmation { .. } => {
            ("awaiting_bootstrap_confirmation", false, 0)
        }
        SessionState::Ready(_) => ("ready", false, 0),
        SessionState::Rejecting { .. } => ("rejecting", false, 0),
        SessionState::Locked(_) => ("locked", false, 0),
        SessionState::CapabilityBlocked { .. } => ("capability_blocked", false, 0),
        SessionState::SignedOut => ("signed_out", false, 0),
        SessionState::Restoring => ("restoring", false, 0),
        SessionState::SwitchingAccount { .. } => ("switching", false, 0),
        SessionState::Authenticating { .. } => ("authenticating", false, 0),
        SessionState::LoggingOut => ("logging_out", false, 0),
    }
}

pub(super) fn gate_session_phase(session: &SessionState) -> &'static str {
    match session {
        SessionState::Provisional {
            phase: koushi_state::ProvisionalPhase::CheckingTrust,
            ..
        } => "checking_trust",
        SessionState::Provisional {
            phase: koushi_state::ProvisionalPhase::DiscoveringMethods,
            ..
        } => "discovering_methods",
        SessionState::Provisional {
            phase: koushi_state::ProvisionalPhase::RecheckingTrust { .. },
            ..
        } => "rechecking_trust",
        SessionState::AwaitingVerification { .. } => "awaiting_verification",
        SessionState::Verifying { .. } => "verifying",
        SessionState::AwaitingBootstrapConfirmation { .. } => "awaiting_bootstrap_confirmation",
        SessionState::Rejecting { .. } => "rejecting",
        SessionState::Ready(_) => "ready",
        SessionState::Locked(_) => "locked",
        SessionState::CapabilityBlocked { .. } => "capability_blocked",
        SessionState::SignedOut => "signed_out",
        SessionState::Restoring => "restoring",
        SessionState::SwitchingAccount { .. } => "switching",
        SessionState::Authenticating { .. } => "authenticating",
        SessionState::LoggingOut => "logging_out",
    }
}

pub(super) fn trust_admission_diagnostic_summary(
    snapshot: &koushi_diagnostics::DiagnosticSnapshot,
) -> String {
    const ALLOWED_STAGES: &[&str] = &[
        "provisional_encryption_sync_started",
        "trust_recheck_requested",
        "trust_recheck_started",
        "trust_recheck_finished_verified",
        "trust_recheck_finished_unverified",
        "trust_recheck_finished_unknown",
        "trust_recheck_finished_failed",
        "trust_persisted",
        "provisional_encryption_sync_stopped",
        "provisional_encryption_sync_skipped",
        "ready_projection_dispatched",
        "trust_projection_reduced_ready",
        "trust_projection_reduced_locked",
        "trust_projection_reduced_gated",
        "trust_projection_ack_delivered",
        "trust_projection_ack_delivery_failed",
        "trust_projection_ack_mismatch",
        "ready_projection_ack",
        "lock_projection_ack",
        "normal_sync_started",
    ];
    let mut stages = snapshot
        .records
        .iter()
        .rev()
        .filter(|record| {
            record.event.source == "core.verification_admission"
                && ALLOWED_STAGES.contains(&record.event.stage)
        })
        .take(12)
        .map(|record| record.event.stage)
        .collect::<Vec<_>>();
    stages.reverse();
    if stages.is_empty() {
        "none".to_owned()
    } else {
        stages.join(">")
    }
}

/// A compact summary of a snapshot's room list for printing.
pub(super) fn room_list_summary(snapshot: &AppState) -> String {
    let spaces = snapshot.spaces.len();
    let rooms = snapshot.rooms.len();
    let dms = snapshot.rooms.iter().filter(|r| r.is_dm).count();
    let unread = snapshot.rooms.iter().filter(|r| r.unread_count > 0).count();
    format!("rooms={rooms} spaces={spaces} dms={dms} unread_rooms={unread}")
}

#[derive(Default)]
struct InviteObserverDiagnosticSummary {
    started: u64,
    rls_wake_max: u64,
    base_wake_max: u64,
    base_invite_update_seen: bool,
    base_membership_change_seen: bool,
    base_projection_required_seen: bool,
    invite_projection: u64,
    invite_projection_delivered: u64,
    invite_projection_undelivered: u64,
    last_projection_rooms: u64,
    last_projection_spaces: u64,
    last_projection_invites: u64,
    last_refresh_entries: u64,
    last_refresh_invites: u64,
    last_refresh_authoritative: bool,
    last_refresh_room_present: bool,
    lagged: u64,
    closed: u64,
    exit: u64,
    last_exit_reason: Option<&'static str>,
    dropped: u64,
}

pub(super) fn diagnostic_count_field(
    event: &koushi_diagnostics::DiagnosticEvent,
    key: &'static str,
) -> Option<u64> {
    event.fields.iter().find_map(|field| {
        if field.key == key
            && let koushi_diagnostics::DiagnosticValue::Count(value) = field.value
        {
            return Some(value);
        }
        None
    })
}

fn diagnostic_boolean_field(
    event: &koushi_diagnostics::DiagnosticEvent,
    key: &'static str,
) -> Option<bool> {
    event.fields.iter().find_map(|field| {
        if field.key == key
            && let koushi_diagnostics::DiagnosticValue::Boolean(value) = field.value
        {
            return Some(value);
        }
        None
    })
}

pub(super) fn diagnostic_has_token(
    event: &koushi_diagnostics::DiagnosticEvent,
    key: &'static str,
    expected: &'static str,
) -> bool {
    event.fields.iter().any(|field| {
        field.key == key && field.value == koushi_diagnostics::DiagnosticValue::Token(expected)
    })
}

pub(super) fn diagnostic_token_field(
    event: &koushi_diagnostics::DiagnosticEvent,
    key: &'static str,
) -> Option<&'static str> {
    event.fields.iter().find_map(|field| {
        if field.key == key
            && let koushi_diagnostics::DiagnosticValue::Token(value) = field.value
        {
            return Some(value);
        }
        None
    })
}

pub(super) fn invite_observer_diagnostic_summary(
    snapshot: &koushi_diagnostics::DiagnosticSnapshot,
) -> String {
    let mut summary = InviteObserverDiagnosticSummary {
        dropped: snapshot.dropped_records,
        ..InviteObserverDiagnosticSummary::default()
    };
    for record in &snapshot.records {
        let event = &record.event;
        if event.source != "core.room" {
            continue;
        }
        match event.stage {
            "live_observer_started" => summary.started = summary.started.saturating_add(1),
            "live_observer_wake_milestone" => {
                let wake_count = diagnostic_count_field(event, "wake_count").unwrap_or(0);
                if diagnostic_has_token(event, "source", "rls_diff") {
                    summary.rls_wake_max = summary.rls_wake_max.max(wake_count);
                } else if diagnostic_has_token(event, "source", "base_room_updates") {
                    summary.base_wake_max = summary.base_wake_max.max(wake_count);
                    summary.base_invite_update_seen |=
                        diagnostic_boolean_field(event, "invite_update_observed").unwrap_or(false);
                    summary.base_membership_change_seen |=
                        diagnostic_boolean_field(event, "invite_membership_changed")
                            .unwrap_or(false);
                    summary.base_projection_required_seen |=
                        diagnostic_boolean_field(event, "projection_required").unwrap_or(false);
                }
            }
            "live_observer_invite_projection" => {
                summary.invite_projection = summary.invite_projection.saturating_add(1);
            }
            "live_observer_invite_projection_completed" => {
                if diagnostic_boolean_field(event, "action_delivered").unwrap_or(false) {
                    summary.invite_projection_delivered =
                        summary.invite_projection_delivered.saturating_add(1);
                } else {
                    summary.invite_projection_undelivered =
                        summary.invite_projection_undelivered.saturating_add(1);
                }
            }
            "room_list_projection" => {
                summary.last_projection_rooms =
                    diagnostic_count_field(event, "rooms_count").unwrap_or(0);
                summary.last_projection_spaces =
                    diagnostic_count_field(event, "spaces_count").unwrap_or(0);
                summary.last_projection_invites =
                    diagnostic_count_field(event, "invites_count").unwrap_or(0);
            }
            "live_observer_refresh_snapshot" => {
                summary.last_refresh_entries =
                    diagnostic_count_field(event, "entries_count").unwrap_or(0);
                summary.last_refresh_invites =
                    diagnostic_count_field(event, "invited_entries_count").unwrap_or(0);
                summary.last_refresh_authoritative =
                    diagnostic_boolean_field(event, "authoritative").unwrap_or(false);
            }
            "live_observer_refresh_room" => {
                summary.last_refresh_room_present =
                    diagnostic_boolean_field(event, "requested_room_present").unwrap_or(false);
            }
            "live_observer_base_lagged" => {
                summary.lagged = summary.lagged.saturating_add(1);
            }
            "live_observer_auxiliary_closed" => {
                summary.closed = summary.closed.saturating_add(1);
            }
            "live_observer_exit" => {
                summary.exit = summary.exit.saturating_add(1);
                summary.last_exit_reason = diagnostic_token_field(event, "reason");
            }
            _ => {}
        }
    }
    format!(
        "observer_diag_started={} observer_diag_rls_wake_max={} \
         observer_diag_base_wake_max={} observer_diag_base_invite_update_seen={} \
         observer_diag_base_membership_change_seen={} \
         observer_diag_base_projection_required_seen={} observer_diag_invite_projection={} \
         observer_diag_invite_projection_delivered={} \
         observer_diag_invite_projection_undelivered={} observer_diag_last_projection_rooms={} \
         observer_diag_last_projection_spaces={} observer_diag_last_projection_invites={} \
         observer_diag_last_refresh_entries={} observer_diag_last_refresh_invites={} \
         observer_diag_last_refresh_authoritative={} \
         observer_diag_last_refresh_room_present={} \
         observer_diag_lagged={} \
         observer_diag_closed={} observer_diag_exit={} observer_diag_last_exit_reason={} \
         observer_diag_dropped={}",
        summary.started,
        summary.rls_wake_max,
        summary.base_wake_max,
        summary.base_invite_update_seen,
        summary.base_membership_change_seen,
        summary.base_projection_required_seen,
        summary.invite_projection,
        summary.invite_projection_delivered,
        summary.invite_projection_undelivered,
        summary.last_projection_rooms,
        summary.last_projection_spaces,
        summary.last_projection_invites,
        summary.last_refresh_entries,
        summary.last_refresh_invites,
        summary.last_refresh_authoritative,
        summary.last_refresh_room_present,
        summary.lagged,
        summary.closed,
        summary.exit,
        summary.last_exit_reason.unwrap_or("unknown"),
        summary.dropped,
    )
}

pub(super) fn sync_diagnostic_summary(snapshot: &koushi_diagnostics::DiagnosticSnapshot) -> String {
    let mut service_build_failed = 0_u64;
    let mut committed_response = 0_u64;
    let mut task_ended = 0_u64;
    let mut command_start = 0_u64;
    let mut command_stop = 0_u64;
    let mut command_restart = 0_u64;
    let mut last_task_kind = "unknown";
    let mut last_command_lifecycle = "unknown";
    let mut last_state = "unknown";
    let mut last_lifecycle = "unknown";
    let mut last_rooms_from_response = 0_u64;
    let mut last_observer_exit_reason = "unknown";
    for record in &snapshot.records {
        let event = &record.event;
        if event.source != "core.sync" {
            continue;
        }
        match event.stage {
            "command" => {
                last_command_lifecycle =
                    diagnostic_token_field(event, "lifecycle").unwrap_or("unknown");
                match diagnostic_token_field(event, "kind").unwrap_or("unknown") {
                    "start" => command_start = command_start.saturating_add(1),
                    "stop" => command_stop = command_stop.saturating_add(1),
                    "restart" => command_restart = command_restart.saturating_add(1),
                    _ => {}
                }
            }
            "service_build_failed" => service_build_failed = service_build_failed.saturating_add(1),
            "committed_response" => {
                committed_response = committed_response.saturating_add(1);
                last_rooms_from_response =
                    diagnostic_count_field(event, "rooms_from_response").unwrap_or(0);
            }
            "task_ended" => {
                task_ended = task_ended.saturating_add(1);
                last_task_kind = diagnostic_token_field(event, "kind").unwrap_or("unknown");
            }
            "sync_service_state" => {
                last_state = diagnostic_token_field(event, "state").unwrap_or("unknown");
            }
            "status_projected" => {
                last_lifecycle = diagnostic_token_field(event, "lifecycle").unwrap_or("unknown");
            }
            "observer_exit" => {
                last_observer_exit_reason =
                    diagnostic_token_field(event, "reason").unwrap_or("unknown");
            }
            _ => {}
        }
    }
    format!(
        "sync_diag_service_build_failed={} sync_diag_committed_response={} \
         sync_diag_task_ended={} sync_diag_command_start={} sync_diag_command_stop={} \
         sync_diag_command_restart={} sync_diag_last_task_kind={} \
         sync_diag_last_command_lifecycle={} sync_diag_last_state={} \
         sync_diag_last_lifecycle={} sync_diag_last_rooms_from_response={} \
         sync_diag_last_observer_exit_reason={}",
        service_build_failed,
        committed_response,
        task_ended,
        command_start,
        command_stop,
        command_restart,
        last_task_kind,
        last_command_lifecycle,
        last_state,
        last_lifecycle,
        last_rooms_from_response,
        last_observer_exit_reason,
    )
}

pub(super) fn runtime_sync_diagnostic_summary(
    snapshot: &koushi_diagnostics::DiagnosticSnapshot,
) -> String {
    let mut stop_command_effect = 0_u64;
    let mut stop_actor_projection = 0_u64;
    let mut account_sync_actor_stop = 0_u64;
    let mut session_invalidated = 0_u64;
    for record in &snapshot.records {
        let event = &record.event;
        match (event.source, event.stage) {
            ("core.runtime", "effect_stop_sync") => {
                match diagnostic_token_field(event, "source").unwrap_or("unknown") {
                    "command_effect" => stop_command_effect = stop_command_effect.saturating_add(1),
                    "actor_projection" => {
                        stop_actor_projection = stop_actor_projection.saturating_add(1)
                    }
                    _ => {}
                }
            }
            ("core.account", "sync_actor_stop") => {
                account_sync_actor_stop = account_sync_actor_stop.saturating_add(1);
            }
            ("core.account", "session_invalidated") => {
                session_invalidated = session_invalidated.saturating_add(1);
            }
            _ => {}
        }
    }
    let trust_path = trust_admission_diagnostic_summary(snapshot);
    format!(
        "runtime_diag_stop_command_effect={} runtime_diag_stop_actor_projection={} \
         runtime_diag_account_sync_actor_stop={} runtime_diag_session_invalidated={} \
         runtime_diag_trust_path={trust_path}",
        stop_command_effect, stop_actor_projection, account_sync_actor_stop, session_invalidated,
    )
}

pub(super) fn sync_state_diagnostic_label(sync: &koushi_state::SyncState) -> &'static str {
    match sync {
        koushi_state::SyncState::Stopped => "stopped",
        koushi_state::SyncState::Starting => "starting",
        koushi_state::SyncState::Running => "running",
        koushi_state::SyncState::Failed { .. } => "failed",
        koushi_state::SyncState::Reconnecting { .. } => "reconnecting",
    }
}

pub(super) fn session_state_diagnostic_label(session: &koushi_state::SessionState) -> &'static str {
    match session {
        koushi_state::SessionState::SignedOut => "signed_out",
        koushi_state::SessionState::Ready(_) => "ready",
        koushi_state::SessionState::Locked(_) => "locked",
        koushi_state::SessionState::LoggingOut => "logging_out",
        koushi_state::SessionState::Restoring => "restoring",
        koushi_state::SessionState::SwitchingAccount { .. } => "switching_account",
        koushi_state::SessionState::Authenticating { .. } => "authenticating",
        koushi_state::SessionState::Provisional { .. } => "provisional",
        koushi_state::SessionState::AwaitingVerification { .. } => "awaiting_verification",
        koushi_state::SessionState::Verifying { .. } => "verifying",
        koushi_state::SessionState::AwaitingBootstrapConfirmation { .. } => {
            "awaiting_bootstrap_confirmation"
        }
        koushi_state::SessionState::Rejecting { .. } => "rejecting",
        koushi_state::SessionState::CapabilityBlocked { .. } => "capability_blocked",
    }
}

pub(super) fn sync_event_diagnostic_label(event: &SyncEvent) -> &'static str {
    match event {
        SyncEvent::Started { .. } => "started",
        SyncEvent::Running => "running",
        SyncEvent::Reconnecting => "reconnecting",
        SyncEvent::Failed => "failed",
        SyncEvent::Stopped { .. } => "stopped",
    }
}

pub(super) fn verification_state_flow_id(state: &VerificationFlowState) -> Option<u64> {
    match state {
        VerificationFlowState::Idle => None,
        VerificationFlowState::Requested { request_id, .. }
        | VerificationFlowState::Accepted { request_id, .. }
        | VerificationFlowState::SasPresented { request_id, .. }
        | VerificationFlowState::Confirming { request_id, .. }
        | VerificationFlowState::Done { request_id, .. }
        | VerificationFlowState::Failed { request_id, .. } => Some(*request_id),
    }
}

pub(super) struct QaTcpProxy {
    listen_addr: SocketAddr,
    enabled: Arc<AtomicBool>,
    room_send_forwarded: Arc<AtomicUsize>,
    room_send_responses_completed: Arc<AtomicUsize>,
    running: Arc<AtomicBool>,
    active_streams: Arc<Mutex<Vec<TcpStream>>>,
    messages_control: Arc<Mutex<QaMessagesProxyControl>>,
    read_state_control: Arc<(Mutex<QaReadStateProxyControl>, Condvar)>,
    accept_thread: Option<JoinHandle<()>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QaProxyRequestKind {
    RoomSend,
    RoomMessages,
    ReadState,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum QaProxyRequestAction {
    Forward,
    FailClosed,
    ServeCannedMessages(Vec<u8>),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct QaMessagesProxyObservation {
    pub(super) room_messages_request_count: u32,
    pub(super) first_request_was_exact_tokenless_limit: bool,
    pub(super) first_request_had_from: bool,
    pub(super) freshness_page_served: bool,
    pub(super) expected_end_token_was_used: bool,
    pub(super) expected_end_token_request_count: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum QaMessagesProxyExpectation {
    TokenlessLiveTail,
    BackwardFrom { token: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QaMessagesProxyPhase {
    Open,
    Armed,
    Served,
    Rejected,
}

impl Default for QaMessagesProxyPhase {
    fn default() -> Self {
        Self::Open
    }
}

struct QaMessagesProxyState {
    phase: QaMessagesProxyPhase,
    expectation: Option<QaMessagesProxyExpectation>,
    tracked_end_token: Option<String>,
    observation: QaMessagesProxyObservation,
}

impl Default for QaMessagesProxyState {
    fn default() -> Self {
        Self {
            phase: QaMessagesProxyPhase::Open,
            expectation: None,
            tracked_end_token: None,
            observation: QaMessagesProxyObservation::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QaMessagesProxyDecision {
    Forward,
    FailClosed,
    ServeCannedPage,
}

impl QaMessagesProxyState {
    fn arm_page(
        &mut self,
        expectation: QaMessagesProxyExpectation,
        tracked_end_token: Option<String>,
    ) {
        self.phase = QaMessagesProxyPhase::Armed;
        self.expectation = Some(expectation);
        self.tracked_end_token = tracked_end_token;
        self.observation = QaMessagesProxyObservation::default();
    }

    fn observe_room_messages_request(
        &mut self,
        metadata: &QaRoomMessagesRequestMetadata,
    ) -> QaMessagesProxyDecision {
        self.observation.room_messages_request_count = self
            .observation
            .room_messages_request_count
            .saturating_add(1);
        if metadata.direction_is_backward
            && self
                .tracked_end_token
                .as_deref()
                .is_some_and(|token| metadata.from_token.as_deref() == Some(token))
        {
            self.observation.expected_end_token_request_count = self
                .observation
                .expected_end_token_request_count
                .saturating_add(1);
        }
        if self.phase != QaMessagesProxyPhase::Armed {
            return QaMessagesProxyDecision::Forward;
        }

        self.observation.first_request_was_exact_tokenless_limit =
            metadata.query_is_exact_tokenless_limit;
        self.observation.first_request_had_from = metadata.has_from;
        let expected_request_matched = match self.expectation.as_ref() {
            Some(QaMessagesProxyExpectation::TokenlessLiveTail) => {
                metadata.query_is_exact_tokenless_limit && !metadata.has_from
            }
            Some(QaMessagesProxyExpectation::BackwardFrom { token }) => {
                let matched = metadata.direction_is_backward
                    && metadata.from_token.as_deref() == Some(token.as_str());
                self.observation.expected_end_token_was_used = matched;
                matched
            }
            None => false,
        };
        if expected_request_matched {
            self.phase = QaMessagesProxyPhase::Served;
            self.observation.freshness_page_served = true;
            QaMessagesProxyDecision::ServeCannedPage
        } else {
            self.phase = QaMessagesProxyPhase::Rejected;
            QaMessagesProxyDecision::FailClosed
        }
    }
}

pub(super) struct QaCannedTimelineEvent {
    pub(super) event_id: String,
    pub(super) sender: String,
    pub(super) body: String,
    pub(super) origin_server_ts: u64,
}

pub(super) struct QaCannedMessagesPage {
    events: Vec<QaCannedTimelineEvent>,
    end: Option<String>,
}

impl QaCannedMessagesPage {
    pub(super) fn anchored_silent_gap(
        newest_known_event_id: String,
        newest_known_body: String,
        missing_event_id: String,
        missing_body: String,
        older_anchor_event_id: String,
        sender: String,
        older_anchor_body: String,
    ) -> Self {
        Self {
            events: vec![
                QaCannedTimelineEvent {
                    event_id: newest_known_event_id,
                    sender: sender.clone(),
                    body: newest_known_body,
                    origin_server_ts: 1_900_000_000_002,
                },
                QaCannedTimelineEvent {
                    event_id: missing_event_id,
                    sender: sender.clone(),
                    body: missing_body,
                    origin_server_ts: 1_900_000_000_001,
                },
                QaCannedTimelineEvent {
                    event_id: older_anchor_event_id,
                    sender,
                    body: older_anchor_body,
                    origin_server_ts: 1,
                },
            ],
            end: None,
        }
    }

    pub(super) fn response_body(&self) -> io::Result<Vec<u8>> {
        let chunk = self
            .events
            .iter()
            .map(|event| {
                serde_json::json!({
                    "type": "m.room.message",
                    "event_id": event.event_id,
                    "sender": event.sender,
                    "origin_server_ts": event.origin_server_ts,
                    "content": {
                        "msgtype": "m.text",
                        "body": event.body,
                    },
                })
            })
            .collect::<Vec<_>>();
        let mut response = serde_json::json!({
            "start": "qa-live-tail-start",
            "chunk": chunk,
            "state": [],
        });
        if let Some(end) = &self.end {
            response["end"] = serde_json::Value::String(end.clone());
        }
        serde_json::to_vec(&response)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }
}

#[derive(Default)]
struct QaMessagesProxyControl {
    state: QaMessagesProxyState,
    canned_page: Option<QaCannedMessagesPage>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum QaReadStateProxyMode {
    Forward,
    Hold,
    FailClosed,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct QaReadStateProxyObservation {
    pub(super) request_count: usize,
    pub(super) held_request_count: usize,
    pub(super) forwarded_count: usize,
    pub(super) completed_count: usize,
    pub(super) max_inflight: usize,
}

struct QaReadStateProxyControl {
    mode: QaReadStateProxyMode,
    observation: QaReadStateProxyObservation,
    inflight: usize,
}

impl Default for QaReadStateProxyControl {
    fn default() -> Self {
        Self {
            mode: QaReadStateProxyMode::Forward,
            observation: QaReadStateProxyObservation::default(),
            inflight: 0,
        }
    }
}

impl QaTcpProxy {
    pub(super) fn start(target_homeserver: &str) -> Result<Self, String> {
        let target = parse_http_homeserver_addr(target_homeserver)?;
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("send_queue proxy bind failed: {e}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("send_queue proxy nonblocking setup failed: {e}"))?;
        let listen_addr = listener
            .local_addr()
            .map_err(|e| format!("send_queue proxy local_addr failed: {e}"))?;
        let enabled = Arc::new(AtomicBool::new(true));
        let room_send_forwarded = Arc::new(AtomicUsize::new(0));
        let room_send_responses_completed = Arc::new(AtomicUsize::new(0));
        let running = Arc::new(AtomicBool::new(true));
        let active_streams = Arc::new(Mutex::new(Vec::new()));
        let messages_control = Arc::new(Mutex::new(QaMessagesProxyControl::default()));
        let read_state_control = Arc::new((
            Mutex::new(QaReadStateProxyControl::default()),
            Condvar::new(),
        ));

        let thread_enabled = enabled.clone();
        let thread_room_send_forwarded = room_send_forwarded.clone();
        let thread_room_send_responses_completed = room_send_responses_completed.clone();
        let thread_running = running.clone();
        let thread_streams = active_streams.clone();
        let thread_messages_control = messages_control.clone();
        let thread_read_state_control = read_state_control.clone();
        let accept_thread = thread::spawn(move || {
            while thread_running.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((client, _)) => {
                        if !thread_enabled.load(Ordering::SeqCst) {
                            let _ = client.shutdown(Shutdown::Both);
                            continue;
                        }
                        spawn_proxy_pair(
                            client,
                            target,
                            thread_streams.clone(),
                            thread_messages_control.clone(),
                            thread_read_state_control.clone(),
                            thread_room_send_forwarded.clone(),
                            thread_room_send_responses_completed.clone(),
                        );
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(20));
                    }
                    Err(_) => {
                        if thread_running.load(Ordering::SeqCst) {
                            thread::sleep(Duration::from_millis(20));
                        }
                    }
                }
            }
        });

        Ok(Self {
            listen_addr,
            enabled,
            room_send_forwarded,
            room_send_responses_completed,
            running,
            active_streams,
            messages_control,
            read_state_control,
            accept_thread: Some(accept_thread),
        })
    }

    pub(super) fn homeserver_url(&self) -> String {
        format!("http://{}", self.listen_addr)
    }

    pub(super) fn disable(&self) {
        self.enabled.store(false, Ordering::SeqCst);
        shutdown_active_streams(&self.active_streams);
    }

    pub(super) fn enable(&self) {
        self.enabled.store(true, Ordering::SeqCst);
    }

    pub(super) fn room_send_forwarded_count(&self) -> usize {
        self.room_send_forwarded.load(Ordering::SeqCst)
    }

    pub(super) fn room_send_responses_completed_count(&self) -> usize {
        self.room_send_responses_completed.load(Ordering::SeqCst)
    }

    pub(super) fn arm_first_live_tail_messages_page(
        &self,
        newest_known_event_id: String,
        newest_known_body: String,
        missing_event_id: String,
        missing_body: String,
        older_anchor_event_id: String,
        sender: String,
        older_anchor_body: String,
    ) -> Result<(), String> {
        self.arm_messages_page(
            QaMessagesProxyExpectation::TokenlessLiveTail,
            QaCannedMessagesPage::anchored_silent_gap(
                newest_known_event_id,
                newest_known_body,
                missing_event_id,
                missing_body,
                older_anchor_event_id,
                sender,
                older_anchor_body,
            ),
            None,
        )
    }

    pub(super) fn arm_detached_live_tail_messages_page(
        &self,
        events: Vec<QaCannedTimelineEvent>,
        end_token: String,
    ) -> Result<(), String> {
        let tracked_end_token = end_token.clone();
        self.arm_messages_page(
            QaMessagesProxyExpectation::TokenlessLiveTail,
            QaCannedMessagesPage {
                events,
                end: Some(end_token),
            },
            Some(tracked_end_token),
        )
    }

    pub(super) fn arm_historical_continuation_messages_page(
        &self,
        end_token: String,
        events: Vec<QaCannedTimelineEvent>,
    ) -> Result<(), String> {
        let tracked_end_token = end_token.clone();
        self.arm_messages_page(
            QaMessagesProxyExpectation::BackwardFrom { token: end_token },
            QaCannedMessagesPage { events, end: None },
            Some(tracked_end_token),
        )
    }

    fn arm_messages_page(
        &self,
        expectation: QaMessagesProxyExpectation,
        page: QaCannedMessagesPage,
        tracked_end_token: Option<String>,
    ) -> Result<(), String> {
        let mut control = self
            .messages_control
            .lock()
            .map_err(|_| "timeline messages proxy state lock was poisoned".to_owned())?;
        control.state.arm_page(expectation, tracked_end_token);
        control.canned_page = Some(page);
        Ok(())
    }

    pub(super) fn live_tail_messages_observation(
        &self,
    ) -> Result<QaMessagesProxyObservation, String> {
        self.messages_control
            .lock()
            .map(|control| control.state.observation)
            .map_err(|_| "timeline messages proxy state lock was poisoned".to_owned())
    }

    pub(super) fn set_read_state_proxy_mode(&self, mode: QaReadStateProxyMode) {
        let (state, wake) = &*self.read_state_control;
        if let Ok(mut state) = state.lock() {
            state.mode = mode;
            wake.notify_all();
        }
    }

    pub(super) fn hold_read_state_writes(&self) {
        self.set_read_state_proxy_mode(QaReadStateProxyMode::Hold);
    }

    pub(super) fn fail_read_state_writes(&self) {
        self.set_read_state_proxy_mode(QaReadStateProxyMode::FailClosed);
    }

    pub(super) fn release_read_state_writes(&self) {
        self.set_read_state_proxy_mode(QaReadStateProxyMode::Forward);
    }

    pub(super) fn read_state_observation(&self) -> Result<QaReadStateProxyObservation, String> {
        self.read_state_control
            .0
            .lock()
            .map(|state| state.observation)
            .map_err(|_| "read-state proxy state lock was poisoned".to_owned())
    }

    pub(super) fn wait_for_held_read_state_writes(
        &self,
        minimum: usize,
        timeout: Duration,
    ) -> Result<QaReadStateProxyObservation, String> {
        let (state_lock, wake) = &*self.read_state_control;
        let state = state_lock
            .lock()
            .map_err(|_| "read-state proxy state lock was poisoned".to_owned())?;
        let (state, _) = wake
            .wait_timeout_while(state, timeout, |state| {
                state.observation.held_request_count < minimum
            })
            .map_err(|_| "read-state proxy wait lock was poisoned".to_owned())?;
        if state.observation.held_request_count < minimum {
            return Err("read-state proxy held-write evidence timed out".to_owned());
        }
        Ok(state.observation)
    }
}

impl Drop for QaTcpProxy {
    fn drop(&mut self) {
        self.set_read_state_proxy_mode(QaReadStateProxyMode::Forward);
        self.running.store(false, Ordering::SeqCst);
        shutdown_active_streams(&self.active_streams);
        let _ = TcpStream::connect(self.listen_addr);
        if let Some(thread) = self.accept_thread.take() {
            let _ = thread.join();
        }
    }
}

fn parse_http_homeserver_addr(homeserver: &str) -> Result<SocketAddr, String> {
    let without_scheme = homeserver.strip_prefix("http://").ok_or_else(|| {
        format!("send_queue proxy requires a local http:// homeserver, got {homeserver}")
    })?;
    let authority = without_scheme
        .split_once('/')
        .map(|(authority, _)| authority)
        .unwrap_or(without_scheme);
    authority
        .to_socket_addrs()
        .map_err(|e| format!("send_queue proxy could not resolve {authority}: {e}"))?
        .next()
        .ok_or_else(|| format!("send_queue proxy could not resolve {authority}"))
}

fn spawn_proxy_pair(
    mut client: TcpStream,
    target: SocketAddr,
    active_streams: Arc<Mutex<Vec<TcpStream>>>,
    messages_control: Arc<Mutex<QaMessagesProxyControl>>,
    read_state_control: Arc<(Mutex<QaReadStateProxyControl>, Condvar)>,
    room_send_forwarded: Arc<AtomicUsize>,
    room_send_responses_completed: Arc<AtomicUsize>,
) {
    thread::spawn(move || {
        let _ = proxy_single_http_request(
            &mut client,
            target,
            active_streams,
            messages_control,
            read_state_control,
            room_send_forwarded,
            room_send_responses_completed,
        );
        let _ = client.shutdown(Shutdown::Both);
    });
}

fn proxy_single_http_request(
    client: &mut TcpStream,
    target: SocketAddr,
    active_streams: Arc<Mutex<Vec<TcpStream>>>,
    messages_control: Arc<Mutex<QaMessagesProxyControl>>,
    read_state_control: Arc<(Mutex<QaReadStateProxyControl>, Condvar)>,
    room_send_forwarded: Arc<AtomicUsize>,
    room_send_responses_completed: Arc<AtomicUsize>,
) -> io::Result<()> {
    let mut request_head = Vec::new();
    {
        let reader_stream = client.try_clone()?;
        let mut reader = io::BufReader::new(reader_stream);
        loop {
            let mut line = Vec::new();
            let bytes = io::BufRead::read_until(&mut reader, b'\n', &mut line)?;
            if bytes == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "client closed before HTTP headers",
                ));
            }
            request_head.extend_from_slice(&line);
            if request_head.ends_with(b"\r\n\r\n") || request_head.ends_with(b"\n\n") {
                break;
            }
            if request_head.len() > 64 * 1024 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "HTTP headers exceeded QA proxy limit",
                ));
            }
        }

        let content_length = http_content_length(&request_head)?;
        if content_length > 0 {
            let mut body = vec![0u8; content_length];
            io::Read::read_exact(&mut reader, &mut body)?;
            request_head.extend_from_slice(&body);
        }
    }

    let request_kind = qa_proxy_request_kind(&request_head)?;
    let action = qa_read_state_proxy_action(&read_state_control, request_kind)?
        .or(qa_messages_proxy_action(
            &messages_control,
            request_kind,
            &request_head,
        )?)
        .unwrap_or(QaProxyRequestAction::Forward);
    let count_forwarded_room_send =
        request_kind == QaProxyRequestKind::RoomSend && action == QaProxyRequestAction::Forward;
    let count_forwarded_read_state =
        request_kind == QaProxyRequestKind::ReadState && action == QaProxyRequestAction::Forward;
    match action {
        QaProxyRequestAction::Forward => {}
        QaProxyRequestAction::FailClosed => {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "QA proxy closed a selected sync request",
            ));
        }
        QaProxyRequestAction::ServeCannedMessages(body) => {
            write_qa_json_response(client, &body)?;
            return Ok(());
        }
    }

    let mut server = TcpStream::connect_timeout(&target, Duration::from_secs(2))?;
    if let Ok(mut streams) = active_streams.lock() {
        if let Ok(stream) = client.try_clone() {
            streams.push(stream);
        }
        if let Ok(stream) = server.try_clone() {
            streams.push(stream);
        }
    }

    let request = rewrite_http_request_connection_close(&request_head)?;
    if count_forwarded_room_send {
        room_send_forwarded.fetch_add(1, Ordering::SeqCst);
    }
    io::Write::write_all(&mut server, &request)?;
    io::copy(&mut server, client)?;
    if count_forwarded_room_send {
        room_send_responses_completed.fetch_add(1, Ordering::SeqCst);
    }
    if count_forwarded_read_state {
        qa_read_state_proxy_completed(&read_state_control);
    }
    Ok(())
}

fn qa_read_state_proxy_action(
    control: &Arc<(Mutex<QaReadStateProxyControl>, Condvar)>,
    request_kind: QaProxyRequestKind,
) -> io::Result<Option<QaProxyRequestAction>> {
    if request_kind != QaProxyRequestKind::ReadState {
        return Ok(None);
    }
    let (state_lock, wake) = &**control;
    let mut state = state_lock
        .lock()
        .map_err(|_| io::Error::other("QA read-state proxy state lock was poisoned"))?;
    state.observation.request_count = state.observation.request_count.saturating_add(1);
    while state.mode == QaReadStateProxyMode::Hold {
        state.observation.held_request_count =
            state.observation.held_request_count.saturating_add(1);
        wake.notify_all();
        state = wake
            .wait(state)
            .map_err(|_| io::Error::other("QA read-state proxy wait failed"))?;
    }
    match state.mode {
        QaReadStateProxyMode::Forward => {
            state.observation.forwarded_count = state.observation.forwarded_count.saturating_add(1);
            state.inflight = state.inflight.saturating_add(1);
            state.observation.max_inflight = state.observation.max_inflight.max(state.inflight);
            Ok(Some(QaProxyRequestAction::Forward))
        }
        QaReadStateProxyMode::FailClosed => Ok(Some(QaProxyRequestAction::FailClosed)),
        QaReadStateProxyMode::Hold => unreachable!("hold is drained before action selection"),
    }
}

fn qa_read_state_proxy_completed(control: &Arc<(Mutex<QaReadStateProxyControl>, Condvar)>) {
    let (state_lock, _) = &**control;
    if let Ok(mut state) = state_lock.lock() {
        state.inflight = state.inflight.saturating_sub(1);
        state.observation.completed_count = state.observation.completed_count.saturating_add(1);
    }
}

fn qa_proxy_request_kind(request: &[u8]) -> io::Result<QaProxyRequestKind> {
    let header_end = find_http_header_end(request)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing HTTP headers"))?;
    let head = String::from_utf8_lossy(&request[..header_end]);
    let line = head
        .lines()
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing HTTP request line"))?;
    let mut fields = line.split_ascii_whitespace();
    let method = fields
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing HTTP method"))?;
    let target = fields
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing HTTP target"))?;
    let version = fields
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing HTTP version"))?;
    if fields.next().is_some() || !version.starts_with("HTTP/") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid HTTP request line",
        ));
    }
    let path = target.split_once('?').map_or(target, |(path, _)| path);
    Ok(match (method, path) {
        ("PUT", path)
            if path.starts_with("/_matrix/client/")
                && path.contains("/rooms/")
                && path.contains("/send/") =>
        {
            QaProxyRequestKind::RoomSend
        }
        (_, path)
            if path.starts_with("/_matrix/client/")
                && path.contains("/rooms/")
                && path.ends_with("/messages") =>
        {
            QaProxyRequestKind::RoomMessages
        }
        (_, path)
            if path.starts_with("/_matrix/client/")
                && path.contains("/rooms/")
                && (path.contains("/receipt/") || path.ends_with("/read_markers")) =>
        {
            QaProxyRequestKind::ReadState
        }
        _ => QaProxyRequestKind::Other,
    })
}

fn qa_messages_proxy_action(
    control: &Arc<Mutex<QaMessagesProxyControl>>,
    request_kind: QaProxyRequestKind,
    request: &[u8],
) -> io::Result<Option<QaProxyRequestAction>> {
    if request_kind != QaProxyRequestKind::RoomMessages {
        return Ok(None);
    }
    let metadata = qa_room_messages_request_metadata(request)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "room messages proxy received a non-room-messages request",
        )
    })?;
    let mut control = control
        .lock()
        .map_err(|_| io::Error::other("QA messages proxy state lock was poisoned"))?;
    match control.state.observe_room_messages_request(&metadata) {
        QaMessagesProxyDecision::Forward => Ok(None),
        QaMessagesProxyDecision::FailClosed => Ok(Some(QaProxyRequestAction::FailClosed)),
        QaMessagesProxyDecision::ServeCannedPage => {
            let page = control.canned_page.take().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "QA messages proxy armed without a canned messages page",
                )
            })?;
            Ok(Some(QaProxyRequestAction::ServeCannedMessages(
                page.response_body()?,
            )))
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct QaRoomMessagesRequestMetadata {
    query_is_exact_tokenless_limit: bool,
    has_from: bool,
    direction_is_backward: bool,
    from_token: Option<String>,
}

fn qa_room_messages_request_metadata(
    request: &[u8],
) -> io::Result<Option<QaRoomMessagesRequestMetadata>> {
    let header_end = find_http_header_end(request)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing HTTP headers"))?;
    let head = String::from_utf8_lossy(&request[..header_end]);
    let line = head
        .lines()
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing HTTP request line"))?;
    let mut fields = line.split_ascii_whitespace();
    let _method = fields
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing HTTP method"))?;
    let target = fields
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing HTTP target"))?;
    let version = fields
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing HTTP version"))?;
    if fields.next().is_some() || !version.starts_with("HTTP/") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid HTTP request line",
        ));
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if !path.starts_with("/_matrix/client/")
        || !path.contains("/rooms/")
        || !path.ends_with("/messages")
    {
        return Ok(None);
    }
    let mut direction_is_backward = false;
    let mut from_token = None;
    for field in query.split('&') {
        let (name, value) = field.split_once('=').unwrap_or((field, ""));
        match name {
            "dir" => direction_is_backward = value == "b",
            "from" => from_token = Some(value.to_owned()),
            _ => {}
        }
    }
    Ok(Some(QaRoomMessagesRequestMetadata {
        query_is_exact_tokenless_limit: query == "dir=b&limit=128",
        has_from: from_token.is_some(),
        direction_is_backward,
        from_token,
    }))
}

fn write_qa_json_response(client: &mut TcpStream, body: &[u8]) -> io::Result<()> {
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    io::Write::write_all(client, headers.as_bytes())?;
    io::Write::write_all(client, body)
}

fn http_content_length(request_head: &[u8]) -> io::Result<usize> {
    let head = String::from_utf8_lossy(request_head);
    for line in head.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            return value.trim().parse::<usize>().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP content-length")
            });
        }
    }
    Ok(0)
}

fn rewrite_http_request_connection_close(request: &[u8]) -> io::Result<Vec<u8>> {
    let Some(header_end) = find_http_header_end(request) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "missing HTTP header terminator",
        ));
    };
    let (head, body) = request.split_at(header_end);
    let head = String::from_utf8_lossy(head);
    let mut lines = head.lines();
    let Some(request_line) = lines.next() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "missing HTTP request line",
        ));
    };

    let mut rewritten = Vec::with_capacity(request.len() + 32);
    rewritten.extend_from_slice(request_line.as_bytes());
    rewritten.extend_from_slice(b"\r\n");
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let lower = line
            .split_once(':')
            .map(|(name, _)| name.trim().to_ascii_lowercase());
        if matches!(lower.as_deref(), Some("connection" | "proxy-connection")) {
            continue;
        }
        rewritten.extend_from_slice(line.as_bytes());
        rewritten.extend_from_slice(b"\r\n");
    }
    rewritten.extend_from_slice(b"Connection: close\r\n\r\n");
    rewritten.extend_from_slice(body);
    Ok(rewritten)
}

fn find_http_header_end(request: &[u8]) -> Option<usize> {
    request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
        .or_else(|| {
            request
                .windows(2)
                .position(|window| window == b"\n\n")
                .map(|position| position + 2)
        })
}

fn shutdown_active_streams(active_streams: &Arc<Mutex<Vec<TcpStream>>>) {
    if let Ok(mut streams) = active_streams.lock() {
        for stream in streams.drain(..) {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod tests;
