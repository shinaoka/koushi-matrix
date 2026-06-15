import { describe, expect, test } from "vitest";

import { createBrowserFakeApi } from "./browserFakeApi";

describe("BrowserFakeApi settings preview", () => {
  test("applies the Rust-shaped settings patch to the fixture snapshot", async () => {
    const api = createBrowserFakeApi();

    const snapshot = await api.updateSettings({
      appearance: { theme: "dark" },
      keyboard: { composer_send_shortcut: "modEnter" }
    });

    expect(snapshot.state.settings.values.appearance.theme).toBe("dark");
    expect(snapshot.state.settings.values.keyboard.composer_send_shortcut).toBe("modEnter");
    expect(snapshot.state.settings.persistence).toEqual({ kind: "idle" });
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

    expect(snapshot.state.locale_profile).toMatchObject({
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
    expect(named.state.profile.own.display_name).toBe("Alice");
    expect(named.state.profile.update).toEqual({ kind: "idle" });

    const avatar = await api.setAvatar("image/png", [1, 2, 3, 4]);
    expect(avatar.state.profile.own.avatar).toEqual({
      mxc_uri: "mxc://browser.fake/profile-avatar",
      thumbnail: { kind: "notRequested" }
    });
    expect(avatar.state.profile.update).toEqual({ kind: "idle" });
  });

  test("updates the Rust-shaped E2EE trust snapshot for preview controls", async () => {
    const api = createBrowserFakeApi();

    await expect(api.bootstrapCrossSigning()).resolves.toMatchObject({
      state: {
        e2ee_trust: {
          cross_signing: { kind: "trusted" }
        }
      }
    });

    await expect(api.enableKeyBackup()).resolves.toMatchObject({
      state: {
        e2ee_trust: {
          key_backup: { kind: "enabled", version: "browser-preview" }
        }
      }
    });

    const awaitingAuth = await api.resetIdentity();
    expect(awaitingAuth.state.e2ee_trust.identity_reset).toMatchObject({
      kind: "awaitingAuth",
      auth_type: "uiaa"
    });

    const flow =
      awaitingAuth.state.e2ee_trust.identity_reset.kind === "awaitingAuth"
        ? awaitingAuth.state.e2ee_trust.identity_reset.request_id
        : 0;
    const reset = await api.submitIdentityResetPassword(flow, "synthetic-password");
    expect(reset.state.e2ee_trust.identity_reset).toEqual({ kind: "idle" });
    expect(reset.state.e2ee_trust.cross_signing).toEqual({ kind: "missing" });
    expect(reset.state.e2ee_trust.key_backup).toEqual({ kind: "disabled" });
  });

  test("does not synthesize pin state for an unknown room", async () => {
    const api = createBrowserFakeApi();

    await api.pinEvent("!missing:browser.fake", "$event:browser.fake");
    const snapshot = await api.unpinEvent("!missing:browser.fake", "$event:browser.fake");

    expect(snapshot.state.room_interactions["!missing:browser.fake"]).toBeUndefined();
  });

  test("models public directory query and join pending substates", async () => {
    const api = createBrowserFakeApi();

    const queryPromise = api.queryDirectory({
      term: "public rooms",
      server_name: "fake.local",
      limit: 20,
      since: null
    });
    expect((await api.getSnapshot()).state.directory.query).toMatchObject({
      kind: "querying",
      query: {
        term: "public rooms",
        server_name: "fake.local",
        limit: 20,
        since: null
      }
    });

    const queried = await queryPromise;
    expect(queried.state.directory.query.kind).toBe("results");

    const joinPromise = api.joinDirectoryRoom("#public-demo:fake.local", "fake.local");
    expect((await api.getSnapshot()).state.directory.join).toMatchObject({
      kind: "joining",
      alias: "#public-demo:fake.local",
      via_server: "fake.local"
    });

    const joined = await joinPromise;
    expect(joined.state.directory.join).toEqual({ kind: "idle" });
  });

  test("models room management settings, moderation, and permission guard substates", async () => {
    const api = createBrowserFakeApi();

    const loaded = await api.loadRoomSettings("!browser-room:browser.fake");
    expect(loaded.state.room_management).toMatchObject({
      selected_room_id: "!browser-room:browser.fake",
      settings: {
        room_id: "!browser-room:browser.fake",
        permissions: {
          can_edit_settings: true,
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
    expect((await api.getSnapshot()).state.room_management.operation).toMatchObject({
      kind: "pending",
      operation: "settings"
    });
    const updated = await updatePromise;
    expect(updated.state.room_management.settings?.topic).toBe("Updated topic");
    expect(updated.state.room_management.operation).toEqual({ kind: "idle" });

    const moderated = await api.moderateRoomMember(
      "!browser-room:browser.fake",
      "@target:browser.fake",
      "kick",
      "Private reason"
    );
    expect(moderated.state.room_management.operation).toEqual({ kind: "idle" });

    await api.loadRoomSettings("!readonly-room:browser.fake");
    const guarded = await api.moderateRoomMember(
      "!readonly-room:browser.fake",
      "@target:browser.fake",
      "kick",
      null
    );
    expect(guarded.state.room_management.operation).toMatchObject({
      kind: "failed",
      operation: "moderation",
      failureKind: "forbidden"
    });
  });
});
