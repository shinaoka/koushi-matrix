//! Public state snapshot and incremental update DTOs.

use koushi_state::{
    AccountManagementCapabilities, AccountManagementState, AccountManagementUrl, ActivityRow,
    ActivityState, AppError, AuthDiscoveryState, BasicOperationState, CjkTextPolicyState,
    ConversationAccess, CurrentSessionStatusState, DeviceCleanupState, DirectoryState,
    E2eeTrustState, FilesViewState, FocusedContextState, IgnoredUserUpdateState, InvitePreview,
    InviteWorkflowState, LinkPreviewSettingsState, LiveEventReceiptSummary, LiveSignalsState,
    LiveTypingUser, LocalEncryptionState, LocalUserAliasUpdateState, MentionCandidatesState,
    NativeAttentionState, NavigationState, OwnProfile, PresenceKind, ProfileState,
    ProfileUpdateState, QrLoginState, RoomInteractionState, RoomListProjection, RoomLiveSignals,
    RoomManagementState, RoomNotificationSettings, RoomPreferencesState, RoomSummary,
    ScheduledSendsListState, SearchCrawlerLastActive, SearchCrawlerRoomState, SearchCrawlerState,
    SearchState, SecureBackupGateState, SessionState, SettingsState, SidebarModel,
    SoftLogoutReauthState, SpaceChildrenState, SpaceMembersState, SpaceSummary, SyncState,
    ThreadAttentionState, ThreadPaneState, ThreadsListState, TimelinePaneState, UserProfile,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fmt;

pub type AppStateSnapshot = koushi_state::AppState;

#[cfg(test)]
mod tests;

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
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomLiveSignalMetadata {
    pub fully_read_event_id: Option<String>,
    pub typing_user_ids: Vec<String>,
    pub typing_users: Vec<LiveTypingUser>,
}

// #1979: boundary payload; the read marker and typing senders are private.
impl fmt::Debug for RoomLiveSignalMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RoomLiveSignalMetadata")
            .field(
                "fully_read_event_id",
                &self.fully_read_event_id.as_ref().map(|_| "EventId(..)"),
            )
            .field("typing_user_count", &self.typing_user_ids.len())
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StateDelta {
    pub generation: u64,
    /// Boxed so the delta stays out of the inline `CoreEvent` representation.
    /// `StateDeltaChangedSlices` is ~7 KiB; keeping it inline made every
    /// `CoreEvent` that size and forced a ~110 MiB per-account event-queue slot
    /// allocation (#1150). Serde and `Deref` access are unchanged.
    pub changed: Box<StateDeltaChangedSlices>,
}

/// Room-local profile replacement delta: room id -> (user id -> replacement),
/// where `None` removes the entry at that level.
pub type RoomProfileReplacementsDelta =
    BTreeMap<String, Option<BTreeMap<String, Option<UserProfile>>>>;

/// Focused- or thread-scope receipt-summary replacement delta:
/// room id -> scope key (permalink target or thread root) -> event id ->
/// replacement, where `None` removes the entry at that level.
pub type ScopedReceiptSummaryChanges =
    BTreeMap<String, BTreeMap<String, BTreeMap<String, Option<LiveEventReceiptSummary>>>>;

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct StateDeltaChangedSlices {
    pub session: Option<SessionState>,
    pub session_disclosure_epoch: Option<u64>,
    pub session_lock_reason: Option<Option<koushi_state::SessionLockReason>>,
    pub secure_backup_gate: Option<SecureBackupGateState>,
    /// #1748: the derived read/send admission. Published whenever it differs, so
    /// no renderer has to project it from the session, list readiness, and
    /// backup gate slices it already receives.
    pub conversation_access: Option<ConversationAccess>,
    pub device_cleanup: Option<DeviceCleanupState>,
    pub current_session_status: Option<CurrentSessionStatusState>,
    pub auth: Option<AuthDiscoveryState>,
    pub account_management_url: Option<Option<AccountManagementUrl>>,
    pub account_management: Option<AccountManagementState>,
    pub account_management_capabilities: Option<AccountManagementCapabilities>,
    pub account_notifications: Option<koushi_state::AccountNotificationsState>,
    pub contact_security: Option<koushi_state::ContactSecurityState>,
    pub sender_device_advisories: Option<koushi_state::SenderDeviceAdvisoriesState>,
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
    pub profile_room_users_by_room: Option<RoomProfileReplacementsDelta>,
    pub profile_local_aliases_by_id: Option<BTreeMap<String, Option<String>>>,
    /// `true` adds the user to the ignored set; `false` removes it.
    pub profile_ignored_user_ids_by_id: Option<BTreeMap<String, bool>>,
    pub profile_local_alias_update: Option<LocalUserAliasUpdateState>,
    pub profile_ignored_user_update: Option<IgnoredUserUpdateState>,
    pub profile_update: Option<ProfileUpdateState>,
    pub space_children: Option<SpaceChildrenState>,
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
    /// The explicitly opened Home/Space scheduled-sends projection (#1160).
    /// Emitted whenever it changes, including an explicit `Closed` when the
    /// panel closes; an omitted field means the slice is unchanged. The backing
    /// queue is never emitted.
    pub scheduled_sends_list: Option<ScheduledSendsListState>,
    pub focused_context: Option<FocusedContextState>,
    pub search: Option<SearchState>,
    pub search_crawler: Option<SearchCrawlerState>,
    pub search_crawler_rooms_by_id: Option<BTreeMap<String, Option<SearchCrawlerRoomState>>>,
    pub search_crawler_last_active: Option<Option<SearchCrawlerLastActive>>,
    pub files_view: Option<FilesViewState>,
    pub history_export: Option<koushi_state::HistoryExportState>,
    pub basic_operation: Option<BasicOperationState>,
    pub room_address_availability: Option<koushi_state::RoomAddressAvailabilityState>,
    pub live_signals: Option<LiveSignalsState>,
    /// Room-local live-signal replacements; `None` removes a room entry.
    /// Receipt-only changes use `live_signals_receipts_by_room_event` instead.
    pub live_signals_rooms: Option<BTreeMap<String, Option<RoomLiveSignals>>>,
    /// Receipt-summary replacements nested by room and event. This avoids
    /// cloning the other events in a room for a receipt move/update.
    pub live_signals_receipts_by_room_event:
        Option<BTreeMap<String, BTreeMap<String, Option<LiveEventReceiptSummary>>>>,
    /// Focused-scope receipt-summary replacements nested by room, permalink
    /// target event, and event. The main scope is never merged into it.
    pub live_signals_focused_receipts_by_room_event: Option<ScopedReceiptSummaryChanges>,
    /// Thread-scope receipt-summary replacements nested by room, thread root
    /// event, and event. The main scope is never merged into it.
    pub live_signals_thread_receipts_by_room_event: Option<ScopedReceiptSummaryChanges>,
    /// Non-receipt room metadata replacements for existing room entries.
    pub live_signals_room_metadata_by_id: Option<BTreeMap<String, Option<RoomLiveSignalMetadata>>>,
    /// User-local presence replacements; `None` removes a user entry.
    pub live_signals_presence_by_user: Option<BTreeMap<String, Option<PresenceKind>>>,
    pub e2ee_trust: Option<E2eeTrustState>,
    pub local_encryption: Option<LocalEncryptionState>,
    pub native_attention: Option<NativeAttentionState>,
    /// #1427: the device-local delivery channel's runtime projection. It carries no
    /// secret, only whether each sink holds one and how its last send ended.
    pub notification_delivery: Option<koushi_state::NotificationDeliveryState>,
    pub cjk_text_policy: Option<CjkTextPolicyState>,
    pub errors: Option<Vec<AppError>>,
    pub sidebar: Option<SidebarModel>,
}

// #1985: the delta reaches account, room and event identities, message text and
// id-keyed maps by construction (every field is one of those, or `None`), so its
// `Debug` names the slices that changed and never their contents.
impl fmt::Debug for StateDeltaChangedSlices {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StateDeltaChangedSlices ")?;
        formatter
            .debug_list()
            .entries(self.changed_slice_names())
            .finish()
    }
}

impl StateDeltaChangedSlices {
    /// The names of this delta's changed slices, in declaration order.
    ///
    /// The `Self` destructuring is the completeness guard: a new slice fails the
    /// build here instead of silently missing from the dump.
    fn changed_slice_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        let Self {
            session,
            session_disclosure_epoch,
            session_lock_reason,
            secure_backup_gate,
            conversation_access,
            device_cleanup,
            current_session_status,
            auth,
            account_management_url,
            account_management,
            account_management_capabilities,
            account_notifications,
            contact_security,
            sender_device_advisories,
            soft_logout_reauth,
            qr_login,
            settings,
            link_preview_settings,
            room_preferences,
            profile,
            profile_own,
            profile_users_by_id,
            profile_room_users_by_room,
            profile_local_aliases_by_id,
            profile_ignored_user_ids_by_id,
            profile_local_alias_update,
            profile_ignored_user_update,
            profile_update,
            space_children,
            space_members,
            sync,
            navigation,
            spaces,
            spaces_by_id,
            rooms,
            rooms_by_id,
            invites,
            invites_by_id,
            invite_workflow,
            room_list,
            room_notification_settings,
            room_notification_settings_by_id,
            room_interactions,
            room_interactions_by_id,
            directory,
            room_management,
            mention_candidates,
            activity,
            activity_recent_rows_by_id,
            activity_unread_rows_by_id,
            timeline,
            thread,
            thread_attention,
            threads_list,
            scheduled_sends_list,
            focused_context,
            search,
            search_crawler,
            search_crawler_rooms_by_id,
            search_crawler_last_active,
            files_view,
            history_export,
            basic_operation,
            room_address_availability,
            live_signals,
            live_signals_rooms,
            live_signals_receipts_by_room_event,
            live_signals_focused_receipts_by_room_event,
            live_signals_thread_receipts_by_room_event,
            live_signals_room_metadata_by_id,
            live_signals_presence_by_user,
            e2ee_trust,
            local_encryption,
            native_attention,
            notification_delivery,
            cjk_text_policy,
            errors,
            sidebar,
        } = self;
        [
            ("session", session.is_some()),
            (
                "session_disclosure_epoch",
                session_disclosure_epoch.is_some(),
            ),
            ("session_lock_reason", session_lock_reason.is_some()),
            ("secure_backup_gate", secure_backup_gate.is_some()),
            ("conversation_access", conversation_access.is_some()),
            ("device_cleanup", device_cleanup.is_some()),
            ("current_session_status", current_session_status.is_some()),
            ("auth", auth.is_some()),
            ("account_management_url", account_management_url.is_some()),
            ("account_management", account_management.is_some()),
            (
                "account_management_capabilities",
                account_management_capabilities.is_some(),
            ),
            ("account_notifications", account_notifications.is_some()),
            ("contact_security", contact_security.is_some()),
            (
                "sender_device_advisories",
                sender_device_advisories.is_some(),
            ),
            ("soft_logout_reauth", soft_logout_reauth.is_some()),
            ("qr_login", qr_login.is_some()),
            ("settings", settings.is_some()),
            ("link_preview_settings", link_preview_settings.is_some()),
            ("room_preferences", room_preferences.is_some()),
            ("profile", profile.is_some()),
            ("profile_own", profile_own.is_some()),
            ("profile_users_by_id", profile_users_by_id.is_some()),
            (
                "profile_room_users_by_room",
                profile_room_users_by_room.is_some(),
            ),
            (
                "profile_local_aliases_by_id",
                profile_local_aliases_by_id.is_some(),
            ),
            (
                "profile_ignored_user_ids_by_id",
                profile_ignored_user_ids_by_id.is_some(),
            ),
            (
                "profile_local_alias_update",
                profile_local_alias_update.is_some(),
            ),
            (
                "profile_ignored_user_update",
                profile_ignored_user_update.is_some(),
            ),
            ("profile_update", profile_update.is_some()),
            ("space_children", space_children.is_some()),
            ("space_members", space_members.is_some()),
            ("sync", sync.is_some()),
            ("navigation", navigation.is_some()),
            ("spaces", spaces.is_some()),
            ("spaces_by_id", spaces_by_id.is_some()),
            ("rooms", rooms.is_some()),
            ("rooms_by_id", rooms_by_id.is_some()),
            ("invites", invites.is_some()),
            ("invites_by_id", invites_by_id.is_some()),
            ("invite_workflow", invite_workflow.is_some()),
            ("room_list", room_list.is_some()),
            (
                "room_notification_settings",
                room_notification_settings.is_some(),
            ),
            (
                "room_notification_settings_by_id",
                room_notification_settings_by_id.is_some(),
            ),
            ("room_interactions", room_interactions.is_some()),
            ("room_interactions_by_id", room_interactions_by_id.is_some()),
            ("directory", directory.is_some()),
            ("room_management", room_management.is_some()),
            ("mention_candidates", mention_candidates.is_some()),
            ("activity", activity.is_some()),
            (
                "activity_recent_rows_by_id",
                activity_recent_rows_by_id.is_some(),
            ),
            (
                "activity_unread_rows_by_id",
                activity_unread_rows_by_id.is_some(),
            ),
            ("timeline", timeline.is_some()),
            ("thread", thread.is_some()),
            ("thread_attention", thread_attention.is_some()),
            ("threads_list", threads_list.is_some()),
            ("scheduled_sends_list", scheduled_sends_list.is_some()),
            ("focused_context", focused_context.is_some()),
            ("search", search.is_some()),
            ("search_crawler", search_crawler.is_some()),
            (
                "search_crawler_rooms_by_id",
                search_crawler_rooms_by_id.is_some(),
            ),
            (
                "search_crawler_last_active",
                search_crawler_last_active.is_some(),
            ),
            ("files_view", files_view.is_some()),
            ("history_export", history_export.is_some()),
            ("basic_operation", basic_operation.is_some()),
            (
                "room_address_availability",
                room_address_availability.is_some(),
            ),
            ("live_signals", live_signals.is_some()),
            ("live_signals_rooms", live_signals_rooms.is_some()),
            (
                "live_signals_receipts_by_room_event",
                live_signals_receipts_by_room_event.is_some(),
            ),
            (
                "live_signals_focused_receipts_by_room_event",
                live_signals_focused_receipts_by_room_event.is_some(),
            ),
            (
                "live_signals_thread_receipts_by_room_event",
                live_signals_thread_receipts_by_room_event.is_some(),
            ),
            (
                "live_signals_room_metadata_by_id",
                live_signals_room_metadata_by_id.is_some(),
            ),
            (
                "live_signals_presence_by_user",
                live_signals_presence_by_user.is_some(),
            ),
            ("e2ee_trust", e2ee_trust.is_some()),
            ("local_encryption", local_encryption.is_some()),
            ("native_attention", native_attention.is_some()),
            ("notification_delivery", notification_delivery.is_some()),
            ("cjk_text_policy", cjk_text_policy.is_some()),
            ("errors", errors.is_some()),
            ("sidebar", sidebar.is_some()),
        ]
        .into_iter()
        .filter_map(|(name, changed)| changed.then_some(name))
    }

    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}
