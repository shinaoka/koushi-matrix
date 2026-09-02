import { invoke } from "@tauri-apps/api/core";

import type {
  ComposerDraftAccountOwner,
  DesktopApi,
  ViewportSyncObservation,
  ViewportSyncReceipt
} from "./desktopApi";
import { COMPOSER_DRAFT_REVISION_ZERO } from "../domain/composerDraftRevision";
import type {
  ActivityMarkReadTarget,
  CommandAdmission,
  CommandSettlement,
  ActivityTab,
  DesktopSnapshot,
  ComposerKeyEvent,
  ComposerResolvedAction,
  ComposerResolverOptions,
  ComposerSurface,
  ComposerTarget,
  ComposerDocument,
  ComposerDraftRevision,
  ComposerDraftAcceptanceResponse,
  DirectoryQuery,
  MentionSurface,
  NavigationPreferenceUpdate,
  OidcAuthorization,
  PresenceKind,
  InviteScopeSelection,
  RoomListFilter,
  RoomModerationAction,
  RoomNotificationMode,
  RoomSettingChange,
  RoomTagKind,
  SavedSessionInfo,
  SearchScopeKind,
  SessionStatusRefreshCommandTrigger,
  SettingsPatch,
  StagedUploadCompressionChoice,
  StagedUploadOutputSelection,
  StageUploadBytesRequestItem,
  AttachmentFilter,
  AttachmentSort,
  CreateRoomRequest,
  FilesViewScope,
  SubmissionResponse,
  ThreadOpenIntent,
  ThreadsListScope
} from "../domain/types";
import type { DiagnosticLogSnapshot } from "../domain/diagnostics";
import type { TimelineKey } from "../domain/coreEvents";
import type {
  ComposerDraftLeaseSnapshot,
  ComposerDraftScope
} from "../domain/composerDraftLifecycle";
import type { DisplayPlatform } from "../domain/types";

export type DesktopInvoke = typeof invoke;

export class TauriDesktopApi implements DesktopApi {
  constructor(private readonly invokeCommand: DesktopInvoke = invoke) {}
  async getSnapshot(): Promise<DesktopSnapshot> {
    return this.invokeCommand<DesktopSnapshot>("get_snapshot");
  }

  async settlementSnapshot(): Promise<DesktopSnapshot> {
    return this.invokeCommand<DesktopSnapshot>("settlement_snapshot");
  }

  async resyncSnapshot(): Promise<DesktopSnapshot> {
    return this.invokeCommand<DesktopSnapshot>("resync_snapshot");
  }

  async getDiagnosticSnapshot(): Promise<DiagnosticLogSnapshot> {
    return this.invokeCommand<DiagnosticLogSnapshot>("get_diagnostic_snapshot");
  }

  async observeViewportSync(
    observation: ViewportSyncObservation
  ): Promise<ViewportSyncReceipt> {
    return this.invokeCommand<ViewportSyncReceipt>("observe_viewport_sync", { observation });
  }

  async discoverLoginMethods(homeserver: string): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("discover_login_methods", { homeserver });
  }

  async startOidcLogin(homeserver: string): Promise<OidcAuthorization> {
    return this.invokeCommand<OidcAuthorization>("start_oidc_login", { homeserver });
  }

  async completeOidcLogin(
    homeserver: string,
    callbackUrl: string
  ): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("complete_oidc_login", { homeserver, callbackUrl });
  }

  async submitLogin(
    homeserver: string,
    username: string,
    password: string,
    deviceDisplayName: string,
    platform: DisplayPlatform
  ): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("submit_login", {
      homeserver,
      username,
      password,
      // An empty device name must not reach the server: the Rust core then
      // applies the platform-aware default ("Koushi on …") after login.
      deviceDisplayName: deviceDisplayName.trim() || undefined,
      platform
    });
  }

  async submitSoftLogoutReauth(password: string): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("submit_soft_logout_reauth", { password });
  }

  async listSavedSessions(): Promise<SavedSessionInfo[]> {
    return this.invokeCommand<SavedSessionInfo[]>("list_saved_sessions");
  }

  async switchAccount(session: SavedSessionInfo): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("switch_account", {
      homeserver: session.homeserver,
      userId: session.user_id,
      deviceId: session.device_id
    });
  }

  async retrySlidingSyncCapability(): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("retry_sliding_sync_capability");
  }

  async changeHomeserver(): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("change_homeserver");
  }

  async logout(): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("logout");
  }

  async submitRecovery(secret: string): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("submit_recovery", { secret });
  }

  async recoverSecureBackup(secret: string): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("recover_secure_backup", { secret });
  }

  async startDeviceCleanup(): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("start_device_cleanup");
  }

  async submitDeviceCleanupUia(flowId: number, password: string): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("submit_device_cleanup_uia", { flowId, password });
  }

  async eraseLocalDataAnyway(): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("erase_local_data_anyway");
  }

  async restartSync(): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("restart_sync");
  }

  async updateSettings(patch: SettingsPatch): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("update_settings", { patch });
  }

  async importLegacySettings(patch: SettingsPatch): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("import_legacy_settings", { patch });
  }

  async updateNavigationPreference(
    update: NavigationPreferenceUpdate
  ): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("update_navigation_preference", { update });
  }

  async rebuildSearchIndex(): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("rebuild_search_index");
  }

  async setRoomUrlPreviewOverride(
    roomId: string,
    enabled: boolean
  ): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("set_room_url_preview_override", { roomId, enabled });
  }

  async selectRoomListFilter(filter: RoomListFilter): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("select_room_list_filter", { filter });
  }

  async markRoomAsRead(roomId: string, eventId: string): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("mark_room_as_read", { roomId, eventId });
  }

  async markRoomAsUnread(roomId: string, unread: boolean): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("mark_room_as_unread", { roomId, unread });
  }

  async forceRotateOutboundSession(roomId: string): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("force_rotate_outbound_session", { roomId });
  }

  async setRoomNotificationMode(
    roomId: string,
    mode: RoomNotificationMode
  ): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("set_room_notification_mode", { roomId, mode });
  }

  async refreshCurrentSessionStatus(
    trigger: SessionStatusRefreshCommandTrigger
  ): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("refresh_current_session_status", { trigger });
  }

  async submitAccountManagementUia(flowId: number, password: string): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("submit_account_management_uia", { flowId, password });
  }

  async loadAccountManagementCapabilities(): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("load_account_management_capabilities");
  }

  async changePassword(newPassword: string): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("change_password", { newPassword });
  }

  async deactivateAccount(eraseData: boolean): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("deactivate_account", { eraseData });
  }

  async probeLocalEncryptionHealth(): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("probe_local_encryption_health");
  }

  async resetLocalData(): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("reset_local_data");
  }

  async bootstrapCrossSigning(): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("bootstrap_cross_signing");
  }

  async enableKeyBackup(): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("enable_key_backup");
  }

  async exportRoomKeys(destinationPath: string, passphrase: string): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("export_room_keys", { destinationPath, passphrase });
  }

  async importRoomKeys(sourcePath: string, passphrase: string): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("import_room_keys", { sourcePath, passphrase });
  }

  async bootstrapSecureBackup(
    passphrase: string | null,
    recoveryKeyDestinationPath: string | null,
    intent: import("../domain/types").SecureBackupSetupIntent
  ): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("bootstrap_secure_backup", {
      passphrase,
      recoveryKeyDestinationPath,
      intent
    });
  }

  async retrySecureBackupInspection(): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("retry_secure_backup_inspection");
  }

  async changeSecureBackupPassphrase(
    oldSecret: string,
    newPassphrase: string,
    recoveryKeyDestinationPath: string | null
  ): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("change_secure_backup_passphrase", {
      oldSecret,
      newPassphrase,
      recoveryKeyDestinationPath
    });
  }

  async acceptVerification(flowId: number): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("accept_verification", { flowId });
  }

  async startOwnUserSas(): Promise<CommandAdmission> { return this.invokeCommand("start_own_user_sas"); }
  async retryCurrentDeviceTrustDiscovery(): Promise<CommandAdmission> { return this.invokeCommand("retry_current_device_trust_discovery"); }
  async mismatchSasVerification(flowId: number): Promise<CommandAdmission> { return this.invokeCommand("mismatch_sas_verification", { flowId }); }
  async startSessionBootstrap(passphrase: string | null, recoveryKeyDestinationPath: string): Promise<CommandAdmission> { return this.invokeCommand("start_session_bootstrap", { passphrase, recoveryKeyDestinationPath }); }
  async confirmSessionBootstrapSaved(flowId: number): Promise<CommandAdmission> { return this.invokeCommand("confirm_session_bootstrap_saved", { flowId }); }

  async confirmSasVerification(flowId: number): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("confirm_sas_verification", { flowId });
  }

  async cancelVerification(flowId: number): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("cancel_verification", { flowId });
  }

  async resetIdentity(): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("reset_identity");
  }

  async cancelIdentityReset(flowId: number): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("cancel_identity_reset", { flowId });
  }

  async submitIdentityResetPassword(
    flowId: number,
    password: string
  ): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("submit_identity_reset_password", { flowId, password });
  }

  async submitIdentityResetOAuth(flowId: number): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("submit_identity_reset_oauth", { flowId });
  }

  async resolveComposerKeyAction(
    surface: ComposerSurface,
    keyEvent: ComposerKeyEvent,
    options: ComposerResolverOptions
  ): Promise<ComposerResolvedAction> {
    return this.invokeCommand<ComposerResolvedAction>("resolve_composer_key_action", {
      surface,
      keyEvent,
      autocompleteOpen: options.autocomplete_open,
      sendEnabled: options.send_enabled
    });
  }

  async selectSpace(spaceId: string | null): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("select_space", { spaceId });
  }

  async reorderSpaces(spaceIds: string[]): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("reorder_spaces", { spaceIds });
  }

  async selectRoom(roomId: string): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("select_room", { roomId });
  }

  async beginComposerDraftRendererGeneration(): Promise<string> {
    return this.invokeCommand<string>("begin_composer_draft_renderer_generation");
  }

  async acquireComposerDraftLease(
    scope: ComposerDraftScope,
    rendererGeneration: string
  ): Promise<ComposerDraftLeaseSnapshot> {
    return this.invokeCommand<ComposerDraftLeaseSnapshot>("acquire_composer_draft_lease", {
      accountHomeserver: scope.account.homeserver,
      accountUserId: scope.account.user_id,
      accountDeviceId: scope.account.device_id,
      target: scope.target,
      rendererGeneration
    });
  }

  async releaseComposerDraftLease(
    leaseId: string,
    rendererGeneration: string
  ): Promise<void> {
    return this.invokeCommand<void>("release_composer_draft_lease", {
      leaseId,
      rendererGeneration
    });
  }

  async sendText(
    account: ComposerDraftAccountOwner,
    leaseId: string,
    rendererGeneration: string,
    submissionId: string,
    roomId: string,
    document: ComposerDocument,
    draftRevision: ComposerDraftRevision = COMPOSER_DRAFT_REVISION_ZERO
  ): Promise<SubmissionResponse> {
    return this.invokeCommand<SubmissionResponse>("send_text", {
      accountHomeserver: account.homeserver,
      accountUserId: account.userId,
      accountDeviceId: account.deviceId,
      leaseId,
      rendererGeneration,
      submissionId,
      roomId,
      document,
      draftRevision
    });
  }

  async scheduleSend(
    account: ComposerDraftAccountOwner,
    leaseId: string,
    rendererGeneration: string,
    target: ComposerTarget,
    body: string,
    sendAtMs: number,
    draftRevision: ComposerDraftRevision
  ): Promise<ComposerDraftAcceptanceResponse> {
    return this.invokeCommand<ComposerDraftAcceptanceResponse>("schedule_send", {
      accountHomeserver: account.homeserver,
      accountUserId: account.userId,
      accountDeviceId: account.deviceId,
      leaseId,
      rendererGeneration,
      target,
      body,
      sendAtMs,
      draftRevision
    });
  }

  async stageUploadBytes(
    target: ComposerTarget,
    items: StageUploadBytesRequestItem[]
  ): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("stage_upload_bytes", { target, items });
  }

  async selectStagedUploadOutput(
    target: ComposerTarget,
    stagedId: string,
    selection: StagedUploadOutputSelection
  ): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("select_staged_upload_output", {
      target,
      stagedId,
      selection
    });
  }

  async preparedUploadPreview(
    target: ComposerTarget,
    stagedId: string,
    variantId: string
  ): Promise<number[]> {
    return this.invokeCommand<number[]>("prepared_upload_preview", { target, stagedId, variantId });
  }

  async retryStagedUploadPreparation(
    target: ComposerTarget,
    stagedId: string
  ): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("retry_staged_upload_preparation", { target, stagedId });
  }

  async useOriginalStagedUpload(
    target: ComposerTarget,
    stagedId: string
  ): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("use_original_staged_upload", { target, stagedId });
  }

  async sendPreparedUploads(
    account: ComposerDraftAccountOwner,
    leaseId: string,
    rendererGeneration: string,
    target: ComposerTarget,
    draftRevision: ComposerDraftRevision
  ): Promise<ComposerDraftAcceptanceResponse> {
    return this.invokeCommand<ComposerDraftAcceptanceResponse>("send_prepared_uploads", {
      accountHomeserver: account.homeserver,
      accountUserId: account.userId,
      accountDeviceId: account.deviceId,
      leaseId,
      rendererGeneration,
      target,
      draftRevision
    });
  }

  async updateStagedUploadCaption(
    target: ComposerTarget,
    stagedId: string,
    document: ComposerDocument | null
  ): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("update_staged_upload_caption", { target, stagedId, document });
  }

  async updateStagedUploadCompression(
    target: ComposerTarget,
    stagedId: string,
    compressionChoice: StagedUploadCompressionChoice
  ): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("update_staged_upload_compression", {
      target,
      stagedId,
      compressionChoice
    });
  }

  async clearUploadStaging(target: ComposerTarget): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("clear_upload_staging", { target });
  }

  async cancelScheduledSend(scheduledId: string): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("cancel_scheduled_send", { scheduledId });
  }

  async rescheduleScheduledSend(
    scheduledId: string,
    body: string,
    sendAtMs: number
  ): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("reschedule_scheduled_send", { scheduledId, body, sendAtMs });
  }

  async retrySend(roomId: string, transactionId: string): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("retry_send", { roomId, transactionId });
  }

  async cancelSend(roomId: string, transactionId: string): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("cancel_send", { roomId, transactionId });
  }

  async sendReaction(
    roomId: string,
    eventId: string,
    reactionKey: string
  ): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("send_reaction", { roomId, eventId, reactionKey });
  }

  async redactReaction(
    roomId: string,
    eventId: string,
    reactionKey: string,
    reactionEventId: string
  ): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("redact_reaction", {
      roomId,
      eventId,
      reactionKey,
      reactionEventId
    });
  }

  async sendReadReceipt(
    roomId: string,
    eventId: string,
    threadRootEventId?: string | null
  ): Promise<void> {
    return this.invokeCommand<void>("send_read_receipt", { roomId, eventId, threadRootEventId });
  }

  async setFullyRead(roomId: string, eventId: string): Promise<void> {
    return this.invokeCommand<void>("set_fully_read", { roomId, eventId });
  }

  async setTyping(roomId: string, isTyping: boolean): Promise<void> {
    return this.invokeCommand<void>("set_typing", { roomId, isTyping });
  }

  async setPresence(presence: PresenceKind): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("set_presence", { presence });
  }

  async setDisplayName(displayName: string | null): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("set_display_name", { displayName });
  }

  async setLocalUserAlias(userId: string, alias: string | null): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("set_local_user_alias", { userId, alias });
  }

  async ignoreUser(userId: string): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("ignore_user", { userId });
  }

  async unignoreUser(userId: string): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("unignore_user", { userId });
  }

  async reportUser(userId: string, reason: string): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("report_user", { userId, reason });
  }

  async reportContent(
    roomId: string,
    eventId: string,
    reason: string
  ): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("report_content", { roomId, eventId, reason });
  }

  async reportRoom(roomId: string, reason: string): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("report_room", { roomId, reason });
  }

  async setAvatar(mimeType: string, bytes: number[]): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("set_avatar", { mimeType, bytes });
  }

  async editMessage(
    roomId: string,
    eventId: string,
    document: ComposerDocument
  ): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("edit_message", { roomId, eventId, document });
  }

  async redactMessage(roomId: string, eventId: string): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("redact_message", { roomId, eventId });
  }

  async loadMessageSource(roomId: string, eventId: string): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("load_message_source", { roomId, eventId });
  }

  async requestRoomKey(
    roomId: string,
    eventId: string,
    origin?: "user" | "automatic",
    timelineKey?: TimelineKey
  ): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("request_room_key", { roomId, eventId, origin, timelineKey });
  }

  async requestLateDecryption(
    roomId: string,
    timelineKey?: TimelineKey
  ): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("request_late_decryption", { roomId, timelineKey });
  }

  async forwardMessage(
    roomId: string,
    sourceEventId: string,
    destinationRoomId: string
  ): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("forward_message", {
      roomId,
      sourceEventId,
      destinationRoomId
    });
  }

  async loadLinkPreviews(roomId: string, eventId: string): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("load_link_previews", { roomId, eventId });
  }

  async hideLinkPreview(roomId: string, eventId: string): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("hide_link_preview", { roomId, eventId });
  }

  async leaveRoom(roomId: string): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("leave_room", { roomId });
  }

  async forgetRoom(roomId: string): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("forget_room", { roomId });
  }

  async setRoomTag(
    roomId: string,
    tag: RoomTagKind,
    order: number | null = null
  ): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("set_room_tag", { roomId, tag, order });
  }

  async removeRoomTag(roomId: string, tag: RoomTagKind): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("remove_room_tag", { roomId, tag });
  }

  async pinEvent(roomId: string, eventId: string): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("pin_event", { roomId, eventId });
  }

  async unpinEvent(roomId: string, eventId: string): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("unpin_event", { roomId, eventId });
  }

  async loadRoomSettings(roomId: string): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("load_room_settings", { roomId });
  }

  async loadSpaceMembers(spaceId: string, generation: number): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("load_space_members", { spaceId, generation });
  }

  async queryMentionCandidates(
    roomId: string,
    surface: MentionSurface,
    query: string
  ): Promise<void> {
    return this.invokeCommand<void>("query_mention_candidates", { roomId, surface, query });
  }

  async repairRoomTimeline(roomId: string): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("repair_room_timeline", { roomId });
  }

  async updateRoomSetting(
    roomId: string,
    change: RoomSettingChange
  ): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("update_room_setting", { roomId, change });
  }

  async moderateRoomMember(
    roomId: string,
    targetUserId: string,
    action: RoomModerationAction,
    reason: string | null = null
  ): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("moderate_room_member", {
      roomId,
      targetUserId,
      action,
      reason
    });
  }

  async updateRoomMemberRole(
    roomId: string,
    targetUserId: string,
    powerLevel: number
  ): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("update_room_member_role", {
      roomId,
      targetUserId,
      powerLevel
    });
  }

  async updateSpaceMemberRole(
    spaceId: string,
    userId: string,
    generation: number,
    expectedPowerLevelsRevision: string | null,
    expectedPowerLevel: number,
    powerLevel: number,
    confirmed: boolean
  ): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("update_space_member_role", {
      spaceId,
      userId,
      generation,
      expectedPowerLevelsRevision,
      expectedPowerLevel,
      powerLevel,
      confirmed
    });
  }

  async openActivity(): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("open_activity");
  }

  async closeActivity(): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("close_activity");
  }

  async setActivityTab(tab: ActivityTab): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("set_activity_tab", { tab });
  }

  async paginateActivity(
    tab: ActivityTab,
    cursor: string | null = null
  ): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("paginate_activity", { tab, cursor });
  }

  async retryActivityResolution(): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("retry_activity_resolution");
  }

  async markActivityRead(target: ActivityMarkReadTarget): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("mark_activity_read", { target });
  }

  async setComposerDraft(
    account: ComposerDraftAccountOwner,
    leaseId: string,
    rendererGeneration: string,
    roomId: string,
    document: ComposerDocument,
    revision: ComposerDraftRevision
  ): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("set_composer_draft", {
      accountHomeserver: account.homeserver,
      accountUserId: account.userId,
      accountDeviceId: account.deviceId,
      leaseId,
      rendererGeneration,
      roomId,
      document,
      draftRevision: revision
    });
  }

  async openThread(
    roomId: string,
    rootEventId: string,
    intent: ThreadOpenIntent
  ): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("open_thread", { roomId, rootEventId, intent });
  }

  async closeThread(): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("close_thread");
  }

  async openThreadsList(scope: ThreadsListScope): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("open_threads_list", { scope });
  }

  async closeThreadsList(): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("close_threads_list");
  }

  async openFilesView(
    scope: FilesViewScope,
    filter: AttachmentFilter,
    sort: AttachmentSort
  ): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("open_files_view", { scope, filter, sort });
  }

  async closeFilesView(): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("close_files_view");
  }

  async paginateThreadsList(scope: ThreadsListScope): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("paginate_threads_list", { scope });
  }

  async setThreadComposerDraft(
    account: ComposerDraftAccountOwner,
    leaseId: string,
    rendererGeneration: string,
    roomId: string,
    rootEventId: string,
    document: ComposerDocument,
    revision: ComposerDraftRevision
  ): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("set_thread_composer_draft", {
      accountHomeserver: account.homeserver,
      accountUserId: account.userId,
      accountDeviceId: account.deviceId,
      leaseId,
      rendererGeneration,
      roomId,
      rootEventId,
      document,
      draftRevision: revision
    });
  }

  async sendThreadReply(
    account: ComposerDraftAccountOwner,
    leaseId: string,
    rendererGeneration: string,
    submissionId: string,
    roomId: string,
    rootEventId: string,
    document: ComposerDocument,
    draftRevision: ComposerDraftRevision = COMPOSER_DRAFT_REVISION_ZERO
  ): Promise<SubmissionResponse> {
    return this.invokeCommand<SubmissionResponse>("send_thread_reply", {
      accountHomeserver: account.homeserver,
      accountUserId: account.userId,
      accountDeviceId: account.deviceId,
      leaseId,
      rendererGeneration,
      submissionId,
      roomId,
      rootEventId,
      document,
      draftRevision
    });
  }

  async selectSearchResult(roomId: string, eventId: string): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("select_search_result", { roomId, eventId });
  }

  async openActivityEvent(roomId: string, eventId: string): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("open_activity_event", { roomId, eventId });
  }

  async openPinnedEvent(roomId: string, eventId: string): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("open_pinned_event", { roomId, eventId });
  }

  async openTimelineAtTimestamp(
    roomId: string,
    timestampMs: number
  ): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("open_timeline_at_timestamp", { roomId, timestampMs });
  }

  async closeFocusedContext(): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("close_focused_context");
  }

  async closeSearch(): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("close_search");
  }

  async submitSearch(query: string, scope: SearchScopeKind): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("submit_search", { query, scope });
  }

  async queryDirectory(query: DirectoryQuery): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("query_directory", {
      term: query.term,
      serverName: query.server_name,
      limit: query.limit,
      since: query.since
    });
  }

  async joinDirectoryRoom(
    roomIdOrAlias: string,
    viaServers: string[] = []
  ): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("join_directory_room", {
      roomIdOrAlias,
      viaServers
    });
  }

  async previewJoinTarget(
    roomIdOrAlias: string,
    viaServers: string[] = []
  ): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("preview_join_target", {
      roomIdOrAlias,
      viaServers
    });
  }

  async dismissDirectoryPreview(): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("dismiss_directory_preview", {});
  }

  async joinRoom(roomId: string): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("join_room", { roomId });
  }

  async createRoom(request: CreateRoomRequest): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("create_room", { options: request });
  }

  async createSpace(name: string): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("create_space", { name });
  }

  async setSpaceChild(spaceId: string, childRoomId: string, viaServer: string): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("set_space_child", { spaceId, childRoomId, viaServer });
  }

  async acceptInvite(roomId: string): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("accept_invite", { roomId });
  }

  async declineInvite(roomId: string): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("decline_invite", { roomId });
  }

  async startDirectMessage(userId: string): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("start_direct_message", { userId });
  }

  async inviteUser(roomId: string, userId: string): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("invite_user", { roomId, userId });
  }

  async inviteUserToSpace(
    spaceId: string,
    userId: string,
    generation: number
  ): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("invite_user_to_space", {
      spaceId,
      userId,
      generation
    });
  }

  async cancelSpaceInvite(
    spaceId: string,
    userId: string,
    generation: number
  ): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("cancel_space_invite", {
      spaceId,
      userId,
      generation
    });
  }

  async openInviteWorkflow(roomId: string): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("open_invite_workflow", { roomId });
  }

  async closeInviteWorkflow(): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("close_invite_workflow");
  }

  async searchInviteTargets(roomId: string, query: string): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("search_invite_targets", { roomId, query });
  }

  async setInviteScope(roomId: string, scope: InviteScopeSelection): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("set_invite_scope", { roomId, scope });
  }

  async selectInviteTarget(roomId: string, userId: string): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("select_invite_target", { roomId, userId });
  }

  async removeInviteTarget(userId: string): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("remove_invite_target", { userId });
  }

  async inviteTargets(
    roomId: string,
    userIds: string[],
    scope: InviteScopeSelection
  ): Promise<CommandSettlement> {
    return this.invokeCommand<CommandSettlement>("invite_targets", { roomId, userIds, scope });
  }

  async setComposerReplyTarget(roomId: string, eventId: string): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("set_composer_reply_target", { roomId, eventId });
  }

  async cancelComposerReply(): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("cancel_composer_reply");
  }

  async sendReply(
    account: ComposerDraftAccountOwner,
    leaseId: string,
    rendererGeneration: string,
    submissionId: string,
    roomId: string,
    inReplyToEventId: string,
    document: ComposerDocument,
    draftRevision: ComposerDraftRevision = COMPOSER_DRAFT_REVISION_ZERO
  ): Promise<SubmissionResponse> {
    return this.invokeCommand<SubmissionResponse>("send_reply", {
      accountHomeserver: account.homeserver,
      accountUserId: account.userId,
      accountDeviceId: account.deviceId,
      leaseId,
      rendererGeneration,
      submissionId,
      roomId,
      inReplyToEventId,
      document,
      draftRevision
    });
  }

  async startRoomCrawl(roomId: string): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("start_room_crawl", { roomId });
  }

  async stopRoomCrawl(roomId: string): Promise<CommandAdmission> {
    return this.invokeCommand<CommandAdmission>("stop_room_crawl", { roomId });
  }
}
