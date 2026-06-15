import { invoke } from "@tauri-apps/api/core";
import { afterEach, describe, expect, test, vi } from "vitest";

import { createDesktopApi } from "./client";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async () => ({ ok: true }))
}));

describe("TauriDesktopApi", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.clearAllMocks();
  });

  test("passes settings patches to the Rust update_settings command", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = createDesktopApi();
    await api.updateSettings({ appearance: { theme: "dark" } });

    expect(invoke).toHaveBeenCalledWith("update_settings", {
      patch: { appearance: { theme: "dark" } }
    });
  });

  test("passes composer resolver facts to the Rust resolver command", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = createDesktopApi();
    await api.resolveComposerKeyAction(
      "main",
      {
        key: "enter",
        modifiers: { ctrl: false, meta: true, shift: false, alt: false },
        is_composing: false,
        selection: { start: 1, end: 3 }
      },
      { autocomplete_open: false, send_enabled: true }
    );

    expect(invoke).toHaveBeenCalledWith("resolve_composer_key_action", {
      surface: "main",
      keyEvent: {
        key: "enter",
        modifiers: { ctrl: false, meta: true, shift: false, alt: false },
        is_composing: false,
        selection: { start: 1, end: 3 }
      },
      autocompleteOpen: false,
      sendEnabled: true
    });
  });

  test("passes E2EE trust actions to Rust-owned commands", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = createDesktopApi();
    await api.bootstrapCrossSigning();
    await api.enableKeyBackup();
    await api.acceptVerification(41);
    await api.confirmSasVerification(42);
    await api.cancelVerification(43);
    await api.resetIdentity();
    await api.submitIdentityResetPassword(44, "synthetic-password");
    await api.submitIdentityResetOAuth(45);

    expect(invoke).toHaveBeenCalledWith("bootstrap_cross_signing");
    expect(invoke).toHaveBeenCalledWith("enable_key_backup");
    expect(invoke).toHaveBeenCalledWith("accept_verification", { flowId: 41 });
    expect(invoke).toHaveBeenCalledWith("confirm_sas_verification", { flowId: 42 });
    expect(invoke).toHaveBeenCalledWith("cancel_verification", { flowId: 43 });
    expect(invoke).toHaveBeenCalledWith("reset_identity");
    expect(invoke).toHaveBeenCalledWith("submit_identity_reset_password", {
      flowId: 44,
      password: "synthetic-password"
    });
    expect(invoke).toHaveBeenCalledWith("submit_identity_reset_oauth", { flowId: 45 });
  });

  test("passes reaction actions to Rust-owned timeline commands", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = createDesktopApi();
    await api.sendReaction("!room:example.invalid", "$event:example.invalid", "👍");
    await api.redactReaction(
      "!room:example.invalid",
      "$event:example.invalid",
      "👍",
      "$reaction:example.invalid"
    );

    expect(invoke).toHaveBeenCalledWith("send_reaction", {
      roomId: "!room:example.invalid",
      eventId: "$event:example.invalid",
      reactionKey: "👍"
    });
    expect(invoke).toHaveBeenCalledWith("redact_reaction", {
      roomId: "!room:example.invalid",
      eventId: "$event:example.invalid",
      reactionKey: "👍",
      reactionEventId: "$reaction:example.invalid"
    });
  });

  test("passes send queue actions to Rust-owned timeline commands", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = createDesktopApi();
    await api.retrySend("!room:example.invalid", "txn-retry");
    await api.cancelSend("!room:example.invalid", "txn-cancel");

    expect(invoke).toHaveBeenCalledWith("retry_send", {
      roomId: "!room:example.invalid",
      transactionId: "txn-retry"
    });
    expect(invoke).toHaveBeenCalledWith("cancel_send", {
      roomId: "!room:example.invalid",
      transactionId: "txn-cancel"
    });
  });

  test("passes profile actions to Rust-owned account commands", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = createDesktopApi();
    await api.setDisplayName("Alice");
    await api.setAvatar("image/png", [1, 2, 3, 4]);

    expect(invoke).toHaveBeenCalledWith("set_display_name", { displayName: "Alice" });
    expect(invoke).toHaveBeenCalledWith("set_avatar", {
      mimeType: "image/png",
      bytes: [1, 2, 3, 4]
    });
  });

  test("passes invite and DM actions to Rust-owned room commands", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = createDesktopApi();
    await api.acceptInvite("!invite:example.invalid");
    await api.declineInvite("!decline:example.invalid");
    await api.startDirectMessage("@target:example.invalid");
    await api.inviteUser("!room:example.invalid", "@target:example.invalid");

    expect(invoke).toHaveBeenCalledWith("accept_invite", {
      roomId: "!invite:example.invalid"
    });
    expect(invoke).toHaveBeenCalledWith("decline_invite", {
      roomId: "!decline:example.invalid"
    });
    expect(invoke).toHaveBeenCalledWith("start_direct_message", {
      userId: "@target:example.invalid"
    });
    expect(invoke).toHaveBeenCalledWith("invite_user", {
      roomId: "!room:example.invalid",
      userId: "@target:example.invalid"
    });
  });

  test("passes public directory actions to Rust-owned room commands", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = createDesktopApi();
    await api.queryDirectory({
      term: "public rooms",
      server_name: "example.invalid",
      limit: 20,
      since: "page-2"
    });
    await api.joinDirectoryRoom("#public:example.invalid", "example.invalid");

    expect(invoke).toHaveBeenCalledWith("query_directory", {
      term: "public rooms",
      serverName: "example.invalid",
      limit: 20,
      since: "page-2"
    });
    expect(invoke).toHaveBeenCalledWith("join_directory_room", {
      alias: "#public:example.invalid",
      viaServer: "example.invalid"
    });
  });

  test("passes room management actions to Rust-owned room commands", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = createDesktopApi();
    await api.loadRoomSettings("!room:example.invalid");
    await api.updateRoomSetting("!room:example.invalid", {
      topic: "Private topic"
    });
    await api.moderateRoomMember(
      "!room:example.invalid",
      "@target:example.invalid",
      "kick",
      "Private reason"
    );

    expect(invoke).toHaveBeenCalledWith("load_room_settings", {
      roomId: "!room:example.invalid"
    });
    expect(invoke).toHaveBeenCalledWith("update_room_setting", {
      roomId: "!room:example.invalid",
      change: { topic: "Private topic" }
    });
    expect(invoke).toHaveBeenCalledWith("moderate_room_member", {
      roomId: "!room:example.invalid",
      targetUserId: "@target:example.invalid",
      action: "kick",
      reason: "Private reason"
    });
  });

  test("passes activity actions to Rust-owned activity commands", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = createDesktopApi();
    await api.openActivity();
    await api.setActivityTab("unread");
    await api.paginateActivity("recent", "recent-page-2");
    await api.markActivityRead({
      kind: "room",
      room_id: "!room:example.invalid",
      up_to_event_id: "$event:example.invalid"
    });
    await api.closeActivity();

    expect(invoke).toHaveBeenCalledWith("open_activity");
    expect(invoke).toHaveBeenCalledWith("set_activity_tab", { tab: "unread" });
    expect(invoke).toHaveBeenCalledWith("paginate_activity", {
      tab: "recent",
      cursor: "recent-page-2"
    });
    expect(invoke).toHaveBeenCalledWith("mark_activity_read", {
      target: {
        kind: "room",
        room_id: "!room:example.invalid",
        up_to_event_id: "$event:example.invalid"
      }
    });
    expect(invoke).toHaveBeenCalledWith("close_activity");
  });
});
