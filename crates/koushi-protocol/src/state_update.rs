//! Public state snapshot and incremental update DTOs.

use koushi_state::{
    AccountManagementCapabilities, AccountManagementState, AccountManagementUrl, ActivityRow,
    ActivityState, AppError, AuthDiscoveryState, BasicOperationState, CjkTextPolicyState,
    CurrentSessionStatusState, DeviceCleanupState, DirectoryState, E2eeTrustState, FilesViewState,
    FocusedContextState, IgnoredUserUpdateState, InvitePreview, InviteWorkflowState,
    LinkPreviewSettingsState, LiveEventReceiptSummary, LiveSignalsState, LiveTypingUser,
    LocalEncryptionState, LocalUserAliasUpdateState, MentionCandidatesState, NativeAttentionState,
    NavigationState, OwnProfile, PresenceKind, ProfileState, ProfileUpdateState, QrLoginState,
    RoomInteractionState, RoomListProjection, RoomLiveSignals, RoomManagementState,
    RoomNotificationSettings, RoomPreferencesState, RoomSummary, SearchCrawlerLastActive,
    SearchCrawlerRoomState, SearchCrawlerState, SearchState, SecureBackupGateState, SessionState,
    SettingsState, SidebarModel, SoftLogoutReauthState, SpaceMembersState, SpaceSummary, SyncState,
    ThreadAttentionState, ThreadPaneState, ThreadsListState, TimelinePaneState, UserProfile,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

pub type AppStateSnapshot = koushi_state::AppState;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VersionedAppStateSnapshot {
    pub generation: u64,
    pub state: AppStateSnapshot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoreCommandAdmission {
    pub admitted_generation: u64,
}

/// Non-receipt room signals that can change without replacing receipt rows.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomLiveSignalMetadata {
    pub fully_read_event_id: Option<String>,
    pub typing_user_ids: Vec<String>,
    pub typing_users: Vec<LiveTypingUser>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StateDelta {
    pub generation: u64,
    pub changed: StateDeltaChangedSlices,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct StateDeltaChangedSlices {
    pub session: Option<SessionState>,
    pub session_lock_reason: Option<Option<koushi_state::SessionLockReason>>,
    pub secure_backup_gate: Option<SecureBackupGateState>,
    pub device_cleanup: Option<DeviceCleanupState>,
    pub current_session_status: Option<CurrentSessionStatusState>,
    pub auth: Option<AuthDiscoveryState>,
    pub account_management_url: Option<Option<AccountManagementUrl>>,
    pub account_management: Option<AccountManagementState>,
    pub account_management_capabilities: Option<AccountManagementCapabilities>,
    pub soft_logout_reauth: Option<SoftLogoutReauthState>,
    pub qr_login: Option<QrLoginState>,
    pub settings: Option<SettingsState>,
    pub link_preview_settings: Option<LinkPreviewSettingsState>,
    pub room_preferences: Option<RoomPreferencesState>,
    pub profile: Option<ProfileState>,
    /// Own-profile replacement; global and room-local observations remain scoped separately.
    pub profile_own: Option<OwnProfile>,
    /// Global profile-user replacements; room-local observations remain scoped separately.
    pub profile_users_by_id: Option<BTreeMap<String, Option<UserProfile>>>,
    /// Room-local profile replacements, nested by room and user.
    pub profile_room_users_by_room:
        Option<BTreeMap<String, Option<BTreeMap<String, Option<UserProfile>>>>>,
    pub profile_local_aliases_by_id: Option<BTreeMap<String, Option<String>>>,
    /// `true` adds the user to the ignored set; `false` removes it.
    pub profile_ignored_user_ids_by_id: Option<BTreeMap<String, bool>>,
    pub profile_local_alias_update: Option<LocalUserAliasUpdateState>,
    pub profile_ignored_user_update: Option<IgnoredUserUpdateState>,
    pub profile_update: Option<ProfileUpdateState>,
    pub space_members: Option<SpaceMembersState>,
    pub sync: Option<SyncState>,
    pub navigation: Option<NavigationState>,
    pub spaces: Option<Vec<SpaceSummary>>,
    /// Space-local replacements when the space ordering is unchanged.
    pub spaces_by_id: Option<BTreeMap<String, Option<SpaceSummary>>>,
    pub rooms: Option<Vec<RoomSummary>>,
    /// Room-local replacements when the room ordering is unchanged.
    pub rooms_by_id: Option<BTreeMap<String, Option<RoomSummary>>>,
    pub invites: Option<Vec<InvitePreview>>,
    /// Invite-local replacements when invite ordering is unchanged.
    pub invites_by_id: Option<BTreeMap<String, Option<InvitePreview>>>,
    pub invite_workflow: Option<InviteWorkflowState>,
    pub room_list: Option<RoomListProjection>,
    pub room_notification_settings: Option<HashMap<String, RoomNotificationSettings>>,
    pub room_notification_settings_by_id:
        Option<BTreeMap<String, Option<RoomNotificationSettings>>>,
    pub room_interactions: Option<BTreeMap<String, RoomInteractionState>>,
    pub room_interactions_by_id: Option<BTreeMap<String, Option<RoomInteractionState>>>,
    pub directory: Option<DirectoryState>,
    pub room_management: Option<RoomManagementState>,
    pub mention_candidates: Option<MentionCandidatesState>,
    pub activity: Option<ActivityState>,
    /// Activity-row replacements when both stream orders and stream metadata are unchanged.
    pub activity_recent_rows_by_id: Option<BTreeMap<String, Option<ActivityRow>>>,
    pub activity_unread_rows_by_id: Option<BTreeMap<String, Option<ActivityRow>>>,
    pub timeline: Option<TimelinePaneState>,
    pub thread: Option<ThreadPaneState>,
    pub thread_attention: Option<ThreadAttentionState>,
    pub threads_list: Option<ThreadsListState>,
    pub focused_context: Option<FocusedContextState>,
    pub search: Option<SearchState>,
    pub search_crawler: Option<SearchCrawlerState>,
    pub search_crawler_rooms_by_id: Option<BTreeMap<String, Option<SearchCrawlerRoomState>>>,
    pub search_crawler_last_active: Option<Option<SearchCrawlerLastActive>>,
    pub files_view: Option<FilesViewState>,
    pub basic_operation: Option<BasicOperationState>,
    pub live_signals: Option<LiveSignalsState>,
    /// Room-local live-signal replacements; `None` removes a room entry.
    /// Receipt-only changes use `live_signals_receipts_by_room_event` instead.
    pub live_signals_rooms: Option<BTreeMap<String, Option<RoomLiveSignals>>>,
    /// Receipt-summary replacements nested by room and event. This avoids
    /// cloning the other events in a room for a receipt move/update.
    pub live_signals_receipts_by_room_event:
        Option<BTreeMap<String, BTreeMap<String, Option<LiveEventReceiptSummary>>>>,
    /// Non-receipt room metadata replacements for existing room entries.
    pub live_signals_room_metadata_by_id: Option<BTreeMap<String, Option<RoomLiveSignalMetadata>>>,
    /// User-local presence replacements; `None` removes a user entry.
    pub live_signals_presence_by_user: Option<BTreeMap<String, Option<PresenceKind>>>,
    pub e2ee_trust: Option<E2eeTrustState>,
    pub local_encryption: Option<LocalEncryptionState>,
    pub native_attention: Option<NativeAttentionState>,
    pub cjk_text_policy: Option<CjkTextPolicyState>,
    pub errors: Option<Vec<AppError>>,
    pub sidebar: Option<SidebarModel>,
}

impl StateDeltaChangedSlices {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}
