use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

// ── Existing cohesive submodules (pre-#87) ──────────────────────────────────
pub mod media_download;
pub mod search_crawler;

// ── New per-feature submodules (#87 Phase 2) ────────────────────────────────
pub mod activity;
pub mod basic_operation;
pub mod cjk;
pub mod directory;
pub mod e2ee;
pub mod errors;
pub mod files_view;
pub mod live_signals;
pub mod local_encryption;
pub mod native_attention;
pub mod navigation;
pub mod profile;
pub mod room;
pub mod room_interactions;
pub mod room_management;
pub mod search;
pub mod session;
pub mod settings;
pub mod sync;
pub mod thread;
pub mod timeline;

// ── Re-exports: media_download ──────────────────────────────────────────────
pub use media_download::{MediaTransferProgress, TimelineMediaDownloadState};

// ── Re-exports: search_crawler ──────────────────────────────────────────────
pub use search_crawler::{
    SearchCrawlerFailureKind, SearchCrawlerRoomState, SearchCrawlerSettings, SearchCrawlerSpeed,
    SearchCrawlerState,
};

// ── Re-exports: errors ──────────────────────────────────────────────────────
pub use errors::{AppError, OperationFailureKind};

// ── Re-exports: sync ────────────────────────────────────────────────────────
pub use sync::{SyncMode, SyncModeFailureKind, SyncState};

// ── Re-exports: session ─────────────────────────────────────────────────────
pub use session::{
    AccountManagementCapabilities, AccountManagementOperation, AccountManagementState,
    AuthDiscoveryState, AuthFailureKind, CapabilityState, DelegatedAuthLinks,
    DeviceSessionListState, DeviceSessionSummary, LoginFlow, LoginFlowKind, QrLoginState,
    RecoveryMethod, SessionInfo, SessionState, SoftLogoutReauthState,
};

// ── Re-exports: settings ────────────────────────────────────────────────────
pub use settings::{
    AppearanceSettings, ComposerSendShortcut, DisplaySettings, EmojiPreference, FontPreference,
    ImageUploadCompressionMode, ImageUploadCompressionPolicy, KeyboardSettings,
    LinkPreviewSettingsState, LocaleSettings, MediaSettings, NotificationSettings,
    RoomNotificationMode, RoomNotificationModeOperation, RoomNotificationSettings,
    RoomUrlPreviews, SettingsPatch, SettingsPersistenceState, SettingsState, SettingsValues,
    TextDirectionPreference, ThemePreference, TimelineSettings, TypographySettings,
};

// ── Re-exports: profile ─────────────────────────────────────────────────────
pub use profile::{
    AvatarImage, AvatarThumbnailFailureKind, AvatarThumbnailState, IgnoredUserUpdateState,
    LocalUserAliasUpdateState, OwnProfile, ProfileState, ProfileUpdateRequest, ProfileUpdateState,
    UserProfile, is_ignored_user, normalize_local_user_alias, refresh_profile_user_display_projection,
    refresh_room_settings_member_display_projection, refresh_room_summary_display_projection,
    resolve_user_display_name,
};

// ── Re-exports: room ────────────────────────────────────────────────────────
pub use room::{
    InvitePreview, RoomAttentionKind, RoomAttentionSummary, RoomSummary, RoomTagInfo, RoomTagKind,
    RoomTags, SpaceSummary, room_attention_kind, room_attention_summary,
};

// ── Re-exports: room_interactions ──────────────────────────────────────────
pub use room_interactions::{
    PinOp, PinOperationState, PinnedEvent, ReplyQuote, ReplyQuoteState, RoomInteractionState,
};

// ── Re-exports: navigation ──────────────────────────────────────────────────
pub use navigation::{
    FocusedContextState, NavigationState, RoomListEntryKind, RoomListFilter, RoomListProjection,
    RoomListProjectionItem, RoomListSort, compute_room_list_projection,
};

// ── Re-exports: activity ────────────────────────────────────────────────────
pub use activity::{
    ActivityMarkReadState, ActivityMarkReadTarget, ActivityRow, ActivityState, ActivityStream,
    ActivityTab,
};

// ── Re-exports: directory ───────────────────────────────────────────────────
pub use directory::{
    DirectoryJoinState, DirectoryQuery, DirectoryQueryState, DirectoryRoomSummary, DirectoryState,
};

// ── Re-exports: room_management ─────────────────────────────────────────────
pub use room_management::{
    RoomHistoryVisibility, RoomJoinRule, RoomManagementOperationKind, RoomManagementOperationState,
    RoomManagementState, RoomMemberRole, RoomMemberSummary, RoomModerationAction,
    RoomPermissionFacts, RoomSettingChange, RoomSettingsSnapshot,
};

// ── Re-exports: e2ee ────────────────────────────────────────────────────────
pub use e2ee::{
    CrossSigningStatus, DeviceTrustLevel, DeviceTrustSummary, E2eeKeyManagementState,
    E2eeRecoveryState, E2eeTrustState, IdentityResetAuthType, IdentityResetState, KeyBackupStatus,
    RecoveryKeyDeliveryState, RoomKeyExportState, RoomKeyImportState, SasEmoji,
    SecureBackupPassphraseChangeState, SecureBackupSetupState, TrustOperationFailureKind,
    VerificationCancelReason, VerificationFlowState, VerificationTarget,
};

// ── Re-exports: local_encryption ────────────────────────────────────────────
pub use local_encryption::{LocalEncryptionHealth, LocalEncryptionState};

// ── Re-exports: native_attention ─────────────────────────────────────────────
pub use native_attention::{
    NativeAttentionCandidate, NativeAttentionCapabilities, NativeAttentionCapability,
    NativeAttentionDispatchState, NativeAttentionObservationKind, NativeAttentionProjectionInput,
    NativeAttentionState, NativeAttentionSummary, NativeAttentionSuppressionReason,
    native_attention_capabilities_for_platform, native_attention_state_from_rooms,
};

// ── Re-exports: cjk ─────────────────────────────────────────────────────────
pub use cjk::{CjkCollationProfile, CjkNormalizationProfile, CjkTextPolicyState, JapaneseCatalogProfile};

// ── Re-exports: timeline ────────────────────────────────────────────────────
pub use timeline::{
    ComposerDraftStore, ComposerMode, ComposerState, MAX_PERSISTED_COMPOSER_DRAFT_BYTES,
    MAX_PERSISTED_COMPOSER_DRAFT_ROOM_COUNT, MAX_PERSISTED_COMPOSER_DRAFT_THREAD_COUNT,
    MediaGalleryStore, PendingComposerSendKind, ScheduledSendCapability, ScheduledSendHandle,
    ScheduledSendItem, ScheduledSendStore, StagedUploadCompressionChoice, StagedUploadItem,
    StagedUploadKind, TimelineMediaGalleryItem, TimelineMediaGalleryMedia,
    TimelineMediaGallerySource, TimelineMediaGalleryThumbnail, TimelineMediaKind,
    TimelinePaneState, UploadStagingStore,
};

// ── Re-exports: thread ──────────────────────────────────────────────────────
pub use thread::{ThreadAttentionState, ThreadPaneState, ThreadsListItem, ThreadsListState};

// ── Re-exports: search ──────────────────────────────────────────────────────
pub use search::{
    SearchMatchField, SearchMatchKind, SearchResult, SearchScope, SearchState, TextRange,
};

// ── Re-exports: files_view ──────────────────────────────────────────────────
pub use files_view::{
    AttachmentFilter, AttachmentKind, AttachmentResult, AttachmentScope, AttachmentSort,
    FilesViewScope, FilesViewState,
};

// ── Re-exports: basic_operation ─────────────────────────────────────────────
pub use basic_operation::{BasicOperationRequest, BasicOperationState};

// ── Re-exports: live_signals ────────────────────────────────────────────────
pub use live_signals::{
    LIVE_READ_RECEIPT_READER_CAP, LiveEventReceipts, LiveEventReceiptSummary, LiveReadReceipt,
    LiveRoomSignalUpdate, LiveSignalsState, PresenceKind, RoomLiveSignals,
};

// ── Helper used by search_crawler submodule via crate::state::default_true ──
pub(crate) fn default_true() -> bool {
    true
}

// ── AppState ─────────────────────────────────────────────────────────────────
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AppState {
    pub session: SessionState,
    pub auth: AuthDiscoveryState,
    #[serde(default)]
    pub device_sessions: DeviceSessionListState,
    #[serde(default)]
    pub account_management: AccountManagementState,
    #[serde(default)]
    pub account_management_capabilities: AccountManagementCapabilities,
    #[serde(default)]
    pub soft_logout_reauth: SoftLogoutReauthState,
    #[serde(default)]
    pub qr_login: QrLoginState,
    pub settings: SettingsState,
    #[serde(default)]
    pub link_preview_settings: LinkPreviewSettingsState,
    pub profile: ProfileState,
    pub sync: SyncState,
    #[serde(default)]
    pub sync_mode: SyncMode,
    pub navigation: NavigationState,
    pub spaces: Vec<SpaceSummary>,
    pub rooms: Vec<RoomSummary>,
    pub invites: Vec<InvitePreview>,
    #[serde(default)]
    pub room_list: RoomListProjection,
    #[serde(default)]
    pub room_notification_settings: HashMap<String, RoomNotificationSettings>,
    pub room_interactions: BTreeMap<String, RoomInteractionState>,
    #[serde(skip)]
    pub composer_drafts: ComposerDraftStore,
    #[serde(skip)]
    pub scheduled_sends: ScheduledSendStore,
    #[serde(skip)]
    pub upload_staging: UploadStagingStore,
    #[serde(skip)]
    pub media_gallery: MediaGalleryStore,
    pub directory: DirectoryState,
    pub room_management: RoomManagementState,
    pub activity: ActivityState,
    pub timeline: TimelinePaneState,
    pub thread: ThreadPaneState,
    pub thread_attention: ThreadAttentionState,
    pub threads_list: ThreadsListState,
    pub focused_context: FocusedContextState,
    pub search: SearchState,
    #[serde(default)]
    pub search_crawler: SearchCrawlerState,
    pub files_view: FilesViewState,
    pub basic_operation: BasicOperationState,
    pub live_signals: LiveSignalsState,
    pub e2ee_trust: E2eeTrustState,
    pub local_encryption: LocalEncryptionState,
    pub native_attention: NativeAttentionState,
    pub cjk_text_policy: CjkTextPolicyState,
    pub errors: Vec<AppError>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            session: SessionState::SignedOut,
            auth: AuthDiscoveryState::Unknown,
            device_sessions: DeviceSessionListState::Idle,
            account_management: AccountManagementState::Idle,
            account_management_capabilities: AccountManagementCapabilities::default(),
            soft_logout_reauth: SoftLogoutReauthState::Idle,
            qr_login: QrLoginState::Idle,
            settings: SettingsState::default(),
            link_preview_settings: LinkPreviewSettingsState::default(),
            profile: ProfileState::default(),
            sync: SyncState::Stopped,
            sync_mode: SyncMode::Unsupported,
            navigation: NavigationState::default(),
            spaces: Vec::new(),
            rooms: Vec::new(),
            invites: Vec::new(),
            room_list: RoomListProjection::default(),
            room_notification_settings: HashMap::new(),
            room_interactions: BTreeMap::new(),
            composer_drafts: ComposerDraftStore::default(),
            scheduled_sends: ScheduledSendStore::default(),
            upload_staging: UploadStagingStore::default(),
            media_gallery: MediaGalleryStore::default(),
            directory: DirectoryState::default(),
            room_management: RoomManagementState::default(),
            activity: ActivityState::Closed,
            timeline: TimelinePaneState::default(),
            thread: ThreadPaneState::Closed,
            thread_attention: ThreadAttentionState::Closed,
            threads_list: ThreadsListState::Closed,
            focused_context: FocusedContextState::Closed,
            search: SearchState::Closed,
            search_crawler: SearchCrawlerState::default(),
            files_view: FilesViewState::Closed,
            basic_operation: BasicOperationState::Idle,
            live_signals: LiveSignalsState::default(),
            e2ee_trust: E2eeTrustState::default(),
            local_encryption: LocalEncryptionState::Unknown,
            native_attention: NativeAttentionState::default(),
            cjk_text_policy: CjkTextPolicyState::default(),
            errors: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn timeline_media_download_state_serializes_as_tagged_union() {
        let pending = TimelineMediaDownloadState::Pending {
            progress: Some(MediaTransferProgress {
                current: 3,
                total: 10,
            }),
        };
        assert_eq!(
            serde_json::to_value(&pending).unwrap(),
            json!({
                "kind": "pending",
                "progress": { "current": 3, "total": 10 }
            })
        );

        let ready = TimelineMediaDownloadState::Ready {
            source_url: "/data/image.png".to_owned(),
            width: Some(640),
            height: Some(480),
            mime_type: Some("image/png".to_owned()),
        };
        assert_eq!(
            serde_json::to_value(&ready).unwrap(),
            json!({
                "kind": "ready",
                "source_url": "/data/image.png",
                "width": 640,
                "height": 480,
                "mime_type": "image/png"
            })
        );

        let failed = TimelineMediaDownloadState::Failed {
            failure_kind: OperationFailureKind::Network,
        };
        assert_eq!(
            serde_json::to_value(&failed).unwrap(),
            json!({ "kind": "failed", "failure_kind": "network" })
        );
    }
}
