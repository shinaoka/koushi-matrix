import type { TimelineKey } from "../domain/coreEvents";
import type {
  ActivityMarkReadTarget,
  ActivityTab,
  AttachmentFilter,
  AttachmentSort,
  CommandAdmission,
  CommandSettlement,
  ComposerDocument,
  ComposerDraftAcceptanceResponse,
  ComposerDraftRevision,
  ComposerKeyEvent,
  ComposerResolvedAction,
  ComposerResolverOptions,
  ComposerSurface,
  ComposerTarget,
  CreateRoomRequest,
  DesktopSnapshot,
  DirectoryQuery,
  DisplayPlatform,
  FilesViewScope,
  InviteScopeSelection,
  MentionSurface,
  NavigationPreferenceUpdate,
  OidcBrowserLaunchResponse,
  PresenceKind,
  RoomListFilter,
  RoomModerationAction,
  RoomNotificationMode,
  RoomSettingChange,
  RoomTagKind,
  SavedSessionInfo,
  SearchScopeKind,
  SecureBackupSetupIntent,
  SessionStatusRefreshCommandTrigger,
  SettingsPatch,
  StageUploadBytesRequestItem,
  StagedUploadCompressionChoice,
  StagedUploadOutputSelection,
  SubmissionResponse,
  ThreadOpenIntent,
  ThreadsListScope,
} from "../domain/types";
import type { DiagnosticLogSnapshot } from "../domain/diagnostics";
import type {
  ComposerDraftLeaseSnapshot,
  ComposerDraftScope
} from "../domain/composerDraftLifecycle";

export type ViewportSyncTrigger =
  | "page_load"
  | "resized"
  | "scale_factor_changed"
  | "density_commit"
  | "browser_resize";
export type ViewportSyncDensity = "compact" | "default" | "comfortable";

export interface ViewportSyncSize {
  width: number;
  height: number;
}

export interface ViewportSyncRect extends ViewportSyncSize {
  top: number;
  left: number;
}

export interface ViewportSyncObservation {
  trigger: ViewportSyncTrigger;
  density: ViewportSyncDensity;
  window: ViewportSyncSize;
  document: ViewportSyncSize;
  visualViewport: {
    present: boolean;
    width: number;
    height: number;
    offsetLeft: number;
    offsetTop: number;
  };
  body: ViewportSyncRect;
  root: ViewportSyncRect;
}

export interface ViewportSyncReceipt {
  generation: number;
  trigger: ViewportSyncTrigger;
  density: ViewportSyncDensity | null;
  nativeSupport: "supported" | "unsupported";
  decision: "in_sync" | "repair_to_parent_bounds" | "unsupported";
  nativeAligned: boolean;
  nativeOriginAligned: boolean;
  nativeSizeAligned: boolean;
  domAligned: boolean;
  domJsAligned: boolean;
  domRootAligned: boolean;
  parent: ViewportSyncRect | null;
  webview: ViewportSyncRect | null;
}

export interface DesktopApi {
  getSnapshot(): Promise<DesktopSnapshot>;
  settlementSnapshot(): Promise<DesktopSnapshot>;
  resyncSnapshot(): Promise<DesktopSnapshot>;
  getDiagnosticSnapshot(): Promise<DiagnosticLogSnapshot>;
  observeViewportSync(observation: ViewportSyncObservation): Promise<ViewportSyncReceipt>;
  discoverLoginMethods(homeserver: string): Promise<CommandSettlement>;
  startOidcLogin(homeserver: string): Promise<OidcBrowserLaunchResponse>;
  completeOidcLogin(homeserver: string, callbackUrl: string): Promise<CommandSettlement>;
  submitLogin(
    homeserver: string,
    username: string,
    password: string,
    deviceDisplayName: string,
    platform: DisplayPlatform
  ): Promise<CommandSettlement>;
  submitSoftLogoutReauth(password: string): Promise<CommandSettlement>;
  listSavedSessions(): Promise<SavedSessionInfo[]>;
  switchAccount(session: SavedSessionInfo): Promise<CommandSettlement>;
  retrySlidingSyncCapability(): Promise<CommandAdmission>;
  changeHomeserver(): Promise<CommandAdmission>;
  logout(): Promise<CommandSettlement>;
  submitRecovery(secret: string): Promise<CommandAdmission>;
  /** Dedicated Secure Backup commands. */
  recoverSecureBackup: (secret: string) => Promise<CommandAdmission>;
  retrySecureBackupInspection: () => Promise<CommandAdmission>;
  startDeviceCleanup(): Promise<CommandAdmission>;
  submitDeviceCleanupUia(flowId: number, password: string): Promise<CommandAdmission>;
  eraseLocalDataAnyway(): Promise<CommandAdmission>;
  restartSync(): Promise<CommandAdmission>;
  updateSettings(patch: SettingsPatch): Promise<CommandAdmission>;
  importLegacySettings(patch: SettingsPatch): Promise<CommandAdmission>;
  updateNavigationPreference(update: NavigationPreferenceUpdate): Promise<CommandAdmission>;
  rebuildSearchIndex(): Promise<CommandAdmission>;
  setRoomUrlPreviewOverride(roomId: string, enabled: boolean): Promise<CommandAdmission>;
  selectRoomListFilter(filter: RoomListFilter): Promise<CommandAdmission>;
  markRoomAsRead(roomId: string, eventId: string): Promise<CommandAdmission>;
  markRoomAsUnread(roomId: string, unread: boolean): Promise<CommandAdmission>;
  forceRotateOutboundSession(roomId: string): Promise<CommandSettlement>;
  setRoomNotificationMode(roomId: string, mode: RoomNotificationMode): Promise<CommandAdmission>;
  refreshCurrentSessionStatus(trigger: SessionStatusRefreshCommandTrigger): Promise<CommandAdmission>;
  submitAccountManagementUia(flowId: number, password: string): Promise<CommandAdmission>;
  loadAccountManagementCapabilities(): Promise<CommandAdmission>;
  changePassword(newPassword: string): Promise<CommandAdmission>;
  deactivateAccount(eraseData: boolean): Promise<CommandAdmission>;
  probeLocalEncryptionHealth(): Promise<CommandAdmission>;
  resetLocalData(): Promise<CommandAdmission>;
  bootstrapCrossSigning(): Promise<CommandAdmission>;
  enableKeyBackup(): Promise<CommandAdmission>;
  exportRoomKeys(destinationPath: string, passphrase: string): Promise<CommandAdmission>;
  importRoomKeys(sourcePath: string, passphrase: string): Promise<CommandAdmission>;
  bootstrapSecureBackup(
    passphrase: string | null,
    recoveryKeyDestinationPath: string | null,
    intent: SecureBackupSetupIntent
  ): Promise<CommandAdmission>;
  changeSecureBackupPassphrase(
    oldSecret: string,
    newPassphrase: string,
    recoveryKeyDestinationPath: string | null
  ): Promise<CommandAdmission>;
  acceptVerification(flowId: number): Promise<CommandAdmission>;
  startOwnUserSas(): Promise<CommandAdmission>;
  retryCurrentDeviceTrustDiscovery(): Promise<CommandAdmission>;
  mismatchSasVerification(flowId: number): Promise<CommandAdmission>;
  startSessionBootstrap(passphrase: string | null, recoveryKeyDestinationPath: string): Promise<CommandAdmission>;
  confirmSessionBootstrapSaved(flowId: number): Promise<CommandAdmission>;
  confirmSasVerification(flowId: number): Promise<CommandAdmission>;
  cancelVerification(flowId: number): Promise<CommandAdmission>;
  resetIdentity(): Promise<CommandAdmission>;
  cancelIdentityReset(flowId: number): Promise<CommandAdmission>;
  submitIdentityResetPassword(flowId: number, password: string): Promise<CommandAdmission>;
  submitIdentityResetOAuth(flowId: number): Promise<CommandAdmission>;
  resolveComposerKeyAction(
    surface: ComposerSurface,
    keyEvent: ComposerKeyEvent,
    options: ComposerResolverOptions
  ): Promise<ComposerResolvedAction>;
  selectSpace(spaceId: string | null): Promise<CommandAdmission>;
  reorderSpaces(spaceIds: string[]): Promise<CommandAdmission>;
  selectRoom(roomId: string): Promise<CommandSettlement>;
  openActivityEvent(roomId: string, eventId: string): Promise<CommandSettlement>;
  openPinnedEvent(roomId: string, eventId: string): Promise<CommandSettlement>;
  selectSearchResult(roomId: string, eventId: string): Promise<CommandSettlement>;
  openTimelineAtTimestamp(roomId: string, timestampMs: number): Promise<CommandSettlement>;
  closeFocusedContext(): Promise<CommandSettlement>;
  closeSearch(): Promise<CommandSettlement>;
  beginComposerDraftRendererGeneration(): Promise<string>;
  acquireComposerDraftLease(
    scope: ComposerDraftScope,
    rendererGeneration: string
  ): Promise<ComposerDraftLeaseSnapshot>;
  releaseComposerDraftLease(leaseId: string, rendererGeneration: string): Promise<void>;
  sendText(
    account: ComposerDraftAccountOwner,
    leaseId: string,
    rendererGeneration: string,
    submissionId: string,
    roomId: string,
    document: ComposerDocument,
    draftRevision?: ComposerDraftRevision
  ): Promise<SubmissionResponse>;
  scheduleSend(
    account: ComposerDraftAccountOwner,
    leaseId: string,
    rendererGeneration: string,
    target: ComposerTarget,
    body: string,
    sendAtMs: number,
    draftRevision: ComposerDraftRevision
  ): Promise<ComposerDraftAcceptanceResponse>;
  stageUploadBytes(
    target: ComposerTarget,
    items: StageUploadBytesRequestItem[]
  ): Promise<CommandSettlement>;
  selectStagedUploadOutput(
    target: ComposerTarget,
    stagedId: string,
    selection: StagedUploadOutputSelection
  ): Promise<CommandSettlement>;
  retryStagedUploadPreparation(target: ComposerTarget, stagedId: string): Promise<CommandSettlement>;
  useOriginalStagedUpload(target: ComposerTarget, stagedId: string): Promise<CommandSettlement>;
  preparedUploadPreview(
    target: ComposerTarget,
    stagedId: string,
    variantId: string
  ): Promise<number[]>;
  sendPreparedUploads(
    account: ComposerDraftAccountOwner,
    leaseId: string,
    rendererGeneration: string,
    target: ComposerTarget,
    draftRevision: ComposerDraftRevision
  ): Promise<ComposerDraftAcceptanceResponse>;
  updateStagedUploadCaption(
    target: ComposerTarget,
    stagedId: string,
    document: ComposerDocument | null
  ): Promise<CommandSettlement>;
  updateStagedUploadCompression(
    target: ComposerTarget,
    stagedId: string,
    compressionChoice: StagedUploadCompressionChoice
  ): Promise<CommandSettlement>;
  clearUploadStaging(target: ComposerTarget): Promise<CommandSettlement>;
  cancelScheduledSend(scheduledId: string): Promise<CommandAdmission>;
  rescheduleScheduledSend(
    scheduledId: string,
    body: string,
    sendAtMs: number
  ): Promise<CommandAdmission>;
  retrySend(roomId: string, transactionId: string): Promise<CommandAdmission>;
  cancelSend(roomId: string, transactionId: string): Promise<CommandAdmission>;
  sendReaction(roomId: string, eventId: string, reactionKey: string): Promise<CommandAdmission>;
  redactReaction(
    roomId: string,
    eventId: string,
    reactionKey: string,
    reactionEventId: string
  ): Promise<CommandAdmission>;
  sendReadReceipt(roomId: string, eventId: string, threadRootEventId?: string | null): Promise<void>;
  setFullyRead(roomId: string, eventId: string): Promise<void>;
  setTyping(roomId: string, isTyping: boolean): Promise<void>;
  setPresence(presence: PresenceKind): Promise<CommandAdmission>;
  setDisplayName(displayName: string | null): Promise<CommandAdmission>;
  setLocalUserAlias(userId: string, alias: string | null): Promise<CommandAdmission>;
  ignoreUser(userId: string): Promise<CommandAdmission>;
  unignoreUser(userId: string): Promise<CommandAdmission>;
  reportUser(userId: string, reason: string): Promise<CommandAdmission>;
  reportContent(roomId: string, eventId: string, reason: string): Promise<CommandAdmission>;
  reportRoom(roomId: string, reason: string): Promise<CommandAdmission>;
  setAvatar(mimeType: string, bytes: number[]): Promise<CommandAdmission>;
  editMessage(
    roomId: string,
    eventId: string,
    document: ComposerDocument
  ): Promise<CommandAdmission>;
  redactMessage(roomId: string, eventId: string): Promise<CommandAdmission>;
  loadMessageSource(roomId: string, eventId: string): Promise<CommandAdmission>;
  requestRoomKey(
    roomId: string,
    eventId: string,
    origin?: "user" | "automatic",
    timelineKey?: TimelineKey
  ): Promise<CommandAdmission>;
  requestLateDecryption(
    roomId: string,
    timelineKey?: TimelineKey
  ): Promise<CommandAdmission>;
  forwardMessage(
    roomId: string,
    sourceEventId: string,
    destinationRoomId: string
  ): Promise<CommandAdmission>;
  loadLinkPreviews(roomId: string, eventId: string): Promise<CommandAdmission>;
  hideLinkPreview(roomId: string, eventId: string): Promise<CommandAdmission>;
  leaveRoom(roomId: string): Promise<CommandAdmission>;
  forgetRoom(roomId: string): Promise<CommandAdmission>;
  setRoomTag(roomId: string, tag: RoomTagKind, order?: number | null): Promise<CommandSettlement>;
  removeRoomTag(roomId: string, tag: RoomTagKind): Promise<CommandSettlement>;
  pinEvent(roomId: string, eventId: string): Promise<CommandSettlement>;
  unpinEvent(roomId: string, eventId: string): Promise<CommandSettlement>;
  openActivity(): Promise<CommandAdmission>;
  closeActivity(): Promise<CommandAdmission>;
  setActivityTab(tab: ActivityTab): Promise<CommandAdmission>;
  paginateActivity(tab: ActivityTab, cursor?: string | null): Promise<CommandAdmission>;
  retryActivityResolution(): Promise<CommandAdmission>;
  markActivityRead(target: ActivityMarkReadTarget): Promise<CommandAdmission>;
  setComposerDraft(
    account: ComposerDraftAccountOwner,
    leaseId: string,
    rendererGeneration: string,
    roomId: string,
    document: ComposerDocument,
    revision: ComposerDraftRevision
  ): Promise<CommandAdmission>;
  openThread(
    roomId: string,
    rootEventId: string,
    intent: ThreadOpenIntent
  ): Promise<CommandAdmission>;
  closeThread(): Promise<CommandAdmission>;
  openThreadsList(scope: ThreadsListScope): Promise<CommandAdmission>;
  closeThreadsList(): Promise<CommandAdmission>;
  paginateThreadsList(scope: ThreadsListScope): Promise<CommandAdmission>;
  openFilesView(scope: FilesViewScope, filter: AttachmentFilter, sort: AttachmentSort): Promise<CommandAdmission>;
  closeFilesView(): Promise<CommandAdmission>;
  setThreadComposerDraft(
    account: ComposerDraftAccountOwner,
    leaseId: string,
    rendererGeneration: string,
    roomId: string,
    rootEventId: string,
    document: ComposerDocument,
    revision: ComposerDraftRevision
  ): Promise<CommandAdmission>;
  sendThreadReply(
    account: ComposerDraftAccountOwner,
    leaseId: string,
    rendererGeneration: string,
    submissionId: string,
    roomId: string,
    rootEventId: string,
    document: ComposerDocument,
    draftRevision?: ComposerDraftRevision
  ): Promise<SubmissionResponse>;
  submitSearch(query: string, scope: SearchScopeKind): Promise<CommandSettlement>;
  queryDirectory(query: DirectoryQuery): Promise<CommandSettlement>;
  joinDirectoryRoom(roomIdOrAlias: string, viaServers?: string[]): Promise<CommandSettlement>;
  previewJoinTarget(roomIdOrAlias: string, viaServers?: string[]): Promise<CommandSettlement>;
  dismissDirectoryPreview(): Promise<CommandAdmission>;
  joinRoom(roomId: string): Promise<CommandSettlement>;
  loadRoomSettings(roomId: string): Promise<CommandSettlement>;
  loadSpaceMembers(spaceId: string, generation: number): Promise<CommandSettlement>;
  inviteUserToSpace(
    spaceId: string,
    userId: string,
    generation: number
  ): Promise<CommandSettlement>;
  cancelSpaceInvite(
    spaceId: string,
    userId: string,
    generation: number
  ): Promise<CommandSettlement>;
  queryMentionCandidates(
    roomId: string,
    surface: MentionSurface,
    query: string
  ): Promise<void>;
  repairRoomTimeline(roomId: string): Promise<CommandAdmission>;
  updateRoomSetting(roomId: string, change: RoomSettingChange): Promise<CommandSettlement>;
  moderateRoomMember(
    roomId: string,
    targetUserId: string,
    action: RoomModerationAction,
    reason?: string | null
  ): Promise<CommandSettlement>;
  updateRoomMemberRole(
    roomId: string,
    targetUserId: string,
    powerLevel: number
  ): Promise<CommandSettlement>;
  updateSpaceMemberRole(
    spaceId: string,
    userId: string,
    generation: number,
    expectedPowerLevelsRevision: string | null,
    expectedPowerLevel: number,
    powerLevel: number,
    confirmed: boolean
  ): Promise<CommandSettlement>;
  createRoom(request: CreateRoomRequest): Promise<CommandSettlement>;
  createSpace(name: string): Promise<CommandSettlement>;
  setSpaceChild(spaceId: string, childRoomId: string, viaServer: string): Promise<CommandAdmission>;
  acceptInvite(roomId: string): Promise<CommandSettlement>;
  declineInvite(roomId: string): Promise<CommandSettlement>;
  startDirectMessage(userId: string): Promise<CommandSettlement>;
  inviteUser(roomId: string, userId: string): Promise<CommandSettlement>;
  openInviteWorkflow(roomId: string): Promise<CommandSettlement>;
  closeInviteWorkflow(): Promise<CommandSettlement>;
  searchInviteTargets(roomId: string, query: string): Promise<CommandSettlement>;
  setInviteScope(roomId: string, scope: InviteScopeSelection): Promise<CommandSettlement>;
  selectInviteTarget(roomId: string, userId: string): Promise<CommandSettlement>;
  removeInviteTarget(userId: string): Promise<CommandSettlement>;
  inviteTargets(
    roomId: string,
    userIds: string[],
    scope: InviteScopeSelection
  ): Promise<CommandSettlement>;
  setComposerReplyTarget(roomId: string, eventId: string): Promise<CommandAdmission>;
  cancelComposerReply(): Promise<CommandAdmission>;
  sendReply(
    account: ComposerDraftAccountOwner,
    leaseId: string,
    rendererGeneration: string,
    submissionId: string,
    roomId: string,
    inReplyToEventId: string,
    document: ComposerDocument,
    draftRevision?: ComposerDraftRevision
  ): Promise<SubmissionResponse>;
  startRoomCrawl(roomId: string): Promise<CommandAdmission>;
  stopRoomCrawl(roomId: string): Promise<CommandAdmission>;
}

export interface ComposerDraftAccountOwner {
  homeserver: string;
  userId: string;
  deviceId: string;
}
