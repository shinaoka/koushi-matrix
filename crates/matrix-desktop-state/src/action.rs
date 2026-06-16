use std::fmt;

use crate::state::{
    ActivityMarkReadTarget, ActivityRow, ActivityStream, ActivityTab, BasicOperationRequest,
    CrossSigningStatus, DirectoryQuery, DirectoryRoomSummary, E2eeRecoveryState,
    IdentityResetAuthType, JapaneseCatalogProfile, LiveEventReceipts, LiveRoomSignalUpdate,
    LocalEncryptionHealth, LoginFlow, NativeAttentionState, OperationFailureKind, OwnProfile,
    PinnedEvent, PresenceKind, ProfileUpdateRequest, RecoveryMethod, RoomModerationAction,
    RoomSettingChange, RoomSettingsSnapshot, RoomSummary, RoomTagInfo, RoomTagKind, RoomTags,
    SasEmoji, ScheduledSendCapability, ScheduledSendHandle, ScheduledSendItem, SearchResult,
    SearchScope, SessionInfo, SettingsPatch, SettingsValues, SpaceSummary,
    TrustOperationFailureKind, UserProfile, VerificationCancelReason, VerificationTarget,
};

#[derive(Clone, Eq, PartialEq)]
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
    OwnProfileUpdated {
        profile: OwnProfile,
    },
    UserProfilesUpdated {
        profiles: Vec<UserProfile>,
    },
    LocalUserAliasesLoaded {
        aliases: std::collections::BTreeMap<String, String>,
    },
    LocalUserAliasUpdateRequested {
        request_id: u64,
        user_id: String,
        alias: Option<String>,
    },
    LocalUserAliasUpdateSucceeded {
        request_id: u64,
    },
    LocalUserAliasUpdateFailed {
        request_id: u64,
        message: String,
    },
    ProfileUpdateRequested {
        request_id: u64,
        request: ProfileUpdateRequest,
    },
    ProfileUpdateSucceeded {
        request_id: u64,
        profile: OwnProfile,
    },
    ProfileUpdateFailed {
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
        reason: VerificationCancelReason,
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
    RoomTagsUpdated {
        room_id: String,
        tags: RoomTags,
    },
    RoomTagSet {
        room_id: String,
        tag: RoomTagKind,
        info: RoomTagInfo,
    },
    RoomTagRemoved {
        room_id: String,
        tag: RoomTagKind,
    },
    RoomPinnedEventsUpdated {
        room_id: String,
        pinned: Vec<PinnedEvent>,
    },
    PinEventRequested {
        request_id: u64,
        room_id: String,
        event_id: String,
    },
    PinEventCompleted {
        request_id: u64,
        room_id: String,
    },
    PinEventFailed {
        request_id: u64,
        room_id: String,
        kind: OperationFailureKind,
    },
    UnpinEventRequested {
        request_id: u64,
        room_id: String,
        event_id: String,
    },
    UnpinEventCompleted {
        request_id: u64,
        room_id: String,
    },
    UnpinEventFailed {
        request_id: u64,
        room_id: String,
        kind: OperationFailureKind,
    },
    DirectoryQueryRequested {
        request_id: u64,
        query: DirectoryQuery,
    },
    DirectoryQuerySucceeded {
        request_id: u64,
        query: DirectoryQuery,
        rooms: Vec<DirectoryRoomSummary>,
        next_batch: Option<String>,
    },
    DirectoryQueryFailed {
        request_id: u64,
        query: DirectoryQuery,
        kind: OperationFailureKind,
    },
    DirectoryJoinRequested {
        request_id: u64,
        alias: String,
        via_server: Option<String>,
    },
    DirectoryJoinSucceeded {
        request_id: u64,
        room_id: String,
    },
    DirectoryJoinFailed {
        request_id: u64,
        alias: String,
        via_server: Option<String>,
        kind: OperationFailureKind,
    },
    RoomSettingsSnapshotLoaded {
        room_id: String,
        settings: RoomSettingsSnapshot,
    },
    RoomSettingUpdateRequested {
        request_id: u64,
        room_id: String,
        change: RoomSettingChange,
    },
    RoomSettingUpdateSucceeded {
        request_id: u64,
        room_id: String,
        settings: RoomSettingsSnapshot,
    },
    RoomSettingUpdateFailed {
        request_id: u64,
        room_id: String,
        kind: OperationFailureKind,
    },
    RoomModerationRequested {
        request_id: u64,
        room_id: String,
        target_user_id: String,
        action: RoomModerationAction,
        reason: Option<String>,
    },
    RoomModerationSucceeded {
        request_id: u64,
        room_id: String,
        target_user_id: String,
        action: RoomModerationAction,
    },
    RoomModerationFailed {
        request_id: u64,
        room_id: String,
        target_user_id: String,
        action: RoomModerationAction,
        kind: OperationFailureKind,
    },
    RoomMemberRoleUpdateRequested {
        request_id: u64,
        room_id: String,
        target_user_id: String,
        power_level: i64,
    },
    RoomMemberRoleUpdateSucceeded {
        request_id: u64,
        room_id: String,
        target_user_id: String,
        power_level: i64,
    },
    RoomMemberRoleUpdateFailed {
        request_id: u64,
        room_id: String,
        target_user_id: String,
        kind: OperationFailureKind,
    },
    ActivityOpened {
        request_id: u64,
    },
    ActivityClosed,
    ActivitySnapshotLoaded {
        request_id: u64,
        active_tab: ActivityTab,
        recent: ActivityStream,
        unread: ActivityStream,
        excluded_room_ids: Vec<String>,
    },
    ActivityRowsObserved {
        rows: Vec<ActivityRow>,
    },
    ActivityRowsUpdated {
        recent: ActivityStream,
        unread: ActivityStream,
        excluded_room_ids: Vec<String>,
    },
    ActivityTabSelected {
        tab: ActivityTab,
    },
    ActivityMarkReadRequested {
        request_id: u64,
        target: ActivityMarkReadTarget,
    },
    ActivityMarkReadSucceeded {
        request_id: u64,
        cleared_event_ids: Vec<String>,
    },
    ActivityMarkReadFailed {
        request_id: u64,
        target: ActivityMarkReadTarget,
        kind: OperationFailureKind,
    },
    LocalEncryptionProbeRequested {
        request_id: u64,
    },
    LocalEncryptionHealthChanged {
        request_id: u64,
        health: LocalEncryptionHealth,
    },
    ResetLocalDataRequested {
        request_id: u64,
    },
    ResetLocalDataCompleted {
        request_id: u64,
    },
    ResetLocalDataFailed {
        request_id: u64,
    },
    NativeAttentionUpdated {
        attention: NativeAttentionState,
    },
    JapaneseCatalogProfileChanged {
        profile: JapaneseCatalogProfile,
    },
    InviteListUpdated {
        invites: Vec<crate::state::InvitePreview>,
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
    ScheduledSendCapabilityChanged {
        capability: ScheduledSendCapability,
    },
    ScheduledSendCreated {
        item: ScheduledSendItem,
    },
    ScheduledSendRescheduled {
        scheduled_id: String,
        send_at_ms: u64,
        handle: ScheduledSendHandle,
    },
    ScheduledSendCancelled {
        scheduled_id: String,
    },
    ScheduledSendDispatched {
        scheduled_id: String,
    },
    ComposerDraftsLoaded {
        drafts: crate::state::ComposerDraftStore,
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
    ThreadAttentionUpdated {
        room_id: String,
        root_event_id: String,
        notification_count: u64,
        highlight_count: u64,
        live_event_marker_count: u64,
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
    LiveRoomSignalsUpdated {
        room_id: String,
        update: LiveRoomSignalUpdate,
    },
    LiveRoomReceiptsUpdated {
        room_id: String,
        receipts_by_event: Vec<LiveEventReceipts>,
    },
    FullyReadMarkerUpdated {
        room_id: String,
        event_id: Option<String>,
    },
    TypingUsersUpdated {
        room_id: String,
        user_ids: Vec<String>,
    },
    PresenceUpdated {
        user_id: String,
        presence: PresenceKind,
    },
    ComposerReplyTargetSelected {
        room_id: String,
        event_id: String,
    },
    ComposerReplyCancelled,
}

impl fmt::Debug for AppAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LoginSubmitted(request) => formatter
                .debug_tuple("LoginSubmitted")
                .field(request)
                .finish(),
            Self::DirectoryQueryRequested { request_id, query } => formatter
                .debug_struct("DirectoryQueryRequested")
                .field("request_id", request_id)
                .field("query", query)
                .finish(),
            Self::DirectoryQuerySucceeded {
                request_id,
                query,
                rooms,
                next_batch,
            } => formatter
                .debug_struct("DirectoryQuerySucceeded")
                .field("request_id", request_id)
                .field("query", query)
                .field("rooms", rooms)
                .field("next_batch", &next_batch.as_ref().map(|_| "PageToken(..)"))
                .finish(),
            Self::DirectoryQueryFailed {
                request_id,
                query,
                kind,
            } => formatter
                .debug_struct("DirectoryQueryFailed")
                .field("request_id", request_id)
                .field("query", query)
                .field("kind", kind)
                .finish(),
            Self::DirectoryJoinRequested {
                request_id,
                via_server,
                ..
            } => formatter
                .debug_struct("DirectoryJoinRequested")
                .field("request_id", request_id)
                .field("alias", &"RoomAlias(..)")
                .field("via_server", &via_server.as_ref().map(|_| "ServerName(..)"))
                .finish(),
            Self::DirectoryJoinSucceeded { request_id, .. } => formatter
                .debug_struct("DirectoryJoinSucceeded")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::DirectoryJoinFailed {
                request_id,
                via_server,
                kind,
                ..
            } => formatter
                .debug_struct("DirectoryJoinFailed")
                .field("request_id", request_id)
                .field("alias", &"RoomAlias(..)")
                .field("via_server", &via_server.as_ref().map(|_| "ServerName(..)"))
                .field("kind", kind)
                .finish(),
            Self::RoomSettingsSnapshotLoaded { .. } => formatter
                .debug_struct("RoomSettingsSnapshotLoaded")
                .field("room_id", &"RoomId(..)")
                .field("settings", &"RoomSettingsSnapshot(..)")
                .finish(),
            Self::RoomSettingUpdateRequested {
                request_id, change, ..
            } => formatter
                .debug_struct("RoomSettingUpdateRequested")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("change", change)
                .finish(),
            Self::RoomSettingUpdateSucceeded { request_id, .. } => formatter
                .debug_struct("RoomSettingUpdateSucceeded")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("settings", &"RoomSettingsSnapshot(..)")
                .finish(),
            Self::RoomSettingUpdateFailed {
                request_id, kind, ..
            } => formatter
                .debug_struct("RoomSettingUpdateFailed")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("kind", kind)
                .finish(),
            Self::RoomModerationRequested {
                request_id, action, ..
            } => formatter
                .debug_struct("RoomModerationRequested")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("target_user_id", &"UserId(..)")
                .field("action", action)
                .field("reason", &"ModerationReason(..)")
                .finish(),
            Self::RoomModerationSucceeded {
                request_id, action, ..
            } => formatter
                .debug_struct("RoomModerationSucceeded")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("target_user_id", &"UserId(..)")
                .field("action", action)
                .finish(),
            Self::RoomModerationFailed {
                request_id,
                action,
                kind,
                ..
            } => formatter
                .debug_struct("RoomModerationFailed")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("target_user_id", &"UserId(..)")
                .field("action", action)
                .field("kind", kind)
                .finish(),
            Self::RoomMemberRoleUpdateRequested {
                request_id,
                power_level,
                ..
            } => formatter
                .debug_struct("RoomMemberRoleUpdateRequested")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("target_user_id", &"UserId(..)")
                .field("power_level", power_level)
                .finish(),
            Self::RoomMemberRoleUpdateSucceeded {
                request_id,
                power_level,
                ..
            } => formatter
                .debug_struct("RoomMemberRoleUpdateSucceeded")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("target_user_id", &"UserId(..)")
                .field("power_level", power_level)
                .finish(),
            Self::RoomMemberRoleUpdateFailed {
                request_id, kind, ..
            } => formatter
                .debug_struct("RoomMemberRoleUpdateFailed")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("target_user_id", &"UserId(..)")
                .field("kind", kind)
                .finish(),
            Self::ActivityOpened { request_id } => formatter
                .debug_struct("ActivityOpened")
                .field("request_id", request_id)
                .finish(),
            Self::ActivityClosed => formatter.write_str("ActivityClosed"),
            Self::ActivitySnapshotLoaded {
                request_id,
                active_tab,
                recent,
                unread,
                excluded_room_ids,
            } => formatter
                .debug_struct("ActivitySnapshotLoaded")
                .field("request_id", request_id)
                .field("active_tab", active_tab)
                .field("recent", recent)
                .field("unread", unread)
                .field(
                    "excluded_room_ids",
                    &format_args!("{} room id(s)", excluded_room_ids.len()),
                )
                .finish(),
            Self::ActivityRowsObserved { rows } => formatter
                .debug_struct("ActivityRowsObserved")
                .field("rows", &format_args!("{} row(s)", rows.len()))
                .finish(),
            Self::ActivityRowsUpdated {
                recent,
                unread,
                excluded_room_ids,
            } => formatter
                .debug_struct("ActivityRowsUpdated")
                .field("recent", recent)
                .field("unread", unread)
                .field(
                    "excluded_room_ids",
                    &format_args!("{} room id(s)", excluded_room_ids.len()),
                )
                .finish(),
            Self::ActivityTabSelected { tab } => formatter
                .debug_struct("ActivityTabSelected")
                .field("tab", tab)
                .finish(),
            Self::ActivityMarkReadRequested { request_id, target } => formatter
                .debug_struct("ActivityMarkReadRequested")
                .field("request_id", request_id)
                .field("target", target)
                .finish(),
            Self::ActivityMarkReadSucceeded {
                request_id,
                cleared_event_ids,
            } => formatter
                .debug_struct("ActivityMarkReadSucceeded")
                .field("request_id", request_id)
                .field(
                    "cleared_event_ids",
                    &format_args!("{} event id(s)", cleared_event_ids.len()),
                )
                .finish(),
            Self::ActivityMarkReadFailed {
                request_id,
                target,
                kind,
            } => formatter
                .debug_struct("ActivityMarkReadFailed")
                .field("request_id", request_id)
                .field("target", target)
                .field("kind", kind)
                .finish(),
            _ => formatter.write_str("AppAction(..)"),
        }
    }
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
