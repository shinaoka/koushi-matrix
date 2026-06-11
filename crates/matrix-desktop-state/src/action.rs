use std::fmt;

use crate::state::{RoomSummary, SearchResult, SearchScope, SessionInfo, SpaceSummary};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppAction {
    AppStarted,
    RestoreSessionSucceeded(SessionInfo),
    RestoreSessionNotFound,
    RestoreSessionFailed {
        message: String,
    },
    LoginSubmitted(LoginRequest),
    LoginSucceeded(SessionInfo),
    LoginFailed {
        message: String,
    },
    SessionLocked,
    LogoutRequested,
    LogoutFinished,
    SyncStarted,
    SyncFailed {
        reason: String,
    },
    SyncRecovered,
    SyncStopped,
    RoomListUpdated {
        spaces: Vec<SpaceSummary>,
        rooms: Vec<RoomSummary>,
    },
    SelectSpace {
        space_id: Option<String>,
    },
    SelectRoom {
        room_id: String,
    },
    TimelineSubscribed {
        room_id: String,
    },
    TimelineSubscriptionFailed {
        room_id: String,
        message: String,
    },
    ComposerDraftChanged {
        room_id: String,
        draft: String,
    },
    SendTextSubmitted {
        room_id: String,
        transaction_id: String,
        body: String,
    },
    SendTextFinished {
        room_id: String,
        transaction_id: String,
    },
    OpenThread {
        room_id: String,
        root_event_id: String,
    },
    ThreadSubscribed {
        room_id: String,
        root_event_id: String,
    },
    CloseThread,
    SearchEdited {
        query: String,
        scope: SearchScope,
    },
    SearchSubmitted {
        request_id: u64,
        query: String,
        scope: SearchScope,
    },
    SearchSucceeded {
        request_id: u64,
        results: Vec<SearchResult>,
    },
    SearchFailed {
        request_id: u64,
        message: String,
    },
    ClearError {
        code: String,
    },
}

#[derive(Clone, Eq, PartialEq)]
pub struct LoginRequest {
    pub homeserver: String,
    pub username: String,
    pub password: AuthSecret,
    pub device_display_name: Option<String>,
}

impl fmt::Debug for LoginRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LoginRequest")
            .field("homeserver", &self.homeserver)
            .field("username", &"LoginIdentifier(..)")
            .field("password", &self.password)
            .field(
                "device_display_name",
                &self.device_display_name.as_ref().map(|_| "DeviceName(..)"),
            )
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct AuthSecret(String);

impl AuthSecret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose_secret(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for AuthSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthSecret(..)")
    }
}

impl Drop for AuthSecret {
    fn drop(&mut self) {
        use zeroize::Zeroize;

        self.0.zeroize();
    }
}
