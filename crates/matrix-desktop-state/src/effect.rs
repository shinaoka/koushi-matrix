use crate::{
    action::{LoginRequest, RecoveryRequest},
    state::{SearchScope, SessionInfo},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppEffect {
    RestoreSession,
    RestoreSessionFor(SessionInfo),
    DiscoverLogin {
        homeserver: String,
    },
    Login(LoginRequest),
    RecoverE2ee(RecoveryRequest),
    PersistSession(SessionInfo),
    ClearSession,
    StartSync,
    StopSync,
    SubscribeTimeline {
        room_id: String,
    },
    PaginateTimelineBackwards {
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UiEvent {
    SessionChanged,
    AuthChanged,
    RoomListChanged,
    TimelineChanged { room_id: String },
    ThreadChanged,
    SearchChanged,
    ErrorChanged,
}
