import { describe, expect, test } from "vitest";

import { createBrowserFakeApi } from "./browserFakeApi";

describe("BrowserFakeApi settings preview", () => {
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
      "!room-planning:example.invalid",
      "!room-search:example.invalid"
    ]);

    const people = await api.selectRoomListFilter({ kind: "people" });
    expect(people.state.ui.room_list.items).toEqual([
      { room_id: "!dm-member-1:example.invalid", kind: "room" },
      { room_id: "!dm-member-2:example.invalid", kind: "room" }
    ]);

    const unread = await api.selectRoomListFilter({ kind: "unread" });
    expect(unread.state.ui.room_list.items?.map((item) => item.room_id)).toEqual([
      "!dm-member-1:example.invalid",
      "!room-alpha:example.invalid",
      "!room-planning:example.invalid",
      "!room-search:example.invalid"
    ]);

    await api.setRoomTag("!room-planning:example.invalid", "favourite");
    const favourites = await api.selectRoomListFilter({ kind: "favourites" });
    expect(favourites.state.ui.room_list.items).toEqual([
      { room_id: "!room-planning:example.invalid", kind: "room" }
    ]);

    const invites = await api.selectRoomListFilter({ kind: "invites" });
    expect(invites.state.ui.room_list.items).toEqual([
      { room_id: "!invite-design-review:example.invalid", kind: "invite" }
    ]);
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
    const reset = await api.submitIdentityResetPassword(flow, "synthetic-password");
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

  test("selectRoom closes focused context after search navigation", async () => {
    const api = createBrowserFakeApi();

    const focused = await api.selectSearchResult(
      "!room-alpha:example.invalid",
      "$alpha-update"
    );
    expect(focused.state.ui.focused_context.kind).toBe("opening");

    const selected = await api.selectRoom("!room-planning:example.invalid");

    expect(selected.state.ui.focused_context).toEqual({ kind: "closed" });
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
    expect(opened.state.domain.activity.unread.rows.some((row) => row.event_id === "$alpha-update")).toBe(
      true
    );

    const switched = await api.setActivityTab("unread");
    expect(switched.state.domain.activity.kind).toBe("open");
    if (switched.state.domain.activity.kind !== "open") {
      throw new Error("activity should stay open");
    }
    expect(switched.state.domain.activity.active_tab).toBe("unread");
    expect(switched.state.domain.activity.unread.rows.some((row) => row.event_id === "$alpha-update")).toBe(
      true
    );

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
