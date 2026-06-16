import { composeSidebar, roomIsInScope, textRangeUtf16 } from "../domain/desktopModel";
import type {
  ActivityMarkReadTarget,
  ActivityRow,
  ActivityStream,
  ActivityTab,
  DesktopSnapshot,
  ComposerKeyEvent,
  ComposerResolvedAction,
  ComposerResolverOptions,
  ComposerSurface,
  DirectoryQuery,
  RoomModerationAction,
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
  SpaceSummary,
  TimelineMessage
} from "../domain/types";

export interface DesktopApi {
  getSnapshot(): Promise<DesktopSnapshot>;
  discoverLoginMethods(homeserver: string): Promise<DesktopSnapshot>;
  submitLogin(
    homeserver: string,
    username: string,
    password: string,
    deviceDisplayName: string
  ): Promise<DesktopSnapshot>;
  listSavedSessions(): Promise<SavedSessionInfo[]>;
  switchAccount(session: SavedSessionInfo): Promise<DesktopSnapshot>;
  submitRecovery(secret: string): Promise<DesktopSnapshot>;
  restartSync(): Promise<DesktopSnapshot>;
  updateSettings(patch: SettingsPatch): Promise<DesktopSnapshot>;
  probeLocalEncryptionHealth(): Promise<DesktopSnapshot>;
  resetLocalData(): Promise<DesktopSnapshot>;
  bootstrapCrossSigning(): Promise<DesktopSnapshot>;
  enableKeyBackup(): Promise<DesktopSnapshot>;
  acceptVerification(flowId: number): Promise<DesktopSnapshot>;
  confirmSasVerification(flowId: number): Promise<DesktopSnapshot>;
  cancelVerification(flowId: number): Promise<DesktopSnapshot>;
  resetIdentity(): Promise<DesktopSnapshot>;
  submitIdentityResetPassword(flowId: number, password: string): Promise<DesktopSnapshot>;
  submitIdentityResetOAuth(flowId: number): Promise<DesktopSnapshot>;
  resolveComposerKeyAction(
    surface: ComposerSurface,
    keyEvent: ComposerKeyEvent,
    options: ComposerResolverOptions
  ): Promise<ComposerResolvedAction>;
  selectSpace(spaceId: string | null): Promise<DesktopSnapshot>;
  selectRoom(roomId: string): Promise<DesktopSnapshot>;
  selectSearchResult(roomId: string, eventId: string): Promise<DesktopSnapshot>;
  closeFocusedContext(): Promise<DesktopSnapshot>;
  paginateTimelineBackwards(roomId: string): Promise<DesktopSnapshot>;
  sendText(roomId: string, body: string, mentions?: MentionIntent): Promise<DesktopSnapshot>;
  retrySend(roomId: string, transactionId: string): Promise<DesktopSnapshot>;
  cancelSend(roomId: string, transactionId: string): Promise<DesktopSnapshot>;
  sendReaction(roomId: string, eventId: string, reactionKey: string): Promise<DesktopSnapshot>;
  redactReaction(
    roomId: string,
    eventId: string,
    reactionKey: string,
    reactionEventId: string
  ): Promise<DesktopSnapshot>;
  sendReadReceipt(roomId: string, eventId: string): Promise<DesktopSnapshot>;
  setFullyRead(roomId: string, eventId: string): Promise<DesktopSnapshot>;
  setTyping(roomId: string, isTyping: boolean): Promise<DesktopSnapshot>;
  setPresence(presence: PresenceKind): Promise<DesktopSnapshot>;
  setDisplayName(displayName: string | null): Promise<DesktopSnapshot>;
  setLocalUserAlias(userId: string, alias: string | null): Promise<DesktopSnapshot>;
  setAvatar(mimeType: string, bytes: number[]): Promise<DesktopSnapshot>;
  editMessage(roomId: string, eventId: string, body: string): Promise<DesktopSnapshot>;
  redactMessage(roomId: string, eventId: string): Promise<DesktopSnapshot>;
  loadMessageSource(roomId: string, eventId: string): Promise<DesktopSnapshot>;
  forwardMessage(
    roomId: string,
    sourceEventId: string,
    destinationRoomId: string
  ): Promise<DesktopSnapshot>;
  leaveRoom(roomId: string): Promise<DesktopSnapshot>;
  forgetRoom(roomId: string): Promise<DesktopSnapshot>;
  setRoomTag(roomId: string, tag: RoomTagKind, order?: number | null): Promise<DesktopSnapshot>;
  removeRoomTag(roomId: string, tag: RoomTagKind): Promise<DesktopSnapshot>;
  pinEvent(roomId: string, eventId: string): Promise<DesktopSnapshot>;
  unpinEvent(roomId: string, eventId: string): Promise<DesktopSnapshot>;
  openActivity(): Promise<DesktopSnapshot>;
  closeActivity(): Promise<DesktopSnapshot>;
  setActivityTab(tab: ActivityTab): Promise<DesktopSnapshot>;
  paginateActivity(tab: ActivityTab, cursor?: string | null): Promise<DesktopSnapshot>;
  markActivityRead(target: ActivityMarkReadTarget): Promise<DesktopSnapshot>;
  openThread(roomId: string, rootEventId: string): Promise<DesktopSnapshot>;
  closeThread(): Promise<DesktopSnapshot>;
  setThreadComposerDraft(roomId: string, rootEventId: string, draft: string): Promise<DesktopSnapshot>;
  sendThreadReply(roomId: string, rootEventId: string, body: string): Promise<DesktopSnapshot>;
  submitSearch(query: string, scope: SearchScopeKind): Promise<DesktopSnapshot>;
  queryDirectory(query: DirectoryQuery): Promise<DesktopSnapshot>;
  joinDirectoryRoom(alias: string, viaServer?: string | null): Promise<DesktopSnapshot>;
  loadRoomSettings(roomId: string): Promise<DesktopSnapshot>;
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
  createRoom(name: string): Promise<DesktopSnapshot>;
  createSpace(name: string): Promise<DesktopSnapshot>;
  setSpaceChild(spaceId: string, childRoomId: string, viaServer: string): Promise<DesktopSnapshot>;
  acceptInvite(roomId: string): Promise<DesktopSnapshot>;
  declineInvite(roomId: string): Promise<DesktopSnapshot>;
  startDirectMessage(userId: string): Promise<DesktopSnapshot>;
  inviteUser(roomId: string, userId: string): Promise<DesktopSnapshot>;
  setComposerReplyTarget(roomId: string, eventId: string): Promise<DesktopSnapshot>;
  cancelComposerReply(): Promise<DesktopSnapshot>;
  sendReply(
    roomId: string,
    inReplyToEventId: string,
    body: string,
    mentions?: MentionIntent
  ): Promise<DesktopSnapshot>;
}

export interface BrowserFakeApiOptions {
  restoreSession?: boolean;
  session?: "ready" | "signedOut" | "needsRecovery";
}

export function createBrowserFakeApi(options: BrowserFakeApiOptions = {}): DesktopApi {
  return new BrowserFakeApi(options);
}

class BrowserFakeApi implements DesktopApi {
  private snapshot: DesktopSnapshot;
  private requestSequence = 1_000;

  constructor(options: BrowserFakeApiOptions) {
    this.snapshot = createInitialSnapshot(initialSession(options));
  }

  async getSnapshot(): Promise<DesktopSnapshot> {
    return clone(this.snapshot);
  }

  async discoverLoginMethods(homeserver: string): Promise<DesktopSnapshot> {
    const normalizedHomeserver = normalizeHomeserver(homeserver);
    this.snapshot.state.auth = {
      kind: "ready",
      homeserver: normalizedHomeserver,
      flows: [
        {
          kind: "password",
          delegated_oidc_compatibility: false
        },
        {
          kind: "sso",
          delegated_oidc_compatibility: true
        }
      ]
    };

    return this.getSnapshot();
  }

  async submitLogin(
    homeserver: string,
    username: string,
    password: string,
    deviceDisplayName: string
  ): Promise<DesktopSnapshot> {
    this.snapshot.state.session = {
      kind: "authenticating",
      homeserver: normalizeHomeserver(homeserver)
    };
    this.snapshot.state.errors = this.snapshot.state.errors.filter(
      (error) => error.code !== "login_failed"
    );
    this.clearSessionViews();
    void username;
    void password;
    void deviceDisplayName;

    this.snapshot.state.session = { kind: "signedOut" };
    this.snapshot.state.errors.push({
      code: "login_failed",
      message: "real Matrix login is not wired in this pre-login foundation",
      recoverable: true
    });

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
    this.snapshot.state.session = {
      ...knownSession,
      kind: "switchingAccount"
    };
    this.snapshot.state.sync = "stopped";
    this.clearSessionViews();
    this.snapshot = createReadySnapshot(knownSession);
    return this.getSnapshot();
  }

  async submitRecovery(secret: string): Promise<DesktopSnapshot> {
    if (
      this.snapshot.state.session.kind !== "needsRecovery" &&
      this.snapshot.state.session.kind !== "recovering"
    ) {
      return this.getSnapshot();
    }

    this.snapshot.state.session = {
      ...this.snapshot.state.session,
      kind: "recovering"
    };
    this.snapshot.state.errors = this.snapshot.state.errors.filter(
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

    this.snapshot.state.navigation.active_space_id = spaceId;
    this.snapshot.sidebar = composeSidebar(
      this.snapshot.state.navigation.active_space_id,
      this.snapshot.state.spaces,
      this.snapshot.state.rooms
    );

    const firstRoom = this.snapshot.sidebar.space_rooms[0];
    if (firstRoom) {
      await this.selectRoom(firstRoom.room_id);
    }

    return this.getSnapshot();
  }

  async restartSync(): Promise<DesktopSnapshot> {
    if (this.canRestartSync()) {
      this.snapshot.state.sync = "running";
    }

    return this.getSnapshot();
  }

  async updateSettings(patch: SettingsPatch): Promise<DesktopSnapshot> {
    this.snapshot.state.settings.values = applySettingsPatch(
      this.snapshot.state.settings.values,
      patch
    );
    this.snapshot.state.locale_profile = resolveLocaleDisplayProfile(
      this.snapshot.state.settings.values.locale
    );
    this.snapshot.state.typography_profile = resolveTypographyDisplayProfile(
      this.snapshot.state.settings.values.typography
    );
    this.snapshot.state.settings.persistence = { kind: "idle" };
    return this.getSnapshot();
  }

  async probeLocalEncryptionHealth(): Promise<DesktopSnapshot> {
    const requestId = this.nextRequestId();
    this.snapshot.state.local_encryption = { kind: "probing", request_id: requestId };
    await Promise.resolve();
    this.snapshot.state.local_encryption = { kind: "healthy" };
    return this.getSnapshot();
  }

  async resetLocalData(): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    this.snapshot.state.local_encryption = {
      kind: "resetting",
      request_id: this.nextRequestId()
    };
    await Promise.resolve();
    this.snapshot.state.session = { kind: "signedOut" };
    this.snapshot.state.sync = "stopped";
    this.snapshot.state.local_encryption = { kind: "unknown" };
    this.clearSessionViews();
    return this.getSnapshot();
  }

  async bootstrapCrossSigning(): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    this.snapshot.state.e2ee_trust.cross_signing = { kind: "trusted" };
    return this.getSnapshot();
  }

  async enableKeyBackup(): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    this.snapshot.state.e2ee_trust.key_backup = {
      kind: "enabled",
      version: "browser-preview"
    };
    return this.getSnapshot();
  }

  async acceptVerification(flowId: number): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    const verification = this.snapshot.state.e2ee_trust.verification;
    if (verification.kind === "requested" && verification.request_id === flowId) {
      this.snapshot.state.e2ee_trust.verification = {
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

    const verification = this.snapshot.state.e2ee_trust.verification;
    if (
      (verification.kind === "sasPresented" || verification.kind === "confirming") &&
      verification.request_id === flowId
    ) {
      this.snapshot.state.e2ee_trust.verification = {
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

    const verification = this.snapshot.state.e2ee_trust.verification;
    if (verification.kind !== "idle" && verification.request_id === flowId) {
      this.snapshot.state.e2ee_trust.verification = { kind: "idle" };
    }
    return this.getSnapshot();
  }

  async resetIdentity(): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    this.snapshot.state.e2ee_trust.identity_reset = {
      kind: "awaitingAuth",
      request_id: this.nextRequestId(),
      auth_type: "uiaa"
    };
    return this.getSnapshot();
  }

  async submitIdentityResetPassword(flowId: number, password: string): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    void password;
    const identityReset = this.snapshot.state.e2ee_trust.identity_reset;
    if (identityReset.kind === "awaitingAuth" && identityReset.request_id === flowId) {
      this.completeIdentityReset();
    }
    return this.getSnapshot();
  }

  async submitIdentityResetOAuth(flowId: number): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    const identityReset = this.snapshot.state.e2ee_trust.identity_reset;
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
      this.snapshot.state.settings.values.keyboard.composer_send_shortcut,
      keyEvent,
      options
    );
  }

  async selectRoom(roomId: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    this.snapshot.state.navigation.active_room_id = roomId;
    this.snapshot.state.timeline.room_id = roomId;
    this.snapshot.state.timeline.is_subscribed = true;
    this.snapshot.state.thread = { kind: "closed" };
    this.snapshot.state.focused_context = { kind: "closed" };
    this.snapshot.thread = null;
    this.snapshot.timeline = timelineMessages.filter((message) => message.room_id === roomId);
    return this.getSnapshot();
  }

  async selectSearchResult(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    await this.selectRoom(roomId);
    this.snapshot.state.focused_context = {
      kind: "opening",
      room_id: roomId,
      event_id: eventId
    };
    return this.getSnapshot();
  }

  async closeFocusedContext(): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    this.snapshot.state.focused_context = { kind: "closed" };
    return this.getSnapshot();
  }

  async paginateTimelineBackwards(roomId: string): Promise<DesktopSnapshot> {
    if (
      !this.canUseSyncedViews() ||
      this.snapshot.state.timeline.room_id !== roomId ||
      this.snapshot.state.timeline.is_paginating_backwards
    ) {
      return this.getSnapshot();
    }

    this.snapshot.state.timeline.is_paginating_backwards = true;
    const existingEventIds = new Set(this.snapshot.timeline.map((message) => message.event_id));
    const olderMessages = backwardTimelineMessages.filter(
      (message) => message.room_id === roomId && !existingEventIds.has(message.event_id)
    );
    this.snapshot.timeline = [...olderMessages, ...this.snapshot.timeline];
    this.snapshot.state.timeline.is_paginating_backwards = false;
    return this.getSnapshot();
  }

  async sendText(
    roomId: string,
    body: string,
    mentions: MentionIntent = emptyMentionIntent()
  ): Promise<DesktopSnapshot> {
    void mentions;
    const session = this.snapshot.state.session;
    if (
      session.kind !== "ready" ||
      !session.user_id ||
      this.snapshot.state.timeline.room_id !== roomId ||
      body.trim().length === 0
    ) {
      return this.getSnapshot();
    }
    const sender = session.user_id;

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
    this.snapshot.state.timeline.composer.pending_transaction_id = null;
    this.snapshot.state.timeline.composer.draft = "";
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

  async sendReadReceipt(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    const session = this.snapshot.state.session;
    if (!this.isReady() || !session.user_id || eventId.trim().length === 0) {
      return this.getSnapshot();
    }
    const roomSignals = ensureRoomLiveSignals(this.snapshot, roomId);
    const existing = roomSignals.receipts_by_event[eventId]?.readers ?? [];
    roomSignals.receipts_by_event[eventId] = projectReceiptSummary([
      ...existing.filter((receipt) => receipt.user_id !== session.user_id),
      {
        user_id: session.user_id,
        display_name: this.snapshot.state.profile.own.display_name,
        original_display_label: this.snapshot.state.profile.own.display_name?.trim() || session.user_id,
        avatar: this.snapshot.state.profile.own.avatar,
        timestamp_ms: Date.now()
      }
    ]);
    return this.getSnapshot();
  }

  async setFullyRead(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    if (!this.isReady() || eventId.trim().length === 0) {
      return this.getSnapshot();
    }
    ensureRoomLiveSignals(this.snapshot, roomId).fully_read_event_id = eventId;
    return this.getSnapshot();
  }

  async setTyping(roomId: string, isTyping: boolean): Promise<DesktopSnapshot> {
    const session = this.snapshot.state.session;
    if (!this.isReady() || !session.user_id) {
      return this.getSnapshot();
    }
    const roomSignals = ensureRoomLiveSignals(this.snapshot, roomId);
    const withoutSelf = roomSignals.typing_user_ids.filter((userId) => userId !== session.user_id);
    roomSignals.typing_user_ids = isTyping ? [...withoutSelf, session.user_id] : withoutSelf;
    return this.getSnapshot();
  }

  async setPresence(presence: PresenceKind): Promise<DesktopSnapshot> {
    const session = this.snapshot.state.session;
    if (!this.isReady() || !session.user_id) {
      return this.getSnapshot();
    }
    this.snapshot.state.live_signals.presence[session.user_id] = presence;
    return this.getSnapshot();
  }

  async setDisplayName(displayName: string | null): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }
    const normalized = displayName?.trim() ? displayName.trim() : null;
    const requestId = this.nextRequestId();
    this.snapshot.state.profile.update = {
      kind: "settingDisplayName",
      request_id: requestId,
      display_name: normalized
    };
    this.snapshot.state.profile.own.display_name = normalized;
    this.snapshot.state.profile.update = { kind: "idle" };
    return this.getSnapshot();
  }

  async setLocalUserAlias(userId: string, alias: string | null): Promise<DesktopSnapshot> {
    if (!this.isReady() || userId.trim().length === 0) {
      return this.getSnapshot();
    }

    const normalizedUserId = userId.trim();
    const normalizedAlias = alias?.trim() ? alias.trim() : null;
    const requestId = this.nextRequestId();
    this.snapshot.state.profile.local_alias_update = {
      kind: "saving",
      request_id: requestId
    };

    await Promise.resolve();

    if (normalizedAlias) {
      this.snapshot.state.profile.local_aliases[normalizedUserId] = normalizedAlias;
    } else {
      delete this.snapshot.state.profile.local_aliases[normalizedUserId];
    }
    this.refreshLocalAliasProjections(normalizedUserId);
    this.snapshot.state.profile.local_alias_update = { kind: "idle" };
    return this.getSnapshot();
  }

  async setAvatar(mimeType: string, bytes: number[]): Promise<DesktopSnapshot> {
    if (!this.isReady() || bytes.length === 0) {
      return this.getSnapshot();
    }
    const requestId = this.nextRequestId();
    this.snapshot.state.profile.update = {
      kind: "settingAvatar",
      request_id: requestId,
      mime_type: mimeType,
      byte_count: bytes.length
    };
    this.snapshot.state.profile.own.avatar = {
      mxc_uri: "mxc://browser.fake/profile-avatar",
      thumbnail: { kind: "notRequested" }
    };
    this.snapshot.state.profile.update = { kind: "idle" };
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

  async forwardMessage(
    _roomId: string,
    _sourceEventId: string,
    _destinationRoomId: string
  ): Promise<DesktopSnapshot> {
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

    this.snapshot.state.thread = {
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

    this.snapshot.state.thread = { kind: "closed" };
    this.snapshot.thread = null;
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

    const thread = this.snapshot.state.thread;
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
    roomId: string,
    rootEventId: string,
    body: string
  ): Promise<DesktopSnapshot> {
    const session = this.snapshot.state.session;
    const thread = this.snapshot.state.thread;
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
      return this.getSnapshot();
    }

    thread.composer.pending_transaction_id = null;
    thread.composer.draft = "";
    return this.getSnapshot();
  }

  async submitSearch(query: string, scope: SearchScopeKind): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    const results = search(query, scope, this.snapshot);
    this.snapshot.state.search = {
      kind: "results",
      request_id: Date.now(),
      query,
      scope,
      results
    };
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
    this.snapshot.state.directory.query = {
      kind: "querying",
      request_id: requestId,
      query: normalizedQuery
    };

    await Promise.resolve();

    const alias = "#public-demo:fake.local";
    this.snapshot.state.directory.query = {
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
    this.snapshot.state.directory.join = {
      kind: "joining",
      request_id: requestId,
      alias: normalizedAlias,
      via_server: normalizedViaServer
    };

    await Promise.resolve();

    const roomId = `!joined-${this.snapshot.state.rooms.length + 1}:fake.local`;
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
      parent_space_ids: []
    };

    this.snapshot.state.rooms = [...this.snapshot.state.rooms, joinedRoom];
    this.snapshot.state.directory.join = { kind: "idle" };
    this.snapshot.sidebar = composeSidebar(
      this.snapshot.state.navigation.active_space_id,
      this.snapshot.state.spaces,
      this.snapshot.state.rooms
    );
    await this.selectRoom(roomId);
    return this.getSnapshot();
  }

  async loadRoomSettings(roomId: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || !roomId.trim()) {
      return this.getSnapshot();
    }

    const normalizedRoomId = roomId.trim();
    this.snapshot.state.room_management = {
      selected_room_id: normalizedRoomId,
      settings: this.roomSettingsSnapshot(normalizedRoomId),
      operation: { kind: "idle" }
    };
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
      this.snapshot.state.room_management.settings?.room_id === normalizedRoomId
        ? this.snapshot.state.room_management.settings
        : this.roomSettingsSnapshot(normalizedRoomId);
    const requestId = this.nextRequestId();

    if (!settings.permissions.can_edit_settings) {
      this.snapshot.state.room_management = {
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

    this.snapshot.state.room_management = {
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
    this.snapshot.state.room_management = {
      selected_room_id: normalizedRoomId,
      settings: updated,
      operation: { kind: "idle" }
    };
    this.snapshot.state.rooms = this.snapshot.state.rooms.map((room) =>
      room.room_id === normalizedRoomId && "name" in change
        ? {
            ...room,
            display_name: change.name ?? room.display_name,
            display_label: room.is_dm ? room.display_label : change.name ?? room.display_label
          }
        : room
    );
    this.snapshot.sidebar = composeSidebar(
      this.snapshot.state.navigation.active_space_id,
      this.snapshot.state.spaces,
      this.snapshot.state.rooms
    );
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
      this.snapshot.state.room_management.settings?.room_id === normalizedRoomId
        ? this.snapshot.state.room_management.settings
        : this.roomSettingsSnapshot(normalizedRoomId);
    const requestId = this.nextRequestId();

    if (!roomModerationAllowed(settings.permissions, action)) {
      this.snapshot.state.room_management = {
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

    this.snapshot.state.room_management = {
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
    this.snapshot.state.room_management = {
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
      this.snapshot.state.room_management.settings?.room_id === normalizedRoomId
        ? this.snapshot.state.room_management.settings
        : this.roomSettingsSnapshot(normalizedRoomId);
    const requestId = this.nextRequestId();

    if (!settings.permissions.can_edit_roles) {
      this.snapshot.state.room_management = {
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

    this.snapshot.state.room_management = {
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
    this.snapshot.state.room_management = {
      selected_room_id: normalizedRoomId,
      settings: updatedSettings,
      operation: { kind: "idle" }
    };
    return this.getSnapshot();
  }

  async createRoom(name: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    const count = this.snapshot.state.rooms.length + 1;
    const newRoomId = `!local-room-${count}:fake.local`;
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
      parent_space_ids: []
    };
    this.snapshot.state.rooms = [...this.snapshot.state.rooms, newRoom];
    this.snapshot.sidebar = composeSidebar(
      this.snapshot.state.navigation.active_space_id,
      this.snapshot.state.spaces,
      this.snapshot.state.rooms
    );
    await this.selectRoom(newRoomId);
    return this.getSnapshot();
  }

  async createSpace(name: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    const count = this.snapshot.state.spaces.length + 1;
    const newSpaceId = `!local-space-${count}:fake.local`;
    const newSpace: SpaceSummary = {
      space_id: newSpaceId,
      display_name: name,
      avatar: null,
      child_room_ids: []
    };
    this.snapshot.state.spaces = [...this.snapshot.state.spaces, newSpace];
    await this.selectSpace(newSpaceId);
    return this.getSnapshot();
  }

  async setSpaceChild(spaceId: string, childRoomId: string, viaServer: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }
    void viaServer;

    this.snapshot.state.spaces = this.snapshot.state.spaces.map((space) =>
      space.space_id === spaceId
        ? {
            ...space,
            child_room_ids: space.child_room_ids.includes(childRoomId)
              ? space.child_room_ids
              : [...space.child_room_ids, childRoomId]
          }
        : space
    );
    this.snapshot.state.rooms = this.snapshot.state.rooms.map((room) =>
      room.room_id === childRoomId
        ? {
            ...room,
            parent_space_ids: room.parent_space_ids.includes(spaceId)
              ? room.parent_space_ids
              : [...room.parent_space_ids, spaceId]
          }
        : room
    );
    this.snapshot.sidebar = composeSidebar(
      this.snapshot.state.navigation.active_space_id,
      this.snapshot.state.spaces,
      this.snapshot.state.rooms
    );
    return this.getSnapshot();
  }

  async acceptInvite(roomId: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    const invite = this.snapshot.state.invites.find((candidate) => candidate.room_id === roomId);
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
      parent_space_ids: []
    };
    this.snapshot.state.invites = this.snapshot.state.invites.filter(
      (candidate) => candidate.room_id !== roomId
    );
    this.snapshot.state.rooms = [...this.snapshot.state.rooms, joinedRoom];
    this.snapshot.sidebar = composeSidebar(
      this.snapshot.state.navigation.active_space_id,
      this.snapshot.state.spaces,
      this.snapshot.state.rooms
    );
    await this.selectRoom(roomId);
    return this.getSnapshot();
  }

  async declineInvite(roomId: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    this.snapshot.state.invites = this.snapshot.state.invites.filter(
      (candidate) => candidate.room_id !== roomId
    );
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

    const count = this.snapshot.state.rooms.filter((room) => room.is_dm).length + 1;
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
      parent_space_ids: []
    };
    this.snapshot.state.rooms = [...this.snapshot.state.rooms, newRoom];
    this.snapshot.sidebar = composeSidebar(
      this.snapshot.state.navigation.active_space_id,
      this.snapshot.state.spaces,
      this.snapshot.state.rooms
    );
    await this.selectRoom(newRoomId);
    return this.getSnapshot();
  }

  async inviteUser(roomId: string, userId: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || !roomId.trim() || !userId.trim()) {
      return this.getSnapshot();
    }

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

    this.snapshot.state.rooms = this.snapshot.state.rooms.map((room) =>
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
    this.snapshot.sidebar = composeSidebar(
      this.snapshot.state.navigation.active_space_id,
      this.snapshot.state.spaces,
      this.snapshot.state.rooms
    );
    return this.getSnapshot();
  }

  async removeRoomTag(roomId: string, tag: RoomTagKind): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    this.snapshot.state.rooms = this.snapshot.state.rooms.map((room) =>
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
    this.snapshot.sidebar = composeSidebar(
      this.snapshot.state.navigation.active_space_id,
      this.snapshot.state.spaces,
      this.snapshot.state.rooms
    );
    return this.getSnapshot();
  }

  async pinEvent(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || !roomId.trim() || !eventId.trim() || !this.hasRoom(roomId)) {
      return this.getSnapshot();
    }

    const entry = this.snapshot.state.room_interactions[roomId] ?? {
      pinned_events: [],
      pin_operation: { kind: "idle" as const }
    };
    const alreadyPinned = entry.pinned_events.some((event) => event.event_id === eventId);
    this.snapshot.state.room_interactions = {
      ...this.snapshot.state.room_interactions,
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

    const entry = this.snapshot.state.room_interactions[roomId] ?? {
      pinned_events: [],
      pin_operation: { kind: "idle" as const }
    };
    this.snapshot.state.room_interactions = {
      ...this.snapshot.state.room_interactions,
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
    this.snapshot.state.activity = {
      kind: "opening",
      request_id: requestId,
      tab: "recent"
    };

    await Promise.resolve();

    const streams = createActivityStreams(false);
    this.snapshot.state.activity = {
      kind: "open",
      active_tab: "recent",
      recent: streams.recent,
      unread: streams.unread,
      mark_read: { kind: "idle" }
    };
    return this.getSnapshot();
  }

  async closeActivity(): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    this.snapshot.state.activity = { kind: "closed" };
    return this.getSnapshot();
  }

  async setActivityTab(tab: ActivityTab): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || this.snapshot.state.activity.kind !== "open") {
      return this.getSnapshot();
    }

    this.snapshot.state.activity.active_tab = tab;
    return this.getSnapshot();
  }

  async paginateActivity(
    tab: ActivityTab,
    cursor: string | null = null
  ): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || this.snapshot.state.activity.kind !== "open") {
      return this.getSnapshot();
    }

    const normalizedCursor = cursor?.trim() ? cursor.trim() : null;
    if (tab !== "recent" || normalizedCursor === null) {
      return this.getSnapshot();
    }

    const existingEventIds = new Set(
      this.snapshot.state.activity.recent.rows.map((row) => row.event_id)
    );
    const olderRows = activityRows(backwardTimelineMessages, new Set())
      .filter((row) => !existingEventIds.has(row.event_id))
      .map((row) => ({ ...row, unread: false, highlight: false }));
    this.snapshot.state.activity.recent = {
      rows: [...this.snapshot.state.activity.recent.rows, ...olderRows],
      next_batch: null
    };
    return this.getSnapshot();
  }

  async markActivityRead(target: ActivityMarkReadTarget): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews() || this.snapshot.state.activity.kind !== "open") {
      return this.getSnapshot();
    }

    const requestId = this.nextRequestId();
    this.snapshot.state.activity.mark_read = {
      kind: "pending",
      request_id: requestId,
      target
    };

    await Promise.resolve();

    if (target.kind === "all") {
      this.snapshot.state.activity.unread = { rows: [], next_batch: null };
      this.snapshot.state.rooms = this.snapshot.state.rooms.map((room) => ({
        ...room,
        unread_count: 0
      }));
    } else {
      this.snapshot.state.activity.unread = {
        ...this.snapshot.state.activity.unread,
        rows: this.snapshot.state.activity.unread.rows.filter(
          (row) => row.room_id !== target.room_id
        )
      };
      this.snapshot.state.rooms = this.snapshot.state.rooms.map((room) =>
        room.room_id === target.room_id ? { ...room, unread_count: 0 } : room
      );
    }
    this.snapshot.state.activity.mark_read = { kind: "idle" };
    return this.getSnapshot();
  }

  async setComposerReplyTarget(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    if (this.snapshot.state.timeline.room_id === roomId) {
      this.snapshot.state.timeline.composer.mode = { Reply: { in_reply_to_event_id: eventId } };
    }
    return this.getSnapshot();
  }

  async cancelComposerReply(): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    this.snapshot.state.timeline.composer.mode = "Plain";
    return this.getSnapshot();
  }

  async sendReply(
    roomId: string,
    inReplyToEventId: string,
    body: string,
    mentions: MentionIntent = emptyMentionIntent()
  ): Promise<DesktopSnapshot> {
    void mentions;
    const session = this.snapshot.state.session;
    if (
      session.kind !== "ready" ||
      !session.user_id ||
      this.snapshot.state.timeline.room_id !== roomId ||
      body.trim().length === 0
    ) {
      return this.getSnapshot();
    }
    const sender = session.user_id;

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
    this.snapshot.state.timeline.composer.pending_transaction_id = null;
    this.snapshot.state.timeline.composer.draft = "";
    this.snapshot.state.timeline.composer.mode = "Plain";
    return this.getSnapshot();
  }

  private isReady() {
    return this.snapshot.state.session.kind === "ready";
  }

  private canUseSyncedViews() {
    const sessionKind = this.snapshot.state.session.kind;
    return (
      sessionKind === "ready" ||
      sessionKind === "needsRecovery" ||
      sessionKind === "recovering"
    );
  }

  private hasRoom(roomId: string): boolean {
    return this.snapshot.state.rooms.some((room) => room.room_id === roomId);
  }

  private roomSettingsSnapshot(roomId: string): RoomSettingsSnapshot {
    const room = this.snapshot.state.rooms.find((candidate) => candidate.room_id === roomId);
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
    const profiles = Object.values(this.snapshot.state.profile.users);
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
    const displayLabel = this.snapshot.state.profile.local_aliases[userId] ?? originalDisplayLabel;
    this.snapshot.state.profile.users[userId] = {
      ...profile,
      display_label: displayLabel,
      original_display_label: originalDisplayLabel,
      mention_search_terms: uniqueNonBlank([displayLabel, originalDisplayLabel, userId])
    };
    this.snapshot.state.rooms = this.snapshot.state.rooms.map((room) =>
      room.is_dm && room.dm_user_ids.includes(userId)
        ? {
            ...room,
            display_label: displayLabel,
            original_display_label: originalDisplayLabel
          }
        : room
    );
    this.snapshot.state.room_management =
      this.snapshot.state.room_management.settings === null
        ? this.snapshot.state.room_management
        : {
            ...this.snapshot.state.room_management,
            settings: {
              ...this.snapshot.state.room_management.settings,
              members: this.snapshot.state.room_management.settings.members.map((member) =>
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
    this.snapshot.sidebar = composeSidebar(
      this.snapshot.state.navigation.active_space_id,
      this.snapshot.state.spaces,
      this.snapshot.state.rooms
    );
  }

  private ensureUserProfile(userId: string): DesktopSnapshot["state"]["profile"]["users"][string] {
    const existing = this.snapshot.state.profile.users[userId];
    if (existing) {
      return existing;
    }
    const originalDisplayLabel =
      this.snapshot.state.rooms.find((room) => room.is_dm && room.dm_user_ids.includes(userId))
        ?.original_display_label.trim() || userId;
    const profile = {
      user_id: userId,
      display_name: originalDisplayLabel === userId ? null : originalDisplayLabel,
      display_label: originalDisplayLabel,
      original_display_label: originalDisplayLabel,
      mention_search_terms: uniqueNonBlank([originalDisplayLabel, userId]),
      avatar: null
    };
    this.snapshot.state.profile.users[userId] = profile;
    return profile;
  }

  private canRestartSync() {
    const sync = this.snapshot.state.sync;
    return (
      sync === "stopped" ||
      sync === "starting" ||
      (typeof sync === "object" && ("failed" in sync || "reconnecting" in sync))
    );
  }

  private nextRequestId(): number {
    const requestId = this.requestSequence;
    this.requestSequence += 1;
    return requestId;
  }

  private completeIdentityReset() {
    this.snapshot.state.e2ee_trust.identity_reset = { kind: "idle" };
    this.snapshot.state.e2ee_trust.cross_signing = { kind: "missing" };
    this.snapshot.state.e2ee_trust.key_backup = { kind: "disabled" };
    this.snapshot.state.e2ee_trust.devices = this.snapshot.state.e2ee_trust.devices.map(
      (device) => ({
        ...device,
        trust_level: "unverified"
      })
    );
  }

  private clearSessionViews() {
    this.snapshot.state.sync = "stopped";
    this.snapshot.state.navigation = {
      active_space_id: null,
      active_room_id: null
    };
    this.snapshot.state.spaces = [];
    this.snapshot.state.rooms = [];
    this.snapshot.state.invites = [];
    this.snapshot.state.timeline = {
      room_id: null,
      is_subscribed: false,
      is_paginating_backwards: false,
      composer: {
        pending_transaction_id: null,
        draft: "",
        mode: "Plain"
      }
    };
    this.snapshot.state.thread = { kind: "closed" };
    this.snapshot.state.focused_context = { kind: "closed" };
    this.snapshot.state.search = { kind: "closed" };
    this.snapshot.state.directory = defaultDirectoryState();
    this.snapshot.state.room_management = defaultRoomManagementState();
    this.snapshot.state.activity = { kind: "closed" };
    this.snapshot.state.basic_operation = { kind: "idle" };
    this.snapshot.state.profile = defaultProfileState(null);
    this.snapshot.state.e2ee_trust = defaultE2eeTrustState();
    this.snapshot.sidebar = emptySidebar();
    this.snapshot.timeline = [];
    this.snapshot.thread = null;
  }

  private async removeRoomFromFakeSnapshot(roomId: string): Promise<DesktopSnapshot> {
    if (!this.isReady()) {
      return this.getSnapshot();
    }

    this.snapshot.state.rooms = this.snapshot.state.rooms.filter((room) => room.room_id !== roomId);
    this.snapshot.state.spaces = this.snapshot.state.spaces.map((space) => ({
      ...space,
      child_room_ids: space.child_room_ids.filter((childRoomId) => childRoomId !== roomId)
    }));
    if (this.snapshot.state.navigation.active_room_id === roomId) {
      this.snapshot.state.navigation.active_room_id = null;
      this.snapshot.state.timeline.room_id = null;
      this.snapshot.state.timeline.is_subscribed = false;
      this.snapshot.timeline = [];
      this.snapshot.state.thread = { kind: "closed" };
      this.snapshot.thread = null;
    }
    this.snapshot.sidebar = composeSidebar(
      this.snapshot.state.navigation.active_space_id,
      this.snapshot.state.spaces,
      this.snapshot.state.rooms
    );
    return this.getSnapshot();
  }
}

function createInitialSnapshot(session: BrowserFakeApiOptions["session"]): DesktopSnapshot {
  if (session === "signedOut") {
    return createSignedOutSnapshot();
  }

  if (session === "needsRecovery") {
    return createNeedsRecoverySnapshot();
  }

  return createReadySnapshot();
}

function createReadySnapshot(session: SavedSessionInfo = savedSessions[0]): DesktopSnapshot {
  const active_space_id = "!space-alpha:example.invalid";
  const active_room_id = "!room-alpha:example.invalid";
  const sidebar = composeSidebar(active_space_id, spaces, rooms);
  const snapshot: DesktopSnapshot = {
    state: {
      session: {
        ...session,
        kind: "ready"
      },
      auth: { kind: "unknown" },
      settings: defaultSettingsState(),
      locale_profile: defaultLocaleDisplayProfile(),
      typography_profile: defaultTypographyDisplayProfile(),
      profile: defaultProfileState(session.user_id),
      sync: "running",
      navigation: {
        active_space_id,
        active_room_id
      },
      spaces,
      rooms,
      invites: [],
      room_interactions: {},
      directory: defaultDirectoryState(),
      room_management: defaultRoomManagementState(),
      activity: { kind: "closed" },
      timeline: {
        room_id: active_room_id,
        is_subscribed: true,
        is_paginating_backwards: false,
        composer: {
          pending_transaction_id: null,
          draft: "",
          mode: "Plain"
        }
      },
      thread: {
        kind: "open",
        room_id: active_room_id,
        root_event_id: "$alpha-update",
        is_subscribed: true,
        composer: {
          pending_transaction_id: null,
          draft: "",
          mode: "Plain"
        }
      },
      thread_attention: {
        kind: "tracking",
        room_id: active_room_id,
        root_event_id: "$alpha-update",
        notification_count: 0,
        highlight_count: 0,
        live_event_marker_count: 0
      },
      focused_context: { kind: "closed" },
      search: { kind: "closed" },
      errors: [],
      basic_operation: { kind: "idle" },
      live_signals: defaultLiveSignalsState(),
      e2ee_trust: defaultE2eeTrustState(),
      local_encryption: { kind: "unknown" },
      native_attention: defaultNativeAttentionState(),
      cjk_text_policy: defaultCjkTextPolicyState()
    },
    sidebar,
    timeline: timelineMessages.filter((message) => message.room_id === active_room_id),
    thread: {
      room_id: active_room_id,
      root_event_id: "$alpha-update",
      replies: threadReplies.filter(
        (reply) => reply.room_id === active_room_id && reply.root_event_id === "$alpha-update"
      )
    }
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
  snapshot.state.session = {
    ...savedSessions[0],
    kind: "needsRecovery",
    recovery_methods: ["recoveryKey", "securityPhrase"]
  };
  return snapshot;
}

function createSignedOutSnapshot(): DesktopSnapshot {
  return {
    state: {
      session: { kind: "signedOut" },
      auth: { kind: "unknown" },
      settings: defaultSettingsState(),
      locale_profile: defaultLocaleDisplayProfile(),
      typography_profile: defaultTypographyDisplayProfile(),
      profile: defaultProfileState(null),
      sync: "stopped",
      navigation: {
        active_space_id: null,
        active_room_id: null
      },
      spaces: [],
      rooms: [],
      invites: [],
      room_interactions: {},
      directory: defaultDirectoryState(),
      room_management: defaultRoomManagementState(),
      activity: { kind: "closed" },
      timeline: {
        room_id: null,
        is_subscribed: false,
        is_paginating_backwards: false,
        composer: {
          pending_transaction_id: null,
          draft: "",
          mode: "Plain"
        }
      },
      thread: { kind: "closed" },
      thread_attention: { kind: "closed" },
      focused_context: { kind: "closed" },
      search: { kind: "closed" },
      errors: [],
      basic_operation: { kind: "idle" },
      live_signals: defaultLiveSignalsState(),
      e2ee_trust: defaultE2eeTrustState(),
      local_encryption: { kind: "unknown" },
      native_attention: defaultNativeAttentionState(),
      cjk_text_policy: defaultCjkTextPolicyState()
    },
    sidebar: emptySidebar(),
    timeline: [],
    thread: null
  };
}

function defaultSettingsState(): DesktopSnapshot["state"]["settings"] {
  return {
    values: {
      locale: { language_tag: null, text_direction: "auto" },
      appearance: { theme: "system" },
      typography: { font: "system", emoji: "system" },
      keyboard: { composer_send_shortcut: "enter" },
      notifications: { desktop_notifications: true, sound: true, badges: true },
      display: { code_block_wrap: true, hide_redacted: false },
      media: {
        image_upload_compression: "never",
        image_upload_compression_policy: {
          threshold_bytes: 1048576,
          threshold_long_edge: 2560,
          target_long_edge: 2048,
          quality_percent: 82
        }
      }
    },
    persistence: { kind: "idle" }
  };
}

function defaultDirectoryState(): DesktopSnapshot["state"]["directory"] {
  return {
    query: { kind: "closed" },
    join: { kind: "idle" }
  };
}

function defaultRoomManagementState(): DesktopSnapshot["state"]["room_management"] {
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

function defaultE2eeTrustState(): DesktopSnapshot["state"]["e2ee_trust"] {
  return {
    verification: { kind: "idle" },
    cross_signing: { kind: "unknown" },
    key_backup: { kind: "unknown" },
    identity_reset: { kind: "idle" },
    devices: []
  };
}

function defaultLiveSignalsState(): DesktopSnapshot["state"]["live_signals"] {
  return {
    rooms: {},
    presence: {}
  };
}

function defaultNativeAttentionState(): DesktopSnapshot["state"]["native_attention"] {
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

function defaultCjkTextPolicyState(): DesktopSnapshot["state"]["cjk_text_policy"] {
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

function defaultProfileState(userId: string | null | undefined): DesktopSnapshot["state"]["profile"] {
  return {
    own: {
      display_name: userId ? "Demo User" : null,
      avatar: null
    },
    users: {},
    local_aliases: {},
    local_alias_update: { kind: "idle" },
    update: { kind: "idle" }
  };
}

function ensureRoomLiveSignals(
  snapshot: DesktopSnapshot,
  roomId: string
): DesktopSnapshot["state"]["live_signals"]["rooms"][string] {
  snapshot.state.live_signals.rooms[roomId] ??= {
    receipts_by_event: {},
    fully_read_event_id: null,
    typing_user_ids: []
  };
  return snapshot.state.live_signals.rooms[roomId];
}

function projectReceiptSummary(receipts: LiveReadReceipt[]): LiveEventReceiptSummary {
  const readers = [...receipts].sort((left, right) => {
    const byTimestamp = (right.timestamp_ms ?? 0) - (left.timestamp_ms ?? 0);
    return byTimestamp || left.user_id.localeCompare(right.user_id);
  });
  const totalCount = readers.length;
  return {
    readers: readers.slice(0, 3),
    total_count: totalCount,
    overflow_count: Math.max(0, totalCount - 3)
  };
}

function defaultLocaleDisplayProfile(): LocaleDisplayProfile {
  return resolveLocaleDisplayProfile({ language_tag: null, text_direction: "auto" });
}

function defaultTypographyDisplayProfile(): DesktopSnapshot["state"]["typography_profile"] {
  return resolveTypographyDisplayProfile({ font: "system", emoji: "system" });
}

function resolveTypographyDisplayProfile(
  typography: DesktopSnapshot["state"]["settings"]["values"]["typography"]
): DesktopSnapshot["state"]["typography_profile"] {
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
  values: DesktopSnapshot["state"]["settings"]["values"],
  patch: SettingsPatch
): DesktopSnapshot["state"]["settings"]["values"] {
  return {
    locale: patch.locale ?? values.locale,
    appearance: patch.appearance ?? values.appearance,
    typography: patch.typography ?? values.typography,
    keyboard: patch.keyboard ?? values.keyboard,
    notifications: patch.notifications ?? values.notifications,
    display: patch.display ?? values.display,
    media: patch.media ?? values.media
  };
}

function resolveComposerKeyActionFromSettings(
  sendShortcut: DesktopSnapshot["state"]["settings"]["values"]["keyboard"]["composer_send_shortcut"],
  keyEvent: ComposerKeyEvent,
  options: ComposerResolverOptions
): ComposerResolvedAction {
  if (keyEvent.is_composing) {
    return "commitImeCandidate";
  }
  if (keyEvent.key === "escape") {
    return "cancel";
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
        right.score_millis - left.score_millis ||
        left.timestamp_ms - right.timestamp_ms ||
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

function createActivityStreams(includeBackfill: boolean): {
  recent: ActivityStream;
  unread: ActivityStream;
} {
  const unreadRoomIds = new Set(
    rooms.filter((room) => room.unread_count > 0).map((room) => room.room_id)
  );
  const messages = includeBackfill
    ? [...timelineMessages, ...backwardTimelineMessages]
    : timelineMessages;
  return {
    recent: {
      rows: activityRows(messages, unreadRoomIds),
      next_batch: includeBackfill ? null : "browser-activity-recent-page-2"
    },
    unread: {
      rows: activityRows(
        timelineMessages.filter((message) => unreadRoomIds.has(message.room_id)),
        unreadRoomIds
      ),
      next_batch: null
    }
  };
}

function activityRows(
  messages: TimelineMessage[],
  unreadRoomIds: Set<string>
): ActivityRow[] {
  return messages
    .map((message) => {
      const room = rooms.find((candidate) => candidate.room_id === message.room_id);
      return {
        room_id: message.room_id,
        event_id: message.event_id,
        room_label: room?.display_name ?? "Unknown room",
        sender_label: message.sender,
        preview: message.body,
        timestamp_ms: message.timestamp_ms,
        unread: unreadRoomIds.has(message.room_id),
        highlight: message.event_id === "$alpha-update"
      };
    })
    .sort(
      (left, right) =>
        right.timestamp_ms - left.timestamp_ms || left.event_id.localeCompare(right.event_id)
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
    parent_space_ids: ["!space-alpha:example.invalid"]
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
    parent_space_ids: ["!space-alpha:example.invalid"]
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
    parent_space_ids: ["!space-beta:example.invalid"]
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
    parent_space_ids: []
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
    parent_space_ids: []
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
