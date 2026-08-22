import { describe, expect, test, vi } from "vitest";

import { createBrowserFakeApi } from "./browserFakeApi";
import { documentFromText, insertMention } from "../domain/composerDocument";
import { parseComposerDraftRevision as revision } from "../domain/composerDraftRevision";
import type {
  ComposerTarget,
  DesktopSnapshot,
  LiveReadReceipt,
  SecureBackupGateState
} from "../domain/types";

async function readyAccount(api: ReturnType<typeof createBrowserFakeApi>) {
  const session = (await api.getSnapshot()).state.domain.session;
  if (!session.homeserver || !session.user_id || !session.device_id) {
    throw new Error("expected ready browser-fake account");
  }
  return {
    homeserver: session.homeserver,
    userId: session.user_id,
    deviceId: session.device_id
  };
}

async function beginComposerLease(
  api: ReturnType<typeof createBrowserFakeApi>,
  account: Awaited<ReturnType<typeof readyAccount>>,
  target: ComposerTarget,
  rendererGeneration?: string
) {
  const generation =
    rendererGeneration ?? (await api.beginComposerDraftRendererGeneration());
  const lease = await api.acquireComposerDraftLease(
    {
      account: {
        homeserver: account.homeserver,
        user_id: account.userId,
        device_id: account.deviceId
      },
      target
    },
    generation
  );
  return { generation, lease };
}

function receipt(
  userId: string,
  displayName: string,
  timestampMs: number
): LiveReadReceipt {
  return {
    user_id: userId,
    display_name: displayName,
    original_display_label: displayName,
    avatar: null,
    timestamp_ms: timestampMs
  };
}

function resetSessionViewProjection(snapshot: DesktopSnapshot) {
  const { domain, ui } = snapshot.state;
  return {
    secure_backup_gate: domain.secure_backup_gate,
    current_session_status: domain.current_session_status,
    device_cleanup: domain.device_cleanup,
    account_management_capabilities: domain.account_management_capabilities,
    link_preview_settings: domain.link_preview_settings,
    room_preferences: domain.room_preferences,
    space_members: domain.space_members,
    invite_workflow: domain.invite_workflow,
    room_notification_settings: domain.room_notification_settings,
    room_interactions: domain.room_interactions,
    mention_candidates: domain.mention_candidates,
    thread_attention: domain.thread_attention,
    search_crawler: domain.search_crawler,
    live_signals: domain.live_signals,
    local_encryption: domain.local_encryption,
    native_attention: domain.native_attention,
    navigation: ui.navigation,
    threads_list: ui.threads_list,
    files_view: ui.files_view
  };
}

function sessionViewProjection(snapshot: DesktopSnapshot) {
  const { domain, ui } = snapshot.state;
  return {
    secure_backup_gate: domain.secure_backup_gate,
    current_session_status: domain.current_session_status,
    device_cleanup: domain.device_cleanup,
    account_management_capabilities: domain.account_management_capabilities,
    link_preview_settings: domain.link_preview_settings,
    room_preferences: domain.room_preferences,
    space_members: domain.space_members,
    invite_workflow: domain.invite_workflow,
    room_notification_settings: domain.room_notification_settings,
    room_interactions: domain.room_interactions,
    mention_candidates: domain.mention_candidates,
    thread_attention: domain.thread_attention,
    search_crawler: domain.search_crawler,
    live_signals: domain.live_signals,
    local_encryption: domain.local_encryption,
    native_attention: domain.native_attention,
    sync: domain.sync,
    spaces: domain.spaces,
    rooms: domain.rooms,
    invites: domain.invites,
    room_list: ui.room_list,
    navigation: ui.navigation,
    timeline_state: ui.timeline,
    thread_state: ui.thread,
    threads_list: ui.threads_list,
    focused_context: ui.focused_context,
    files_view: ui.files_view,
    search: domain.search,
    directory: domain.directory,
    room_management: domain.room_management,
    activity: domain.activity,
    device_sessions: domain.device_sessions,
    account_management: domain.account_management,
    soft_logout_reauth: domain.soft_logout_reauth,
    qr_login: domain.qr_login,
    basic_operation: ui.basic_operation,
    profile: domain.profile,
    e2ee_trust: domain.e2ee_trust,
    sidebar: snapshot.sidebar,
    timeline: snapshot.timeline,
    thread: snapshot.thread
  };
}

async function dirtyBrowserFakeSessionViews(api: ReturnType<typeof createBrowserFakeApi>) {
  const roomId = "!room-alpha:example.invalid";
  await api.refreshCurrentSessionStatus("manual");
  await api.loadAccountManagementCapabilities();
  await api.setRoomUrlPreviewOverride(roomId, false);
  await api.selectSpace("!space-alpha:example.invalid");
  await api.loadSpaceMembers("!space-alpha:example.invalid", 1);
  await api.setRoomNotificationMode(roomId, { kind: "mute" });
  await api.pinEvent(roomId, "$session-reset-pin");
  await api.queryMentionCandidates(roomId, "main", "ali");
  await api.openInviteWorkflow(roomId);
  await api.startRoomCrawl(roomId);
  await api.setPresence("online");
  await api.probeLocalEncryptionHealth();
  await api.openActivityEvent(roomId, "$alpha-update");
  await api.openThreadsList({ kind: "room", room_id: roomId });
  await api.openFilesView(
    { kind: "room", room_id: roomId },
    { kinds: ["image", "video", "audio", "file", "sticker"], filename_query: null },
    "newestFirst"
  );
}

const alphaRoomId = "!room-alpha:example.invalid";
const planningRoomId = "!room-planning:example.invalid";
const alphaSpaceId = "!space-alpha:example.invalid";
const betaSpaceId = "!space-beta:example.invalid";
const allAttachmentKinds = ["image", "video", "audio", "file", "sticker"] as const;

async function dirtyRoomOwnedState(
  api: ReturnType<typeof createBrowserFakeApi>,
  roomId: string,
  eventId: string
) {
  await api.setRoomUrlPreviewOverride(roomId, false);
  await api.setRoomNotificationMode(roomId, { kind: "mentions" });
  await api.pinEvent(roomId, `${eventId}-pin`);
  await api.queryMentionCandidates(roomId, "main", "member");
  await api.startRoomCrawl(roomId);
  await api.sendReadReceipt(roomId, eventId);
  await api.setFullyRead(roomId, eventId);
  await api.setTyping(roomId, true);
}

function roomOwnedProjection(snapshot: DesktopSnapshot, roomId: string) {
  const { domain, ui } = snapshot.state;
  const activityRows =
    domain.activity.kind === "open"
      ? [...domain.activity.recent.rows, ...domain.activity.unread.rows].filter(
          (row) => row.room_id === roomId
        )
      : [];
  const searchResults =
    domain.search.kind === "results"
      ? domain.search.results.filter((result) => result.room_id === roomId)
      : [];
  const threadsItems =
    ui.threads_list.kind === "open"
      ? ui.threads_list.items.filter((item) => item.room_id === roomId)
      : [];
  const filesItems = ui.files_view.kind === "open" ? ui.files_view.items.filter((item) => item.room_id === roomId) : [];

  return {
    room: domain.rooms.find((room) => room.room_id === roomId),
    roomPreference: domain.room_preferences.rooms[roomId],
    linkPreviewOverride: domain.link_preview_settings.room_overrides[roomId],
    notification: domain.room_notification_settings[roomId],
    interaction: domain.room_interactions[roomId],
    crawler: domain.search_crawler.rooms[roomId],
    crawlerLastActive:
      domain.search_crawler.last_active?.room_id === roomId
        ? domain.search_crawler.last_active
        : null,
    liveSignals: domain.live_signals.rooms[roomId],
    mentionTargets: domain.mention_candidates.targets.filter((target) => target.room_id === roomId),
    searchResults,
    activityRows,
    threadsItems,
    filesItems
  };
}

describe("BrowserFakeApi session-view reset", () => {
  test("locked construction exposes only the locked session boundary", async () => {
    const api = createBrowserFakeApi({ session: "locked" });
    const locked = await api.getSnapshot();
    const signedOut = await createBrowserFakeApi({ session: "signedOut" }).getSnapshot();
    const [savedSession] = await api.listSavedSessions();

    expect(locked.state_generation).toBe(0);
    expect(locked.state.domain.session).toEqual({ ...savedSession, kind: "locked" });
    expect(sessionViewProjection(locked)).toEqual(sessionViewProjection(signedOut));
  });

  test.each(["logout", "changeHomeserver", "failedSubmitLogin", "resetLocalData"] as const)(
    "%s clears every session-owned projection",
    async (operation) => {
      const api = createBrowserFakeApi();
      const signedOut = await createBrowserFakeApi({ session: "signedOut" }).getSnapshot();
      await dirtyBrowserFakeSessionViews(api);
      const dirty = await api.getSnapshot();

      expect(resetSessionViewProjection(dirty)).not.toEqual(
        resetSessionViewProjection(signedOut)
      );
      expect(dirty.state.ui.navigation.main_timeline_anchor).toEqual({
        event_id: "$alpha-update"
      });
      expect(dirty.state.domain.local_encryption.kind).toBe("healthy");
      expect(dirty.state.ui.threads_list.kind).toBe("open");
      expect(dirty.state.ui.files_view.kind).toBe("open");

      const snapshot =
        operation === "logout"
          ? await api.logout()
          : operation === "changeHomeserver"
            ? await api.changeHomeserver()
            : operation === "failedSubmitLogin"
              ? await api.submitLogin("https://example.invalid", "user", "password", "device", "linux")
              : await api.resetLocalData();

      expect(resetSessionViewProjection(snapshot)).toEqual(
        resetSessionViewProjection(signedOut)
      );
      expect(snapshot.state.domain.secure_backup_gate).toEqual({ kind: "inactive" });
      expect(snapshot.state.ui.navigation).toEqual({
        active_space_id: null,
        active_room_id: null,
        space_order: [],
        last_room_by_space_id: {}
      });
      expect(snapshot.state.ui.errors.filter((error) => error.code === "login_failed")).toHaveLength(
        operation === "failedSubmitLogin" ? 1 : 0
      );
    }
  );

  test.each(["completeOidcLogin", "switchAccount"] as const)(
    "%s replacement returns canonical ready projections",
    async (operation) => {
      const api = createBrowserFakeApi();
      const ready = await createBrowserFakeApi().getSnapshot();
      await dirtyBrowserFakeSessionViews(api);

      const snapshot =
        operation === "completeOidcLogin"
          ? await api.completeOidcLogin("https://example.invalid", "http://localhost/callback")
          : await api.switchAccount((await api.listSavedSessions())[1]);

      expect(snapshot.state.domain.session.kind).toBe("ready");
      expect(sessionViewProjection(snapshot)).toEqual(sessionViewProjection(ready));
    }
  );
});

describe("BrowserFakeApi Space member audit", () => {
  const spaceId = "!space-alpha:example.invalid";
  const childOnlyUserId = "@child-only:example.invalid";

  test("starts with joined, invited, child-only, and incomplete fixtures", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();
    const members = snapshot.state.domain.space_members;

    expect(members.selected_space_id).toBe(spaceId);
    expect(members.space_joined.map((entry) => entry.user_id)).toContain(
      "@joined:example.invalid"
    );
    expect(members.space_invited.map((entry) => entry.user_id)).toContain(
      "@invited:example.invalid"
    );
    expect(members.child_room_only.map((entry) => entry.user_id)).toContain(
      childOnlyUserId
    );
    expect(members.child_room_count).toBe(2);
    expect(members.complete_child_room_count).toBe(1);
    expect(members.incomplete_child_room_count).toBe(1);
  });

  test("loads the requested Space generation and preserves classified sections", async () => {
    const api = createBrowserFakeApi();

    const snapshot = await api.loadSpaceMembers(spaceId, 1);

    expect(snapshot.state.domain.space_members).toMatchObject({
      selected_space_id: spaceId,
      generation: 1,
      operation: { kind: "idle" },
      space_joined: expect.any(Array),
      space_invited: expect.any(Array),
      child_room_only: expect.any(Array)
    });
  });

  test("ignores a load from the wrong Space without changing the active projection", async () => {
    const api = createBrowserFakeApi();
    const before = await api.getSnapshot();

    const after = await api.loadSpaceMembers("!space-beta:example.invalid", 1);

    expect(after).toEqual(before);
  });

  test("ignores a stale or future generation without changing the active projection", async () => {
    const api = createBrowserFakeApi();
    const before = await api.getSnapshot();

    const stale = await api.loadSpaceMembers(spaceId, 0);
    const future = await api.loadSpaceMembers(spaceId, 2);

    expect(stale).toEqual(before);
    expect(future).toEqual(before);
  });

  test("loads only the active Space and permits its initial load when selection is unset", async () => {
    const api = createBrowserFakeApi();
    const initial = await api.getSnapshot();
    const mutable = api as unknown as { snapshot: DesktopSnapshot };
    mutable.snapshot.state.domain.space_members = {
      ...initial.state.domain.space_members,
      selected_space_id: null,
      operation: { kind: "idle" }
    };

    const rejected = await api.loadSpaceMembers("!space-beta:example.invalid", 99);
    expect(rejected.state.domain.space_members.selected_space_id).toBeNull();

    const snapshot = await api.loadSpaceMembers(spaceId, 99);

    expect(snapshot.state.domain.space_members).toMatchObject({
      selected_space_id: spaceId,
      generation: 99
    });
  });

  test("does not let a late load completion clear a newer member operation", async () => {
    const api = createBrowserFakeApi({ spaceMemberInviteOutcome: "pending" });
    const initial = await api.getSnapshot();
    const childOnly = initial.state.domain.space_members.child_room_only[0];
    expect(childOnly).toBeDefined();

    const staleLoad = api.loadSpaceMembers(spaceId, 1);
    void api.selectSpace(null);
    void api.selectSpace(spaceId);

    const mutable = api as unknown as { snapshot: DesktopSnapshot };
    const currentMembers = mutable.snapshot.state.domain.space_members;
    mutable.snapshot.state.domain.space_members = {
      ...currentMembers,
      child_room_only: [childOnly!],
      operation: { kind: "idle" }
    };
    const newerInvite = api.inviteUserToSpace(spaceId, childOnly!.user_id, currentMembers.generation);

    const snapshot = await staleLoad;
    await newerInvite;

    expect(snapshot.state.domain.space_members.operation).toMatchObject({
      kind: "inviting",
      space_id: spaceId,
      user_id: childOnly!.user_id,
      generation: currentMembers.generation
    });
  });

  test("exposes room-scoped profile cache entries in the snapshot contract", async () => {
    const api = createBrowserFakeApi();
    const profile = (await api.getSnapshot()).state.domain.profile;

    expect(profile.room_users).toEqual({});
  });

  test("does not replace an in-flight invite with a load operation", async () => {
    const api = createBrowserFakeApi({ spaceMemberInviteOutcome: "pending" });
    const before = await api.inviteUserToSpace(spaceId, childOnlyUserId, 1);

    const after = await api.loadSpaceMembers(spaceId, 1);

    expect(after).toEqual(before);
  });

  test("switching Spaces fences and clears the previous member projection", async () => {
    const api = createBrowserFakeApi();

    const snapshot = await api.selectSpace("!space-beta:example.invalid");

    expect(snapshot.state.domain.space_members).toMatchObject({
      selected_space_id: "!space-beta:example.invalid",
      generation: 2,
      space_joined: [],
      space_invited: [],
      child_room_only: [],
      operation: { kind: "idle" }
    });
  });

  test("keeps an invite in the pending operation state when settlement is deferred", async () => {
    const api = createBrowserFakeApi({ spaceMemberInviteOutcome: "pending" });

    const snapshot = await api.inviteUserToSpace(spaceId, childOnlyUserId, 1);
    const members = snapshot.state.domain.space_members;

    expect(members.child_room_only.map((entry) => entry.user_id)).not.toContain(
      childOnlyUserId
    );
    expect(members.space_invited).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          user_id: childOnlyUserId,
          invite_pending: true,
          membership: "space_invited"
        })
      ])
    );
    expect(members.operation).toMatchObject({
      kind: "inviting",
      space_id: spaceId,
      user_id: childOnlyUserId,
      generation: 1
    });
  });

  test("settles a successful fake invite as a non-pending Space invitation", async () => {
    const api = createBrowserFakeApi({ spaceMemberInviteOutcome: "success" });

    const snapshot = await api.inviteUserToSpace(spaceId, childOnlyUserId, 1);
    const members = snapshot.state.domain.space_members;

    expect(members.space_invited).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          user_id: childOnlyUserId,
          invite_pending: false,
          membership: "space_invited"
        })
      ])
    );
    expect(members.operation).toEqual({ kind: "idle" });
  });

  test("returns a failed fake invite to the child-only section", async () => {
    const api = createBrowserFakeApi({ spaceMemberInviteOutcome: "failure" });

    const snapshot = await api.inviteUserToSpace(spaceId, childOnlyUserId, 1);
    const members = snapshot.state.domain.space_members;

    expect(members.child_room_only.map((entry) => entry.user_id)).toContain(
      childOnlyUserId
    );
    expect(members.space_invited.map((entry) => entry.user_id)).not.toContain(
      childOnlyUserId
    );
    expect(members.operation).toMatchObject({
      kind: "failed",
      space_id: spaceId,
      user_id: childOnlyUserId,
      generation: 1,
      failureKind: "sdk"
    });
  });

  test("cancels an invited fake Space member and settles idle", async () => {
    const api = createBrowserFakeApi();

    const snapshot = await api.cancelSpaceInvite(
      spaceId,
      "@invited:example.invalid",
      1
    );
    const members = snapshot.state.domain.space_members;

    expect(members.space_invited.map((entry) => entry.user_id)).not.toContain(
      "@invited:example.invalid"
    );
    expect(members.operation).toEqual({ kind: "idle" });
  });

  test("keeps the fake cancellation operation fenced while settlement is pending", async () => {
    const api = createBrowserFakeApi({
      spaceMemberInviteCancellationOutcome: "pending"
    });

    const snapshot = await api.cancelSpaceInvite(
      spaceId,
      "@invited:example.invalid",
      1
    );

    expect(snapshot.state.domain.space_members.operation).toMatchObject({
      kind: "cancellingInvite",
      space_id: spaceId,
      user_id: "@invited:example.invalid",
      generation: 1
    });
    expect(snapshot.state.domain.space_members.space_invited).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          user_id: "@invited:example.invalid",
          membership: "space_invited"
        })
      ])
    );
  });

  test("does not cancel a joined or non-invited fake Space member", async () => {
    const api = createBrowserFakeApi();

    const snapshot = await api.cancelSpaceInvite(
      spaceId,
      "@joined:example.invalid",
      1
    );

    expect(snapshot.state.domain.space_members.space_joined.map((entry) => entry.user_id)).toContain(
      "@joined:example.invalid"
    );
    expect(snapshot.state.domain.space_members.operation).toEqual({ kind: "idle" });
  });

  test("rejects cancellation admission for a target absent from the invited projection", async () => {
    const api = createBrowserFakeApi();
    const before = await api.getSnapshot();

    const rejected = await api.cancelSpaceInvite(
      spaceId,
      "@missing:example.invalid",
      1
    );

    expect(rejected).toEqual(before);
    expect(rejected.state.domain.space_members.operation).toEqual({ kind: "idle" });
  });

  test("reconciles a locally invited target that is already joined on the server", async () => {
    const api = createBrowserFakeApi({
      spaceMemberInviteCancellationOutcome: "notInvited"
    });

    const snapshot = await api.cancelSpaceInvite(
      spaceId,
      "@invited:example.invalid",
      1
    );
    const members = snapshot.state.domain.space_members;
    const joinedEntry = members.space_joined.find(
      (entry) => entry.user_id === "@invited:example.invalid"
    );

    expect(members.space_invited.map((entry) => entry.user_id)).not.toContain(
      "@invited:example.invalid"
    );
    expect(joinedEntry).toMatchObject({
      user_id: "@invited:example.invalid",
      membership: "space_joined",
      invite_pending: false
    });
    expect(members.operation).toEqual({ kind: "idle" });
  });

  test("retains the invited fake member when cancellation transport rejects", async () => {
    const api = createBrowserFakeApi({ spaceMemberInviteCancellationOutcome: "failure" });

    const snapshot = await api.cancelSpaceInvite(
      spaceId,
      "@invited:example.invalid",
      1
    );
    const members = snapshot.state.domain.space_members;

    expect(members.space_invited.map((entry) => entry.user_id)).toContain(
      "@invited:example.invalid"
    );
    expect(members.operation).toMatchObject({
      kind: "failed",
      space_id: spaceId,
      user_id: "@invited:example.invalid",
      generation: 1,
      failureKind: "sdk"
    });
  });

  test("retries a failed cancellation through the fake transport for the exact context", async () => {
    const api = createBrowserFakeApi({
      spaceMemberInviteCancellationOutcomes: ["failure", "success"]
    });

    const failed = await api.cancelSpaceInvite(
      spaceId,
      "@invited:example.invalid",
      1
    );
    expect(failed.state.domain.space_members.operation).toMatchObject({
      kind: "failed",
      space_id: spaceId,
      user_id: "@invited:example.invalid",
      generation: 1
    });

    const retried = await api.cancelSpaceInvite(
      spaceId,
      "@invited:example.invalid",
      1
    );
    expect(retried.state.domain.space_members.space_invited.map((entry) => entry.user_id)).not.toContain(
      "@invited:example.invalid"
    );
    expect(retried.state.domain.space_members.operation).toEqual({ kind: "idle" });
  });

  test("rejects stale-generation cancellation admission without changing state", async () => {
    const api = createBrowserFakeApi();
    const before = await api.getSnapshot();

    const stale = await api.cancelSpaceInvite(
      spaceId,
      "@invited:example.invalid",
      0
    );

    expect(stale).toEqual(before);
  });
});

describe("BrowserFakeApi secure backup gate fixtures", () => {
  test("defaults a ready session to a ready secure backup gate", async () => {
    const snapshot = await createBrowserFakeApi().getSnapshot();

    expect(snapshot.state.domain.session.kind).toBe("ready");
    expect(snapshot.state.domain.secure_backup_gate).toEqual({ kind: "ready" });
  });

  test.each([
    { kind: "checking" },
    { kind: "setupRequired" },
    { kind: "explicitlyDisabledRequiresSetup" },
    { kind: "uploadingExistingKeys", pending: "two_to_ten" },
    { kind: "blockedFailed", failure: "forbidden" }
  ] satisfies SecureBackupGateState[])(
    "can seed the Rust-shaped non-ready gate fixture %#",
    async (secureBackupGate) => {
      const snapshot = await createBrowserFakeApi({ secureBackupGate }).getSnapshot();

      expect(snapshot.state.domain.secure_backup_gate).toEqual(secureBackupGate);
    }
  );

  test("uses dedicated secure-backup recovery and re-enable operations", async () => {
    const api = createBrowserFakeApi({
      secureBackupGate: { kind: "existingBackupNeedsRecovery" }
    });
    const legacyRecovery = vi.spyOn(api, "submitRecovery");
    const legacyEnable = vi.spyOn(api, "enableKeyBackup");
    const legacyBootstrap = vi.spyOn(api, "bootstrapSecureBackup");

    const recovered = await api.recoverSecureBackup("synthetic-recovery-key");
    const setup = await api.setupSecureBackup("synthetic-passphrase", "/tmp/recovery-key.txt");
    const reenabled = await api.reenableSecureBackup(
      "reenable-passphrase",
      "/tmp/reenable-recovery-key.txt"
    );

    expect(legacyRecovery).not.toHaveBeenCalled();
    expect(legacyEnable).not.toHaveBeenCalled();
    expect(legacyBootstrap).not.toHaveBeenCalled();
    expect(recovered.state.domain.secure_backup_gate).toEqual({ kind: "ready" });
    expect(setup.state.domain.secure_backup_gate).toEqual({ kind: "ready" });
    expect(reenabled.state.domain.secure_backup_gate).toEqual({ kind: "ready" });
  });

  test("exposes a dedicated inspection retry instead of using getSnapshot as the command", async () => {
    const api = createBrowserFakeApi({
      secureBackupGate: { kind: "blockedFailed", failure: "network" }
    });
    const getSnapshot = vi.spyOn(api, "getSnapshot");

    const retried = await api.retrySecureBackupInspection();

    expect(getSnapshot).not.toHaveBeenCalled();
    expect(retried.state.domain.secure_backup_gate).toEqual({
      kind: "blockedFailed",
      failure: "network"
    });
  });
});

describe("BrowserFakeApi settings preview", () => {
  test("verification retries clear the completed attempt failure", async () => {
    for (const method of ["existingDeviceSas", "recoveryKey"] as const) {
      const api = createBrowserFakeApi();
      const mutable = api as unknown as { snapshot: DesktopSnapshot };
      mutable.snapshot.state.domain.session = {
        kind: "awaitingVerification",
        homeserver: "https://example.invalid",
        user_id: "@gate:example.invalid",
        device_id: "DEVICE",
        gate: {
          methods: [method],
          account_kind: "existingIdentity",
          failureKind: "timeout"
        }
      };

      const retried = method === "existingDeviceSas"
        ? await api.startOwnUserSas()
        : await api.submitRecovery("synthetic-recovery-key");

      expect(retried.state.domain.session).toMatchObject({
        kind: "verifying",
        method,
        gate: { failureKind: null }
      });
    }

    const api = createBrowserFakeApi();
    const mutable = api as unknown as { snapshot: DesktopSnapshot };
    mutable.snapshot.state.domain.session = {
      kind: "awaitingVerification",
      homeserver: "https://example.invalid",
      user_id: "@gate:example.invalid",
      device_id: "DEVICE",
      gate: {
        methods: ["bootstrap"],
        account_kind: "newIdentity",
        failureKind: "timeout"
      }
    };
    const bootstrapped = await api.startSessionBootstrap(
      "synthetic-passphrase",
      "/tmp/synthetic-recovery-key.txt"
    );
    expect(bootstrapped.state.domain.session).toMatchObject({
      kind: "awaitingBootstrapConfirmation",
      gate: { failureKind: null }
    });
  });

  test("gate SAS confirm rechecks trust only for the matching flow", async () => {
    const api = createBrowserFakeApi();
    const mutable = api as unknown as { snapshot: DesktopSnapshot };
    mutable.snapshot.state.domain.session = {
      kind: "verifying",
      homeserver: "https://example.invalid",
      user_id: "@gate:example.invalid",
      device_id: "DEVICE",
      method: "existingDeviceSas",
      flow_id: 51,
      gate: { methods: ["existingDeviceSas"], account_kind: "existingIdentity", failureKind: null }
      ,
      sas_emojis: []
    };
    expect((await api.confirmSasVerification(50)).state.domain.session.flow_id).toBe(51);
    const confirmed = await api.confirmSasVerification(51);
    expect(confirmed.state.domain.session).toMatchObject({
      kind: "provisional",
      phase: { recheckingTrust: { failureKind: null } }
    });
    expect(confirmed.state.domain.e2ee_trust.verification).toEqual({ kind: "idle" });
  });
  test("projects disabled badge settings to zero without frontend recomputation", async () => {
    const api = createBrowserFakeApi();
    const mutable = api as unknown as { snapshot: DesktopSnapshot };
    mutable.snapshot.state.domain.native_attention.summary.unread_count = 6;
    mutable.snapshot.state.domain.native_attention.summary.badge_count = 6;
    mutable.snapshot.state.domain.native_attention.summary.capabilities.badge = "available";
    const snapshot = await api.updateSettings({
      notifications: {
        ...mutable.snapshot.state.domain.settings.values.notifications,
        badges: false
      }
    });
    expect(snapshot.state.domain.native_attention.summary.badge_count).toBe(0);
  });
  test("deduplicates main submissions by id and exposes accepted terminal snapshot fields", async () => {
    const api = createBrowserFakeApi();
    const roomId = "!room-alpha:example.invalid";
    await api.selectRoom(roomId);
    const account = await readyAccount(api);
    const { generation, lease } = await beginComposerLease(
      api,
      account,
      { kind: "main", room_id: roomId }
    );
    const before = (await api.getSnapshot()).timeline.length;

    const first = await api.sendText(
      account,
      lease.leaseId,
      generation,
      "submission-same",
      roomId,
      documentFromText("original")
    );
    const replay = await api.sendText(
      account,
      lease.leaseId,
      generation,
      "submission-same",
      roomId,
      documentFromText("changed")
    );

    expect(first.outcome).toBe("accepted");
    expect(replay.transactionId).toBe(first.transactionId);
    expect(replay.snapshot.timeline).toHaveLength(before + 1);
    expect(replay.snapshot.timeline.at(-1)?.body).toBe("original");
    expect(replay.snapshot.state.ui.timeline.composer.pending_submission_id).toBeNull();
    expect(replay.snapshot.state.ui.timeline.composer.accepted_submission_ids).toContain("submission-same");
  });

  test("reuses a submission id after an account switch with a fresh composer lease", async () => {
    const api = createBrowserFakeApi();
    const roomId = "!room-alpha:example.invalid";
    await api.selectRoom(roomId);
    const sessions = await api.listSavedSessions();
    const accountA = await readyAccount(api);
    const firstLease = await beginComposerLease(api, accountA, {
      kind: "main",
      room_id: roomId
    });
    await api.sendText(
      accountA,
      firstLease.lease.leaseId,
      firstLease.generation,
      "session-reuse",
      roomId,
      documentFromText("old body")
    );
    await api.switchAccount(sessions[1]!);
    await api.switchAccount(sessions[0]!);
    await api.selectRoom(roomId);
    const freshLease = await beginComposerLease(api, accountA, {
      kind: "main",
      room_id: roomId
    });
    const before = (await api.getSnapshot()).timeline.length;
    const response = await api.sendText(
      accountA,
      freshLease.lease.leaseId,
      freshLease.generation,
      "session-reuse",
      roomId,
      documentFromText("new body")
    );
    expect(response.snapshot.timeline).toHaveLength(before + 1);
    expect(response.snapshot.timeline.at(-1)?.body).toBe("new body");
  });

  test("draft snapshots retain structured mention identity instead of inferring display text", async () => {
    const api = createBrowserFakeApi();
    const roomId = "!room-alpha:example.invalid";
    await api.selectRoom(roomId);
    const account = await readyAccount(api);
    const { generation, lease } = await beginComposerLease(api, account, {
      kind: "main",
      room_id: roomId
    });
    const target = {
      kind: "user" as const,
      user_id: "@alice:example.invalid",
      display_label: "Same Name"
    };
    const document = insertMention(documentFromText("hello "), 6, 6, target, "Same Name");

    const snapshot = await api.setComposerDraft(
      account,
      lease.leaseId,
      generation,
      roomId,
      document,
      revision("1")
    );

    expect(snapshot.state.ui.timeline.composer.draft).toBe("hello @Same Name");
    expect(snapshot.state.ui.timeline.composer.document).toEqual(document);
    expect(snapshot.state.ui.timeline.composer.document.inlines.at(-1)).toMatchObject({
      kind: "mention",
      target: { user_id: "@alice:example.invalid" }
    });
  });

  test("revokes composer leases when returning to a logged-out saved account", async () => {
    const api = createBrowserFakeApi();
    const roomId = "!room-alpha:example.invalid";
    await api.selectRoom(roomId);
    const sessions = await api.listSavedSessions();
    const account = await readyAccount(api);
    const target = { kind: "main" as const, room_id: roomId };
    const scope = {
      account: {
        homeserver: account.homeserver,
        user_id: account.userId,
        device_id: account.deviceId
      },
      target
    };
    const rendererGeneration = await api.beginComposerDraftRendererGeneration();
    const lease = await api.acquireComposerDraftLease(scope, rendererGeneration);

    await api.logout();
    await api.switchAccount(sessions[0]!);
    await api.selectRoom(roomId);

    await expect(
      api.setComposerDraft(
        account,
        lease.leaseId,
        rendererGeneration,
        roomId,
        documentFromText("stale"),
        revision("1")
      )
    ).rejects.toThrow();
    await expect(api.releaseComposerDraftLease(lease.leaseId, rendererGeneration)).rejects.toThrow();
    await expect(api.acquireComposerDraftLease(scope, rendererGeneration)).rejects.toThrow();

    const freshGeneration = await api.beginComposerDraftRendererGeneration();
    const freshLease = await api.acquireComposerDraftLease(scope, freshGeneration);
    await expect(
      api.releaseComposerDraftLease(freshLease.leaseId, freshGeneration)
    ).resolves.toBeUndefined();
  });

  test("leases preserve exact large revisions and expose the Rust-owned clear token", async () => {
    const api = createBrowserFakeApi();
    const roomId = "!room-alpha:example.invalid";
    await api.selectRoom(roomId);
    const account = await readyAccount(api);
    const target = { kind: "main" as const, room_id: roomId };
    const scope = {
      account: {
        homeserver: account.homeserver,
        user_id: account.userId,
        device_id: account.deviceId
      },
      target
    };
    const rendererGeneration = await api.beginComposerDraftRendererGeneration();
    const lease = await api.acquireComposerDraftLease(scope, rendererGeneration);
    const captured = revision("9007199254740993");
    const accepted = revision("9007199254740994");

    await api.setComposerDraft(
      account,
      lease.leaseId,
      rendererGeneration,
      roomId,
      documentFromText("captured"),
      captured
    );
    const response = await api.sendText(
      account,
      lease.leaseId,
      rendererGeneration,
      "large-revision-send",
      roomId,
      documentFromText("captured"),
      captured
    );

    expect(response.outcome).toBe("accepted");
    expect(response.snapshot.state.ui.timeline.composer).toMatchObject({
      draft: "",
      draft_revision: accepted,
      last_accepted_clear_revision: accepted
    });
    expect(lease.revision).toBe("0");
    expect(typeof response.snapshot.state.ui.timeline.composer.draft_revision).toBe("string");
    await api.releaseComposerDraftLease(lease.leaseId, rendererGeneration);
  });

  test("fences stale main and thread draft writes after accepted sends", async () => {
    const api = createBrowserFakeApi();
    const roomId = "!room-alpha:example.invalid";
    await api.selectRoom(roomId);
    const session = (await api.getSnapshot()).state.domain.session;
    const account = {
      homeserver: session.homeserver!,
      userId: session.user_id!,
      deviceId: session.device_id!
    };
    const target = { kind: "main" as const, room_id: roomId };
    const { generation, lease: mainLease } = await beginComposerLease(
      api,
      account,
      target
    );
    await api.setComposerDraft(
      account,
      mainLease.leaseId,
      generation,
      roomId,
      documentFromText("main accepted"),
      revision("1")
    );
    const sent = await api.sendText(
      account,
      mainLease.leaseId,
      generation,
      "revision-main",
      roomId,
      documentFromText("main accepted"),
      revision("1")
    );
    expect(sent.outcome).toBe("accepted");
    expect(sent.snapshot.state.ui.timeline.composer).toMatchObject({
      draft: "",
      draft_revision: "2"
    });
    const staleMain = await api.setComposerDraft(
      account,
      mainLease.leaseId,
      generation,
      roomId,
      documentFromText("main accepted"),
      revision("1")
    );
    expect(staleMain.state.ui.timeline.composer.draft).toBe("");
    const nextMain = await api.setComposerDraft(
      account,
      mainLease.leaseId,
      generation,
      roomId,
      documentFromText("immediate next"),
      revision("3")
    );
    expect(nextMain.state.ui.timeline.composer.draft).toBe("immediate next");
    const lateMainAcceptance = await api.sendText(
      account,
      mainLease.leaseId,
      generation,
      "revision-main-late",
      roomId,
      documentFromText("main accepted"),
      revision("1")
    );
    expect(lateMainAcceptance.snapshot.state.ui.timeline.composer).toMatchObject({
      draft: "immediate next",
      draft_revision: "4"
    });

    const rootId = nextMain.timeline[0]!.event_id;
    await api.openThread(roomId, rootId, "existingThread");
    const { lease: threadLease } = await beginComposerLease(
      api,
      account,
      { kind: "thread", room_id: roomId, root_event_id: rootId },
      generation
    );
    await api.setThreadComposerDraft(
      account,
      threadLease.leaseId,
      generation,
      roomId,
      rootId,
      documentFromText("thread accepted"),
      revision("5")
    );
    const threadSent = await api.sendThreadReply(
      account,
      threadLease.leaseId,
      generation,
      "revision-thread",
      roomId,
      rootId,
      documentFromText("thread accepted"),
      revision("5")
    );
    expect(threadSent.outcome).toBe("accepted");
    const staleThread = await api.setThreadComposerDraft(
      account,
      threadLease.leaseId,
      generation,
      roomId,
      rootId,
      documentFromText("thread accepted"),
      revision("5")
    );
    expect(staleThread.state.ui.thread).toMatchObject({
      kind: "open",
      composer: { draft: "", draft_revision: "6" }
    });
    await api.setThreadComposerDraft(
      account,
      threadLease.leaseId,
      generation,
      roomId,
      rootId,
      documentFromText("immediate thread next"),
      revision("7")
    );
    const lateThreadAcceptance = await api.sendThreadReply(
      account,
      threadLease.leaseId,
      generation,
      "revision-thread-late",
      roomId,
      rootId,
      documentFromText("thread accepted"),
      revision("5")
    );
    expect(lateThreadAcceptance.snapshot.state.ui.thread).toMatchObject({
      kind: "open",
      composer: { draft: "immediate thread next", draft_revision: "8" }
    });
  });

  test("preserves a newer persisted draft when a reply acceptance settles late", async () => {
    const api = createBrowserFakeApi();
    const roomId = "!room-alpha:example.invalid";
    const selected = await api.selectRoom(roomId);
    const rootId = selected.timeline[0]!.event_id;
    const account = await readyAccount(api);
    const { generation, lease } = await beginComposerLease(api, account, {
      kind: "main",
      room_id: roomId
    });

    await api.setComposerDraft(
      account,
      lease.leaseId,
      generation,
      roomId,
      documentFromText("captured reply"),
      revision("1")
    );
    await api.setComposerDraft(
      account,
      lease.leaseId,
      generation,
      roomId,
      documentFromText("newer draft"),
      revision("2")
    );
    const response = await api.sendReply(
      account,
      lease.leaseId,
      generation,
      "late-reply-acceptance",
      roomId,
      rootId,
      documentFromText("captured reply"),
      revision("1")
    );

    expect(response.outcome).toBe("accepted");
    expect(response.snapshot.state.ui.timeline.composer.draft).toBe("newer draft");
    expect((await api.selectRoom(roomId)).state.ui.timeline.composer.draft).toBe("newer draft");
  });

  test("rejects draft writes and acceptances captured for another account", async () => {
    const api = createBrowserFakeApi();
    const roomId = "!room-alpha:example.invalid";
    await api.selectRoom(roomId);
    const before = await api.getSnapshot();
    const rootId = before.timeline[0]!.event_id;
    await api.openThread(roomId, rootId, "existingThread");
    const account = await readyAccount(api);
    const mainTarget = { kind: "main" as const, room_id: roomId };
    const threadTarget = {
      kind: "thread" as const,
      room_id: roomId,
      root_event_id: rootId
    };
    const { generation, lease: mainLease } = await beginComposerLease(
      api,
      account,
      mainTarget
    );
    const { lease: threadLease } = await beginComposerLease(
      api,
      account,
      threadTarget,
      generation
    );

    const staleAccount = {
      homeserver: "https://stale.example.invalid",
      userId: "@stale-account:example.invalid",
      deviceId: "STALE"
    };
    await expect(
      api.setComposerDraft(
        staleAccount,
        mainLease.leaseId,
        generation,
        roomId,
        documentFromText("must not cross accounts"),
        revision("1")
      )
    ).rejects.toThrow("composer draft lease mismatch");
    await expect(
      api.setThreadComposerDraft(
        staleAccount,
        threadLease.leaseId,
        generation,
        roomId,
        rootId,
        documentFromText("must not cross accounts"),
        revision("1")
      )
    ).rejects.toThrow("composer draft lease mismatch");
    await expect(
      api.sendText(
        staleAccount,
        mainLease.leaseId,
        generation,
        "stale-main-send",
        roomId,
        documentFromText("must not send")
      )
    ).rejects.toThrow("composer draft lease mismatch");
    await expect(
      api.sendThreadReply(
        staleAccount,
        threadLease.leaseId,
        generation,
        "stale-thread-send",
        roomId,
        rootId,
        documentFromText("must not send")
      )
    ).rejects.toThrow("composer draft lease mismatch");
    await expect(
      api.scheduleSend(
        staleAccount,
        threadLease.leaseId,
        generation,
        threadTarget,
        "must not schedule",
        Date.now() + 60_000,
        revision("0")
      )
    ).rejects.toThrow("composer draft lease mismatch");
    await api.stageUploadBytes(threadTarget, [
      {
        stagedId: "stale-account-upload",
        position: 0,
        filename: "synthetic.txt",
        mimeType: "text/plain",
        bytes: [1, 2, 3]
      }
    ]);
    await expect(
      api.sendPreparedUploads(
        staleAccount,
        threadLease.leaseId,
        generation,
        threadTarget,
        revision("0")
      )
    ).rejects.toThrow("composer draft lease mismatch");
    const after = await api.getSnapshot();
    expect(after.state.ui.timeline.scheduled_sends).toHaveLength(0);
    expect(after.state.ui.timeline.composer.draft).toBe("");
    expect(after.state.ui.thread).toMatchObject({
      kind: "open",
      composer: { draft: "", draft_revision: "0" },
      staged_uploads: [{ staged_id: "stale-account-upload" }]
    });
  });

  test("deduplicates reply submissions without incrementing the root twice", async () => {
    const api = createBrowserFakeApi();
    const roomId = "!room-alpha:example.invalid";
    await api.selectRoom(roomId);
    const account = await readyAccount(api);
    const root = (await api.getSnapshot()).timeline[0]!;
    const { generation, lease } = await beginComposerLease(
      api,
      account,
      { kind: "main", room_id: roomId }
    );
    const before = root.reply_count;
    await api.sendReply(
      account,
      lease.leaseId,
      generation,
      "reply-same",
      roomId,
      root.event_id,
      documentFromText("original")
    );
    const replay = await api.sendReply(
      account,
      lease.leaseId,
      generation,
      "reply-same",
      roomId,
      root.event_id,
      documentFromText("changed")
    );
    expect(replay.snapshot.timeline.find((item) => item.event_id === root.event_id)?.reply_count).toBe(before + 1);
  });

  test("deduplicates an unknown thread retry and preserves terminal correlation fields", async () => {
    const api = createBrowserFakeApi();
    const roomId = "!room-alpha:example.invalid";
    await api.selectRoom(roomId);
    const account = await readyAccount(api);
    const rootId = (await api.getSnapshot()).timeline[0]!.event_id;
    await api.openThread(roomId, rootId, "existingThread");
    const { generation, lease } = await beginComposerLease(
      api,
      account,
      { kind: "thread", room_id: roomId, root_event_id: rootId }
    );
    const first = await api.sendThreadReply(
      account,
      lease.leaseId,
      generation,
      "thread-unknown",
      roomId,
      rootId,
      documentFromText("original")
    );
    const replay = await api.sendThreadReply(
      account,
      lease.leaseId,
      generation,
      "thread-unknown",
      roomId,
      rootId,
      documentFromText("edited")
    );
    expect(replay.transactionId).toBe(first.transactionId);
    const thread = replay.snapshot.state.ui.thread;
    expect(thread.kind).toBe("open");
    if (thread.kind === "open") {
      expect(thread.composer?.pending_submission_id).toBeNull();
      expect(thread.composer?.accepted_submission_ids).toContain("thread-unknown");
    }
  });

  test("bounds terminal submission replay tombstones to 128 entries", async () => {
    const api = createBrowserFakeApi();
    const roomId = "!room-alpha:example.invalid";
    await api.selectRoom(roomId);
    const account = await readyAccount(api);
    const { generation, lease } = await beginComposerLease(
      api,
      account,
      { kind: "main", room_id: roomId }
    );
    for (let index = 0; index < 129; index += 1) {
      await api.sendText(
        account,
        lease.leaseId,
        generation,
        `bounded-${index}`,
        roomId,
        documentFromText(`body-${index}`)
      );
    }
    const bounded = await api.getSnapshot();
    const before = bounded.timeline.length;
    expect(bounded.state.ui.timeline.submission_registry.accepted_submission_ids).toHaveLength(0);
    expect(bounded.state.ui.timeline.submission_registry.settled_submission_ids).toHaveLength(128);
    expect(bounded.state.ui.timeline.submission_registry.settled_submission_ids).not.toContain("bounded-0");
    expect(bounded.state.ui.timeline.composer.accepted_submission_ids).toHaveLength(128);
    expect(bounded.state.ui.timeline.composer.accepted_submission_ids).not.toContain("bounded-0");
    await api.sendText(
      account,
      lease.leaseId,
      generation,
      "bounded-1",
      roomId,
      documentFromText("deduped")
    );
    expect((await api.getSnapshot()).timeline).toHaveLength(before);
    await api.sendText(
      account,
      lease.leaseId,
      generation,
      "bounded-0",
      roomId,
      documentFromText("evicted")
    );
    expect((await api.getSnapshot()).timeline).toHaveLength(before + 1);
  });

  test("bounds thread submission tombstones to 128 entries", async () => {
    const api = createBrowserFakeApi();
    const roomId = "!room-alpha:example.invalid";
    await api.selectRoom(roomId);
    const account = await readyAccount(api);
    const rootId = (await api.getSnapshot()).timeline[0]!.event_id;
    await api.openThread(roomId, rootId, "existingThread");
    const { generation, lease } = await beginComposerLease(api, account, {
      kind: "thread",
      room_id: roomId,
      root_event_id: rootId
    });
    for (let index = 0; index < 129; index += 1) {
      await api.sendThreadReply(
        account,
        lease.leaseId,
        generation,
        `thread-bounded-${index}`,
        roomId,
        rootId,
        documentFromText(`body-${index}`)
      );
    }
    const thread = (await api.getSnapshot()).state.ui.thread;
    expect(thread.kind).toBe("open");
    if (thread.kind === "open") {
      expect(thread.composer?.accepted_submission_ids).toHaveLength(128);
      expect(thread.composer?.accepted_submission_ids).not.toContain("thread-bounded-0");
    }
  });

  test("returns an empty diagnostic snapshot in the browser fake", async () => {
    const api = createBrowserFakeApi();

    await expect(api.getDiagnosticSnapshot()).resolves.toEqual({
      entries: [],
      droppedEntries: 0,
      slidingSync: {
        discoveryState: "not_started",
        advertised: false,
        discoverySource: "unknown",
        lastProbeAgeBucket: "never",
        lastHttpStatusClass: "unknown",
        requestSchema: "element_x_all_rooms",
        engine: "SyncService",
        sdkSlidingSyncVersion: "unknown",
        roomListSharePos: true,
        encryptionSharePos: false,
        encryptionConnectionProfile: "sdk_default_encryption",
        encryptionExtensionProfile: "e2ee_to_device",
        provisionalEncryptionStarted: false,
        provisionalFirstResponseSeen: false,
        provisionalStoppedBeforeFirstResponse: false,
        provisionalToNormalHandoffBucket: "never",
        lifecycle: "stopped",
        connectivityProven: false,
        committedGeneration: 0,
        lastSuccessAgeBucket: "never",
        consecutiveFailureCount: 0,
        lastFailureOrigin: "none",
        lastFailureKind: "none",
        lastFailureStage: "none",
        lastHttpErrorSource: "none",
        lastHttpStatus: "none",
        lastMatrixErrorKind: "none",
        lastFailureRetryability: "none",
        roomListTaskRunning: false,
        encryptionTaskRunning: false,
        posPresent: false,
        directAccountDataSource: "unavailable",
        directMappedRoomCount: 0,
        directTargetCount: 0,
        projectedDmCount: 0,
        explicitDmCount: 0,
        fallbackDmCount: 0,
        directNonDmCount: 0,
        directInvalidEntryCount: 0,
        directEventWakeCount: 0,
        directEventAppliedCount: 0,
        directEventStreamRunning: false
      }
    });
  });

  test("logout clears the active session and session-owned views", async () => {
    const api = createBrowserFakeApi();

    const snapshot = await api.logout();

    expect(snapshot.state.domain.session.kind).toBe("signedOut");
    expect(snapshot.state.ui.navigation.active_room_id).toBeNull();
    expect(snapshot.state.ui.timeline.room_id).toBeNull();
    expect(snapshot.timeline).toEqual([]);
  });

  test("applies the Rust-shaped settings patch to the fixture snapshot", async () => {
    const api = createBrowserFakeApi();

    const snapshot = await api.updateSettings({
      appearance: { theme: "dark" },
      keyboard: { composer_send_shortcut: "modEnter" }
    });

    expect(snapshot.state.domain.settings.values.appearance.theme).toBe("dark");
    expect(snapshot.state.domain.settings.values.keyboard.composer_send_shortcut).toBe("modEnter");
    expect(snapshot.state.domain.settings.persistence).toEqual({ kind: "idle" });
  });

  test("stores room URL-preview overrides outside settings values", async () => {
    const api = createBrowserFakeApi();
    const roomId = "!room-alpha:example.invalid";

    const disabled = await api.setRoomUrlPreviewOverride(roomId, false);
    expect(disabled.state.domain.link_preview_settings.room_overrides[roomId]).toBe(false);
    expect("room_url_previews" in disabled.state.domain.settings.values).toBe(false);

    const restored = await api.setRoomUrlPreviewOverride(roomId, true);
    expect(restored.state.domain.link_preview_settings.room_overrides[roomId]).toBeUndefined();
  });

  test("projects room-list filters like the Rust reducer", async () => {
    const api = createBrowserFakeApi();

    const initial = await api.getSnapshot();
    expect(initial.state.ui.room_list.items?.map((item) => item.room_id)).toEqual([
      "!room-alpha:example.invalid",
      "!room-planning:example.invalid"
    ]);

    const people = await api.selectRoomListFilter({ kind: "people" });
    expect(people.state.ui.room_list.items).toEqual([]);

    const unread = await api.selectRoomListFilter({ kind: "unread" });
    expect(unread.state.ui.room_list.items?.map((item) => item.room_id)).toEqual([
      "!room-alpha:example.invalid",
      "!room-planning:example.invalid"
    ]);

    await api.setRoomTag("!room-planning:example.invalid", "favourite");
    const roomsAfterFavourite = await api.selectRoomListFilter({ kind: "rooms" });
    expect(roomsAfterFavourite.state.ui.room_list.items).toEqual([
      { room_id: "!room-alpha:example.invalid", kind: "room" }
    ]);

    const favourites = await api.selectRoomListFilter({ kind: "favourites" });
    expect(favourites.state.ui.room_list.items).toEqual([
      { room_id: "!room-planning:example.invalid", kind: "room" }
    ]);

    const invites = await api.selectRoomListFilter({ kind: "invites" });
    expect(invites.state.ui.room_list.items).toEqual([
      { room_id: "!invite-design-review:example.invalid", kind: "invite" }
    ]);
  });

  test("people filter at account home includes all DMs", async () => {
    const api = createBrowserFakeApi();
    await api.selectSpace(null);

    const people = await api.selectRoomListFilter({ kind: "people" });
    expect(people.state.ui.room_list.items).toEqual([
      { room_id: "!dm-member-1:example.invalid", kind: "room" },
      { room_id: "!dm-member-2:example.invalid", kind: "room" }
    ]);
  });

  test("selecting account home clears the active room instead of selecting a default timeline", async () => {
    const api = createBrowserFakeApi();
    await api.selectSpace("!space-beta:example.invalid");

    const home = await api.selectSpace(null);

    expect(home.state.ui.navigation.active_space_id).toBeNull();
    expect(home.state.ui.navigation.active_room_id).toBeNull();
    expect(home.state.ui.timeline.room_id).toBeNull();
  });

  test("projects room-list filters within the active space like the Rust reducer", async () => {
    const api = createBrowserFakeApi();

    const initial = await api.getSnapshot();
    expect(initial.state.ui.navigation.active_space_id).toBe("!space-alpha:example.invalid");
    expect(initial.state.ui.room_list.items?.map((item) => item.room_id)).toEqual([
      "!room-alpha:example.invalid",
      "!room-planning:example.invalid"
    ]);

    const beta = await api.selectSpace("!space-beta:example.invalid");
    expect(beta.state.ui.navigation.active_space_id).toBe("!space-beta:example.invalid");
    expect(beta.state.ui.room_list.items?.map((item) => item.room_id)).toEqual([
      "!room-search:example.invalid"
    ]);
  });

  test("preserves all read-receipt readers when adding the current user", async () => {
    const api = createBrowserFakeApi();
    const eventId = "$receipt-target:example.invalid";
    const existingReaders: LiveReadReceipt[] = [
      receipt("@alice:example.invalid", "Alice", 1_000),
      receipt("@bob:example.invalid", "Bob", 2_000),
      receipt("@carol:example.invalid", "Carol", 3_000)
    ];
    const mutableApi = api as unknown as { snapshot: DesktopSnapshot };
    mutableApi.snapshot.state.domain.live_signals.rooms["!room-alpha:example.invalid"] = {
      receipts_by_event: {
        [eventId]: {
          readers: existingReaders,
          total_count: existingReaders.length,
          overflow_count: 0
        }
      },
      fully_read_event_id: null,
      typing_user_ids: [],
      typing_users: []
    };

    await api.sendReadReceipt("!room-alpha:example.invalid", eventId);
    const updated = await api.getSnapshot();

    const summary =
      updated.state.domain.live_signals.rooms["!room-alpha:example.invalid"]?.receipts_by_event[
        eventId
      ];
    expect(summary?.total_count).toBe(4);
    expect(summary?.overflow_count).toBe(0);
    expect(summary?.readers).toHaveLength(4);
    expect(summary?.readers.map((reader) => reader.user_id)).toContain("@demo-user:example.invalid");
  });

  test("resolves composer key actions from the Rust-shaped settings snapshot", async () => {
    const api = createBrowserFakeApi();

    await expect(
      api.resolveComposerKeyAction(
        "main",
        {
          key: "enter",
          modifiers: { ctrl: false, meta: false, shift: false, alt: false },
          is_composing: false,
          selection: null
        },
        { autocomplete_open: false, send_enabled: true }
      )
    ).resolves.toBe("send");

    await api.updateSettings({
      keyboard: { composer_send_shortcut: "modEnter" }
    });

    await expect(
      api.resolveComposerKeyAction(
        "thread",
        {
          key: "enter",
          modifiers: { ctrl: false, meta: false, shift: false, alt: false },
          is_composing: false,
          selection: null
        },
        { autocomplete_open: false, send_enabled: true }
      )
    ).resolves.toBe("insertNewline");

    await expect(
      api.resolveComposerKeyAction(
        "thread",
        {
          key: "enter",
          modifiers: { ctrl: true, meta: false, shift: false, alt: false },
          is_composing: false,
          selection: null
        },
        { autocomplete_open: false, send_enabled: true }
      )
    ).resolves.toBe("send");
  });

  test("composer resolver mirrors Rust IME and no-op actions", async () => {
    const api = createBrowserFakeApi();

    await expect(
      api.resolveComposerKeyAction(
        "main",
        {
          key: "enter",
          modifiers: { ctrl: false, meta: false, shift: false, alt: false },
          is_composing: true,
          selection: { start: 0, end: 0 }
        },
        { autocomplete_open: true, send_enabled: true }
      )
    ).resolves.toBe("commitImeCandidate");

    await expect(
      api.resolveComposerKeyAction(
        "edit",
        {
          key: "enter",
          modifiers: { ctrl: false, meta: false, shift: false, alt: false },
          is_composing: false,
          selection: null
        },
        { autocomplete_open: false, send_enabled: false }
      )
    ).resolves.toBe("noop");
  });

  test("updates the Rust-shaped locale display profile from locale settings", async () => {
    const api = createBrowserFakeApi();

    const snapshot = await api.updateSettings({
      locale: { language_tag: "ar-XB", text_direction: "auto" }
    });

    expect(snapshot.state.domain.locale_profile).toMatchObject({
      lang: "ar-XB",
      dir: "rtl",
      catalog_locale: "pseudo",
      pseudo_locale: "bidi",
      platform: "linux",
      modifier_labels: { primary: "Ctrl" }
    });
  });

  test("updates the Rust-shaped profile snapshot for preview controls", async () => {
    const api = createBrowserFakeApi();

    const named = await api.setDisplayName("Alice");
    expect(named.state.domain.profile.own.display_name).toBe("Alice");
    expect(named.state.domain.profile.update).toEqual({ kind: "idle" });

    const avatar = await api.setAvatar("image/png", [1, 2, 3, 4]);
    expect(avatar.state.domain.profile.own.avatar).toEqual({
      mxc_uri: "mxc://browser.fake/profile-avatar",
      thumbnail: { kind: "notRequested" }
    });
    expect(avatar.state.domain.profile.update).toEqual({ kind: "idle" });
  });

  test("updates Rust-shaped local alias projections for profile, rooms, and room members", async () => {
    const api = createBrowserFakeApi();
    const targetUserId = "@member-1:example.invalid";

    const aliased = await api.setLocalUserAlias(targetUserId, "Desk Alias");

    expect(aliased.state.domain.profile.local_aliases[targetUserId]).toBe("Desk Alias");
    expect(aliased.state.domain.profile.local_alias_update).toEqual({ kind: "idle" });
    expect(aliased.state.domain.profile.users[targetUserId]).toMatchObject({
      display_label: "Desk Alias",
      original_display_label: "Member 1"
    });
    expect(
      aliased.state.domain.profile.users[targetUserId]?.mention_search_terms
    ).toEqual(["Desk Alias", "Member 1", targetUserId]);
    expect(
      aliased.state.domain.rooms.find((room) => room.room_id === "!dm-member-1:example.invalid")
    ).toMatchObject({
      display_label: "Desk Alias",
      original_display_label: "Member 1"
    });

    const loaded = await api.loadRoomSettings("!room-alpha:example.invalid");
    expect(
      loaded.state.domain.room_management.settings?.members.find(
        (member) => member.user_id === targetUserId
      )
    ).toMatchObject({
      display_label: "Desk Alias",
      original_display_label: "Member 1"
    });

    const cleared = await api.setLocalUserAlias(targetUserId, null);
    expect(cleared.state.domain.profile.local_aliases[targetUserId]).toBeUndefined();
    expect(cleared.state.domain.profile.users[targetUserId]).toMatchObject({
      display_label: "Member 1",
      original_display_label: "Member 1"
    });
    expect(
      cleared.state.domain.rooms.find((room) => room.room_id === "!dm-member-1:example.invalid")
    ).toMatchObject({
      display_label: "Member 1",
      original_display_label: "Member 1"
    });
  });

  test("queries mention candidates only from the loaded room member projection", async () => {
    const api = createBrowserFakeApi();
    const loaded = await api.loadRoomSettings("!room-alpha:example.invalid");
    const roomMember = loaded.state.domain.room_management.settings?.members[0];
    expect(roomMember).toBeDefined();
    await api.queryMentionCandidates(
      "!room-alpha:example.invalid",
      "thread",
      roomMember!.display_label
    );

    const queried = await api.getSnapshot();
    expect(
      queried.state.domain.mention_candidates.targets.find(
        (target) =>
          target.room_id === "!room-alpha:example.invalid" &&
          target.surface === "thread"
      )
    ).toMatchObject({
      query: roomMember!.display_label,
      completeness: "complete",
      candidates: [
        expect.objectContaining({
          user_id: roomMember!.user_id,
          membership: "joined"
        })
      ]
    });

    await api.queryMentionCandidates("!other:example.invalid", "main", "");
    const failClosed = await api.getSnapshot();
    expect(
      failClosed.state.domain.mention_candidates.targets.find(
        (target) =>
          target.room_id === "!other:example.invalid" &&
          target.surface === "main"
      )
    ).toMatchObject({
      completeness: "partial",
      candidates: []
    });
  });

  test("updates the Rust-shaped E2EE trust snapshot for preview controls", async () => {
    const api = createBrowserFakeApi();

    await expect(api.bootstrapCrossSigning()).resolves.toMatchObject({
      state: {
        domain: {
          e2ee_trust: {
            cross_signing: { kind: "trusted" }
          }
        }
      }
    });

    await expect(api.enableKeyBackup()).resolves.toMatchObject({
      state: {
        domain: {
          e2ee_trust: {
            key_backup: { kind: "enabled", version: "browser-preview" }
          }
        }
      }
    });

    const awaitingAuth = await api.resetIdentity();
    expect(awaitingAuth.state.domain.e2ee_trust.identity_reset).toMatchObject({
      kind: "awaitingAuth",
      auth_type: "uiaa"
    });

    const flow =
      awaitingAuth.state.domain.e2ee_trust.identity_reset.kind === "awaitingAuth"
        ? awaitingAuth.state.domain.e2ee_trust.identity_reset.request_id
        : 0;
    const cancelled = await api.cancelIdentityReset(flow);
    expect(cancelled.state.domain.e2ee_trust.identity_reset).toEqual({
      kind: "failed",
      request_id: flow,
      failureKind: "cancelled"
    });

    const retryAwaitingAuth = await api.resetIdentity();
    const retryFlow =
      retryAwaitingAuth.state.domain.e2ee_trust.identity_reset.kind === "awaitingAuth"
        ? retryAwaitingAuth.state.domain.e2ee_trust.identity_reset.request_id
        : 0;
    const reset = await api.submitIdentityResetPassword(retryFlow, "synthetic-password");
    expect(reset.state.domain.e2ee_trust.identity_reset).toEqual({ kind: "idle" });
    expect(reset.state.domain.e2ee_trust.cross_signing).toEqual({ kind: "missing" });
    expect(reset.state.domain.e2ee_trust.key_backup).toEqual({ kind: "disabled" });
  });

  test("updates Rust-shaped key-management state without retaining secrets or paths", async () => {
    const api = createBrowserFakeApi();

    const exported = await api.exportRoomKeys(
      "/tmp/private-export.txt",
      "private-room-key-passphrase"
    );
    expect(exported.state.domain.e2ee_trust.key_management.room_key_export).toMatchObject({
      kind: "exported",
      exported_sessions: null
    });

    const imported = await api.importRoomKeys(
      "/tmp/private-import.txt",
      "private-room-key-passphrase"
    );
    expect(imported.state.domain.e2ee_trust.key_management.room_key_import).toMatchObject({
      kind: "imported",
      imported_count: 1,
      total_count: 1
    });

    const setup = await api.bootstrapSecureBackup(
      "private-secure-backup-passphrase",
      "/tmp/private-recovery.txt"
    );
    expect(setup.state.domain.e2ee_trust.key_management.secure_backup_setup).toMatchObject({
      kind: "recoveryKeyReady",
      delivery: { kind: "written" }
    });

    const changed = await api.changeSecureBackupPassphrase(
      "private-old-secure-backup-passphrase",
      "private-new-secure-backup-passphrase",
      null
    );
    expect(changed.state.domain.e2ee_trust.key_management.passphrase_change).toMatchObject({
      kind: "changed",
      delivery: { kind: "notWritten" }
    });

    const serialized = JSON.stringify(changed.state.domain.e2ee_trust.key_management);
    expect(serialized).not.toContain("private-room-key-passphrase");
    expect(serialized).not.toContain("private-secure-backup-passphrase");
    expect(serialized).not.toContain("private-recovery");
  });

  test("does not synthesize pin state for an unknown room", async () => {
    const api = createBrowserFakeApi();

    await api.pinEvent("!missing:browser.fake", "$event:browser.fake");
    const snapshot = await api.unpinEvent("!missing:browser.fake", "$event:browser.fake");

    expect(snapshot.state.domain.room_interactions["!missing:browser.fake"]).toBeUndefined();
  });

  test("selectRoom mirrors the Rust unknown-room guard", async () => {
    const api = createBrowserFakeApi();
    const before = await api.getSnapshot();

    const selected = await api.selectRoom("!missing:example.invalid");

    expect(selected.state.ui.navigation.active_room_id).toBe(
      before.state.ui.navigation.active_room_id
    );
    expect(selected.state.ui.timeline.room_id).toBe(before.state.ui.timeline.room_id);
    expect(selected.timeline.map((message) => message.room_id)).toEqual(
      before.timeline.map((message) => message.room_id)
    );
  });

  test("selectRoom closes dependent panes like the Rust reducer", async () => {
    const api = createBrowserFakeApi();

    await api.openThreadsList({ kind: "room", room_id: "!room-alpha:example.invalid" });
    const selected = await api.selectRoom("!room-planning:example.invalid");

    expect(selected.state.ui.navigation.active_room_id).toBe("!room-planning:example.invalid");
    expect(selected.state.ui.thread).toEqual({ kind: "closed" });
    expect(selected.state.domain.thread_attention).toEqual({ kind: "closed" });
    expect(selected.state.ui.threads_list).toEqual({ kind: "closed" });
    expect(selected.state.ui.focused_context).toEqual({ kind: "closed" });
    expect(selected.thread).toBeNull();
  });

  test("selectSpace restores the last non-DM room visited in that space", async () => {
    const api = createBrowserFakeApi();

    await api.selectRoom("!room-planning:example.invalid");
    await api.selectRoom("!room-search:example.invalid");
    const restored = await api.selectSpace("!space-alpha:example.invalid");

    expect(restored.state.ui.navigation.active_space_id).toBe("!space-alpha:example.invalid");
    expect(restored.state.ui.navigation.active_room_id).toBe("!room-planning:example.invalid");
    expect(restored.state.ui.timeline.room_id).toBe("!room-planning:example.invalid");
    expect(restored.state.ui.navigation.last_room_by_space_id).toMatchObject({
      "!space-alpha:example.invalid": "!room-planning:example.invalid",
      "!space-beta:example.invalid": "!room-search:example.invalid"
    });
  });

  test("reorderSpaces persists the synthetic rail order", async () => {
    const api = createBrowserFakeApi();

    const reordered = await api.reorderSpaces([
      "!space-beta:example.invalid",
      "!space-alpha:example.invalid"
    ]);

    expect(reordered.state.ui.navigation.space_order).toEqual([
      "!space-beta:example.invalid",
      "!space-alpha:example.invalid"
    ]);
    expect(reordered.state.domain.spaces.map((space) => space.space_id)).toEqual([
      "!space-beta:example.invalid",
      "!space-alpha:example.invalid"
    ]);
    expect(reordered.sidebar.space_rail.map((space) => space.space_id)).toEqual([
      "!space-beta:example.invalid",
      "!space-alpha:example.invalid"
    ]);
  });

  test("reorderSpaces preserves hidden Space preference slots", async () => {
    const api = createBrowserFakeApi();
    const mutable = api as unknown as { snapshot: DesktopSnapshot };
    mutable.snapshot.state.ui.navigation.space_order = [
      "!space-alpha:example.invalid",
      "!space-hidden:example.invalid",
      "!space-beta:example.invalid"
    ];

    const reordered = await api.reorderSpaces([
      "!space-beta:example.invalid",
      "!space-alpha:example.invalid"
    ]);

    expect(reordered.state.ui.navigation.space_order).toEqual([
      "!space-beta:example.invalid",
      "!space-hidden:example.invalid",
      "!space-alpha:example.invalid"
    ]);
  });

  test("leaveRoom removes a Space without leaving its child rooms", async () => {
    const api = createBrowserFakeApi();

    const left = await api.leaveRoom("!space-alpha:example.invalid");

    expect(left.state.domain.spaces.map((space) => space.space_id)).toEqual([
      "!space-beta:example.invalid"
    ]);
    expect(left.state.ui.navigation.active_space_id).toBeNull();
    expect(left.state.domain.rooms.some((room) => room.room_id === "!room-alpha:example.invalid")).toBe(
      true
    );
    expect(
      left.state.domain.rooms
        .find((room) => room.room_id === "!room-alpha:example.invalid")
        ?.parent_space_ids
    ).toEqual([]);
    expect(left.sidebar.space_rail.map((space) => space.space_id)).toEqual([
      "!space-beta:example.invalid"
    ]);
  });

  test.each(["leaveRoom", "forgetRoom"] as const)(
    "%s clears every active ordinary-room projection",
    async (operation) => {
      const api = createBrowserFakeApi();
      await api.selectRoom(alphaRoomId);
      await dirtyRoomOwnedState(api, alphaRoomId, "$alpha-update");
      await api.openActivity();
      await api.openActivityEvent(alphaRoomId, "$alpha-update");
      await api.selectSearchResult(alphaRoomId, "$alpha-update");
      await api.submitSearch("Alpha", "currentRoom");
      await api.openThreadsList({ kind: "room", room_id: alphaRoomId });
      await api.openFilesView(
        { kind: "room", room_id: alphaRoomId },
        { kinds: [...allAttachmentKinds], filename_query: null },
        "newestFirst"
      );

      const account = await readyAccount(api);
      const { generation, lease } = await beginComposerLease(api, account, {
        kind: "main",
        room_id: alphaRoomId
      });
      await api.setComposerDraft(
        account,
        lease.leaseId,
        generation,
        alphaRoomId,
        documentFromText("Alpha draft"),
        revision("7")
      );
      await api.stageUploadBytes(
        { kind: "main", room_id: alphaRoomId },
        [{
          stagedId: "alpha-upload",
          position: 0,
          filename: "fixture_alpha.txt",
          mimeType: "text/plain",
          bytes: [1, 2, 3]
        }]
      );

      const dirty = roomOwnedProjection(await api.getSnapshot(), alphaRoomId);
      expect(dirty).toMatchObject({
        room: expect.objectContaining({ room_id: alphaRoomId }),
        roomPreference: expect.any(Object),
        linkPreviewOverride: false,
        notification: expect.any(Object),
        interaction: expect.any(Object),
        crawler: expect.any(Object),
        crawlerLastActive: expect.any(Object),
        liveSignals: expect.any(Object)
      });
      expect(dirty.mentionTargets).not.toEqual([]);
      expect(dirty.searchResults).not.toEqual([]);
      expect(dirty.activityRows).not.toEqual([]);
      expect(dirty.threadsItems).not.toEqual([]);
      expect(dirty.filesItems).not.toEqual([]);

      const markRead = api.markActivityRead({
        kind: "room",
        room_id: alphaRoomId,
        up_to_event_id: "$alpha-update"
      });
      const removed =
        operation === "leaveRoom" ? await api.leaveRoom(alphaRoomId) : await api.forgetRoom(alphaRoomId);
      await markRead;

      expect(removed.state.domain.rooms.some((room) => room.room_id === alphaRoomId)).toBe(false);
      expect(removed.state.domain.room_preferences.rooms[alphaRoomId]).toBeUndefined();
      expect(removed.state.domain.link_preview_settings.room_overrides[alphaRoomId]).toBeUndefined();
      expect(removed.state.domain.room_notification_settings[alphaRoomId]).toBeUndefined();
      expect(removed.state.domain.room_interactions[alphaRoomId]).toBeUndefined();
      expect(removed.state.domain.search_crawler.rooms[alphaRoomId]).toBeUndefined();
      expect(removed.state.domain.search_crawler.last_active).toBeNull();
      expect(removed.state.domain.live_signals.rooms[alphaRoomId]).toBeUndefined();
      expect(
        removed.state.domain.mention_candidates.targets.some((target) => target.room_id === alphaRoomId)
      ).toBe(false);
      expect(removed.state.domain.search).toEqual({ kind: "closed" });
      expect(removed.state.domain.activity.kind).toBe("open");
      if (removed.state.domain.activity.kind === "open") {
        expect(
          [...removed.state.domain.activity.recent.rows, ...removed.state.domain.activity.unread.rows].some(
            (row) => row.room_id === alphaRoomId
          )
        ).toBe(false);
        expect(removed.state.domain.activity.mark_read).toEqual({ kind: "idle" });
      }
      expect(removed.state.ui.navigation.active_room_id).toBeNull();
      expect(removed.state.ui.navigation.main_timeline_anchor).toBeNull();
      expect(removed.state.ui.timeline.room_id).toBeNull();
      expect(removed.state.ui.timeline.is_subscribed).toBe(false);
      expect(removed.state.ui.timeline.composer.draft).toBe("");
      expect(removed.state.ui.timeline.staged_uploads).toEqual([]);
      expect(removed.state.ui.thread).toEqual({ kind: "closed" });
      expect(removed.state.ui.threads_list).toEqual({ kind: "closed" });
      expect(removed.state.ui.focused_context).toEqual({ kind: "closed" });
      expect(removed.state.ui.files_view).toEqual({ kind: "closed" });
      expect(removed.timeline).toEqual([]);
      expect(removed.thread).toBeNull();
      expect(removed.state.domain.rooms.some((room) => room.room_id === planningRoomId)).toBe(true);
    }
  );

  test("removing inactive Alpha filters only Alpha-owned state", async () => {
    const api = createBrowserFakeApi();
    await dirtyRoomOwnedState(api, alphaRoomId, "$alpha-update");
    await dirtyRoomOwnedState(api, planningRoomId, "$late-original");
    await api.selectRoom(planningRoomId);
    await api.openActivity();
    await api.openThreadsList({ kind: "home" });
    await api.openFilesView(
      { kind: "space", space_id: alphaSpaceId },
      { kinds: [...allAttachmentKinds], filename_query: null },
      "newestFirst"
    );
    await api.submitSearch("synthetic", "allRooms");
    await api.openThread(alphaRoomId, "$alpha-update", "existingThread");

    const account = await readyAccount(api);
    const { generation, lease } = await beginComposerLease(api, account, {
      kind: "main",
      room_id: planningRoomId
    });
    await api.setComposerDraft(
      account,
      lease.leaseId,
      generation,
      planningRoomId,
      documentFromText("Retained Planning draft"),
      revision("9")
    );
    await api.stageUploadBytes(
      { kind: "main", room_id: planningRoomId },
      [{
        stagedId: "retained-planning-upload",
        position: 0,
        filename: "fixture_planning.txt",
        mimeType: "text/plain",
        bytes: [4, 5, 6]
      }]
    );

    const before = await api.getSnapshot();
    const planningBefore = roomOwnedProjection(before, planningRoomId);
    const alphaBefore = roomOwnedProjection(before, alphaRoomId);
    expect(alphaBefore.searchResults).not.toEqual([]);
    expect(alphaBefore.activityRows).not.toEqual([]);
    expect(alphaBefore.threadsItems).not.toEqual([]);
    expect(alphaBefore.filesItems).not.toEqual([]);
    expect(before.state.ui.thread).toMatchObject({ room_id: alphaRoomId });

    const markRead = api.markActivityRead({
      kind: "room",
      room_id: alphaRoomId,
      up_to_event_id: "$alpha-update"
    });
    await api.leaveRoom(alphaRoomId);
    await markRead;
    const after = await api.getSnapshot();

    expect(roomOwnedProjection(after, planningRoomId)).toEqual(planningBefore);
    expect(roomOwnedProjection(after, alphaRoomId)).toEqual({
      room: undefined,
      roomPreference: undefined,
      linkPreviewOverride: undefined,
      notification: undefined,
      interaction: undefined,
      crawler: undefined,
      crawlerLastActive: null,
      liveSignals: undefined,
      mentionTargets: [],
      searchResults: [],
      activityRows: [],
      threadsItems: [],
      filesItems: []
    });
    expect(after.state.domain.search.kind).toBe("results");
    expect(after.state.domain.activity.kind).toBe("open");
    if (after.state.domain.activity.kind === "open") {
      expect(after.state.domain.activity.mark_read).toEqual({ kind: "idle" });
    }
    expect(after.state.ui.navigation.active_room_id).toBe(planningRoomId);
    expect(after.state.ui.timeline.room_id).toBe(planningRoomId);
    expect(after.state.ui.timeline.composer.draft).toBe("Retained Planning draft");
    expect(after.state.ui.timeline.staged_uploads).toHaveLength(1);
    expect(after.state.ui.focused_context).toEqual({ kind: "closed" });
    expect(after.state.ui.thread).toEqual({ kind: "closed" });
    expect(after.thread).toBeNull();
    expect(after.state.ui.threads_list.kind).toBe("open");
    expect(after.state.ui.files_view.kind).toBe("open");
    if (after.state.ui.files_view.kind === "open") {
      expect(after.state.ui.files_view.scope).toEqual({
        kind: "space",
        space_id: alphaSpaceId,
        child_room_ids: [planningRoomId]
      });
    }
  });

  test("removing a Space preserves children and closes only its Space scopes", async () => {
    const api = createBrowserFakeApi();
    await api.selectSpace(alphaSpaceId);
    await api.loadSpaceMembers(alphaSpaceId, 1);
    await api.openThreadsList({ kind: "space", space_id: alphaSpaceId });
    await api.openFilesView(
      { kind: "space", space_id: alphaSpaceId },
      { kinds: [...allAttachmentKinds], filename_query: null },
      "newestFirst"
    );

    const before = await api.getSnapshot();
    const removed = await api.leaveRoom(alphaSpaceId);

    expect(removed.state.domain.spaces.map((space) => space.space_id)).toEqual([betaSpaceId]);
    expect(removed.state.domain.rooms).toEqual(
      before.state.domain.rooms.map((room) => ({
        ...room,
        parent_space_ids: room.parent_space_ids.filter((spaceId) => spaceId !== alphaSpaceId),
        dm_space_ids: room.dm_space_ids.filter((spaceId) => spaceId !== alphaSpaceId)
      }))
    );
    expect(removed.state.ui.navigation.active_space_id).toBeNull();
    expect(removed.state.ui.navigation.space_order).not.toContain(alphaSpaceId);
    expect(removed.state.ui.navigation.last_room_by_space_id).not.toHaveProperty(alphaSpaceId);
    expect(removed.state.ui.navigation.last_selection_by_space_id).not.toHaveProperty(alphaSpaceId);
    expect(removed.state.domain.space_members).toEqual({
      selected_space_id: null,
      generation: 0,
      space_joined: [],
      space_invited: [],
      child_room_only: [],
      child_room_count: 0,
      complete_child_room_count: 0,
      incomplete_child_room_count: 0,
      operation: { kind: "idle" }
    });
    expect(removed.state.ui.threads_list).toEqual({ kind: "closed" });
    expect(removed.state.ui.files_view).toEqual({ kind: "closed" });
    expect(removed.state.domain.rooms.map((room) => room.room_id)).toEqual(
      before.state.domain.rooms.map((room) => room.room_id)
    );
    expect(removed.state.domain.spaces[0]?.child_room_ids).toEqual(["!room-search:example.invalid"]);
  });

  test("Space removal retains unrelated Home and account scopes", async () => {
    const api = createBrowserFakeApi();
    await api.openThreadsList({ kind: "home" });
    await api.openFilesView(
      { kind: "account" },
      { kinds: [...allAttachmentKinds], filename_query: null },
      "newestFirst"
    );

    const removed = await api.forgetRoom(alphaSpaceId);

    expect(removed.state.ui.threads_list.kind).toBe("open");
    expect(removed.state.ui.files_view.kind).toBe("open");
    expect(removed.state.ui.threads_list.kind === "open" ? removed.state.ui.threads_list.items : []).toContainEqual(
      expect.objectContaining({ room_id: alphaRoomId })
    );
  });

  test("openThreadsList mirrors visible timeline thread summaries", async () => {
    const api = createBrowserFakeApi();

    const opened = await api.openThreadsList({ kind: "room", room_id: "!room-alpha:example.invalid" });

    expect(opened.state.ui.threads_list).toMatchObject({
      kind: "open",
      room_id: "!room-alpha:example.invalid",
      end_reached: true,
      items: [
        expect.objectContaining({
          root_event_id: "$alpha-update",
          root_body_preview: "Alpha keyword update from demo coordinator.",
          latest_event_id: "$thread-2",
          latest_body_preview: "Synthetic follow-up item two.",
          reply_count: 2
        })
      ]
    });
  });

  test("openThreadsList aggregates Home and Space scopes with owning room ids", async () => {
    const api = createBrowserFakeApi();

    const home = await api.openThreadsList({ kind: "home" });
    expect(home.state.ui.threads_list).toMatchObject({ kind: "open", room_id: "home" });
    expect(
      home.state.ui.threads_list.kind === "open"
        ? home.state.ui.threads_list.items.every((item) => item.room_id)
        : false
    ).toBe(true);

    const space = await api.openThreadsList({
      kind: "space",
      space_id: "!space-alpha:example.invalid"
    });
    expect(space.state.ui.threads_list).toMatchObject({
      kind: "open",
      room_id: "space:!space-alpha:example.invalid"
    });
    expect(
      space.state.ui.threads_list.kind === "open"
        ? space.state.ui.threads_list.items.every((item) => item.room_id === "!room-alpha:example.invalid")
        : false
    ).toBe(true);
  });

  test("openFilesView mirrors visible timeline attachments", async () => {
    const api = createBrowserFakeApi();

    const opened = await api.openFilesView(
      { kind: "room", room_id: "!room-alpha:example.invalid" },
      { kinds: ["image", "video", "audio", "file", "sticker"], filename_query: null },
      "newestFirst"
    );

    expect(opened.state.ui.files_view).toMatchObject({
      kind: "open",
      items: [
        expect.objectContaining({
          room_id: "!room-alpha:example.invalid",
          event_id: "$budget-file",
          filename: "fixture_budget.xlsx",
          kind: "file"
        })
      ]
    });
  });

  test("selectSearchResult anchors the main timeline without using the right-panel context", async () => {
    const api = createBrowserFakeApi();

    const focused = await api.selectSearchResult(
      "!room-alpha:example.invalid",
      "$alpha-update"
    );
    expect(focused.state.ui.focused_context.kind).toBe("closed");
    expect(focused.state.ui.navigation.main_timeline_anchor).toEqual({
      event_id: "$alpha-update"
    });

    const selected = await api.selectRoom("!room-planning:example.invalid");

    expect(selected.state.ui.focused_context).toEqual({ kind: "closed" });
  });

  test("openActivityEvent anchors the activity event in the main timeline", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await (api as unknown as {
      openActivityEvent(roomId: string, eventId: string): Promise<DesktopSnapshot>;
    }).openActivityEvent("!room-alpha:example.invalid", "$alpha-update");

    expect(snapshot.state.ui.focused_context).toEqual({ kind: "closed" });
    expect(snapshot.state.ui.navigation.main_timeline_anchor).toEqual({
      event_id: "$alpha-update"
    });
    expect(
      snapshot.state.ui.navigation.room_scroll_anchors?.["!room-alpha:example.invalid"]
    ).toBeUndefined();
  });

  test("initial browser fake snapshot starts with thread panel closed", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();

    expect(snapshot.state.ui.thread).toEqual({ kind: "closed" });
    expect(snapshot.state.domain.thread_attention).toEqual({ kind: "closed" });
    expect(snapshot.thread).toBeNull();
  });

  test("initial browser fake snapshot includes a pending invite fixture", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();

    expect(snapshot.state.domain.invites.map((invite) => invite.room_id)).toContain(
      "!invite-design-review:example.invalid"
    );
  });

  test("acceptInvite joins the invited room", async () => {
    const api = createBrowserFakeApi();

    const accepted = await api.acceptInvite("!invite-design-review:example.invalid");

    expect(
      accepted.state.domain.invites.some(
        (invite) => invite.room_id === "!invite-design-review:example.invalid"
      )
    ).toBe(false);
    expect(
      accepted.state.domain.rooms.some(
        (room) => room.room_id === "!invite-design-review:example.invalid"
      )
    ).toBe(true);
  });

  test("declineInvite removes the pending invite", async () => {
    const api = createBrowserFakeApi();

    const declined = await api.declineInvite("!invite-design-review:example.invalid");

    expect(
      declined.state.domain.invites.some(
        (invite) => invite.room_id === "!invite-design-review:example.invalid"
      )
    ).toBe(false);
  });

  test("builds invite workflow candidates and active-space scope plan", async () => {
    const api = createBrowserFakeApi();
    const roomId = "!room-alpha:example.invalid";

    const opened = await api.openInviteWorkflow(roomId);
    expect(opened.state.domain.invite_workflow?.scope_plan?.default_scope).toEqual({
      kind: "parentSpaceAndRoom",
      space_id: "!space-alpha:example.invalid"
    });

    const searched = await api.searchInviteTargets(roomId, "@new:example.invalid");
    expect(searched.state.domain.invite_workflow?.query.explicit_user_id).toMatchObject({
      user_id: "@new:example.invalid",
      status: "selectable"
    });

    const selected = await api.selectInviteTarget(roomId, "@new:example.invalid");
    expect(selected.state.domain.invite_workflow?.selected_targets).toEqual([
      {
        user_id: "@new:example.invalid",
        display_label: "@new:example.invalid",
        avatar: null
      }
    ]);
  });

  test("records already-in-space notice while continuing room invite", async () => {
    const api = createBrowserFakeApi();
    await api.loadRoomSettings("!space-alpha:example.invalid");

    const invited = await api.inviteTargets(
      "!room-alpha:example.invalid",
      ["@browser-member:browser.fake"],
      { kind: "parentSpaceAndRoom", space_id: "!space-alpha:example.invalid" }
    );

    expect(invited.state.domain.invite_workflow?.operation).toMatchObject({
      kind: "completed",
      notice: "既にスペースにいます",
      results: [
        { kind: "alreadyInSpace", destination: { kind: "space" } },
        { kind: "invited", destination: { kind: "room" } }
      ]
    });
  });

  test("models public directory query and join pending substates", async () => {
    const api = createBrowserFakeApi();

    const queryPromise = api.queryDirectory({
      term: "public rooms",
      server_name: "fake.local",
      limit: 20,
      since: null
    });
    expect((await api.getSnapshot()).state.domain.directory.query).toMatchObject({
      kind: "querying",
      query: {
        term: "public rooms",
        server_name: "fake.local",
        limit: 20,
        since: null
      }
    });

    const queried = await queryPromise;
    expect(queried.state.domain.directory.query.kind).toBe("results");

    const joinPromise = api.joinDirectoryRoom("#public-demo:fake.local", ["fake.local"]);
    expect((await api.getSnapshot()).state.domain.directory.join).toMatchObject({
      kind: "joining",
      room_id_or_alias: "#public-demo:fake.local",
      via_servers: ["fake.local"]
    });

    const joined = await joinPromise;
    expect(joined.state.domain.directory.join).toEqual({ kind: "idle" });
    expect(joined.state.ui.navigation.active_space_id).toBeNull();
    expect(joined.state.ui.navigation.active_room_id).toMatch(/^!joined-/);
    expect(joined.state.ui.timeline.room_id).toBe(joined.state.ui.navigation.active_room_id);
    expect(joined.sidebar.space_rooms).toContainEqual(
      expect.objectContaining({
        room_id: joined.state.ui.navigation.active_room_id,
        display_name: "public-demo"
      })
    );
  });

  test("models room management settings, moderation, and permission guard substates", async () => {
    const api = createBrowserFakeApi();

    const loaded = await api.loadRoomSettings("!browser-room:browser.fake");
    expect(loaded.state.domain.room_management).toMatchObject({
      selected_room_id: "!browser-room:browser.fake",
      settings: {
        room_id: "!browser-room:browser.fake",
        permissions: {
          can_edit_settings: true,
          can_edit_roles: true,
          can_kick: true,
          can_ban: true,
          can_invite: true,
          can_unban: true
        }
      },
      operation: { kind: "idle" }
    });

    const updatePromise = api.updateRoomSetting("!browser-room:browser.fake", {
      topic: "Updated topic"
    });
    expect((await api.getSnapshot()).state.domain.room_management.operation).toMatchObject({
      kind: "pending",
      operation: "settings"
    });
    const updated = await updatePromise;
    expect(updated.state.domain.room_management.settings?.topic).toBe("Updated topic");
    expect(updated.state.domain.room_management.operation).toEqual({ kind: "idle" });

    const moderated = await api.moderateRoomMember(
      "!browser-room:browser.fake",
      "@target:browser.fake",
      "kick",
      "Private reason"
    );
    expect(moderated.state.domain.room_management.operation).toEqual({ kind: "idle" });

    const readonly = await api.loadRoomSettings("!readonly-room:browser.fake");
    expect(readonly.state.domain.room_management.settings?.permissions.can_invite).toBe(false);
    const guarded = await api.moderateRoomMember(
      "!readonly-room:browser.fake",
      "@target:browser.fake",
      "kick",
      null
    );
    expect(guarded.state.domain.room_management.operation).toMatchObject({
      kind: "failed",
      operation: "moderation",
      failureKind: "forbidden"
    });
  });

  test("initializes exact room permission facts from typed fixture options", async () => {
    const roomId = "!space-alpha:example.invalid";
    const api = createBrowserFakeApi({
      roomPermissions: {
        [roomId]: {
          can_edit_settings: true,
          can_edit_roles: true,
          can_invite: true,
          can_kick: false,
          can_ban: true,
          can_unban: true
        }
      }
    });

    const loaded = await api.loadRoomSettings(roomId);

    expect(loaded.state.domain.room_management.settings?.permissions).toEqual({
      can_edit_settings: true,
      can_edit_roles: true,
      can_invite: true,
      can_kick: false,
      can_ban: true,
      can_unban: true
    });
  });

  test("models room member role updates from Rust-owned power-level facts", async () => {
    const api = createBrowserFakeApi();

    const loaded = await api.loadRoomSettings("!browser-room:browser.fake");
    const targetUserId = loaded.state.domain.room_management.settings?.members[0]?.user_id ?? "";
    expect(targetUserId).toBeTruthy();
    expect(loaded.state.domain.room_management.settings?.members[0]).toMatchObject({
      power_level: 0,
      role: "user"
    });

    const updatePromise = api.updateRoomMemberRole(
      "!browser-room:browser.fake",
      targetUserId,
      50
    );
    expect((await api.getSnapshot()).state.domain.room_management.operation).toMatchObject({
      kind: "pending",
      operation: "roles"
    });

    const updated = await updatePromise;
    expect(updated.state.domain.room_management.operation).toEqual({ kind: "idle" });
    expect(updated.state.domain.room_management.settings?.members[0]).toMatchObject({
      user_id: targetUserId,
      power_level: 50,
      role: "moderator"
    });

    await api.loadRoomSettings("!readonly-room:browser.fake");
    const guarded = await api.updateRoomMemberRole(
      "!readonly-room:browser.fake",
      targetUserId,
      100
    );
    expect(guarded.state.domain.room_management.operation).toMatchObject({
      kind: "failed",
      operation: "roles",
      failureKind: "forbidden"
    });
  });

  test("models activity recent, unread, pagination, and mark-read substates", async () => {
    const api = createBrowserFakeApi();

    const opened = await api.openActivity();
    expect(opened.state.domain.activity.kind).toBe("open");
    if (opened.state.domain.activity.kind !== "open") {
      throw new Error("activity should be open");
    }
    expect(opened.state.domain.activity.active_tab).toBe("recent");
    expect(opened.state.domain.activity.recent.rows.map((row) => row.event_id).slice(0, 3)).toEqual([
      "$search-dev-note",
      "$late-original",
      "$false-positive"
    ]);
    expect(opened.state.domain.activity.recent.rows.slice(0, 3).map((row) => row.context_label)).toEqual([
      "Synthetic Lab / matrix-sdk-search",
      "Synthetic Workspace / planning-room",
      "Synthetic Workspace / synthetic-room"
    ]);
    expect(
      opened.state.domain.activity.recent.rows.filter((row) => row.kind === "event").every((row) =>
        Boolean(row.context_label)
      )
    ).toBe(true);
    expect(opened.state.domain.activity.recent.rows.every((row) => row.kind === "event")).toBe(
      true
    );
    expect(
      opened.state.domain.activity.unread.rows.every(
        (row) => row.kind === "roomUnread" && row.event_id === null
      )
    ).toBe(true);
    expect(
      opened.state.domain.activity.unread.rows.some(
        (row) => row.room_id === "!room-alpha:example.invalid"
      )
    ).toBe(true);
    expect(
      opened.state.domain.activity.unread.rows.some(
        (row) => row.room_id === "!dm-member-1:example.invalid"
      )
    ).toBe(true);

    const switched = await api.setActivityTab("unread");
    expect(switched.state.domain.activity.kind).toBe("open");
    if (switched.state.domain.activity.kind !== "open") {
      throw new Error("activity should stay open");
    }
    expect(switched.state.domain.activity.active_tab).toBe("unread");
    expect(switched.state.domain.activity.unread.resolution.kind).toBe("resolving");
    expect(
      switched.state.domain.activity.unread.rows.every(
        (row) => row.kind === "roomUnread" && row.event_id === null
      )
    ).toBe(true);

    switched.state.domain.activity.unread.resolution = {
      kind: "failed",
      generation: 1,
      unresolved_room_count: 2,
      failure_kind: "network"
    };
    const retried = await api.retryActivityResolution();
    expect(retried.state.domain.activity.kind).toBe("open");
    if (retried.state.domain.activity.kind !== "open") {
      throw new Error("activity should stay open after retry");
    }
    expect(retried.state.domain.activity.unread.resolution.kind).toBe("resolving");

    const paged = await api.paginateActivity("recent", switched.state.domain.activity.recent.next_batch);
    expect(paged.state.domain.activity.kind).toBe("open");
    if (paged.state.domain.activity.kind !== "open") {
      throw new Error("activity should stay open after pagination");
    }
    expect(paged.state.domain.activity.recent.rows.at(-1)?.event_id).toBe("$alpha-history");
    expect(paged.state.domain.activity.recent.next_batch).toBeNull();

    const markedRoom = await api.markActivityRead({
      kind: "room",
      room_id: "!room-alpha:example.invalid",
      up_to_event_id: "$false-positive"
    });
    expect(markedRoom.state.domain.activity.kind).toBe("open");
    if (markedRoom.state.domain.activity.kind !== "open") {
      throw new Error("activity should stay open after mark-read");
    }
    expect(markedRoom.state.domain.activity.mark_read).toEqual({ kind: "idle" });
    expect(
      markedRoom.state.domain.activity.unread.rows.some(
        (row) => row.room_id === "!room-alpha:example.invalid"
      )
    ).toBe(false);

    const markedAll = await api.markActivityRead({ kind: "all" });
    expect(markedAll.state.domain.activity.kind).toBe("open");
    if (markedAll.state.domain.activity.kind !== "open") {
      throw new Error("activity should stay open after mark-all-read");
    }
    expect(markedAll.state.domain.activity.unread.rows).toEqual([]);
  });

  test("removes muted rooms from activity unread rows", async () => {
    const api = createBrowserFakeApi();

    await api.openActivity();
    const muted = await api.setRoomNotificationMode("!room-alpha:example.invalid", {
      kind: "mute"
    });

    expect(muted.state.domain.activity.kind).toBe("open");
    if (muted.state.domain.activity.kind !== "open") {
      throw new Error("activity should stay open");
    }
    expect(
      muted.state.domain.activity.unread.rows.some(
        (row) => row.room_id === "!room-alpha:example.invalid"
      )
    ).toBe(false);
  });

  test("preserves the selected activity tab across close and duplicate open", async () => {
    const api = createBrowserFakeApi();

    await api.openActivity();
    const selected = await api.setActivityTab("unread");
    expect(selected.state.domain.activity).toMatchObject({
      kind: "open",
      active_tab: "unread"
    });

    await api.closeActivity();
    const reopened = await api.openActivity();
    expect(reopened.state.domain.activity).toMatchObject({
      kind: "open",
      active_tab: "unread"
    });

    const duplicate = await api.openActivity();
    expect(duplicate.state.domain.activity).toMatchObject({
      kind: "open",
      active_tab: "unread"
    });
  });

  test("removes notification-only rooms from activity recent unless highlighted", async () => {
    const api = createBrowserFakeApi();

    await api.openActivity();
    const updated = await api.setRoomNotificationMode("!room-alpha:example.invalid", {
      kind: "mentions"
    });

    expect(updated.state.domain.activity.kind).toBe("open");
    if (updated.state.domain.activity.kind !== "open") {
      throw new Error("activity should open after notification mode change");
    }
    expect(
      updated.state.domain.activity.recent.rows.some(
        (row) => row.room_id === "!room-alpha:example.invalid" && !row.highlight
      )
    ).toBe(false);
  });

  test("editing an attachment caption keeps the attachment", async () => {
    // Core edits a media event's caption in place, so the attachment survives
    // the edit (issue #328). A fake that cleared it here would hide the bug.
    const api = createBrowserFakeApi();
    const roomId = "!room-alpha:example.invalid";
    await api.selectRoom(roomId);

    const edited = await api.editMessage(roomId, "$budget-file", documentFromText("Edited caption."));

    expect(
      edited.timeline.find((message) => message.event_id === "$budget-file")
    ).toMatchObject({
      body: "Edited caption.",
      attachment_filename: "fixture_budget.xlsx"
    });
  });

  test("startDirectMessage gets-or-creates and opens the DM", async () => {
    // The fake must match the real backend's contract (#368): the first call
    // creates the DM and opens it, and a repeat call reuses the same room
    // instead of minting a duplicate.
    const api = createBrowserFakeApi();
    const target = "@dm-target:example.invalid";
    const before = (await api.getSnapshot()).state.domain.rooms.length;

    const created = await api.startDirectMessage(target);
    const createdRooms = created.state.domain.rooms.filter(
      (room) => room.is_dm && room.dm_user_ids.length === 1 && room.dm_user_ids[0] === target
    );
    expect(createdRooms).toHaveLength(1);
    expect(created.state.domain.rooms).toHaveLength(before + 1);
    expect(created.state.ui.navigation.active_room_id).toBe(createdRooms[0].room_id);

    await api.selectRoom("!room-alpha:example.invalid");
    const reused = await api.startDirectMessage(target);

    expect(
      reused.state.domain.rooms.filter(
        (room) => room.is_dm && room.dm_user_ids.length === 1 && room.dm_user_ids[0] === target
      )
    ).toHaveLength(1);
    expect(reused.state.domain.rooms).toHaveLength(before + 1);
    expect(reused.state.ui.navigation.active_room_id).toBe(createdRooms[0].room_id);
  });

  test("models local encryption health probe as Rust-owned state", async () => {
    const api = createBrowserFakeApi();

    expect((await api.getSnapshot()).state.domain.local_encryption).toEqual({ kind: "unknown" });

    const probing = api.probeLocalEncryptionHealth();
    expect((await api.getSnapshot()).state.domain.local_encryption).toMatchObject({
      kind: "probing",
      request_id: expect.any(Number)
    });

    const snapshot = await probing;
    expect(snapshot.state.domain.local_encryption).toEqual({ kind: "healthy" });
  });

  test("isolates link-preview hiding between fake instances", async () => {
    const roomId = "!room-alpha:example.invalid";
    const eventId = "$alpha-update";
    const fakeA = createBrowserFakeApi();
    const initialA = await fakeA.getSnapshot();
    expect(
      initialA.timeline.find(
        (message) => message.room_id === roomId && message.event_id === eventId
      )?.link_previews
    ).toBeUndefined();

    const hiddenA = await fakeA.hideLinkPreview(roomId, eventId);
    expect(
      hiddenA.timeline.find(
        (message) => message.room_id === roomId && message.event_id === eventId
      )?.link_previews
    ).toEqual([]);

    const fakeB = createBrowserFakeApi();
    const initialB = await fakeB.getSnapshot();
    expect(
      initialB.timeline.find(
        (message) => message.room_id === roomId && message.event_id === eventId
      )?.link_previews
    ).toBeUndefined();
  });

  test("allocates distinct request IDs for searches in one millisecond", async () => {
    const api = createBrowserFakeApi();
    const now = vi.spyOn(Date, "now").mockReturnValue(1_700_000_000_000);

    try {
      const first = await api.submitSearch("Alpha", "allRooms");
      const second = await api.submitSearch("Beta", "allRooms");
      const firstSearch = first.state.domain.search;
      const secondSearch = second.state.domain.search;

      expect(firstSearch.kind).toBe("results");
      expect(secondSearch.kind).toBe("results");
      if (firstSearch.kind !== "results" || secondSearch.kind !== "results") {
        throw new Error("expected search results");
      }
      expect(secondSearch.request_id).toBeGreaterThan(firstSearch.request_id);
    } finally {
      now.mockRestore();
    }
  });
});

describe("BrowserFakeApi prepared upload lifecycle", () => {
  const roomId = "!room-alpha:example.invalid";
  const otherRoomId = "!room-planning:example.invalid";
  const error = "attachment batch is empty or exceeds the supported limit";

  function item(stagedId: string, bytes: number[]): {
    stagedId: string;
    position: number;
    filename: string;
    mimeType: string;
    bytes: number[];
  } {
    return { stagedId, position: 0, filename: `${stagedId}.txt`, mimeType: "text/plain", bytes };
  }

  async function rootId(api: ReturnType<typeof createBrowserFakeApi>): Promise<string> {
    return (await api.getSnapshot()).timeline[0]!.event_id;
  }

  async function stageMain(api: ReturnType<typeof createBrowserFakeApi>, stagedId: string, bytes = [1, 2, 3]) {
    await api.selectRoom(roomId);
    await api.stageUploadBytes({ kind: "main", room_id: roomId }, [item(stagedId, bytes)]);
  }

  async function stageThread(
    api: ReturnType<typeof createBrowserFakeApi>,
    root: string,
    stagedId: string,
    bytes = [4, 5, 6]
  ) {
    await api.openThread(roomId, root, "existingThread");
    await api.stageUploadBytes(
      { kind: "thread", room_id: roomId, root_event_id: root },
      [item(stagedId, bytes)]
    );
  }

  test("replacing main staging removes disjoint prepared bytes", async () => {
    const api = createBrowserFakeApi();
    const target = { kind: "main" as const, room_id: roomId };
    await stageMain(api, "old", [1]);
    await api.stageUploadBytes(target, [item("new", [2])]);

    await expect(api.preparedUploadPreview(target, "old", "original-keep")).resolves.toEqual([]);
    await expect(api.preparedUploadPreview(target, "new", "original-keep")).resolves.toEqual([2]);
  });

  test.each([
    ["empty", []],
    ["17 items", Array.from({ length: 17 }, (_, index) => item(`item-${index}`, [index]))],
    ["over 128 MiB", [item("sparse", Object.assign([], { length: 128 * 1024 * 1024 + 1 }))]]
  ])("rejects %s batches before the active guard without mutation", async (_name, items) => {
    const api = createBrowserFakeApi();
    const active = { kind: "main" as const, room_id: roomId };
    await stageMain(api, "retained", [9]);
    const before = await api.getSnapshot();

    await expect(
      api.stageUploadBytes({ kind: "main", room_id: otherRoomId }, items)
    ).rejects.toThrow(error);
    expect(await api.getSnapshot()).toEqual(before);
    await expect(api.preparedUploadPreview(active, "retained", "original-keep")).resolves.toEqual([9]);
  });

  test("explicit close clears thread prepared bytes", async () => {
    const api = createBrowserFakeApi();
    const root = await rootId(api);
    await stageThread(api, root, "thread-upload");
    await api.closeThread();

    await expect(
      api.preparedUploadPreview({ kind: "thread", room_id: roomId, root_event_id: root }, "thread-upload", "original-keep")
    ).resolves.toEqual([]);
  });

  test("opening root B clears root A prepared bytes", async () => {
    const api = createBrowserFakeApi();
    const rootA = await rootId(api);
    const rootB = (await api.getSnapshot()).timeline[1]!.event_id;
    await stageThread(api, rootA, "root-a");
    await api.openThread(roomId, rootB, "existingThread");

    await expect(api.preparedUploadPreview({ kind: "thread", room_id: roomId, root_event_id: rootA }, "root-a", "original-keep")).resolves.toEqual([]);
  });

  test.each(["select room", "select home"]) ("%s implicitly closes the thread", async (operation) => {
    const api = createBrowserFakeApi();
    const root = await rootId(api);
    await stageThread(api, root, "navigation-upload");
    if (operation === "select room") {
      await api.selectRoom(otherRoomId);
    } else {
      await api.selectSpace(null);
    }

    await expect(api.preparedUploadPreview({ kind: "thread", room_id: roomId, root_event_id: root }, "navigation-upload", "original-keep")).resolves.toEqual([]);
  });

  test("room removal clears main prepared bytes", async () => {
    const api = createBrowserFakeApi();
    const target = { kind: "main" as const, room_id: roomId };
    await stageMain(api, "removed");
    await api.leaveRoom(roomId);

    await expect(api.preparedUploadPreview(target, "removed", "original-keep")).resolves.toEqual([]);
  });

  test.each([
    "completeOidcLogin",
    "submitLogin",
    "switchAccount",
    "changeHomeserver",
    "logout",
    "resetLocalData"
  ])("%s clears the prepared byte cache", async (operation) => {
    const api = createBrowserFakeApi();
    const target = { kind: "main" as const, room_id: roomId };
    await stageMain(api, operation);
    if (operation === "completeOidcLogin") await api.completeOidcLogin("https://example.invalid", "callback");
    if (operation === "submitLogin") await api.submitLogin("https://example.invalid", "user", "password", "device", "linux");
    if (operation === "switchAccount") await api.switchAccount((await api.listSavedSessions())[1]!);
    if (operation === "changeHomeserver") await api.changeHomeserver();
    if (operation === "logout") await api.logout();
    if (operation === "resetLocalData") await api.resetLocalData();

    await expect(api.preparedUploadPreview(target, operation, "original-keep")).resolves.toEqual([]);
  });

  test("clear and send clean only their target and preserve the other kind", async () => {
    const api = createBrowserFakeApi();
    const main = { kind: "main" as const, room_id: roomId };
    const root = await rootId(api);
    const thread = { kind: "thread" as const, room_id: roomId, root_event_id: root };
    await stageMain(api, "main");
    await stageThread(api, root, "thread");
    await api.clearUploadStaging(thread);
    await expect(
      api.preparedUploadPreview(thread, "thread", "original-keep")
    ).resolves.toEqual([]);
    await expect(api.preparedUploadPreview(main, "main", "original-keep")).resolves.toEqual([
      1, 2, 3
    ]);

    await api.stageUploadBytes(thread, [item("thread-again", [7])]);
    const account = await readyAccount(api);
    const { generation, lease } = await beginComposerLease(api, account, thread);
    await api.sendPreparedUploads(account, lease.leaseId, generation, thread, revision("0"));
    await expect(
      api.preparedUploadPreview(thread, "thread-again", "original-keep")
    ).resolves.toEqual([]);
    await expect(api.preparedUploadPreview(main, "main", "original-keep")).resolves.toEqual([
      1, 2, 3
    ]);
  });
});

describe("BrowserFakeApi async completion fences", () => {
  async function expectSignedOut(result: Promise<DesktopSnapshot>) {
    const signedOut = await createBrowserFakeApi({ session: "signedOut" }).getSnapshot();
    const snapshot = await result;
    expect(snapshot.state.domain.session).toEqual({ kind: "signedOut" });
    expect(resetSessionViewProjection(snapshot)).toEqual(resetSessionViewProjection(signedOut));
    expect(snapshot.state.domain.profile).toEqual(signedOut.state.domain.profile);
    expect(snapshot.state.domain.rooms).toEqual([]);
    expect(snapshot.state.domain.directory).toEqual(signedOut.state.domain.directory);
    expect(snapshot.state.domain.room_management).toEqual(signedOut.state.domain.room_management);
    expect(snapshot.state.domain.activity).toEqual(signedOut.state.domain.activity);
    return snapshot;
  }

  test.each([
    ["probeLocalEncryptionHealth", async () => {}, (api: ReturnType<typeof createBrowserFakeApi>) => api.probeLocalEncryptionHealth()],
    ["setLocalUserAlias", async () => {}, (api: ReturnType<typeof createBrowserFakeApi>) => api.setLocalUserAlias("@alias:example.invalid", "Alias")],
    ["ignoreUser", async () => {}, (api: ReturnType<typeof createBrowserFakeApi>) => api.ignoreUser("@ignored:example.invalid")],
    ["queryDirectory", async () => {}, (api: ReturnType<typeof createBrowserFakeApi>) => api.queryDirectory({ term: "synthetic", server_name: null, limit: 10, since: null })],
    ["previewJoinTarget", async () => {}, (api: ReturnType<typeof createBrowserFakeApi>) => api.previewJoinTarget("#synthetic:example.invalid")],
    ["joinDirectoryRoom", async (api: ReturnType<typeof createBrowserFakeApi>) => {
      await api.previewJoinTarget("#synthetic:example.invalid");
    }, (api: ReturnType<typeof createBrowserFakeApi>) => api.joinDirectoryRoom("#synthetic:example.invalid")],
    ["updateRoomSetting", async (api: ReturnType<typeof createBrowserFakeApi>) => {
      await api.loadRoomSettings("!room-alpha:example.invalid");
    }, (api: ReturnType<typeof createBrowserFakeApi>) => api.updateRoomSetting("!room-alpha:example.invalid", { name: "Renamed" })],
    ["moderateRoomMember", async (api: ReturnType<typeof createBrowserFakeApi>) => {
      await api.loadRoomSettings("!room-alpha:example.invalid");
    }, (api: ReturnType<typeof createBrowserFakeApi>) => api.moderateRoomMember("!room-alpha:example.invalid", "@member:example.invalid", "kick")],
    ["updateRoomMemberRole", async (api: ReturnType<typeof createBrowserFakeApi>) => {
      await api.loadRoomSettings("!room-alpha:example.invalid");
    }, (api: ReturnType<typeof createBrowserFakeApi>) => api.updateRoomMemberRole("!room-alpha:example.invalid", "@member:example.invalid", 50)],
    ["openActivity", async () => {}, (api: ReturnType<typeof createBrowserFakeApi>) => api.openActivity()],
    ["markActivityRead", async (api: ReturnType<typeof createBrowserFakeApi>) => {
      await api.openActivity();
    }, (api: ReturnType<typeof createBrowserFakeApi>) => api.markActivityRead({ kind: "all" })]
  ] as const)("does not apply stale %s completion after logout", async (_name, prepare, start) => {
    const api = createBrowserFakeApi();
    await prepare(api);
    const pending = start(api);
    await api.logout();
    await expectSignedOut(pending);
  });

  test.each(["completeOidcLogin", "submitLogin", "switchAccount", "changeHomeserver", "logout"] as const)(
    "alias completion is fenced by %s",
    async (operation) => {
      const api = createBrowserFakeApi();
      const sessions = operation === "switchAccount" ? await api.listSavedSessions() : [];
      const pending = api.setLocalUserAlias("@alias:example.invalid", "Alias");
      if (operation === "completeOidcLogin") await api.completeOidcLogin("https://example.invalid", "callback");
      if (operation === "submitLogin") await api.submitLogin("https://example.invalid", "user", "password", "device", "linux");
      if (operation === "switchAccount") await api.switchAccount(sessions[1]!);
      if (operation === "changeHomeserver") await api.changeHomeserver();
      if (operation === "logout") await api.logout();
      const snapshot = await pending;
      expect(snapshot.state.domain.profile.local_aliases).toEqual({});
      expect(snapshot.state.domain.profile.local_alias_update).toEqual({ kind: "idle" });
    }
  );

  test("resetLocalData settles before a stale alias continuation", async () => {
    const api = createBrowserFakeApi();
    const reset = api.resetLocalData();
    const alias = api.setLocalUserAlias("@alias:example.invalid", "Alias");
    await reset;
    const snapshot = await alias;
    expect(snapshot.state.domain.session).toEqual({ kind: "signedOut" });
    expect(snapshot.state.domain.profile.local_aliases).toEqual({});
    expect(snapshot.state.domain.profile.local_alias_update).toEqual({ kind: "idle" });
  });

  test("unignore completion cannot settle a newer account operation", async () => {
    const userId = "@ignored:example.invalid";
    const api = createBrowserFakeApi();
    await api.ignoreUser(userId);
    const stale = api.unignoreUser(userId);
    const replacement = api.completeOidcLogin("https://example.invalid", "callback");
    const current = api.ignoreUser(userId);
    await replacement;

    const staleSnapshot = await stale;
    expect(staleSnapshot.state.domain.profile.ignored_user_update.kind).toBe("saving");
    const currentSnapshot = await current;
    expect(currentSnapshot.state.domain.profile.ignored_user_ids).toEqual([userId]);
    expect(currentSnapshot.state.domain.profile.ignored_user_update).toEqual({ kind: "idle" });
  });

  test("resetLocalData cannot overwrite a ready OIDC replacement", async () => {
    const api = createBrowserFakeApi();
    const reset = api.resetLocalData();
    await api.completeOidcLogin("https://example.invalid", "callback");
    const snapshot = await reset;
    expect(snapshot.state.domain.session.kind).toBe("ready");
  });

  test("directory query A is superseded by query B", async () => {
    const api = createBrowserFakeApi();
    const query = { term: "synthetic", server_name: null, limit: 10, since: null };
    const first = api.queryDirectory(query);
    const second = api.queryDirectory({ ...query, term: "newer" });
    const stale = await first;
    expect(stale.state.domain.directory.query).toMatchObject({ kind: "querying", query: { term: "newer" } });
    const current = await second;
    expect(current.state.domain.directory.query).toMatchObject({ kind: "results", query: { term: "newer" } });
  });
});
