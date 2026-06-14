use std::fmt;

use crate::state::{
    BasicOperationRequest, CrossSigningStatus, E2eeRecoveryState, IdentityResetAuthType, LoginFlow,
    RecoveryMethod, RoomSummary, SasEmoji, SearchResult, SearchScope, SessionInfo, SettingsPatch,
    SettingsValues, SpaceSummary, TrustOperationFailureKind, VerificationTarget,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppAction {
    AppStarted,
    RestoreSessionSucceeded(SessionInfo),
    RestoreSessionNotFound,
    RestoreSessionFailed {
        message: String,
    },
    SwitchAccountRequested {
        info: SessionInfo,
    },
    LoginDiscoveryRequested {
        homeserver: String,
    },
    LoginDiscoverySucceeded {
        homeserver: String,
        flows: Vec<LoginFlow>,
    },
    LoginDiscoveryFailed {
        homeserver: String,
        message: String,
    },
    SettingsLoaded {
        values: SettingsValues,
    },
    SettingsLoadFailed {
        message: String,
    },
    SettingsUpdateRequested {
        request_id: u64,
        patch: SettingsPatch,
    },
    SettingsPersisted {
        request_id: u64,
    },
    SettingsPersistFailed {
        request_id: u64,
        message: String,
    },
    LoginSubmitted(LoginRequest),
    LoginSucceeded(SessionInfo),
    E2eeRecoveryRequired {
        info: SessionInfo,
        methods: Vec<RecoveryMethod>,
    },
    E2eeRecoverySubmitted(RecoveryRequest),
    E2eeRecoverySucceeded,
    E2eeRecoveryFailed {
        message: String,
    },
    E2eeRecoveryStateChanged {
        state: E2eeRecoveryState,
        methods: Vec<RecoveryMethod>,
    },
    VerificationRequested {
        request_id: u64,
        target: VerificationTarget,
    },
    VerificationAccepted {
        request_id: u64,
    },
    VerificationSasPresented {
        request_id: u64,
        emojis: Vec<SasEmoji>,
    },
    VerificationConfirmed {
        request_id: u64,
    },
    VerificationCancelled {
        request_id: u64,
    },
    VerificationCompleted {
        request_id: u64,
    },
    VerificationFailed {
        request_id: u64,
        kind: TrustOperationFailureKind,
    },
    CrossSigningStatusChanged {
        status: CrossSigningStatus,
    },
    BootstrapCrossSigningRequested {
        request_id: u64,
    },
    BootstrapCrossSigningFailed {
        request_id: u64,
        kind: TrustOperationFailureKind,
    },
    EnableKeyBackupRequested {
        request_id: u64,
    },
    KeyBackupEnabled {
        request_id: u64,
        version: String,
    },
    KeyBackupFailed {
        request_id: u64,
        kind: TrustOperationFailureKind,
    },
    RestoreKeyBackupRequested {
        request_id: u64,
        version: Option<String>,
    },
    KeyBackupRestoreProgress {
        request_id: u64,
        restored_rooms: u64,
        total_rooms: Option<u64>,
    },
    KeyBackupRestored {
        request_id: u64,
        version: Option<String>,
    },
    ResetIdentityRequested {
        request_id: u64,
    },
    ResetIdentityAuthRequired {
        request_id: u64,
        auth_type: IdentityResetAuthType,
    },
    ResetIdentityAuthSubmitted {
        request_id: u64,
    },
    ResetIdentityCompleted {
        request_id: u64,
    },
    ResetIdentityFailed {
        request_id: u64,
        kind: TrustOperationFailureKind,
    },
    LoginFailed {
        message: String,
    },
    SessionPersistenceFailed {
        message: String,
    },
    SessionLocked,
    LogoutRequested,
    LogoutFinished,
    SyncStarted,
    SyncFailed {
        reason: String,
    },
    SyncReconnecting {
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
    TimelineBackPaginationRequested {
        room_id: String,
    },
    TimelineBackPaginationFinished {
        room_id: String,
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
    SendTextFailed {
        room_id: String,
        transaction_id: String,
        message: String,
    },
    ThreadComposerDraftChanged {
        room_id: String,
        root_event_id: String,
        draft: String,
    },
    ThreadReplySubmitted {
        room_id: String,
        root_event_id: String,
        transaction_id: String,
        body: String,
    },
    ThreadReplyFinished {
        room_id: String,
        root_event_id: String,
        transaction_id: String,
    },
    ThreadReplyFailed {
        room_id: String,
        root_event_id: String,
        transaction_id: String,
        message: String,
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
    OpenFocusedContext {
        room_id: String,
        event_id: String,
    },
    FocusedContextSubscribed {
        room_id: String,
        event_id: String,
    },
    CloseFocusedContext,
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
    BasicOperationRequested {
        request_id: u64,
        request: BasicOperationRequest,
    },
    BasicOperationSucceeded {
        request_id: u64,
    },
    BasicOperationFailed {
        request_id: u64,
        message: String,
    },
    ComposerReplyTargetSelected {
        room_id: String,
        event_id: String,
    },
    ComposerReplyCancelled,
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

#[derive(Clone, Eq, PartialEq)]
pub struct RecoveryRequest {
    pub secret: AuthSecret,
}

impl fmt::Debug for RecoveryRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecoveryRequest")
            .field("secret", &self.secret)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub enum IdentityResetAuthRequest {
    OAuthApproved,
    UiaaPassword { password: AuthSecret },
}

impl fmt::Debug for IdentityResetAuthRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OAuthApproved => formatter.write_str("OAuthApproved"),
            Self::UiaaPassword { password } => formatter
                .debug_struct("UiaaPassword")
                .field("password", password)
                .finish(),
        }
    }
}
