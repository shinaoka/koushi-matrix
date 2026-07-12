import { invoke } from "@tauri-apps/api/core";

import { createBrowserFakeApi, type DesktopApi } from "./browserFakeApi";
import type {
  ActivityMarkReadTarget,
  ActivityTab,
  DesktopSnapshot,
  ComposerKeyEvent,
  ComposerResolvedAction,
  ComposerResolverOptions,
  ComposerSurface,
  DirectoryQuery,
  MentionIntent,
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
  SettingsPatch,
  StagedUploadCompressionChoice,
  UploadStagingRequestItem,
  AttachmentFilter,
  AttachmentSort,
  CreateRoomRequest,
  FilesViewScope
  ,SubmissionResponse
} from "../domain/types";
import type { DiagnosticLogSnapshot } from "../domain/diagnostics";

export function createDesktopApi(): DesktopApi {
  if (isTauriRuntime()) {
    return new TauriDesktopApi();
  }

  return createBrowserFakeApi();
}

class TauriDesktopApi implements DesktopApi {
  async getSnapshot(): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("get_snapshot");
  }

  async getDiagnosticSnapshot(): Promise<DiagnosticLogSnapshot> {
    return invoke<DiagnosticLogSnapshot>("get_diagnostic_snapshot");
  }

  async discoverLoginMethods(homeserver: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("discover_login_methods", { homeserver });
  }

  async startOidcLogin(homeserver: string): Promise<OidcAuthorization> {
    return invoke<OidcAuthorization>("start_oidc_login", { homeserver });
  }

  async completeOidcLogin(
    homeserver: string,
    callbackUrl: string
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("complete_oidc_login", { homeserver, callbackUrl });
  }

  async submitLogin(
    homeserver: string,
    username: string,
    password: string,
    deviceDisplayName: string
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("submit_login", {
      homeserver,
      username,
      password,
      deviceDisplayName
    });
  }

  async submitSoftLogoutReauth(password: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("submit_soft_logout_reauth", { password });
  }

  async listSavedSessions(): Promise<SavedSessionInfo[]> {
    return invoke<SavedSessionInfo[]>("list_saved_sessions");
  }

  async switchAccount(session: SavedSessionInfo): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("switch_account", {
      homeserver: session.homeserver,
      userId: session.user_id,
      deviceId: session.device_id
    });
  }

  async logout(): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("logout");
  }

  async submitRecovery(secret: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("submit_recovery", { secret });
  }

  async restartSync(): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("restart_sync");
  }

  async updateSettings(patch: SettingsPatch): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("update_settings", { patch });
  }

  async rebuildSearchIndex(): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("rebuild_search_index");
  }

  async setRoomUrlPreviewOverride(
    roomId: string,
    enabled: boolean
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("set_room_url_preview_override", { roomId, enabled });
  }

  async selectRoomListFilter(filter: RoomListFilter): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("select_room_list_filter", { filter });
  }

  setRoomListProjection(): void {
    // No-op in Tauri runtime; helper exists only for browser fakes/tests.
  }

  async markRoomAsRead(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("mark_room_as_read", { roomId, eventId });
  }

  async markRoomAsUnread(roomId: string, unread: boolean): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("mark_room_as_unread", { roomId, unread });
  }

  async setRoomNotificationMode(
    roomId: string,
    mode: RoomNotificationMode
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("set_room_notification_mode", { roomId, mode });
  }

  async queryDevices(): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("query_devices");
  }

  async renameDevice(deviceOrdinal: number, displayName: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("rename_device", { deviceOrdinal, displayName });
  }

  async deleteDevices(deviceOrdinals: number[]): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("delete_devices", { deviceOrdinals });
  }

  async submitAccountManagementUia(flowId: number, password: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("submit_account_management_uia", { flowId, password });
  }

  async loadAccountManagementCapabilities(): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("load_account_management_capabilities");
  }

  async changePassword(newPassword: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("change_password", { newPassword });
  }

  async deactivateAccount(eraseData: boolean): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("deactivate_account", { eraseData });
  }

  async probeLocalEncryptionHealth(): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("probe_local_encryption_health");
  }

  async resetLocalData(): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("reset_local_data");
  }

  async bootstrapCrossSigning(): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("bootstrap_cross_signing");
  }

  async enableKeyBackup(): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("enable_key_backup");
  }

  async exportRoomKeys(destinationPath: string, passphrase: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("export_room_keys", { destinationPath, passphrase });
  }

  async importRoomKeys(sourcePath: string, passphrase: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("import_room_keys", { sourcePath, passphrase });
  }

  async bootstrapSecureBackup(
    passphrase: string | null,
    recoveryKeyDestinationPath: string | null
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("bootstrap_secure_backup", {
      passphrase,
      recoveryKeyDestinationPath
    });
  }

  async changeSecureBackupPassphrase(
    oldSecret: string,
    newPassphrase: string,
    recoveryKeyDestinationPath: string | null
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("change_secure_backup_passphrase", {
      oldSecret,
      newPassphrase,
      recoveryKeyDestinationPath
    });
  }

  async acceptVerification(flowId: number): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("accept_verification", { flowId });
  }

  async startOwnUserSas(): Promise<DesktopSnapshot> { return invoke("start_own_user_sas"); }
  async retryCurrentDeviceTrustDiscovery(): Promise<DesktopSnapshot> { return invoke("retry_current_device_trust_discovery"); }
  async mismatchSasVerification(flowId: number): Promise<DesktopSnapshot> { return invoke("mismatch_sas_verification", { flowId }); }
  async startSessionBootstrap(passphrase: string | null, recoveryKeyDestinationPath: string): Promise<DesktopSnapshot> { return invoke("start_session_bootstrap", { passphrase, recoveryKeyDestinationPath }); }
  async confirmSessionBootstrapSaved(flowId: number): Promise<DesktopSnapshot> { return invoke("confirm_session_bootstrap_saved", { flowId }); }

  async confirmSasVerification(flowId: number): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("confirm_sas_verification", { flowId });
  }

  async cancelVerification(flowId: number): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("cancel_verification", { flowId });
  }

  async resetIdentity(): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("reset_identity");
  }

  async cancelIdentityReset(flowId: number): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("cancel_identity_reset", { flowId });
  }

  async submitIdentityResetPassword(
    flowId: number,
    password: string
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("submit_identity_reset_password", { flowId, password });
  }

  async submitIdentityResetOAuth(flowId: number): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("submit_identity_reset_oauth", { flowId });
  }

  async resolveComposerKeyAction(
    surface: ComposerSurface,
    keyEvent: ComposerKeyEvent,
    options: ComposerResolverOptions
  ): Promise<ComposerResolvedAction> {
    return invoke<ComposerResolvedAction>("resolve_composer_key_action", {
      surface,
      keyEvent,
      autocompleteOpen: options.autocomplete_open,
      sendEnabled: options.send_enabled
    });
  }

  async selectSpace(spaceId: string | null): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("select_space", { spaceId });
  }

  async reorderSpaces(spaceIds: string[]): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("reorder_spaces", { spaceIds });
  }

  async selectRoom(roomId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("select_room", { roomId });
  }

  async sendText(
    submissionId: string,
    roomId: string,
    body: string,
    mentions: MentionIntent = { targets: [] }
  ): Promise<SubmissionResponse> {
    return invoke<SubmissionResponse>("send_text", { submissionId, roomId, body, mentions });
  }

  async scheduleSend(
    roomId: string,
    body: string,
    sendAtMs: number
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("schedule_send", { roomId, body, sendAtMs });
  }

  async stageUploads(
    roomId: string,
    items: UploadStagingRequestItem[]
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("stage_uploads", { roomId, items });
  }

  async updateStagedUploadCaption(
    stagedId: string,
    caption: string | null
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("update_staged_upload_caption", { stagedId, caption });
  }

  async updateStagedUploadCompression(
    stagedId: string,
    compressionChoice: StagedUploadCompressionChoice
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("update_staged_upload_compression", {
      stagedId,
      compressionChoice
    });
  }

  async clearUploadStaging(roomId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("clear_upload_staging", { roomId });
  }

  async cancelScheduledSend(scheduledId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("cancel_scheduled_send", { scheduledId });
  }

  async rescheduleScheduledSend(
    scheduledId: string,
    sendAtMs: number
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("reschedule_scheduled_send", { scheduledId, sendAtMs });
  }

  async retrySend(roomId: string, transactionId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("retry_send", { roomId, transactionId });
  }

  async cancelSend(roomId: string, transactionId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("cancel_send", { roomId, transactionId });
  }

  async sendReaction(
    roomId: string,
    eventId: string,
    reactionKey: string
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("send_reaction", { roomId, eventId, reactionKey });
  }

  async redactReaction(
    roomId: string,
    eventId: string,
    reactionKey: string,
    reactionEventId: string
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("redact_reaction", {
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
    return invoke<void>("send_read_receipt", { roomId, eventId, threadRootEventId });
  }

  async setFullyRead(roomId: string, eventId: string): Promise<void> {
    return invoke<void>("set_fully_read", { roomId, eventId });
  }

  async setTyping(roomId: string, isTyping: boolean): Promise<void> {
    return invoke<void>("set_typing", { roomId, isTyping });
  }

  async setPresence(presence: PresenceKind): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("set_presence", { presence });
  }

  async setDisplayName(displayName: string | null): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("set_display_name", { displayName });
  }

  async setLocalUserAlias(userId: string, alias: string | null): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("set_local_user_alias", { userId, alias });
  }

  async ignoreUser(userId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("ignore_user", { userId });
  }

  async unignoreUser(userId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("unignore_user", { userId });
  }

  async reportUser(userId: string, reason: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("report_user", { userId, reason });
  }

  async reportContent(
    roomId: string,
    eventId: string,
    reason: string
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("report_content", { roomId, eventId, reason });
  }

  async reportRoom(roomId: string, reason: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("report_room", { roomId, reason });
  }

  async setAvatar(mimeType: string, bytes: number[]): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("set_avatar", { mimeType, bytes });
  }

  async editMessage(roomId: string, eventId: string, body: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("edit_message", { roomId, eventId, body });
  }

  async redactMessage(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("redact_message", { roomId, eventId });
  }

  async loadMessageSource(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("load_message_source", { roomId, eventId });
  }

  async requestRoomKey(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("request_room_key", { roomId, eventId });
  }

  async forwardMessage(
    roomId: string,
    sourceEventId: string,
    destinationRoomId: string
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("forward_message", {
      roomId,
      sourceEventId,
      destinationRoomId
    });
  }

  async loadLinkPreviews(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("load_link_previews", { roomId, eventId });
  }

  async hideLinkPreview(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("hide_link_preview", { roomId, eventId });
  }

  async leaveRoom(roomId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("leave_room", { roomId });
  }

  async forgetRoom(roomId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("forget_room", { roomId });
  }

  async setRoomTag(
    roomId: string,
    tag: RoomTagKind,
    order: number | null = null
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("set_room_tag", { roomId, tag, order });
  }

  async removeRoomTag(roomId: string, tag: RoomTagKind): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("remove_room_tag", { roomId, tag });
  }

  async pinEvent(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("pin_event", { roomId, eventId });
  }

  async unpinEvent(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("unpin_event", { roomId, eventId });
  }

  async reshareRoomKey(roomId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("reshare_room_key", { roomId });
  }

  async loadRoomSettings(roomId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("load_room_settings", { roomId });
  }

  async resetRoomTimelineCache(roomId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("reset_room_timeline_cache", { roomId });
  }

  async updateRoomSetting(
    roomId: string,
    change: RoomSettingChange
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("update_room_setting", { roomId, change });
  }

  async moderateRoomMember(
    roomId: string,
    targetUserId: string,
    action: RoomModerationAction,
    reason: string | null = null
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("moderate_room_member", {
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
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("update_room_member_role", {
      roomId,
      targetUserId,
      powerLevel
    });
  }

  async openActivity(): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("open_activity");
  }

  async closeActivity(): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("close_activity");
  }

  async setActivityTab(tab: ActivityTab): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("set_activity_tab", { tab });
  }

  async paginateActivity(
    tab: ActivityTab,
    cursor: string | null = null
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("paginate_activity", { tab, cursor });
  }

  async markActivityRead(target: ActivityMarkReadTarget): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("mark_activity_read", { target });
  }

  async setComposerDraft(roomId: string, draft: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("set_composer_draft", { roomId, draft });
  }

  async openThread(roomId: string, rootEventId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("open_thread", { roomId, rootEventId });
  }

  async closeThread(): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("close_thread");
  }

  async openThreadsList(roomId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("open_threads_list", { roomId });
  }

  async closeThreadsList(): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("close_threads_list");
  }

  async openFilesView(
    scope: FilesViewScope,
    filter: AttachmentFilter,
    sort: AttachmentSort
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("open_files_view", { scope, filter, sort });
  }

  async closeFilesView(): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("close_files_view");
  }

  async paginateThreadsList(roomId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("paginate_threads_list", { roomId });
  }

  async setThreadComposerDraft(
    roomId: string,
    rootEventId: string,
    draft: string
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("set_thread_composer_draft", { roomId, rootEventId, draft });
  }

  async sendThreadReply(
    submissionId: string,
    roomId: string,
    rootEventId: string,
    body: string
  ): Promise<SubmissionResponse> {
    return invoke<SubmissionResponse>("send_thread_reply", {
      submissionId,
      roomId,
      rootEventId,
      body
    });
  }

  async selectSearchResult(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("select_search_result", { roomId, eventId });
  }

  async openActivityEvent(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("open_activity_event", { roomId, eventId });
  }

  async openTimelineAtTimestamp(
    roomId: string,
    timestampMs: number
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("open_timeline_at_timestamp", { roomId, timestampMs });
  }

  async closeFocusedContext(): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("close_focused_context");
  }

  async closeSearch(): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("close_search");
  }

  async submitSearch(query: string, scope: SearchScopeKind): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("submit_search", { query, scope });
  }

  async queryDirectory(query: DirectoryQuery): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("query_directory", {
      term: query.term,
      serverName: query.server_name,
      limit: query.limit,
      since: query.since
    });
  }

  async joinDirectoryRoom(
    alias: string,
    viaServer: string | null = null
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("join_directory_room", { alias, viaServer });
  }

  async joinRoom(roomId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("join_room", { roomId });
  }

  async createRoom(request: CreateRoomRequest): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("create_room", { options: request });
  }

  async createSpace(name: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("create_space", { name });
  }

  async setSpaceChild(spaceId: string, childRoomId: string, viaServer: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("set_space_child", { spaceId, childRoomId, viaServer });
  }

  async acceptInvite(roomId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("accept_invite", { roomId });
  }

  async declineInvite(roomId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("decline_invite", { roomId });
  }

  async startDirectMessage(userId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("start_direct_message", { userId });
  }

  async inviteUser(roomId: string, userId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("invite_user", { roomId, userId });
  }

  async openInviteWorkflow(roomId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("open_invite_workflow", { roomId });
  }

  async closeInviteWorkflow(): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("close_invite_workflow");
  }

  async searchInviteTargets(roomId: string, query: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("search_invite_targets", { roomId, query });
  }

  async selectInviteTarget(roomId: string, userId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("select_invite_target", { roomId, userId });
  }

  async removeInviteTarget(userId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("remove_invite_target", { userId });
  }

  async inviteTargets(
    roomId: string,
    userIds: string[],
    scope: InviteScopeSelection
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("invite_targets", { roomId, userIds, scope });
  }

  async setComposerReplyTarget(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("set_composer_reply_target", { roomId, eventId });
  }

  async cancelComposerReply(): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("cancel_composer_reply");
  }

  async sendReply(
    submissionId: string,
    roomId: string,
    inReplyToEventId: string,
    body: string,
    mentions: MentionIntent = { targets: [] }
  ): Promise<SubmissionResponse> {
    return invoke<SubmissionResponse>("send_reply", {
      submissionId,
      roomId,
      inReplyToEventId,
      body,
      mentions
    });
  }

  async startRoomCrawl(roomId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("start_room_crawl", { roomId });
  }

  async stopRoomCrawl(roomId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("stop_room_crawl", { roomId });
  }
}

function isTauriRuntime(): boolean {
  return "__TAURI_INTERNALS__" in window;
}
