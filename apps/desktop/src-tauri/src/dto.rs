use matrix_desktop_backend::{DesktopSnapshot, ThreadMessage, ThreadSnapshot, TimelineMessage};
use matrix_desktop_state::{
    AppError, AppState, ComposerState, NavigationState, RoomSummary, SearchMatchField,
    SearchMatchKind, SearchResult, SearchScope, SearchState, SessionState, SidebarModel,
    SpaceSummary, SyncState, ThreadPaneState, TimelinePaneState,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize)]
pub struct FrontendDesktopSnapshot {
    pub state: FrontendAppState,
    pub sidebar: SidebarModel,
    pub timeline: Vec<TimelineMessage>,
    pub thread: Option<ThreadSnapshot>,
}

impl From<DesktopSnapshot> for FrontendDesktopSnapshot {
    fn from(snapshot: DesktopSnapshot) -> Self {
        Self {
            state: snapshot.state.into(),
            sidebar: snapshot.sidebar,
            timeline: snapshot.timeline,
            thread: snapshot.thread,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FrontendAppState {
    pub session: FrontendSessionState,
    pub sync: FrontendSyncState,
    pub navigation: NavigationState,
    pub spaces: Vec<SpaceSummary>,
    pub rooms: Vec<RoomSummary>,
    pub timeline: TimelinePaneState,
    pub thread: FrontendThreadPaneState,
    pub search: FrontendSearchState,
    pub errors: Vec<AppError>,
}

impl From<AppState> for FrontendAppState {
    fn from(state: AppState) -> Self {
        Self {
            session: state.session.into(),
            sync: state.sync.into(),
            navigation: state.navigation,
            spaces: state.spaces,
            rooms: state.rooms,
            timeline: state.timeline,
            thread: state.thread.into(),
            search: state.search.into(),
            errors: state.errors,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FrontendSessionState {
    SignedOut,
    Restoring,
    Authenticating {
        homeserver: String,
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
            SessionState::Authenticating { homeserver } => Self::Authenticating { homeserver },
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
    Recovering { recovering: String },
}

impl From<SyncState> for FrontendSyncState {
    fn from(sync: SyncState) -> Self {
        match sync {
            SyncState::Stopped => Self::Name("stopped"),
            SyncState::Starting => Self::Name("starting"),
            SyncState::Running => Self::Name("running"),
            SyncState::Recovering { reason } => Self::Recovering { recovering: reason },
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

#[allow(dead_code)]
fn _assert_snapshot_children_are_serializable(
    _: TimelineMessage,
    _: ThreadMessage,
    _: ThreadSnapshot,
) {
}

#[cfg(test)]
mod tests {
    use matrix_desktop_backend::FakeDesktopBackend;
    use matrix_desktop_state::SearchScope;
    use serde_json::json;

    use super::FrontendDesktopSnapshot;

    #[test]
    fn frontend_snapshot_serializes_to_the_typescript_contract() {
        let mut backend = FakeDesktopBackend::booted();
        backend.submit_search("Zoom", SearchScope::AllRooms);

        let value = serde_json::to_value(FrontendDesktopSnapshot::from(backend.snapshot()))
            .expect("snapshot should serialize");

        assert_eq!(value["state"]["session"]["kind"], json!("ready"));
        assert_eq!(
            value["state"]["session"]["homeserver"],
            json!("https://matrix.org")
        );
        assert_eq!(value["state"]["sync"], json!("running"));
        assert_eq!(value["state"]["thread"]["kind"], json!("open"));
        assert_eq!(value["state"]["search"]["kind"], json!("results"));
        assert_eq!(
            value["state"]["search"]["results"][0]["match_field"],
            json!("messageBody")
        );
        assert_eq!(
            value["state"]["search"]["results"][0]["match_kind"],
            json!("exact")
        );
        assert_eq!(
            value["state"]["search"]["results"][0]["highlights"][0],
            json!({ "start_utf16": 33, "end_utf16": 37 })
        );
    }
}
