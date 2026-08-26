mod auth;

mod client_session;

mod e2ee;

mod login_store;

#[cfg(any(test, feature = "test-hooks"))]
pub mod login_store_test_support;

mod profile;

mod qa_reports;

mod room_operations;

mod room_projection;

mod search;

mod sync;

mod timeline;

mod sliding_sync_discovery;

#[cfg(test)]
mod test_source;

pub use auth::{
    Homeserver, LOCAL_USER_ALIASES_ACCOUNT_DATA_TYPE, LoginDiscovery, LoginDiscoveryError,
    MatrixLoginDiscovery, MatrixLoginFlow, MatrixLoginFlowKind, OidcAuthorization,
    PasswordLoginError, PendingOidcLogin, discover_login_flows, finish_oidc_login,
    login_with_password, login_with_password_blocking, login_with_password_with_new_device,
    login_with_password_with_store, login_with_password_with_store_and_device, logout_blocking,
    map_login_flows_to_desktop, parse_login_discovery, parse_login_discovery_http_response,
    parse_matrix_login_flows, parse_well_known_client,
    resolve_active_session_account_management_url, start_oidc_login, start_oidc_login_with_store,
};

pub use login_store::{
    LocalServerDeviceKeyComparison, SavedCryptoStorePreflight, preflight_saved_crypto_store,
};

pub use client_session::{
    MatrixClientSession, MatrixClientStoreConfig, MatrixClientStoreKey, MatrixEventCacheError,
    MatrixEventCacheStatus, MatrixSlidingSyncInviteListSupport, PersistableAuthKind,
    PersistableMatrixSession, ProvisionalEncryptionSyncError, enable_event_cache, logout,
    restore_session, restore_session_blocking, restore_session_with_store,
    restore_session_with_verified_store,
};

pub use e2ee::{
    AccountManagementCapabilities, AccountManagementError, CurrentDeviceTrustObservation,
    CurrentDeviceTrustRecheckError, CurrentDeviceTrustStream, E2eeRecoveryError,
    E2eeRecoveryStateStream, E2eeTrustError, E2eeTrustFailureKind, IdentityResetOutcome,
    KeyBackupRestoreScope, KeyBackupRestoreSummary, MatrixCrossSigningStatus,
    MatrixCurrentSessionInspection, MatrixCurrentSessionInspectionError,
    MatrixDeviceCleanupOutcome, MatrixDeviceNameOutcome, MatrixForceNewSessionOutcome,
    MatrixForceNewSessionSummary, MatrixIdentityResetAuthType, MatrixIdentityResetHandle,
    MatrixIncomingVerificationRequest, MatrixIncomingVerificationRequestObserver,
    MatrixIndex0ClaimOutcome, MatrixIndex0ResendOutcome, MatrixIndex0ResendSummary,
    MatrixIndex0ShareOutcome, MatrixIndex0ShareSummary, MatrixOutboundGroupSessionToken,
    MatrixOwnUserVerificationHandle, MatrixRoomKeyReceiveDiagnostics, MatrixRoomKeyReshareOutcome,
    MatrixRoomKeyReshareTarget, MatrixRoomKeyRotationReason, MatrixRoomKeyWithheldCode,
    MatrixSasState, MatrixSasStateStream, MatrixSasVerificationHandle,
    MatrixSecureBackupInspection, MatrixSecureBackupLocalState, MatrixSecureBackupRecoveryState,
    MatrixSecureBackupServerState, MatrixSecureBackupState, MatrixSecureBackupStateObservation,
    MatrixSecureBackupTrustState, MatrixSecureBackupUploadState, MatrixVerificationCancelKind,
    MatrixVerificationRequestHandle, MatrixVerificationRequestState,
    MatrixVerificationRequestStateStream, RoomKeyExportSummary, RoomKeyImportSummary,
    SecureBackupSetupSummary, SecureBackupStateStream, accept_sas_verification,
    accept_verification_request, account_management_capabilities, bootstrap_cross_signing,
    bootstrap_secure_backup, cancel_own_user_sas_verification, cancel_sas_verification,
    cancel_verification_request, change_password, change_secure_backup_passphrase,
    cleanup_current_device, complete_identity_reset, confirm_sas_verification,
    cross_signing_status, current_outbound_group_session_index,
    current_outbound_group_session_token, deactivate_account, discard_outbound_group_session,
    discover_current_session_verification_methods, download_joined_room_keys_from_backup,
    download_room_key_from_backup, enable_key_backup, ensure_device_display_name,
    force_new_outbound_session, force_reshare_room_key, has_inbound_group_session,
    late_decryption_report_stream, map_backup_state_to_desktop,
    map_cross_signing_status_to_desktop, map_identity_reset_auth_type_to_desktop,
    map_sdk_sas_emojis_to_desktop, mismatch_sas_verification,
    observe_incoming_verification_requests, preshare_outbound_group_session, recover_e2ee,
    recover_e2ee_blocking, request_device_verification, request_late_decryption,
    request_own_user_sas_verification, request_room_key_for_event, resend_index0_room_key,
    reset_identity, reshare_room_key, restore_key_backup, room_key_receive_diagnostics,
    room_key_rotation_reason, room_key_withheld_codes, room_key_withheld_stream,
    share_index0_room_key, start_own_user_sas_verification, start_sas_verification,
};

#[cfg(not(target_family = "wasm"))]
pub use e2ee::export_room_keys_to_file;

#[cfg(not(target_family = "wasm"))]
pub use e2ee::import_room_keys_from_file;

pub use profile::{
    MatrixIgnoredUserListError, MatrixIgnoredUserListFailureKind, MatrixLocalUserAliases,
    MatrixOwnProfile, MatrixProfileError, MatrixProfileFailureKind, MatrixReportError,
    MatrixReportFailureKind, get_ignored_user_list, get_local_user_aliases, get_own_profile,
    ignore_user, report_content, report_room, report_user, set_avatar, set_display_name,
    set_local_user_aliases, unignore_user, update_local_user_alias,
};

pub use qa_reports::{
    RealAccountQaReport, RoomListSmokeReport, SearchSmokeReport, TimelineSmokeReport,
    real_account_qa_report, real_account_qa_report_with_search, restored_real_account_qa_report,
    room_list_smoke_report, search_smoke_report, timeline_smoke_report,
};

pub use room_operations::{
    MatrixCreateRoomOptions, MatrixCreateRoomParentSpace, MatrixCreateRoomVisibility,
    MatrixJoinTarget, MatrixPreviewJoinability, MatrixPreviewMembership,
    MatrixPublicRoomDirectoryQuery, MatrixPublicRoomDirectoryResult, MatrixPublicRoomDirectoryRoom,
    MatrixRoomHistoryVisibility, MatrixRoomJoinRule, MatrixRoomMemberRole,
    MatrixRoomModerationAction, MatrixRoomOperationError, MatrixRoomOperationFailureKind,
    MatrixRoomPermissionFacts, MatrixRoomPreview, MatrixRoomSettingChange,
    MatrixRoomSettingsSnapshot, MatrixSpaceInviteCancellationOutcome,
    MatrixSpaceMemberRoleFailureKind, MatrixSpaceMemberRoleUpdateResult, MatrixUserTrustState,
    cancel_space_invite, create_public_directory_room, create_room, create_space, forget_room,
    get_room_settings_snapshot, invite_user_to_room, join_room_by_id, join_room_target, leave_room,
    load_pinned_event_ids, mark_room_as_read, mark_room_as_unread, moderate_room_member, pin_event,
    preview_join_target, query_public_room_directory, remove_room_tag, room_can_send_text_message,
    room_has_active_member_no_sync, room_id_server_name, room_is_joined,
    set_room_notification_mode, set_room_tag, set_space_child, start_direct_message, unpin_event,
    update_room_member_power_level, update_room_setting, update_space_member_power_level,
};

pub use room_projection::{
    MatrixCachedDirectAccountData, MatrixConversationActivity, MatrixConversationActivitySource,
    MatrixDirectTargetsByRoom, MatrixInvitePreview, MatrixJoinedMemberSnapshot,
    MatrixRoomLatestEventSummary, MatrixRoomListError, MatrixRoomListRoom, MatrixRoomListSnapshot,
    MatrixRoomListSpace, MatrixRoomMemberSummary, MatrixRoomTagInfo, MatrixRoomTagKind,
    MatrixRoomTags, MatrixSpaceMemberEntry, MatrixSpaceMemberRoleOption,
    MatrixSpaceMembersProjection, MatrixUserProfile, cached_direct_account_data_targets_by_room,
    direct_account_data_targets_by_room, matrix_space_members_projection,
    room_attention_summary_from_counts, room_attention_summary_from_room, room_list_snapshot,
    room_list_snapshot_blocking, room_list_snapshot_from_sdk_rooms,
    room_list_snapshot_from_sdk_rooms_with_direct_targets,
    room_list_snapshot_from_sdk_rooms_with_invites,
};

pub use search::{
    MatrixSearchCandidate, MatrixSearchError, MatrixSearchIndexKey, MatrixSearchIndexStoreConfig,
    MatrixSearchScope, search_message_candidates, search_message_candidates_blocking,
    search_message_candidates_scoped,
};

pub use sync::{
    EncryptionSyncLifecycleOwner, EncryptionSyncLifecycleStage, EncryptionSyncPermitOwner,
    MatrixSyncError, MatrixSyncLoopControl, close_session_stores, new_encryption_sync_permit_owner,
    probe_sliding_sync_invite_list_support, provisional_encryption_sync_loop,
    record_encryption_sync_lifecycle, sync_loop,
};

#[cfg(any(test, feature = "test-hooks", feature = "smoke"))]
pub use sync::sync_once_blocking;

#[cfg(any(test, feature = "test-hooks", feature = "smoke"))]
pub use sync::sync_once;

pub use timeline::{
    MatrixCommittedRoomTimelineCheckpoint, MatrixLiveTailRefreshCancellation,
    MatrixLiveTailRefreshDiagnostics, MatrixLiveTailRefreshOutcome, MatrixLiveTailRefreshResult,
    MatrixRoomSubscriptionCheckpoint, MatrixTimelineContinuity, MatrixTimelineError,
    MatrixTimelineGapError, MatrixTimelineGapHandle, MatrixTimelineGapInspection,
    MatrixTimelineGapRepairBudget, MatrixTimelineGapRepairOutcome, MatrixTimelineGapRepairResult,
    MatrixTimelineItem, MatrixTimelinePaginationHandle, MatrixTimelineSubscription,
    MatrixTimelineUpdate, MatrixTimelineUpdateStream, edit_text_message, redact_message,
    room_timeline_visible_items, room_timeline_visible_items_blocking, send_text_message,
    subscribe_room_timeline, subscribe_room_timeline_blocking,
};

pub use sliding_sync_discovery::{
    DiscoveryResponseFailureKind, DiscoverySource, DiscoveryTransportFailureKind, HttpStatusClass,
    SlidingSyncDiscoveryResult, discover_sliding_sync_support,
};

pub use koushi_state::E2eeRecoveryState;
