//! Core-owned request settlement over the event broadcast and snapshot watch.
//!
//! These types are intentionally not serde DTOs. They are an in-process API;
//! adapters may choose how to encode the returned result at their boundary.

use std::fmt;

use koushi_state::{
    AppState, ComposerDraftRevision, ComposerTarget, FocusedContextState, InviteOperationState,
    InviteScopeSelection, SearchScope, SessionState, SpaceMemberInviteOutcome,
    SpaceMemberRoleFailureKind, SubmissionId,
};
use tokio::sync::broadcast;

use super::connection::CoreConnection;
use koushi_protocol::event::{
    AccountEvent, CoreEvent, IntentNoOpReason, IntentOutcome, RoomEvent, TimelineEvent,
};
use koushi_protocol::failure::{CoreFailure, RoomFailureKind};
use koushi_protocol::ids::{AccountKey, RequestId, TimelineKey};
use koushi_protocol::state_update::VersionedAppStateSnapshot;

#[derive(Clone, Eq, PartialEq)]
pub enum OutcomeCorrelation {
    Request(RequestId),
    Submission {
        request_id: RequestId,
        submission_id: SubmissionId,
    },
}

#[derive(Clone, Eq, PartialEq)]
pub enum RoomOperationKind {
    SpaceChildSet {
        space_id: String,
        child_room_id: String,
    },
    UserInvited {
        user_id: String,
    },
    InviteAccepted,
    InviteDeclined,
    MarkedAsRead,
    MarkedAsUnread,
    OutboundSessionRotationForced,
    RoomLeft,
    RoomForgotten,
    RoomTagSet {
        tag: koushi_state::RoomTagKind,
    },
    RoomTagRemoved {
        tag: koushi_state::RoomTagKind,
    },
    PinEvent {
        event_id: String,
    },
    UnpinEvent {
        event_id: String,
    },
    PinnedEventsRefreshed,
    RoomSettingsLoaded,
    RoomSettingUpdated,
    SpaceMembersLoaded {
        generation: u64,
    },
    MemberModerated {
        target_user_id: String,
        action: koushi_state::RoomModerationAction,
    },
    MemberRoleUpdated {
        target_user_id: String,
    },
    SpaceMemberInviteSettled {
        target_user_id: String,
        generation: u64,
    },
    SpaceMemberInviteCancellationSettled {
        target_user_id: String,
        generation: u64,
    },
    SpaceMemberRoleUpdated {
        target_user_id: String,
        generation: u64,
    },
    InviteBatch {
        user_ids: Vec<String>,
        scope: InviteScopeSelection,
    },
    DirectoryQuery,
    DirectoryPreview,
}

#[derive(Clone, Eq, PartialEq)]
pub enum RequestOutcomeExpectation {
    OidcAuthorization {
        request_id: RequestId,
    },
    AuthDiscovery {
        request_id: RequestId,
        homeserver: String,
    },
    Authenticated {
        request_id: RequestId,
        account_key: Option<AccountKey>,
    },
    SignedOut {
        request_id: RequestId,
        account_key: AccountKey,
        allow_projection_only: bool,
    },
    SavedSessions {
        request_id: RequestId,
    },
    RoomSelected {
        request_id: RequestId,
        room_id: String,
        account_key: Option<AccountKey>,
        allow_initial: bool,
    },
    FocusedContextClosed {
        request_id: RequestId,
        account_key: AccountKey,
        room_id: Option<String>,
        allow_projection_only: bool,
    },
    FocusedContextOpened {
        request_id: RequestId,
        account_key: AccountKey,
        room_id: String,
        event_id: Option<String>,
    },
    MainTimelineAnchor {
        request_id: RequestId,
        key: TimelineKey,
        event_id: String,
        allow_live_fallback: bool,
    },
    RoomCreated {
        request_id: RequestId,
        account_key: AccountKey,
    },
    SpaceCreated {
        request_id: RequestId,
        account_key: AccountKey,
    },
    DirectMessageStarted {
        request_id: RequestId,
        account_key: AccountKey,
    },
    RoomJoined {
        request_id: RequestId,
        account_key: AccountKey,
        room_id: String,
    },
    InviteWorkflow {
        request_id: RequestId,
        account_key: AccountKey,
        room_id: String,
        query: String,
        closed: bool,
    },
    DirectoryQuery {
        request_id: RequestId,
        account_key: AccountKey,
    },
    DirectoryPreview {
        request_id: RequestId,
        account_key: AccountKey,
    },
    RoomOperation {
        request_id: RequestId,
        account_key: AccountKey,
        room_id: String,
        operation: RoomOperationKind,
    },
    SearchStarted {
        request_id: RequestId,
        account_key: Option<AccountKey>,
        query: String,
        scope: SearchScope,
    },
    SearchClosed {
        request_id: RequestId,
        account_key: Option<AccountKey>,
        allow_initial: bool,
        allow_projection_only: bool,
    },
    UploadStaging {
        request_id: RequestId,
        account_key: AccountKey,
        target: ComposerTarget,
        staged_ids: Vec<String>,
        allow_initial: bool,
    },
    ComposerAccepted {
        request_id: RequestId,
        account_key: AccountKey,
        target: ComposerTarget,
        expected_revision: ComposerDraftRevision,
    },
    Submission {
        request_id: RequestId,
        account_key: AccountKey,
        target: ComposerTarget,
        submission_id: SubmissionId,
    },
    PreparedMediaQueued {
        request_id: RequestId,
        key: TimelineKey,
        transaction_id: String,
    },
}

#[derive(Clone, Eq, PartialEq)]
pub enum RequestOutcome {
    OidcAuthorization {
        request_id: RequestId,
        authorization_url: String,
        state: String,
        generation: u64,
    },
    AuthDiscovery {
        request_id: RequestId,
        snapshot: VersionedAppStateSnapshot,
    },
    Authenticated {
        request_id: RequestId,
        snapshot: VersionedAppStateSnapshot,
    },
    SignedOut {
        request_id: RequestId,
        snapshot: VersionedAppStateSnapshot,
    },
    SavedSessions {
        request_id: RequestId,
        sessions: Vec<koushi_state::SessionInfo>,
    },
    RoomSelected {
        snapshot: VersionedAppStateSnapshot,
    },
    FocusedContext {
        snapshot: VersionedAppStateSnapshot,
    },
    MainTimelineAnchor {
        snapshot: VersionedAppStateSnapshot,
    },
    RoomCreated {
        request_id: RequestId,
        room_id: String,
        snapshot: VersionedAppStateSnapshot,
    },
    SpaceCreated {
        request_id: RequestId,
        space_id: String,
        snapshot: VersionedAppStateSnapshot,
    },
    DirectMessageStarted {
        request_id: RequestId,
        room_id: String,
        snapshot: VersionedAppStateSnapshot,
    },
    RoomJoined {
        request_id: RequestId,
        room_id: String,
        snapshot: VersionedAppStateSnapshot,
    },
    InviteWorkflow {
        request_id: RequestId,
        snapshot: VersionedAppStateSnapshot,
    },
    Directory {
        request_id: RequestId,
        snapshot: VersionedAppStateSnapshot,
    },
    RoomOperation {
        request_id: RequestId,
        snapshot: VersionedAppStateSnapshot,
    },
    Search {
        request_id: RequestId,
        snapshot: VersionedAppStateSnapshot,
    },
    UploadStaging {
        request_id: RequestId,
        snapshot: VersionedAppStateSnapshot,
    },
    ComposerAccepted {
        request_id: RequestId,
        revision: ComposerDraftRevision,
        snapshot: VersionedAppStateSnapshot,
    },
    SubmissionAccepted {
        request_id: RequestId,
        submission_id: SubmissionId,
        transaction_id: String,
        snapshot: VersionedAppStateSnapshot,
    },
    SubmissionRejected {
        request_id: RequestId,
        submission_id: SubmissionId,
        kind: koushi_protocol::failure::TimelineFailureKind,
        snapshot: VersionedAppStateSnapshot,
    },
    PreparedMediaQueued {
        request_id: RequestId,
        key: TimelineKey,
        transaction_id: String,
        snapshot: VersionedAppStateSnapshot,
    },
}

#[derive(Clone, Copy, Eq, PartialEq, thiserror::Error)]
pub enum RequestOutcomeError {
    #[error("request operation failed")]
    OperationFailed { failure: CoreFailure },
    #[error("request completed without applying a state change")]
    FailedNoOp { reason: IntentNoOpReason },
    #[error("request outcome event stream lagged")]
    Lagged,
    #[error("request outcome event stream disconnected")]
    Disconnected,
    #[error("request outcome timed out")]
    TimedOut,
    #[error("request outcome correlation or expectation is invalid")]
    InvalidOutcome,
}

impl fmt::Debug for OutcomeCorrelation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Request(request_id) => formatter
                .debug_struct("Request")
                .field("request_id", request_id)
                .finish(),
            Self::Submission { request_id, .. } => formatter
                .debug_struct("Submission")
                .field("request_id", request_id)
                .field("submission_id", &"SubmissionId(..)")
                .finish(),
        }
    }
}

impl fmt::Debug for RoomOperationKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self {
            Self::SpaceChildSet { .. } => "SpaceChildSet",
            Self::UserInvited { .. } => "UserInvited",
            Self::InviteAccepted => "InviteAccepted",
            Self::InviteDeclined => "InviteDeclined",
            Self::MarkedAsRead => "MarkedAsRead",
            Self::MarkedAsUnread => "MarkedAsUnread",
            Self::OutboundSessionRotationForced => "OutboundSessionRotationForced",
            Self::RoomLeft => "RoomLeft",
            Self::RoomForgotten => "RoomForgotten",
            Self::RoomTagSet { .. } => "RoomTagSet",
            Self::RoomTagRemoved { .. } => "RoomTagRemoved",
            Self::PinEvent { .. } => "PinEvent",
            Self::UnpinEvent { .. } => "UnpinEvent",
            Self::PinnedEventsRefreshed => "PinnedEventsRefreshed",
            Self::RoomSettingsLoaded => "RoomSettingsLoaded",
            Self::RoomSettingUpdated => "RoomSettingUpdated",
            Self::SpaceMembersLoaded { .. } => "SpaceMembersLoaded",
            Self::MemberModerated { .. } => "MemberModerated",
            Self::MemberRoleUpdated { .. } => "MemberRoleUpdated",
            Self::SpaceMemberInviteSettled { .. } => "SpaceMemberInviteSettled",
            Self::SpaceMemberInviteCancellationSettled { .. } => {
                "SpaceMemberInviteCancellationSettled"
            }
            Self::SpaceMemberRoleUpdated { .. } => "SpaceMemberRoleUpdated",
            Self::InviteBatch { .. } => "InviteBatch",
            Self::DirectoryQuery => "DirectoryQuery",
            Self::DirectoryPreview => "DirectoryPreview",
        };
        formatter
            .debug_tuple("RoomOperationKind")
            .field(&kind)
            .finish()
    }
}

impl fmt::Debug for RequestOutcomeExpectation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self {
            Self::OidcAuthorization { .. } => "OidcAuthorization",
            Self::AuthDiscovery { .. } => "AuthDiscovery",
            Self::Authenticated { .. } => "Authenticated",
            Self::SignedOut { .. } => "SignedOut",
            Self::SavedSessions { .. } => "SavedSessions",
            Self::RoomSelected { .. } => "RoomSelected",
            Self::FocusedContextClosed { .. } => "FocusedContextClosed",
            Self::FocusedContextOpened { .. } => "FocusedContextOpened",
            Self::MainTimelineAnchor { .. } => "MainTimelineAnchor",
            Self::RoomCreated { .. } => "RoomCreated",
            Self::SpaceCreated { .. } => "SpaceCreated",
            Self::DirectMessageStarted { .. } => "DirectMessageStarted",
            Self::RoomJoined { .. } => "RoomJoined",
            Self::InviteWorkflow { .. } => "InviteWorkflow",
            Self::DirectoryQuery { .. } => "DirectoryQuery",
            Self::DirectoryPreview { .. } => "DirectoryPreview",
            Self::RoomOperation { .. } => "RoomOperation",
            Self::SearchStarted { .. } => "SearchStarted",
            Self::SearchClosed { .. } => "SearchClosed",
            Self::UploadStaging { .. } => "UploadStaging",
            Self::ComposerAccepted { .. } => "ComposerAccepted",
            Self::Submission { .. } => "Submission",
            Self::PreparedMediaQueued { .. } => "PreparedMediaQueued",
        };
        formatter
            .debug_tuple("RequestOutcomeExpectation")
            .field(&kind)
            .finish()
    }
}

impl fmt::Debug for RequestOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self {
            Self::OidcAuthorization { .. } => "OidcAuthorization",
            Self::AuthDiscovery { .. } => "AuthDiscovery",
            Self::Authenticated { .. } => "Authenticated",
            Self::SignedOut { .. } => "SignedOut",
            Self::SavedSessions { .. } => "SavedSessions",
            Self::RoomSelected { .. } => "RoomSelected",
            Self::FocusedContext { .. } => "FocusedContext",
            Self::MainTimelineAnchor { .. } => "MainTimelineAnchor",
            Self::RoomCreated { .. } => "RoomCreated",
            Self::SpaceCreated { .. } => "SpaceCreated",
            Self::DirectMessageStarted { .. } => "DirectMessageStarted",
            Self::RoomJoined { .. } => "RoomJoined",
            Self::InviteWorkflow { .. } => "InviteWorkflow",
            Self::Directory { .. } => "Directory",
            Self::RoomOperation { .. } => "RoomOperation",
            Self::Search { .. } => "Search",
            Self::UploadStaging { .. } => "UploadStaging",
            Self::ComposerAccepted { .. } => "ComposerAccepted",
            Self::SubmissionAccepted { .. } => "SubmissionAccepted",
            Self::SubmissionRejected { .. } => "SubmissionRejected",
            Self::PreparedMediaQueued { .. } => "PreparedMediaQueued",
        };
        formatter
            .debug_tuple("RequestOutcome")
            .field(&kind)
            .finish()
    }
}

impl fmt::Debug for RequestOutcomeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OperationFailed { failure } => formatter
                .debug_struct("OperationFailed")
                .field("failure", failure)
                .finish(),
            Self::FailedNoOp { reason } => formatter
                .debug_struct("FailedNoOp")
                .field("reason", reason)
                .finish(),
            Self::Lagged => formatter.write_str("Lagged"),
            Self::Disconnected => formatter.write_str("Disconnected"),
            Self::TimedOut => formatter.write_str("TimedOut"),
            Self::InvalidOutcome => formatter.write_str("InvalidOutcome"),
        }
    }
}

impl RequestOutcomeExpectation {
    fn request_id(&self) -> RequestId {
        match self {
            Self::OidcAuthorization { request_id }
            | Self::AuthDiscovery { request_id, .. }
            | Self::Authenticated { request_id, .. }
            | Self::SignedOut { request_id, .. }
            | Self::SavedSessions { request_id, .. }
            | Self::RoomSelected { request_id, .. }
            | Self::FocusedContextClosed { request_id, .. }
            | Self::FocusedContextOpened { request_id, .. }
            | Self::MainTimelineAnchor { request_id, .. }
            | Self::RoomCreated { request_id, .. }
            | Self::SpaceCreated { request_id, .. }
            | Self::DirectMessageStarted { request_id, .. }
            | Self::RoomJoined { request_id, .. }
            | Self::InviteWorkflow { request_id, .. }
            | Self::DirectoryQuery { request_id, .. }
            | Self::DirectoryPreview { request_id, .. }
            | Self::RoomOperation { request_id, .. }
            | Self::SearchStarted { request_id, .. }
            | Self::SearchClosed { request_id, .. }
            | Self::UploadStaging { request_id, .. }
            | Self::ComposerAccepted { request_id, .. }
            | Self::Submission { request_id, .. }
            | Self::PreparedMediaQueued { request_id, .. } => *request_id,
        }
    }

    fn lag_is_terminal(&self) -> bool {
        matches!(
            self,
            Self::OidcAuthorization { .. }
                | Self::AuthDiscovery { .. }
                | Self::SearchStarted { .. }
                | Self::SearchClosed { .. }
                | Self::RoomCreated { .. }
                | Self::SpaceCreated { .. }
                | Self::DirectoryQuery { .. }
                | Self::DirectoryPreview { .. }
                | Self::RoomOperation { .. }
                | Self::ComposerAccepted { .. }
                | Self::Submission { .. }
                | Self::PreparedMediaQueued { .. }
                | Self::SavedSessions { .. }
        )
    }
}

impl CoreConnection {
    /// Wait for a closed typed request outcome. The broadcast is a wake source;
    /// the watch snapshot is the authority for projection-backed success.
    pub async fn wait_for_request_outcome(
        &mut self,
        correlation: OutcomeCorrelation,
        expectation: RequestOutcomeExpectation,
        baseline_generation: u64,
        deadline: tokio::time::Instant,
    ) -> Result<RequestOutcome, RequestOutcomeError> {
        if !correlation_matches(&correlation, &expectation) {
            return Err(RequestOutcomeError::InvalidOutcome);
        }

        let mut progress: Option<EventProgress> = None;
        if let Some(outcome) = snapshot_outcome(
            &expectation,
            &self.versioned_snapshot(),
            baseline_generation,
            allows_initial_snapshot(&expectation),
        ) {
            return Ok(outcome);
        }

        loop {
            if let Some(outcome) = snapshot_outcome(
                &expectation,
                &self.versioned_snapshot(),
                baseline_generation,
                allows_initial_snapshot(&expectation),
            ) {
                return Ok(outcome);
            }
            if let Some(outcome) = progress.as_ref().and_then(|progress| {
                progress.snapshot_outcome(
                    &expectation,
                    &self.versioned_snapshot(),
                    baseline_generation,
                )
            }) {
                return Ok(outcome);
            }
            if tokio::time::Instant::now() >= deadline {
                return final_result(
                    &expectation,
                    &self.versioned_snapshot(),
                    baseline_generation,
                    RequestOutcomeError::TimedOut,
                    progress,
                );
            }

            let received = tokio::time::timeout_at(deadline, async {
                tokio::select! {
                    biased;
                    changed = self.snapshot_rx.changed() => SnapshotWake::from_changed(changed),
                    event = self.event_rx.recv() => SnapshotWake::from_event(event, &self),
                }
            })
            .await;

            match received {
                Ok(SnapshotWake::SnapshotChanged) => {}
                Ok(SnapshotWake::Event(event)) => match event_progress(event, &expectation) {
                    Ok(Some(next)) => {
                        if let Some(outcome) =
                            next.event_outcome(&expectation, &self.versioned_snapshot())
                        {
                            return Ok(outcome);
                        }
                        progress = Some(next);
                    }
                    Ok(None) => {}
                    Err(error) => return Err(error),
                },
                Ok(SnapshotWake::Lagged) => {
                    if let Some(outcome) = progress.as_ref().and_then(|progress| {
                        progress.snapshot_outcome(
                            &expectation,
                            &self.versioned_snapshot(),
                            baseline_generation,
                        )
                    }) {
                        return Ok(outcome);
                    }
                    if expectation.lag_is_terminal() {
                        return final_result(
                            &expectation,
                            &self.versioned_snapshot(),
                            baseline_generation,
                            RequestOutcomeError::Lagged,
                            progress,
                        );
                    }
                }
                Ok(SnapshotWake::Disconnected) => {
                    return final_result(
                        &expectation,
                        &self.versioned_snapshot(),
                        baseline_generation,
                        RequestOutcomeError::Disconnected,
                        progress,
                    );
                }
                Err(_) => {
                    return final_result(
                        &expectation,
                        &self.versioned_snapshot(),
                        baseline_generation,
                        RequestOutcomeError::TimedOut,
                        progress,
                    );
                }
            }
        }
    }
}

#[derive(Clone)]
enum EventProgress {
    Oidc {
        request_id: RequestId,
        authorization_url: String,
        state: String,
    },
    AuthDiscovery {
        request_id: RequestId,
        homeserver: String,
    },
    SavedSessions {
        request_id: RequestId,
        sessions: Vec<koushi_state::SessionInfo>,
    },
    RoomCreated {
        request_id: RequestId,
        room_id: String,
    },
    SpaceCreated {
        request_id: RequestId,
        space_id: String,
    },
    DirectMessageStarted {
        request_id: RequestId,
        room_id: String,
    },
    RoomJoined {
        request_id: RequestId,
        room_id: String,
    },
    Authenticated {
        request_id: RequestId,
        account_key: AccountKey,
    },
    SignedOut {
        request_id: RequestId,
        account_key: AccountKey,
    },
    Focused {
        request_id: RequestId,
        opened: bool,
    },
    Anchor {
        request_id: RequestId,
        live_fallback: bool,
    },
    RoomOperation {
        request_id: RequestId,
        room_id: String,
        event_id: Option<String>,
        user_id: Option<String>,
        action: Option<koushi_state::RoomModerationAction>,
        generation: Option<u64>,
    },
    InviteBatch {
        request_id: RequestId,
        room_id: String,
        user_ids: Vec<String>,
        scope: InviteScopeSelection,
    },
    InviteWorkflow {
        request_id: RequestId,
    },
    Directory {
        request_id: RequestId,
    },
    Search {
        request_id: RequestId,
    },
    SubmissionAccepted {
        request_id: RequestId,
        key: TimelineKey,
        submission_id: SubmissionId,
        transaction_id: String,
    },
    SubmissionRejected {
        request_id: RequestId,
        key: TimelineKey,
        submission_id: SubmissionId,
        kind: koushi_protocol::failure::TimelineFailureKind,
    },
    PreparedMediaQueued {
        request_id: RequestId,
        key: TimelineKey,
        transaction_id: String,
    },
}

impl EventProgress {
    fn event_outcome(
        &self,
        expectation: &RequestOutcomeExpectation,
        snapshot: &VersionedAppStateSnapshot,
    ) -> Option<RequestOutcome> {
        match (self, expectation) {
            (
                Self::Oidc {
                    request_id,
                    authorization_url,
                    state,
                },
                RequestOutcomeExpectation::OidcAuthorization { .. },
            ) => Some(RequestOutcome::OidcAuthorization {
                request_id: *request_id,
                authorization_url: authorization_url.clone(),
                state: state.clone(),
                generation: snapshot.generation,
            }),
            (
                Self::SavedSessions {
                    request_id,
                    sessions,
                },
                RequestOutcomeExpectation::SavedSessions { .. },
            ) => Some(RequestOutcome::SavedSessions {
                request_id: *request_id,
                sessions: sessions.clone(),
            }),
            (
                Self::SubmissionAccepted {
                    request_id,
                    key,
                    submission_id,
                    transaction_id,
                },
                RequestOutcomeExpectation::Submission {
                    account_key,
                    target,
                    submission_id: expected_submission_id,
                    ..
                },
            ) if submission_id == expected_submission_id
                && timeline_key_matches_composer_target(key, account_key, target) =>
            {
                Some(RequestOutcome::SubmissionAccepted {
                    request_id: *request_id,
                    submission_id: submission_id.clone(),
                    transaction_id: transaction_id.clone(),
                    snapshot: snapshot.clone(),
                })
            }
            (
                Self::SubmissionRejected {
                    request_id,
                    key,
                    submission_id,
                    kind,
                },
                RequestOutcomeExpectation::Submission {
                    account_key,
                    target,
                    submission_id: expected_submission_id,
                    ..
                },
            ) if submission_id == expected_submission_id
                && timeline_key_matches_composer_target(key, account_key, target) =>
            {
                Some(RequestOutcome::SubmissionRejected {
                    request_id: *request_id,
                    submission_id: submission_id.clone(),
                    kind: *kind,
                    snapshot: snapshot.clone(),
                })
            }
            (
                Self::PreparedMediaQueued {
                    request_id,
                    key,
                    transaction_id,
                },
                RequestOutcomeExpectation::PreparedMediaQueued {
                    key: expected_key,
                    transaction_id: expected_transaction_id,
                    ..
                },
            ) if key == expected_key && transaction_id == expected_transaction_id => {
                Some(RequestOutcome::PreparedMediaQueued {
                    request_id: *request_id,
                    key: key.clone(),
                    transaction_id: transaction_id.clone(),
                    snapshot: snapshot.clone(),
                })
            }
            (
                Self::Directory { request_id },
                RequestOutcomeExpectation::DirectoryQuery { .. }
                | RequestOutcomeExpectation::DirectoryPreview { .. },
            ) => Some(RequestOutcome::Directory {
                request_id: *request_id,
                snapshot: snapshot.clone(),
            }),
            (
                Self::RoomOperation { request_id, .. },
                RequestOutcomeExpectation::RoomOperation { operation, .. },
            ) if room_operation_is_event_terminal(operation) => {
                Some(RequestOutcome::RoomOperation {
                    request_id: *request_id,
                    snapshot: snapshot.clone(),
                })
            }
            _ => None,
        }
    }

    fn request_id(&self) -> RequestId {
        match self {
            Self::Oidc { request_id, .. }
            | Self::AuthDiscovery { request_id, .. }
            | Self::SavedSessions { request_id, .. }
            | Self::RoomCreated { request_id, .. }
            | Self::SpaceCreated { request_id, .. }
            | Self::DirectMessageStarted { request_id, .. }
            | Self::RoomJoined { request_id, .. }
            | Self::Authenticated { request_id, .. }
            | Self::SignedOut { request_id, .. }
            | Self::Focused { request_id, .. }
            | Self::Anchor { request_id, .. }
            | Self::RoomOperation { request_id, .. }
            | Self::InviteBatch { request_id, .. }
            | Self::InviteWorkflow { request_id }
            | Self::Directory { request_id }
            | Self::Search { request_id }
            | Self::PreparedMediaQueued { request_id, .. }
            | Self::SubmissionAccepted { request_id, .. }
            | Self::SubmissionRejected { request_id, .. } => *request_id,
        }
    }

    fn snapshot_outcome(
        &self,
        expectation: &RequestOutcomeExpectation,
        snapshot: &VersionedAppStateSnapshot,
        baseline_generation: u64,
    ) -> Option<RequestOutcome> {
        snapshot_outcome_for_progress(self, expectation, snapshot, baseline_generation)
    }
}

enum SnapshotWake {
    SnapshotChanged,
    Event(CoreEvent),
    Lagged,
    Disconnected,
}

impl SnapshotWake {
    fn from_changed(result: Result<(), tokio::sync::watch::error::RecvError>) -> Self {
        if result.is_ok() {
            Self::SnapshotChanged
        } else {
            Self::Disconnected
        }
    }

    fn from_event(
        result: Result<CoreEvent, broadcast::error::RecvError>,
        connection: &CoreConnection,
    ) -> Self {
        match result {
            Ok(event) => Self::Event(connection.project_event_for_consumer(event)),
            Err(broadcast::error::RecvError::Lagged(_)) => Self::Lagged,
            Err(broadcast::error::RecvError::Closed) => Self::Disconnected,
        }
    }
}

fn correlation_matches(
    correlation: &OutcomeCorrelation,
    expectation: &RequestOutcomeExpectation,
) -> bool {
    match correlation {
        OutcomeCorrelation::Request(request_id) => *request_id == expectation.request_id(),
        OutcomeCorrelation::Submission {
            request_id,
            submission_id,
        } => matches!(
            expectation,
            RequestOutcomeExpectation::Submission {
                request_id: expected_request_id,
                submission_id: expected_submission_id,
                ..
            } if request_id == expected_request_id && submission_id == expected_submission_id
        ),
    }
}

fn event_progress(
    event: CoreEvent,
    expectation: &RequestOutcomeExpectation,
) -> Result<Option<EventProgress>, RequestOutcomeError> {
    let request_id = expectation.request_id();
    match event {
        CoreEvent::OperationFailed {
            request_id: event_request_id,
            failure,
        } if event_request_id == request_id => {
            Err(RequestOutcomeError::OperationFailed { failure })
        }
        CoreEvent::IntentLifecycle {
            request_id: event_request_id,
            outcome: IntentOutcome::FailedNoOp(reason),
            ..
        } if event_request_id == request_id => Err(RequestOutcomeError::FailedNoOp { reason }),
        CoreEvent::Account(account_event) => match account_event {
            AccountEvent::OidcAuthorizationCreated {
                request_id: event_request_id,
                authorization_url,
                state,
            } if matches!(
                expectation,
                RequestOutcomeExpectation::OidcAuthorization { .. }
            ) && event_request_id == request_id =>
            {
                Ok(Some(EventProgress::Oidc {
                    request_id,
                    authorization_url,
                    state,
                }))
            }
            AccountEvent::AuthDiscoveryChanged {
                request_id: event_request_id,
                homeserver,
            } if matches!(expectation, RequestOutcomeExpectation::AuthDiscovery { .. })
                && event_request_id == request_id =>
            {
                Ok(Some(EventProgress::AuthDiscovery {
                    request_id,
                    homeserver,
                }))
            }
            AccountEvent::SavedSessionsListed {
                request_id: event_request_id,
                sessions,
            } if matches!(expectation, RequestOutcomeExpectation::SavedSessions { .. })
                && event_request_id == request_id =>
            {
                Ok(Some(EventProgress::SavedSessions {
                    request_id,
                    sessions,
                }))
            }
            AccountEvent::LoggedIn {
                request_id: event_request_id,
                account_key,
            }
            | AccountEvent::SessionRestored {
                request_id: event_request_id,
                account_key,
            } if matches!(expectation, RequestOutcomeExpectation::Authenticated { .. })
                && event_request_id == request_id
                && matches!(
                    expectation,
                    RequestOutcomeExpectation::Authenticated {
                        account_key: None,
                        ..
                    } | RequestOutcomeExpectation::Authenticated {
                        account_key: Some(_),
                        ..
                    }
                ) =>
            {
                Ok(Some(EventProgress::Authenticated {
                    request_id,
                    account_key,
                }))
            }
            AccountEvent::LoggedOut {
                request_id: event_request_id,
                account_key,
            } if matches!(expectation, RequestOutcomeExpectation::SignedOut { .. })
                && event_request_id == request_id =>
            {
                Ok(Some(EventProgress::SignedOut {
                    request_id,
                    account_key,
                }))
            }
            _ => Ok(None),
        },
        CoreEvent::Room(room_event) => room_event_progress(room_event, expectation, request_id),
        CoreEvent::Timeline(timeline_event) => {
            timeline_event_progress(timeline_event, expectation, request_id)
        }
        CoreEvent::Search(search_event) => match search_event {
            koushi_protocol::event::SearchEvent::Results {
                request_id: event_request_id,
                ..
            } if matches!(expectation, RequestOutcomeExpectation::SearchStarted { .. })
                && event_request_id == request_id =>
            {
                Ok(Some(EventProgress::Search { request_id }))
            }
            _ => Ok(None),
        },
        CoreEvent::IntentLifecycle {
            request_id: event_request_id,
            outcome: IntentOutcome::BenignNoOp(reason),
            ..
        } if event_request_id == request_id
            && matches!(
                expectation,
                RequestOutcomeExpectation::MainTimelineAnchor { .. }
            ) =>
        {
            if matches!(reason, IntentNoOpReason::TimelineTargetMissing)
                && matches!(
                    expectation,
                    RequestOutcomeExpectation::MainTimelineAnchor {
                        allow_live_fallback: true,
                        ..
                    }
                )
            {
                Ok(Some(EventProgress::Anchor {
                    request_id,
                    live_fallback: true,
                }))
            } else {
                Err(RequestOutcomeError::FailedNoOp { reason })
            }
        }
        CoreEvent::IntentLifecycle {
            request_id: event_request_id,
            outcome: IntentOutcome::Committed,
            ..
        } if event_request_id == request_id => match expectation {
            RequestOutcomeExpectation::RoomSelected { .. } => Ok(Some(EventProgress::Focused {
                request_id,
                opened: false,
            })),
            RequestOutcomeExpectation::FocusedContextClosed { .. } => {
                Ok(Some(EventProgress::Focused {
                    request_id,
                    opened: false,
                }))
            }
            RequestOutcomeExpectation::FocusedContextOpened { .. } => {
                Ok(Some(EventProgress::Focused {
                    request_id,
                    opened: true,
                }))
            }
            RequestOutcomeExpectation::MainTimelineAnchor { .. } => {
                Ok(Some(EventProgress::Anchor {
                    request_id,
                    live_fallback: false,
                }))
            }
            RequestOutcomeExpectation::SearchStarted { .. } => {
                Ok(Some(EventProgress::Search { request_id }))
            }
            RequestOutcomeExpectation::SearchClosed { .. } => {
                Ok(Some(EventProgress::Search { request_id }))
            }
            RequestOutcomeExpectation::InviteWorkflow { .. } => {
                Ok(Some(EventProgress::InviteWorkflow { request_id }))
            }
            RequestOutcomeExpectation::DirectoryQuery { .. }
            | RequestOutcomeExpectation::DirectoryPreview { .. } => {
                Ok(Some(EventProgress::Directory { request_id }))
            }
            _ => Ok(None),
        },
        _ => Ok(None),
    }
}

fn room_event_progress(
    event: RoomEvent,
    expectation: &RequestOutcomeExpectation,
    request_id: RequestId,
) -> Result<Option<EventProgress>, RequestOutcomeError> {
    match event {
        RoomEvent::RoomCreated {
            request_id: event_request_id,
            room_id,
        } if matches!(expectation, RequestOutcomeExpectation::RoomCreated { .. })
            && event_request_id == request_id =>
        {
            Ok(Some(EventProgress::RoomCreated {
                request_id,
                room_id,
            }))
        }
        RoomEvent::SpaceCreated {
            request_id: event_request_id,
            space_id,
        } if matches!(expectation, RequestOutcomeExpectation::SpaceCreated { .. })
            && event_request_id == request_id =>
        {
            Ok(Some(EventProgress::SpaceCreated {
                request_id,
                space_id,
            }))
        }
        RoomEvent::DirectMessageStarted {
            request_id: event_request_id,
            room_id,
        } if matches!(
            expectation,
            RequestOutcomeExpectation::DirectMessageStarted { .. }
        ) && event_request_id == request_id =>
        {
            Ok(Some(EventProgress::DirectMessageStarted {
                request_id,
                room_id,
            }))
        }
        RoomEvent::RoomJoined {
            request_id: event_request_id,
            room_id,
        } if matches!(expectation, RequestOutcomeExpectation::RoomJoined { .. })
            && event_request_id == request_id =>
        {
            Ok(Some(EventProgress::RoomJoined {
                request_id,
                room_id,
            }))
        }
        RoomEvent::ComposerSlashCommandRejected {
            request_id: event_request_id,
            ..
        } if matches!(
            expectation,
            RequestOutcomeExpectation::ComposerAccepted { .. }
        ) && event_request_id == request_id =>
        {
            Err(RequestOutcomeError::FailedNoOp {
                reason: IntentNoOpReason::SessionNotReady,
            })
        }
        RoomEvent::DirectoryQueryCompleted {
            request_id: event_request_id,
            ..
        } if matches!(
            expectation,
            RequestOutcomeExpectation::DirectoryQuery { .. }
        ) && event_request_id == request_id =>
        {
            Ok(Some(EventProgress::Directory { request_id }))
        }
        RoomEvent::DirectoryPreviewLoaded {
            request_id: event_request_id,
            ..
        } if matches!(
            expectation,
            RequestOutcomeExpectation::DirectoryPreview { .. }
        ) && event_request_id == request_id =>
        {
            Ok(Some(EventProgress::Directory { request_id }))
        }
        event => room_operation_progress(event, expectation, request_id),
    }
}

fn room_operation_progress(
    event: RoomEvent,
    expectation: &RequestOutcomeExpectation,
    request_id: RequestId,
) -> Result<Option<EventProgress>, RequestOutcomeError> {
    let RequestOutcomeExpectation::RoomOperation {
        room_id: expected_room_id,
        operation,
        ..
    } = expectation
    else {
        return Ok(None);
    };

    let progress = match event {
        RoomEvent::SpaceChildSet {
            request_id: event_request_id,
            space_id,
            child_room_id,
        } if event_request_id == request_id
            && matches!(
                operation,
                RoomOperationKind::SpaceChildSet {
                    space_id: expected_space_id,
                    child_room_id: expected_child_room_id,
                } if expected_space_id == &space_id && expected_child_room_id == &child_room_id
            ) =>
        {
            EventProgress::RoomOperation {
                request_id,
                room_id: space_id,
                event_id: Some(child_room_id),
                user_id: None,
                action: None,
                generation: None,
            }
        }
        RoomEvent::UserInvited {
            request_id: event_request_id,
            room_id,
            user_id,
        } if event_request_id == request_id
            && expected_room_id == &room_id
            && matches!(operation, RoomOperationKind::UserInvited { user_id: expected } if expected == &user_id) =>
        {
            EventProgress::RoomOperation {
                request_id,
                room_id,
                event_id: None,
                user_id: Some(user_id),
                action: None,
                generation: None,
            }
        }
        RoomEvent::InviteAccepted {
            request_id: event_request_id,
            room_id,
        } if event_request_id == request_id
            && expected_room_id == &room_id
            && matches!(operation, RoomOperationKind::InviteAccepted) =>
        {
            EventProgress::RoomOperation {
                request_id,
                room_id,
                event_id: None,
                user_id: None,
                action: None,
                generation: None,
            }
        }
        RoomEvent::InviteDeclined {
            request_id: event_request_id,
            room_id,
        } if event_request_id == request_id
            && expected_room_id == &room_id
            && matches!(operation, RoomOperationKind::InviteDeclined) =>
        {
            EventProgress::RoomOperation {
                request_id,
                room_id,
                event_id: None,
                user_id: None,
                action: None,
                generation: None,
            }
        }
        RoomEvent::MarkedAsRead {
            request_id: event_request_id,
            room_id,
        } if event_request_id == request_id
            && expected_room_id == &room_id
            && matches!(operation, RoomOperationKind::MarkedAsRead) =>
        {
            EventProgress::RoomOperation {
                request_id,
                room_id,
                event_id: None,
                user_id: None,
                action: None,
                generation: None,
            }
        }
        RoomEvent::MarkedAsUnread {
            request_id: event_request_id,
            room_id,
            ..
        } if event_request_id == request_id
            && expected_room_id == &room_id
            && matches!(operation, RoomOperationKind::MarkedAsUnread) =>
        {
            EventProgress::RoomOperation {
                request_id,
                room_id,
                event_id: None,
                user_id: None,
                action: None,
                generation: None,
            }
        }
        RoomEvent::OutboundSessionRotationForced {
            request_id: event_request_id,
            room_id,
        } if event_request_id == request_id
            && expected_room_id == &room_id
            && matches!(operation, RoomOperationKind::OutboundSessionRotationForced) =>
        {
            EventProgress::RoomOperation {
                request_id,
                room_id,
                event_id: None,
                user_id: None,
                action: None,
                generation: None,
            }
        }
        RoomEvent::RoomLeft {
            request_id: event_request_id,
            room_id,
        } if event_request_id == request_id
            && expected_room_id == &room_id
            && matches!(operation, RoomOperationKind::RoomLeft) =>
        {
            EventProgress::RoomOperation {
                request_id,
                room_id,
                event_id: None,
                user_id: None,
                action: None,
                generation: None,
            }
        }
        RoomEvent::RoomForgotten {
            request_id: event_request_id,
            room_id,
        } if event_request_id == request_id
            && expected_room_id == &room_id
            && matches!(operation, RoomOperationKind::RoomForgotten) =>
        {
            EventProgress::RoomOperation {
                request_id,
                room_id,
                event_id: None,
                user_id: None,
                action: None,
                generation: None,
            }
        }
        RoomEvent::RoomTagSet {
            request_id: event_request_id,
            room_id,
            tag,
        } if event_request_id == request_id
            && expected_room_id == &room_id
            && matches!(operation, RoomOperationKind::RoomTagSet { tag: expected } if expected == &tag) =>
        {
            EventProgress::RoomOperation {
                request_id,
                room_id,
                event_id: None,
                user_id: None,
                action: None,
                generation: None,
            }
        }
        RoomEvent::RoomTagRemoved {
            request_id: event_request_id,
            room_id,
            tag,
        } if event_request_id == request_id
            && expected_room_id == &room_id
            && matches!(operation, RoomOperationKind::RoomTagRemoved { tag: expected } if expected == &tag) =>
        {
            EventProgress::RoomOperation {
                request_id,
                room_id,
                event_id: None,
                user_id: None,
                action: None,
                generation: None,
            }
        }
        RoomEvent::PinEventCompleted {
            request_id: event_request_id,
            room_id,
            event_id,
        } if event_request_id == request_id
            && expected_room_id == &room_id
            && matches!(operation, RoomOperationKind::PinEvent { event_id: expected } if expected == &event_id) =>
        {
            EventProgress::RoomOperation {
                request_id,
                room_id,
                event_id: Some(event_id),
                user_id: None,
                action: None,
                generation: None,
            }
        }
        RoomEvent::UnpinEventCompleted {
            request_id: event_request_id,
            room_id,
            event_id,
        } if event_request_id == request_id
            && expected_room_id == &room_id
            && matches!(operation, RoomOperationKind::UnpinEvent { event_id: expected } if expected == &event_id) =>
        {
            EventProgress::RoomOperation {
                request_id,
                room_id,
                event_id: Some(event_id),
                user_id: None,
                action: None,
                generation: None,
            }
        }
        RoomEvent::PinnedEventsUpdated {
            request_id: Some(event_request_id),
            room_id,
            ..
        } if event_request_id == request_id
            && expected_room_id == &room_id
            && matches!(operation, RoomOperationKind::PinnedEventsRefreshed) =>
        {
            EventProgress::RoomOperation {
                request_id,
                room_id,
                event_id: None,
                user_id: None,
                action: None,
                generation: None,
            }
        }
        RoomEvent::RoomSettingsLoaded {
            request_id: event_request_id,
            settings,
        } if event_request_id == request_id
            && expected_room_id == &settings.room_id
            && matches!(operation, RoomOperationKind::RoomSettingsLoaded) =>
        {
            EventProgress::RoomOperation {
                request_id,
                room_id: settings.room_id,
                event_id: None,
                user_id: None,
                action: None,
                generation: None,
            }
        }
        RoomEvent::RoomSettingUpdated {
            request_id: event_request_id,
            settings,
        } if event_request_id == request_id
            && expected_room_id == &settings.room_id
            && matches!(operation, RoomOperationKind::RoomSettingUpdated) =>
        {
            EventProgress::RoomOperation {
                request_id,
                room_id: settings.room_id,
                event_id: None,
                user_id: None,
                action: None,
                generation: None,
            }
        }
        RoomEvent::RoomMemberModerated {
            request_id: event_request_id,
            room_id,
            target_user_id,
            action,
        } if event_request_id == request_id
            && expected_room_id == &room_id
            && matches!(
                operation,
                RoomOperationKind::MemberModerated {
                    target_user_id: expected_user_id,
                    action: expected_action,
                } if expected_user_id == &target_user_id && expected_action == &action
            ) =>
        {
            EventProgress::RoomOperation {
                request_id,
                room_id,
                event_id: None,
                user_id: Some(target_user_id),
                action: Some(action),
                generation: None,
            }
        }
        RoomEvent::RoomMemberRoleUpdated {
            request_id: event_request_id,
            room_id,
            target_user_id,
            ..
        } if event_request_id == request_id
            && expected_room_id == &room_id
            && matches!(operation, RoomOperationKind::MemberRoleUpdated { target_user_id: expected } if expected == &target_user_id) =>
        {
            EventProgress::RoomOperation {
                request_id,
                room_id,
                event_id: None,
                user_id: Some(target_user_id),
                action: None,
                generation: None,
            }
        }
        RoomEvent::SpaceMembersLoaded {
            request_id: event_request_id,
            generation,
            ..
        } if event_request_id == request_id
            && matches!(operation, RoomOperationKind::SpaceMembersLoaded { generation: expected } if *expected == generation) =>
        {
            EventProgress::RoomOperation {
                request_id,
                room_id: expected_room_id.clone(),
                event_id: None,
                user_id: None,
                action: None,
                generation: Some(generation),
            }
        }
        RoomEvent::SpaceMemberInviteSettled {
            request_id: event_request_id,
            space_id,
            user_id,
            generation,
            outcome,
        } if event_request_id == request_id
            && expected_room_id == &space_id
            && matches!(
                operation,
                RoomOperationKind::SpaceMemberInviteSettled {
                    target_user_id: expected_user_id,
                    generation: expected_generation,
                } if expected_user_id == &user_id && *expected_generation == generation
            ) =>
        {
            if let SpaceMemberInviteOutcome::Failed(kind) = outcome {
                return Err(RequestOutcomeError::OperationFailed {
                    failure: CoreFailure::RoomOperationFailed {
                        kind: operation_failure_to_room_failure(kind),
                    },
                });
            }
            EventProgress::RoomOperation {
                request_id,
                room_id: space_id,
                event_id: None,
                user_id: Some(user_id),
                action: None,
                generation: Some(generation),
            }
        }
        RoomEvent::SpaceMemberInviteCancellationSettled {
            request_id: event_request_id,
            space_id,
            user_id,
            generation,
            outcome,
        } if event_request_id == request_id
            && expected_room_id == &space_id
            && matches!(
                operation,
                RoomOperationKind::SpaceMemberInviteCancellationSettled {
                    target_user_id: expected_user_id,
                    generation: expected_generation,
                } if expected_user_id == &user_id && *expected_generation == generation
            ) =>
        {
            if let SpaceMemberInviteOutcome::Failed(kind) = outcome {
                return Err(RequestOutcomeError::OperationFailed {
                    failure: CoreFailure::RoomOperationFailed {
                        kind: operation_failure_to_room_failure(kind),
                    },
                });
            }
            EventProgress::RoomOperation {
                request_id,
                room_id: space_id,
                event_id: None,
                user_id: Some(user_id),
                action: None,
                generation: Some(generation),
            }
        }
        RoomEvent::SpaceMemberRoleUpdateSettled {
            request_id: event_request_id,
            space_id,
            user_id,
            generation,
            outcome,
        } if event_request_id == request_id
            && expected_room_id == &space_id
            && matches!(
                operation,
                RoomOperationKind::SpaceMemberRoleUpdated {
                    target_user_id: expected_user_id,
                    generation: expected_generation,
                } if expected_user_id == &user_id && *expected_generation == generation
            ) =>
        {
            if let koushi_state::SpaceMemberRoleUpdateOutcome::Failed(kind) = outcome {
                return Err(RequestOutcomeError::OperationFailed {
                    failure: CoreFailure::RoomOperationFailed {
                        kind: role_failure_to_room_failure(kind),
                    },
                });
            }
            EventProgress::RoomOperation {
                request_id,
                room_id: space_id,
                event_id: None,
                user_id: Some(user_id),
                action: None,
                generation: Some(generation),
            }
        }
        RoomEvent::InviteBatchCompleted {
            request_id: event_request_id,
            room_id,
            results,
        } if event_request_id == request_id
            && expected_room_id == &room_id
            && matches!(operation, RoomOperationKind::InviteBatch { .. })
            && invite_batch_matches(operation, &results) =>
        {
            let RoomOperationKind::InviteBatch { user_ids, scope } = operation else {
                unreachable!()
            };
            EventProgress::InviteBatch {
                request_id,
                room_id,
                user_ids: user_ids.clone(),
                scope: scope.clone(),
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(progress))
}

fn operation_failure_to_room_failure(kind: koushi_state::OperationFailureKind) -> RoomFailureKind {
    match kind {
        koushi_state::OperationFailureKind::Forbidden => RoomFailureKind::Forbidden,
        koushi_state::OperationFailureKind::NotFound => RoomFailureKind::NotFound,
        koushi_state::OperationFailureKind::Network => RoomFailureKind::Network,
        koushi_state::OperationFailureKind::Timeout
        | koushi_state::OperationFailureKind::Invalid
        | koushi_state::OperationFailureKind::Sdk => RoomFailureKind::Sdk,
    }
}

fn role_failure_to_room_failure(kind: SpaceMemberRoleFailureKind) -> RoomFailureKind {
    match kind {
        SpaceMemberRoleFailureKind::Forbidden => RoomFailureKind::Forbidden,
        SpaceMemberRoleFailureKind::NotFound => RoomFailureKind::NotFound,
        SpaceMemberRoleFailureKind::Network => RoomFailureKind::Network,
        SpaceMemberRoleFailureKind::Timeout
        | SpaceMemberRoleFailureKind::Invalid
        | SpaceMemberRoleFailureKind::Sdk => RoomFailureKind::Sdk,
        SpaceMemberRoleFailureKind::Stale => RoomFailureKind::NotFound,
    }
}

fn invite_batch_matches(
    operation: &RoomOperationKind,
    results: &[koushi_state::InviteDestinationResult],
) -> bool {
    let RoomOperationKind::InviteBatch { user_ids, scope } = operation else {
        return false;
    };
    let expected_destinations = match scope {
        InviteScopeSelection::RoomOnly => 1,
        InviteScopeSelection::ParentSpaceAndRoom { .. } => 2,
    };
    user_ids.iter().all(|user_id| {
        results
            .iter()
            .filter(|result| {
                &result.user_id == user_id
                    && match scope {
                        InviteScopeSelection::RoomOnly => matches!(
                            &result.destination,
                            koushi_state::InviteDestination::Room { .. }
                        ),
                        InviteScopeSelection::ParentSpaceAndRoom { space_id } => {
                            match &result.destination {
                                koushi_state::InviteDestination::Room { .. } => true,
                                koushi_state::InviteDestination::Space { space_id: actual } => {
                                    actual == space_id
                                }
                            }
                        }
                    }
            })
            .count()
            == expected_destinations
    }) && results.len() == user_ids.len() * expected_destinations
}

fn timeline_event_progress(
    event: TimelineEvent,
    expectation: &RequestOutcomeExpectation,
    request_id: RequestId,
) -> Result<Option<EventProgress>, RequestOutcomeError> {
    match event {
        TimelineEvent::SubmissionAccepted {
            request_id: event_request_id,
            key,
            submission_id,
            transaction_id,
        } if matches!(expectation, RequestOutcomeExpectation::Submission { .. })
            && event_request_id == request_id =>
        {
            Ok(Some(EventProgress::SubmissionAccepted {
                request_id,
                key,
                submission_id,
                transaction_id,
            }))
        }
        TimelineEvent::SubmissionRejected {
            request_id: event_request_id,
            key,
            submission_id,
            kind,
        } if matches!(expectation, RequestOutcomeExpectation::Submission { .. })
            && event_request_id == request_id =>
        {
            Ok(Some(EventProgress::SubmissionRejected {
                request_id,
                key,
                submission_id,
                kind,
            }))
        }
        TimelineEvent::MediaSendQueued {
            request_id: event_request_id,
            key,
            transaction_id,
        } if matches!(
            expectation,
            RequestOutcomeExpectation::PreparedMediaQueued { .. }
        ) && event_request_id == request_id =>
        {
            Ok(Some(EventProgress::PreparedMediaQueued {
                request_id,
                key,
                transaction_id,
            }))
        }
        _ => Ok(None),
    }
}

fn snapshot_outcome(
    expectation: &RequestOutcomeExpectation,
    snapshot: &VersionedAppStateSnapshot,
    baseline_generation: u64,
    allow_initial: bool,
) -> Option<RequestOutcome> {
    if !allow_initial && snapshot.generation <= baseline_generation {
        return None;
    }
    match expectation {
        RequestOutcomeExpectation::RoomSelected {
            room_id,
            account_key,
            ..
        } if snapshot.state.navigation.active_room_id.as_deref() == Some(room_id.as_str())
            && account_matches(&snapshot.state, account_key.as_ref()) =>
        {
            Some(RequestOutcome::RoomSelected {
                snapshot: snapshot.clone(),
            })
        }
        RequestOutcomeExpectation::SignedOut {
            request_id,
            allow_projection_only: true,
            ..
        } if matches!(snapshot.state.session, SessionState::SignedOut) => {
            Some(RequestOutcome::SignedOut {
                request_id: *request_id,
                snapshot: snapshot.clone(),
            })
        }
        RequestOutcomeExpectation::FocusedContextClosed {
            request_id,
            account_key,
            room_id,
            allow_projection_only: true,
        } if account_matches(&snapshot.state, Some(account_key))
            && room_target_matches(&snapshot.state, room_id.as_deref())
            && snapshot.state.focused_context == FocusedContextState::Closed
            && snapshot.state.navigation.main_timeline_anchor.is_none() =>
        {
            Some(RequestOutcome::FocusedContext {
                snapshot: snapshot.clone(),
            })
        }
        RequestOutcomeExpectation::SearchClosed {
            request_id,
            account_key,
            allow_projection_only: true,
            ..
        } if snapshot.state.search == koushi_state::SearchState::Closed
            && account_matches(&snapshot.state, account_key.as_ref()) =>
        {
            Some(RequestOutcome::Search {
                request_id: *request_id,
                snapshot: snapshot.clone(),
            })
        }
        RequestOutcomeExpectation::UploadStaging {
            request_id,
            account_key,
            target,
            staged_ids,
            ..
        } if account_matches(&snapshot.state, Some(account_key))
            && staged_upload_ids_match(&snapshot.state, target, staged_ids) =>
        {
            Some(RequestOutcome::UploadStaging {
                request_id: *request_id,
                snapshot: snapshot.clone(),
            })
        }
        RequestOutcomeExpectation::ComposerAccepted {
            request_id,
            account_key,
            target,
            expected_revision,
        } if account_matches(&snapshot.state, Some(account_key))
            && composer_target_matches_snapshot(&snapshot.state, target)
            && composer_draft_revision(&snapshot.state, target) >= *expected_revision =>
        {
            Some(RequestOutcome::ComposerAccepted {
                request_id: *request_id,
                revision: composer_draft_revision(&snapshot.state, target),
                snapshot: snapshot.clone(),
            })
        }
        _ => None,
    }
}

pub(super) fn progress_generation_is_eligible(
    expectation: &RequestOutcomeExpectation,
    progress_request_id: RequestId,
    snapshot_generation: u64,
    baseline_generation: u64,
) -> bool {
    snapshot_generation > baseline_generation
        || (expectation.request_id() == progress_request_id
            && matches!(
                expectation,
                RequestOutcomeExpectation::RoomOperation {
                    operation: RoomOperationKind::RoomSettingsLoaded,
                    ..
                }
            ))
}

fn snapshot_outcome_for_progress(
    progress: &EventProgress,
    expectation: &RequestOutcomeExpectation,
    snapshot: &VersionedAppStateSnapshot,
    baseline_generation: u64,
) -> Option<RequestOutcome> {
    if !progress_generation_is_eligible(
        expectation,
        progress.request_id(),
        snapshot.generation,
        baseline_generation,
    ) {
        return None;
    }
    if progress.request_id() != expectation.request_id() {
        return None;
    }
    match (progress, expectation) {
        (
            EventProgress::Oidc {
                authorization_url,
                state,
                ..
            },
            RequestOutcomeExpectation::OidcAuthorization { request_id },
        ) => Some(RequestOutcome::OidcAuthorization {
            request_id: *request_id,
            authorization_url: authorization_url.clone(),
            state: state.clone(),
            generation: snapshot.generation,
        }),
        (
            EventProgress::AuthDiscovery { homeserver, .. },
            RequestOutcomeExpectation::AuthDiscovery {
                request_id,
                homeserver: expected_homeserver,
            },
        ) if homeserver == expected_homeserver
            && auth_discovery_matches(&snapshot.state, expected_homeserver) =>
        {
            Some(RequestOutcome::AuthDiscovery {
                request_id: *request_id,
                snapshot: snapshot.clone(),
            })
        }
        (
            EventProgress::Authenticated { account_key, .. },
            RequestOutcomeExpectation::Authenticated {
                request_id,
                account_key: expected_account_key,
            },
        ) if expected_account_key
            .as_ref()
            .is_none_or(|expected| expected == account_key)
            && account_matches(&snapshot.state, Some(account_key))
            && session_is_login_transport_terminal(&snapshot.state.session) =>
        {
            Some(RequestOutcome::Authenticated {
                request_id: *request_id,
                snapshot: snapshot.clone(),
            })
        }
        (
            EventProgress::SignedOut { account_key, .. },
            RequestOutcomeExpectation::SignedOut {
                request_id,
                account_key: expected_account_key,
                ..
            },
        ) if account_key == expected_account_key
            && matches!(snapshot.state.session, SessionState::SignedOut) =>
        {
            Some(RequestOutcome::SignedOut {
                request_id: *request_id,
                snapshot: snapshot.clone(),
            })
        }
        (
            EventProgress::RoomCreated { room_id, .. },
            RequestOutcomeExpectation::RoomCreated {
                request_id,
                account_key,
            },
        ) if account_matches(&snapshot.state, Some(account_key))
            && snapshot
                .state
                .rooms
                .iter()
                .any(|room| room.room_id == *room_id) =>
        {
            Some(RequestOutcome::RoomCreated {
                request_id: *request_id,
                room_id: room_id.clone(),
                snapshot: snapshot.clone(),
            })
        }
        (
            EventProgress::SpaceCreated { space_id, .. },
            RequestOutcomeExpectation::SpaceCreated {
                request_id,
                account_key,
            },
        ) if account_matches(&snapshot.state, Some(account_key))
            && snapshot
                .state
                .spaces
                .iter()
                .any(|space| space.space_id == *space_id) =>
        {
            Some(RequestOutcome::SpaceCreated {
                request_id: *request_id,
                space_id: space_id.clone(),
                snapshot: snapshot.clone(),
            })
        }
        (
            EventProgress::DirectMessageStarted { room_id, .. },
            RequestOutcomeExpectation::DirectMessageStarted {
                request_id,
                account_key,
            },
        ) if account_matches(&snapshot.state, Some(account_key))
            && snapshot
                .state
                .rooms
                .iter()
                .any(|room| room.room_id == *room_id) =>
        {
            Some(RequestOutcome::DirectMessageStarted {
                request_id: *request_id,
                room_id: room_id.clone(),
                snapshot: snapshot.clone(),
            })
        }
        (
            EventProgress::RoomJoined { room_id, .. },
            RequestOutcomeExpectation::RoomJoined {
                request_id,
                account_key,
                room_id: expected_room_id,
            },
        ) if (expected_room_id.is_empty() || room_id == expected_room_id)
            && account_matches(&snapshot.state, Some(account_key))
            && snapshot
                .state
                .rooms
                .iter()
                .any(|room| room.room_id == *room_id) =>
        {
            Some(RequestOutcome::RoomJoined {
                request_id: *request_id,
                room_id: room_id.clone(),
                snapshot: snapshot.clone(),
            })
        }
        (
            EventProgress::Focused { opened: false, .. },
            RequestOutcomeExpectation::FocusedContextClosed {
                request_id,
                account_key,
                room_id,
                ..
            },
        ) if account_matches(&snapshot.state, Some(account_key))
            && snapshot.state.focused_context == FocusedContextState::Closed
            && snapshot.state.navigation.main_timeline_anchor.is_none()
            && room_target_matches(&snapshot.state, room_id.as_deref()) =>
        {
            Some(RequestOutcome::FocusedContext {
                snapshot: snapshot.clone(),
            })
        }
        (
            EventProgress::Focused { opened: true, .. },
            RequestOutcomeExpectation::FocusedContextOpened {
                request_id,
                account_key,
                room_id,
                event_id,
            },
        ) if account_matches(&snapshot.state, Some(account_key))
            && focused_context_matches(&snapshot.state, room_id, event_id.as_deref()) =>
        {
            Some(RequestOutcome::FocusedContext {
                snapshot: snapshot.clone(),
            })
        }
        (
            EventProgress::Focused { opened: false, .. },
            RequestOutcomeExpectation::RoomSelected {
                room_id,
                account_key,
                ..
            },
        ) if snapshot.state.navigation.active_room_id.as_deref() == Some(room_id.as_str())
            && account_matches(&snapshot.state, account_key.as_ref()) =>
        {
            Some(RequestOutcome::RoomSelected {
                snapshot: snapshot.clone(),
            })
        }
        (
            EventProgress::Anchor { live_fallback, .. },
            RequestOutcomeExpectation::MainTimelineAnchor {
                request_id,
                key,
                event_id,
                ..
            },
        ) if account_matches(&snapshot.state, Some(&key.account_key))
            && timeline_key_matches(key, event_id)
            && if *live_fallback {
                snapshot_has_live_main_timeline(&snapshot.state, key.room_id())
            } else {
                snapshot_has_main_timeline_anchor(&snapshot.state, key.room_id(), event_id)
            } =>
        {
            Some(RequestOutcome::MainTimelineAnchor {
                snapshot: snapshot.clone(),
            })
        }
        (
            EventProgress::InviteBatch {
                request_id,
                room_id,
                user_ids,
                scope,
            },
            RequestOutcomeExpectation::RoomOperation {
                request_id: expected_request_id,
                account_key,
                room_id: expected_room_id,
                operation: RoomOperationKind::InviteBatch { .. },
            },
        ) if request_id == expected_request_id
            && room_id == expected_room_id
            && account_matches(&snapshot.state, Some(account_key))
            && invite_batch_snapshot_matches(
                &snapshot.state,
                request_id.sequence,
                room_id,
                user_ids,
                scope,
            ) =>
        {
            Some(RequestOutcome::RoomOperation {
                request_id: *expected_request_id,
                snapshot: snapshot.clone(),
            })
        }
        (
            EventProgress::RoomOperation {
                request_id,
                room_id,
                generation,
                ..
            },
            RequestOutcomeExpectation::RoomOperation {
                request_id: expected_request_id,
                account_key,
                room_id: expected_room_id,
                operation,
            },
        ) if request_id == expected_request_id
            && room_id == expected_room_id
            && account_matches(&snapshot.state, Some(account_key))
            && room_operation_snapshot_matches(
                &snapshot.state,
                request_id.sequence,
                room_id,
                generation,
                operation,
            ) =>
        {
            Some(RequestOutcome::RoomOperation {
                request_id: *expected_request_id,
                snapshot: snapshot.clone(),
            })
        }
        (
            EventProgress::InviteWorkflow { .. },
            RequestOutcomeExpectation::InviteWorkflow {
                request_id,
                account_key,
                room_id,
                query,
                closed,
            },
        ) if account_matches(&snapshot.state, Some(account_key))
            && if *closed {
                snapshot.state.invite_workflow == Default::default()
            } else {
                snapshot.state.invite_workflow.query.room_id.as_deref() == Some(room_id.as_str())
                    && snapshot.state.invite_workflow.query.query == *query
            } =>
        {
            Some(RequestOutcome::InviteWorkflow {
                request_id: *request_id,
                snapshot: snapshot.clone(),
            })
        }
        (
            EventProgress::Directory { request_id },
            RequestOutcomeExpectation::DirectoryQuery {
                request_id: expected_request_id,
                account_key,
            }
            | RequestOutcomeExpectation::DirectoryPreview {
                request_id: expected_request_id,
                account_key,
            },
        ) if request_id == expected_request_id
            && account_matches(&snapshot.state, Some(account_key)) =>
        {
            Some(RequestOutcome::Directory {
                request_id: *expected_request_id,
                snapshot: snapshot.clone(),
            })
        }
        (
            EventProgress::Search { .. },
            RequestOutcomeExpectation::SearchStarted {
                request_id,
                account_key,
                query,
                scope,
            },
        ) if account_matches(&snapshot.state, account_key.as_ref())
            && search_state_matches(&snapshot.state, request_id, query, scope) =>
        {
            Some(RequestOutcome::Search {
                request_id: *request_id,
                snapshot: snapshot.clone(),
            })
        }
        (
            EventProgress::Search { .. },
            RequestOutcomeExpectation::SearchClosed {
                request_id,
                account_key,
                ..
            },
        ) if account_matches(&snapshot.state, account_key.as_ref())
            && snapshot.state.search == koushi_state::SearchState::Closed =>
        {
            Some(RequestOutcome::Search {
                request_id: *request_id,
                snapshot: snapshot.clone(),
            })
        }
        (
            EventProgress::SubmissionAccepted {
                key,
                submission_id,
                transaction_id,
                ..
            },
            RequestOutcomeExpectation::Submission {
                request_id,
                account_key,
                target,
                submission_id: expected_submission_id,
            },
        ) if submission_id == expected_submission_id
            && timeline_key_matches_composer_target(key, account_key, target)
            && account_matches(&snapshot.state, Some(account_key))
            && snapshot
                .state
                .timeline
                .submission_registry
                .active_submissions
                .iter()
                .any(|active| {
                    &active.submission_id == submission_id
                        && active.transaction_id == *transaction_id
                        && active.target == *target
                }) =>
        {
            Some(RequestOutcome::SubmissionAccepted {
                request_id: *request_id,
                submission_id: submission_id.clone(),
                transaction_id: transaction_id.clone(),
                snapshot: snapshot.clone(),
            })
        }
        (
            EventProgress::SubmissionRejected {
                key,
                submission_id,
                kind,
                ..
            },
            RequestOutcomeExpectation::Submission {
                request_id,
                account_key,
                target,
                submission_id: expected_submission_id,
            },
        ) if submission_id == expected_submission_id
            && timeline_key_matches_composer_target(key, account_key, target) =>
        {
            Some(RequestOutcome::SubmissionRejected {
                request_id: *request_id,
                submission_id: submission_id.clone(),
                kind: *kind,
                snapshot: snapshot.clone(),
            })
        }
        (
            EventProgress::PreparedMediaQueued {
                key: event_key,
                transaction_id,
                ..
            },
            RequestOutcomeExpectation::PreparedMediaQueued {
                request_id,
                key: expected_key,
                transaction_id: expected_transaction_id,
            },
        ) if event_key == expected_key && transaction_id == expected_transaction_id => {
            Some(RequestOutcome::PreparedMediaQueued {
                request_id: *request_id,
                key: event_key.clone(),
                transaction_id: transaction_id.clone(),
                snapshot: snapshot.clone(),
            })
        }
        _ => None,
    }
}

fn room_operation_is_event_terminal(operation: &RoomOperationKind) -> bool {
    !matches!(
        operation,
        RoomOperationKind::InviteBatch { .. }
            | RoomOperationKind::RoomLeft
            | RoomOperationKind::RoomForgotten
            | RoomOperationKind::RoomSettingsLoaded
            | RoomOperationKind::RoomSettingUpdated
            | RoomOperationKind::SpaceMembersLoaded { .. }
            | RoomOperationKind::SpaceMemberInviteSettled { .. }
            | RoomOperationKind::SpaceMemberInviteCancellationSettled { .. }
            | RoomOperationKind::SpaceMemberRoleUpdated { .. }
    )
}

fn room_operation_snapshot_matches(
    state: &AppState,
    request_sequence: u64,
    room_id: &str,
    generation: &Option<u64>,
    operation: &RoomOperationKind,
) -> bool {
    match operation {
        RoomOperationKind::RoomLeft | RoomOperationKind::RoomForgotten => {
            !state.rooms.iter().any(|room| room.room_id == room_id)
        }
        RoomOperationKind::InviteBatch { user_ids, scope } => {
            invite_batch_snapshot_matches(state, request_sequence, room_id, user_ids, scope)
        }
        RoomOperationKind::RoomSettingsLoaded | RoomOperationKind::RoomSettingUpdated => state
            .room_management
            .settings
            .as_ref()
            .is_some_and(|settings| settings.room_id == room_id),
        RoomOperationKind::SpaceMembersLoaded {
            generation: expected_generation,
        }
        | RoomOperationKind::SpaceMemberInviteSettled {
            generation: expected_generation,
            ..
        }
        | RoomOperationKind::SpaceMemberInviteCancellationSettled {
            generation: expected_generation,
            ..
        }
        | RoomOperationKind::SpaceMemberRoleUpdated {
            generation: expected_generation,
            ..
        } => {
            state.space_members.selected_space_id.as_deref() == Some(room_id)
                && state.space_members.generation == *expected_generation
                && generation == &Some(*expected_generation)
        }
        _ => true,
    }
}

fn invite_batch_snapshot_matches(
    state: &AppState,
    request_sequence: u64,
    room_id: &str,
    user_ids: &[String],
    scope: &InviteScopeSelection,
) -> bool {
    let InviteOperationState::Completed {
        request_id,
        room_id: completed_room_id,
        results,
        ..
    } = &state.invite_workflow.operation
    else {
        return false;
    };
    *request_id == request_sequence
        && completed_room_id == room_id
        && invite_batch_matches(
            &RoomOperationKind::InviteBatch {
                user_ids: user_ids.to_vec(),
                scope: scope.clone(),
            },
            results,
        )
}

fn final_result(
    expectation: &RequestOutcomeExpectation,
    snapshot: &VersionedAppStateSnapshot,
    baseline_generation: u64,
    error: RequestOutcomeError,
    progress: Option<EventProgress>,
) -> Result<RequestOutcome, RequestOutcomeError> {
    if let Some(outcome) = snapshot_outcome(
        expectation,
        snapshot,
        baseline_generation,
        allows_initial_snapshot(expectation),
    ) {
        return Ok(outcome);
    }
    if let Some(progress) = progress
        .and_then(|progress| progress.snapshot_outcome(expectation, snapshot, baseline_generation))
    {
        return Ok(progress);
    }
    Err(error)
}

fn allows_initial_snapshot(expectation: &RequestOutcomeExpectation) -> bool {
    matches!(
        expectation,
        RequestOutcomeExpectation::RoomSelected {
            allow_initial: true,
            ..
        } | RequestOutcomeExpectation::SearchClosed {
            allow_initial: true,
            ..
        } | RequestOutcomeExpectation::UploadStaging {
            allow_initial: true,
            ..
        }
    )
}

fn composer_draft_revision(state: &AppState, target: &ComposerTarget) -> ComposerDraftRevision {
    match target {
        ComposerTarget::Main { room_id } => state.composer_drafts.room_revision(room_id),
        ComposerTarget::Thread {
            room_id,
            root_event_id,
        } => state
            .composer_drafts
            .thread_revision(room_id, root_event_id),
    }
}

fn composer_target_matches_snapshot(state: &AppState, target: &ComposerTarget) -> bool {
    match target {
        ComposerTarget::Main { room_id } => {
            state.timeline.room_id.as_deref() == Some(room_id.as_str())
        }
        ComposerTarget::Thread {
            room_id,
            root_event_id,
        } => matches!(
            &state.thread,
            koushi_state::ThreadPaneState::Open {
                room_id: current_room_id,
                root_event_id: current_root_event_id,
                ..
            } if current_room_id == room_id && current_root_event_id == root_event_id
        ),
    }
}

fn staged_upload_ids_match(
    state: &AppState,
    target: &ComposerTarget,
    expected_ids: &[String],
) -> bool {
    let items = match target {
        ComposerTarget::Main { room_id }
            if state.timeline.room_id.as_deref() == Some(room_id.as_str()) =>
        {
            Some(state.timeline.staged_uploads.as_slice())
        }
        ComposerTarget::Thread {
            room_id,
            root_event_id,
        } => match &state.thread {
            koushi_state::ThreadPaneState::Open {
                room_id: current_room_id,
                root_event_id: current_root_event_id,
                staged_uploads,
                ..
            } if current_room_id == room_id && current_root_event_id == root_event_id => {
                Some(staged_uploads.as_slice())
            }
            _ => None,
        },
        _ => None,
    };
    items.is_some_and(|items| {
        items.len() == expected_ids.len()
            && items
                .iter()
                .map(|item| item.staged_id.as_str())
                .eq(expected_ids.iter().map(String::as_str))
    })
}

fn timeline_key_matches_composer_target(
    key: &TimelineKey,
    account_key: &AccountKey,
    target: &ComposerTarget,
) -> bool {
    if key.account_key != *account_key {
        return false;
    }
    match (target, &key.kind) {
        (
            ComposerTarget::Main { room_id },
            koushi_protocol::ids::TimelineKind::Room {
                room_id: key_room_id,
            },
        ) => room_id == key_room_id,
        (
            ComposerTarget::Thread {
                room_id,
                root_event_id,
            },
            koushi_protocol::ids::TimelineKind::Thread {
                room_id: key_room_id,
                root_event_id: key_root_event_id,
            },
        ) => room_id == key_room_id && root_event_id == key_root_event_id,
        _ => false,
    }
}

fn snapshot_account_key(state: &AppState) -> Option<String> {
    match &state.session {
        SessionState::SwitchingAccount { info }
        | SessionState::Provisional { info, .. }
        | SessionState::AwaitingVerification { info, .. }
        | SessionState::Verifying { info, .. }
        | SessionState::AwaitingBootstrapConfirmation { info, .. }
        | SessionState::Rejecting { info, .. }
        | SessionState::Ready(info)
        | SessionState::Locked(info)
        | SessionState::CapabilityBlocked { info, .. } => Some(info.user_id.clone()),
        SessionState::SignedOut
        | SessionState::Restoring
        | SessionState::Authenticating { .. }
        | SessionState::LoggingOut => None,
    }
}

fn account_matches(state: &AppState, expected: Option<&AccountKey>) -> bool {
    expected
        .is_none_or(|expected| snapshot_account_key(state).as_deref() == Some(expected.0.as_str()))
}

fn session_is_login_transport_terminal(session: &SessionState) -> bool {
    matches!(
        session,
        SessionState::Provisional {
            phase: koushi_state::ProvisionalPhase::RecheckingTrust { failure: Some(_) },
            ..
        } | SessionState::AwaitingVerification { .. }
            | SessionState::Verifying { .. }
            | SessionState::AwaitingBootstrapConfirmation { .. }
            | SessionState::Rejecting { .. }
            | SessionState::Ready(_)
    )
}

fn auth_discovery_matches(state: &AppState, homeserver: &str) -> bool {
    matches!(
        &state.auth,
        koushi_state::AuthDiscoveryState::Ready { homeserver: current, .. }
            | koushi_state::AuthDiscoveryState::Failed { homeserver: current, .. }
            if current == homeserver
    )
}

fn room_target_matches(state: &AppState, expected_room_id: Option<&str>) -> bool {
    state.navigation.active_room_id.as_deref() == expected_room_id
}

fn focused_context_matches(state: &AppState, room_id: &str, event_id: Option<&str>) -> bool {
    match &state.focused_context {
        FocusedContextState::Opening {
            room_id: current_room_id,
            event_id: current_event_id,
        }
        | FocusedContextState::Open {
            room_id: current_room_id,
            event_id: current_event_id,
            ..
        } => {
            current_room_id == room_id
                && event_id.is_none_or(|expected| expected == current_event_id)
        }
        FocusedContextState::Closed => false,
    }
}

fn timeline_key_matches(key: &TimelineKey, event_id: &str) -> bool {
    matches!(
        &key.kind,
        koushi_protocol::ids::TimelineKind::Focused {
            room_id,
            event_id: key_event_id,
        } if room_id == key.room_id() && key_event_id == event_id
    )
}

fn snapshot_has_main_timeline_anchor(state: &AppState, room_id: &str, event_id: &str) -> bool {
    state.navigation.active_room_id.as_deref() == Some(room_id)
        && state
            .navigation
            .main_timeline_anchor
            .as_ref()
            .is_some_and(|anchor| anchor.event_id == event_id)
}

fn snapshot_has_live_main_timeline(state: &AppState, room_id: &str) -> bool {
    state.navigation.active_room_id.as_deref() == Some(room_id)
        && state.focused_context == FocusedContextState::Closed
        && state.navigation.main_timeline_anchor.is_none()
}

fn search_state_matches(
    state: &AppState,
    request_id: &RequestId,
    query: &str,
    scope: &SearchScope,
) -> bool {
    matches!(
        &state.search,
        koushi_state::SearchState::TooShort {
            request_id: state_request_id,
            query: state_query,
            scope: state_scope,
            ..
        }
        | koushi_state::SearchState::Searching {
            request_id: state_request_id,
            query: state_query,
            scope: state_scope,
        }
        | koushi_state::SearchState::Results {
            request_id: state_request_id,
            query: state_query,
            scope: state_scope,
            ..
        }
        | koushi_state::SearchState::Failed {
            request_id: state_request_id,
            query: state_query,
            scope: state_scope,
            ..
        } if *state_request_id == request_id.sequence
            && state_query == query
            && state_scope == scope
    )
}
