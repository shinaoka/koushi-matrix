//! Observer of reducer-owned navigation; private identities never leave this module.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, OnceLock};

use koushi_diagnostics::{
    NavigationCause, NavigationDiagnostic, NavigationOrigin, NavigationStage, record_navigation,
    reset_navigation,
};
use koushi_protocol::RequestId;
use koushi_state::{
    AppAction, AppState, EventNavigationSource, EventNavigationState, MainTimelineAnchor,
    SessionInfo, SessionState,
};

const ALIAS_CAPACITY: usize = 2048;

// Deliberately no Debug/Serialize implementations on private identifier owners.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd)]
struct AccountIdentity {
    homeserver: String,
    user: String,
    device: String,
    epoch: u64,
}

impl AccountIdentity {
    fn new(info: &SessionInfo, epoch: u64) -> Self {
        Self {
            homeserver: info.homeserver.clone(),
            user: info.user_id.clone(),
            device: info.device_id.clone(),
            epoch,
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
struct Capture {
    account: Option<AccountIdentity>,
    space: Option<String>,
    room: Option<String>,
    dm: Option<bool>,
    event: EventNavigationState,
    anchor: Option<MainTimelineAnchor>,
}

impl Capture {
    fn from_state(state: &AppState) -> Self {
        let info = match &state.session {
            SessionState::Ready(info)
            | SessionState::Locked(info)
            | SessionState::SwitchingAccount { info }
            | SessionState::Provisional { info, .. }
            | SessionState::AwaitingVerification { info, .. }
            | SessionState::Verifying { info, .. }
            | SessionState::AwaitingBootstrapConfirmation { info, .. }
            | SessionState::Rejecting { info, .. }
            | SessionState::CapabilityBlocked { info, .. } => Some(info),
            _ => None,
        };
        Self {
            event: state.navigation.event_navigation,
            anchor: state.navigation.main_timeline_anchor.clone(),
            account: info.map(|info| AccountIdentity::new(info, state.session_disclosure_epoch)),
            space: state.navigation.active_space_id.clone(),
            room: state.navigation.active_room_id.clone(),
            dm: state.navigation.active_room_id.as_ref().and_then(|id| {
                state
                    .rooms
                    .iter()
                    .find(|room| &room.room_id == id)
                    .map(|room| room.is_dm)
            }),
        }
    }

    fn navigation_changed(&self, after: &Self) -> bool {
        self.account != after.account
            || self.space != after.space
            || self.room != after.room
            || self.event != after.event
            || self.anchor != after.anchor
    }
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd)]
enum AliasKey {
    Account(AccountIdentity),
    Space(Option<AccountIdentity>, String),
    Room(Option<AccountIdentity>, String),
}

#[derive(Default)]
struct Aliases {
    values: BTreeMap<AliasKey, u64>,
    serial: u64,
}

impl Aliases {
    fn reset(&mut self) {
        self.values.clear();
        // Serial survives: aliases from earlier records are never reassigned.
        reset_navigation();
    }

    fn prepare(&mut self, keys: &[AliasKey]) {
        let additional = keys
            .iter()
            .filter(|key| !self.values.contains_key(*key))
            .collect::<BTreeSet<_>>()
            .len();
        if self.values.len() + additional > ALIAS_CAPACITY {
            self.reset();
        }
    }

    fn alias(&mut self, key: AliasKey) -> Option<u64> {
        if let Some(value) = self.values.get(&key) {
            return Some(*value);
        }
        if self.values.len() == ALIAS_CAPACITY {
            self.reset();
        }
        self.serial = self.serial.checked_add(1)?;
        self.values.insert(key, self.serial);
        Some(self.serial)
    }
}

static ALIASES: OnceLock<Mutex<Aliases>> = OnceLock::new();

// No latest-command slot: observations own their own exact before/request facts.
pub(crate) struct NavigationObservation {
    authoritative: bool,
    before: Capture,
    requested: Capture,
    cause: NavigationCause,
    origin: NavigationOrigin,
    request: Option<RequestId>,
    explicit: bool,
    rejected: bool,
    list_authoritative: Option<bool>,
    event_failure: bool,
    completion: bool,
}

impl NavigationObservation {
    pub(crate) fn begin(
        state: &AppState,
        action: &AppAction,
        origin: NavigationOrigin,
        request: Option<RequestId>,
    ) -> Self {
        let before = Capture::from_state(state);
        let mut requested = before.clone();
        requested.space = None;
        requested.room = None;
        requested.dm = None;
        let ready = matches!(state.session, SessionState::Ready(_));
        let (cause, explicit, rejected, list_authoritative) = match action {
            AppAction::SelectRoom { room_id } => {
                requested.room = Some(room_id.clone());
                (
                    NavigationCause::SelectRoom,
                    true,
                    !ready || !state.rooms.iter().any(|room| &room.room_id == room_id),
                    None,
                )
            }
            AppAction::SelectSpace { space_id } => {
                requested.space = space_id.clone();
                (
                    NavigationCause::SelectSpace,
                    true,
                    !ready
                        || space_id.as_ref().is_some_and(|id| {
                            !state.spaces.iter().any(|space| &space.space_id == id)
                        }),
                    None,
                )
            }
            // #1850: the cancel-restore re-selects the Space the displaced
            // navigation left, with no room open.
            AppAction::SelectSpaceWithoutRoom { space_id } => {
                requested.space = Some(space_id.clone());
                (
                    NavigationCause::SelectSpace,
                    true,
                    !ready || !state.spaces.iter().any(|space| &space.space_id == space_id),
                    None,
                )
            }
            // The legacy replacement handler passes authoritative=true to the
            // same reducer helper as RoomListSnapshotAuthoritative.
            AppAction::RoomListUpdated { .. } => (
                NavigationCause::RoomListReplacement,
                false,
                false,
                Some(true),
            ),
            AppAction::RoomLeftLocally { .. } => {
                (NavigationCause::RoomListReplacement, false, false, None)
            }
            AppAction::RoomListSnapshotProvisional { .. } => (
                NavigationCause::RoomListReplacement,
                false,
                false,
                Some(false),
            ),
            AppAction::RoomListSnapshotAuthoritative { .. } => (
                NavigationCause::RoomListReplacement,
                false,
                false,
                Some(true),
            ),
            AppAction::EventNavigationStarted { source } => (
                match source {
                    EventNavigationSource::Notification => NavigationCause::Notification,
                    EventNavigationSource::Activity => NavigationCause::ActivityNavigation,
                    EventNavigationSource::Search => NavigationCause::SearchNavigation,
                    EventNavigationSource::Pinned => NavigationCause::PinnedNavigation,
                    EventNavigationSource::ReplyQuote => NavigationCause::ReplyQuoteNavigation,
                },
                true,
                !ready,
                None,
            ),
            AppAction::EnterAnchoredTimeline { room_id, .. }
            | AppAction::ReturnMainTimelineToLive { room_id } => {
                requested.room = Some(room_id.clone());
                (
                    NavigationCause::EventNavigation,
                    true,
                    !ready || state.navigation.active_room_id.as_ref() != Some(room_id),
                    None,
                )
            }
            AppAction::EventNavigationAnchored { .. }
            | AppAction::EventNavigationLiveFallback { .. }
            | AppAction::EventNavigationFailed { .. }
            | AppAction::EventNavigationCleared => {
                (NavigationCause::EventNavigation, true, false, None)
            }
            AppAction::SwitchAccountRequested { info } => {
                requested.account =
                    Some(AccountIdentity::new(info, state.session_disclosure_epoch));
                (NavigationCause::SessionReset, true, false, None)
            }
            AppAction::LogoutRequested
            | AppAction::LogoutFinished
            | AppAction::RestoreSessionNotFound
            | AppAction::ProvisionalSessionDiscarded => {
                requested.account = None;
                (NavigationCause::SessionReset, true, false, None)
            }
            AppAction::RestoreSessionSucceeded(info) | AppAction::LoginSucceeded { info, .. } => {
                requested.account =
                    Some(AccountIdentity::new(info, state.session_disclosure_epoch));
                (
                    if matches!(action, AppAction::RestoreSessionSucceeded(_)) {
                        NavigationCause::StartupRestoration
                    } else {
                        NavigationCause::SessionReset
                    },
                    true,
                    false,
                    None,
                )
            }
            AppAction::AppStarted => (NavigationCause::StartupRestoration, true, false, None),
            AppAction::SessionLocked
            | AppAction::SessionAuthenticationInvalidated { .. }
            | AppAction::RestoreSessionRequested
            | AppAction::RestoreSessionFailed { .. } => {
                (NavigationCause::SessionReset, true, false, None)
            }
            _ => (NavigationCause::Unknown, false, false, None),
        };
        let observation = Self {
            authoritative: true,
            before,
            requested,
            cause,
            origin,
            request,
            explicit,
            rejected,
            list_authoritative,
            event_failure: matches!(action, AppAction::EventNavigationFailed { .. }),
            completion: matches!(
                action,
                AppAction::EventNavigationAnchored { .. }
                    | AppAction::EventNavigationLiveFallback { .. }
            ),
        };
        if matches!(
            action,
            AppAction::SelectRoom { .. }
                | AppAction::SelectSpace { .. }
                | AppAction::SelectSpaceWithoutRoom { .. }
                | AppAction::EventNavigationStarted { .. }
                | AppAction::EnterAnchoredTimeline { .. }
                | AppAction::ReturnMainTimelineToLive { .. }
                | AppAction::LogoutRequested
                | AppAction::SwitchAccountRequested { .. }
                | AppAction::RestoreSessionRequested
        ) {
            observation.emit(state, &observation.before, NavigationStage::Admitted, false);
        }
        observation
    }

    pub(crate) fn finish(&self, state: &AppState) {
        let after = Capture::from_state(state);
        let changed = self.before.navigation_changed(&after);
        if !changed && !self.explicit {
            return;
        }
        let stage = if self.rejected || (changed && self.event_failure) {
            NavigationStage::Rejected
        } else if changed && self.completion {
            NavigationStage::Completed
        } else if changed {
            NavigationStage::Committed
        } else {
            NavigationStage::NoOp
        };
        self.emit(state, &after, stage, self.before.account != after.account);
    }

    fn emit(&self, state: &AppState, after: &Capture, stage: NavigationStage, reset: bool) {
        self.emit_record(state, after, stage, reset, false);
    }

    fn emit_record(
        &self,
        state: &AppState,
        after: &Capture,
        stage: NavigationStage,
        reset: bool,
        selected_changed: bool,
    ) -> NavigationDiagnostic {
        let mut aliases = ALIASES
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if reset {
            aliases.reset();
        }
        let captures = [&self.before, &self.requested, after];
        let keys = captures
            .iter()
            .flat_map(|capture| {
                let mut keys = Vec::with_capacity(3);
                if let Some(account) = &capture.account {
                    keys.push(AliasKey::Account(account.clone()));
                }
                if let Some(space) = &capture.space {
                    keys.push(AliasKey::Space(capture.account.clone(), space.clone()));
                }
                if let Some(room) = &capture.room {
                    keys.push(AliasKey::Room(capture.account.clone(), room.clone()));
                }
                keys
            })
            .collect::<Vec<_>>();
        aliases.prepare(&keys);
        let mut values = [[None; 3]; 3];
        for (index, capture) in captures.iter().enumerate() {
            values[index][0] = capture
                .account
                .clone()
                .and_then(|account| aliases.alias(AliasKey::Account(account)));
            values[index][1] = capture.space.as_ref().and_then(|space| {
                aliases.alias(AliasKey::Space(capture.account.clone(), space.clone()))
            });
            values[index][2] = capture.room.as_ref().and_then(|room| {
                aliases.alias(AliasKey::Room(capture.account.clone(), room.clone()))
            });
        }
        let diagnostic = NavigationDiagnostic {
            authoritative_transition: self.authoritative
                && (selected_changed || self.before.navigation_changed(after)),
            cause: if self.cause == NavigationCause::Unknown && self.before.account != after.account
            {
                NavigationCause::SessionReset
            } else {
                self.cause
            },
            origin: self.origin,
            stage,
            request_connection: self.request.map(|request| request.connection_id.0),
            request_sequence: self.request.map(|request| request.sequence),
            previous_account_alias: values[0][0],
            requested_account_alias: values[1][0],
            committed_account_alias: values[2][0],
            previous_space_alias: values[0][1],
            requested_space_alias: values[1][1],
            committed_space_alias: values[2][1],
            previous_room_alias: values[0][2],
            requested_room_alias: values[1][2],
            committed_room_alias: values[2][2],
            old_room_present: self
                .before
                .room
                .as_ref()
                .map(|id| state.rooms.iter().any(|room| &room.room_id == id)),
            old_dm: self.before.dm,
            new_dm: after.dm,
            list_authoritative: self.list_authoritative,
        };
        record_navigation(diagnostic);
        diagnostic
    }
}

#[cfg(test)]
#[path = "navigation_diagnostics_tests.rs"]
mod tests;

/// A private caller operation keeps its own before-state and request, including cancellation.
pub struct NavigationOperation {
    observation: NavigationObservation,
    snapshots:
        tokio::sync::watch::Receiver<koushi_protocol::state_update::VersionedAppStateSnapshot>,
    outcome: NavigationStage,
}
impl NavigationOperation {
    pub(crate) fn new(
        snapshots: tokio::sync::watch::Receiver<
            koushi_protocol::state_update::VersionedAppStateSnapshot,
        >,
        cause: NavigationCause,
        origin: NavigationOrigin,
        request: RequestId,
    ) -> Self {
        let capture = Capture::from_state(&snapshots.borrow().state);
        let observation = NavigationObservation {
            authoritative: false,
            before: capture.clone(),
            requested: capture.clone(),
            cause,
            origin,
            request: Some(request),
            explicit: true,
            rejected: false,
            list_authoritative: None,
            event_failure: false,
            completion: false,
        };
        observation.emit(
            &snapshots.borrow().state,
            &capture,
            NavigationStage::Admitted,
            false,
        );
        Self {
            observation,
            snapshots,
            outcome: NavigationStage::Superseded,
        }
    }
    pub(crate) fn room_selection(
        snapshots: tokio::sync::watch::Receiver<
            koushi_protocol::state_update::VersionedAppStateSnapshot,
        >,
        origin: NavigationOrigin,
        request: RequestId,
        room_id: String,
    ) -> Self {
        let capture = Capture::from_state(&snapshots.borrow().state);
        let mut requested = capture.clone();
        requested.room = Some(room_id);
        let observation = NavigationObservation {
            authoritative: false,
            before: capture.clone(),
            requested,
            cause: NavigationCause::SelectRoom,
            origin,
            request: Some(request),
            explicit: true,
            rejected: false,
            list_authoritative: None,
            event_failure: false,
            completion: false,
        };
        observation.emit(
            &snapshots.borrow().state,
            &capture,
            NavigationStage::Admitted,
            false,
        );
        Self {
            observation,
            snapshots,
            outcome: NavigationStage::Superseded,
        }
    }
    pub fn finish(mut self, success: bool) {
        self.outcome = if success {
            NavigationStage::Completed
        } else {
            NavigationStage::Rejected
        };
    }
}
impl Drop for NavigationOperation {
    fn drop(&mut self) {
        let state = self.snapshots.borrow();
        let after = Capture::from_state(&state.state);
        self.observation
            .emit(&state.state, &after, self.outcome, false);
    }
}

#[cfg(test)]
pub(crate) fn record_account_selection(before: &AppState, after: &AppState, changed: bool) {
    record_account_selection_with_cause(before, after, changed, NavigationCause::SelectAccount);
}

#[cfg(test)]
pub(crate) fn record_account_selection_with_cause(
    before: &AppState,
    after: &AppState,
    changed: bool,
    cause: NavigationCause,
) {
    AccountSelectionObservation::new(before).finish(after, changed, cause);
}

/// Owns only private navigation metadata, never an application snapshot.
pub(crate) struct AccountSelectionObservation {
    before: Capture,
}
impl AccountSelectionObservation {
    pub(crate) fn new(state: &AppState) -> Self {
        Self {
            before: Capture::from_state(state),
        }
    }
    pub(crate) fn finish(
        self,
        after: &AppState,
        changed: bool,
        cause: NavigationCause,
    ) -> NavigationDiagnostic {
        let observation = NavigationObservation {
            authoritative: true,
            before: self.before,
            requested: Capture::from_state(after),
            cause,
            origin: NavigationOrigin::Unknown,
            request: None,
            explicit: true,
            rejected: false,
            list_authoritative: None,
            event_failure: false,
            completion: false,
        };
        let capture = Capture::from_state(after);
        observation.emit_record(
            after,
            &capture,
            if changed {
                NavigationStage::Committed
            } else {
                NavigationStage::NoOp
            },
            false,
            changed,
        )
    }
}
