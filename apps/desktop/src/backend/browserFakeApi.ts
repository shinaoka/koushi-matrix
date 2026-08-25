import {
  projectRoomSummaries,
  roomIsInScope,
  textRangeUtf16
} from "../domain/desktopModel";
import {
  COMPOSER_DRAFT_REVISION_ZERO,
  compareComposerDraftRevisions,
  nextComposerDraftRevision
} from "../domain/composerDraftRevision";
import { computeBrowserRoomListProjection } from "./roomListProjection";
import { composeBrowserFakeSidebar, emptySidebar } from "./browser-fake/sidebar";
import {
  compareSpaceMemberEntries,
  emptyBrowserFakeSpaceMembersState,
  createBrowserFakeSpaceMembersState,
  spaceMemberRoleOptionsForPowerLevel
} from "./browser-fake/spaceMembers";
import {
  INVITE_ALREADY_IN_SPACE_MESSAGE,
  buildFakeInviteHistoryPolicy,
  inviteScopeKey,
  defaultInviteWorkflowState,
  buildFakeInviteScopePlan,
  buildFakeInviteTargetQuery,
  fakeRoomHasMember
} from "./browser-fake/inviteWorkflow";
import {
  applyRoomSettingChange,
  defaultRoomManagementState,
  editableRoomPermissionFacts,
  readonlyRoomPermissionFacts,
  roomMemberRoleFromPowerLevel,
  roomModerationAllowed
} from "./browser-fake/roomManagement";
import {
  defaultSettingsState,
  defaultLocaleDisplayProfile,
  defaultTypographyDisplayProfile,
  resolveTypographyDisplayProfile,
  resolveLocaleDisplayProfile,
  applySettingsPatch,
  resolveComposerKeyActionFromSettings
} from "./browser-fake/settings";
import {
  defaultDirectoryState,
  defaultE2eeTrustState,
  defaultDelegatedAuthLinks,
  defaultLiveSignalsState,
  defaultNativeAttentionState,
  defaultCjkTextPolicyState,
  defaultProfileState
} from "./browser-fake/snapshotDefaults";
import { documentFromText, plainBodyFromDocument } from "../domain/composerDocument";
import {
  browserComposerTargetIsActive,
  browserComposerForTarget,
  browserComposerDraftTargetKey,
  browserStagedUploadsForTarget,
  setBrowserStagedUploadsForTarget,
  browserPreparedUploadKey,
  browserPreparedUploadItem
} from "./browser-fake/composerUploads";
import type {
  AvatarThumbnailState,
  TimelineMediaSource
} from "../domain/coreEvents";
import type { DisplayPlatform } from "../domain/types";
import type { LinkPreview, LinkPreviewImage, LinkPreviewState } from "../domain/linkPreview";
import type {
  ActivityMarkReadTarget,
  ActivityRow,
  ActivityStream,
  ActivityTab,
  AttachmentResult,
  CreateRoomRequest,
  ComposerState,
  ComposerDocument,
  DesktopSnapshot,
  ComposerKeyEvent,
  ComposerResolvedAction,
  ComposerResolverOptions,
  ComposerSurface,
  ComposerTarget,
  ComposerDraftRevision,
  ComposerDraftAcceptanceResponse,
  DirectoryQuery,
  RoomListFilter,
  RoomListProjection,
  RoomModerationAction,
  RoomNotificationMode,
  RoomKeyReshareOutcome,
  RoomNotificationSettings,
  InviteScopeSelection,
  InvitePreview,
  RoomPermissionFacts,
  RoomSummary,
  RoomSettingChange,
  RoomSettingsSnapshot,
  RoomTagKind,
  RoomTags,
  SavedSessionInfo,
  SearchResult,
  SearchScopeKind,
  SessionStatusRefreshTrigger,
  SettingsPatch,
  PresenceKind,
  MentionSurface,
  OidcAuthorization,
  SpaceNavigationSelection,
  SpaceSummary,
  SubmissionResponse,
  StagedUploadCompressionChoice,
  StagedUploadOutputSelection,
  StageUploadBytesRequestItem,
  TimelineMessage,
  ThreadOpenIntent,
  ThreadsListItem,
  ThreadsListScope,
  UploadStagingRequestItem,
  AttachmentFilter,
  AttachmentScope,
  AttachmentSort,
  FilesViewScope,
  UserProfile,
  SpaceMemberEntry,
  SpaceMemberRoleFailureKind,
  SpaceMembersState,
  SecureBackupGateState,
  EncryptionDebugOperationOutcome
} from "../domain/types";
import {
  DEFAULT_SLIDING_SYNC_DIAGNOSTICS,
  type DiagnosticLogSnapshot
} from "../domain/diagnostics";
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
  getDiagnosticSnapshot(): Promise<DiagnosticLogSnapshot>;
  observeViewportSync(observation: ViewportSyncObservation): Promise<ViewportSyncReceipt>;
  discoverLoginMethods(homeserver: string): Promise<DesktopSnapshot>;
  startOidcLogin(homeserver: string): Promise<OidcAuthorization>;
  completeOidcLogin(homeserver: string, callbackUrl: string): Promise<DesktopSnapshot>;
  submitLogin(
    homeserver: string,
    username: string,
    password: string,
    deviceDisplayName: string,
    platform: DisplayPlatform
  ): Promise<DesktopSnapshot>;
  submitSoftLogoutReauth(password: string): Promise<DesktopSnapshot>;
  listSavedSessions(): Promise<SavedSessionInfo[]>;
  switchAccount(session: SavedSessionInfo): Promise<DesktopSnapshot>;
  retrySlidingSyncCapability(): Promise<DesktopSnapshot>;
  changeHomeserver(): Promise<DesktopSnapshot>;
  logout(): Promise<DesktopSnapshot>;
  submitRecovery(secret: string): Promise<DesktopSnapshot>;
  /** Dedicated Secure Backup commands. */
  recoverSecureBackup: (secret: string) => Promise<DesktopSnapshot>;
  setupSecureBackup: (
    passphrase: string | null,
    recoveryKeyDestinationPath: string | null
  ) => Promise<DesktopSnapshot>;
  reenableSecureBackup: (
    passphrase: string | null,
    recoveryKeyDestinationPath: string | null
  ) => Promise<DesktopSnapshot>;
  retrySecureBackupInspection: () => Promise<DesktopSnapshot>;
  startDeviceCleanup(): Promise<DesktopSnapshot>;
  submitDeviceCleanupUia(flowId: number, password: string): Promise<DesktopSnapshot>;
  eraseLocalDataAnyway(): Promise<DesktopSnapshot>;
  restartSync(): Promise<DesktopSnapshot>;
  updateSettings(patch: SettingsPatch): Promise<DesktopSnapshot>;
  rebuildSearchIndex(): Promise<DesktopSnapshot>;
  setRoomUrlPreviewOverride(roomId: string, enabled: boolean): Promise<DesktopSnapshot>;
  selectRoomListFilter(filter: RoomListFilter): Promise<DesktopSnapshot>;
  markRoomAsRead(roomId: string, eventId: string): Promise<DesktopSnapshot>;
  markRoomAsUnread(roomId: string, unread: boolean): Promise<DesktopSnapshot>;
  setRoomNotificationMode(roomId: string, mode: RoomNotificationMode): Promise<DesktopSnapshot>;
  queryDevices(): Promise<DesktopSnapshot>;
  refreshCurrentSessionStatus(trigger: SessionStatusRefreshTrigger): Promise<DesktopSnapshot>;
  renameDevice(deviceOrdinal: number, displayName: string): Promise<DesktopSnapshot>;
  deleteDevices(deviceOrdinals: number[]): Promise<DesktopSnapshot>;
  submitAccountManagementUia(flowId: number, password: string): Promise<DesktopSnapshot>;
  loadAccountManagementCapabilities(): Promise<DesktopSnapshot>;
  changePassword(newPassword: string): Promise<DesktopSnapshot>;
  deactivateAccount(eraseData: boolean): Promise<DesktopSnapshot>;
  probeLocalEncryptionHealth(): Promise<DesktopSnapshot>;
  resetLocalData(): Promise<DesktopSnapshot>;
  bootstrapCrossSigning(): Promise<DesktopSnapshot>;
  enableKeyBackup(): Promise<DesktopSnapshot>;
  exportRoomKeys(destinationPath: string, passphrase: string): Promise<DesktopSnapshot>;
  importRoomKeys(sourcePath: string, passphrase: string): Promise<DesktopSnapshot>;
  bootstrapSecureBackup(
    passphrase: string | null,
    recoveryKeyDestinationPath: string | null
  ): Promise<DesktopSnapshot>;
  changeSecureBackupPassphrase(
    oldSecret: string,
    newPassphrase: string,
    recoveryKeyDestinationPath: string | null
  ): Promise<DesktopSnapshot>;
  acceptVerification(flowId: number): Promise<DesktopSnapshot>;
  startOwnUserSas(): Promise<DesktopSnapshot>;
  retryCurrentDeviceTrustDiscovery(): Promise<DesktopSnapshot>;
  mismatchSasVerification(flowId: number): Promise<DesktopSnapshot>;
  startSessionBootstrap(passphrase: string | null, recoveryKeyDestinationPath: string): Promise<DesktopSnapshot>;
  confirmSessionBootstrapSaved(flowId: number): Promise<DesktopSnapshot>;
  confirmSasVerification(flowId: number): Promise<DesktopSnapshot>;
  cancelVerification(flowId: number): Promise<DesktopSnapshot>;
  resetIdentity(): Promise<DesktopSnapshot>;
  cancelIdentityReset(flowId: number): Promise<DesktopSnapshot>;
  submitIdentityResetPassword(flowId: number, password: string): Promise<DesktopSnapshot>;
  submitIdentityResetOAuth(flowId: number): Promise<DesktopSnapshot>;
  resolveComposerKeyAction(
    surface: ComposerSurface,
    keyEvent: ComposerKeyEvent,
    options: ComposerResolverOptions
  ): Promise<ComposerResolvedAction>;
  selectSpace(spaceId: string | null): Promise<DesktopSnapshot>;
  reorderSpaces(spaceIds: string[]): Promise<DesktopSnapshot>;
  selectRoom(roomId: string): Promise<DesktopSnapshot>;
  openActivityEvent(roomId: string, eventId: string): Promise<DesktopSnapshot>;
  openPinnedEvent(roomId: string, eventId: string): Promise<DesktopSnapshot>;
  selectSearchResult(roomId: string, eventId: string): Promise<DesktopSnapshot>;
  acknowledgeTimelineProjection(
    projectionRequestId: import("../domain/coreEvents").RequestId,
    key: import("../domain/coreEvents").TimelineKey,
    generation: number,
    itemCount: number,
    targetPresent: boolean
  ): Promise<void>;
  acknowledgeTimelineBatchRendered(
    key: import("../domain/coreEvents").TimelineKey,
    actorGeneration: number,
    timelineGeneration: number,
    repairGeneration: number,
    batchId: number
  ): Promise<void>;
  openTimelineAtTimestamp(roomId: string, timestampMs: number): Promise<DesktopSnapshot>;
  closeFocusedContext(): Promise<DesktopSnapshot>;
  closeSearch(): Promise<DesktopSnapshot>;
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
  stageUploads(roomId: string, items: UploadStagingRequestItem[]): Promise<DesktopSnapshot>;
  stageUploadBytes(
    target: ComposerTarget,
    items: StageUploadBytesRequestItem[]
  ): Promise<DesktopSnapshot>;
  selectStagedUploadOutput(
    target: ComposerTarget,
    stagedId: string,
    selection: StagedUploadOutputSelection
  ): Promise<DesktopSnapshot>;
  retryStagedUploadPreparation(target: ComposerTarget, stagedId: string): Promise<DesktopSnapshot>;
  useOriginalStagedUpload(target: ComposerTarget, stagedId: string): Promise<DesktopSnapshot>;
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
    caption: string | null
  ): Promise<DesktopSnapshot>;
  updateStagedUploadCompression(
    stagedId: string,
    compressionChoice: StagedUploadCompressionChoice
  ): Promise<DesktopSnapshot>;
  clearUploadStaging(target: ComposerTarget): Promise<DesktopSnapshot>;
  cancelScheduledSend(scheduledId: string): Promise<DesktopSnapshot>;
  rescheduleScheduledSend(
    scheduledId: string,
    body: string,
    sendAtMs: number
  ): Promise<DesktopSnapshot>;
  retrySend(roomId: string, transactionId: string): Promise<DesktopSnapshot>;
  cancelSend(roomId: string, transactionId: string): Promise<DesktopSnapshot>;
  sendReaction(roomId: string, eventId: string, reactionKey: string): Promise<DesktopSnapshot>;
  redactReaction(
    roomId: string,
    eventId: string,
    reactionKey: string,
    reactionEventId: string
  ): Promise<DesktopSnapshot>;
  sendReadReceipt(roomId: string, eventId: string, threadRootEventId?: string | null): Promise<void>;
  setFullyRead(roomId: string, eventId: string): Promise<void>;
  setTyping(roomId: string, isTyping: boolean): Promise<void>;
  setPresence(presence: PresenceKind): Promise<DesktopSnapshot>;
  setDisplayName(displayName: string | null): Promise<DesktopSnapshot>;
  setLocalUserAlias(userId: string, alias: string | null): Promise<DesktopSnapshot>;
  ignoreUser(userId: string): Promise<DesktopSnapshot>;
  unignoreUser(userId: string): Promise<DesktopSnapshot>;
  reportUser(userId: string, reason: string): Promise<DesktopSnapshot>;
  reportContent(roomId: string, eventId: string, reason: string): Promise<DesktopSnapshot>;
  reportRoom(roomId: string, reason: string): Promise<DesktopSnapshot>;
  setAvatar(mimeType: string, bytes: number[]): Promise<DesktopSnapshot>;
  editMessage(
    roomId: string,
    eventId: string,
    document: ComposerDocument
  ): Promise<DesktopSnapshot>;
  redactMessage(roomId: string, eventId: string): Promise<DesktopSnapshot>;
  loadMessageSource(roomId: string, eventId: string): Promise<DesktopSnapshot>;
  requestRoomKey(
    roomId: string,
    eventId: string,
    origin?: "user" | "automatic",
    timelineKey?: import("../domain/coreEvents").TimelineKey
  ): Promise<DesktopSnapshot>;
  requestLateDecryption(
    roomId: string,
    timelineKey?: import("../domain/coreEvents").TimelineKey
  ): Promise<DesktopSnapshot>;
  forwardMessage(
    roomId: string,
    sourceEventId: string,
    destinationRoomId: string
  ): Promise<DesktopSnapshot>;
  loadLinkPreviews(roomId: string, eventId: string): Promise<DesktopSnapshot>;
  hideLinkPreview(roomId: string, eventId: string): Promise<DesktopSnapshot>;
  leaveRoom(roomId: string): Promise<DesktopSnapshot>;
  forgetRoom(roomId: string): Promise<DesktopSnapshot>;
  setRoomTag(roomId: string, tag: RoomTagKind, order?: number | null): Promise<DesktopSnapshot>;
  removeRoomTag(roomId: string, tag: RoomTagKind): Promise<DesktopSnapshot>;
  pinEvent(roomId: string, eventId: string): Promise<DesktopSnapshot>;
  unpinEvent(roomId: string, eventId: string): Promise<DesktopSnapshot>;
  reshareRoomKey(roomId: string): Promise<RoomKeyReshareOutcome>;
  forceNewOutboundSession(roomId: string): Promise<EncryptionDebugOperationOutcome>;
  shareIndex0RoomKey(roomId: string): Promise<EncryptionDebugOperationOutcome>;
  resendIndex0RoomKey(roomId: string): Promise<EncryptionDebugOperationOutcome>;
  openActivity(): Promise<DesktopSnapshot>;
  closeActivity(): Promise<DesktopSnapshot>;
  setActivityTab(tab: ActivityTab): Promise<DesktopSnapshot>;
  paginateActivity(tab: ActivityTab, cursor?: string | null): Promise<DesktopSnapshot>;
  retryActivityResolution(): Promise<DesktopSnapshot>;
  markActivityRead(target: ActivityMarkReadTarget): Promise<DesktopSnapshot>;
  setComposerDraft(
    account: ComposerDraftAccountOwner,
    leaseId: string,
    rendererGeneration: string,
    roomId: string,
    document: ComposerDocument,
    revision: ComposerDraftRevision
  ): Promise<DesktopSnapshot>;
  openThread(
    roomId: string,
    rootEventId: string,
    intent: ThreadOpenIntent
  ): Promise<DesktopSnapshot>;
  closeThread(): Promise<DesktopSnapshot>;
  openThreadsList(scope: ThreadsListScope): Promise<DesktopSnapshot>;
  closeThreadsList(): Promise<DesktopSnapshot>;
  paginateThreadsList(scope: ThreadsListScope): Promise<DesktopSnapshot>;
  openFilesView(scope: FilesViewScope, filter: AttachmentFilter, sort: AttachmentSort): Promise<DesktopSnapshot>;
  closeFilesView(): Promise<DesktopSnapshot>;
  setThreadComposerDraft(
    account: ComposerDraftAccountOwner,
    leaseId: string,
    rendererGeneration: string,
    roomId: string,
    rootEventId: string,
    document: ComposerDocument,
    revision: ComposerDraftRevision
  ): Promise<DesktopSnapshot>;
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
  submitSearch(query: string, scope: SearchScopeKind): Promise<DesktopSnapshot>;
  queryDirectory(query: DirectoryQuery): Promise<DesktopSnapshot>;
  joinDirectoryRoom(roomIdOrAlias: string, viaServers?: string[]): Promise<DesktopSnapshot>;
  previewJoinTarget(roomIdOrAlias: string, viaServers?: string[]): Promise<DesktopSnapshot>;
  dismissDirectoryPreview(): Promise<DesktopSnapshot>;
  joinRoom(roomId: string): Promise<DesktopSnapshot>;
  loadRoomSettings(roomId: string): Promise<DesktopSnapshot>;
  loadSpaceMembers(spaceId: string, generation: number): Promise<DesktopSnapshot>;
  inviteUserToSpace(
    spaceId: string,
    userId: string,
    generation: number
  ): Promise<DesktopSnapshot>;
  cancelSpaceInvite(
    spaceId: string,
    userId: string,
    generation: number
  ): Promise<DesktopSnapshot>;
  queryMentionCandidates(
    roomId: string,
    surface: MentionSurface,
    query: string
  ): Promise<void>;
  repairRoomTimeline(roomId: string): Promise<DesktopSnapshot>;
  updateRoomSetting(roomId: string, change: RoomSettingChange): Promise<DesktopSnapshot>;
  moderateRoomMember(
    roomId: string,
    targetUserId: string,
    action: RoomModerationAction,
    reason?: string | null
  ): Promise<DesktopSnapshot>;
  updateRoomMemberRole(
    roomId: string,
    targetUserId: string,
    powerLevel: number
  ): Promise<DesktopSnapshot>;
  updateSpaceMemberRole(
    spaceId: string,
    userId: string,
    generation: number,
    expectedPowerLevelsRevision: string | null,
    expectedPowerLevel: number,
    powerLevel: number,
    confirmed: boolean
  ): Promise<DesktopSnapshot>;
  createRoom(request: CreateRoomRequest): Promise<DesktopSnapshot>;
  createSpace(name: string): Promise<DesktopSnapshot>;
  setSpaceChild(spaceId: string, childRoomId: string, viaServer: string): Promise<DesktopSnapshot>;
  acceptInvite(roomId: string): Promise<DesktopSnapshot>;
  declineInvite(roomId: string): Promise<DesktopSnapshot>;
  startDirectMessage(userId: string): Promise<DesktopSnapshot>;
  inviteUser(roomId: string, userId: string): Promise<DesktopSnapshot>;
  openInviteWorkflow(roomId: string): Promise<DesktopSnapshot>;
  closeInviteWorkflow(): Promise<DesktopSnapshot>;
  searchInviteTargets(roomId: string, query: string): Promise<DesktopSnapshot>;
  setInviteScope(roomId: string, scope: InviteScopeSelection): Promise<DesktopSnapshot>;
  selectInviteTarget(roomId: string, userId: string): Promise<DesktopSnapshot>;
  removeInviteTarget(userId: string): Promise<DesktopSnapshot>;
  inviteTargets(
    roomId: string,
    userIds: string[],
    scope: InviteScopeSelection
  ): Promise<DesktopSnapshot>;
  setComposerReplyTarget(roomId: string, eventId: string): Promise<DesktopSnapshot>;
  cancelComposerReply(): Promise<DesktopSnapshot>;
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
  setRoomListProjection(projection: RoomListProjection): void;
  startRoomCrawl(roomId: string): Promise<DesktopSnapshot>;
  stopRoomCrawl(roomId: string): Promise<DesktopSnapshot>;
}

export interface ComposerDraftAccountOwner {
  homeserver: string;
  userId: string;
  deviceId: string;
}

export interface BrowserFakeApiOptions {
  restoreSession?: boolean;
  session?: "ready" | "signedOut" | "needsRecovery" | "locked";
  secureBackupGate?: SecureBackupGateState;
  roomPermissions?: Readonly<Record<string, RoomPermissionFacts>>;
  spaceMemberInviteOutcome?: "pending" | "success" | "failure";
  spaceMemberInviteCancellationOutcome?:
    | "pending"
    | "success"
    | "failure"
    | "notInvited";
  spaceMemberInviteCancellationOutcomes?: Array<
    NonNullable<BrowserFakeApiOptions["spaceMemberInviteCancellationOutcome"]>
  >;
  spaceMemberRoleUpdateOutcome?: BrowserFakeSpaceMemberRoleUpdateOutcome;
  spaceMemberRoleUpdateOutcomes?: BrowserFakeSpaceMemberRoleUpdateOutcome[];
}

export type BrowserFakeSpaceMemberRoleUpdateOutcome =
  | "success"
  | SpaceMemberRoleFailureKind
  | "pending";

export type BrowserFakeApiContract = DesktopApi &
  Required<
    Pick<
      DesktopApi,
      | "recoverSecureBackup"
      | "setupSecureBackup"
      | "reenableSecureBackup"
      | "retrySecureBackupInspection"
    >
  >;

const MAX_PREPARATION_BATCH_SIZE = 16;
const MAX_PREPARATION_BATCH_BYTES = 128 * 1024 * 1024;
const ATTACHMENT_BATCH_ERROR = "attachment batch is empty or exceeds the supported limit";

export function createBrowserFakeApi(options: BrowserFakeApiOptions = {}): BrowserFakeApiContract {
  return new BrowserFakeApi(options);
}

class BrowserFakeApi implements DesktopApi {
  private snapshot: DesktopSnapshot;
  private readonly roomPermissions: Readonly<Record<string, RoomPermissionFacts>>;
  private readonly spaceMemberInviteOutcome: NonNullable<
    BrowserFakeApiOptions["spaceMemberInviteOutcome"]
  >;
  private readonly spaceMemberInviteCancellationOutcome: NonNullable<
    BrowserFakeApiOptions["spaceMemberInviteCancellationOutcome"]
  >;
  private readonly spaceMemberInviteCancellationOutcomes: Array<
    NonNullable<BrowserFakeApiOptions["spaceMemberInviteCancellationOutcome"]>
  >;
  private readonly spaceMemberRoleUpdateOutcome: BrowserFakeSpaceMemberRoleUpdateOutcome;
  private readonly spaceMemberRoleUpdateOutcomes: BrowserFakeSpaceMemberRoleUpdateOutcome[];
  private requestSequence = 1_000;
  private composerRendererGeneration = 0n;
  private nextComposerLeaseId = 0n;
  private composerLeases = new Map<
    string,
    { rendererGeneration: string; scope: ComposerDraftScope }
  >();
  private composerDrafts = new Map<string, ComposerDocument>();
  private composerDraftRevisions = new Map<string, ComposerDraftRevision>();
  private threadComposerDrafts = new Map<string, ComposerDocument>();
  private threadComposerDraftRevisions = new Map<string, ComposerDraftRevision>();
  private preparedUploadBytes = new Map<string, number[]>();
  private submissionLedger = new Map<string, string>();
  private viewportSyncGeneration = 0;

  private clearPreparedUploadBytes(target: ComposerTarget): void {
    const prefix =
      target.kind === "main"
        ? `main:${target.room_id}::`
        : `thread:${target.room_id}:${target.root_event_id}:`;
    for (const key of this.preparedUploadBytes.keys()) {
      if (key.startsWith(prefix)) this.preparedUploadBytes.delete(key);
    }
  }

  private clearPreparedThreadUploadBytesForRoom(roomId: string): void {
    const prefix = `thread:${roomId}:`;
    for (const key of this.preparedUploadBytes.keys()) {
      if (key.startsWith(prefix)) this.preparedUploadBytes.delete(key);
    }
  }

  private requireComposerLease(
    account: ComposerDraftAccountOwner,
    target: ComposerTarget,
    leaseId: string,
    rendererGeneration: string
  ): void {
    const lease = this.composerLeases.get(leaseId);
    if (
      rendererGeneration !== this.composerRendererGeneration.toString() ||
      lease?.rendererGeneration !== rendererGeneration ||
      lease.scope.account.homeserver !== account.homeserver ||
      lease.scope.account.user_id !== account.userId ||
      lease.scope.account.device_id !== account.deviceId ||
      browserComposerDraftTargetKey(lease.scope.target) !== browserComposerDraftTargetKey(target)
    ) {
      throw new Error("composer draft lease mismatch");
    }
  }

  private acceptComposerDraftTarget(
    target: ComposerTarget,
    submittedRevision: ComposerDraftRevision
  ): ComposerDraftRevision {
    const key = browserComposerDraftTargetKey(target);
    const revisions =
      target.kind === "main"
        ? this.composerDraftRevisions
        : this.threadComposerDraftRevisions;
    const currentRevision = revisions.get(key) ?? COMPOSER_DRAFT_REVISION_ZERO;
    const preserveNewerDraft =
      compareComposerDraftRevisions(currentRevision, submittedRevision) > 0;
    const revision = nextComposerDraftRevision(currentRevision, submittedRevision);
    revisions.set(key, revision);
    if (!preserveNewerDraft) {
      if (target.kind === "main") {
        this.composerDrafts.delete(target.room_id);
      } else {
        this.threadComposerDrafts.delete(key);
      }
    }
    const composer = browserComposerForTarget(this.snapshot, target);
    if (composer) {
      composer.document =
        (target.kind === "main"
          ? this.composerDrafts.get(target.room_id)
          : this.threadComposerDrafts.get(key)) ?? documentFromText("");
      composer.draft = plainBodyFromDocument(composer.document);
      composer.draft_revision = revision;
      if (!preserveNewerDraft) {
        composer.last_accepted_clear_revision = revision;
      }
    }
    return revision;
  }

  private preflightComposerDraftAcceptance(
    target: ComposerTarget,
    submittedRevision: ComposerDraftRevision
  ): void {
    const key = browserComposerDraftTargetKey(target);
    const revisions =
      target.kind === "main"
        ? this.composerDraftRevisions
        : this.threadComposerDraftRevisions;
    nextComposerDraftRevision(
      revisions.get(key) ?? COMPOSER_DRAFT_REVISION_ZERO,
      submittedRevision
    );
  }

  private replaySubmission(submissionId: string): SubmissionResponse | null {
    const admitted = this.submissionLedger.get(submissionId);
    if (!admitted) return null;
    return {
      outcome: "accepted",
      submissionId,
      transactionId: admitted,
      snapshot: clone(this.snapshot)
    };
  }

  private acceptSubmission(submissionId: string, composer: ComposerState): string {
    const transactionId = `$browser-${submissionId}`;
    while (this.submissionLedger.size >= 128) {
      const oldest = this.submissionLedger.keys().next().value;
      if (oldest === undefined) break;
      this.submissionLedger.delete(oldest);
    }
    this.submissionLedger.set(submissionId, transactionId);
    this.rememberSubmissionRegistryId(
      this.snapshot.state.ui.timeline.submission_registry.accepted_submission_ids,
      submissionId
    );
    this.rememberSubmissionRegistryId(composer.accepted_submission_ids, submissionId);
    composer.pending_submission_id = submissionId;
    composer.pending_transaction_id = transactionId;
    return transactionId;
  }

  private terminalSubmission(composer: ComposerState): void {
    const submissionId = composer.pending_submission_id;
    composer.pending_submission_id = null;
    composer.pending_transaction_id = null;
    if (submissionId) {
      const active = this.snapshot.state.ui.timeline.submission_registry.accepted_submission_ids;
      this.snapshot.state.ui.timeline.submission_registry.accepted_submission_ids = active.filter(
        (id) => id !== submissionId
      );
      this.rememberSubmissionRegistryId(
        this.snapshot.state.ui.timeline.submission_registry.settled_submission_ids,
        submissionId
      );
    }
  }

  private rememberSubmissionRegistryId(ids: string[], id: string): void {
    if (ids.includes(id)) return;
    while (ids.length >= 128) ids.shift();
    ids.push(id);
  }

  constructor(options: BrowserFakeApiOptions) {
    this.roomPermissions = Object.fromEntries(
      Object.entries(options.roomPermissions ?? {}).map(([roomId, permissions]) => [
        roomId,
        { ...permissions }
      ])
    );
    this.spaceMemberInviteOutcome = options.spaceMemberInviteOutcome ?? "success";
    this.spaceMemberInviteCancellationOutcome =
      options.spaceMemberInviteCancellationOutcome ?? "success";
    this.spaceMemberInviteCancellationOutcomes = [
      ...(options.spaceMemberInviteCancellationOutcomes ?? [])
    ];
    this.spaceMemberRoleUpdateOutcome = options.spaceMemberRoleUpdateOutcome ?? "success";
    this.spaceMemberRoleUpdateOutcomes = [...(options.spaceMemberRoleUpdateOutcomes ?? [])];
    this.snapshot = createInitialSnapshot(initialSession(options), options.secureBackupGate);
    const spaceMembers = this.snapshot.state.domain.space_members;
    const spacePermissions = this.roomPermissions[spaceMembers.selected_space_id ?? ""];
    if (spacePermissions && !spacePermissions.can_edit_roles) {
      this.snapshot.state.domain.space_members = {
        ...spaceMembers,
        can_edit_roles: false,
        space_joined: spaceMembers.space_joined.map((entry) => ({ ...entry, role_options: [] }))
      };
    }
  }

  async getSnapshot(): Promise<DesktopSnapshot> {
    this.refreshRoomPresentation();
    return clone(this.snapshot);
  }

  async beginComposerDraftRendererGeneration(): Promise<string> {
    this.composerRendererGeneration += 1n;
    this.composerLeases.clear();
    return this.composerRendererGeneration.toString();
  }

  async acquireComposerDraftLease(
    scope: ComposerDraftScope,
    rendererGeneration: string
  ): Promise<ComposerDraftLeaseSnapshot> {
    if (
      rendererGeneration !== this.composerRendererGeneration.toString() ||
      !scope.account.homeserver ||
      !scope.account.user_id ||
      !scope.account.device_id ||
      !browserComposerAccountMatches(this.snapshot.state.domain.session, {
        homeserver: scope.account.homeserver,
        userId: scope.account.user_id,
        deviceId: scope.account.device_id
      }) ||
      !browserComposerForTarget(this.snapshot, scope.target)
    ) {
      throw new Error("composer draft lease mismatch");
    }
    this.nextComposerLeaseId += 1n;
    const leaseId = this.nextComposerLeaseId.toString();
    this.composerLeases.set(leaseId, {
      rendererGeneration,
      scope: clone(scope)
    });
    const composer = browserComposerForTarget(this.snapshot, scope.target)!;
    return {
      rendererGeneration,
      leaseId,
      revision: composer.draft_revision,
      lastAcceptedClearRevision: composer.last_accepted_clear_revision,
      hasAuthoritativeContent: composer.draft.length > 0
    };
  }

  async releaseComposerDraftLease(
    leaseId: string,
    rendererGeneration: string
  ): Promise<void> {
    const lease = this.composerLeases.get(leaseId);
    if (lease?.rendererGeneration !== rendererGeneration) {
      throw new Error("composer draft lease mismatch");
    }
    this.composerLeases.delete(leaseId);
  }

  async getDiagnosticSnapshot(): Promise<DiagnosticLogSnapshot> {
    return {
      entries: [],
      droppedEntries: 0,
      slidingSync: { ...DEFAULT_SLIDING_SYNC_DIAGNOSTICS }
    };
  }

  async observeViewportSync(
    observation: ViewportSyncObservation
  ): Promise<ViewportSyncReceipt> {
    this.viewportSyncGeneration += 1;
    return {
      generation: this.viewportSyncGeneration,
      trigger: observation.trigger,
      density: observation.density,
      nativeSupport: "unsupported",
      decision: "unsupported",
      nativeAligned: false,
      nativeOriginAligned: false,
      nativeSizeAligned: false,
      domAligned: true,
      domJsAligned: true,
      domRootAligned: true,
      parent: null,
      webview: null
    };
  }

  async discoverLoginMethods(homeserver: string): Promise<DesktopSnapshot> {
    const normalizedHomeserver = normalizeHomeserver(homeserver);
    this.snapshot.state.domain.auth = {
      kind: "ready",
      homeserver: normalizedHomeserver,
      flows: [
        {
          kind: "password",
          delegated_oidc_compatibility: false,
          display_name: null
        },
        {
          kind: "sso",
          delegated_oidc_compatibility: true,
          display_name: null
        }
      ],
      delegated: defaultDelegatedAuthLinks()
    };

    return this.getSnapshot();
  }

  async startOidcLogin(_homeserver: string): Promise<OidcAuthorization> {
    return {
      authorization_url: "https://auth.example.test/authorize",
      state: "browser-fake-state"
    };
  }

  async completeOidcLogin(
    _homeserver: string,
    _callbackUrl: string
  ): Promise<DesktopSnapshot> {
    this.clearSessionViews();
    this.snapshot = createReadySnapshot(savedSessions[0]);
    return clone(this.snapshot);
  }

  async submitLogin(
    homeserver: string,
    username: string,
    password: string,
    deviceDisplayName: string,
    platform: DisplayPlatform
  ): Promise<DesktopSnapshot> {
    const attempt_id = this.nextRequestId();
    this.snapshot.state.domain.session = {
      kind: "authenticating",
      homeserver: normalizeHomeserver(homeserver),
      attempt_id: { connection_id: 1, sequence: attempt_id }
    };
    this.snapshot.state.domain.session_lock_reason = null;
    this.snapshot.state.ui.errors = this.snapshot.state.ui.errors.filter(
      (error) => error.code !== "login_failed"
    );
    this.clearSessionViews();
    void username;
    void password;
    void deviceDisplayName;
    void platform;

    this.snapshot.state.domain.session = { kind: "signedOut" };
    this.snapshot.state.ui.errors.push({
      code: "login_failed",
      message: "real Matrix login is not wired in this pre-login foundation",
      recoverable: true
    });

    return this.getSnapshot();
  }

  async submitSoftLogoutReauth(password: string): Promise<DesktopSnapshot> {
    if (this.snapshot.state.domain.session.kind !== "locked") {
      return this.getSnapshot();
    }

    const requestId = this.nextRequestId();
    const session = this.snapshot.state.domain.session;
    this.snapshot.state.domain.soft_logout_reauth = {
      kind: "authenticating",
      request_id: requestId
    };
    void password;

    this.snapshot = createReadySnapshot({
      homeserver: session.homeserver ?? savedSessions[0].homeserver,
      user_id: session.user_id ?? savedSessions[0].user_id,
      device_id: session.device_id ?? savedSessions[0].device_id
    });
    this.snapshot.state.domain.soft_logout_reauth = {
      kind: "succeeded",
      request_id: requestId
    };
    return this.getSnapshot();
  }

  async listSavedSessions(): Promise<SavedSessionInfo[]> {
    return clone(savedSessions);
  }

  async switchAccount(session: SavedSessionInfo): Promise<DesktopSnapshot> {
    const knownSession =
      savedSessions.find(
        (candidate) =>
          candidate.homeserver === session.homeserver &&
          candidate.user_id === session.user_id &&
          candidate.device_id === session.device_id
      ) ?? session;
    this.snapshot.state.domain.session = {
      ...knownSession,
      kind: "switchingAccount"
    };
    this.snapshot.state.domain.sync = "stopped";
    this.clearSessionViews();
    this.snapshot = createReadySnapshot(knownSession);
    return this.getSnapshot();
  }

  async retrySlidingSyncCapability(): Promise<DesktopSnapshot> {
    return this.getSnapshot();
  }

  async changeHomeserver(): Promise<DesktopSnapshot> {
    this.snapshot.state.domain.session = { kind: "signedOut" };
    this.snapshot.state.domain.session_lock_reason = null;
    this.clearSessionViews();
    return this.getSnapshot();
  }

  async logout(): Promise<DesktopSnapshot> {
    this.snapshot.state.domain.session = { kind: "signedOut" };
    this.snapshot.state.domain.session_lock_reason = null;
    this.clearSessionViews();
    return this.getSnapshot();
  }

  async submitRecovery(secret: string): Promise<DesktopSnapshot> {
    if (
      this.snapshot.state.domain.session.kind !== "awaitingVerification" &&
      this.snapshot.state.domain.session.kind !== "verifying" &&
      this.snapshot.state.domain.session.kind !== "needsRecovery"
    ) {
      return this.getSnapshot();
    }

    const session = this.snapshot.state.domain.session;
    const gate =
      "gate" in session && session.gate
        ? { ...session.gate, failureKind: null }
        : {
            methods: ["recoveryKey" as const],
            account_kind: "newIdentity" as const,
            failureKind: null
          };
    this.snapshot.state.domain.session = {
      kind: "verifying",
      homeserver: session.homeserver ?? savedSessions[0].homeserver,
      user_id: session.user_id ?? savedSessions[0].user_id,
      device_id: session.device_id ?? savedSessions[0].device_id,
      gate,
      method: "recoveryKey",
      flow_id: session.flow_id ?? this.nextRequestId(),
      sas_emojis: []
    };
    this.snapshot.state.ui.errors = this.snapshot.state.ui.errors.filter(
      (error) => error.code !== "e2ee_recovery_failed"
    );
    void secret;

    return this.getSnapshot();
  }

  async recoverSecureBackup(secret: string): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    void secret;
    this.snapshot.state.domain.secure_backup_gate = { kind: "ready" };
    return this.getSnapshot();
  }

  async setupSecureBackup(
    passphrase: string | null,
    recoveryKeyDestinationPath: string | null
  ): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    void passphrase;
    void recoveryKeyDestinationPath;
    this.snapshot.state.domain.secure_backup_gate = { kind: "ready" };
    return this.getSnapshot();
  }

  async reenableSecureBackup(
    passphrase: string | null,
    recoveryKeyDestinationPath: string | null
  ): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    void passphrase;
    void recoveryKeyDestinationPath;
    this.snapshot.state.domain.secure_backup_gate = { kind: "ready" };
    return this.getSnapshot();
  }

  async retrySecureBackupInspection(): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    this.refreshRoomPresentation();
    return clone(this.snapshot);
  }

  async startDeviceCleanup(): Promise<DesktopSnapshot> {
    return this.getSnapshot();
  }

  async submitDeviceCleanupUia(flowId: number, password: string): Promise<DesktopSnapshot> {
    void flowId;
    void password;
    return this.getSnapshot();
  }

  async eraseLocalDataAnyway(): Promise<DesktopSnapshot> {
    return this.getSnapshot();
  }

  async selectSpace(spaceId: string | null): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    this.rememberActiveRoomForCurrentSpace();
    const nextSpaceId =
      spaceId && this.snapshot.state.domain.spaces.some((space) => space.space_id === spaceId)
        ? spaceId
        : null;
    if (this.snapshot.state.domain.space_members.selected_space_id !== nextSpaceId) {
      const generation = this.snapshot.state.domain.space_members.generation + 1;
      this.snapshot.state.domain.space_members = {
        ...emptyBrowserFakeSpaceMembersState(),
        selected_space_id: nextSpaceId,
        generation
      };
    }
    this.snapshot.state.ui.navigation.active_space_id = nextSpaceId;
    // #445: restore this Space's surface before projecting its room list, and
    // only when the Space actually has remembered state — a Space with no memory
    // must leave the current filter alone.
    const restoredSelection = nextSpaceId ? this.preferredSelectionInSpace(nextSpaceId) : null;
    if (
      nextSpaceId &&
      restoredSelection &&
      this.snapshot.state.ui.navigation.last_selection_by_space_id?.[nextSpaceId]
    ) {
      this.snapshot.state.ui.room_list.active_filter = {
        kind: restoredSelection.surface === "dms" ? "people" : "rooms"
      };
    }
    this.refreshRoomListProjection();
    this.refreshSidebar();

    const targetRoomId = restoredSelection?.room_id ?? null;
    if (targetRoomId && targetRoomId !== this.snapshot.state.ui.navigation.active_room_id) {
      await this.selectRoom(targetRoomId);
    } else if (!targetRoomId) {
      this.clearActiveRoomSelection();
    }
    this.rememberActiveRoomForCurrentSpace();

    return this.getSnapshot();
  }

  async reorderSpaces(spaceIds: string[]): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || !isCompleteSpaceOrder(this.snapshot.state.domain.spaces, spaceIds)) {
      return this.getSnapshot();
    }

    const spaceOrder = [...(this.snapshot.state.ui.navigation.space_order ?? [])];
    for (const space of this.snapshot.state.domain.spaces) {
      if (!spaceOrder.includes(space.space_id)) {
        spaceOrder.push(space.space_id);
      }
    }
    const visibleSpaceIds = new Set(this.snapshot.state.domain.spaces.map((space) => space.space_id));
    const requestedSpaceIds = [...spaceIds];
    for (let index = 0; index < spaceOrder.length; index += 1) {
      if (visibleSpaceIds.has(spaceOrder[index]!)) {
        spaceOrder[index] = requestedSpaceIds.shift()!;
      }
    }

    const positionBySpaceId = new Map(spaceOrder.map((spaceId, index) => [spaceId, index]));
    this.snapshot.state.ui.navigation.space_order = spaceOrder;
    this.snapshot.state.domain.spaces = [...this.snapshot.state.domain.spaces].sort(
      (left, right) =>
        (positionBySpaceId.get(left.space_id) ?? Number.MAX_SAFE_INTEGER) -
        (positionBySpaceId.get(right.space_id) ?? Number.MAX_SAFE_INTEGER)
    );
    this.refreshSidebar();
    return this.getSnapshot();
  }

  async restartSync(): Promise<DesktopSnapshot> {
    if (this.canRestartSync()) {
      this.snapshot.state.domain.sync = "running";
    }

    return this.getSnapshot();
  }

  async updateSettings(patch: SettingsPatch): Promise<DesktopSnapshot> {
    this.snapshot.state.domain.settings.values = applySettingsPatch(
      this.snapshot.state.domain.settings.values,
      patch
    );
    this.snapshot.state.domain.locale_profile = resolveLocaleDisplayProfile(
      this.snapshot.state.domain.settings.values.locale
    );
    this.snapshot.state.domain.typography_profile = resolveTypographyDisplayProfile(
      this.snapshot.state.domain.settings.values.typography
    );
    this.snapshot.state.domain.settings.persistence = { kind: "idle" };
    const attention = this.snapshot.state.domain.native_attention.summary;
    attention.badge_count =
      this.snapshot.state.domain.settings.values.notifications.badges &&
      attention.capabilities.badge !== "unavailable"
        ? attention.unread_count
        : 0;
    this.snapshot.state.ui.room_list = computeBrowserRoomListProjection(
      this.snapshot.state.ui.room_list.active_filter,
      this.snapshot.state.domain.settings.values.room_list_sort,
      this.snapshot.state.ui.navigation.active_space_id,
      this.snapshot.state.domain.spaces,
      this.snapshot.state.domain.rooms,
      this.snapshot.state.domain.invites
    );
    return this.getSnapshot();
  }

  async setRoomUrlPreviewOverride(
    roomId: string,
    enabled: boolean
  ): Promise<DesktopSnapshot> {
    const room = this.snapshot.state.domain.rooms.find((candidate) => candidate.room_id === roomId);
    if (!room || !this.canUseSyncedViews()) {
      return this.getSnapshot();
    }
    const defaultEnabled = room.is_encrypted
      ? this.snapshot.state.domain.settings.values.display.encrypted_url_previews_enabled
      : this.snapshot.state.domain.settings.values.display.url_previews_enabled;
    const preference = { ...this.snapshot.state.domain.room_preferences.rooms[roomId] };
    if (enabled === defaultEnabled) {
      delete this.snapshot.state.domain.link_preview_settings.room_overrides[roomId];
      delete preference.url_previews_enabled_override;
    } else {
      this.snapshot.state.domain.link_preview_settings.room_overrides[roomId] = enabled;
      preference.url_previews_enabled_override = enabled;
    }
    if (
      preference.url_previews_enabled_override === undefined &&
      preference.notification_mode === undefined
    ) {
      delete this.snapshot.state.domain.room_preferences.rooms[roomId];
    } else {
      this.snapshot.state.domain.room_preferences.rooms[roomId] = preference;
    }
    return this.getSnapshot();
  }

  async selectRoomListFilter(filter: RoomListFilter): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }
    this.refreshRoomListProjection(filter);
    return this.getSnapshot();
  }

  async markRoomAsRead(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    // Do NOT mutate unread counts. Tests seed the expected Rust-shaped snapshot.
    void roomId;
    void eventId;
    return this.getSnapshot();
  }

  async markRoomAsUnread(roomId: string, unread: boolean): Promise<DesktopSnapshot> {
    // Do NOT mutate unread counts. Tests seed the expected Rust-shaped snapshot.
    void roomId;
    void unread;
    return this.getSnapshot();
  }

  async setRoomNotificationMode(
    roomId: string,
    mode: RoomNotificationMode
  ): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }
    const known =
      this.snapshot.state.domain.rooms.some((room) => room.room_id === roomId) ||
      this.snapshot.state.domain.invites.some((invite) => invite.room_id === roomId);
    if (!known) {
      return this.getSnapshot();
    }
    this.snapshot.state.domain.room_notification_settings[roomId] = {
      mode,
      operation: { kind: "idle" }
    };
    const preference = { ...this.snapshot.state.domain.room_preferences.rooms[roomId] };
    if (mode.kind === "all") {
      delete preference.notification_mode;
    } else {
      preference.notification_mode = mode;
    }
    if (
      preference.url_previews_enabled_override === undefined &&
      preference.notification_mode === undefined
    ) {
      delete this.snapshot.state.domain.room_preferences.rooms[roomId];
    } else {
      this.snapshot.state.domain.room_preferences.rooms[roomId] = preference;
    }
    this.refreshSidebar();
    this.refreshActivityStreams();
    return this.getSnapshot();
  }

  setRoomListProjection(projection: RoomListProjection): void {
    this.snapshot.state.ui.room_list = projection;
  }

  private refreshRoomListProjection(filter = this.snapshot.state.ui.room_list.active_filter): void {
    this.snapshot.state.ui.room_list = computeBrowserRoomListProjection(
      filter,
      this.snapshot.state.domain.settings.values.room_list_sort,
      this.snapshot.state.ui.navigation.active_space_id,
      this.snapshot.state.domain.spaces,
      this.snapshot.state.domain.rooms,
      this.snapshot.state.domain.invites
    );
  }

  private refreshSidebar(): void {
    this.snapshot.sidebar = composeBrowserFakeSidebar(
      this.snapshot.state.ui.navigation.active_space_id,
      this.snapshot.state.domain.spaces,
      this.snapshot.state.domain.rooms,
      this.snapshot.state.domain.room_notification_settings,
      this.snapshot.state.domain.invites.length
    );
  }

  private refreshRoomPresentation(): void {
    this.snapshot.state.domain.rooms = projectRoomSummaries(
      this.snapshot.state.domain.rooms,
      this.snapshot.state.domain.profile
    );
    this.refreshSidebar();
  }

  async queryDevices(): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }
    this.snapshot.state.domain.device_sessions = {
      kind: "loaded",
      devices: [
        {
          device_ordinal: 1,
          display_name: "Current session",
          current: true,
          verified: true,
          inactive: false
        },
        {
          device_ordinal: 2,
          display_name: "Other session",
          current: false,
          verified: false,
          inactive: true
        }
      ]
    };
    return this.getSnapshot();
  }

  async refreshCurrentSessionStatus(
    trigger: SessionStatusRefreshTrigger
  ): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }
    const requestId = this.nextRequestId();
    this.snapshot.state.domain.current_session_status = {
      status: "checking",
      request_id: requestId,
      trigger
    };
    const session = this.snapshot.state.domain.session;
    this.snapshot.state.domain.current_session_status = {
      status: "ready",
      request_id: requestId,
      details: {
        device_display_name: "Koushi on Linux",
        device_id: session.device_id ?? "",
        authentication_method: "unknown",
        sync_state: this.snapshot.state.domain.sync === "running" ? "running" : "stopped",
        is_cross_signed_by_owner: true,
        own_identity_verification: "verified",
        key_backup: "ready",
        verification: "verified",
        checked_at_ms: Date.now()
      }
    };
    return this.getSnapshot();
  }

  async renameDevice(deviceOrdinal: number, displayName: string): Promise<DesktopSnapshot> {
    if (this.snapshot.state.domain.device_sessions.kind === "loaded") {
      for (const device of this.snapshot.state.domain.device_sessions.devices) {
        if (device.device_ordinal === deviceOrdinal) {
          device.display_name = displayName;
        }
      }
    }
    return this.getSnapshot();
  }

  async deleteDevices(deviceOrdinals: number[]): Promise<DesktopSnapshot> {
    if (this.snapshot.state.domain.device_sessions.kind === "loaded") {
      this.snapshot.state.domain.device_sessions.devices =
        this.snapshot.state.domain.device_sessions.devices.filter(
          (d) => !deviceOrdinals.includes(d.device_ordinal)
        );
    }
    return this.getSnapshot();
  }

  async submitAccountManagementUia(flowId: number, password: string): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }
    void flowId;
    void password;
    this.snapshot.state.domain.account_management = { kind: "idle" };
    return this.getSnapshot();
  }

  async loadAccountManagementCapabilities(): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }
    this.snapshot.state.domain.account_management_capabilities = {
      change_password: { kind: "enabled" }
    };
    return this.getSnapshot();
  }

  async changePassword(newPassword: string): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }
    void newPassword;
    this.snapshot.state.domain.account_management = {
      kind: "succeeded",
      request_id: this.nextRequestId(),
      operation: "changePassword"
    };
    return this.getSnapshot();
  }

  async deactivateAccount(eraseData: boolean): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }
    void eraseData;
    this.snapshot.state.domain.account_management = {
      kind: "succeeded",
      request_id: this.nextRequestId(),
      operation: "deactivateAccount"
    };
    return this.getSnapshot();
  }

  async probeLocalEncryptionHealth(): Promise<DesktopSnapshot> {
    const requestId = this.nextRequestId();
    this.snapshot.state.domain.local_encryption = { kind: "probing", request_id: requestId };
    await Promise.resolve();
    if (
      this.snapshot.state.domain.local_encryption.kind !== "probing" ||
      this.snapshot.state.domain.local_encryption.request_id !== requestId
    ) {
      return this.getSnapshot();
    }
    this.snapshot.state.domain.local_encryption = { kind: "healthy" };
    return this.getSnapshot();
  }

  async resetLocalData(): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    const requestId = this.nextRequestId();
    this.snapshot.state.domain.local_encryption = {
      kind: "resetting",
      request_id: requestId
    };
    await Promise.resolve();
    if (
      this.snapshot.state.domain.local_encryption.kind !== "resetting" ||
      this.snapshot.state.domain.local_encryption.request_id !== requestId
    ) {
      return this.getSnapshot();
    }
    this.snapshot.state.domain.session = { kind: "signedOut" };
    this.snapshot.state.domain.session_lock_reason = null;
    this.snapshot.state.domain.sync = "stopped";
    this.snapshot.state.domain.local_encryption = { kind: "unknown" };
    this.clearSessionViews();
    return this.getSnapshot();
  }

  async bootstrapCrossSigning(): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    this.snapshot.state.domain.e2ee_trust.cross_signing = { kind: "trusted" };
    return this.getSnapshot();
  }

  async enableKeyBackup(): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    this.snapshot.state.domain.e2ee_trust.key_backup = {
      kind: "enabled",
      version: "browser-preview"
    };
    return this.getSnapshot();
  }

  async exportRoomKeys(destinationPath: string, passphrase: string): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    void destinationPath;
    void passphrase;
    this.snapshot.state.domain.e2ee_trust.key_management.room_key_export = {
      kind: "exported",
      request_id: this.nextRequestId(),
      exported_sessions: null
    };
    return this.getSnapshot();
  }

  async importRoomKeys(sourcePath: string, passphrase: string): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    void sourcePath;
    void passphrase;
    this.snapshot.state.domain.e2ee_trust.key_management.room_key_import = {
      kind: "imported",
      request_id: this.nextRequestId(),
      imported_count: 1,
      total_count: 1
    };
    return this.getSnapshot();
  }

  async bootstrapSecureBackup(
    passphrase: string | null,
    recoveryKeyDestinationPath: string | null
  ): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    void passphrase;
    this.snapshot.state.domain.e2ee_trust.key_management.secure_backup_setup = {
      kind: "recoveryKeyReady",
      request_id: this.nextRequestId(),
      delivery: recoveryKeyDestinationPath?.trim() ? { kind: "written" } : { kind: "notWritten" }
    };
    return this.getSnapshot();
  }

  async changeSecureBackupPassphrase(
    oldSecret: string,
    newPassphrase: string,
    recoveryKeyDestinationPath: string | null
  ): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    void oldSecret;
    void newPassphrase;
    this.snapshot.state.domain.e2ee_trust.key_management.passphrase_change = {
      kind: "changed",
      request_id: this.nextRequestId(),
      delivery: recoveryKeyDestinationPath?.trim() ? { kind: "written" } : { kind: "notWritten" }
    };
    return this.getSnapshot();
  }

  async acceptVerification(flowId: number): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    const verification = this.snapshot.state.domain.e2ee_trust.verification;
    if (verification.kind === "requested" && verification.request_id === flowId) {
      this.snapshot.state.domain.e2ee_trust.verification = {
        kind: "accepted",
        request_id: flowId,
        target: verification.target
      };
    }
    return this.getSnapshot();
  }

  async startOwnUserSas(): Promise<DesktopSnapshot> {
    const flowId = this.nextRequestId();
    const session = this.snapshot.state.domain.session;
    if (session.kind === "awaitingVerification") {
      this.snapshot.state.domain.session = {
        kind: "verifying",
        homeserver: session.homeserver,
        user_id: session.user_id,
        device_id: session.device_id,
        gate: { ...session.gate, failureKind: null },
        method: "existingDeviceSas",
        flow_id: flowId,
        sas_emojis: []
      };
    }
    return this.getSnapshot();
  }
  async retryCurrentDeviceTrustDiscovery(): Promise<DesktopSnapshot> {
    const session = this.snapshot.state.domain.session;
    if (session.kind === "awaitingVerification" || session.kind === "provisional") this.snapshot.state.domain.session = { ...session, kind: "provisional", phase: { recheckingTrust: {} } };
    return this.getSnapshot();
  }
  async mismatchSasVerification(flowId: number): Promise<DesktopSnapshot> {
    const session = this.snapshot.state.domain.session;
    if (session.kind === "verifying" && session.flow_id === flowId) {
      this.snapshot.state.domain.session = {
        kind: "awaitingVerification",
        homeserver: session.homeserver,
        user_id: session.user_id,
        device_id: session.device_id,
        gate: { ...session.gate, failureKind: "mismatch" }
      };
    }
    return this.getSnapshot();
  }
  async startSessionBootstrap(passphrase: string | null, recoveryKeyDestinationPath: string): Promise<DesktopSnapshot> {
    const flowId = this.nextRequestId();
    const session = this.snapshot.state.domain.session;
    void passphrase;
    if (session.kind === "awaitingVerification" && recoveryKeyDestinationPath.trim()) {
      this.snapshot.state.domain.session = {
        kind: "awaitingBootstrapConfirmation",
        homeserver: session.homeserver,
        user_id: session.user_id,
        device_id: session.device_id,
        gate: { ...session.gate, failureKind: null },
        flow_id: flowId,
        destination_written: true
      };
    }
    return this.getSnapshot();
  }
  async confirmSessionBootstrapSaved(flowId: number): Promise<DesktopSnapshot> {
    const session = this.snapshot.state.domain.session;
    if (session.kind === "awaitingBootstrapConfirmation" && session.flow_id === flowId) this.snapshot.state.domain.session = { ...session, kind: "provisional", phase: { recheckingTrust: {} }, flow_id: undefined, destination_written: undefined };
    return this.getSnapshot();
  }

  async confirmSasVerification(flowId: number): Promise<DesktopSnapshot> {
    const session = this.snapshot.state.domain.session;
    if (
      session.kind === "verifying" &&
      session.method === "existingDeviceSas" &&
      session.flow_id === flowId
    ) {
      this.snapshot.state.domain.session = {
        ...session,
        kind: "provisional",
        phase: { recheckingTrust: { failureKind: null } },
        method: undefined,
        flow_id: undefined,
        sas_emojis: undefined
      };
      this.snapshot.state.domain.e2ee_trust.verification = { kind: "idle" };
      return this.getSnapshot();
    }
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    const verification = this.snapshot.state.domain.e2ee_trust.verification;
    if (
      (verification.kind === "sasPresented" || verification.kind === "confirming") &&
      verification.request_id === flowId
    ) {
      this.snapshot.state.domain.e2ee_trust.verification = {
        kind: "done",
        request_id: flowId,
        target: verification.target
      };
    }
    return this.getSnapshot();
  }

  async cancelVerification(flowId: number): Promise<DesktopSnapshot> {
    const session = this.snapshot.state.domain.session;
    if (session.kind === "verifying" && session.flow_id === flowId) {
      this.snapshot.state.domain.session = {
        kind: "awaitingVerification",
        homeserver: session.homeserver,
        user_id: session.user_id,
        device_id: session.device_id,
        gate: { ...session.gate, failureKind: "cancelled" }
      };
      return this.getSnapshot();
    }
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    const verification = this.snapshot.state.domain.e2ee_trust.verification;
    if (verification.kind !== "idle" && verification.request_id === flowId) {
      this.snapshot.state.domain.e2ee_trust.verification = { kind: "idle" };
    }
    return this.getSnapshot();
  }

  async resetIdentity(): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    this.snapshot.state.domain.e2ee_trust.identity_reset = {
      kind: "awaitingAuth",
      request_id: this.nextRequestId(),
      auth_type: "uiaa"
    };
    return this.getSnapshot();
  }

  async cancelIdentityReset(flowId: number): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    const identityReset = this.snapshot.state.domain.e2ee_trust.identity_reset;
    if (identityReset.kind === "awaitingAuth" && identityReset.request_id === flowId) {
      this.snapshot.state.domain.e2ee_trust.identity_reset = {
        kind: "failed",
        request_id: flowId,
        failureKind: "cancelled"
      };
      this.snapshot.state.domain.e2ee_trust.cross_signing = {
        kind: "failed",
        request_id: flowId,
        failureKind: "cancelled"
      };
    }
    return this.getSnapshot();
  }

  async submitIdentityResetPassword(flowId: number, password: string): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    void password;
    const identityReset = this.snapshot.state.domain.e2ee_trust.identity_reset;
    if (identityReset.kind === "awaitingAuth" && identityReset.request_id === flowId) {
      this.completeIdentityReset();
    }
    return this.getSnapshot();
  }

  async submitIdentityResetOAuth(flowId: number): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    const identityReset = this.snapshot.state.domain.e2ee_trust.identity_reset;
    if (identityReset.kind === "awaitingAuth" && identityReset.request_id === flowId) {
      this.completeIdentityReset();
    }
    return this.getSnapshot();
  }

  async resolveComposerKeyAction(
    surface: ComposerSurface,
    keyEvent: ComposerKeyEvent,
    options: ComposerResolverOptions
  ): Promise<ComposerResolvedAction> {
    void surface;
    return resolveComposerKeyActionFromSettings(
      this.snapshot.state.domain.settings.values.keyboard.composer_send_shortcut,
      keyEvent,
      options
    );
  }

  async selectRoom(roomId: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }
    const selectedRoom = this.snapshot.state.domain.rooms.find((room) => room.room_id === roomId);
    if (!selectedRoom) {
      return this.getSnapshot();
    }

    const outgoingRoomId = this.snapshot.state.ui.navigation.active_room_id;
    if (outgoingRoomId && outgoingRoomId !== roomId) {
      this.clearPreparedUploadBytes({ kind: "main", room_id: outgoingRoomId });
      this.snapshot.state.ui.timeline.staged_uploads = [];
    }

    this.rememberActiveRoomForCurrentSpace();
    if (!selectedRoom.is_dm) {
      const activeSpaceContainsSelectedRoom = Boolean(
        this.snapshot.state.ui.navigation.active_space_id &&
          selectedRoom.parent_space_ids.includes(this.snapshot.state.ui.navigation.active_space_id)
      );
      if (!activeSpaceContainsSelectedRoom) {
        this.snapshot.state.ui.navigation.active_space_id =
          selectedRoom.parent_space_ids[0] ?? null;
        this.refreshRoomListProjection();
        this.refreshSidebar();
      }
    }
    const openThreadRoomId =
      this.snapshot.state.ui.thread.kind === "open" ? this.snapshot.state.ui.thread.room_id : null;
    if (openThreadRoomId) this.clearPreparedThreadUploadBytesForRoom(openThreadRoomId);
    this.snapshot.state.ui.navigation.active_room_id = roomId;
    this.snapshot.state.ui.navigation.main_timeline_anchor = null;
    this.snapshot.state.ui.timeline.room_id = roomId;
    this.snapshot.state.ui.timeline.is_subscribed = true;
    this.snapshot.state.ui.timeline.composer = {
      accepted_submission_ids: [],
      pending_transaction_id: null,
      draft_revision:
        this.composerDraftRevisions.get(roomId) ?? COMPOSER_DRAFT_REVISION_ZERO,
      last_accepted_clear_revision: COMPOSER_DRAFT_REVISION_ZERO,
      document: this.composerDrafts.get(roomId) ?? documentFromText(""),
      draft: plainBodyFromDocument(this.composerDrafts.get(roomId) ?? documentFromText("")),
      mode: "Plain"
    };
    this.snapshot.state.ui.thread = { kind: "closed" };
    this.snapshot.state.domain.thread_attention = { kind: "closed" };
    this.snapshot.state.ui.threads_list = { kind: "closed" };
    this.snapshot.state.ui.focused_context = { kind: "closed" };
    this.snapshot.thread = null;
    this.snapshot.timeline = timelineMessages.filter((message) => message.room_id === roomId);
    this.rememberActiveRoomForCurrentSpace();
    return this.getSnapshot();
  }

  async selectSearchResult(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    await this.selectRoom(roomId);
    this.snapshot.state.ui.navigation.main_timeline_anchor = { event_id: eventId };
    this.snapshot.state.ui.focused_context = { kind: "closed" };
    return this.getSnapshot();
  }

  async openActivityEvent(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    await this.selectRoom(roomId);
    this.snapshot.state.ui.navigation.main_timeline_anchor = { event_id: eventId };
    this.snapshot.state.ui.focused_context = { kind: "closed" };
    return this.getSnapshot();
  }

  async openPinnedEvent(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    return this.openActivityEvent(roomId, eventId);
  }

  async acknowledgeTimelineProjection(): Promise<void> {
    // Browser fakes apply snapshots synchronously and have no Core actor lease.
  }

  async acknowledgeTimelineBatchRendered(): Promise<void> {
    // Browser fakes apply timeline batches synchronously and have no Core actor lease.
  }

  async openTimelineAtTimestamp(
    roomId: string,
    timestampMs: number
  ): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    await this.selectRoom(roomId);
    const roomMessages = timelineMessages.filter((message) => message.room_id === roomId);
    const target =
      roomMessages.find((message) => message.timestamp_ms >= timestampMs) ??
      roomMessages.at(-1);
    if (target) {
      this.snapshot.state.ui.focused_context = {
        kind: "opening",
        room_id: roomId,
        event_id: target.event_id
      };
    }
    return this.getSnapshot();
  }

  async closeFocusedContext(): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    this.snapshot.state.ui.focused_context = { kind: "closed" };
    this.snapshot.state.ui.navigation.main_timeline_anchor = null;
    return this.getSnapshot();
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
    const body = plainBodyFromDocument(document);
    this.requireComposerLease(
      account,
      { kind: "main", room_id: roomId },
      leaseId,
      rendererGeneration
    );
    const replay = this.replaySubmission(submissionId);
    if (replay) return replay;
    const session = this.snapshot.state.domain.session;
    if (
      session.kind !== "ready" ||
      !browserComposerAccountMatches(session, account) ||
      !session.user_id ||
      this.snapshot.state.ui.timeline.room_id !== roomId ||
      body.trim().length === 0
    ) {
      return {
        outcome: { rejected: { kind: "invalid" } },
        submissionId,
        transactionId: null,
        snapshot: await this.getSnapshot()
      };
    }
    this.preflightComposerDraftAcceptance({ kind: "main", room_id: roomId }, draftRevision);
    const sender = session.user_id;
    const composer = this.snapshot.state.ui.timeline.composer;
    const transactionId = this.acceptSubmission(submissionId, composer);

    this.snapshot.timeline = [
      ...this.snapshot.timeline,
      {
        room_id: roomId,
        event_id: `$local-browser-${this.snapshot.timeline.length + 1}`,
        sender,
        timestamp_ms: 1_820_000_000_000 + this.snapshot.timeline.length,
        body,
        attachment_filename: null,
        reply_count: 0
      }
    ];
    this.terminalSubmission(composer);
    this.acceptComposerDraftTarget({ kind: "main", room_id: roomId }, draftRevision);
    return {
      outcome: "accepted",
      submissionId,
      transactionId,
      snapshot: await this.getSnapshot()
    };
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
    this.requireComposerLease(account, target, leaseId, rendererGeneration);
    const session = this.snapshot.state.domain.session;
    if (
      session.kind !== "ready" ||
      !browserComposerAccountMatches(session, account) ||
      !browserComposerTargetIsActive(this.snapshot, target) ||
      body.trim().length === 0 ||
      !Number.isFinite(sendAtMs)
    ) {
      return { acceptedRevision: null, snapshot: await this.getSnapshot() };
    }
    this.preflightComposerDraftAcceptance(target, draftRevision);

    this.snapshot.state.ui.timeline.scheduled_send_capability = "localFallback";
    this.snapshot.state.ui.timeline.scheduled_sends = [
      ...this.snapshot.state.ui.timeline.scheduled_sends,
      {
        scheduled_id: `browser-scheduled-${this.snapshot.state.ui.timeline.scheduled_sends.length + 1}`,
        room_id: target.room_id,
        thread_root_event_id: target.kind === "thread" ? target.root_event_id : null,
        body,
        send_at_ms: sendAtMs,
        handle: { kind: "local" }
      }
    ];
    const acceptedRevision = this.acceptComposerDraftTarget(target, draftRevision);
    return { acceptedRevision, snapshot: await this.getSnapshot() };
  }

  async stageUploads(
    roomId: string,
    items: UploadStagingRequestItem[]
  ): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || this.snapshot.state.ui.timeline.room_id !== roomId) {
      return this.getSnapshot();
    }
    this.clearPreparedUploadBytes({ kind: "main", room_id: roomId });
    this.snapshot.state.ui.timeline.staged_uploads = items.map((item, index) => ({
      staged_id: item.stagedId,
      room_id: roomId,
      position: item.position || index + 1,
      filename: item.filename.trim() || "attachment",
      mime_type: item.mimeType.trim() || "application/octet-stream",
      byte_count: Math.max(0, Math.floor(item.byteCount)),
      kind: item.kind,
      caption: null,
      compression_choice: item.compressionChoice,
      preparation: { kind: "preparing" }
    }));
    return this.getSnapshot();
  }

  async stageUploadBytes(
    target: ComposerTarget,
    items: StageUploadBytesRequestItem[]
  ): Promise<DesktopSnapshot> {
    if (items.length === 0 || items.length > MAX_PREPARATION_BATCH_SIZE) {
      throw new Error(ATTACHMENT_BATCH_ERROR);
    }
    const totalBytes = items.reduce((total, item) => total + item.bytes.length, 0);
    if (!Number.isSafeInteger(totalBytes) || totalBytes > MAX_PREPARATION_BATCH_BYTES) {
      throw new Error(ATTACHMENT_BATCH_ERROR);
    }
    if (!this.canUseSyncedViews() || !browserComposerTargetIsActive(this.snapshot, target)) {
      return this.getSnapshot();
    }
    this.clearPreparedUploadBytes(target);
    const staged = items.map((item, index) => {
      const prepared = browserPreparedUploadItem(target, item, index);
      if (prepared.preparation.kind === "ready") {
        prepared.preparation.variants.forEach((variant) => {
          this.preparedUploadBytes.set(
            browserPreparedUploadKey(target, item.stagedId, variant.variant_id),
            item.bytes
          );
        });
      }
      return prepared;
    });
    if (target.kind === "main") {
      this.snapshot.state.ui.timeline.staged_uploads = staged;
    } else if (this.snapshot.state.ui.thread.kind === "open") {
      this.snapshot.state.ui.thread.staged_uploads = staged;
    }
    return this.getSnapshot();
  }

  async selectStagedUploadOutput(
    target: ComposerTarget,
    stagedId: string,
    selection: StagedUploadOutputSelection
  ): Promise<DesktopSnapshot> {
    const items = browserStagedUploadsForTarget(this.snapshot, target);
    const next = items.map((item) => {
      if (item.staged_id !== stagedId || item.preparation.kind !== "ready") {
        return item;
      }
      const prepared = item.preparation.variants.find(
        (variant) =>
          variant.resize === selection.resize && variant.format_choice === selection.format
      );
      // Mirrors the Rust store: an already-prepared pair is adopted at once, an
      // unprepared pair becomes pending under a fresh generation.
      if (!prepared) {
        return {
          ...item,
          preparation: {
            ...item.preparation,
            selected: selection,
            pending: selection,
            generation: item.preparation.generation + 1
          }
        };
      }
      return {
        ...item,
        filename: prepared.filename,
        mime_type: prepared.mime_type,
        byte_count: prepared.byte_count,
        kind: {
          kind: "image" as const,
          width: prepared.width,
          height: prepared.height
        },
        preparation: {
          ...item.preparation,
          selected: selection,
          pending: null
        }
      };
    });
    setBrowserStagedUploadsForTarget(this.snapshot, target, next);
    return this.getSnapshot();
  }

  async preparedUploadPreview(
    target: ComposerTarget,
    stagedId: string,
    variantId: string
  ): Promise<number[]> {
    return this.preparedUploadBytes.get(browserPreparedUploadKey(target, stagedId, variantId)) ?? [];
  }

  async retryStagedUploadPreparation(
    target: ComposerTarget,
    _stagedId: string
  ): Promise<DesktopSnapshot> {
    if (!browserComposerTargetIsActive(this.snapshot, target)) return this.getSnapshot();
    return this.getSnapshot();
  }

  async useOriginalStagedUpload(
    target: ComposerTarget,
    stagedId: string
  ): Promise<DesktopSnapshot> {
    const items = browserStagedUploadsForTarget(this.snapshot, target).map((item) => {
      if (item.staged_id !== stagedId || item.preparation.kind !== "failed") return item;
      const variant = {
        variant_id: "original",
        filename: item.filename,
        mime_type: item.mime_type,
        byte_count: item.byte_count,
        width: item.kind.kind === "image" ? item.kind.width : null,
        height: item.kind.kind === "image" ? item.kind.height : null,
        format: "original" as const,
        resize: "original" as const,
        format_choice: "keep" as const,
        savings_percent: 0,
        metadata_stripped: false,
        thumbnail_refreshed: false
      };
      return {
        ...item,
        preparation: {
          kind: "ready" as const,
          variants: [variant],
          selected: { resize: "original" as const, format: "keep" as const },
          pending: null,
          generation: 0
        }
      };
    });
    setBrowserStagedUploadsForTarget(this.snapshot, target, items);
    return this.getSnapshot();
  }

  async sendPreparedUploads(
    account: ComposerDraftAccountOwner,
    leaseId: string,
    rendererGeneration: string,
    target: ComposerTarget,
    draftRevision: ComposerDraftRevision
  ): Promise<ComposerDraftAcceptanceResponse> {
    this.requireComposerLease(account, target, leaseId, rendererGeneration);
    if (!browserComposerAccountMatches(this.snapshot.state.domain.session, account)) {
      return { acceptedRevision: null, snapshot: await this.getSnapshot() };
    }
    this.preflightComposerDraftAcceptance(target, draftRevision);
    setBrowserStagedUploadsForTarget(this.snapshot, target, []);
    this.clearPreparedUploadBytes(target);
    const acceptedRevision = this.acceptComposerDraftTarget(target, draftRevision);
    return { acceptedRevision, snapshot: await this.getSnapshot() };
  }

  async updateStagedUploadCaption(
    target: ComposerTarget,
    stagedId: string,
    caption: string | null
  ): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || !browserComposerTargetIsActive(this.snapshot, target)) {
      return this.getSnapshot();
    }
    const normalized = caption?.trim() ? caption : null;
    setBrowserStagedUploadsForTarget(this.snapshot, target, browserStagedUploadsForTarget(this.snapshot, target).map(
      (item) =>
        item.staged_id === stagedId
          ? {
              ...item,
              caption: normalized
                ? { plain_body: normalized, formatted_body: null, mentions: { targets: [] } }
                : null
            }
          : item
    ));
    return this.getSnapshot();
  }

  async updateStagedUploadCompression(
    stagedId: string,
    compressionChoice: StagedUploadCompressionChoice
  ): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }
    this.snapshot.state.ui.timeline.staged_uploads = this.snapshot.state.ui.timeline.staged_uploads.map(
      (item) =>
        item.staged_id === stagedId
          ? { ...item, compression_choice: compressionChoice }
          : item
    );
    return this.getSnapshot();
  }

  async clearUploadStaging(target: ComposerTarget): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || !browserComposerTargetIsActive(this.snapshot, target)) {
      return this.getSnapshot();
    }
    setBrowserStagedUploadsForTarget(this.snapshot, target, []);
    this.clearPreparedUploadBytes(target);
    return this.getSnapshot();
  }

  async cancelScheduledSend(scheduledId: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }
    this.snapshot.state.ui.timeline.scheduled_sends =
      this.snapshot.state.ui.timeline.scheduled_sends.filter(
        (item) => item.scheduled_id !== scheduledId
      );
    return this.getSnapshot();
  }

  async rescheduleScheduledSend(
    scheduledId: string,
    body: string,
    sendAtMs: number
  ): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || !body.trim() || !Number.isFinite(sendAtMs)) {
      return this.getSnapshot();
    }
    this.snapshot.state.ui.timeline.scheduled_sends =
      this.snapshot.state.ui.timeline.scheduled_sends.map((item) =>
        item.scheduled_id === scheduledId ? { ...item, body, send_at_ms: sendAtMs } : item
      );
    return this.getSnapshot();
  }

  async retrySend(roomId: string, transactionId: string): Promise<DesktopSnapshot> {
    void roomId;
    void transactionId;
    return this.getSnapshot();
  }

  async cancelSend(roomId: string, transactionId: string): Promise<DesktopSnapshot> {
    void roomId;
    void transactionId;
    return this.getSnapshot();
  }

  async sendReaction(
    roomId: string,
    eventId: string,
    reactionKey: string
  ): Promise<DesktopSnapshot> {
    void roomId;
    void eventId;
    void reactionKey;
    return this.getSnapshot();
  }

  async redactReaction(
    roomId: string,
    eventId: string,
    reactionKey: string,
    reactionEventId: string
  ): Promise<DesktopSnapshot> {
    void roomId;
    void eventId;
    void reactionKey;
    void reactionEventId;
    return this.getSnapshot();
  }

  async sendReadReceipt(
    roomId: string,
    eventId: string,
    threadRootEventId?: string | null
  ): Promise<void> {
    // The fake is a transport boundary, not a second read-state reducer. A
    // Rust-shaped snapshot/event must install receipt state before the fake
    // exposes it to the UI.
    void roomId;
    void eventId;
    void threadRootEventId;
  }

  async setFullyRead(roomId: string, eventId: string): Promise<void> {
    // See sendReadReceipt: never repair the installed snapshot locally.
    void roomId;
    void eventId;
  }

  async setTyping(roomId: string, isTyping: boolean): Promise<void> {
    const session = this.snapshot.state.domain.session;
    if (!this.isReady() || !session.user_id) {
      return;
    }
    const roomSignals = ensureRoomLiveSignals(this.snapshot, roomId);
    const withoutSelf = roomSignals.typing_user_ids.filter((userId) => userId !== session.user_id);
    roomSignals.typing_user_ids = isTyping ? [...withoutSelf, session.user_id] : withoutSelf;
    roomSignals.typing_users = roomSignals.typing_user_ids.map((userId) => ({
      user_id: userId,
      display_label:
        this.snapshot.state.domain.profile.local_aliases[userId]?.trim() ||
        this.snapshot.state.domain.profile.users[userId]?.display_label?.trim() ||
        (userId === session.user_id
          ? this.snapshot.state.domain.profile.own.display_name?.trim() || null
          : null)
    }));
  }

  async setPresence(presence: PresenceKind): Promise<DesktopSnapshot> {
    const session = this.snapshot.state.domain.session;
    if (!this.isReady() || !session.user_id) {
      return this.getSnapshot();
    }
    this.snapshot.state.domain.live_signals.presence[session.user_id] = presence;
    return this.getSnapshot();
  }

  async setDisplayName(displayName: string | null): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }
    const normalized = displayName?.trim() ? displayName.trim() : null;
    const requestId = this.nextRequestId();
    this.snapshot.state.domain.profile.update = {
      kind: "settingDisplayName",
      request_id: requestId,
      display_name: normalized
    };
    this.snapshot.state.domain.profile.own.display_name = normalized;
    this.snapshot.state.domain.profile.update = { kind: "idle" };
    return this.getSnapshot();
  }

  async setLocalUserAlias(userId: string, alias: string | null): Promise<DesktopSnapshot> {
    if (!this.isReady() || userId.trim().length === 0) {
      return this.getSnapshot();
    }

    const normalizedUserId = userId.trim();
    const normalizedAlias = alias?.trim() ? alias.trim() : null;
    const requestId = this.nextRequestId();
    this.snapshot.state.domain.profile.local_alias_update = {
      kind: "saving",
      request_id: requestId
    };

    await Promise.resolve();

    if (
      this.snapshot.state.domain.profile.local_alias_update.kind !== "saving" ||
      this.snapshot.state.domain.profile.local_alias_update.request_id !== requestId
    ) {
      return this.getSnapshot();
    }
    if (normalizedAlias) {
      this.snapshot.state.domain.profile.local_aliases[normalizedUserId] = normalizedAlias;
    } else {
      delete this.snapshot.state.domain.profile.local_aliases[normalizedUserId];
    }
    this.refreshLocalAliasProjections(normalizedUserId);
    this.snapshot.state.domain.profile.local_alias_update = { kind: "idle" };
    return this.getSnapshot();
  }

  async ignoreUser(userId: string): Promise<DesktopSnapshot> {
    if (!this.isReady() || !userId.trim()) {
      return this.getSnapshot();
    }
    const normalizedUserId = userId.trim();
    const requestId = this.nextRequestId();
    this.snapshot.state.domain.profile.ignored_user_update = {
      kind: "saving",
      request_id: requestId
    };
    await Promise.resolve();
    if (
      this.snapshot.state.domain.profile.ignored_user_update.kind !== "saving" ||
      this.snapshot.state.domain.profile.ignored_user_update.request_id !== requestId
    ) {
      return this.getSnapshot();
    }
    if (!this.snapshot.state.domain.profile.ignored_user_ids.includes(normalizedUserId)) {
      this.snapshot.state.domain.profile.ignored_user_ids = [
        ...this.snapshot.state.domain.profile.ignored_user_ids,
        normalizedUserId
      ];
    }
    this.snapshot.state.domain.profile.ignored_user_update = { kind: "idle" };
    return this.getSnapshot();
  }

  async unignoreUser(userId: string): Promise<DesktopSnapshot> {
    if (!this.isReady() || !userId.trim()) {
      return this.getSnapshot();
    }
    const normalizedUserId = userId.trim();
    const requestId = this.nextRequestId();
    this.snapshot.state.domain.profile.ignored_user_update = {
      kind: "saving",
      request_id: requestId
    };
    await Promise.resolve();
    if (
      this.snapshot.state.domain.profile.ignored_user_update.kind !== "saving" ||
      this.snapshot.state.domain.profile.ignored_user_update.request_id !== requestId
    ) {
      return this.getSnapshot();
    }
    this.snapshot.state.domain.profile.ignored_user_ids =
      this.snapshot.state.domain.profile.ignored_user_ids.filter((id) => id !== normalizedUserId);
    this.snapshot.state.domain.profile.ignored_user_update = { kind: "idle" };
    return this.getSnapshot();
  }

  async reportUser(userId: string, reason: string): Promise<DesktopSnapshot> {
    if (!this.isReady() || !userId.trim()) {
      return this.getSnapshot();
    }
    void reason;
    return this.getSnapshot();
  }

  async reportContent(
    roomId: string,
    eventId: string,
    reason: string
  ): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || !roomId.trim() || !eventId.trim()) {
      return this.getSnapshot();
    }
    void reason;
    return this.getSnapshot();
  }

  async reportRoom(roomId: string, reason: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || !roomId.trim()) {
      return this.getSnapshot();
    }
    void reason;
    return this.getSnapshot();
  }

  async setAvatar(mimeType: string, bytes: number[]): Promise<DesktopSnapshot> {
    if (!this.isReady() || bytes.length === 0) {
      return this.getSnapshot();
    }
    const requestId = this.nextRequestId();
    this.snapshot.state.domain.profile.update = {
      kind: "settingAvatar",
      request_id: requestId,
      mime_type: mimeType,
      byte_count: bytes.length
    };
    this.snapshot.state.domain.profile.own.avatar = {
      mxc_uri: "mxc://browser.fake/profile-avatar",
      thumbnail: { kind: "notRequested" }
    };
    this.snapshot.state.domain.profile.update = { kind: "idle" };
    return this.getSnapshot();
  }

  async editMessage(
    roomId: string,
    eventId: string,
    document: ComposerDocument
  ): Promise<DesktopSnapshot> {
    const body = plainBodyFromDocument(document);
    if (!this.isReady() || body.trim().length === 0) {
      return this.getSnapshot();
    }

    // An edit replaces the visible text only. Core edits a media event's caption
    // in place, so the attachment survives (issue #328); clearing
    // attachment_filename here would model data loss the product does not have.
    this.snapshot.timeline = this.snapshot.timeline.map((message) =>
      message.room_id === roomId && message.event_id === eventId
        ? { ...message, body }
        : message
    );
    return this.getSnapshot();
  }

  async redactMessage(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    this.snapshot.timeline = this.snapshot.timeline.filter(
      (message) => !(message.room_id === roomId && message.event_id === eventId)
    );
    return this.getSnapshot();
  }

  async loadMessageSource(_roomId: string, _eventId: string): Promise<DesktopSnapshot> {
    return this.getSnapshot();
  }

  async requestRoomKey(
    _roomId: string,
    _eventId: string,
    _origin?: "user" | "automatic",
    _timelineKey?: import("../domain/coreEvents").TimelineKey
  ): Promise<DesktopSnapshot> {
    return this.getSnapshot();
  }

  async requestLateDecryption(
    _roomId: string,
    _timelineKey?: import("../domain/coreEvents").TimelineKey
  ): Promise<DesktopSnapshot> {
    return this.getSnapshot();
  }

  async forwardMessage(
    _roomId: string,
    _sourceEventId: string,
    _destinationRoomId: string
  ): Promise<DesktopSnapshot> {
    return this.getSnapshot();
  }

  async loadLinkPreviews(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    const message = this.findTimelineMessage(roomId, eventId);
    if (message && message.link_previews?.some((preview) => preview.state === "pending")) {
      await new Promise((resolve) => setTimeout(resolve, 50));
      const readyPreviews: LinkPreview[] = message.link_previews.map((preview) =>
        preview.state === "pending"
          ? {
              ...preview,
              title: preview.title ?? "Synthetic preview",
              description: preview.description ?? "A synthetic link preview for testing.",
              image: preview.image ?? syntheticLinkPreviewImage(),
              state: "ready" as LinkPreviewState
            }
          : preview
      );
      this.updateTimelineMessageLinkPreviews(roomId, eventId, readyPreviews);
    }
    return this.getSnapshot();
  }

  async hideLinkPreview(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    this.updateTimelineMessageLinkPreviews(roomId, eventId, []);
    return this.getSnapshot();
  }

  async leaveRoom(roomId: string): Promise<DesktopSnapshot> {
    return this.removeRoomFromFakeSnapshot(roomId);
  }

  async forgetRoom(roomId: string): Promise<DesktopSnapshot> {
    return this.removeRoomFromFakeSnapshot(roomId);
  }

  async openThread(
    roomId: string,
    rootEventId: string,
    intent: ThreadOpenIntent
  ): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    this.clearPreparedThreadUploadBytesForRoom(roomId);
    this.snapshot.state.ui.thread = {
      kind: "open",
      room_id: roomId,
      root_event_id: rootEventId,
      intent,
      is_subscribed: true,
      staged_uploads: [],
      composer: {
        accepted_submission_ids: [],
        pending_transaction_id: null,
        draft_revision:
          this.threadComposerDraftRevisions.get(
            browserComposerDraftTargetKey({
              kind: "thread",
              room_id: roomId,
              root_event_id: rootEventId
            })
          ) ?? COMPOSER_DRAFT_REVISION_ZERO,
        last_accepted_clear_revision: COMPOSER_DRAFT_REVISION_ZERO,
        document:
          this.threadComposerDrafts.get(
            browserComposerDraftTargetKey({
              kind: "thread",
              room_id: roomId,
              root_event_id: rootEventId
            })
          ) ?? documentFromText(""),
        draft: plainBodyFromDocument(
          this.threadComposerDrafts.get(
            browserComposerDraftTargetKey({
              kind: "thread",
              room_id: roomId,
              root_event_id: rootEventId
            })
          ) ?? documentFromText("")
        ),
        mode: "Plain"
      }
    };
    this.snapshot.thread = {
      room_id: roomId,
      root_event_id: rootEventId,
      replies: threadReplies.filter(
        (reply) => reply.room_id === roomId && reply.root_event_id === rootEventId
      )
    };
    return this.getSnapshot();
  }

  async closeThread(): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    const openThreadRoomId =
      this.snapshot.state.ui.thread.kind === "open" ? this.snapshot.state.ui.thread.room_id : null;
    if (openThreadRoomId) this.clearPreparedThreadUploadBytesForRoom(openThreadRoomId);
    this.snapshot.state.ui.thread = { kind: "closed" };
    this.snapshot.thread = null;
    return this.getSnapshot();
  }

  async openThreadsList(scope: ThreadsListScope): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }
    const resolved = resolveThreadsListScope(scope, this.snapshot);
    if (
      scope.kind === "room" &&
      !this.snapshot.state.domain.rooms.some((room) => room.room_id === scope.room_id)
    ) {
      return this.getSnapshot();
    }
    const scopeKey = threadsListScopeKey(scope);
    this.snapshot.state.ui.threads_list = {
      kind: "open",
      room_id: scopeKey,
      request_id: 0,
      items: threadsListItemsForRooms(resolved.room_ids),
      is_paginating: false,
      end_reached: true,
    };
    return this.getSnapshot();
  }

  async closeThreadsList(): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }
    this.snapshot.state.ui.threads_list = { kind: "closed" };
    return this.getSnapshot();
  }

  async openFilesView(
    scope: FilesViewScope,
    filter: AttachmentFilter,
    sort: AttachmentSort
  ): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }
    const resolvedScope = this.resolveFilesViewScope(scope);
    this.snapshot.state.ui.files_view = {
      kind: "open",
      request_id: 0,
      scope: resolvedScope,
      filter,
      sort,
      items: attachmentResultsForScope(resolvedScope, filter, sort),
      selected_event_id: null
    };
    return this.getSnapshot();
  }

  async closeFilesView(): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }
    this.snapshot.state.ui.files_view = { kind: "closed" };
    return this.getSnapshot();
  }

  private resolveFilesViewScope(scope: FilesViewScope) {
    if (scope.kind === "space") {
      const space = this.snapshot.state.domain.spaces.find((s) => s.space_id === scope.space_id);
      return {
        kind: "space" as const,
        space_id: scope.space_id,
        child_room_ids: space?.child_room_ids ?? []
      };
    }
    return scope;
  }

  async paginateThreadsList(scope: ThreadsListScope): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }
    const list = this.snapshot.state.ui.threads_list;
    if (
      list.kind === "open" &&
      list.room_id === threadsListScopeKey(scope) &&
      !list.is_paginating &&
      !list.end_reached
    ) {
      list.is_paginating = true;
    }
    return this.getSnapshot();
  }

  async setThreadComposerDraft(
    account: ComposerDraftAccountOwner,
    leaseId: string,
    rendererGeneration: string,
    roomId: string,
    rootEventId: string,
    document: ComposerDocument,
    revision: ComposerDraftRevision
  ): Promise<DesktopSnapshot> {
    this.requireComposerLease(
      account,
      { kind: "thread", room_id: roomId, root_event_id: rootEventId },
      leaseId,
      rendererGeneration
    );
    if (
      !this.canUseSyncedViews() ||
      this.snapshot.state.domain.session.homeserver !== account.homeserver ||
      this.snapshot.state.domain.session.user_id !== account.userId ||
      this.snapshot.state.domain.session.device_id !== account.deviceId ||
      !this.snapshot.state.domain.rooms.some((room) => room.room_id === roomId)
    ) {
      return this.getSnapshot();
    }

    const key = browserComposerDraftTargetKey({
      kind: "thread",
      room_id: roomId,
      root_event_id: rootEventId
    });
    if (
      compareComposerDraftRevisions(
        revision,
        this.threadComposerDraftRevisions.get(key) ?? COMPOSER_DRAFT_REVISION_ZERO
      ) <= 0
    ) {
      return this.getSnapshot();
    }
    this.threadComposerDraftRevisions.set(key, revision);
    if (document.inlines.length === 0) {
      this.threadComposerDrafts.delete(key);
    } else {
      this.threadComposerDrafts.set(key, structuredClone(document));
    }
    const thread = this.snapshot.state.ui.thread;
    if (
      thread.kind === "open" &&
      thread.room_id === roomId &&
      thread.root_event_id === rootEventId &&
      thread.composer &&
      compareComposerDraftRevisions(revision, thread.composer.draft_revision) > 0
    ) {
      thread.composer.document = structuredClone(document);
      thread.composer.draft = plainBodyFromDocument(document);
      thread.composer.draft_revision = revision;
    }
    return this.getSnapshot();
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
    const body = plainBodyFromDocument(document);
    this.requireComposerLease(
      account,
      { kind: "thread", room_id: roomId, root_event_id: rootEventId },
      leaseId,
      rendererGeneration
    );
    const replay = this.replaySubmission(submissionId);
    if (replay) return replay;
    const session = this.snapshot.state.domain.session;
    const thread = this.snapshot.state.ui.thread;
    if (
      session.kind !== "ready" ||
      !browserComposerAccountMatches(session, account) ||
      !session.user_id ||
      thread.kind !== "open" ||
      thread.room_id !== roomId ||
      thread.root_event_id !== rootEventId ||
      !thread.composer ||
      thread.composer.pending_transaction_id ||
      body.trim().length === 0
    ) {
      return {
        outcome: { rejected: { kind: "invalid" } },
        submissionId,
        transactionId: null,
        snapshot: await this.getSnapshot()
      };
    }

    this.preflightComposerDraftAcceptance(
      { kind: "thread", room_id: roomId, root_event_id: rootEventId },
      draftRevision
    );

    const transactionId = this.acceptSubmission(submissionId, thread.composer);
    this.terminalSubmission(thread.composer);
    this.acceptComposerDraftTarget(
      { kind: "thread", room_id: roomId, root_event_id: rootEventId },
      draftRevision
    );
    return {
      outcome: "accepted",
      submissionId,
      transactionId,
      snapshot: await this.getSnapshot()
    };
  }

  async submitSearch(query: string, scope: SearchScopeKind): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    const requestId = this.nextRequestId();
    const trimmed = query.trim();
    const minChars = searchMinChars(trimmed);
    if (trimmed.length > 0 && [...trimmed].length < minChars) {
      this.snapshot.state.domain.search = {
        kind: "tooShort",
        request_id: requestId,
        query: trimmed,
        scope,
        min_chars: minChars
      };
      return this.getSnapshot();
    }

    const results = search(trimmed, scope, this.snapshot);
    this.snapshot.state.domain.search = {
      kind: "results",
      request_id: requestId,
      query: trimmed,
      scope,
      results
    };
    return this.getSnapshot();
  }

  async closeSearch(): Promise<DesktopSnapshot> {
    this.snapshot.state.domain.search = { kind: "closed" };
    return this.getSnapshot();
  }

  async queryDirectory(query: DirectoryQuery): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    const requestId = this.nextRequestId();
    const normalizedQuery: DirectoryQuery = {
      term: query.term?.trim() ? query.term.trim() : null,
      server_name: query.server_name?.trim() ? query.server_name.trim() : null,
      limit: query.limit,
      since: query.since?.trim() ? query.since.trim() : null
    };
    this.snapshot.state.domain.directory.query = {
      kind: "querying",
      request_id: requestId,
      query: normalizedQuery
    };

    await Promise.resolve();

    if (
      this.snapshot.state.domain.directory.query.kind !== "querying" ||
      this.snapshot.state.domain.directory.query.request_id !== requestId
    ) {
      return this.getSnapshot();
    }
    const alias = "#public-demo:fake.local";
    this.snapshot.state.domain.directory.query = {
      kind: "results",
      request_id: requestId,
      query: normalizedQuery,
      rooms: [
        {
          room_id: "!public-demo:fake.local",
          canonical_alias: alias,
          room_type: null,
          name: "Public Demo Room",
          topic: "Synthetic browser directory result",
          avatar_url: null,
          joined_members: 3,
          world_readable: true,
          guest_can_join: false
        }
      ],
      next_batch: null
    };
    return this.getSnapshot();
  }

  async previewJoinTarget(
    roomIdOrAlias: string,
    viaServers: string[] = []
  ): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || roomIdOrAlias.trim().length === 0) {
      return this.getSnapshot();
    }

    const requestId = this.nextRequestId();
    const normalizedTarget = roomIdOrAlias.trim();
    const normalizedViaServers = viaServers
      .map((server) => server.trim())
      .filter((server) => server.length > 0);
    this.snapshot.state.domain.directory.preview = {
      kind: "loading",
      request_id: requestId,
      room_id_or_alias: normalizedTarget,
      via_servers: normalizedViaServers
    };

    await Promise.resolve();

    if (
      this.snapshot.state.domain.directory.preview.kind !== "loading" ||
      this.snapshot.state.domain.directory.preview.request_id !== requestId
    ) {
      return this.getSnapshot();
    }
    const label = normalizedTarget.replace(/^[#!]/, "").split(":")[0] ?? "";
    this.snapshot.state.domain.directory.preview = {
      kind: "ready",
      request_id: requestId,
      room_id_or_alias: normalizedTarget,
      via_servers: normalizedViaServers,
      room: {
        room_id: normalizedTarget.startsWith("!") ? normalizedTarget : "!previewed:fake.local",
        canonical_alias: normalizedTarget.startsWith("#") ? normalizedTarget : null,
        room_type: null,
        name: label,
        topic: null,
        joined_members: 3,
        joinability: "open",
        membership: "none"
      }
    };
    return this.getSnapshot();
  }

  async dismissDirectoryPreview(): Promise<DesktopSnapshot> {
    this.snapshot.state.domain.directory.preview = { kind: "closed" };
    return this.getSnapshot();
  }

  async joinDirectoryRoom(roomIdOrAlias: string, viaServers: string[] = []): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || roomIdOrAlias.trim().length === 0) {
      return this.getSnapshot();
    }

    const requestId = this.nextRequestId();
    const normalizedTarget = roomIdOrAlias.trim();
    const normalizedViaServers = viaServers
      .map((server) => server.trim())
      .filter((server) => server.length > 0);
    // The Rust reducer closes the preview when a join is requested; the fake
    // must not leave a dialog standing over a joining room.
    this.snapshot.state.domain.directory.preview = { kind: "closed" };
    this.snapshot.state.domain.directory.join = {
      kind: "joining",
      request_id: requestId,
      room_id_or_alias: normalizedTarget,
      via_servers: normalizedViaServers
    };

    await Promise.resolve();

    if (
      this.snapshot.state.domain.directory.join.kind !== "joining" ||
      this.snapshot.state.domain.directory.join.request_id !== requestId
    ) {
      return this.getSnapshot();
    }
    const roomId = `!joined-${this.snapshot.state.domain.rooms.length + 1}:fake.local`;
    const displayName = normalizedTarget.replace(/^[#!]/, "").split(":")[0] || "Public Room";
    const joinedRoom: RoomSummary = {
      room_id: roomId,
      display_name: displayName,
      display_label: displayName,
      original_display_label: displayName,
      avatar: null,
      is_dm: false,
      dm_user_ids: [],
      tags: emptyRoomTags(),
      unread_count: 0,
      parent_space_ids: [],
      dm_space_ids: [],
      is_encrypted: false
    };

    this.snapshot.state.domain.rooms = [...this.snapshot.state.domain.rooms, joinedRoom];
    this.snapshot.state.domain.directory.join = { kind: "idle" };
    this.refreshRoomListProjection();
    this.refreshSidebar();
    return this.selectRoom(roomId);
  }

  async joinRoom(roomId: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || roomId.trim().length === 0) {
      return this.getSnapshot();
    }

    const normalizedRoomId = roomId.trim();
    if (this.snapshot.state.domain.rooms.some((room) => room.room_id === normalizedRoomId)) {
      return this.selectRoom(normalizedRoomId);
    }

    const activeSpaceId = this.snapshot.state.ui.navigation.active_space_id;
    const joinedRoom: RoomSummary = {
      room_id: normalizedRoomId,
      display_name: normalizedRoomId,
      display_label: normalizedRoomId,
      original_display_label: normalizedRoomId,
      avatar: null,
      is_dm: false,
      dm_user_ids: [],
      tags: emptyRoomTags(),
      unread_count: 0,
      parent_space_ids: activeSpaceId ? [activeSpaceId] : [],
      dm_space_ids: [],
      is_encrypted: false
    };

    this.snapshot.state.domain.rooms = [...this.snapshot.state.domain.rooms, joinedRoom];
    if (activeSpaceId) {
      this.snapshot.state.domain.spaces = this.snapshot.state.domain.spaces.map((space) =>
        space.space_id === activeSpaceId && !space.child_room_ids.includes(normalizedRoomId)
          ? { ...space, child_room_ids: [...space.child_room_ids, normalizedRoomId] }
          : space
      );
    }
    this.refreshRoomListProjection();
    this.refreshSidebar();
    return this.selectRoom(normalizedRoomId);
  }

  async loadRoomSettings(roomId: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || !roomId.trim()) {
      return this.getSnapshot();
    }

    const normalizedRoomId = roomId.trim();
    this.snapshot.state.domain.room_management = {
      selected_room_id: normalizedRoomId,
      settings: this.roomSettingsSnapshot(normalizedRoomId),
      operation: { kind: "idle" }
    };
    return this.getSnapshot();
  }

  async loadSpaceMembers(spaceId: string, generation: number): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || !spaceId.trim()) {
      return this.getSnapshot();
    }

    const normalizedSpaceId = spaceId.trim();
    if (this.snapshot.state.ui.navigation.active_space_id !== normalizedSpaceId) {
      return this.getSnapshot();
    }
    const current = this.snapshot.state.domain.space_members;
    if (
      (current.selected_space_id !== null &&
        (current.selected_space_id !== normalizedSpaceId || current.generation !== generation)) ||
      current.operation.kind === "inviting" ||
      current.operation.kind === "cancellingInvite"
    ) {
      return this.getSnapshot();
    }
    const requestId = this.nextRequestId();
    this.snapshot.state.domain.space_members = {
      ...current,
      selected_space_id: normalizedSpaceId,
      generation,
      operation: {
        kind: "loading",
        request_id: requestId,
        space_id: normalizedSpaceId,
        generation
      }
    };

    await Promise.resolve();

    const active = this.snapshot.state.domain.space_members;
    if (
      this.snapshot.state.ui.navigation.active_space_id !== normalizedSpaceId ||
      active.selected_space_id !== normalizedSpaceId ||
      active.generation !== generation ||
      active.operation.kind !== "loading" ||
      active.operation.request_id !== requestId ||
      active.operation.space_id !== normalizedSpaceId ||
      active.operation.generation !== generation
    ) {
      return this.getSnapshot();
    }
    this.snapshot.state.domain.space_members = {
      ...active,
      operation: { kind: "idle" }
    };
    return this.getSnapshot();
  }

  async inviteUserToSpace(
    spaceId: string,
    userId: string,
    generation: number
  ): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || !spaceId.trim() || !userId.trim()) {
      return this.getSnapshot();
    }

    const normalizedSpaceId = spaceId.trim();
    const normalizedUserId = userId.trim();
    const current = this.snapshot.state.domain.space_members;
    const childOnly = current.child_room_only.find(
      (entry) => entry.user_id === normalizedUserId
    );
    if (
      current.selected_space_id !== normalizedSpaceId ||
      current.generation !== generation ||
      !childOnly ||
      current.operation.kind === "inviting" ||
      current.operation.kind === "cancellingInvite"
    ) {
      return this.getSnapshot();
    }

    const requestId = this.nextRequestId();
    const pendingEntry: SpaceMemberEntry = {
      ...childOnly,
      membership: "space_invited",
      invite_pending: true
    };
    const inviting: SpaceMembersState = {
      ...current,
      space_invited: [...current.space_invited, pendingEntry].sort(compareSpaceMemberEntries),
      child_room_only: current.child_room_only.filter(
        (entry) => entry.user_id !== normalizedUserId
      ),
      operation: {
        kind: "inviting",
        request_id: requestId,
        space_id: normalizedSpaceId,
        user_id: normalizedUserId,
        generation
      }
    };
    this.snapshot.state.domain.space_members = inviting;

    if (this.spaceMemberInviteOutcome === "pending") {
      return this.getSnapshot();
    }

    if (this.spaceMemberInviteOutcome === "success") {
      this.snapshot.state.domain.space_members = {
        ...this.snapshot.state.domain.space_members,
        space_invited: this.snapshot.state.domain.space_members.space_invited.map((entry) =>
          entry.user_id === normalizedUserId ? { ...entry, invite_pending: false } : entry
        ),
        operation: { kind: "idle" }
      };
    } else {
      this.snapshot.state.domain.space_members = {
        ...this.snapshot.state.domain.space_members,
        space_invited: this.snapshot.state.domain.space_members.space_invited.filter(
          (entry) => entry.user_id !== normalizedUserId
        ),
        child_room_only: [
          ...this.snapshot.state.domain.space_members.child_room_only,
          {
            ...childOnly,
            membership: "child_room_only" as const,
            invite_pending: false
          }
        ].sort(compareSpaceMemberEntries),
        operation: {
          kind: "failed",
          request_id: requestId,
          space_id: normalizedSpaceId,
          user_id: normalizedUserId,
          generation,
          failureKind: "sdk"
        }
      };
    }

    return this.getSnapshot();
  }

  async cancelSpaceInvite(
    spaceId: string,
    userId: string,
    generation: number
  ): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || !spaceId.trim() || !userId.trim()) {
      return this.getSnapshot();
    }

    const normalizedSpaceId = spaceId.trim();
    const normalizedUserId = userId.trim();
    const current = this.snapshot.state.domain.space_members;
    if (
      current.selected_space_id !== normalizedSpaceId ||
      current.generation !== generation
    ) {
      return this.getSnapshot();
    }
    const cancellationContextIsRetryable =
      current.operation.kind === "idle" ||
      (current.operation.kind === "failed" &&
        current.operation.space_id === normalizedSpaceId &&
        current.operation.user_id === normalizedUserId &&
        current.operation.generation === generation);
    if (!cancellationContextIsRetryable) {
      return this.getSnapshot();
    }
    const invitedEntry = current.space_invited.find(
      (entry) => entry.user_id === normalizedUserId
    );
    if (!invitedEntry) {
      return this.getSnapshot();
    }

    const requestId = this.nextRequestId();
    this.snapshot.state.domain.space_members = {
      ...current,
      operation: {
        kind: "cancellingInvite",
        request_id: requestId,
        space_id: normalizedSpaceId,
        user_id: normalizedUserId,
        generation
      }
    };

    const cancellationOutcome =
      this.spaceMemberInviteCancellationOutcomes.shift() ??
      this.spaceMemberInviteCancellationOutcome;
    if (cancellationOutcome === "pending") {
      return this.getSnapshot();
    }

    const active = this.snapshot.state.domain.space_members;
    if (cancellationOutcome === "success") {
      this.snapshot.state.domain.space_members = {
        ...active,
        space_invited: active.space_invited.filter(
          (entry) => entry.user_id !== normalizedUserId
        ),
        operation: { kind: "idle" }
      };
    } else if (cancellationOutcome === "notInvited") {
      this.snapshot.state.domain.space_members = {
        ...active,
        space_invited: active.space_invited.filter(
          (entry) => entry.user_id !== normalizedUserId
        ),
        space_joined: [
          ...active.space_joined,
          {
            ...invitedEntry,
            membership: "space_joined" as const,
            invite_pending: false
          }
        ].sort(compareSpaceMemberEntries),
        operation: { kind: "idle" }
      };
    } else {
      this.snapshot.state.domain.space_members = {
        ...active,
        operation: {
          kind: "failed",
          request_id: requestId,
          space_id: normalizedSpaceId,
          user_id: normalizedUserId,
          generation,
          failureKind: "sdk"
        }
      };
    }

    return this.getSnapshot();
  }

  async queryMentionCandidates(
    roomId: string,
    surface: MentionSurface,
    query: string
  ): Promise<void> {
    const normalizedRoomId = roomId.trim();
    if (!this.canUseSyncedViews() || !normalizedRoomId) {
      return;
    }
    const previous = this.snapshot.state.domain.mention_candidates.targets.find(
      (target) => target.room_id === normalizedRoomId && target.surface === surface
    );
    const settings =
      this.snapshot.state.domain.room_management.selected_room_id === normalizedRoomId &&
      this.snapshot.state.domain.room_management.settings?.room_id === normalizedRoomId
        ? this.snapshot.state.domain.room_management.settings
        : null;
    const normalizedQuery = query.trim().toLocaleLowerCase();
    const candidates = (settings?.members ?? [])
      .filter((member) =>
        [member.display_label, member.original_display_label, member.user_id].some((value) =>
          value.toLocaleLowerCase().includes(normalizedQuery)
        )
      )
      .map((member) => ({
        user_id: member.user_id,
        display_label: member.display_label || null,
        original_display_label: member.original_display_label || null,
        avatar: member.avatar_url
          ? {
              mxc_uri: member.avatar_url,
              thumbnail: { kind: "notRequested" as const }
            }
          : null,
        membership: "joined" as const
      }));
    const target = {
      room_id: normalizedRoomId,
      generation: (previous?.generation ?? 0) + 1,
      request_id: this.nextRequestId(),
      query,
      surface,
      completeness: settings ? ("complete" as const) : ("partial" as const),
      candidates,
      room_mention_allowed: "unknown" as const,
      failure_kind: null
    };
    this.snapshot.state.domain.mention_candidates.targets =
      this.snapshot.state.domain.mention_candidates.targets
        .filter(
          (candidateTarget) =>
            candidateTarget.room_id !== normalizedRoomId ||
            candidateTarget.surface !== surface
        )
        .concat(target);
  }

  async forceNewOutboundSession(_roomId: string): Promise<EncryptionDebugOperationOutcome> {
    return "completed";
  }

  async shareIndex0RoomKey(_roomId: string): Promise<EncryptionDebugOperationOutcome> {
    return "completed";
  }

  async resendIndex0RoomKey(_roomId: string): Promise<EncryptionDebugOperationOutcome> {
    return "completed";
  }

  async reshareRoomKey(_roomId: string): Promise<RoomKeyReshareOutcome> {
    return { kind: "sent", request_count: 1, recipient_count: 1 };
  }

  async repairRoomTimeline(_roomId: string): Promise<DesktopSnapshot> {
    return this.getSnapshot();
  }

  async updateRoomSetting(
    roomId: string,
    change: RoomSettingChange
  ): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || !roomId.trim()) {
      return this.getSnapshot();
    }

    const normalizedRoomId = roomId.trim();
    const settings =
      this.snapshot.state.domain.room_management.settings?.room_id === normalizedRoomId
        ? this.snapshot.state.domain.room_management.settings
        : this.roomSettingsSnapshot(normalizedRoomId);
    const requestId = this.nextRequestId();

    if (!settings.permissions.can_edit_settings) {
      this.snapshot.state.domain.room_management = {
        selected_room_id: normalizedRoomId,
        settings,
        operation: {
          kind: "failed",
          request_id: requestId,
          room_id: normalizedRoomId,
          operation: "settings",
          failureKind: "forbidden"
        }
      };
      return this.getSnapshot();
    }

    this.snapshot.state.domain.room_management = {
      selected_room_id: normalizedRoomId,
      settings,
      operation: {
        kind: "pending",
        request_id: requestId,
        room_id: normalizedRoomId,
        operation: "settings"
      }
    };

    await Promise.resolve();

    const operation = this.snapshot.state.domain.room_management.operation;
    if (
      operation.kind !== "pending" ||
      operation.request_id !== requestId ||
      operation.room_id !== normalizedRoomId ||
      operation.operation !== "settings"
    ) {
      return this.getSnapshot();
    }
    const updated = applyRoomSettingChange(settings, change);
    this.snapshot.state.domain.room_management = {
      selected_room_id: normalizedRoomId,
      settings: updated,
      operation: { kind: "idle" }
    };
    this.snapshot.state.domain.rooms = this.snapshot.state.domain.rooms.map((room) =>
      room.room_id === normalizedRoomId && "name" in change
        ? {
            ...room,
            display_name: change.name ?? room.display_name,
            display_label: room.is_dm ? room.display_label : change.name ?? room.display_label
          }
        : room
    );
    this.refreshRoomListProjection();
    this.refreshSidebar();
    return this.getSnapshot();
  }

  async moderateRoomMember(
    roomId: string,
    targetUserId: string,
    action: RoomModerationAction,
    reason: string | null = null
  ): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || !roomId.trim() || !targetUserId.trim()) {
      return this.getSnapshot();
    }
    void reason;

    const normalizedRoomId = roomId.trim();
    const settings =
      this.snapshot.state.domain.room_management.settings?.room_id === normalizedRoomId
        ? this.snapshot.state.domain.room_management.settings
        : this.roomSettingsSnapshot(normalizedRoomId);
    const requestId = this.nextRequestId();

    if (!roomModerationAllowed(settings.permissions, action)) {
      this.snapshot.state.domain.room_management = {
        selected_room_id: normalizedRoomId,
        settings,
        operation: {
          kind: "failed",
          request_id: requestId,
          room_id: normalizedRoomId,
          operation: "moderation",
          failureKind: "forbidden"
        }
      };
      return this.getSnapshot();
    }

    this.snapshot.state.domain.room_management = {
      selected_room_id: normalizedRoomId,
      settings,
      operation: {
        kind: "pending",
        request_id: requestId,
        room_id: normalizedRoomId,
        operation: "moderation"
      }
    };

    await Promise.resolve();

    const operation = this.snapshot.state.domain.room_management.operation;
    if (
      operation.kind !== "pending" ||
      operation.request_id !== requestId ||
      operation.room_id !== normalizedRoomId ||
      operation.operation !== "moderation"
    ) {
      return this.getSnapshot();
    }
    const updatedSettings =
      action === "unban"
        ? settings
        : {
            ...settings,
            members: settings.members.filter((member) => member.user_id !== targetUserId.trim())
          };
    this.snapshot.state.domain.room_management = {
      selected_room_id: normalizedRoomId,
      settings: updatedSettings,
      operation: { kind: "idle" }
    };
    return this.getSnapshot();
  }

  async updateRoomMemberRole(
    roomId: string,
    targetUserId: string,
    powerLevel: number
  ): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || !roomId.trim() || !targetUserId.trim()) {
      return this.getSnapshot();
    }

    const normalizedRoomId = roomId.trim();
    const normalizedTargetUserId = targetUserId.trim();
    const settings =
      this.snapshot.state.domain.room_management.settings?.room_id === normalizedRoomId
        ? this.snapshot.state.domain.room_management.settings
        : this.roomSettingsSnapshot(normalizedRoomId);
    const requestId = this.nextRequestId();

    if (!settings.permissions.can_edit_roles) {
      this.snapshot.state.domain.room_management = {
        selected_room_id: normalizedRoomId,
        settings,
        operation: {
          kind: "failed",
          request_id: requestId,
          room_id: normalizedRoomId,
          operation: "roles",
          failureKind: "forbidden"
        }
      };
      return this.getSnapshot();
    }

    this.snapshot.state.domain.room_management = {
      selected_room_id: normalizedRoomId,
      settings,
      operation: {
        kind: "pending",
        request_id: requestId,
        room_id: normalizedRoomId,
        operation: "roles"
      }
    };

    await Promise.resolve();

    const operation = this.snapshot.state.domain.room_management.operation;
    if (
      operation.kind !== "pending" ||
      operation.request_id !== requestId ||
      operation.room_id !== normalizedRoomId ||
      operation.operation !== "roles"
    ) {
      return this.getSnapshot();
    }
    const updatedSettings = {
      ...settings,
      members: settings.members.map((member) =>
        member.user_id === normalizedTargetUserId
          ? {
              ...member,
              power_level: powerLevel,
              role: roomMemberRoleFromPowerLevel(powerLevel)
            }
          : member
      )
    };
    this.snapshot.state.domain.room_management = {
      selected_room_id: normalizedRoomId,
      settings: updatedSettings,
      operation: { kind: "idle" }
    };
    return this.getSnapshot();
  }

  async updateSpaceMemberRole(
    spaceId: string,
    userId: string,
    generation: number,
    expectedPowerLevelsRevision: string | null,
    expectedPowerLevel: number,
    powerLevel: number,
    confirmed: boolean
  ): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || !spaceId.trim() || !userId.trim()) {
      return this.getSnapshot();
    }

    const normalizedSpaceId = spaceId.trim();
    const normalizedUserId = userId.trim();
    const current = this.snapshot.state.domain.space_members;
    const target = current.space_joined.find((entry) => entry.user_id === normalizedUserId);
    const retryable =
      current.operation.kind === "idle" ||
      (current.operation.kind === "roleUpdateFailed" &&
        current.operation.space_id === normalizedSpaceId &&
        current.operation.user_id === normalizedUserId &&
        current.operation.generation === generation);
    if (
      current.selected_space_id !== normalizedSpaceId ||
      current.generation !== generation ||
      !retryable ||
      !current.can_edit_roles ||
      !target ||
      target.membership !== "space_joined" ||
      target.power_level !== expectedPowerLevel ||
      current.power_levels_revision !== expectedPowerLevelsRevision
    ) {
      return this.getSnapshot();
    }
    const option = target.role_options.find((candidate) => candidate.power_level === powerLevel);
    if (!option || (option.requires_confirmation && !confirmed)) {
      return this.getSnapshot();
    }

    const requestId = this.nextRequestId();
    this.snapshot.state.domain.space_members = {
      ...current,
      operation: {
        kind: "updatingRole",
        request_id: requestId,
        space_id: normalizedSpaceId,
        user_id: normalizedUserId,
        generation,
        expected_power_levels_revision: expectedPowerLevelsRevision,
        expected_power_level: expectedPowerLevel,
        power_level: powerLevel,
        confirmed
      }
    };

    const outcome =
      this.spaceMemberRoleUpdateOutcomes.shift() ?? this.spaceMemberRoleUpdateOutcome;
    if (outcome === "pending") {
      return this.getSnapshot();
    }
    if (outcome !== "success") {
      this.snapshot.state.domain.space_members = {
        ...this.snapshot.state.domain.space_members,
        power_levels_revision:
          outcome === "stale"
            ? `revision-${requestId}`
            : this.snapshot.state.domain.space_members.power_levels_revision,
        operation: {
          kind: "roleUpdateFailed",
          request_id: requestId,
          space_id: normalizedSpaceId,
          user_id: normalizedUserId,
          generation,
          expected_power_levels_revision: expectedPowerLevelsRevision,
          expected_power_level: expectedPowerLevel,
          power_level: powerLevel,
          sent_revision: null,
          failureKind: outcome
        }
      };
      return this.getSnapshot();
    }

    const nextRevision = `revision-${requestId}`;
    const nextProjection: SpaceMembersState = {
      ...this.snapshot.state.domain.space_members,
      power_levels_revision: nextRevision,
      space_joined: this.snapshot.state.domain.space_members.space_joined.map((entry) =>
        entry.user_id === normalizedUserId
          ? {
              ...entry,
              power_level: powerLevel,
              role: roomMemberRoleFromPowerLevel(powerLevel),
              role_options: spaceMemberRoleOptionsForPowerLevel(powerLevel)
            }
          : entry
      ),
      operation: { kind: "idle" }
    };
    this.snapshot.state.domain.space_members = nextProjection;
    return this.getSnapshot();
  }

  async createRoom(request: CreateRoomRequest): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    const count = this.snapshot.state.domain.rooms.length + 1;
    const newRoomId = `!local-room-${count}:fake.local`;
    const name = request.name.trim();
    const parentSpaceId = request.parentSpace?.spaceId ?? null;
    const newRoom: RoomSummary = {
      room_id: newRoomId,
      display_name: name,
      display_label: name,
      original_display_label: name,
      avatar: null,
      is_dm: false,
      dm_user_ids: [],
      tags: emptyRoomTags(),
      unread_count: 0,
      parent_space_ids: parentSpaceId ? [parentSpaceId] : [],
      dm_space_ids: [],
      is_encrypted: request.visibility === "public" ? false : request.encrypted
    };
    this.snapshot.state.domain.rooms = [...this.snapshot.state.domain.rooms, newRoom];
    this.refreshRoomListProjection();
    this.refreshSidebar();
    await this.selectRoom(newRoomId);
    return this.getSnapshot();
  }

  async createSpace(name: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    const count = this.snapshot.state.domain.spaces.length + 1;
    const newSpaceId = `!local-space-${count}:fake.local`;
    const newSpace: SpaceSummary = {
      space_id: newSpaceId,
      display_name: name,
      avatar: null,
      child_room_ids: []
    };
    this.snapshot.state.domain.spaces = [...this.snapshot.state.domain.spaces, newSpace];
    await this.selectSpace(newSpaceId);
    return this.getSnapshot();
  }

  async setSpaceChild(spaceId: string, childRoomId: string, viaServer: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }
    void viaServer;

    this.snapshot.state.domain.spaces = this.snapshot.state.domain.spaces.map((space) =>
      space.space_id === spaceId
        ? {
            ...space,
            child_room_ids: space.child_room_ids.includes(childRoomId)
              ? space.child_room_ids
              : [...space.child_room_ids, childRoomId]
          }
        : space
    );
    this.snapshot.state.domain.rooms = this.snapshot.state.domain.rooms.map((room) =>
      room.room_id === childRoomId
        ? {
            ...room,
            parent_space_ids: room.parent_space_ids.includes(spaceId)
              ? room.parent_space_ids
              : [...room.parent_space_ids, spaceId]
          }
        : room
    );
    this.refreshRoomListProjection();
    this.refreshSidebar();
    return this.getSnapshot();
  }

  async acceptInvite(roomId: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    const invite = this.snapshot.state.domain.invites.find((candidate) => candidate.room_id === roomId);
    if (!invite) {
      return this.getSnapshot();
    }

    const joinedRoom: RoomSummary = {
      room_id: roomId,
      display_name: invite.display_name,
      display_label: invite.display_name,
      original_display_label: invite.display_name,
      avatar: invite.avatar,
      is_dm: invite.is_dm,
      dm_user_ids: [],
      tags: emptyRoomTags(),
      unread_count: 0,
      parent_space_ids: [],
      dm_space_ids: [],
      is_encrypted: false
    };
    this.snapshot.state.domain.invites = this.snapshot.state.domain.invites.filter(
      (candidate) => candidate.room_id !== roomId
    );
    this.snapshot.state.domain.rooms = [...this.snapshot.state.domain.rooms, joinedRoom];
    this.refreshRoomListProjection();
    this.refreshSidebar();
    await this.selectRoom(roomId);
    return this.getSnapshot();
  }

  async declineInvite(roomId: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    this.snapshot.state.domain.invites = this.snapshot.state.domain.invites.filter(
      (candidate) => candidate.room_id !== roomId
    );
    this.refreshRoomListProjection();
    return this.getSnapshot();
  }

  async startDirectMessage(userId: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    const trimmedUserId = userId.trim();
    if (!trimmedUserId) {
      return this.getSnapshot();
    }

    // Same get-or-create-and-open contract as the real backend (#368): an
    // existing one-to-one DM with this target is reused, never duplicated.
    const existing = this.snapshot.state.domain.rooms.find(
      (room) =>
        room.is_dm && room.dm_user_ids.length === 1 && room.dm_user_ids[0] === trimmedUserId
    );
    if (existing) {
      await this.selectRoom(existing.room_id);
      return this.getSnapshot();
    }

    const count = this.snapshot.state.domain.rooms.filter((room) => room.is_dm).length + 1;
    const newRoomId = `!local-dm-${count}:fake.local`;
    const newRoom: RoomSummary = {
      room_id: newRoomId,
      display_name: trimmedUserId,
      display_label: trimmedUserId,
      original_display_label: trimmedUserId,
      avatar: null,
      is_dm: true,
      dm_user_ids: [trimmedUserId],
      tags: emptyRoomTags(),
      unread_count: 0,
      parent_space_ids: [],
      dm_space_ids: [],
      is_encrypted: false
    };
    this.snapshot.state.domain.rooms = [...this.snapshot.state.domain.rooms, newRoom];
    this.refreshRoomListProjection();
    this.refreshSidebar();
    await this.selectRoom(newRoomId);
    return this.getSnapshot();
  }

  async inviteUser(roomId: string, userId: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || !roomId.trim() || !userId.trim()) {
      return this.getSnapshot();
    }

    return this.getSnapshot();
  }

  async openInviteWorkflow(roomId: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || !roomId.trim()) {
      return this.getSnapshot();
    }
    const workflow = this.snapshot.state.domain.invite_workflow ?? defaultInviteWorkflowState();
    const scopePlan = buildFakeInviteScopePlan(this.snapshot, roomId);
    const selectedScope = workflow.selected_scope && scopePlan?.options.some(
      (option) => inviteScopeKey(option.scope) === inviteScopeKey(workflow.selected_scope!)
    )
      ? workflow.selected_scope
      : scopePlan?.default_scope ?? null;
    this.snapshot.state.domain.invite_workflow = {
      ...workflow,
      query: {
        ...workflow.query,
        room_id: roomId
      },
      scope_plan: scopePlan,
      selected_scope: selectedScope,
      history_policy: buildFakeInviteHistoryPolicy(this.snapshot, roomId)
    };
    return this.getSnapshot();
  }

  async closeInviteWorkflow(): Promise<DesktopSnapshot> {
    this.snapshot.state.domain.invite_workflow = defaultInviteWorkflowState();
    return this.getSnapshot();
  }

  async searchInviteTargets(roomId: string, query: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || !roomId.trim()) {
      return this.getSnapshot();
    }
    const workflow = this.snapshot.state.domain.invite_workflow ?? defaultInviteWorkflowState();
    const scopePlan = buildFakeInviteScopePlan(this.snapshot, roomId);
    const selectedScope = workflow.selected_scope && scopePlan?.options.some(
      (option) => inviteScopeKey(option.scope) === inviteScopeKey(workflow.selected_scope!)
    )
      ? workflow.selected_scope
      : scopePlan?.default_scope ?? null;
    this.snapshot.state.domain.invite_workflow = {
      ...workflow,
      query: buildFakeInviteTargetQuery(this.snapshot, roomId, query),
      scope_plan: scopePlan,
      selected_scope: selectedScope,
      history_policy: buildFakeInviteHistoryPolicy(this.snapshot, roomId)
    };
    return this.getSnapshot();
  }

  async setInviteScope(roomId: string, scope: InviteScopeSelection): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || !roomId.trim()) {
      return this.getSnapshot();
    }
    const workflow = this.snapshot.state.domain.invite_workflow ?? defaultInviteWorkflowState();
    const valid = workflow.scope_plan?.options.some(
      (option) => inviteScopeKey(option.scope) === inviteScopeKey(scope)
    );
    if (workflow.query.room_id === roomId && valid) {
      this.snapshot.state.domain.invite_workflow = {
        ...workflow,
        selected_scope: scope
      };
    }
    return this.getSnapshot();
  }

  async selectInviteTarget(roomId: string, userId: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || !roomId.trim() || !userId.trim()) {
      return this.getSnapshot();
    }
    const workflow = this.snapshot.state.domain.invite_workflow ?? defaultInviteWorkflowState();
    const candidate = [
      ...workflow.query.candidates,
      ...(workflow.query.explicit_user_id ? [workflow.query.explicit_user_id] : [])
    ].find((entry) => entry.user_id === userId && entry.status === "selectable");
    if (!candidate || workflow.selected_targets.some((target) => target.user_id === userId)) {
      return this.getSnapshot();
    }
    this.snapshot.state.domain.invite_workflow = {
      ...workflow,
      selected_targets: [
        ...workflow.selected_targets,
        {
          user_id: candidate.user_id,
          display_label: candidate.display_label,
          avatar: candidate.avatar
        }
      ],
      query: buildFakeInviteTargetQuery(this.snapshot, roomId, workflow.query.query)
    };
    return this.getSnapshot();
  }

  async removeInviteTarget(userId: string): Promise<DesktopSnapshot> {
    const workflow = this.snapshot.state.domain.invite_workflow ?? defaultInviteWorkflowState();
    const roomId = workflow.query.room_id;
    this.snapshot.state.domain.invite_workflow = {
      ...workflow,
      selected_targets: workflow.selected_targets.filter((target) => target.user_id !== userId)
    };
    if (roomId) {
      this.snapshot.state.domain.invite_workflow.query = buildFakeInviteTargetQuery(
        this.snapshot,
        roomId,
        workflow.query.query
      );
    }
    return this.getSnapshot();
  }

  async inviteTargets(
    roomId: string,
    userIds: string[],
    scope: InviteScopeSelection
  ): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || !roomId.trim() || userIds.length === 0) {
      return this.getSnapshot();
    }
    const requestId = this.nextRequestId();
    const workflow = this.snapshot.state.domain.invite_workflow ?? defaultInviteWorkflowState();
    const results = userIds.flatMap((userId) => {
      const scopedResults = [];
      if (scope.kind === "parentSpaceAndRoom") {
        const alreadyInSpace = fakeRoomHasMember(this.snapshot, scope.space_id, userId);
        scopedResults.push({
          user_id: userId,
          destination: { kind: "space" as const, space_id: scope.space_id },
          kind: alreadyInSpace ? ("alreadyInSpace" as const) : ("invited" as const),
          message: alreadyInSpace ? INVITE_ALREADY_IN_SPACE_MESSAGE : null
        });
      }
      scopedResults.push({
        user_id: userId,
        destination: { kind: "room" as const, room_id: roomId },
        kind: "invited" as const,
        message: null
      });
      return scopedResults;
    });
    this.snapshot.state.domain.invite_workflow = {
      ...workflow,
      selected_targets: [],
      operation: {
        kind: "completed",
        request_id: requestId,
        room_id: roomId,
        results,
        notice: results.some((result) => result.kind === "alreadyInSpace")
          ? INVITE_ALREADY_IN_SPACE_MESSAGE
          : null
      }
    };
    return this.getSnapshot();
  }

  async setRoomTag(
    roomId: string,
    tag: RoomTagKind,
    order: number | null = null
  ): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    this.snapshot.state.domain.rooms = this.snapshot.state.domain.rooms.map((room) =>
      room.room_id === roomId
        ? {
            ...room,
            tags:
              tag === "favourite"
                ? {
                    favourite: { order: order == null ? null : String(order) },
                    low_priority: null
                  }
                : {
                    favourite: null,
                    low_priority: { order: order == null ? null : String(order) }
                  }
          }
        : room
    );
    this.refreshRoomListProjection();
    this.refreshSidebar();
    return this.getSnapshot();
  }

  async removeRoomTag(roomId: string, tag: RoomTagKind): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    this.snapshot.state.domain.rooms = this.snapshot.state.domain.rooms.map((room) =>
      room.room_id === roomId
        ? {
            ...room,
            tags: {
              ...room.tags,
              ...(tag === "favourite" ? { favourite: null } : { low_priority: null })
            }
          }
        : room
    );
    this.refreshRoomListProjection();
    this.refreshSidebar();
    return this.getSnapshot();
  }

  async pinEvent(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || !roomId.trim() || !eventId.trim() || !this.hasRoom(roomId)) {
      return this.getSnapshot();
    }

    const entry = this.snapshot.state.domain.room_interactions[roomId] ?? {
      pinned_events: [],
      pin_operation: { kind: "idle" as const },
      encryption_debug_operation: { state: "idle" as const }
    };
    const alreadyPinned = entry.pinned_events.some((event) => event.event_id === eventId);
    this.snapshot.state.domain.room_interactions = {
      ...this.snapshot.state.domain.room_interactions,
      [roomId]: {
        pinned_events: alreadyPinned
          ? entry.pinned_events
          : [
              ...entry.pinned_events,
              {
                event_id: eventId,
                sender: null,
                sender_label: null,
                body_preview: null,
                redacted: false,
                timestamp_ms: null,
                state: "ready",
                thread_root_event_id: null
              }
            ],
        pin_operation: { kind: "idle" },
        encryption_debug_operation: { state: "idle" }
      }
    };
    return this.getSnapshot();
  }

  async unpinEvent(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || !roomId.trim() || !eventId.trim() || !this.hasRoom(roomId)) {
      return this.getSnapshot();
    }

    const entry = this.snapshot.state.domain.room_interactions[roomId] ?? {
      pinned_events: [],
      pin_operation: { kind: "idle" as const },
      encryption_debug_operation: { state: "idle" as const }
    };
    this.snapshot.state.domain.room_interactions = {
      ...this.snapshot.state.domain.room_interactions,
      [roomId]: {
        pinned_events: entry.pinned_events.filter((event) => event.event_id !== eventId),
        pin_operation: { kind: "idle" },
        encryption_debug_operation: { state: "idle" }
      }
    };
    return this.getSnapshot();
  }

  async openActivity(): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    const currentActivity = this.snapshot.state.domain.activity;
    if (currentActivity.kind !== "closed") {
      return this.getSnapshot();
    }
    const selectedTab = currentActivity.last_selected_tab ?? "recent";

    const requestId = this.nextRequestId();
    this.snapshot.state.domain.activity = {
      kind: "opening",
      request_id: requestId,
      tab: selectedTab
    };

    await Promise.resolve();

    if (
      this.snapshot.state.domain.activity.kind !== "opening" ||
      this.snapshot.state.domain.activity.request_id !== requestId
    ) {
      return this.getSnapshot();
    }
    const streams = createActivityStreams(
      false,
      this.snapshot.state.domain.profile.users,
      this.snapshot.state.domain.room_notification_settings
    );
    this.snapshot.state.domain.activity = {
      kind: "open",
      active_tab: selectedTab,
      recent: streams.recent,
      unread: streams.unread,
      mark_read: { kind: "idle" }
    };
    return this.getSnapshot();
  }

  private refreshActivityStreams(): void {
    const activity = this.snapshot.state.domain.activity;
    if (activity.kind !== "open") {
      return;
    }
    const streams = createActivityStreams(
      false,
      this.snapshot.state.domain.profile.users,
      this.snapshot.state.domain.room_notification_settings
    );
    this.snapshot.state.domain.activity = {
      ...activity,
      recent: streams.recent,
      unread: streams.unread
    };
  }

  async closeActivity(): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    const activity = this.snapshot.state.domain.activity;
    const lastSelectedTab =
      activity.kind === "open"
        ? activity.active_tab
        : activity.kind === "opening"
          ? activity.tab
          : activity.last_selected_tab ?? "recent";
    this.snapshot.state.domain.activity = {
      kind: "closed",
      last_selected_tab: lastSelectedTab
    };
    return this.getSnapshot();
  }

  async setActivityTab(tab: ActivityTab): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || this.snapshot.state.domain.activity.kind !== "open") {
      return this.getSnapshot();
    }

    this.snapshot.state.domain.activity.active_tab = tab;
    return this.getSnapshot();
  }

  async paginateActivity(
    tab: ActivityTab,
    cursor: string | null = null
  ): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || this.snapshot.state.domain.activity.kind !== "open") {
      return this.getSnapshot();
    }

    const normalizedCursor = cursor?.trim() ? cursor.trim() : null;
    if (tab !== "recent" || normalizedCursor === null) {
      return this.getSnapshot();
    }

    const existingEventIds = new Set(
      this.snapshot.state.domain.activity.recent.rows.map((row) => row.event_id)
    );
    const spacesById = new Map(spaces.map((space) => [space.space_id, space]));
    const olderRows = activityRows(
      backwardTimelineMessages,
      new Set(),
      this.snapshot.state.domain.profile.users,
      spacesById,
      this.snapshot.state.domain.room_notification_settings
    )
      .filter((row) => !existingEventIds.has(row.event_id))
      .map((row) => ({ ...row, unread: false, highlight: false }));
    this.snapshot.state.domain.activity.recent = {
      rows: [...this.snapshot.state.domain.activity.recent.rows, ...olderRows],
      next_batch: null,
      resolution: this.snapshot.state.domain.activity.recent.resolution
    };
    return this.getSnapshot();
  }

  async retryActivityResolution(): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || this.snapshot.state.domain.activity.kind !== "open") {
      return this.getSnapshot();
    }
    const unread = this.snapshot.state.domain.activity.unread;
    unread.resolution = { kind: "resolving", generation: this.nextRequestId(), unresolved_room_count: unread.rows.filter((row) => row.kind === "roomUnread").length };
    return this.getSnapshot();
  }

  async markActivityRead(target: ActivityMarkReadTarget): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || this.snapshot.state.domain.activity.kind !== "open") {
      return this.getSnapshot();
    }

    const requestId = this.nextRequestId();
    this.snapshot.state.domain.activity.mark_read = {
      kind: "pending",
      request_id: requestId,
      target
    };

    await Promise.resolve();

    const activity = this.snapshot.state.domain.activity;
    if (
      activity.kind !== "open" ||
      activity.mark_read.kind !== "pending" ||
      activity.mark_read.request_id !== requestId
    ) {
      return this.getSnapshot();
    }
    if (target.kind === "all") {
      this.snapshot.state.domain.activity.unread = {
        rows: [],
        next_batch: null,
        resolution: { kind: "idle" }
      };
      this.snapshot.state.domain.rooms = this.snapshot.state.domain.rooms.map((room) => ({
        ...room,
        unread_count: 0
      }));
    } else {
      this.snapshot.state.domain.activity.unread = {
        ...this.snapshot.state.domain.activity.unread,
        rows: this.snapshot.state.domain.activity.unread.rows.filter(
          (row) => row.room_id !== target.room_id
        )
      };
      this.snapshot.state.domain.rooms = this.snapshot.state.domain.rooms.map((room) =>
        room.room_id === target.room_id ? { ...room, unread_count: 0 } : room
      );
    }
    this.snapshot.state.domain.activity.mark_read = { kind: "idle" };
    return this.getSnapshot();
  }

  async setComposerDraft(
    account: ComposerDraftAccountOwner,
    leaseId: string,
    rendererGeneration: string,
    roomId: string,
    document: ComposerDocument,
    revision: ComposerDraftRevision
  ): Promise<DesktopSnapshot> {
    this.requireComposerLease(
      account,
      { kind: "main", room_id: roomId },
      leaseId,
      rendererGeneration
    );
    if (
      !this.canUseSyncedViews() ||
      this.snapshot.state.domain.session.homeserver !== account.homeserver ||
      this.snapshot.state.domain.session.user_id !== account.userId ||
      this.snapshot.state.domain.session.device_id !== account.deviceId ||
      !this.snapshot.state.domain.rooms.some((room) => room.room_id === roomId)
    ) {
      return this.getSnapshot();
    }

    if (
      compareComposerDraftRevisions(
        revision,
        this.composerDraftRevisions.get(roomId) ?? COMPOSER_DRAFT_REVISION_ZERO
      ) <= 0
    ) {
      return this.getSnapshot();
    }
    this.composerDraftRevisions.set(roomId, revision);
    if (document.inlines.length === 0) {
      this.composerDrafts.delete(roomId);
    } else {
      this.composerDrafts.set(roomId, structuredClone(document));
    }
    if (this.snapshot.state.ui.timeline.room_id === roomId) {
      this.snapshot.state.ui.timeline.composer.document = structuredClone(document);
      this.snapshot.state.ui.timeline.composer.draft = plainBodyFromDocument(document);
      this.snapshot.state.ui.timeline.composer.draft_revision = revision;
    }
    return this.getSnapshot();
  }

  async setComposerReplyTarget(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    if (this.snapshot.state.ui.timeline.room_id === roomId) {
      this.snapshot.state.ui.timeline.composer.mode = { Reply: { in_reply_to_event_id: eventId } };
    }
    return this.getSnapshot();
  }

  async cancelComposerReply(): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    this.snapshot.state.ui.timeline.composer.mode = "Plain";
    return this.getSnapshot();
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
    const body = plainBodyFromDocument(document);
    this.requireComposerLease(
      account,
      { kind: "main", room_id: roomId },
      leaseId,
      rendererGeneration
    );
    const replay = this.replaySubmission(submissionId);
    if (replay) return replay;
    const session = this.snapshot.state.domain.session;
    if (
      session.kind !== "ready" ||
      !browserComposerAccountMatches(session, account) ||
      !session.user_id ||
      this.snapshot.state.ui.timeline.room_id !== roomId ||
      body.trim().length === 0
    ) {
      return {
        outcome: { rejected: { kind: "invalid" } },
        submissionId,
        transactionId: null,
        snapshot: await this.getSnapshot()
      };
    }
    const sender = session.user_id;
    const composer = this.snapshot.state.ui.timeline.composer;
    const transactionId = this.acceptSubmission(submissionId, composer);

    this.snapshot.timeline = [
      ...this.snapshot.timeline,
      {
        room_id: roomId,
        event_id: `$local-browser-${this.snapshot.timeline.length + 1}`,
        sender,
        timestamp_ms: 1_820_000_000_000 + this.snapshot.timeline.length,
        body,
        attachment_filename: null,
        reply_count: 0
      }
    ];
    this.snapshot.timeline = this.snapshot.timeline.map((message) =>
      message.event_id === inReplyToEventId
        ? { ...message, reply_count: message.reply_count + 1 }
        : message
    );
    this.terminalSubmission(composer);
    this.acceptComposerDraftTarget({ kind: "main", room_id: roomId }, draftRevision);
    this.snapshot.state.ui.timeline.composer.mode = "Plain";
    return {
      outcome: "accepted",
      submissionId,
      transactionId,
      snapshot: await this.getSnapshot()
    };
  }

  async rebuildSearchIndex(): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }
    this.snapshot.state.domain.search_crawler = {
      rooms: Object.fromEntries(
        this.snapshot.state.domain.rooms.map((room) => [room.room_id, { kind: "idle" as const }])
      ),
      last_active: null
    };
    return this.getSnapshot();
  }

  async startRoomCrawl(roomId: string): Promise<DesktopSnapshot> {
    // Browser fake: transition the room to running state so tests can observe state changes.
    if (!this.canUseSyncedViews() || !roomId.trim()) {
      return this.getSnapshot();
    }
    this.snapshot.state.domain.search_crawler = {
      rooms: {
        ...this.snapshot.state.domain.search_crawler.rooms,
        [roomId]: { kind: "queued" }
      },
      last_active: {
        room_id: roomId,
        updated_at_ms: Date.now(),
        status: "queued",
        processed: 0,
        indexed: 0
      }
    };
    return this.getSnapshot();
  }

  async stopRoomCrawl(roomId: string): Promise<DesktopSnapshot> {
    // Browser fake: transition the room to idle (matching the Rust contract) so
    // the status row stays visible with a Start button instead of disappearing.
    if (!this.canUseSyncedViews() || !roomId.trim()) {
      return this.getSnapshot();
    }
    this.snapshot.state.domain.search_crawler = {
      rooms: {
        ...this.snapshot.state.domain.search_crawler.rooms,
        [roomId]: { kind: "idle" }
      },
      last_active: this.snapshot.state.domain.search_crawler.last_active
    };
    return this.getSnapshot();
  }

  private isReady() {
    return this.snapshot.state.domain.session.kind === "ready";
  }

  private canUseSyncedViews() {
    const sessionKind = this.snapshot.state.domain.session.kind;
    return (
      sessionKind === "ready" ||
      sessionKind === "needsRecovery" ||
      sessionKind === "recovering"
    );
  }

  private hasRoom(roomId: string): boolean {
    return this.snapshot.state.domain.rooms.some((room) => room.room_id === roomId);
  }

  private roomSettingsSnapshot(roomId: string): RoomSettingsSnapshot {
    const room = this.snapshot.state.domain.rooms.find((candidate) => candidate.room_id === roomId);
    const configuredPermissions = this.roomPermissions[roomId];
    const permissions = configuredPermissions
      ? { ...configuredPermissions }
      : roomId.includes("readonly")
        ? readonlyRoomPermissionFacts()
        : editableRoomPermissionFacts();
    return {
      room_id: roomId,
      name: room?.display_name ?? null,
      topic: null,
      avatar_url: null,
      join_rule: "invite",
      history_visibility: "shared",
      permissions,
      members: this.roomMemberSnapshot()
    };
  }

  private roomMemberSnapshot(): RoomSettingsSnapshot["members"] {
    const profiles = Object.values(this.snapshot.state.domain.profile.users);
    const members = profiles.length
        ? profiles
        : [
            {
              user_id: "@browser-member:browser.fake",
              display_name: "Browser Member",
              display_label: "Browser Member",
              original_display_label: "Browser Member",
              avatar: null
            }
          ];
    return members
      .map((profile) => {
        const displayLabel = profile.display_label.trim();
        const originalDisplayLabel =
          profile.original_display_label.trim() || profile.display_name?.trim() || profile.user_id;
        return {
          user_id: profile.user_id,
          display_name: profile.display_name,
          display_label: displayLabel || profile.display_name?.trim() || profile.user_id,
          original_display_label: originalDisplayLabel,
          avatar_url: profile.avatar?.mxc_uri ?? null,
          power_level: 0,
          role: "user" as const
        };
      })
      .sort((left, right) => left.user_id.localeCompare(right.user_id));
  }

  private refreshLocalAliasProjections(userId: string): void {
    const profile = this.ensureUserProfile(userId);
    const originalDisplayLabel =
      profile.original_display_label.trim() || profile.display_name?.trim() || userId;
    const displayLabel = this.snapshot.state.domain.profile.local_aliases[userId] ?? originalDisplayLabel;
    this.snapshot.state.domain.profile.users[userId] = {
      ...profile,
      display_label: displayLabel,
      original_display_label: originalDisplayLabel,
      mention_search_terms: uniqueNonBlank([displayLabel, originalDisplayLabel, userId])
    };
    this.snapshot.state.domain.rooms = this.snapshot.state.domain.rooms.map((room) =>
      room.is_dm && room.dm_user_ids.includes(userId)
        ? {
            ...room,
            display_label: displayLabel,
            original_display_label: originalDisplayLabel
          }
        : room
    );
    this.snapshot.state.domain.room_management =
      this.snapshot.state.domain.room_management.settings === null
        ? this.snapshot.state.domain.room_management
        : {
            ...this.snapshot.state.domain.room_management,
            settings: {
              ...this.snapshot.state.domain.room_management.settings,
              members: this.snapshot.state.domain.room_management.settings.members.map((member) =>
                member.user_id === userId
                  ? {
                      ...member,
                      display_label: displayLabel,
                      original_display_label: originalDisplayLabel
                    }
                  : member
              )
            }
          };
    this.refreshSidebar();
  }

  private ensureUserProfile(userId: string): DesktopSnapshot["state"]["domain"]["profile"]["users"][string] {
    const existing = this.snapshot.state.domain.profile.users[userId];
    if (existing) {
      return existing;
    }
    const originalDisplayLabel =
      this.snapshot.state.domain.rooms.find((room) => room.is_dm && room.dm_user_ids.includes(userId))
        ?.original_display_label.trim() || userId;
    const profile = {
      user_id: userId,
      display_name: originalDisplayLabel === userId ? null : originalDisplayLabel,
      display_label: originalDisplayLabel,
      original_display_label: originalDisplayLabel,
      mention_search_terms: uniqueNonBlank([originalDisplayLabel, userId]),
      avatar: null
    };
    this.snapshot.state.domain.profile.users[userId] = profile;
    return profile;
  }

  private canRestartSync() {
    const sync = this.snapshot.state.domain.sync;
    return (
      sync === "stopped" ||
      sync === "starting" ||
      (typeof sync === "object" && ("failed" in sync || "reconnecting" in sync))
    );
  }

  private roomBelongsToSpace(roomId: string, spaceId: string): boolean {
    const room = this.snapshot.state.domain.rooms.find((candidate) => candidate.room_id === roomId);
    if (!room) {
      return false;
    }
    if (room.is_dm) {
      // #445: mirrors the Rust reducer — a DM is not a Matrix child room of a
      // Space, but every Space has a DMs surface showing a Space-filtered DM
      // list, so for navigation memory a DM belongs to the Spaces whose DM
      // projection shows it.
      return room.dm_space_ids?.includes(spaceId) ?? false;
    }
    return (
      this.snapshot.state.domain.spaces
        .find((space) => space.space_id === spaceId)
        ?.child_room_ids.includes(roomId) ?? false
    );
  }

  private rememberActiveRoomForCurrentSpace(): void {
    const spaceId = this.snapshot.state.ui.navigation.active_space_id;
    const roomId = this.snapshot.state.ui.navigation.active_room_id;
    if (!spaceId || !roomId || !this.roomBelongsToSpace(roomId, spaceId)) {
      return;
    }
    const isDm =
      this.snapshot.state.domain.rooms.find((candidate) => candidate.room_id === roomId)?.is_dm ??
      false;
    this.snapshot.state.ui.navigation.last_selection_by_space_id = {
      ...(this.snapshot.state.ui.navigation.last_selection_by_space_id ?? {}),
      [spaceId]: { surface: isDm ? "dms" : "rooms", room_id: roomId }
    };
    if (!isDm) {
      // The legacy map stays non-DM-only, exactly as the Rust reducer keeps it.
      this.snapshot.state.ui.navigation.last_room_by_space_id = {
        ...(this.snapshot.state.ui.navigation.last_room_by_space_id ?? {}),
        [spaceId]: roomId
      };
    }
  }

  private retainNavigationRoomMemory(authoritative: boolean): void {
    // #445: a provisional or incomplete projection is not evidence that a
    // remembered conversation is gone.
    if (!authoritative) {
      return;
    }
    const rememberedRooms = Object.entries(
      this.snapshot.state.ui.navigation.last_room_by_space_id ?? {}
    ).filter(([spaceId, roomId]) => this.roomBelongsToSpace(roomId, spaceId));
    this.snapshot.state.ui.navigation.last_room_by_space_id =
      Object.fromEntries(rememberedRooms);

    const knownSpaceIds = new Set(
      this.snapshot.state.domain.spaces.map((space) => space.space_id)
    );
    const rememberedSelections = Object.entries(
      this.snapshot.state.ui.navigation.last_selection_by_space_id ?? {}
    )
      .filter(([spaceId]) => knownSpaceIds.has(spaceId))
      .map(([spaceId, selection]) => {
        // A Space the user still has keeps its surface memory even when the
        // remembered conversation became inaccessible.
        const roomId =
          selection.room_id && this.roomBelongsToSpace(selection.room_id, spaceId)
            ? selection.room_id
            : null;
        return [spaceId, { surface: selection.surface, room_id: roomId }] as const;
      });
    this.snapshot.state.ui.navigation.last_selection_by_space_id =
      Object.fromEntries(rememberedSelections);
  }

  private firstRoomIdInSpace(spaceId: string): string | null {
    const space = this.snapshot.state.domain.spaces.find((candidate) => candidate.space_id === spaceId);
    return (
      space?.child_room_ids.find((roomId) => {
        const room = this.snapshot.state.domain.rooms.find(
          (candidate) => candidate.room_id === roomId
        );
        return Boolean(room) && !room?.is_dm && this.roomBelongsToSpace(roomId, spaceId);
      }) ?? null
    );
  }

  private firstDmRoomIdInSpace(spaceId: string): string | null {
    return (
      this.snapshot.state.domain.rooms.find(
        (room) => room.is_dm && (room.dm_space_ids?.includes(spaceId) ?? false)
      )?.room_id ?? null
    );
  }

  private preferredSelectionInSpace(spaceId: string): SpaceNavigationSelection {
    const remembered = this.snapshot.state.ui.navigation.last_selection_by_space_id?.[spaceId];
    if (remembered) {
      if (remembered.room_id && this.roomBelongsToSpace(remembered.room_id, spaceId)) {
        return remembered;
      }
      if (remembered.surface === "dms") {
        return { surface: "dms", room_id: this.firstDmRoomIdInSpace(spaceId) };
      }
    }
    const legacyRoomId = this.snapshot.state.ui.navigation.last_room_by_space_id?.[spaceId];
    if (legacyRoomId && this.roomBelongsToSpace(legacyRoomId, spaceId)) {
      return { surface: "rooms", room_id: legacyRoomId };
    }
    return { surface: "rooms", room_id: this.firstRoomIdInSpace(spaceId) };
  }


  private clearActiveRoomSelection(): void {
    const outgoingRoomId = this.snapshot.state.ui.navigation.active_room_id;
    if (outgoingRoomId) {
      this.clearPreparedUploadBytes({ kind: "main", room_id: outgoingRoomId });
    }
    const openThreadRoomId =
      this.snapshot.state.ui.thread.kind === "open" ? this.snapshot.state.ui.thread.room_id : null;
    if (openThreadRoomId) this.clearPreparedThreadUploadBytesForRoom(openThreadRoomId);
    this.snapshot.state.ui.navigation.active_room_id = null;
    this.snapshot.state.ui.timeline = {
      room_id: null,
      is_subscribed: false,
      is_paginating_backwards: false,
      composer: {
        accepted_submission_ids: [],
        pending_transaction_id: null,
        draft_revision: COMPOSER_DRAFT_REVISION_ZERO,
        last_accepted_clear_revision: COMPOSER_DRAFT_REVISION_ZERO,
        draft: "",
        document: documentFromText(""),
        mode: "Plain"
      },
      submission_registry: { accepted_submission_ids: [], settled_submission_ids: [] },
      scheduled_send_capability: this.snapshot.state.ui.timeline.scheduled_send_capability,
      scheduled_sends: [],
      staged_uploads: [],
      media_gallery: [],
      media_downloads: {},
      continuity: { kind: "unknown" }
    };
    this.snapshot.state.ui.thread = { kind: "closed" };
    this.snapshot.state.domain.thread_attention = { kind: "closed" };
    this.snapshot.state.ui.threads_list = { kind: "closed" };
    this.snapshot.state.ui.focused_context = { kind: "closed" };
    this.snapshot.thread = null;
    this.snapshot.timeline = [];
  }

  private nextRequestId(): number {
    const requestId = this.requestSequence;
    this.requestSequence += 1;
    return requestId;
  }

  private findTimelineMessage(
    roomId: string,
    eventId: string
  ): TimelineMessage | undefined {
    return this.snapshot.timeline.find(
      (message) => message.room_id === roomId && message.event_id === eventId
    );
  }

  private updateTimelineMessageLinkPreviews(
    roomId: string,
    eventId: string,
    linkPreviews: LinkPreview[]
  ): void {
    this.snapshot.timeline = this.snapshot.timeline.map((message) =>
      message.room_id === roomId && message.event_id === eventId
        ? { ...message, link_previews: linkPreviews }
        : message
    );
  }

  private completeIdentityReset() {
    this.snapshot.state.domain.e2ee_trust.identity_reset = { kind: "idle" };
    this.snapshot.state.domain.e2ee_trust.cross_signing = { kind: "missing" };
    this.snapshot.state.domain.e2ee_trust.key_backup = { kind: "disabled" };
    this.snapshot.state.domain.e2ee_trust.devices = this.snapshot.state.domain.e2ee_trust.devices.map(
      (device) => ({
        ...device,
        trust_level: "unverified"
      })
    );
  }

  private clearSessionViews() {
    this.composerRendererGeneration += 1n;
    this.composerLeases.clear();
    this.submissionLedger.clear();
    this.composerDrafts.clear();
    this.composerDraftRevisions.clear();
    this.threadComposerDrafts.clear();
    this.threadComposerDraftRevisions.clear();
    this.preparedUploadBytes.clear();
    this.snapshot.state.domain.secure_backup_gate = { kind: "inactive" };
    this.snapshot.state.domain.current_session_status = { status: "idle" };
    this.snapshot.state.domain.device_cleanup = { kind: "idle" };
    this.snapshot.state.domain.link_preview_settings = { room_overrides: {} };
    this.snapshot.state.domain.room_preferences = { rooms: {} };
    this.snapshot.state.domain.space_members = emptyBrowserFakeSpaceMembersState();
    this.snapshot.state.domain.invite_workflow = defaultInviteWorkflowState();
    this.snapshot.state.domain.room_notification_settings = {};
    this.snapshot.state.domain.room_interactions = {};
    this.snapshot.state.domain.mention_candidates = { targets: [] };
    this.snapshot.state.domain.thread_attention = { kind: "closed" };
    this.snapshot.state.domain.search_crawler = { rooms: {}, last_active: null };
    this.snapshot.state.domain.live_signals = defaultLiveSignalsState();
    this.snapshot.state.domain.local_encryption = { kind: "unknown" };
    this.snapshot.state.domain.native_attention = defaultNativeAttentionState();
    this.snapshot.state.domain.account_management_capabilities = {
      change_password: { kind: "unknown" }
    };
    this.snapshot.state.domain.sync = "stopped";
    this.snapshot.state.ui.navigation = {
      active_space_id: null,
      active_room_id: null,
      space_order: [],
      last_room_by_space_id: {}
    };
    this.snapshot.state.domain.spaces = [];
    this.snapshot.state.domain.rooms = [];
    this.snapshot.state.domain.invites = [];
    this.snapshot.state.ui.room_list = {
      readiness: { kind: "uninitialized" },
      active_filter: { kind: "rooms" },
      sort: { kind: "activity" },
      items: []
    };
    this.snapshot.state.ui.timeline = {
      room_id: null,
      is_subscribed: false,
      is_paginating_backwards: false,
      composer: {
        accepted_submission_ids: [],
        pending_transaction_id: null,
        draft_revision: COMPOSER_DRAFT_REVISION_ZERO,
        last_accepted_clear_revision: COMPOSER_DRAFT_REVISION_ZERO,
        draft: "",
        document: documentFromText(""),
        mode: "Plain"
      },
      submission_registry: { accepted_submission_ids: [], settled_submission_ids: [] },
      scheduled_send_capability: "unknown",
      scheduled_sends: [],
        staged_uploads: [],
        media_gallery: [],
      media_downloads: {},
      continuity: { kind: "unknown" }
    };
    this.snapshot.state.ui.thread = { kind: "closed" };
    this.snapshot.state.ui.threads_list = { kind: "closed" };
    this.snapshot.state.ui.files_view = { kind: "closed" };
    this.snapshot.state.ui.focused_context = { kind: "closed" };
    this.snapshot.state.domain.search = { kind: "closed" };
    this.snapshot.state.domain.directory = defaultDirectoryState();
    this.snapshot.state.domain.room_management = defaultRoomManagementState();
    this.snapshot.state.domain.activity = { kind: "closed" };
    this.snapshot.state.domain.device_sessions = { kind: "idle" };
    this.snapshot.state.domain.account_management = { kind: "idle" };
    this.snapshot.state.domain.soft_logout_reauth = { kind: "idle" };
    this.snapshot.state.domain.qr_login = { kind: "idle" };
    this.snapshot.state.ui.basic_operation = { kind: "idle" };
    this.snapshot.state.domain.profile = defaultProfileState(null);
    this.snapshot.state.domain.e2ee_trust = defaultE2eeTrustState();
    this.snapshot.sidebar = emptySidebar();
    this.snapshot.timeline = [];
    this.snapshot.thread = null;
  }

  private async removeRoomFromFakeSnapshot(roomId: string): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    const removedSpace = this.snapshot.state.domain.spaces.find((space) => space.space_id === roomId);
    if (removedSpace) {
      this.snapshot.state.domain.spaces = this.snapshot.state.domain.spaces.filter(
        (space) => space.space_id !== roomId
      );
      this.snapshot.state.domain.rooms = this.snapshot.state.domain.rooms.map((room) => ({
        ...room,
        parent_space_ids: room.parent_space_ids.filter((spaceId) => spaceId !== roomId),
        dm_space_ids: room.dm_space_ids.filter((spaceId) => spaceId !== roomId)
      }));
      this.snapshot.state.ui.navigation.space_order =
        this.snapshot.state.ui.navigation.space_order?.filter((spaceId) => spaceId !== roomId) ?? [];
      if (this.snapshot.state.ui.navigation.last_room_by_space_id) {
        delete this.snapshot.state.ui.navigation.last_room_by_space_id[roomId];
      }
      if (this.snapshot.state.ui.navigation.last_selection_by_space_id) {
        delete this.snapshot.state.ui.navigation.last_selection_by_space_id[roomId];
      }
      if (this.snapshot.state.ui.navigation.active_space_id === roomId) {
        this.snapshot.state.ui.navigation.active_space_id = null;
        this.snapshot.state.domain.space_members = emptyBrowserFakeSpaceMembersState();
      }
      if (
        this.snapshot.state.ui.threads_list.kind !== "closed" &&
        this.snapshot.state.ui.threads_list.room_id === `space:${roomId}`
      ) {
        this.snapshot.state.ui.threads_list = { kind: "closed" };
      }
      if (
        this.snapshot.state.ui.files_view.kind !== "closed" &&
        this.snapshot.state.ui.files_view.scope.kind === "space" &&
        this.snapshot.state.ui.files_view.scope.space_id === roomId
      ) {
        this.snapshot.state.ui.files_view = { kind: "closed" };
      }
      this.refreshSidebar();
      this.refreshRoomListProjection();
      return this.getSnapshot();
    }

    const wasActiveRoom = this.snapshot.state.ui.navigation.active_room_id === roomId;
    delete this.snapshot.state.domain.room_preferences.rooms[roomId];
    delete this.snapshot.state.domain.link_preview_settings.room_overrides[roomId];
    delete this.snapshot.state.domain.room_notification_settings[roomId];
    delete this.snapshot.state.domain.room_interactions[roomId];
    delete this.snapshot.state.domain.search_crawler.rooms[roomId];
    delete this.snapshot.state.domain.live_signals.rooms[roomId];
    if (this.snapshot.state.domain.search_crawler.last_active?.room_id === roomId) {
      this.snapshot.state.domain.search_crawler.last_active = null;
    }
    this.snapshot.state.domain.mention_candidates.targets =
      this.snapshot.state.domain.mention_candidates.targets.filter(
        (target) => target.room_id !== roomId
      );
    if (this.snapshot.state.domain.search.kind === "results") {
      this.snapshot.state.domain.search = {
        ...this.snapshot.state.domain.search,
        results: this.snapshot.state.domain.search.results.filter(
          (result) => result.room_id !== roomId
        )
      };
    }
    if (
      wasActiveRoom &&
      this.snapshot.state.domain.search.kind !== "closed" &&
      this.snapshot.state.domain.search.scope === "currentRoom"
    ) {
      this.snapshot.state.domain.search = { kind: "closed" };
    }
    if (this.snapshot.state.domain.activity.kind === "open") {
      this.snapshot.state.domain.activity = {
        ...this.snapshot.state.domain.activity,
        recent: {
          ...this.snapshot.state.domain.activity.recent,
          rows: this.snapshot.state.domain.activity.recent.rows.filter(
            (row) => row.room_id !== roomId
          )
        },
        unread: {
          ...this.snapshot.state.domain.activity.unread,
          rows: this.snapshot.state.domain.activity.unread.rows.filter(
            (row) => row.room_id !== roomId
          )
        },
        mark_read:
          this.snapshot.state.domain.activity.mark_read.kind === "pending" &&
          this.snapshot.state.domain.activity.mark_read.target.kind === "room" &&
          this.snapshot.state.domain.activity.mark_read.target.room_id === roomId
            ? { kind: "idle" }
            : this.snapshot.state.domain.activity.mark_read
      };
    }
    if (this.snapshot.state.ui.threads_list.kind !== "closed") {
      if (this.snapshot.state.ui.threads_list.room_id === roomId) {
        this.snapshot.state.ui.threads_list = { kind: "closed" };
      } else if (this.snapshot.state.ui.threads_list.kind === "open") {
        this.snapshot.state.ui.threads_list = {
          ...this.snapshot.state.ui.threads_list,
          items: this.snapshot.state.ui.threads_list.items.filter((item) => item.room_id !== roomId)
        };
      }
    }
    if (this.snapshot.state.ui.files_view.kind !== "closed") {
      const filesView = this.snapshot.state.ui.files_view;
      if (filesView.scope.kind === "room" && filesView.scope.room_id === roomId) {
        this.snapshot.state.ui.files_view = { kind: "closed" };
      } else {
        const scope =
          filesView.scope.kind === "space"
            ? {
                ...filesView.scope,
                child_room_ids: filesView.scope.child_room_ids.filter(
                  (childRoomId) => childRoomId !== roomId
                )
              }
            : filesView.scope;
        this.snapshot.state.ui.files_view =
          filesView.kind === "open"
            ? {
                ...filesView,
                scope,
                items: filesView.items.filter((item) => item.room_id !== roomId)
              }
            : { ...filesView, scope };
      }
    }
    if (
      this.snapshot.state.ui.focused_context.kind !== "closed" &&
      this.snapshot.state.ui.focused_context.room_id === roomId
    ) {
      this.snapshot.state.ui.focused_context = { kind: "closed" };
    }
    if (
      this.snapshot.state.ui.thread.kind === "open" &&
      this.snapshot.state.ui.thread.room_id === roomId
    ) {
      this.snapshot.state.ui.thread = { kind: "closed" };
      this.snapshot.state.domain.thread_attention = { kind: "closed" };
      this.snapshot.thread = null;
    }

    this.composerDrafts.delete(roomId);
    this.composerDraftRevisions.delete(roomId);
    this.clearPreparedUploadBytes({ kind: "main", room_id: roomId });
    this.clearPreparedThreadUploadBytesForRoom(roomId);
    for (const key of this.threadComposerDrafts.keys()) {
      if (key.startsWith(`thread\u0000${roomId}\u0000`)) {
        this.threadComposerDrafts.delete(key);
      }
    }
    for (const key of this.threadComposerDraftRevisions.keys()) {
      if (key.startsWith(`thread\u0000${roomId}\u0000`)) {
        this.threadComposerDraftRevisions.delete(key);
      }
    }
    this.snapshot.state.domain.rooms = this.snapshot.state.domain.rooms.filter((room) => room.room_id !== roomId);
    this.snapshot.state.domain.spaces = this.snapshot.state.domain.spaces.map((space) => ({
      ...space,
      child_room_ids: space.child_room_ids.filter((childRoomId) => childRoomId !== roomId)
    }));
    // Leaving/forgetting a room is positive evidence of removal.
    this.retainNavigationRoomMemory(true);
    if (wasActiveRoom) {
      this.clearActiveRoomSelection();
      this.snapshot.state.ui.navigation.main_timeline_anchor = null;
    }
    this.refreshSidebar();
    this.refreshRoomListProjection();
    return this.getSnapshot();
  }
}

function browserComposerAccountMatches(
  session: DesktopSnapshot["state"]["domain"]["session"],
  account: ComposerDraftAccountOwner
): boolean {
  return (
    session.kind === "ready" &&
    session.homeserver === account.homeserver &&
    session.user_id === account.userId &&
    session.device_id === account.deviceId
  );
}

function syntheticLinkPreviewImage(): LinkPreviewImage {
  const source: TimelineMediaSource = {
    mxc_uri: "mxc://example.invalid/synthetic-preview",
    encrypted: false,
    encryption_version: null
  };
  const thumbnail: AvatarThumbnailState = { kind: "notRequested" };
  return {
    source,
    thumbnail
  };
}

function isCompleteSpaceOrder(spaces: SpaceSummary[], spaceIds: string[]): boolean {
  if (spaces.length !== spaceIds.length) {
    return false;
  }

  const currentSpaceIds = new Set(spaces.map((space) => space.space_id));
  const requestedSpaceIds = new Set(spaceIds);
  if (requestedSpaceIds.size !== spaceIds.length) {
    return false;
  }

  return [...currentSpaceIds].every((spaceId) => requestedSpaceIds.has(spaceId));
}

function createInitialSnapshot(
  session: BrowserFakeApiOptions["session"],
  secureBackupGate: SecureBackupGateState | undefined
): DesktopSnapshot {
  if (session === "signedOut") {
    return createSignedOutSnapshot(secureBackupGate);
  }

  if (session === "needsRecovery") {
    return createNeedsRecoverySnapshot(secureBackupGate);
  }

  if (session === "locked") {
    return createLockedSnapshot(secureBackupGate);
  }

  return createReadySnapshot(savedSessions[0], secureBackupGate);
}

function createReadySnapshot(
  session: SavedSessionInfo = savedSessions[0],
  secureBackupGate: SecureBackupGateState = { kind: "ready" }
): DesktopSnapshot {
  const active_space_id = "!space-alpha:example.invalid";
  const active_room_id = "!room-alpha:example.invalid";
  const sidebar = composeBrowserFakeSidebar(active_space_id, spaces, rooms, {}, invites.length);
  const snapshot: DesktopSnapshot = {
    state_generation: 0,
    state: {
      schema_version: 4,
      domain: {
        session: {
          ...session,
          kind: "ready"
        },
        session_lock_reason: null,
        secure_backup_gate: secureBackupGate,
        current_session_status: { status: "idle" },
        device_cleanup: { kind: "idle" },
        auth: { kind: "unknown" },
        device_sessions: { kind: "idle" },
        account_management: { kind: "idle" },
        account_management_capabilities: { change_password: { kind: "unknown" } },
        soft_logout_reauth: { kind: "idle" },
        qr_login: { kind: "idle" },
        settings: defaultSettingsState(),
        link_preview_settings: { room_overrides: {} },
        room_preferences: { rooms: {} },
        locale_profile: defaultLocaleDisplayProfile(),
        typography_profile: defaultTypographyDisplayProfile(),
        profile: defaultProfileState(session.user_id),
        space_members: createBrowserFakeSpaceMembersState(active_space_id),
        sync: "running",
        spaces,
        rooms,
        invites,
        invite_workflow: defaultInviteWorkflowState(),
        room_notification_settings: {},
        room_interactions: {},
        directory: defaultDirectoryState(),
        room_management: defaultRoomManagementState(),
        mention_candidates: { targets: [] },
        activity: { kind: "closed" },
        thread_attention: { kind: "closed" },
        search: { kind: "closed" },
        search_crawler: { rooms: {}, last_active: null },
        live_signals: defaultLiveSignalsState(),
        e2ee_trust: defaultE2eeTrustState(),
        local_encryption: { kind: "unknown" },
        native_attention: defaultNativeAttentionState(),
        cjk_text_policy: defaultCjkTextPolicyState()
      },
      ui: {
        navigation: {
          active_space_id,
          active_room_id,
          space_order: spaces.map((space) => space.space_id),
          last_room_by_space_id: {
            [active_space_id]: active_room_id
          }
        },
        room_list: computeBrowserRoomListProjection(
          { kind: "rooms" },
          { kind: "activity" },
          active_space_id,
          spaces,
          rooms,
          invites
        ),
        timeline: {
          room_id: active_room_id,
          is_subscribed: true,
          is_paginating_backwards: false,
          composer: {
            accepted_submission_ids: [],
            pending_transaction_id: null,
            draft_revision: COMPOSER_DRAFT_REVISION_ZERO,
            last_accepted_clear_revision: COMPOSER_DRAFT_REVISION_ZERO,
            draft: "",
            document: documentFromText(""),
            mode: "Plain"
          },
          submission_registry: { accepted_submission_ids: [], settled_submission_ids: [] },
          scheduled_send_capability: "unknown",
          scheduled_sends: [],
          staged_uploads: [],
          media_gallery: [],
          media_downloads: {},
          continuity: { kind: "unknown" }
        },
        thread: { kind: "closed" },
        focused_context: { kind: "closed" },
        files_view: { kind: "closed" },
        threads_list: { kind: "closed" },
        errors: [],
        basic_operation: { kind: "idle" }
      }
    },
    sidebar,
    timeline: timelineMessages.filter((message) => message.room_id === active_room_id),
    thread: null
  };

  return snapshot;
}

const savedSessions: SavedSessionInfo[] = [
  {
    homeserver: "https://matrix.org",
    user_id: "@demo-user:example.invalid",
    device_id: "FAKEDEVICE"
  },
  {
    homeserver: "https://matrix.org",
    user_id: "@second-user:example.invalid",
    device_id: "SECONDDEVICE"
  }
];

function createNeedsRecoverySnapshot(
  secureBackupGate: SecureBackupGateState | undefined
): DesktopSnapshot {
  const snapshot = createReadySnapshot(savedSessions[0], secureBackupGate);
  snapshot.state.domain.session = {
    ...savedSessions[0],
    kind: "needsRecovery",
    recovery_methods: ["recoveryKey", "securityPhrase"]
  };
  return snapshot;
}

function createLockedSnapshot(
  secureBackupGate: SecureBackupGateState | undefined
): DesktopSnapshot {
  const snapshot = createSignedOutSnapshot(secureBackupGate);
  snapshot.state_generation = 0;
  snapshot.state.domain.session = {
    ...savedSessions[0],
    kind: "locked"
  };
  snapshot.state.domain.session_lock_reason = { kind: "unknownToken", soft_logout: false };
  return snapshot;
}

function createSignedOutSnapshot(
  secureBackupGate: SecureBackupGateState | undefined
): DesktopSnapshot {
  return {
    state: {
      schema_version: 4,
      domain: {
        session: { kind: "signedOut" },
        session_lock_reason: null,
        secure_backup_gate: secureBackupGate ?? { kind: "inactive" },
        current_session_status: { status: "idle" },
        device_cleanup: { kind: "idle" },
        auth: { kind: "unknown" },
        device_sessions: { kind: "idle" },
        account_management: { kind: "idle" },
        account_management_capabilities: { change_password: { kind: "unknown" } },
        soft_logout_reauth: { kind: "idle" },
        qr_login: { kind: "idle" },
        settings: defaultSettingsState(),
        link_preview_settings: { room_overrides: {} },
        room_preferences: { rooms: {} },
        locale_profile: defaultLocaleDisplayProfile(),
        typography_profile: defaultTypographyDisplayProfile(),
        profile: defaultProfileState(null),
        space_members: emptyBrowserFakeSpaceMembersState(),
        sync: "stopped",
        spaces: [],
        rooms: [],
        invites: [],
        invite_workflow: defaultInviteWorkflowState(),
        room_notification_settings: {},
        room_interactions: {},
        directory: defaultDirectoryState(),
        room_management: defaultRoomManagementState(),
        mention_candidates: { targets: [] },
        activity: { kind: "closed" },
        thread_attention: { kind: "closed" },
        search: { kind: "closed" },
        search_crawler: { rooms: {}, last_active: null },
        live_signals: defaultLiveSignalsState(),
        e2ee_trust: defaultE2eeTrustState(),
        local_encryption: { kind: "unknown" },
        native_attention: defaultNativeAttentionState(),
        cjk_text_policy: defaultCjkTextPolicyState()
      },
      ui: {
        navigation: {
          active_space_id: null,
          active_room_id: null,
          space_order: [],
          last_room_by_space_id: {}
        },
        room_list: computeBrowserRoomListProjection(
          { kind: "rooms" },
          { kind: "activity" },
          null,
          [],
          [],
          []
        ),
        timeline: {
          room_id: null,
          is_subscribed: false,
          is_paginating_backwards: false,
          composer: {
            accepted_submission_ids: [],
            pending_transaction_id: null,
            draft_revision: COMPOSER_DRAFT_REVISION_ZERO,
            last_accepted_clear_revision: COMPOSER_DRAFT_REVISION_ZERO,
            draft: "",
            document: documentFromText(""),
            mode: "Plain"
          },
          submission_registry: { accepted_submission_ids: [], settled_submission_ids: [] },
          scheduled_send_capability: "unknown",
          scheduled_sends: [],
          staged_uploads: [],
          media_gallery: [],
          media_downloads: {},
          continuity: { kind: "unknown" }
        },
        thread: { kind: "closed" },
        threads_list: { kind: "closed" },
        focused_context: { kind: "closed" },
        files_view: { kind: "closed" },
        errors: [],
        basic_operation: { kind: "idle" }
      }
    },
    sidebar: emptySidebar(),
    timeline: [],
    thread: null
  };
}

function ensureRoomLiveSignals(
  snapshot: DesktopSnapshot,
  roomId: string
): DesktopSnapshot["state"]["domain"]["live_signals"]["rooms"][string] {
  snapshot.state.domain.live_signals.rooms[roomId] ??= {
    receipts_by_event: {},
    fully_read_event_id: null,
    typing_user_ids: [],
    typing_users: []
  };
  return snapshot.state.domain.live_signals.rooms[roomId];
}

function initialSession(options: BrowserFakeApiOptions): BrowserFakeApiOptions["session"] {
  if (options.session) {
    return options.session;
  }

  if (options.restoreSession !== undefined) {
    return options.restoreSession ? "ready" : "signedOut";
  }

  if (typeof window === "undefined") {
    return "ready";
  }

  const session = new URLSearchParams(window.location.search).get("session");
  if (session === "signed-out") {
    return "signedOut";
  }
  if (session === "recovery") {
    return "needsRecovery";
  }
  if (session === "locked") {
    return "locked";
  }

  return "ready";
}

function normalizeHomeserver(homeserver: string): string {
  const trimmed = homeserver.trim();
  if (!trimmed.length) {
    return "https://matrix.org";
  }

  return trimmed.includes("://") ? trimmed : `https://${trimmed}`;
}

function search(
  query: string,
  scope: SearchScopeKind,
  snapshot: DesktopSnapshot
): SearchResult[] {
  if (query.length === 0) {
    return [];
  }

  return timelineMessages
    .filter((message) => roomIsInScope(message.room_id, scope, snapshot))
    .map((message) => searchMessage(message, query, scope, snapshot))
    .filter((result): result is SearchResult => Boolean(result))
    .sort(
      (left, right) =>
        right.timestamp_ms - left.timestamp_ms ||
        right.score_millis - left.score_millis ||
        left.event_id.localeCompare(right.event_id)
    );
}

function searchMinChars(query: string): number {
  return [...query].some((character) => isCjkSearchCharacter(character)) ? 2 : 3;
}

function isCjkSearchCharacter(character: string): boolean {
  const codePoint = character.codePointAt(0);
  if (codePoint === undefined) {
    return false;
  }
  return (
    (codePoint >= 0x3040 && codePoint <= 0x30ff) ||
    (codePoint >= 0x3400 && codePoint <= 0x9fff) ||
    (codePoint >= 0xf900 && codePoint <= 0xfaff) ||
    (codePoint >= 0xac00 && codePoint <= 0xd7af)
  );
}

function searchMessage(
  message: TimelineMessage,
  query: string,
  scope: SearchScopeKind,
  snapshot: DesktopSnapshot
): SearchResult | null {
  const contextLabel = searchResultContextLabel(message.room_id, scope, snapshot);
  const bodyRange = textRangeUtf16(message.body, query);
  if (bodyRange) {
    return {
      room_id: message.room_id,
      event_id: message.event_id,
      context_label: contextLabel,
      sender: message.sender,
      timestamp_ms: message.timestamp_ms,
      score_millis: candidateScore(message.event_id),
      snippet: message.body,
      match_field: "messageBody",
      highlights: [bodyRange],
      match_kind: "exact"
    };
  }

  if (message.attachment_filename) {
    const attachmentRange = textRangeUtf16(message.attachment_filename, query);
    if (attachmentRange) {
      return {
        room_id: message.room_id,
        event_id: message.event_id,
        context_label: contextLabel,
        sender: message.sender,
        timestamp_ms: message.timestamp_ms,
        score_millis: candidateScore(message.event_id),
        snippet: message.attachment_filename,
        match_field: "attachmentFileName",
        highlights: [attachmentRange],
        match_kind: "exact"
      };
    }
  }

  return null;
}

function searchResultContextLabel(
  roomId: string,
  scope: SearchScopeKind,
  snapshot: DesktopSnapshot
): string | null {
  const room = snapshot.state.domain.rooms.find((candidate) => candidate.room_id === roomId);
  if (!room) {
    return null;
  }
  const roomLabel = room.display_label.trim() || room.display_name.trim() || room.room_id;
  const spaceLabel = searchResultSpaceLabel(room, scope, snapshot);
  return spaceLabel ? `${spaceLabel} · ${roomLabel}` : roomLabel;
}

function searchResultSpaceLabel(
  room: RoomSummary,
  scope: SearchScopeKind,
  snapshot: DesktopSnapshot
): string | null {
  const spacesById = new Map(snapshot.state.domain.spaces.map((space) => [space.space_id, space]));
  if (scope === "currentSpace") {
    const activeSpaceId = snapshot.state.ui.navigation.active_space_id;
    if (activeSpaceId && roomBelongsToSpace(room, activeSpaceId)) {
      const activeLabel = spaceDisplayLabel(spacesById.get(activeSpaceId));
      if (activeLabel) {
        return activeLabel;
      }
    }
  }

  const activeSpaceId = snapshot.state.ui.navigation.active_space_id;
  if (activeSpaceId && roomBelongsToSpace(room, activeSpaceId)) {
    const activeLabel = spaceDisplayLabel(spacesById.get(activeSpaceId));
    if (activeLabel) {
      return activeLabel;
    }
  }

  return snapshot.state.domain.spaces
    .filter((space) => roomBelongsToSpace(room, space.space_id))
    .map((space) => spaceDisplayLabel(space))
    .find((label): label is string => Boolean(label)) ?? null;
}

function roomBelongsToSpace(room: RoomSummary, spaceId: string): boolean {
  return room.parent_space_ids.includes(spaceId) || room.dm_space_ids.includes(spaceId);
}

function spaceDisplayLabel(space: SpaceSummary | undefined): string | null {
  const label = space?.display_name.trim();
  return label ? label : null;
}

function candidateScore(eventId: string): number {
  switch (eventId) {
    case "$false-positive":
      return 1000;
    case "$alpha-update":
      return 950;
    case "$budget-file":
      return 900;
    case "$late-original":
      return 850;
    default:
      return 700;
  }
}

function createActivityStreams(
  includeBackfill: boolean,
  profileUsers: Record<string, UserProfile>,
  roomNotificationSettings: Record<string, RoomNotificationSettings>
): {
  recent: ActivityStream;
  unread: ActivityStream;
} {
  const spacesById = new Map(spaces.map((space) => [space.space_id, space]));
  const mutedRoomIds = new Set(
    Object.entries(roomNotificationSettings)
      .filter(([, settings]) => settings.mode.kind === "mute")
      .map(([roomId]) => roomId)
  );
  const unreadRoomIds = new Set(
    rooms
      .filter(
        (room) =>
          roomActivityUnreadCountForMode(room, roomNotificationSettings) > 0 &&
          !mutedRoomIds.has(room.room_id)
      )
      .map((room) => room.room_id)
  );
  const messages = includeBackfill
    ? [...timelineMessages, ...backwardTimelineMessages]
    : timelineMessages;
  const recentRows = activityRows(
    messages.filter((message) => !mutedRoomIds.has(message.room_id)),
    unreadRoomIds,
    profileUsers,
    spacesById,
    roomNotificationSettings
  );
  const unreadPlaceholderRows: ActivityRow[] = rooms
    .filter(
      (room) =>
        roomActivityUnreadCountForMode(room, roomNotificationSettings) > 0 &&
        !mutedRoomIds.has(room.room_id)
    )
    .map((room) => ({
      kind: "roomUnread" as const,
      room_id: room.room_id,
      event_id: null,
      thread_root_event_id: null,
      sender_id: null,
      room_label: room.display_label,
      sender_label: null,
      sender_avatar: null,
      preview: null,
      timestamp_ms:
        room.latest_event?.timestamp_ms ?? room.conversation_activity?.timestamp_ms ?? 0,
      unread: true,
      highlight: (room.highlight_count ?? 0) > 0,
      context_label: activityRowContextLabel(room, spacesById)
    }));
  return {
    recent: {
      rows: recentRows,
      next_batch: includeBackfill ? null : "browser-activity-recent-page-2",
      resolution: { kind: "idle" }
    },
    unread: {
      rows: sortActivityRows(unreadPlaceholderRows),
      next_batch: null,
      resolution: unreadPlaceholderRows.length
        ? { kind: "resolving", generation: 1, unresolved_room_count: unreadPlaceholderRows.length }
        : { kind: "idle" }
    }
  };
}

function roomActivityUnreadCount(room: RoomSummary): number {
  const notificationCount = room.notification_count ?? room.unread_count;
  const highlightCount = room.highlight_count ?? 0;
  const count = Math.max(notificationCount, highlightCount);
  if (count > 0) {
    return count;
  }
  return room.marked_unread ? 1 : 0;
}

function roomActivityUnreadCountForMode(
  room: RoomSummary,
  roomNotificationSettings: Record<string, RoomNotificationSettings>
): number {
  const mode = roomNotificationSettings[room.room_id]?.mode.kind;
  if (mode === "mentions" && (room.highlight_count ?? 0) === 0) {
    return 0;
  }
  return roomActivityUnreadCount(room);
}

function activityRows(
  messages: TimelineMessage[],
  unreadRoomIds: Set<string>,
  profileUsers: Record<string, UserProfile>,
  spacesById: Map<string, SpaceSummary>,
  roomNotificationSettings: Record<string, RoomNotificationSettings>
): ActivityRow[] {
  return messages
    .flatMap((message) => {
      const room = rooms.find((candidate) => candidate.room_id === message.room_id);
      const sender = profileUsers[message.sender];
      const highlight = message.event_id === "$alpha-update";
      const mode = room ? roomNotificationSettings[room.room_id]?.mode.kind : undefined;
      const roomActivityUnread = room
        ? roomActivityUnreadCountForMode(room, roomNotificationSettings) > 0
        : false;
      if (mode === "mentions" && !highlight && !roomActivityUnread) {
        return [];
      }
      return [
        {
          kind: "event" as const,
          room_id: message.room_id,
          event_id: message.event_id,
          thread_root_event_id: message.thread_root ?? null,
          sender_id: message.sender,
          room_label: room?.display_label ?? room?.display_name ?? "Unknown room",
          sender_label: sender?.display_label ?? message.sender,
          sender_avatar: sender?.avatar ?? null,
          preview: message.body,
          timestamp_ms: message.timestamp_ms,
          unread: unreadRoomIds.has(message.room_id),
          highlight,
          context_label: activityRowContextLabel(room ?? null, spacesById)
        }
      ];
    })
    .sort(compareActivityRows);
}

function activityRowContextLabel(
  room: RoomSummary | null,
  spacesById: Map<string, SpaceSummary>
): string {
  if (!room) {
    return "Room";
  }
  if (room.is_dm) {
    return "DM";
  }
  const spaceId = room.parent_space_ids[0];
  const space = spaceId ? spacesById.get(spaceId) : undefined;
  if (space) {
    return `${space.display_name} / ${room.display_label}`;
  }
  return room.display_label;
}

function sortActivityRows(rows: ActivityRow[]): ActivityRow[] {
  return rows.sort(compareActivityRows);
}

function compareActivityRows(left: ActivityRow, right: ActivityRow): number {
  return (
    right.timestamp_ms - left.timestamp_ms ||
    left.room_id.localeCompare(right.room_id) ||
    (left.event_id ?? "").localeCompare(right.event_id ?? "")
  );
}

function clone<T>(value: T): T {
  return structuredClone(value);
}

function emptyRoomTags(): RoomTags {
  return {
    favourite: null,
    low_priority: null
  };
}


function uniqueNonBlank(values: Array<string | null | undefined>): string[] {
  const terms: string[] = [];
  for (const value of values) {
    const normalized = value?.trim();
    if (normalized && !terms.includes(normalized)) {
      terms.push(normalized);
    }
  }
  return terms;
}

const spaces: SpaceSummary[] = [
  {
    space_id: "!space-alpha:example.invalid",
    display_name: "Synthetic Workspace",
    avatar: null,
    child_room_ids: ["!room-alpha:example.invalid", "!room-planning:example.invalid"]
  },
  {
    space_id: "!space-beta:example.invalid",
    display_name: "Synthetic Lab",
    avatar: null,
    child_room_ids: ["!room-search:example.invalid"]
  }
];

const rooms: RoomSummary[] = [
  {
    room_id: "!room-alpha:example.invalid",
    display_name: "synthetic-room",
    display_label: "synthetic-room",
    original_display_label: "synthetic-room",
    avatar: null,
    is_dm: false,
    dm_user_ids: [],
    tags: emptyRoomTags(),
    unread_count: 8,
    recency_stamp: 100,
    conversation_activity: { timestamp_ms: 100, source: "message" },
    latest_event: {
      event_id: "$alpha-latest:example.invalid",
      is_redacted: false,
      relation_type: null,
      relation_event_id: null,
      sender_id: "@member-1:example.invalid",
      sender_label: "Member 1",
      sender_avatar: null,
      preview: "Synthetic latest message",
      timestamp_ms: 100
    },
    parent_space_ids: ["!space-alpha:example.invalid"],
    dm_space_ids: [],
    is_encrypted: false
  },
  {
    room_id: "!room-planning:example.invalid",
    display_name: "planning-room",
    display_label: "planning-room",
    original_display_label: "planning-room",
    avatar: null,
    is_dm: false,
    dm_user_ids: [],
    tags: emptyRoomTags(),
    unread_count: 2,
    recency_stamp: 90,
    conversation_activity: { timestamp_ms: 90, source: "message" },
    parent_space_ids: ["!space-alpha:example.invalid"],
    dm_space_ids: [],
    is_encrypted: false
  },
  {
    room_id: "!room-search:example.invalid",
    display_name: "matrix-sdk-search",
    display_label: "matrix-sdk-search",
    original_display_label: "matrix-sdk-search",
    avatar: null,
    is_dm: false,
    dm_user_ids: [],
    tags: emptyRoomTags(),
    unread_count: 1,
    recency_stamp: 80,
    conversation_activity: { timestamp_ms: 80, source: "message" },
    parent_space_ids: ["!space-beta:example.invalid"],
    dm_space_ids: [],
    is_encrypted: false
  },
  {
    room_id: "!dm-member-1:example.invalid",
    display_name: "Member 1",
    display_label: "Member 1",
    original_display_label: "Member 1",
    avatar: null,
    is_dm: true,
    dm_user_ids: ["@member-1:example.invalid"],
    tags: emptyRoomTags(),
    unread_count: 1,
    recency_stamp: 70,
    conversation_activity: { timestamp_ms: 70, source: "message" },
    parent_space_ids: [],
    dm_space_ids: [],
    is_encrypted: false
  },
  {
    room_id: "!dm-member-2:example.invalid",
    display_name: "Member 2",
    display_label: "Member 2",
    original_display_label: "Member 2",
    avatar: null,
    is_dm: true,
    dm_user_ids: ["@member-2:example.invalid"],
    tags: emptyRoomTags(),
    unread_count: 0,
    recency_stamp: 60,
    conversation_activity: null,
    latest_event: {
      event_id: "$dm-redacted-latest:example.invalid",
      is_redacted: true,
      relation_type: null,
      relation_event_id: null,
      sender_id: "@member-2:example.invalid",
      sender_label: "Member 2",
      sender_avatar: null,
      preview: null,
      timestamp_ms: 60
    },
    parent_space_ids: [],
    dm_space_ids: [],
    is_encrypted: false
  }
];

const invites: InvitePreview[] = [
  {
    room_id: "!invite-design-review:example.invalid",
    display_name: "design-review-invite",
    avatar: null,
    topic: "Pending invite fixture for local UI review",
    inviter_display_name: "Design Reviewer",
    inviter_user_id: "@reviewer:example.invalid",
    is_dm: false
  }
];

const timelineMessages: TimelineMessage[] = [
  {
    room_id: "!room-alpha:example.invalid",
    event_id: "$alpha-update",
    sender: "Demo Coordinator",
    timestamp_ms: 1_806_986_400_000,
    body: "Alpha keyword update from demo coordinator.",
    attachment_filename: null,
    reply_count: 2
  },
  {
    room_id: "!room-alpha:example.invalid",
    event_id: "$agenda",
    sender: "Demo Coordinator",
    timestamp_ms: 1_806_990_000_000,
    body: "Synthetic planning note.\n\n- Fixture item one\n- Fixture item two",
    attachment_filename: null,
    reply_count: 0
  },
  {
    room_id: "!room-alpha:example.invalid",
    event_id: "$budget-file",
    sender: "Member 5",
    timestamp_ms: 1_806_993_600_000,
    body: "Budget spreadsheet attached.",
    attachment_filename: "fixture_budget.xlsx",
    reply_count: 0
  },
  {
    room_id: "!room-alpha:example.invalid",
    event_id: "$false-positive",
    sender: "Member 3",
    timestamp_ms: 1_806_997_200_000,
    body: "Non-matching synthetic note.",
    attachment_filename: null,
    reply_count: 0
  },
  {
    room_id: "!room-planning:example.invalid",
    event_id: "$late-original",
    sender: "Member 1",
    timestamp_ms: 1_807_000_800_000,
    body: "Final synthetic checklist",
    attachment_filename: null,
    reply_count: 0
  },
  {
    room_id: "!room-search:example.invalid",
    event_id: "$search-dev-note",
    sender: "Member 4",
    timestamp_ms: 1_807_004_400_000,
    body: "matrix-sdk-search adapter review notes",
    attachment_filename: null,
    reply_count: 0
  }
];

const backwardTimelineMessages: TimelineMessage[] = [
  {
    room_id: "!room-alpha:example.invalid",
    event_id: "$alpha-history",
    sender: "Demo Coordinator",
    timestamp_ms: 1_806_982_800_000,
    body: "Older synthetic context from the selected room.",
    attachment_filename: null,
    reply_count: 0
  }
];

const threadReplies = [
  {
    room_id: "!room-alpha:example.invalid",
    root_event_id: "$alpha-update",
    event_id: "$thread-1",
    sender: "Member 2",
    timestamp_ms: 1_806_987_000_000,
    body: "Synthetic follow-up item one."
  },
  {
    room_id: "!room-alpha:example.invalid",
    root_event_id: "$alpha-update",
    event_id: "$thread-2",
    sender: "Member 1",
    timestamp_ms: 1_806_987_600_000,
    body: "Synthetic follow-up item two."
  }
];

function threadsListItemsForRooms(roomIds: string[]): ThreadsListItem[] {
  return timelineMessages
    .filter((message) => roomIds.includes(message.room_id) && message.reply_count > 0)
    .map((message) => {
      const replies = threadReplies
        .filter((reply) => reply.room_id === message.room_id && reply.root_event_id === message.event_id)
        .sort((left, right) => left.timestamp_ms - right.timestamp_ms);
      const latestReply = replies[replies.length - 1] ?? null;
      return {
        room_id: message.room_id,
        root_event_id: message.event_id,
        root_sender: message.sender,
        root_sender_label: message.sender,
        root_body_preview: message.body,
        root_timestamp_ms: message.timestamp_ms,
        latest_event_id: latestReply?.event_id ?? null,
        latest_sender: latestReply?.sender ?? null,
        latest_sender_label: latestReply?.sender ?? null,
        latest_body_preview: latestReply?.body ?? null,
        latest_timestamp_ms: latestReply?.timestamp_ms ?? null,
        reply_count: Math.max(message.reply_count, replies.length)
      };
    });
}

function threadsListScopeKey(scope: ThreadsListScope): string {
  if (scope.kind === "home") return "home";
  if (scope.kind === "space") return `space:${scope.space_id}`;
  return scope.room_id;
}

function resolveThreadsListScope(
  scope: ThreadsListScope,
  snapshot: DesktopSnapshot
): { room_ids: string[] } {
  if (scope.kind === "room") {
    return { room_ids: [scope.room_id] };
  }
  if (scope.kind === "home") {
    return { room_ids: snapshot.state.domain.rooms.map((room) => room.room_id) };
  }
  const space = snapshot.state.domain.spaces.find((candidate) => candidate.space_id === scope.space_id);
  const room_ids = snapshot.state.domain.rooms
    .filter(
      (room) =>
        space?.child_room_ids.includes(room.room_id) === true ||
        room.parent_space_ids.includes(scope.space_id)
    )
    .map((room) => room.room_id);
  return { room_ids };
}

function attachmentResultsForScope(
  scope: AttachmentScope,
  filter: AttachmentFilter,
  sort: AttachmentSort
): AttachmentResult[] {
  const filenameQuery = filter.filename_query?.trim().toLocaleLowerCase() ?? "";
  const results = timelineMessages
    .filter((message) => message.attachment_filename)
    .filter((message) => attachmentInScope(scope, message.room_id))
    .map((message) => attachmentResultFromMessage(message))
    .filter((item) => filter.kinds.includes(item.kind))
    .filter((item) =>
      filenameQuery ? item.filename.toLocaleLowerCase().includes(filenameQuery) : true
    );

  return results.sort((left, right) => compareAttachmentResults(left, right, sort));
}

function attachmentInScope(scope: AttachmentScope, roomId: string): boolean {
  switch (scope.kind) {
    case "account":
      return true;
    case "room":
      return scope.room_id === roomId;
    case "space":
      return scope.child_room_ids.includes(roomId);
  }
}

function attachmentResultFromMessage(message: TimelineMessage): AttachmentResult {
  const filename = message.attachment_filename ?? "attachment";
  return {
    room_id: message.room_id,
    event_id: message.event_id,
    sender: message.sender,
    sender_label: null,
    timestamp_ms: message.timestamp_ms,
    kind: "file",
    filename,
    mimetype: mimetypeForFilename(filename),
    size: 18_432,
    source_mxc: `mxc://browser.fake/${message.event_id.slice(1)}`,
    thumbnail_mxc: null,
    thread_root: null,
    encrypted: false,
    encryption_version: null,
    width: null,
    height: null,
    is_edited: false
  };
}

function compareAttachmentResults(
  left: AttachmentResult,
  right: AttachmentResult,
  sort: AttachmentSort
): number {
  switch (sort) {
    case "oldestFirst":
      return left.timestamp_ms - right.timestamp_ms;
    case "sender":
      return left.sender.localeCompare(right.sender) || right.timestamp_ms - left.timestamp_ms;
    case "filename":
      return left.filename.localeCompare(right.filename) || right.timestamp_ms - left.timestamp_ms;
    case "newestFirst":
      return right.timestamp_ms - left.timestamp_ms;
  }
}

function mimetypeForFilename(filename: string): string | null {
  const lower = filename.toLocaleLowerCase();
  if (lower.endsWith(".xlsx")) {
    return "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
  }
  if (lower.endsWith(".pdf")) {
    return "application/pdf";
  }
  return null;
}
