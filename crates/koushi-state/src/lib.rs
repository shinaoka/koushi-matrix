// #87 Phase 3: the crate's public surface is the curated `pub use` re-exports below
// (and the `prelude`), not the module tree. Feature modules are crate-internal so
// consumers depend on the stable root API rather than reaching into module internals.
mod action;
mod avatar_demand;
mod composer_document;
mod composer_revision;
mod composer_shortcuts;
mod effect;
mod locale_profile;
mod reducer;
mod room_access;
mod room_address;
mod sidebar;
mod space_add_rooms;
mod space_leave;
mod state;
mod submission;
mod typography_profile;

pub use action::{AppAction, AuthSecret, IdentityResetAuthRequest, LoginRequest, RecoveryRequest};
pub use avatar_demand::{
    AVATAR_PREFETCH_CAPACITY, AVATAR_VISIBLE_CAPACITY, AvatarDemandContext, AvatarDemandError,
    AvatarDemandState, VIEW_SCOPE_CAPACITY,
};
pub use composer_document::{ComposerDocument, ComposerInline};
pub use composer_revision::{ComposerDraftRevision, ComposerDraftRevisionError};
pub use composer_shortcuts::{
    ComposerFormattingOptions, ComposerKey, ComposerKeyEvent, ComposerKeyFacts,
    ComposerKeyModifiers, ComposerResolvedAction, ComposerResolverContext, ComposerSelection,
    ComposerSendIntent, ComposerSurface, FormattedMessageDraft, MentionIntent, MentionTarget,
    SlashCommandIntent, build_formatted_message_draft, build_formatted_message_draft_with_options,
    parse_slash_command, resolve_composer_key_action, resolve_composer_send_intent,
    resolve_composer_send_intent_with_options,
};
pub use effect::{AppEffect, ProfileDisplayChange, UiEvent};
pub use locale_profile::{
    CatalogLocale, DisplayPlatform, LocaleDirection, LocaleDisplayProfile, ModifierLabelProfile,
    PseudoLocaleMode, cjk_display_sort_key, normalize_cjk_search_text, resolve_catalog_locale,
    resolve_locale_display_profile,
};
pub use reducer::reduce;
pub use room_access::{
    RoomAccessOutcome, RoomAccessOutcomeLine, RoomAccessResolveInput, RoomAccessViewerFacts,
    RoomDirectoryVisibility, resolve_room_access_outcome,
};
pub use room_address::{
    CreateRoomDefaults, CreateRoomVisibility, RoomAddressAvailability,
    RoomAddressAvailabilityState, RoomAddressError, RoomAddressPreview, RoomAddressSuggestion,
    create_room_defaults_for_state, default_room_alias_localpart,
    suggest_alternative_room_alias_localpart,
};
pub use sidebar::{
    AccountAttentionSummary, AccountHomeItem, RoomListItem, SIDEBAR_SECTION_DEFAULT_VISIBLE_COUNT,
    SidebarModel, SidebarSectionBound, SidebarSectionBounds, SidebarSectionVisibility,
    SidebarSections, SpaceRailItem, account_attention_summary_for_state, compose_sidebar,
    compose_sidebar_for_state, compose_sidebar_with_account_facts,
};
pub use state::{
    AccountDisplaySettings, AccountManagementCapabilities, AccountManagementOperation,
    AccountManagementState, AccountManagementUrl, AccountNotificationSettings,
    AccountNotificationsFailureKind, AccountNotificationsLoadState, AccountNotificationsOperation,
    AccountNotificationsOperationState, AccountNotificationsSnapshot, AccountNotificationsState,
    AccountSearchCrawlerSettings, AccountSettingsValues, ActivityMarkReadState,
    ActivityMarkReadTarget, ActivityResolutionState, ActivityRow, ActivityRowKind, ActivityState,
    ActivityStream, ActivityTab, AppDisplaySettings, AppError, AppNotificationSettings,
    AppSettingsValues, AppState, AppearanceSettings, ArmedSessionStatusCheck, AttachmentFilter,
    AttachmentKind, AttachmentResult, AttachmentScope, AttachmentSort, AuthDiscoveryState,
    AuthFailureDetail, AuthFailureKind, AuthFailureStage, AuthFailureTransport,
    AuthMatrixErrorKind, AuthMethod, AvatarImage, AvatarThumbnailFailureKind, AvatarThumbnailState,
    BasicOperationRequest, BasicOperationState, BoundedNotificationDeliveryText, CapabilityState,
    CjkCollationProfile, CjkNormalizationProfile, CjkTextPolicyState,
    ComposerDraftPersistenceEntry, ComposerDraftPersistenceImportError,
    ComposerDraftPersistenceProjection, ComposerDraftProtection, ComposerDraftStore, ComposerMode,
    ComposerSendShortcut, ComposerSettings, ComposerState, ComposerSubmissionRecord,
    ComposerSubmissionRegistry, ContactDeviceCounts, ContactDeviceSignature, ContactDevicesStatus,
    ContactIdentityVerification, ContactSecurityFailureKind, ContactSecurityLoadState,
    ContactSecurityState, ContactSecuritySummary, ContactVerificationDirectChat,
    ContactVerificationOffer, ConversationActivity, ConversationActivitySource,
    CreateRoomAccessPreview, CreateRoomAccessPreviewInput, CreateRoomAccessRejection,
    CreateRoomAccessSeed, CrossSigningStatus, CurrentDeviceTrustState, CurrentSessionBackupState,
    CurrentSessionStatusDetails, CurrentSessionStatusFailureKind, CurrentSessionStatusState,
    CurrentSessionSyncState, DEFAULT_NOTIFICATION_DELIVERY_PREFIX_GRAPHEMES,
    DEFAULT_NOTIFICATION_SENDER_NAME, DEFAULT_RESEND_FROM_ADDRESS, DEFAULT_SERVERCHAN_ENDPOINT,
    DelegatedAuthLinks, DelegatedAuthMethod, DeviceCleanupAuthMode, DeviceCleanupFailureKind,
    DeviceCleanupLocalMode, DeviceCleanupOfferReason, DeviceCleanupRemoteOutcome,
    DeviceCleanupState, DeviceTrustLevel, DeviceTrustSummary, DirectPartner, DirectoryJoinState,
    DirectoryPreviewJoinability, DirectoryPreviewMembership, DirectoryPreviewState, DirectoryQuery,
    DirectoryQueryState, DirectoryRoomPreview, DirectoryRoomSummary, DirectoryState,
    DisplayDensity, DisplaySettings, E2eeKeyManagementState, E2eeRecoveryState, E2eeTrustState,
    EmojiPreference, EventNavigationFailureKind, EventNavigationSource, EventNavigationState,
    FilesViewScope, FilesViewState, FocusedContextState, FontPreference, HistoryKeyRecoveryFailure,
    HistoryKeyRecoveryState, HistoryKeyRecoveryStatus, HomeSelection,
    INVITE_ALREADY_IN_SPACE_MESSAGE, IdentityResetAuthType, IdentityResetState,
    IgnoredUserUpdateState, ImageUploadCompressionMode, ImageUploadCompressionPolicy,
    InviteDestination, InviteDestinationKind, InviteDestinationResult, InviteDestinationResultKind,
    InviteHistoryPolicy, InviteHistoryReadiness, InviteOperationState, InvitePreview,
    InviteScopeOption, InviteScopePlan, InviteScopeSelection, InviteSelectedTarget,
    InviteTargetCandidate, InviteTargetCandidateSource, InviteTargetCandidateStatus,
    InviteTargetQueryState, InviteWorkflowState, JapaneseCatalogProfile, KeyBackupStatus,
    KeyboardSettings, LinkPreviewSettingsState, LiveEventReceiptSummary,
    LiveEventReceiptSummaryUpdate, LiveEventReceipts, LiveReadReceipt, LiveRoomSignalUpdate,
    LiveSignalsState, LiveTypingUser, LocalEncryptionHealth, LocalEncryptionState,
    LocalUserAliasUpdateState, LocaleSettings, LoginAttemptId, LoginFlow, LoginFlowKind,
    MAX_MENTION_CANDIDATE_TARGETS, MAX_NOTIFICATION_DELIVERY_DWELL_SECS,
    MAX_NOTIFICATION_DELIVERY_MESSAGE_SOURCE_CHARS, MAX_NOTIFICATION_DELIVERY_MIN_INTERVAL_SECS,
    MAX_NOTIFICATION_DELIVERY_PREFIX_GRAPHEMES, MAX_NOTIFICATION_DELIVERY_SINKS,
    MAX_NOTIFICATION_EMAIL_LEN, MAX_NOTIFICATION_KEYWORD_CHARS, MAX_NOTIFICATION_KEYWORDS,
    MAX_SPACE_LOCAL_PRESENTATIONS, MainTimelineAnchor, MediaGalleryStore,
    MediaPreparationFailureKind, MediaSettings, MediaTransferProgress, MentionCandidate,
    MentionCandidateMembership, MentionCandidatesCompleteness, MentionCandidatesFailureKind,
    MentionCandidatesState, MentionCandidatesTarget, MentionSurface, MissingTargetPolicy,
    NOTIFICATION_DELIVERY_MAIL_TEXT_BUDGET, NOTIFICATION_DELIVERY_SERVERCHAN_TEXT_BUDGET,
    NativeAttentionCandidate, NativeAttentionCapabilities, NativeAttentionCapability,
    NativeAttentionContext, NativeAttentionDispatchId, NativeAttentionDispatchState,
    NativeAttentionObservationKind, NativeAttentionProjection, NativeAttentionProjectionInput,
    NativeAttentionSoundOutcome, NativeAttentionState, NativeAttentionSummary,
    NativeAttentionSuppressionReason, NativeNotificationPayload, NativeNotificationTarget,
    NavigationPreferenceUpdate, NavigationState, NotificationCategory, NotificationCategoryState,
    NotificationCategoryStates, NotificationDeliveryContent, NotificationDeliveryContentConsent,
    NotificationDeliveryOutcomeState, NotificationDeliveryPartnerLabel,
    NotificationDeliveryPresetState, NotificationDeliverySettings, NotificationDeliverySinkState,
    NotificationDeliveryState, NotificationDeliveryTarget, NotificationDeliveryTargets,
    NotificationDeliveryTiming, NotificationEmailAddress, NotificationEmailManagement,
    NotificationKeywordRuleState, NotificationSettings, NotificationSinkConfig,
    NotificationSinkSettings, OWN_SESSION_CHECK_INTERVAL_MS, OWN_SESSION_CHECK_RETRY_MS,
    OperationFailureKind, OwnIdentityVerification, OwnProfile, PendingComposerSendKind,
    PendingKeyCountBucket, PendingNotificationEmail, PinOp, PinOperationState, PinnedEvent,
    PinnedEventState, PreparedUploadFormat, PreparedUploadVariant, PresenceKind, ProfileResolution,
    ProfileResolutionInput, ProfileResolutionSource, ProfileState, ProfileUpdateRequest,
    ProfileUpdateState, ProvisionalPhase, QrLoginState, RECENTLY_JOINED_ROOM_LIMIT, ReceiptScope,
    RecentlyJoinedRoomIds, RecoveryKeyDeliveryState, RecoveryKeyMaterial, RecoveryMethod,
    ReplyQuote, ReplyQuoteCodeBlock, ReplyQuoteFormattedBody, ReplyQuoteState,
    RestrictedConditions, RoomAccessCondition, RoomAccessDraft, RoomAccessDraftScope,
    RoomAccessObservation, RoomAccessPolicy, RoomAccessPreview, RoomAccessPreviewContext,
    RoomActivityRecord, RoomActivityState, RoomAllowTarget, RoomAllowTargetKind,
    RoomAttentionCandidate, RoomAttentionKind, RoomAttentionProjection, RoomAttentionSummary,
    RoomHistoryVisibility, RoomInteractionState, RoomJoinRule, RoomKeyExportState,
    RoomKeyImportState, RoomLatestEventSummary, RoomListEntryKind, RoomListFailureKind,
    RoomListFilter, RoomListProjection, RoomListProjectionItem, RoomListReadiness, RoomListSort,
    RoomListSource, RoomLiveSignals, RoomManagementOperationKind, RoomManagementOperationState,
    RoomManagementState, RoomMemberMembership, RoomMemberRole, RoomMemberRoleOption,
    RoomMemberSummary, RoomMentionPermission, RoomModerationAction, RoomNamePlaceholder,
    RoomNotificationMode, RoomNotificationModeOperation, RoomNotificationSettings,
    RoomPermissionFacts, RoomPreference, RoomPreferencesState, RoomSettingChange,
    RoomSettingsSnapshot, RoomSummary, RoomTagInfo, RoomTagKind, RoomTags, RoomUrlPreviews,
    SESSION_STATUS_FAILURE_BACKOFF_BASE_MS, SESSION_STATUS_FAILURE_BACKOFF_CAP_MS,
    SESSION_STATUS_FRESHNESS_MS, SESSION_STATUS_SCHEDULED_REQUEST_ID_BASE, SIDEBAR_HOME_SCOPE_KEY,
    SasEmoji, ScheduledSendCapability, ScheduledSendHandle, ScheduledSendItem, ScheduledSendStore,
    ScheduledSendsListState, ScheduledSendsScope, SearchCrawlerFailureKind,
    SearchCrawlerLastActive, SearchCrawlerLastActiveStatus, SearchCrawlerRoomState,
    SearchCrawlerSettings, SearchCrawlerSpeed, SearchCrawlerState, SearchMatchField,
    SearchMatchKind, SearchResult, SearchRoomFilter, SearchScope, SearchState,
    SecureBackupFailureDetail, SecureBackupFailureStage, SecureBackupFailureTransport,
    SecureBackupGateFailureKind, SecureBackupGateState, SecureBackupInspectionFailure,
    SecureBackupMatrixErrorKind, SecureBackupPassphraseChangeState, SecureBackupSetupAdmission,
    SecureBackupSetupIntent, SecureBackupSetupState, SenderDeviceAdvisoriesState,
    ServerEmailDelivery, SessionAuthenticationMethod, SessionInfo, SessionLockReason, SessionState,
    SessionStatusCheckDecision, SessionStatusCheckStats, SessionStatusRefreshTrigger,
    SessionStatusSchedule, SettingsPatch, SettingsPatchScope, SettingsPersistenceState,
    SettingsState, SettingsValues, SidebarCategory, SidebarCollapsedSections, SidebarScopeSettings,
    SidebarSectionKind, SidebarSectionPatch, SidebarSectionReveal, SidebarSectionSettings,
    SidebarSettings, SmtpTls, SoftLogoutReauthState, SpaceChildMembership,
    SpaceChildSuggestedEditState, SpaceChildSummary, SpaceChildrenLoadState, SpaceChildrenState,
    SpaceConversationSurface, SpaceLocalPresentation, SpaceLocalPresentations, SpaceMemberEntry,
    SpaceMemberInviteOutcome, SpaceMemberMembership, SpaceMemberRoleFailureKind,
    SpaceMemberRoleOption, SpaceMemberRoleUpdateOutcome, SpaceMembersCommandRejection,
    SpaceMembersOperationState, SpaceMembersProjection, SpaceMembersState,
    SpaceNavigationSelection, SpaceSummary, StagedUploadCompressionChoice,
    StagedUploadFormatChoice, StagedUploadItem, StagedUploadKind, StagedUploadOutputSelection,
    StagedUploadPreparation, StagedUploadResizeChoice, SuggestedRoomCandidate,
    SuggestedRoomJoinOutcome, SuggestedRoomJoinResult, SuggestedRoomsOffer, SyncLifecycleStatus,
    SyncState, TextDirectionPreference, TextRange, ThemePreference, ThreadAttentionState,
    ThreadListOrder, ThreadOpenIntent, ThreadPaneState, ThreadRootAttention,
    ThreadRootProjectionState, ThreadRootProjectionStatus, ThreadsListItem, ThreadsListScope,
    ThreadsListState, TimelineContinuityInspection, TimelineContinuityState,
    TimelineGapRepairFailureKind, TimelineMediaDownloadState, TimelineMediaGalleryItem,
    TimelineMediaGalleryMedia, TimelineMediaGallerySource, TimelineMediaGalleryThumbnail,
    TimelineMediaKind, TimelinePaneState, TimelineScrollAnchor, TimelineScrollAnchorEdge,
    TimelineSettings, TimelineThreadRootOrder, TrustOperationFailureKind, TypographySettings,
    UnverifiedOwnSessionsAlert, UpdatesSettings, UploadStagingStore, UserProfile, UserTrustState,
    VerificationAccountKind, VerificationCancelReason, VerificationFlowState,
    VerificationGateFailureKind, VerificationGateRejectReason, VerificationGateState,
    VerificationInitiator, VerificationMethod, VerificationMethodCapability, VerificationTarget,
    WindowSettings, access_policy_target_rejection, admit_space_member_cancellation,
    admit_space_member_invite, admit_space_member_role, admit_space_members_load,
    attention_room_candidates, attention_room_candidates_for_state,
    bounded_notification_delivery_message_source, canonical_access_policy,
    compute_room_list_projection, confirmed_access_policy, confirmed_room_access_outcome,
    enrich_live_receipt, history_key_request_targets_all, is_ignored_user,
    native_attention_capabilities_for_platform, native_attention_projection_from_rooms,
    native_attention_state_from_rooms, normalize_notification_email,
    normalize_notification_keyword, normalize_notification_keyword_removal,
    own_session_check_due_at_ms, preview_create_room_access, preview_room_access_draft,
    refresh_live_receipt_display_projection, refresh_live_typing_user_display_projection,
    refresh_profile_user_display_projection, refresh_room_settings_member_display_projection,
    refresh_room_summary_display_projection, resolve_live_receipt_profile,
    resolve_optional_user_display_name, resolve_people_label, resolve_user_display_name,
    room_activity_unread_count, room_attention_kind, room_attention_projection,
    room_attention_summary, search_query_too_short, session_status_failure_backoff_ms,
    sidebar_scope_key, sort_threads_list_items, staged_upload_item_with_completed_output,
    staged_upload_send_consumes_composer_draft, staged_uploads_are_sendable,
    unverified_own_sessions_alert,
};

pub fn encrypted_messaging_is_admitted(state: &AppState) -> bool {
    matches!(state.session, SessionState::Ready(_)) && state.secure_backup_gate.backup_is_ready()
}
pub use state::{
    SlidingSyncAdmission, SlidingSyncAdmissionKind, SlidingSyncAdmissionSource,
    SlidingSyncCapabilityFailureKind, SlidingSyncCapabilityResult, SlidingSyncCapabilityState,
    SlidingSyncPositiveEvidence, SlidingSyncRevalidationState, ThreadDeliveryDiagnostic,
};
// Space add-existing-room projection and linking settlements (#1007).
pub use space_add_rooms::{
    SpaceAddRoomCandidate, SpaceAddRoomStatus, SpaceAddRoomsModel, space_add_rooms_for_state,
};
pub use state::{SpaceChildLinkOutcome, SpaceChildLinkResult, SpaceChildLinkResults};
// Leaving a Space together with its joined child rooms.
pub use space_leave::{
    SpaceLeaveCandidate, admit_space_leave_room_ids, space_leave_candidates_for_state,
};
// History export keeps its own block so the feature does not edit the shared
// list above.
pub use state::{
    HistoryExportFailureKind, HistoryExportRange, HistoryExportRoom, HistoryExportRoomCounts,
    HistoryExportRoomFailureKind, HistoryExportRoomPhase, HistoryExportRoomSkipReason,
    HistoryExportScope, HistoryExportState,
};
pub use submission::{
    ComposerSubmissionTarget, ComposerSubmissionTerminalOutcome, ComposerTarget, SubmissionId,
};
// Composer-draft persistence limits consumed by koushi-core's store and the
// state-crate integration tests; root-re-exported so `state` can stay crate-internal.
pub use state::{
    MAX_LIVE_COMPOSER_ROOM_TOMBSTONES, MAX_LIVE_COMPOSER_THREAD_TOMBSTONES,
    MAX_PERSISTED_COMPOSER_DRAFT_BYTES, MAX_PERSISTED_COMPOSER_DRAFT_ROOM_COUNT,
    MAX_PERSISTED_COMPOSER_DRAFT_THREAD_COUNT,
};
pub use typography_profile::{
    TypographyAssetStatus, TypographyDisplayProfile, resolve_typography_display_profile,
};

/// Curated entry-point re-exports for consumers that prefer a single glob import
/// (`use koushi_state::prelude::*`). This is a convenience surface over the
/// crate root; it intentionally exposes the common product-state types and the
/// reducer/sidebar entry points, not the full API.
pub mod prelude {
    pub use crate::{
        AppAction, AppEffect, AppError, AppState, ComposerResolvedAction, LocaleDisplayProfile,
        MentionIntent, SettingsPatch, SidebarModel, TypographyDisplayProfile, UiEvent,
        compose_sidebar, reduce, resolve_composer_key_action, resolve_locale_display_profile,
    };
}
