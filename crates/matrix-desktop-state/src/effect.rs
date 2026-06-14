use crate::{
    action::{LoginRequest, RecoveryRequest},
    state::{SearchScope, SessionInfo, SettingsValues, VerificationTarget},
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
    RequestVerification {
        request_id: u64,
        target: VerificationTarget,
    },
    AcceptVerification {
        request_id: u64,
    },
    ConfirmSasVerification {
        request_id: u64,
    },
    CancelVerification {
        request_id: u64,
    },
    BootstrapCrossSigning {
        request_id: u64,
    },
    EnableKeyBackup {
        request_id: u64,
    },
    RestoreKeyBackup {
        request_id: u64,
        version: Option<String>,
    },
    ResetIdentity {
        request_id: u64,
    },
    PersistSession(SessionInfo),
    PersistSettings {
        request_id: u64,
        values: SettingsValues,
    },
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
    OpenFocusedTimeline {
        room_id: String,
        event_id: String,
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
    SettingsChanged,
    RoomListChanged,
    TimelineChanged { room_id: String },
    ThreadChanged,
    SearchChanged,
    E2eeTrustChanged,
    ErrorChanged,
}
