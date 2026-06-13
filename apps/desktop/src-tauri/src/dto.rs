//! Data-transfer objects: Rust → TypeScript serialization contract.
//!
//! `FrontendDesktopSnapshot` is built from `AppStateSnapshot` (the core state
//! projection). Timeline items and thread messages are REMOVED from the
//! snapshot in Phase 7; they flow as `CoreEvent::Timeline` diffs over
//! `matrix-desktop://event`. The TS types.ts contract keeps `timeline` and
//! `thread` fields for backward compat; the adapter now always sends `[]` /
//! `null` and the React timeline store populates them from events.
//!
//! References: overview.md "Async rule 4" — timeline items never in AppState.

use matrix_desktop_state::{
    AppError, AppState, AuthDiscoveryState, BasicOperationState, ComposerState, NavigationState,
    RecoveryMethod, RoomSummary, SearchMatchField, SearchMatchKind, SearchResult, SearchScope,
    SearchState, SessionState, SidebarModel, SpaceSummary, SyncState, ThreadPaneState,
    TimelinePaneState,
};
use serde::{Deserialize, Serialize};

/// The snapshot returned by all Tauri commands.
///
/// `timeline` and `thread` are always empty / `None` in Phase 7; timeline
/// items flow as `TimelineEvent` diffs over `matrix-desktop://event`.
#[derive(Clone, Debug, Serialize)]
pub struct FrontendDesktopSnapshot {
    pub state: FrontendAppState,
    pub sidebar: SidebarModel,
    /// Always empty in Phase 7; timeline items flow as diffs.
    pub timeline: Vec<()>,
    /// Always None in Phase 7; thread flow as events.
    pub thread: Option<()>,
}

impl From<AppState> for FrontendDesktopSnapshot {
    fn from(state: AppState) -> Self {
        let sidebar = matrix_desktop_state::compose_sidebar(
            state.navigation.active_space_id.as_deref(),
            &state.spaces,
            &state.rooms,
        );
        Self {
            state: state.into(),
            sidebar,
            timeline: Vec::new(),
            thread: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FrontendAppState {
    pub session: FrontendSessionState,
    pub auth: AuthDiscoveryState,
    pub sync: FrontendSyncState,
    pub navigation: NavigationState,
    pub spaces: Vec<SpaceSummary>,
    pub rooms: Vec<RoomSummary>,
    pub timeline: TimelinePaneState,
    pub thread: FrontendThreadPaneState,
    pub search: FrontendSearchState,
    pub basic_operation: BasicOperationState,
    pub errors: Vec<AppError>,
}

impl From<AppState> for FrontendAppState {
    fn from(state: AppState) -> Self {
        Self {
            session: state.session.into(),
            auth: state.auth,
            sync: state.sync.into(),
            navigation: state.navigation,
            spaces: state.spaces,
            rooms: state.rooms,
            timeline: state.timeline,
            thread: state.thread.into(),
            search: state.search.into(),
            basic_operation: state.basic_operation,
            errors: state.errors,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FrontendSessionState {
    SignedOut,
    Restoring,
    SwitchingAccount {
        homeserver: String,
        user_id: String,
        device_id: String,
    },
    Authenticating {
        homeserver: String,
    },
    NeedsRecovery {
        homeserver: String,
        user_id: String,
        device_id: String,
        recovery_methods: Vec<RecoveryMethod>,
    },
    Recovering {
        homeserver: String,
        user_id: String,
        device_id: String,
        recovery_methods: Vec<RecoveryMethod>,
    },
    Ready {
        homeserver: String,
        user_id: String,
        device_id: String,
    },
    Locked {
        homeserver: String,
        user_id: String,
        device_id: String,
    },
    LoggingOut,
}

impl From<SessionState> for FrontendSessionState {
    fn from(session: SessionState) -> Self {
        match session {
            SessionState::SignedOut => Self::SignedOut,
            SessionState::Restoring => Self::Restoring,
            SessionState::SwitchingAccount { info } => Self::SwitchingAccount {
                homeserver: info.homeserver,
                user_id: info.user_id,
                device_id: info.device_id,
            },
            SessionState::Authenticating { homeserver } => Self::Authenticating { homeserver },
            SessionState::NeedsRecovery { info, methods } => Self::NeedsRecovery {
                homeserver: info.homeserver,
                user_id: info.user_id,
                device_id: info.device_id,
                recovery_methods: methods,
            },
            SessionState::Recovering { info, methods } => Self::Recovering {
                homeserver: info.homeserver,
                user_id: info.user_id,
                device_id: info.device_id,
                recovery_methods: methods,
            },
            SessionState::Ready(info) => Self::Ready {
                homeserver: info.homeserver,
                user_id: info.user_id,
                device_id: info.device_id,
            },
            SessionState::Locked(info) => Self::Locked {
                homeserver: info.homeserver,
                user_id: info.user_id,
                device_id: info.device_id,
            },
            SessionState::LoggingOut => Self::LoggingOut,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum FrontendSyncState {
    Name(&'static str),
    Failed { failed: String },
    Reconnecting { reconnecting: String },
}

impl From<SyncState> for FrontendSyncState {
    fn from(sync: SyncState) -> Self {
        match sync {
            SyncState::Stopped => Self::Name("stopped"),
            SyncState::Starting => Self::Name("starting"),
            SyncState::Running => Self::Name("running"),
            SyncState::Failed { reason } => Self::Failed { failed: reason },
            SyncState::Reconnecting { reason } => Self::Reconnecting {
                reconnecting: reason,
            },
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FrontendThreadPaneState {
    Closed,
    Opening {
        room_id: String,
        root_event_id: String,
    },
    Open {
        room_id: String,
        root_event_id: String,
        is_subscribed: bool,
        composer: ComposerState,
    },
}

impl From<ThreadPaneState> for FrontendThreadPaneState {
    fn from(thread: ThreadPaneState) -> Self {
        match thread {
            ThreadPaneState::Closed => Self::Closed,
            ThreadPaneState::Opening {
                room_id,
                root_event_id,
            } => Self::Opening {
                room_id,
                root_event_id,
            },
            ThreadPaneState::Open {
                room_id,
                root_event_id,
                is_subscribed,
                composer,
            } => Self::Open {
                room_id,
                root_event_id,
                is_subscribed,
                composer,
            },
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FrontendSearchState {
    Closed,
    Editing {
        query: String,
        scope: SearchScopeKind,
    },
    Searching {
        request_id: u64,
        query: String,
        scope: SearchScopeKind,
    },
    Results {
        request_id: u64,
        query: String,
        scope: SearchScopeKind,
        results: Vec<FrontendSearchResult>,
    },
    Failed {
        request_id: u64,
        query: String,
        scope: SearchScopeKind,
        message: String,
    },
}

impl From<SearchState> for FrontendSearchState {
    fn from(search: SearchState) -> Self {
        match search {
            SearchState::Closed => Self::Closed,
            SearchState::Editing { query, scope } => Self::Editing {
                query,
                scope: scope.into(),
            },
            SearchState::Searching {
                request_id,
                query,
                scope,
            } => Self::Searching {
                request_id,
                query,
                scope: scope.into(),
            },
            SearchState::Results {
                request_id,
                query,
                scope,
                results,
            } => Self::Results {
                request_id,
                query,
                scope: scope.into(),
                results: results.into_iter().map(Into::into).collect(),
            },
            SearchState::Failed {
                request_id,
                query,
                scope,
                message,
            } => Self::Failed {
                request_id,
                query,
                scope: scope.into(),
                message,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchScopeKind {
    CurrentRoom,
    CurrentSpace,
    Dms,
    AllRooms,
}

impl SearchScopeKind {
    #[allow(dead_code)]
    pub fn resolve(self, state: &AppState) -> SearchScope {
        match self {
            Self::CurrentRoom => state
                .navigation
                .active_room_id
                .as_ref()
                .map(|room_id| SearchScope::CurrentRoom {
                    room_id: room_id.clone(),
                })
                .unwrap_or(SearchScope::AllRooms),
            Self::CurrentSpace => state
                .navigation
                .active_space_id
                .as_ref()
                .map(|space_id| SearchScope::CurrentSpace {
                    space_id: space_id.clone(),
                })
                .unwrap_or(SearchScope::AllRooms),
            Self::Dms => SearchScope::Dms,
            Self::AllRooms => SearchScope::AllRooms,
        }
    }
}

impl From<SearchScope> for SearchScopeKind {
    fn from(scope: SearchScope) -> Self {
        match scope {
            SearchScope::CurrentRoom { .. } => Self::CurrentRoom,
            SearchScope::CurrentSpace { .. } => Self::CurrentSpace,
            SearchScope::Dms => Self::Dms,
            SearchScope::AllRooms => Self::AllRooms,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FrontendSearchResult {
    pub room_id: String,
    pub event_id: String,
    pub sender: String,
    pub timestamp_ms: u64,
    pub score_millis: u32,
    pub snippet: String,
    pub match_field: FrontendSearchMatchField,
    pub highlights: Vec<matrix_desktop_state::TextRange>,
    pub match_kind: FrontendSearchMatchKind,
}

impl From<SearchResult> for FrontendSearchResult {
    fn from(result: SearchResult) -> Self {
        Self {
            room_id: result.room_id,
            event_id: result.event_id,
            sender: result.sender,
            timestamp_ms: result.timestamp_ms,
            score_millis: result.score_millis,
            snippet: result.snippet,
            match_field: result.match_field.into(),
            highlights: result.highlights,
            match_kind: result.match_kind.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FrontendSearchMatchField {
    MessageBody,
    AttachmentFileName,
}

impl From<SearchMatchField> for FrontendSearchMatchField {
    fn from(field: SearchMatchField) -> Self {
        match field {
            SearchMatchField::MessageBody => Self::MessageBody,
            SearchMatchField::AttachmentFileName => Self::AttachmentFileName,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FrontendSearchMatchKind {
    Exact,
}

impl From<SearchMatchKind> for FrontendSearchMatchKind {
    fn from(kind: SearchMatchKind) -> Self {
        match kind {
            SearchMatchKind::Exact => Self::Exact,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{FrontendDesktopSnapshot, FrontendSyncState};
    use matrix_desktop_state::{AppState, RecoveryMethod, SessionInfo, SessionState, SyncState};

    fn booted_app_state() -> AppState {
        AppState {
            session: SessionState::Ready(SessionInfo {
                homeserver: "https://matrix.org".to_owned(),
                user_id: "@user:matrix.org".to_owned(),
                device_id: "DEVICE".to_owned(),
            }),
            sync: SyncState::Running,
            ..AppState::default()
        }
    }

    #[test]
    fn frontend_snapshot_serializes_to_the_typescript_contract() {
        let state = booted_app_state();
        let value = serde_json::to_value(FrontendDesktopSnapshot::from(state))
            .expect("snapshot should serialize");

        assert_eq!(value["state"]["session"]["kind"], json!("ready"));
        assert_eq!(
            value["state"]["session"]["homeserver"],
            json!("https://matrix.org")
        );
        assert_eq!(value["state"]["sync"], json!("running"));
        // Phase 7: timeline is always [] (items flow as diffs)
        assert_eq!(value["timeline"], json!([]));
        // Phase 7: the legacy top-level thread is always null...
        assert_eq!(value["thread"], json!(null));
        // ...product thread state lives in state.thread (default Closed). The UI
        // reads the open/closed decision from here, not the legacy placeholder.
        assert_eq!(value["state"]["thread"]["kind"], json!("closed"));
        // basic_operation must be present (default Idle) so the UI can read
        // snapshot.state.basic_operation.kind without crashing.
        assert_eq!(value["state"]["basic_operation"]["kind"], json!("idle"));
        // composer.mode must be present (default Plain) for the same reason.
        assert_eq!(value["state"]["timeline"]["composer"]["mode"], json!("Plain"));
    }

    #[test]
    fn frontend_snapshot_serializes_e2ee_recovery_step() {
        let state = AppState {
            session: SessionState::NeedsRecovery {
                info: SessionInfo {
                    homeserver: "https://matrix.org".to_owned(),
                    user_id: "@user:matrix.org".to_owned(),
                    device_id: "DEVICE".to_owned(),
                },
                methods: vec![RecoveryMethod::RecoveryKey, RecoveryMethod::SecurityPhrase],
            },
            sync: SyncState::Running,
            ..AppState::default()
        };

        let value = serde_json::to_value(FrontendDesktopSnapshot::from(state))
            .expect("snapshot should serialize");

        assert_eq!(value["state"]["session"]["kind"], json!("needsRecovery"));
        assert_eq!(
            value["state"]["session"]["recovery_methods"],
            json!(["recoveryKey", "securityPhrase"])
        );
        assert_eq!(value["state"]["sync"], json!("running"));
    }

    #[test]
    fn frontend_sync_state_serializes_failed_and_reconnecting() {
        assert_eq!(
            serde_json::to_value(FrontendSyncState::from(SyncState::Failed {
                reason: "limited network".to_owned(),
            }))
            .expect("failed sync should serialize"),
            json!({ "failed": "limited network" })
        );
        assert_eq!(
            serde_json::to_value(FrontendSyncState::from(SyncState::Reconnecting {
                reason: "limited network".to_owned(),
            }))
            .expect("reconnecting sync should serialize"),
            json!({ "reconnecting": "limited network" })
        );
    }
}
