use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

// ── Existing cohesive submodules (pre-#87) ──────────────────────────────────
pub mod media_download;
pub mod search_crawler;

// ── New per-feature submodules (#87 Phase 2) ────────────────────────────────
mod activity;
mod basic_operation;
mod cjk;
mod composer_draft;
mod directory;
mod e2ee;
mod errors;
mod files_view;
mod invite_workflow;
mod live_signals;
mod local_encryption;
mod mention;
mod native_attention;
mod navigation;
mod profile;
mod room;
mod room_interactions;
mod room_management;
mod search;
mod session;
mod session_status;
mod settings;
mod sliding_sync;
mod space_members;
mod sync;
mod thread;
mod timeline;

// ── Re-exports: media_download ──────────────────────────────────────────────
pub use media_download::{MediaTransferProgress, TimelineMediaDownloadState};

// ── Re-exports: search_crawler ──────────────────────────────────────────────
pub use search_crawler::{
    SearchCrawlerFailureKind, SearchCrawlerLastActive, SearchCrawlerLastActiveStatus,
    SearchCrawlerRoomState, SearchCrawlerSettings, SearchCrawlerSpeed, SearchCrawlerState,
};

// ── Re-exports: errors ──────────────────────────────────────────────────────
pub use composer_draft::{
    ComposerDraftProtection, MAX_LIVE_COMPOSER_ROOM_TOMBSTONES, MAX_LIVE_COMPOSER_THREAD_TOMBSTONES,
};
pub use errors::{AppError, OperationFailureKind};

// ── Re-exports: sync ────────────────────────────────────────────────────────
pub use sync::{SyncLifecycleStatus, SyncState};

// ── Re-exports: session ─────────────────────────────────────────────────────
pub use session::{
    AccountManagementCapabilities, AccountManagementOperation, AccountManagementState,
    AccountManagementUrl, AuthDiscoveryState, AuthFailureKind, CapabilityState,
    CurrentDeviceTrustState, DelegatedAuthLinks, DeviceCleanupAuthMode, DeviceCleanupFailureKind,
    DeviceCleanupLocalMode, DeviceCleanupOfferReason, DeviceCleanupRemoteOutcome,
    DeviceCleanupState, LoginAttemptId, LoginFlow, LoginFlowKind, PendingKeyCountBucket,
    ProvisionalPhase, QrLoginState, RecoveryMethod, SecureBackupGateFailureKind,
    SecureBackupGateState, SecureBackupSetupAdmission, SecureBackupSetupIntent,
    SessionAuthenticationMethod, SessionInfo, SessionLockReason, SessionState,
    SoftLogoutReauthState, VerificationAccountKind, VerificationGateFailureKind,
    VerificationGateRejectReason, VerificationGateState, VerificationMethod,
    VerificationMethodCapability,
};
pub use session_status::{
    CurrentSessionBackupState, CurrentSessionStatusDetails, CurrentSessionStatusFailureKind,
    CurrentSessionStatusState, CurrentSessionSyncState, OwnIdentityVerification,
    SessionStatusRefreshTrigger,
};
pub use sliding_sync::{
    SlidingSyncAdmission, SlidingSyncAdmissionKind, SlidingSyncAdmissionSource,
    SlidingSyncCapabilityFailureKind, SlidingSyncCapabilityResult, SlidingSyncCapabilityState,
    SlidingSyncPositiveEvidence, SlidingSyncRevalidationState,
};

// ── Re-exports: settings ────────────────────────────────────────────────────
pub use settings::{
    AppearanceSettings, ComposerSendShortcut, ComposerSettings, DisplayDensity, DisplaySettings,
    EmojiPreference, FontPreference, ImageUploadCompressionMode, ImageUploadCompressionPolicy,
    KeyboardSettings, LinkPreviewSettingsState, LocaleSettings, MediaSettings,
    NotificationSettings, RoomNotificationMode, RoomNotificationModeOperation,
    RoomNotificationSettings, RoomPreference, RoomPreferencesState, RoomUrlPreviews, SettingsPatch,
    SettingsPersistenceState, SettingsState, SettingsValues, SidebarCategory,
    SidebarCollapsedSections, SidebarSettings, TextDirectionPreference, ThemePreference,
    ThreadListOrder, TimelineSettings, TimelineThreadRootOrder, TypographySettings, WindowSettings,
};

// ── Re-exports: profile ─────────────────────────────────────────────────────
pub use profile::{
    AvatarImage, AvatarThumbnailFailureKind, AvatarThumbnailState, IgnoredUserUpdateState,
    LocalUserAliasUpdateState, OwnProfile, ProfileResolution, ProfileResolutionInput,
    ProfileResolutionSource, ProfileState, ProfileUpdateRequest, ProfileUpdateState, UserProfile,
    is_ignored_user, normalize_local_user_alias, refresh_profile_user_display_projection,
    refresh_room_settings_member_display_projection, refresh_room_summary_display_projection,
    resolve_optional_user_display_name, resolve_people_label, resolve_user_display_name,
};

// ── Re-exports: space members ─────────────────────────────────────────────
pub use space_members::{
    SpaceMemberEntry, SpaceMemberInviteOutcome, SpaceMemberMembership, SpaceMemberRoleFailureKind,
    SpaceMemberRoleOption, SpaceMemberRoleUpdateOutcome, SpaceMembersCommandRejection,
    SpaceMembersOperationState, SpaceMembersProjection, SpaceMembersState,
    admit_space_member_cancellation, admit_space_member_invite, admit_space_member_role,
    admit_space_members_load, refresh_space_member_display_projection,
    resolve_space_members_projection, sort_entries,
};

// ── Re-exports: room ────────────────────────────────────────────────────────
pub(crate) use room::compare_conversation_activity;
pub use room::{
    ConversationActivity, ConversationActivitySource, InvitePreview, RoomAttentionKind,
    RoomAttentionProjection, RoomAttentionSummary, RoomLatestEventSummary, RoomSummary,
    RoomTagInfo, RoomTagKind, RoomTags, SpaceSummary, room_activity_unread_count,
    room_attention_kind, room_attention_projection, room_attention_summary,
};

// ── Re-exports: invite_workflow ─────────────────────────────────────────────
pub use invite_workflow::{
    INVITE_ALREADY_IN_SPACE_MESSAGE, InviteDestination, InviteDestinationKind,
    InviteDestinationResult, InviteDestinationResultKind, InviteHistoryPolicy,
    InviteHistoryReadiness, InviteOperationState, InviteScopeOption, InviteScopePlan,
    InviteScopeSelection, InviteSelectedTarget, InviteTargetCandidate, InviteTargetCandidateSource,
    InviteTargetCandidateStatus, InviteTargetQueryState, InviteWorkflowState,
    build_invite_history_policy, build_invite_scope_plan, build_invite_target_query_state,
    invite_notice_from_results, selected_target_from_query,
};

// ── Re-exports: room_interactions ──────────────────────────────────────────
pub use room_interactions::{
    PinOp, PinOperationState, PinnedEvent, PinnedEventState, ReplyQuote, ReplyQuoteCodeBlock,
    ReplyQuoteFormattedBody, ReplyQuoteState, RoomInteractionState,
};

// ── Re-exports: navigation ──────────────────────────────────────────────────
pub use navigation::{
    EventNavigationFailureKind, EventNavigationSource, EventNavigationState, FocusedContextState,
    HomeSelection, MAX_SPACE_LOCAL_PRESENTATIONS, MainTimelineAnchor, MissingTargetPolicy,
    NavigationPreferenceUpdate, NavigationState, RoomListEntryKind, RoomListFailureKind,
    RoomListFilter, RoomListProjection, RoomListProjectionItem, RoomListReadiness, RoomListSort,
    RoomListSource, SpaceConversationSurface, SpaceLocalPresentation, SpaceLocalPresentations,
    SpaceNavigationSelection, TimelineScrollAnchor, TimelineScrollAnchorEdge,
    compute_room_list_projection,
};

// ── Re-exports: activity ────────────────────────────────────────────────────
pub use activity::{
    ActivityMarkReadState, ActivityMarkReadTarget, ActivityResolutionState, ActivityRow,
    ActivityRowKind, ActivityState, ActivityStream, ActivityTab,
};

// ── Re-exports: directory ───────────────────────────────────────────────────
pub use directory::{
    DirectoryJoinState, DirectoryPreviewJoinability, DirectoryPreviewMembership,
    DirectoryPreviewState, DirectoryQuery, DirectoryQueryState, DirectoryRoomPreview,
    DirectoryRoomSummary, DirectoryState,
};

// ── Re-exports: room_management ─────────────────────────────────────────────
pub use room_management::{
    RoomHistoryVisibility, RoomJoinRule, RoomManagementOperationKind, RoomManagementOperationState,
    RoomManagementState, RoomMemberRole, RoomMemberRoleOption, RoomMemberSummary,
    RoomModerationAction, RoomPermissionFacts, RoomSettingChange, RoomSettingsSnapshot,
    UserTrustState, room_settings_share_link,
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
    NativeAttentionContext, NativeAttentionDispatchId, NativeAttentionDispatchState,
    NativeAttentionObservationKind, NativeAttentionProjection, NativeAttentionProjectionInput,
    NativeAttentionSoundOutcome, NativeAttentionState, NativeAttentionSummary,
    NativeAttentionSuppressionReason, native_attention_capabilities_for_platform,
    native_attention_projection_from_rooms, native_attention_state_from_rooms,
};

// ── Re-exports: cjk ─────────────────────────────────────────────────────────
pub use cjk::{
    CjkCollationProfile, CjkNormalizationProfile, CjkTextPolicyState, JapaneseCatalogProfile,
};

// ── Re-exports: timeline ────────────────────────────────────────────────────
pub use timeline::{
    ComposerDraftPersistenceEntry, ComposerDraftPersistenceImportError,
    ComposerDraftPersistenceProjection, ComposerDraftStore, ComposerMode, ComposerState,
    ComposerSubmissionRecord, ComposerSubmissionRegistry, MAX_PERSISTED_COMPOSER_DRAFT_BYTES,
    MAX_PERSISTED_COMPOSER_DRAFT_ROOM_COUNT, MAX_PERSISTED_COMPOSER_DRAFT_THREAD_COUNT,
    MediaGalleryStore, MediaPreparationFailureKind, PendingComposerSendKind, PreparedUploadFormat,
    PreparedUploadVariant, ScheduledSendCapability, ScheduledSendHandle, ScheduledSendItem,
    ScheduledSendStore, StagedUploadCompressionChoice, StagedUploadFormatChoice, StagedUploadItem,
    StagedUploadKind, StagedUploadOutputSelection, StagedUploadPreparation,
    StagedUploadResizeChoice, TimelineContinuityInspection, TimelineContinuityState,
    TimelineGapRepairFailureKind, TimelineMediaGalleryItem, TimelineMediaGalleryMedia,
    TimelineMediaGallerySource, TimelineMediaGalleryThumbnail, TimelineMediaKind,
    TimelinePaneState, UploadStagingStore, staged_upload_item_with_completed_output,
    staged_uploads_are_sendable,
};

// ── Re-exports: thread ──────────────────────────────────────────────────────
pub use thread::{
    ThreadAttentionState, ThreadOpenIntent, ThreadPaneState, ThreadRootProjectionState,
    ThreadRootProjectionStatus, ThreadsListItem, ThreadsListScope, ThreadsListState,
    sort_threads_list_items,
};

// ── Re-exports: search ──────────────────────────────────────────────────────
pub use search::{
    SearchMatchField, SearchMatchKind, SearchResult, SearchRoomFilter, SearchScope, SearchState,
    TextRange, search_query_too_short,
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
    LiveEventReceiptSummary, LiveEventReceipts, LiveReadReceipt, LiveRoomSignalUpdate,
    LiveSignalsState, LiveTypingUser, PresenceKind, RoomLiveSignals,
    refresh_live_receipt_display_projection, refresh_live_typing_user_display_projection,
    resolve_live_receipt_profile,
};

// ── Re-exports: mention candidates ──────────────────────────────────────────
pub use mention::{
    MAX_MENTION_CANDIDATE_TARGETS, MentionCandidate, MentionCandidateMembership,
    MentionCandidatesCompleteness, MentionCandidatesFailureKind, MentionCandidatesState,
    MentionCandidatesTarget, MentionSurface, RoomMentionPermission,
};

// ── Helper used by search_crawler submodule via crate::state::default_true ──
pub(crate) fn default_true() -> bool {
    true
}

// ── AppState ─────────────────────────────────────────────────────────────────
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AppState {
    pub session: SessionState,
    #[serde(default)]
    pub session_lock_reason: Option<SessionLockReason>,
    #[serde(default)]
    pub secure_backup_gate: SecureBackupGateState,
    #[serde(skip)]
    pub sliding_sync_account_epoch: u64,
    #[serde(skip)]
    pub sliding_sync_capability: SlidingSyncCapabilityState,
    #[serde(default)]
    pub device_cleanup: DeviceCleanupState,
    #[serde(default)]
    pub current_session_status: CurrentSessionStatusState,
    pub auth: AuthDiscoveryState,
    #[serde(default)]
    pub account_management_url: Option<AccountManagementUrl>,
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
    #[serde(default)]
    pub room_preferences: RoomPreferencesState,
    pub profile: ProfileState,
    #[serde(default)]
    pub space_members: SpaceMembersState,
    pub sync: SyncState,
    #[serde(default)]
    pub sync_generation: u64,
    pub navigation: NavigationState,
    pub spaces: Vec<SpaceSummary>,
    pub rooms: Vec<RoomSummary>,
    pub invites: Vec<InvitePreview>,
    #[serde(default)]
    pub invite_workflow: InviteWorkflowState,
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
    #[serde(default)]
    pub mention_candidates: MentionCandidatesState,
    pub activity: ActivityState,
    pub timeline: TimelinePaneState,
    pub thread: ThreadPaneState,
    pub thread_attention: ThreadAttentionState,
    pub threads_list: ThreadsListState,
    #[serde(default)]
    pub thread_root_projections: ThreadRootProjectionState,
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
    #[serde(skip)]
    pub native_attention_context: NativeAttentionContext,
    pub cjk_text_policy: CjkTextPolicyState,
    pub errors: Vec<AppError>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            session: SessionState::SignedOut,
            session_lock_reason: None,
            secure_backup_gate: SecureBackupGateState::Inactive,
            sliding_sync_account_epoch: 0,
            sliding_sync_capability: SlidingSyncCapabilityState::Unknown,
            device_cleanup: DeviceCleanupState::Idle,
            current_session_status: CurrentSessionStatusState::Idle,
            auth: AuthDiscoveryState::Unknown,
            account_management_url: None,
            account_management: AccountManagementState::Idle,
            account_management_capabilities: AccountManagementCapabilities::default(),
            soft_logout_reauth: SoftLogoutReauthState::Idle,
            qr_login: QrLoginState::Idle,
            settings: SettingsState::default(),
            link_preview_settings: LinkPreviewSettingsState::default(),
            room_preferences: RoomPreferencesState::default(),
            profile: ProfileState::default(),
            space_members: SpaceMembersState::default(),
            sync: SyncState::Stopped,
            sync_generation: 0,
            navigation: NavigationState::default(),
            spaces: Vec::new(),
            rooms: Vec::new(),
            invites: Vec::new(),
            invite_workflow: InviteWorkflowState::default(),
            room_list: RoomListProjection::default(),
            room_notification_settings: HashMap::new(),
            room_interactions: BTreeMap::new(),
            composer_drafts: ComposerDraftStore::default(),
            scheduled_sends: ScheduledSendStore::default(),
            upload_staging: UploadStagingStore::default(),
            media_gallery: MediaGalleryStore::default(),
            directory: DirectoryState::default(),
            room_management: RoomManagementState::default(),
            mention_candidates: MentionCandidatesState::default(),
            activity: ActivityState::default(),
            timeline: TimelinePaneState::default(),
            thread: ThreadPaneState::Closed,
            thread_attention: ThreadAttentionState::Closed,
            threads_list: ThreadsListState::Closed,
            thread_root_projections: ThreadRootProjectionState::default(),
            focused_context: FocusedContextState::Closed,
            search: SearchState::Closed,
            search_crawler: SearchCrawlerState::default(),
            files_view: FilesViewState::Closed,
            basic_operation: BasicOperationState::Idle,
            live_signals: LiveSignalsState::default(),
            e2ee_trust: E2eeTrustState::default(),
            local_encryption: LocalEncryptionState::Unknown,
            native_attention: NativeAttentionState::default(),
            native_attention_context: NativeAttentionContext::default(),
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
    fn native_attention_context_is_process_local_and_defaults_focused() {
        let state = AppState::default();

        assert!(state.native_attention_context.window_focused);
        assert!(
            serde_json::to_value(&state)
                .unwrap()
                .get("native_attention_context")
                .is_none()
        );

        let restored: AppState = serde_json::from_value(serde_json::to_value(state).unwrap())
            .expect("serialized AppState should restore");
        assert!(restored.native_attention_context.window_focused);
    }

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
