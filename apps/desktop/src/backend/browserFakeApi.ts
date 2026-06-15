import { composeSidebar, roomIsInScope, textRangeUtf16 } from "../domain/desktopModel";
import type {
  DesktopSnapshot,
  ComposerKeyEvent,
  ComposerResolvedAction,
  ComposerResolverOptions,
  ComposerSurface,
  RoomSummary,
  RoomTagKind,
  RoomTags,
  SavedSessionInfo,
  SearchResult,
  SearchScopeKind,
  SettingsPatch,
  PresenceKind,
  LocaleSettings,
  LocaleDisplayProfile,
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
  sendText(roomId: string, body: string): Promise<DesktopSnapshot>;
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
  setAvatar(mimeType: string, bytes: number[]): Promise<DesktopSnapshot>;
  editMessage(roomId: string, eventId: string, body: string): Promise<DesktopSnapshot>;
  redactMessage(roomId: string, eventId: string): Promise<DesktopSnapshot>;
  leaveRoom(roomId: string): Promise<DesktopSnapshot>;
  forgetRoom(roomId: string): Promise<DesktopSnapshot>;
  setRoomTag(roomId: string, tag: RoomTagKind, order?: number | null): Promise<DesktopSnapshot>;
  removeRoomTag(roomId: string, tag: RoomTagKind): Promise<DesktopSnapshot>;
  pinEvent(roomId: string, eventId: string): Promise<DesktopSnapshot>;
  unpinEvent(roomId: string, eventId: string): Promise<DesktopSnapshot>;
  openThread(roomId: string, rootEventId: string): Promise<DesktopSnapshot>;
  closeThread(): Promise<DesktopSnapshot>;
  setThreadComposerDraft(roomId: string, rootEventId: string, draft: string): Promise<DesktopSnapshot>;
  sendThreadReply(roomId: string, rootEventId: string, body: string): Promise<DesktopSnapshot>;
  submitSearch(query: string, scope: SearchScopeKind): Promise<DesktopSnapshot>;
  createRoom(name: string): Promise<DesktopSnapshot>;
  createSpace(name: string): Promise<DesktopSnapshot>;
  setSpaceChild(spaceId: string, childRoomId: string, viaServer: string): Promise<DesktopSnapshot>;
  acceptInvite(roomId: string): Promise<DesktopSnapshot>;
  declineInvite(roomId: string): Promise<DesktopSnapshot>;
  startDirectMessage(userId: string): Promise<DesktopSnapshot>;
  inviteUser(roomId: string, userId: string): Promise<DesktopSnapshot>;
  setComposerReplyTarget(roomId: string, eventId: string): Promise<DesktopSnapshot>;
  cancelComposerReply(): Promise<DesktopSnapshot>;
  sendReply(roomId: string, inReplyToEventId: string, body: string): Promise<DesktopSnapshot>;
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

  async sendText(roomId: string, body: string): Promise<DesktopSnapshot> {
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
    const existing = roomSignals.receipts_by_event[eventId] ?? [];
    roomSignals.receipts_by_event[eventId] = [
      ...existing.filter((receipt) => receipt.user_id !== session.user_id),
      { user_id: session.user_id, timestamp_ms: Date.now() }
    ];
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

  async createRoom(name: string): Promise<DesktopSnapshot> {
    if (!this.canUseSyncedViews()) {
      return this.getSnapshot();
    }

    const count = this.snapshot.state.rooms.length + 1;
    const newRoomId = `!local-room-${count}:fake.local`;
    const newRoom: RoomSummary = {
      room_id: newRoomId,
      display_name: name,
      avatar: null,
      is_dm: false,
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
      avatar: invite.avatar,
      is_dm: invite.is_dm,
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
      avatar: null,
      is_dm: true,
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

  async sendReply(roomId: string, inReplyToEventId: string, body: string): Promise<DesktopSnapshot> {
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
      directory: { kind: "closed" },
      room_management: { selected_room_id: null, operation: { kind: "idle" } },
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
      directory: { kind: "closed" },
      room_management: { selected_room_id: null, operation: { kind: "idle" } },
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
      keyboard: { composer_send_shortcut: "enter" }
    },
    persistence: { kind: "idle" }
  };
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
    keyboard: patch.keyboard ?? values.keyboard
  };
}

function resolveComposerKeyActionFromSettings(
  sendShortcut: DesktopSnapshot["state"]["settings"]["values"]["keyboard"]["composer_send_shortcut"],
  keyEvent: ComposerKeyEvent,
  options: ComposerResolverOptions
): ComposerResolvedAction {
  if (keyEvent.is_composing) {
    return "ignore";
  }
  if (keyEvent.key === "escape") {
    return "cancel";
  }
  if (keyEvent.key !== "enter") {
    return "ignore";
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
  return options.send_enabled ? "send" : "ignore";
}

function emptySidebar() {
  return {
    active_space_id: null,
    account_home: {
      display_name: "Home",
      unread_count: 0,
      is_active: true
    },
    space_rail: [],
    space_rooms: [],
    global_dms: [],
    space_unread_count: 0,
    dm_unread_count: 0
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

function clone<T>(value: T): T {
  return structuredClone(value);
}

function emptyRoomTags(): RoomTags {
  return {
    favourite: null,
    low_priority: null
  };
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
    avatar: null,
    is_dm: false,
    tags: emptyRoomTags(),
    unread_count: 8,
    parent_space_ids: ["!space-alpha:example.invalid"]
  },
  {
    room_id: "!room-planning:example.invalid",
    display_name: "planning-room",
    avatar: null,
    is_dm: false,
    tags: emptyRoomTags(),
    unread_count: 2,
    parent_space_ids: ["!space-alpha:example.invalid"]
  },
  {
    room_id: "!room-search:example.invalid",
    display_name: "matrix-sdk-search",
    avatar: null,
    is_dm: false,
    tags: emptyRoomTags(),
    unread_count: 1,
    parent_space_ids: ["!space-beta:example.invalid"]
  },
  {
    room_id: "!dm-member-1:example.invalid",
    display_name: "Member 1",
    avatar: null,
    is_dm: true,
    tags: emptyRoomTags(),
    unread_count: 1,
    parent_space_ids: []
  },
  {
    room_id: "!dm-member-2:example.invalid",
    display_name: "Member 2",
    avatar: null,
    is_dm: true,
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
