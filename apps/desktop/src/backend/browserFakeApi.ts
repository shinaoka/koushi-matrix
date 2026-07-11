import {
  composeSidebar,
  projectRoomSummaries,
  roomIsInScope,
  textRangeUtf16
} from "../domain/desktopModel";
import { computeBrowserRoomListProjection } from "./roomListProjection";
import type {
  AvatarThumbnailState,
  TimelineMediaSource
} from "../domain/coreEvents";
import type { LinkPreview, LinkPreviewImage, LinkPreviewState } from "../domain/linkPreview";
import type {
  ActivityMarkReadTarget,
  ActivityRow,
  ActivityStream,
  ActivityTab,
  AttachmentResult,
  CreateRoomRequest,
  ComposerState,
  DesktopSnapshot,
  ComposerKeyEvent,
  ComposerResolvedAction,
  ComposerResolverOptions,
  ComposerSurface,
  DirectoryQuery,
  RoomListFilter,
  RoomListProjection,
  RoomModerationAction,
  RoomNotificationMode,
  RoomNotificationSettings,
  InviteTargetCandidate,
  InviteScopeSelection,
  InviteWorkflowState,
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
  SettingsPatch,
  PresenceKind,
  LocaleSettings,
  LocaleDisplayProfile,
  LiveEventReceiptSummary,
  LiveReadReceipt,
  MentionIntent,
  OidcAuthorization,
  SpaceSummary,
  SubmissionResponse,
  StagedUploadCompressionChoice,
  TimelineMessage,
  ThreadsListItem,
  UploadStagingRequestItem,
  AttachmentFilter,
  AttachmentScope,
  AttachmentSort,
  FilesViewScope,
  UserProfile
} from "../domain/types";
import type { DiagnosticLogSnapshot } from "../domain/diagnostics";

export interface DesktopApi {
  getSnapshot(): Promise<DesktopSnapshot>;
  getDiagnosticSnapshot(): Promise<DiagnosticLogSnapshot>;
  discoverLoginMethods(homeserver: string): Promise<DesktopSnapshot>;
  startOidcLogin(homeserver: string): Promise<OidcAuthorization>;
  completeOidcLogin(homeserver: string, callbackUrl: string): Promise<DesktopSnapshot>;
  submitLogin(
    homeserver: string,
    username: string,
    password: string,
    deviceDisplayName: string
  ): Promise<DesktopSnapshot>;
  submitSoftLogoutReauth(password: string): Promise<DesktopSnapshot>;
  listSavedSessions(): Promise<SavedSessionInfo[]>;
  switchAccount(session: SavedSessionInfo): Promise<DesktopSnapshot>;
  logout(): Promise<DesktopSnapshot>;
  submitRecovery(secret: string): Promise<DesktopSnapshot>;
  restartSync(): Promise<DesktopSnapshot>;
  updateSettings(patch: SettingsPatch): Promise<DesktopSnapshot>;
  rebuildSearchIndex(): Promise<DesktopSnapshot>;
  setRoomUrlPreviewOverride(roomId: string, enabled: boolean): Promise<DesktopSnapshot>;
  selectRoomListFilter(filter: RoomListFilter): Promise<DesktopSnapshot>;
  markRoomAsRead(roomId: string, eventId: string): Promise<DesktopSnapshot>;
  markRoomAsUnread(roomId: string, unread: boolean): Promise<DesktopSnapshot>;
  setRoomNotificationMode(roomId: string, mode: RoomNotificationMode): Promise<DesktopSnapshot>;
  queryDevices(): Promise<DesktopSnapshot>;
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
  selectSearchResult(roomId: string, eventId: string): Promise<DesktopSnapshot>;
  openTimelineAtTimestamp(roomId: string, timestampMs: number): Promise<DesktopSnapshot>;
  closeFocusedContext(): Promise<DesktopSnapshot>;
  closeSearch(): Promise<DesktopSnapshot>;
  sendText(
    submissionId: string,
    roomId: string,
    body: string,
    mentions?: MentionIntent
  ): Promise<SubmissionResponse>;
  scheduleSend(roomId: string, body: string, sendAtMs: number): Promise<DesktopSnapshot>;
  stageUploads(roomId: string, items: UploadStagingRequestItem[]): Promise<DesktopSnapshot>;
  updateStagedUploadCaption(stagedId: string, caption: string | null): Promise<DesktopSnapshot>;
  updateStagedUploadCompression(
    stagedId: string,
    compressionChoice: StagedUploadCompressionChoice
  ): Promise<DesktopSnapshot>;
  clearUploadStaging(roomId: string): Promise<DesktopSnapshot>;
  cancelScheduledSend(scheduledId: string): Promise<DesktopSnapshot>;
  rescheduleScheduledSend(scheduledId: string, sendAtMs: number): Promise<DesktopSnapshot>;
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
  editMessage(roomId: string, eventId: string, body: string): Promise<DesktopSnapshot>;
  redactMessage(roomId: string, eventId: string): Promise<DesktopSnapshot>;
  loadMessageSource(roomId: string, eventId: string): Promise<DesktopSnapshot>;
  requestRoomKey(roomId: string, eventId: string): Promise<DesktopSnapshot>;
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
  reshareRoomKey(roomId: string): Promise<DesktopSnapshot>;
  openActivity(): Promise<DesktopSnapshot>;
  closeActivity(): Promise<DesktopSnapshot>;
  setActivityTab(tab: ActivityTab): Promise<DesktopSnapshot>;
  paginateActivity(tab: ActivityTab, cursor?: string | null): Promise<DesktopSnapshot>;
  markActivityRead(target: ActivityMarkReadTarget): Promise<DesktopSnapshot>;
  setComposerDraft(roomId: string, draft: string): Promise<DesktopSnapshot>;
  openThread(roomId: string, rootEventId: string): Promise<DesktopSnapshot>;
  closeThread(): Promise<DesktopSnapshot>;
  openThreadsList(roomId: string): Promise<DesktopSnapshot>;
  closeThreadsList(): Promise<DesktopSnapshot>;
  paginateThreadsList(roomId: string): Promise<DesktopSnapshot>;
  openFilesView(scope: FilesViewScope, filter: AttachmentFilter, sort: AttachmentSort): Promise<DesktopSnapshot>;
  closeFilesView(): Promise<DesktopSnapshot>;
  setThreadComposerDraft(roomId: string, rootEventId: string, draft: string): Promise<DesktopSnapshot>;
  sendThreadReply(
    submissionId: string,
    roomId: string,
    rootEventId: string,
    body: string
  ): Promise<SubmissionResponse>;
  submitSearch(query: string, scope: SearchScopeKind): Promise<DesktopSnapshot>;
  queryDirectory(query: DirectoryQuery): Promise<DesktopSnapshot>;
  joinDirectoryRoom(alias: string, viaServer?: string | null): Promise<DesktopSnapshot>;
  joinRoom(roomId: string): Promise<DesktopSnapshot>;
  loadRoomSettings(roomId: string): Promise<DesktopSnapshot>;
  resetRoomTimelineCache(roomId: string): Promise<DesktopSnapshot>;
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
    submissionId: string,
    roomId: string,
    inReplyToEventId: string,
    body: string,
    mentions?: MentionIntent
  ): Promise<SubmissionResponse>;
  setRoomListProjection(projection: RoomListProjection): void;
  startRoomCrawl(roomId: string): Promise<DesktopSnapshot>;
  stopRoomCrawl(roomId: string): Promise<DesktopSnapshot>;
}

export interface BrowserFakeApiOptions {
  restoreSession?: boolean;
  session?: "ready" | "signedOut" | "needsRecovery" | "locked";
}

export function createBrowserFakeApi(options: BrowserFakeApiOptions = {}): DesktopApi {
  return new BrowserFakeApi(options);
}

class BrowserFakeApi implements DesktopApi {
  private snapshot: DesktopSnapshot;
  private requestSequence = 1_000;
  private composerDrafts = new Map<string, string>();
  private submissionLedger = new Map<string, { transactionId: string; target: string }>();

  private replaySubmission(submissionId: string): SubmissionResponse | null {
    const admitted = this.submissionLedger.get(submissionId);
    if (!admitted) return null;
    return {
      outcome: "accepted",
      submissionId,
      transactionId: admitted.transactionId,
      snapshot: clone(this.snapshot)
    };
  }

  private acceptSubmission(submissionId: string, target: string, composer: ComposerState): string {
    const transactionId = `$browser-${submissionId}`;
    this.submissionLedger.set(submissionId, { transactionId, target });
    composer.accepted_submission_ids = [
      ...(composer.accepted_submission_ids ?? []).filter((id) => id !== submissionId),
      submissionId
    ];
    composer.pending_submission_id = submissionId;
    composer.pending_transaction_id = transactionId;
    return transactionId;
  }

  private terminalSubmission(composer: ComposerState): void {
    composer.pending_submission_id = null;
    composer.pending_transaction_id = null;
  }

  constructor(options: BrowserFakeApiOptions) {
    this.snapshot = createInitialSnapshot(initialSession(options));
  }

  async getSnapshot(): Promise<DesktopSnapshot> {
    this.refreshRoomPresentation();
    return clone(this.snapshot);
  }

  async getDiagnosticSnapshot(): Promise<DiagnosticLogSnapshot> {
    return { entries: [], droppedEntries: 0 };
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
    deviceDisplayName: string
  ): Promise<DesktopSnapshot> {
    this.snapshot.state.domain.session = {
      kind: "authenticating",
      homeserver: normalizeHomeserver(homeserver)
    };
    this.snapshot.state.ui.errors = this.snapshot.state.ui.errors.filter(
      (error) => error.code !== "login_failed"
    );
    this.clearSessionViews();
    void username;
    void password;
    void deviceDisplayName;

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

  async logout(): Promise<DesktopSnapshot> {
    this.snapshot.state.domain.session = { kind: "signedOut" };
    this.clearSessionViews();
    return this.getSnapshot();
  }

  async submitRecovery(secret: string): Promise<DesktopSnapshot> {
    if (
      this.snapshot.state.domain.session.kind !== "needsRecovery" &&
      this.snapshot.state.domain.session.kind !== "recovering"
    ) {
      return this.getSnapshot();
    }

    this.snapshot.state.domain.session = {
      ...this.snapshot.state.domain.session,
      kind: "recovering"
    };
    this.snapshot.state.ui.errors = this.snapshot.state.ui.errors.filter(
      (error) => error.code !== "e2ee_recovery_failed"
    );
    void secret;

    this.snapshot = createReadySnapshot();
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
    this.snapshot.state.ui.navigation.active_space_id = nextSpaceId;
    this.refreshRoomListProjection();
    this.refreshSidebar();

    const targetRoomId = nextSpaceId
      ? this.preferredRoomIdInSpace(nextSpaceId)
      : this.firstDefaultRoomId();
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

    const positionBySpaceId = new Map(spaceIds.map((spaceId, index) => [spaceId, index]));
    this.snapshot.state.ui.navigation.space_order = [...spaceIds];
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
    this.snapshot.sidebar = composeSidebar(
      this.snapshot.state.ui.navigation.active_space_id,
      this.snapshot.state.domain.spaces,
      this.snapshot.state.domain.rooms,
      this.snapshot.state.domain.room_notification_settings
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
    this.snapshot.state.domain.local_encryption = { kind: "healthy" };
    return this.getSnapshot();
  }

  async resetLocalData(): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    this.snapshot.state.domain.local_encryption = {
      kind: "resetting",
      request_id: this.nextRequestId()
    };
    await Promise.resolve();
    this.snapshot.state.domain.session = { kind: "signedOut" };
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

  async confirmSasVerification(flowId: number): Promise<DesktopSnapshot> {
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
    this.snapshot.state.ui.navigation.active_room_id = roomId;
    this.snapshot.state.ui.navigation.main_timeline_anchor = null;
    this.snapshot.state.ui.timeline.room_id = roomId;
    this.snapshot.state.ui.timeline.is_subscribed = true;
    this.snapshot.state.ui.timeline.composer = {
      pending_transaction_id: null,
      draft: this.composerDrafts.get(roomId) ?? "",
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
    return this.getSnapshot();
  }

  async sendText(
    submissionId: string,
    roomId: string,
    body: string,
    mentions: MentionIntent = emptyMentionIntent()
  ): Promise<SubmissionResponse> {
    void mentions;
    const replay = this.replaySubmission(submissionId);
    if (replay) return replay;
    const session = this.snapshot.state.domain.session;
    if (
      session.kind !== "ready" ||
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
    const transactionId = this.acceptSubmission(submissionId, `main:${roomId}`, composer);

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
    this.snapshot.state.ui.timeline.composer.draft = "";
    this.composerDrafts.delete(roomId);
    return {
      outcome: "accepted",
      submissionId,
      transactionId,
      snapshot: await this.getSnapshot()
    };
  }

  async scheduleSend(
    roomId: string,
    body: string,
    sendAtMs: number
  ): Promise<DesktopSnapshot> {
    const session = this.snapshot.state.domain.session;
    if (
      session.kind !== "ready" ||
      this.snapshot.state.ui.timeline.room_id !== roomId ||
      body.trim().length === 0 ||
      !Number.isFinite(sendAtMs)
    ) {
      return this.getSnapshot();
    }

    this.snapshot.state.ui.timeline.scheduled_send_capability = "localFallback";
    this.snapshot.state.ui.timeline.scheduled_sends = [
      ...this.snapshot.state.ui.timeline.scheduled_sends,
      {
        scheduled_id: `browser-scheduled-${this.snapshot.state.ui.timeline.scheduled_sends.length + 1}`,
        room_id: roomId,
        body,
        send_at_ms: sendAtMs,
        handle: { kind: "local" }
      }
    ];
    this.snapshot.state.ui.timeline.composer.draft = "";
    this.composerDrafts.delete(roomId);
    return this.getSnapshot();
  }

  async stageUploads(
    roomId: string,
    items: UploadStagingRequestItem[]
  ): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || this.snapshot.state.ui.timeline.room_id !== roomId) {
      return this.getSnapshot();
    }
    this.snapshot.state.ui.timeline.staged_uploads = items.map((item, index) => ({
      staged_id: item.stagedId,
      room_id: roomId,
      position: item.position || index + 1,
      filename: item.filename.trim() || "attachment",
      mime_type: item.mimeType.trim() || "application/octet-stream",
      byte_count: Math.max(0, Math.floor(item.byteCount)),
      kind: item.kind,
      caption: null,
      compression_choice: item.compressionChoice
    }));
    return this.getSnapshot();
  }

  async updateStagedUploadCaption(
    stagedId: string,
    caption: string | null
  ): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }
    const normalized = caption?.trim() ? caption : null;
    this.snapshot.state.ui.timeline.staged_uploads = this.snapshot.state.ui.timeline.staged_uploads.map(
      (item) =>
        item.staged_id === stagedId
          ? {
              ...item,
              caption: normalized
                ? { plain_body: normalized, formatted_body: null, mentions: { targets: [] } }
                : null
            }
          : item
    );
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

  async clearUploadStaging(roomId: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || this.snapshot.state.ui.timeline.room_id !== roomId) {
      return this.getSnapshot();
    }
    this.snapshot.state.ui.timeline.staged_uploads = [];
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
    sendAtMs: number
  ): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || !Number.isFinite(sendAtMs)) {
      return this.getSnapshot();
    }
    this.snapshot.state.ui.timeline.scheduled_sends =
      this.snapshot.state.ui.timeline.scheduled_sends.map((item) =>
        item.scheduled_id === scheduledId ? { ...item, send_at_ms: sendAtMs } : item
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
    void threadRootEventId;
    const session = this.snapshot.state.domain.session;
    if (!this.isReady() || !session.user_id || eventId.trim().length === 0) {
      return;
    }
    const roomSignals = ensureRoomLiveSignals(this.snapshot, roomId);
    const existing = roomSignals.receipts_by_event[eventId]?.readers ?? [];
    roomSignals.receipts_by_event[eventId] = projectReceiptSummary([
      ...existing.filter((receipt) => receipt.user_id !== session.user_id),
      {
        user_id: session.user_id,
        display_name: this.snapshot.state.domain.profile.own.display_name,
        original_display_label: this.snapshot.state.domain.profile.own.display_name?.trim() || session.user_id,
        avatar: this.snapshot.state.domain.profile.own.avatar,
        timestamp_ms: Date.now()
      }
    ]);
  }

  async setFullyRead(roomId: string, eventId: string): Promise<void> {
    if (!this.isReady() || eventId.trim().length === 0) {
      return;
    }
    ensureRoomLiveSignals(this.snapshot, roomId).fully_read_event_id = eventId;
  }

  async setTyping(roomId: string, isTyping: boolean): Promise<void> {
    const session = this.snapshot.state.domain.session;
    if (!this.isReady() || !session.user_id) {
      return;
    }
    const roomSignals = ensureRoomLiveSignals(this.snapshot, roomId);
    const withoutSelf = roomSignals.typing_user_ids.filter((userId) => userId !== session.user_id);
    roomSignals.typing_user_ids = isTyping ? [...withoutSelf, session.user_id] : withoutSelf;
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
    body: string
  ): Promise<DesktopSnapshot> {
    if (!this.isReady() || body.trim().length === 0) {
      return this.getSnapshot();
    }

    this.snapshot.timeline = this.snapshot.timeline.map((message) =>
      message.room_id === roomId && message.event_id === eventId
        ? { ...message, body, attachment_filename: null }
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

  async requestRoomKey(_roomId: string, _eventId: string): Promise<DesktopSnapshot> {
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

  async openThread(roomId: string, rootEventId: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    this.snapshot.state.ui.thread = {
      kind: "open",
      room_id: roomId,
      root_event_id: rootEventId,
      is_subscribed: true,
      composer: { pending_transaction_id: null, draft: "", mode: "Plain" }
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

    this.snapshot.state.ui.thread = { kind: "closed" };
    this.snapshot.thread = null;
    return this.getSnapshot();
  }

  async openThreadsList(roomId: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }
    if (!this.snapshot.state.domain.rooms.some((room) => room.room_id === roomId)) {
      return this.getSnapshot();
    }
    this.snapshot.state.ui.threads_list = {
      kind: "open",
      room_id: roomId,
      request_id: 0,
      items: threadsListItemsForRoom(roomId),
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

  async paginateThreadsList(roomId: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }
    const list = this.snapshot.state.ui.threads_list;
    if (
      list.kind === "open" &&
      list.room_id === roomId &&
      !list.is_paginating &&
      !list.end_reached
    ) {
      list.is_paginating = true;
    }
    return this.getSnapshot();
  }

  async setThreadComposerDraft(
    roomId: string,
    rootEventId: string,
    draft: string
  ): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    const thread = this.snapshot.state.ui.thread;
    if (
      thread.kind === "open" &&
      thread.room_id === roomId &&
      thread.root_event_id === rootEventId &&
      thread.composer
    ) {
      thread.composer.draft = draft;
    }
    return this.getSnapshot();
  }

  async sendThreadReply(
    submissionId: string,
    roomId: string,
    rootEventId: string,
    body: string
  ): Promise<SubmissionResponse> {
    const replay = this.replaySubmission(submissionId);
    if (replay) return replay;
    const session = this.snapshot.state.domain.session;
    const thread = this.snapshot.state.ui.thread;
    if (
      session.kind !== "ready" ||
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

    const transactionId = this.acceptSubmission(
      submissionId,
      `thread:${roomId}:${rootEventId}`,
      thread.composer
    );
    this.terminalSubmission(thread.composer);
    thread.composer.draft = "";
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

    const results = search(query, scope, this.snapshot);
    this.snapshot.state.domain.search = {
      kind: "results",
      request_id: Date.now(),
      query,
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

    const alias = "#public-demo:fake.local";
    this.snapshot.state.domain.directory.query = {
      kind: "results",
      request_id: requestId,
      query: normalizedQuery,
      rooms: [
        {
          room_id: "!public-demo:fake.local",
          canonical_alias: alias,
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

  async joinDirectoryRoom(alias: string, viaServer: string | null = null): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || alias.trim().length === 0) {
      return this.getSnapshot();
    }

    const requestId = this.nextRequestId();
    const normalizedAlias = alias.trim();
    const normalizedViaServer = viaServer?.trim() ? viaServer.trim() : null;
    this.snapshot.state.domain.directory.join = {
      kind: "joining",
      request_id: requestId,
      alias: normalizedAlias,
      via_server: normalizedViaServer
    };

    await Promise.resolve();

    const roomId = `!joined-${this.snapshot.state.domain.rooms.length + 1}:fake.local`;
    const displayName = normalizedAlias.replace(/^#/, "").split(":")[0] || "Public Room";
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

  async reshareRoomKey(_roomId: string): Promise<DesktopSnapshot> {
    return this.getSnapshot();
  }

  async resetRoomTimelineCache(_roomId: string): Promise<DesktopSnapshot> {
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
    this.snapshot.state.domain.invite_workflow = {
      ...workflow,
      query: {
        ...workflow.query,
        room_id: roomId
      },
      scope_plan: buildFakeInviteScopePlan(this.snapshot, roomId)
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
    this.snapshot.state.domain.invite_workflow = {
      ...workflow,
      query: buildFakeInviteTargetQuery(this.snapshot, roomId, query),
      scope_plan: buildFakeInviteScopePlan(this.snapshot, roomId)
    };
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
      pin_operation: { kind: "idle" as const }
    };
    const alreadyPinned = entry.pinned_events.some((event) => event.event_id === eventId);
    this.snapshot.state.domain.room_interactions = {
      ...this.snapshot.state.domain.room_interactions,
      [roomId]: {
        pinned_events: alreadyPinned
          ? entry.pinned_events
          : [
              ...entry.pinned_events,
              { event_id: eventId, sender: null, body_preview: null, redacted: false }
            ],
        pin_operation: { kind: "idle" }
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
      pin_operation: { kind: "idle" as const }
    };
    this.snapshot.state.domain.room_interactions = {
      ...this.snapshot.state.domain.room_interactions,
      [roomId]: {
        pinned_events: entry.pinned_events.filter((event) => event.event_id !== eventId),
        pin_operation: { kind: "idle" }
      }
    };
    return this.getSnapshot();
  }

  async openActivity(): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    const requestId = this.nextRequestId();
    this.snapshot.state.domain.activity = {
      kind: "opening",
      request_id: requestId,
      tab: "recent"
    };

    await Promise.resolve();

    const streams = createActivityStreams(
      false,
      this.snapshot.state.domain.profile.users,
      this.snapshot.state.domain.room_notification_settings
    );
    this.snapshot.state.domain.activity = {
      kind: "open",
      active_tab: "recent",
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

    this.snapshot.state.domain.activity = { kind: "closed" };
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
      next_batch: null
    };
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

    if (target.kind === "all") {
      this.snapshot.state.domain.activity.unread = { rows: [], next_batch: null };
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

  async setComposerDraft(roomId: string, draft: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || this.snapshot.state.ui.timeline.room_id !== roomId) {
      return this.getSnapshot();
    }

    if (draft.length === 0) {
      this.composerDrafts.delete(roomId);
    } else {
      this.composerDrafts.set(roomId, draft);
    }
    this.snapshot.state.ui.timeline.composer.draft = draft;
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
    submissionId: string,
    roomId: string,
    inReplyToEventId: string,
    body: string,
    mentions: MentionIntent = emptyMentionIntent()
  ): Promise<SubmissionResponse> {
    void mentions;
    const replay = this.replaySubmission(submissionId);
    if (replay) return replay;
    const session = this.snapshot.state.domain.session;
    if (
      session.kind !== "ready" ||
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
    const transactionId = this.acceptSubmission(submissionId, `reply:${roomId}:${inReplyToEventId}`, composer);

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
    this.snapshot.state.ui.timeline.composer.draft = "";
    this.snapshot.state.ui.timeline.composer.mode = "Plain";
    this.composerDrafts.delete(roomId);
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
    const permissions = roomId.includes("readonly")
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
    if (!room || room.is_dm) {
      return false;
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
    this.snapshot.state.ui.navigation.last_room_by_space_id = {
      ...(this.snapshot.state.ui.navigation.last_room_by_space_id ?? {}),
      [spaceId]: roomId
    };
  }

  private retainNavigationRoomMemory(): void {
    const rememberedRooms = Object.entries(
      this.snapshot.state.ui.navigation.last_room_by_space_id ?? {}
    ).filter(([spaceId, roomId]) => this.roomBelongsToSpace(roomId, spaceId));
    this.snapshot.state.ui.navigation.last_room_by_space_id =
      Object.fromEntries(rememberedRooms);
  }

  private firstRoomIdInSpace(spaceId: string): string | null {
    const space = this.snapshot.state.domain.spaces.find((candidate) => candidate.space_id === spaceId);
    return (
      space?.child_room_ids.find((roomId) => this.roomBelongsToSpace(roomId, spaceId)) ?? null
    );
  }

  private preferredRoomIdInSpace(spaceId: string): string | null {
    const rememberedRoomId = this.snapshot.state.ui.navigation.last_room_by_space_id?.[spaceId];
    if (rememberedRoomId && this.roomBelongsToSpace(rememberedRoomId, spaceId)) {
      return rememberedRoomId;
    }
    return this.firstRoomIdInSpace(spaceId);
  }

  private firstDefaultRoomId(): string | null {
    return (
      this.snapshot.state.domain.rooms.find((room) => !room.is_dm)?.room_id ??
      this.snapshot.state.domain.rooms[0]?.room_id ??
      null
    );
  }

  private clearActiveRoomSelection(): void {
    this.snapshot.state.ui.navigation.active_room_id = null;
    this.snapshot.state.ui.timeline = {
      room_id: null,
      is_subscribed: false,
      is_paginating_backwards: false,
      composer: {
        pending_transaction_id: null,
        draft: "",
        mode: "Plain"
      },
      scheduled_send_capability: this.snapshot.state.ui.timeline.scheduled_send_capability,
      scheduled_sends: [],
      staged_uploads: [],
      media_gallery: [],
      media_downloads: {}
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
    const update = (message: TimelineMessage) => {
      if (message.room_id === roomId && message.event_id === eventId) {
        message.link_previews = linkPreviews;
      }
    };
    this.snapshot.timeline.forEach(update);
    timelineMessages.forEach(update);
    backwardTimelineMessages.forEach(update);
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
    this.composerDrafts.clear();
    this.snapshot.state.domain.sync = "stopped";
    this.snapshot.state.domain.sync_mode = { kind: "unsupported" };
    this.snapshot.state.ui.navigation = {
      active_space_id: null,
      active_room_id: null,
      last_room_by_space_id: {}
    };
    this.snapshot.state.domain.spaces = [];
    this.snapshot.state.domain.rooms = [];
    this.snapshot.state.domain.invites = [];
    this.snapshot.state.ui.room_list = {
      active_filter: { kind: "rooms" },
      sort: { kind: "activity" },
      items: []
    };
    this.snapshot.state.ui.timeline = {
      room_id: null,
      is_subscribed: false,
      is_paginating_backwards: false,
      composer: {
        pending_transaction_id: null,
        draft: "",
        mode: "Plain"
      },
      scheduled_send_capability: "unknown",
      scheduled_sends: [],
        staged_uploads: [],
        media_gallery: [],
      media_downloads: {}
    };
    this.snapshot.state.ui.thread = { kind: "closed" };
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
        parent_space_ids: room.parent_space_ids.filter((spaceId) => spaceId !== roomId)
      }));
      this.snapshot.state.ui.navigation.space_order =
        this.snapshot.state.ui.navigation.space_order?.filter((spaceId) => spaceId !== roomId) ?? [];
      if (this.snapshot.state.ui.navigation.last_room_by_space_id) {
        delete this.snapshot.state.ui.navigation.last_room_by_space_id[roomId];
      }
      if (this.snapshot.state.ui.navigation.active_space_id === roomId) {
        this.snapshot.state.ui.navigation.active_space_id = null;
      }
    }

    this.composerDrafts.delete(roomId);
    this.snapshot.state.domain.rooms = this.snapshot.state.domain.rooms.filter((room) => room.room_id !== roomId);
    this.snapshot.state.domain.spaces = this.snapshot.state.domain.spaces.map((space) => ({
      ...space,
      child_room_ids: space.child_room_ids.filter((childRoomId) => childRoomId !== roomId)
    }));
    this.retainNavigationRoomMemory();
    if (this.snapshot.state.ui.navigation.active_room_id === roomId) {
      this.snapshot.state.ui.navigation.active_room_id = null;
      this.snapshot.state.ui.timeline.room_id = null;
      this.snapshot.state.ui.timeline.is_subscribed = false;
      this.snapshot.timeline = [];
      this.snapshot.state.ui.thread = { kind: "closed" };
      this.snapshot.thread = null;
    }
    this.refreshSidebar();
    this.refreshRoomListProjection();
    return this.getSnapshot();
  }
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

function createInitialSnapshot(session: BrowserFakeApiOptions["session"]): DesktopSnapshot {
  if (session === "signedOut") {
    return createSignedOutSnapshot();
  }

  if (session === "needsRecovery") {
    return createNeedsRecoverySnapshot();
  }

  if (session === "locked") {
    return createLockedSnapshot();
  }

  return createReadySnapshot();
}

function createReadySnapshot(session: SavedSessionInfo = savedSessions[0]): DesktopSnapshot {
  const active_space_id = "!space-alpha:example.invalid";
  const active_room_id = "!room-alpha:example.invalid";
  const sidebar = composeSidebar(active_space_id, spaces, rooms);
    const snapshot: DesktopSnapshot = {
    state: {
      schema_version: 2,
      domain: {
        session: {
          ...session,
          kind: "ready"
        },
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
        sync: "running",
        sync_mode: { kind: "unsupported" },
        spaces,
        rooms,
        invites,
        invite_workflow: defaultInviteWorkflowState(),
        room_notification_settings: {},
        room_interactions: {},
        directory: defaultDirectoryState(),
        room_management: defaultRoomManagementState(),
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
            pending_transaction_id: null,
            draft: "",
            mode: "Plain"
          },
          scheduled_send_capability: "unknown",
          scheduled_sends: [],
          staged_uploads: [],
          media_gallery: [],
          media_downloads: {}
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

function createNeedsRecoverySnapshot(): DesktopSnapshot {
  const snapshot = createReadySnapshot();
  snapshot.state.domain.session = {
    ...savedSessions[0],
    kind: "needsRecovery",
    recovery_methods: ["recoveryKey", "securityPhrase"]
  };
  return snapshot;
}

function createLockedSnapshot(): DesktopSnapshot {
  const snapshot = createReadySnapshot();
  snapshot.state.domain.session = {
    ...savedSessions[0],
    kind: "locked"
  };
  snapshot.state.domain.sync = "stopped";
  snapshot.state.ui.navigation.active_room_id = null;
  snapshot.state.ui.timeline.room_id = null;
  snapshot.state.ui.timeline.is_subscribed = false;
  snapshot.timeline = [];
  return snapshot;
}

function createSignedOutSnapshot(): DesktopSnapshot {
  return {
    state: {
      schema_version: 2,
      domain: {
        session: { kind: "signedOut" },
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
        sync: "stopped",
        sync_mode: { kind: "unsupported" },
        spaces: [],
        rooms: [],
        invites: [],
        invite_workflow: defaultInviteWorkflowState(),
        room_notification_settings: {},
        room_interactions: {},
        directory: defaultDirectoryState(),
        room_management: defaultRoomManagementState(),
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
            pending_transaction_id: null,
            draft: "",
            mode: "Plain"
          },
          scheduled_send_capability: "unknown",
          scheduled_sends: [],
          staged_uploads: [],
          media_gallery: [],
          media_downloads: {}
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

function defaultSettingsState(): DesktopSnapshot["state"]["domain"]["settings"] {
  return {
    values: {
      locale: { language_tag: null, text_direction: "auto" },
      appearance: { theme: "system" },
      typography: { font: "system", emoji: "system" },
      keyboard: { composer_send_shortcut: "enter" },
      notifications: {
        desktop_notifications: true,
        sound: true,
        badges: true,
        send_read_receipts: true,
        send_typing_notifications: true
      },
      display: {
        code_block_wrap: true,
        hide_redacted: true,
        url_previews_enabled: true,
        encrypted_url_previews_enabled: true
      },
      media: {
        image_upload_compression: "ask",
        image_upload_compression_policy: {
          threshold_bytes: 1048576,
          threshold_long_edge: 2560,
          target_long_edge: 2048,
          quality_percent: 82
        }
      },
      timeline: {
        auto_load_older_messages: true,
        thread_root_order: { kind: "rootEvent" }
      },
      search_crawler: {
        speed: "standard" as const,
        include_media_captions: true,
        include_filenames: true
      },
      thread_list_order: { kind: "latestReply" },
      room_list_sort: { kind: "activity" }
    },
    persistence: { kind: "idle" }
  };
}

function defaultDirectoryState(): DesktopSnapshot["state"]["domain"]["directory"] {
  return {
    query: { kind: "closed" },
    join: { kind: "idle" }
  };
}

function defaultRoomManagementState(): DesktopSnapshot["state"]["domain"]["room_management"] {
  return {
    selected_room_id: null,
    settings: null,
    operation: { kind: "idle" }
  };
}

function editableRoomPermissionFacts(): RoomPermissionFacts {
  return {
    can_edit_settings: true,
    can_edit_roles: true,
    can_kick: true,
    can_ban: true,
    can_unban: true
  };
}

function readonlyRoomPermissionFacts(): RoomPermissionFacts {
  return {
    can_edit_settings: false,
    can_edit_roles: false,
    can_kick: false,
    can_ban: false,
    can_unban: false
  };
}

function roomMemberRoleFromPowerLevel(powerLevel: number): RoomSettingsSnapshot["members"][number]["role"] {
  if (powerLevel >= 100) {
    return "administrator";
  }
  if (powerLevel >= 50) {
    return "moderator";
  }
  return "user";
}

function applyRoomSettingChange(
  settings: RoomSettingsSnapshot,
  change: RoomSettingChange
): RoomSettingsSnapshot {
  if ("name" in change) {
    return { ...settings, name: change.name };
  }
  if ("topic" in change) {
    return { ...settings, topic: change.topic };
  }
  if ("avatarUrl" in change) {
    return { ...settings, avatar_url: change.avatarUrl };
  }
  if ("joinRule" in change) {
    return { ...settings, join_rule: change.joinRule };
  }
  return { ...settings, history_visibility: change.historyVisibility };
}

function roomModerationAllowed(
  permissions: RoomPermissionFacts,
  action: RoomModerationAction
): boolean {
  switch (action) {
    case "kick":
      return permissions.can_kick;
    case "ban":
      return permissions.can_ban;
    case "unban":
      return permissions.can_unban;
  }
}

function defaultE2eeTrustState(): DesktopSnapshot["state"]["domain"]["e2ee_trust"] {
  return {
    verification: { kind: "idle" },
    cross_signing: { kind: "unknown" },
    key_backup: { kind: "unknown" },
    identity_reset: { kind: "idle" },
    key_management: defaultE2eeKeyManagementState(),
    devices: []
  };
}

function defaultDelegatedAuthLinks(): Extract<
  DesktopSnapshot["state"]["domain"]["auth"],
  { kind: "ready" }
>["delegated"] {
  return {
    registration_url: null,
    account_management_url: null
  };
}

function defaultE2eeKeyManagementState(): DesktopSnapshot["state"]["domain"]["e2ee_trust"]["key_management"] {
  return {
    room_key_export: { kind: "idle" },
    room_key_import: { kind: "idle" },
    secure_backup_setup: { kind: "idle" },
    passphrase_change: { kind: "idle" }
  };
}

function defaultLiveSignalsState(): DesktopSnapshot["state"]["domain"]["live_signals"] {
  return {
    rooms: {},
    presence: {}
  };
}

function defaultNativeAttentionState(): DesktopSnapshot["state"]["domain"]["native_attention"] {
  return {
    summary: {
      unread_count: 0,
      highlight_count: 0,
      badge_count: 0,
      candidate: null,
      capabilities: {
        notifications: "unknown",
        badge: "unknown",
        overlay_icon: "unknown",
        sound: "unknown",
        tray: "unknown",
        activation: "unknown"
      }
    },
    dispatch: { kind: "idle" }
  };
}

function defaultCjkTextPolicyState(): DesktopSnapshot["state"]["domain"]["cjk_text_policy"] {
  return {
    japanese_catalog: {
      catalog_locale: "en",
      complete: true,
      missing_message_ids: []
    },
    normalization: {
      form: "nfkc",
      width_fold: true,
      kana_fold: true
    },
    collation: {
      locale: "ja",
      numeric: true,
      case_first: null
    }
  };
}

function defaultProfileState(userId: string | null | undefined): DesktopSnapshot["state"]["domain"]["profile"] {
  return {
    own: {
      display_name: userId ? "Demo User" : null,
      avatar: null
    },
    users: {},
    local_aliases: {},
    local_alias_update: { kind: "idle" },
    ignored_user_ids: [],
    ignored_user_update: { kind: "idle" },
    update: { kind: "idle" }
  };
}

const INVITE_ALREADY_IN_SPACE_MESSAGE = "既にスペースにいます";

function defaultInviteWorkflowState(): InviteWorkflowState {
  return {
    query: {
      room_id: null,
      query: "",
      candidates: [],
      explicit_user_id: null
    },
    selected_targets: [],
    scope_plan: null,
    operation: { kind: "idle" }
  };
}

function buildFakeInviteScopePlan(
  snapshot: DesktopSnapshot,
  roomId: string
): InviteWorkflowState["scope_plan"] {
  if (snapshot.state.domain.spaces.some((space) => space.space_id === roomId)) {
    return {
      room_id: roomId,
      destination_kind: "space",
      default_scope: { kind: "roomOnly" },
      options: [{ scope: { kind: "roomOnly" }, label: "Space only", detail: null }]
    };
  }
  const room = snapshot.state.domain.rooms.find((candidate) => candidate.room_id === roomId);
  const activeSpaceId = snapshot.state.ui.navigation.active_space_id;
  const parentSpaceIds = room?.parent_space_ids ?? [];
  const orderedParentSpaceIds = [
    ...(activeSpaceId && parentSpaceIds.includes(activeSpaceId) ? [activeSpaceId] : []),
    ...parentSpaceIds.filter((spaceId) => spaceId !== activeSpaceId)
  ];
  const options: NonNullable<InviteWorkflowState["scope_plan"]>["options"] =
    orderedParentSpaceIds.map((spaceId) => {
    const space = snapshot.state.domain.spaces.find((candidate) => candidate.space_id === spaceId);
    return {
      scope: { kind: "parentSpaceAndRoom" as const, space_id: spaceId },
      label: `${space?.display_name ?? "Parent space"} and room`,
      detail: "Invite to the parent space before inviting to this room"
    };
  });
  options.push({ scope: { kind: "roomOnly" }, label: "Room only", detail: null });
  return {
    room_id: roomId,
    destination_kind: "room",
    default_scope: options[0]?.scope ?? { kind: "roomOnly" },
    options
  };
}

function buildFakeInviteTargetQuery(
  snapshot: DesktopSnapshot,
  roomId: string,
  query: string
): InviteWorkflowState["query"] {
  const trimmed = query.trim();
  if (!trimmed) {
    return { room_id: roomId, query, candidates: [], explicit_user_id: null };
  }
  const lowered = trimmed.toLocaleLowerCase();
  const workflow = snapshot.state.domain.invite_workflow ?? defaultInviteWorkflowState();
  const selectedUserIds = new Set(
    workflow.selected_targets.map((target) => target.user_id)
  );
  const members = snapshot.state.domain.room_management.settings?.room_id === roomId
    ? snapshot.state.domain.room_management.settings.members
    : [];
  const destinationMembers = new Set(members.map((member) => member.user_id));
  const candidates = new Map<string, InviteTargetCandidate>();

  for (const [userId, profile] of Object.entries(snapshot.state.domain.profile.users)) {
    const alias = snapshot.state.domain.profile.local_aliases[userId] ?? null;
    if (
      fakeInviteTextMatches(userId, lowered) ||
      fakeInviteTextMatches(alias, lowered) ||
      fakeInviteTextMatches(profile.display_name, lowered) ||
      fakeInviteTextMatches(profile.display_label, lowered) ||
      profile.mention_search_terms.some((term) => fakeInviteTextMatches(term, lowered))
    ) {
      candidates.set(
        userId,
        fakeInviteCandidate(
          userId,
          (alias ?? profile.display_label) || userId,
          profile.original_display_label || profile.display_label || userId,
          profile.avatar,
          alias ? "localAlias" : "profile",
          selectedUserIds,
          destinationMembers
        )
      );
    }
  }

  for (const [userId, alias] of Object.entries(snapshot.state.domain.profile.local_aliases)) {
    if (!candidates.has(userId) && (fakeInviteTextMatches(userId, lowered) || fakeInviteTextMatches(alias, lowered))) {
      candidates.set(
        userId,
        fakeInviteCandidate(
          userId,
          alias,
          alias,
          null,
          "localAlias",
          selectedUserIds,
          destinationMembers
        )
      );
    }
  }

  for (const member of members) {
    const alias = snapshot.state.domain.profile.local_aliases[member.user_id] ?? null;
    if (
      !candidates.has(member.user_id) &&
      (fakeInviteTextMatches(member.user_id, lowered) ||
        fakeInviteTextMatches(alias, lowered) ||
        fakeInviteTextMatches(member.display_name, lowered) ||
        fakeInviteTextMatches(member.display_label, lowered))
    ) {
      candidates.set(
        member.user_id,
        fakeInviteCandidate(
          member.user_id,
          alias ?? member.display_label,
          member.original_display_label || member.display_label,
          null,
          "roomMember",
          selectedUserIds,
          destinationMembers
        )
      );
    }
  }

  const sortedCandidates = [...candidates.values()].sort((left, right) =>
    left.display_label.localeCompare(right.display_label) || left.user_id.localeCompare(right.user_id)
  );
  const explicit_user_id = trimmed.startsWith("@")
    ? fakeInviteCandidate(
        trimmed,
        trimmed,
        trimmed,
        null,
        "matrixId",
        new Set(),
        new Set(),
        fakeValidMatrixUserId(trimmed) ? "selectable" : "invalidMatrixId"
      )
    : null;
  return {
    room_id: roomId,
    query,
    candidates: sortedCandidates.slice(0, 8),
    explicit_user_id
  };
}

function fakeInviteCandidate(
  userId: string,
  displayLabel: string,
  originalDisplayLabel: string,
  avatar: InviteTargetCandidate["avatar"],
  source: InviteTargetCandidate["source"],
  selectedUserIds: Set<string>,
  destinationMembers: Set<string>,
  forcedStatus?: InviteTargetCandidate["status"]
): InviteTargetCandidate {
  return {
    user_id: userId,
    display_label: displayLabel,
    original_display_label: originalDisplayLabel,
    avatar,
    source,
    status:
      forcedStatus ??
      (selectedUserIds.has(userId)
        ? "alreadySelected"
        : destinationMembers.has(userId)
          ? "alreadyInDestination"
          : "selectable"),
    status_message: null
  };
}

function fakeInviteTextMatches(value: string | null | undefined, loweredQuery: string): boolean {
  return value?.toLocaleLowerCase().includes(loweredQuery) ?? false;
}

function fakeValidMatrixUserId(value: string): boolean {
  const match = value.match(/^@[^:\s]+:[^:\s]+$/);
  return match !== null;
}

function fakeRoomHasMember(snapshot: DesktopSnapshot, roomId: string, userId: string): boolean {
  return (
    snapshot.state.domain.room_management.settings?.room_id === roomId &&
    snapshot.state.domain.room_management.settings.members.some((member) => member.user_id === userId)
  );
}

function ensureRoomLiveSignals(
  snapshot: DesktopSnapshot,
  roomId: string
): DesktopSnapshot["state"]["domain"]["live_signals"]["rooms"][string] {
  snapshot.state.domain.live_signals.rooms[roomId] ??= {
    receipts_by_event: {},
    fully_read_event_id: null,
    typing_user_ids: []
  };
  return snapshot.state.domain.live_signals.rooms[roomId];
}

function projectReceiptSummary(receipts: LiveReadReceipt[]): LiveEventReceiptSummary {
  const readers = [...receipts].sort((left, right) => {
    const byTimestamp = (right.timestamp_ms ?? 0) - (left.timestamp_ms ?? 0);
    return byTimestamp || left.user_id.localeCompare(right.user_id);
  });
  const totalCount = readers.length;
  return {
    readers,
    total_count: totalCount,
    overflow_count: 0
  };
}

function defaultLocaleDisplayProfile(): LocaleDisplayProfile {
  return resolveLocaleDisplayProfile({ language_tag: null, text_direction: "auto" });
}

function defaultTypographyDisplayProfile(): DesktopSnapshot["state"]["domain"]["typography_profile"] {
  return resolveTypographyDisplayProfile({ font: "system", emoji: "system" });
}

function resolveTypographyDisplayProfile(
  typography: DesktopSnapshot["state"]["domain"]["settings"]["values"]["typography"]
): DesktopSnapshot["state"]["domain"]["typography_profile"] {
  return {
    font: typography.font,
    emoji: typography.emoji,
    platform: "linux",
    font_asset: typography.font === "inter" ? "bundledPreferred" : "systemFallback",
    emoji_asset: typography.emoji === "twemojiColr" ? "bundledPreferred" : "systemFallback"
  };
}

function resolveLocaleDisplayProfile(locale: LocaleSettings): LocaleDisplayProfile {
  const parsed = parseLocale(locale.language_tag);
  const pseudoLocale = parsed?.pseudo_locale ?? "none";
  const catalogLocale =
    pseudoLocale === "accented" || pseudoLocale === "bidi"
      ? "pseudo"
      : parsed?.language === "ja"
        ? "ja"
        : "en";
  const lang =
    pseudoLocale === "accented"
      ? "en-XA"
      : pseudoLocale === "bidi"
        ? "ar-XB"
        : catalogLocale === "ja"
          ? "ja"
          : "en";
  const dir =
    locale.text_direction === "ltr" || locale.text_direction === "rtl"
      ? locale.text_direction
      : pseudoLocale === "bidi" || parsed?.direction === "rtl"
        ? "rtl"
        : "ltr";

  return {
    lang,
    dir,
    catalog_locale: catalogLocale,
    pseudo_locale: pseudoLocale,
    platform: "linux",
    modifier_labels: { primary: "Ctrl" }
  };
}

function parseLocale(
  rawTag: string | null
): { language: "en" | "ja" | "rtl"; direction: "ltr" | "rtl"; pseudo_locale: "none" | "accented" | "bidi" } | null {
  const normalized = rawTag?.trim().replaceAll("_", "-");
  if (!normalized) {
    return null;
  }
  const [primaryRaw, ...rest] = normalized.split("-");
  const primary = primaryRaw.toLowerCase();
  if (!/^[a-z]{2,3}$/.test(primary) || rest.some((subtag) => subtag.toLowerCase() === "x")) {
    return null;
  }
  if (!rest.every((subtag) => /^[a-z0-9]{1,8}$/i.test(subtag))) {
    return null;
  }
  const pseudo_locale =
    normalized.toLowerCase() === "en-xa"
      ? "accented"
      : normalized.toLowerCase() === "ar-xb"
        ? "bidi"
        : "none";

  if (primary === "en") {
    return { language: "en", direction: "ltr", pseudo_locale };
  }
  if (primary === "ja") {
    return { language: "ja", direction: "ltr", pseudo_locale };
  }
  if (["ar", "dv", "fa", "he", "ps", "sd", "ug", "ur", "yi"].includes(primary)) {
    return { language: "rtl", direction: "rtl", pseudo_locale };
  }
  return null;
}

function applySettingsPatch(
  values: DesktopSnapshot["state"]["domain"]["settings"]["values"],
  patch: SettingsPatch
): DesktopSnapshot["state"]["domain"]["settings"]["values"] {
  return {
    locale: patch.locale ?? values.locale,
    appearance: patch.appearance ?? values.appearance,
    typography: patch.typography ?? values.typography,
    keyboard: patch.keyboard ?? values.keyboard,
    notifications: patch.notifications ?? values.notifications,
    display: patch.display ?? values.display,
    media: patch.media ?? values.media,
    timeline: patch.timeline ?? values.timeline,
    search_crawler: patch.search_crawler ?? values.search_crawler,
    thread_list_order: patch.thread_list_order ?? values.thread_list_order,
    room_list_sort: patch.room_list_sort ?? values.room_list_sort
  };
}

function resolveComposerKeyActionFromSettings(
  sendShortcut: DesktopSnapshot["state"]["domain"]["settings"]["values"]["keyboard"]["composer_send_shortcut"],
  keyEvent: ComposerKeyEvent,
  options: ComposerResolverOptions
): ComposerResolvedAction {
  if (keyEvent.is_composing) {
    return "commitImeCandidate";
  }
  if (keyEvent.key === "escape") {
    return options.autocomplete_open ? "closeAutocomplete" : "cancel";
  }
  if (keyEvent.key !== "enter") {
    return "noop";
  }
  if (keyEvent.modifiers.shift || keyEvent.modifiers.alt) {
    return "insertNewline";
  }
  if (options.autocomplete_open) {
    return "acceptAutocomplete";
  }
  const wantsSend =
    sendShortcut === "enter" ||
    (sendShortcut === "modEnter" && (keyEvent.modifiers.ctrl || keyEvent.modifiers.meta));
  if (!wantsSend) {
    return "insertNewline";
  }
  return options.send_enabled ? "send" : "noop";
}

function emptySidebar() {
  return {
    active_space_id: null,
    account_home: {
      display_name: "Home",
      unread_count: 0,
      highlight_count: 0,
      is_active: true
    },
    space_rail: [],
    space_rooms: [],
    not_joined_space_rooms: [],
    global_dms: [],
    space_unread_count: 0,
    dm_unread_count: 0,
    space_highlight_count: 0,
    dm_highlight_count: 0
  };
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
    .map((message) => searchMessage(message, query))
    .filter((result): result is SearchResult => Boolean(result))
    .sort(
      (left, right) =>
        right.timestamp_ms - left.timestamp_ms ||
        right.score_millis - left.score_millis ||
        left.event_id.localeCompare(right.event_id)
    );
}

function searchMessage(message: TimelineMessage, query: string): SearchResult | null {
  const bodyRange = textRangeUtf16(message.body, query);
  if (bodyRange) {
    return {
      room_id: message.room_id,
      event_id: message.event_id,
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
      timestamp_ms: room.last_activity_ms ?? 0,
      unread: true,
      highlight: (room.highlight_count ?? 0) > 0,
      context_label: activityRowContextLabel(room, spacesById)
    }));
  return {
    recent: {
      rows: recentRows,
      next_batch: includeBackfill ? null : "browser-activity-recent-page-2"
    },
    unread: {
      rows: sortActivityRows(unreadPlaceholderRows),
      next_batch: null
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

function emptyMentionIntent(): MentionIntent {
  return { targets: [] };
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

function threadsListItemsForRoom(roomId: string): ThreadsListItem[] {
  return timelineMessages
    .filter((message) => message.room_id === roomId && message.reply_count > 0)
    .map((message) => {
      const replies = threadReplies
        .filter((reply) => reply.room_id === roomId && reply.root_event_id === message.event_id)
        .sort((left, right) => left.timestamp_ms - right.timestamp_ms);
      const latestReply = replies[replies.length - 1] ?? null;
      return {
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
