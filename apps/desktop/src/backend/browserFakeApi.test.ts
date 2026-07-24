import { describe, expect, test } from "vitest";

import { createBrowserFakeApi } from "./browserFakeApi";
import { parseComposerDraftRevision as revision } from "../domain/composerDraftRevision";
import type { DesktopSnapshot, LiveReadReceipt } from "../domain/types";

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
    const before = (await api.getSnapshot()).timeline.length;

    const first = await api.sendText(account, "submission-same", roomId, "original");
    const replay = await api.sendText(account, "submission-same", roomId, "changed");

    expect(first.outcome).toBe("accepted");
    expect(replay.transactionId).toBe(first.transactionId);
    expect(replay.snapshot.timeline).toHaveLength(before + 1);
    expect(replay.snapshot.timeline.at(-1)?.body).toBe("original");
    expect(replay.snapshot.state.ui.timeline.composer.pending_submission_id).toBeNull();
    expect(replay.snapshot.state.ui.timeline.composer.accepted_submission_ids).toContain("submission-same");
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
    await api.setComposerDraft(account, roomId, "main accepted", revision("1"));
    const sent = await api.sendText(
      account,
      "revision-main",
      roomId,
      "main accepted",
      { targets: [] },
      revision("1")
    );
    expect(sent.outcome).toBe("accepted");
    expect(sent.snapshot.state.ui.timeline.composer).toMatchObject({
      draft: "",
      draft_revision: "2"
    });
    const staleMain = await api.setComposerDraft(
      account,
      roomId,
      "main accepted",
      revision("1")
    );
    expect(staleMain.state.ui.timeline.composer.draft).toBe("");
    const nextMain = await api.setComposerDraft(
      account,
      roomId,
      "immediate next",
      revision("3")
    );
    expect(nextMain.state.ui.timeline.composer.draft).toBe("immediate next");
    const lateMainAcceptance = await api.sendText(
      account,
      "revision-main-late",
      roomId,
      "main accepted",
      { targets: [] },
      revision("1")
    );
    expect(lateMainAcceptance.snapshot.state.ui.timeline.composer).toMatchObject({
      draft: "immediate next",
      draft_revision: "4"
    });

    const rootId = nextMain.timeline[0]!.event_id;
    await api.openThread(roomId, rootId);
    await api.setThreadComposerDraft(
      account,
      roomId,
      rootId,
      "thread accepted",
      revision("5")
    );
    const threadSent = await api.sendThreadReply(
      account,
      "revision-thread",
      roomId,
      rootId,
      "thread accepted",
      { targets: [] },
      revision("5")
    );
    expect(threadSent.outcome).toBe("accepted");
    const staleThread = await api.setThreadComposerDraft(
      account,
      roomId,
      rootId,
      "thread accepted",
      revision("5")
    );
    expect(staleThread.state.ui.thread).toMatchObject({
      kind: "open",
      composer: { draft: "", draft_revision: "6" }
    });
    await api.setThreadComposerDraft(
      account,
      roomId,
      rootId,
      "immediate thread next",
      revision("7")
    );
    const lateThreadAcceptance = await api.sendThreadReply(
      account,
      "revision-thread-late",
      roomId,
      rootId,
      "thread accepted",
      { targets: [] },
      revision("5")
    );
    expect(lateThreadAcceptance.snapshot.state.ui.thread).toMatchObject({
      kind: "open",
      composer: { draft: "immediate thread next", draft_revision: "8" }
    });
  });

  test("rejects draft writes and acceptances captured for another account", async () => {
    const api = createBrowserFakeApi();
    const roomId = "!room-alpha:example.invalid";
    await api.selectRoom(roomId);
    const before = await api.getSnapshot();
    const rootId = before.timeline[0]!.event_id;
    await api.openThread(roomId, rootId);

    const staleAccount = {
      homeserver: "https://stale.example.invalid",
      userId: "@stale-account:example.invalid",
      deviceId: "STALE"
    };
    const main = await api.setComposerDraft(
      staleAccount,
      roomId,
      "must not cross accounts",
      revision("1")
    );
    const thread = await api.setThreadComposerDraft(
      staleAccount,
      roomId,
      rootId,
      "must not cross accounts",
      revision("1")
    );

    expect(main.state.ui.timeline.composer.draft).toBe("");
    expect(thread.state.ui.thread).toMatchObject({
      kind: "open",
      composer: { draft: "", draft_revision: "0" }
    });
    const staleMainSend = await api.sendText(
      staleAccount,
      "stale-main-send",
      roomId,
      "must not send"
    );
    const staleThreadSend = await api.sendThreadReply(
      staleAccount,
      "stale-thread-send",
      roomId,
      rootId,
      "must not send"
    );
    const staleSchedule = await api.scheduleSend(
      staleAccount,
      { kind: "thread", room_id: roomId, root_event_id: rootId },
      "must not schedule",
      Date.now() + 60_000,
      revision("0")
    );
    const threadTarget = {
      kind: "thread" as const,
      room_id: roomId,
      root_event_id: rootId
    };
    await api.stageUploadBytes(threadTarget, [
      {
        stagedId: "stale-account-upload",
        position: 0,
        filename: "synthetic.txt",
        mimeType: "text/plain",
        bytes: [1, 2, 3]
      }
    ]);
    const stalePreparedUpload = await api.sendPreparedUploads(
      staleAccount,
      threadTarget,
      revision("0")
    );
    expect(staleMainSend.outcome).toMatchObject({ rejected: { kind: "invalid" } });
    expect(staleThreadSend.outcome).toMatchObject({ rejected: { kind: "invalid" } });
    expect(staleSchedule.acceptedRevision).toBeNull();
    expect(staleSchedule.snapshot.state.ui.timeline.scheduled_sends).toHaveLength(0);
    expect(stalePreparedUpload.acceptedRevision).toBeNull();
    expect(stalePreparedUpload.snapshot.state.ui.thread).toMatchObject({
      kind: "open",
      staged_uploads: [{ staged_id: "stale-account-upload" }]
    });
  });

  test("deduplicates reply submissions without incrementing the root twice", async () => {
    const api = createBrowserFakeApi();
    const roomId = "!room-alpha:example.invalid";
    await api.selectRoom(roomId);
    const account = await readyAccount(api);
    const root = (await api.getSnapshot()).timeline[0]!;
    const before = root.reply_count;
    await api.sendReply(account, "reply-same", roomId, root.event_id, "original");
    const replay = await api.sendReply(account, "reply-same", roomId, root.event_id, "changed");
    expect(replay.snapshot.timeline.find((item) => item.event_id === root.event_id)?.reply_count).toBe(before + 1);
  });

  test("deduplicates an unknown thread retry and preserves terminal correlation fields", async () => {
    const api = createBrowserFakeApi();
    const roomId = "!room-alpha:example.invalid";
    await api.selectRoom(roomId);
    const account = await readyAccount(api);
    const rootId = (await api.getSnapshot()).timeline[0]!.event_id;
    await api.openThread(roomId, rootId);
    const first = await api.sendThreadReply(account, "thread-unknown", roomId, rootId, "original");
    const replay = await api.sendThreadReply(account, "thread-unknown", roomId, rootId, "edited");
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
    for (let index = 0; index < 129; index += 1) {
      await api.sendText(account, `bounded-${index}`, roomId, `body-${index}`);
    }
    const bounded = await api.getSnapshot();
    const before = bounded.timeline.length;
    expect(bounded.state.ui.timeline.submission_registry.accepted_submission_ids).toHaveLength(0);
    expect(bounded.state.ui.timeline.submission_registry.settled_submission_ids).toHaveLength(128);
    expect(bounded.state.ui.timeline.submission_registry.settled_submission_ids).not.toContain("bounded-0");
    await api.sendText(account, "bounded-1", roomId, "deduped");
    expect((await api.getSnapshot()).timeline).toHaveLength(before);
    await api.sendText(account, "bounded-0", roomId, "evicted");
    expect((await api.getSnapshot()).timeline).toHaveLength(before + 1);
  });

  test("returns an empty diagnostic snapshot in the browser fake", async () => {
    const api = createBrowserFakeApi();

    await expect(api.getDiagnosticSnapshot()).resolves.toEqual({
      entries: [],
      droppedEntries: 0
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
      typing_user_ids: []
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

    await api.openThreadsList("!room-alpha:example.invalid");
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

  test("openThreadsList mirrors visible timeline thread summaries", async () => {
    const api = createBrowserFakeApi();

    const opened = await api.openThreadsList("!room-alpha:example.invalid");

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

    const joinPromise = api.joinDirectoryRoom("#public-demo:fake.local", "fake.local");
    expect((await api.getSnapshot()).state.domain.directory.join).toMatchObject({
      kind: "joining",
      alias: "#public-demo:fake.local",
      via_server: "fake.local"
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

    await api.loadRoomSettings("!readonly-room:browser.fake");
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
});
