use serde::{Deserialize, Serialize};

use crate::state::{SearchScope, SessionInfo};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AppEffect {
    RestoreSession,
    Login {
        homeserver: String,
        username: String,
    },
    PersistSession(SessionInfo),
    ClearSession,
    StartSync,
    StopSync,
    SubscribeTimeline {
        room_id: String,
    },
    SendText {
        room_id: String,
        transaction_id: String,
        body: String,
    },
    OpenThreadTimeline {
        room_id: String,
        root_event_id: String,
    },
    SearchMessages {
        request_id: u64,
        query: String,
        scope: SearchScope,
    },
    EmitUiEvent(UiEvent),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum UiEvent {
    SessionChanged,
    RoomListChanged,
    TimelineChanged { room_id: String },
    ThreadChanged,
    SearchChanged,
    ErrorChanged,
}
