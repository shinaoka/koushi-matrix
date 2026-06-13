use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AppState {
    pub session: SessionState,
    pub auth: AuthDiscoveryState,
    pub sync: SyncState,
    pub navigation: NavigationState,
    pub spaces: Vec<SpaceSummary>,
    pub rooms: Vec<RoomSummary>,
    pub timeline: TimelinePaneState,
    pub thread: ThreadPaneState,
    pub search: SearchState,
    pub errors: Vec<AppError>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            session: SessionState::SignedOut,
            auth: AuthDiscoveryState::Unknown,
            sync: SyncState::Stopped,
            navigation: NavigationState::default(),
            spaces: Vec::new(),
            rooms: Vec::new(),
            timeline: TimelinePaneState::default(),
            thread: ThreadPaneState::Closed,
            search: SearchState::Closed,
            errors: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AuthDiscoveryState {
    Unknown,
    Discovering {
        homeserver: String,
    },
    Ready {
        homeserver: String,
        flows: Vec<LoginFlow>,
    },
    Failed {
        homeserver: String,
        message: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LoginFlow {
    pub kind: LoginFlowKind,
    pub delegated_oidc_compatibility: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LoginFlowKind {
    Password,
    Sso,
    Token,
    Unknown(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SessionState {
    SignedOut,
    Restoring,
    SwitchingAccount {
        info: SessionInfo,
    },
    Authenticating {
        homeserver: String,
    },
    NeedsRecovery {
        info: SessionInfo,
        methods: Vec<RecoveryMethod>,
    },
    Recovering {
        info: SessionInfo,
        methods: Vec<RecoveryMethod>,
    },
    Ready(SessionInfo),
    Locked(SessionInfo),
    LoggingOut,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecoveryMethod {
    RecoveryKey,
    SecurityPhrase,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum E2eeRecoveryState {
    Unknown,
    Enabled,
    Disabled,
    Incomplete,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub homeserver: String,
    pub user_id: String,
    pub device_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SyncState {
    Stopped,
    Starting,
    Running,
    Failed { reason: String },
    Reconnecting { reason: String },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct NavigationState {
    pub active_space_id: Option<String>,
    pub active_room_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SpaceSummary {
    pub space_id: String,
    pub display_name: String,
    pub child_room_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomSummary {
    pub room_id: String,
    pub display_name: String,
    pub is_dm: bool,
    pub unread_count: u64,
    pub notification_count: u64,
    pub highlight_count: u64,
    pub parent_space_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoomAttentionKind {
    Mention,
    Dm,
    Message,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomAttentionSummary {
    pub room_display_name: String,
    pub kind: RoomAttentionKind,
    pub notification_count: u64,
    pub highlight_count: u64,
    pub unread_count: u64,
}

pub fn room_attention_kind(
    is_dm: bool,
    notification_count: u64,
    highlight_count: u64,
    unread_count: u64,
) -> Option<RoomAttentionKind> {
    if highlight_count > 0 {
        return Some(RoomAttentionKind::Mention);
    }

    if notification_count == 0 && unread_count == 0 {
        return None;
    }

    if is_dm {
        Some(RoomAttentionKind::Dm)
    } else {
        Some(RoomAttentionKind::Message)
    }
}

pub fn room_attention_summary(
    room_display_name: String,
    is_dm: bool,
    notification_count: u64,
    highlight_count: u64,
    unread_count: u64,
) -> Option<RoomAttentionSummary> {
    let kind = room_attention_kind(is_dm, notification_count, highlight_count, unread_count)?;

    Some(RoomAttentionSummary {
        room_display_name,
        kind,
        notification_count,
        highlight_count,
        unread_count,
    })
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelinePaneState {
    pub room_id: Option<String>,
    pub is_subscribed: bool,
    pub is_paginating_backwards: bool,
    pub composer: ComposerState,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComposerState {
    pub pending_transaction_id: Option<String>,
    pub draft: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ThreadPaneState {
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SearchState {
    Closed,
    Editing {
        query: String,
        scope: SearchScope,
    },
    Searching {
        request_id: u64,
        query: String,
        scope: SearchScope,
    },
    Results {
        request_id: u64,
        query: String,
        scope: SearchScope,
        results: Vec<SearchResult>,
    },
    Failed {
        request_id: u64,
        query: String,
        scope: SearchScope,
        message: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SearchScope {
    CurrentRoom { room_id: String },
    CurrentSpace { space_id: String },
    Dms,
    AllRooms,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchResult {
    pub room_id: String,
    pub event_id: String,
    pub sender: String,
    pub timestamp_ms: u64,
    pub score_millis: u32,
    pub snippet: String,
    pub match_field: SearchMatchField,
    pub highlights: Vec<TextRange>,
    pub match_kind: SearchMatchKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TextRange {
    /// Half-open range in UTF-16 code units relative to `SearchResult::snippet`.
    pub start_utf16: u32,
    pub end_utf16: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SearchMatchKind {
    Exact,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SearchMatchField {
    MessageBody,
    AttachmentFileName,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AppError {
    pub code: String,
    pub message: String,
    pub recoverable: bool,
}
