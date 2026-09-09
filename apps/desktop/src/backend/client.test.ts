import { invoke } from "@tauri-apps/api/core";
import { afterEach, describe, expect, test, vi } from "vitest";

import { TauriDesktopApi } from "./client";
import { documentFromText } from "../domain/composerDocument";
import { parseComposerDraftRevision } from "../domain/composerDraftRevision";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async () => ({ ok: true }))
}));

describe("TauriDesktopApi", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.clearAllMocks();
  });

  test("observes reader avatars with stable IDs and lossless counters", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    const request = {
      installed_revision: "18446744073709551615", sequence: "18446744073709551614",
      visible_user_ids: ["@visible:example.invalid"], prefetch_user_ids: []
    };
    await new TauriDesktopApi().observeReceiptReaderAvatars("9007199254740993", request);
    expect(invoke).toHaveBeenCalledWith("observe_receipt_reader_avatars", {
      scope: "9007199254740993", request
    });
  });

  test("passes raw room address drafts to Rust and returns its preview unchanged", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    const preview = { localpart: "設計", full_alias: "#設計:example.invalid", error: null };
    vi.mocked(invoke).mockResolvedValueOnce(preview);
    const api = new TauriDesktopApi();
    expect(await api.previewRoomAddress("設計", null)).toEqual(preview);
    expect(invoke).toHaveBeenCalledWith("preview_room_address", { name: "設計", aliasLocalpart: null });
  });

  test("gets the diagnostic snapshot without private arguments", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = new TauriDesktopApi();
    await api.getDiagnosticSnapshot();

    expect(invoke).toHaveBeenCalledWith("get_diagnostic_snapshot");
  });

  test("uses distinct state-only and state-plus-timeline resync commands", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = new TauriDesktopApi();
    await api.settlementSnapshot();
    await api.resyncSnapshot();

    expect(invoke).toHaveBeenNthCalledWith(1, "settlement_snapshot");
    expect(invoke).toHaveBeenNthCalledWith(2, "resync_snapshot");
  });

  test("opens a thread with the Rust-owned creation intent", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = new TauriDesktopApi();
    await api.openThread(
      "!room:example.invalid",
      "$root:example.invalid",
      "newThreadDraft"
    );

    expect(invoke).toHaveBeenCalledWith("open_thread", {
      roomId: "!room:example.invalid",
      rootEventId: "$root:example.invalid",
      intent: "newThreadDraft"
    });
  });

  test("discovers login methods through typed Tauri command", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = new TauriDesktopApi();
    await api.discoverLoginMethods("https://example.test");

    expect(invoke).toHaveBeenCalledWith("discover_login_methods", {
      homeserver: "https://example.test"
    });
  });

  test("refreshes current-session status with the Rust-owned trigger", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = new TauriDesktopApi();
    await api.refreshCurrentSessionStatus("manual");

    expect(invoke).toHaveBeenCalledWith("refresh_current_session_status", {
      trigger: "manual"
    });
  });

  test("passes OIDC login flow commands to Rust", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = new TauriDesktopApi();
    await api.startOidcLogin("https://example.test");
    await api.completeOidcLogin(
      "https://example.test",
      "koushi-desktop://auth/callback?code=synthetic"
    );

    expect(invoke).toHaveBeenCalledWith("start_oidc_login", {
      homeserver: "https://example.test"
    });
    expect(invoke).toHaveBeenCalledWith("complete_oidc_login", {
      homeserver: "https://example.test",
      callbackUrl: "koushi-desktop://auth/callback?code=synthetic"
    });
  });

  test("passes soft logout reauth to the Rust session command", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = new TauriDesktopApi();
    await api.submitSoftLogoutReauth("synthetic-password");

    expect(invoke).toHaveBeenCalledWith("submit_soft_logout_reauth", {
      password: "synthetic-password"
    });
  });

  test("passes explicit device cleanup stages to Rust without exposing remote policy", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = new TauriDesktopApi();
    await api.startDeviceCleanup();
    await api.submitDeviceCleanupUia(370, "synthetic-password");
    await api.eraseLocalDataAnyway();

    expect(invoke).toHaveBeenCalledWith("start_device_cleanup");
    expect(invoke).toHaveBeenCalledWith("submit_device_cleanup_uia", {
      flowId: 370,
      password: "synthetic-password"
    });
    expect(invoke).toHaveBeenCalledWith("erase_local_data_anyway");
  });

  test("passes settings patches to the Rust update_settings command", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = new TauriDesktopApi();
    await api.updateSettings({ appearance: { theme: "dark", density: "comfortable" } });

    expect(invoke).toHaveBeenCalledWith("update_settings", {
      patch: { appearance: { theme: "dark", density: "comfortable" } }
    });
  });

  test("passes room URL-preview overrides to the dedicated Rust command", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = new TauriDesktopApi();
    await api.setRoomUrlPreviewOverride("!room:example.invalid", false);

    expect(invoke).toHaveBeenCalledWith("set_room_url_preview_override", {
      roomId: "!room:example.invalid",
      enabled: false
    });
  });

  test("passes composer resolver facts to the Rust resolver command", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = new TauriDesktopApi();
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

  test("passes the renderer generation and composer draft lease to Rust", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    const api = new TauriDesktopApi();
    const account = {
      homeserver: "https://example.invalid",
      userId: "@user:example.invalid",
      deviceId: "DEVICE"
    };
    const scope = {
      account: {
        homeserver: account.homeserver,
        user_id: account.userId,
        device_id: account.deviceId
      },
      target: { kind: "main" as const, room_id: "!room:example.invalid" }
    };

    await api.beginComposerDraftRendererGeneration();
    await api.acquireComposerDraftLease(scope, "renderer-7");
    await api.setComposerDraft(
      account,
      "lease-9",
      "renderer-7",
      "!room:example.invalid",
      documentFromText("body"),
      parseComposerDraftRevision("9007199254740993")
    );
    await api.releaseComposerDraftLease("lease-9", "renderer-7");

    expect(invoke).toHaveBeenCalledWith("begin_composer_draft_renderer_generation");
    expect(invoke).toHaveBeenCalledWith("acquire_composer_draft_lease", {
      accountHomeserver: account.homeserver,
      accountUserId: account.userId,
      accountDeviceId: account.deviceId,
      target: scope.target,
      rendererGeneration: "renderer-7"
    });
    expect(invoke).toHaveBeenCalledWith("set_composer_draft", {
      accountHomeserver: account.homeserver,
      accountUserId: account.userId,
      accountDeviceId: account.deviceId,
      leaseId: "lease-9",
      rendererGeneration: "renderer-7",
      roomId: "!room:example.invalid",
      document: documentFromText("body"),
      draftRevision: "9007199254740993"
    });
    expect(invoke).toHaveBeenCalledWith("release_composer_draft_lease", {
      leaseId: "lease-9",
      rendererGeneration: "renderer-7"
    });
  });

  test("passes a structured staged caption document through the Tauri command", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    const api = new TauriDesktopApi();
    const document = documentFromText("**caption**");

    await api.updateStagedUploadCaption(
      { kind: "main", room_id: "!room:example.invalid" },
      "staged-1",
      document
    );

    expect(invoke).toHaveBeenCalledWith("update_staged_upload_caption", {
      target: { kind: "main", room_id: "!room:example.invalid" },
      stagedId: "staged-1",
      document
    });
  });

  test("passes structured mention identity to send and edit commands without parallel text metadata", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    const api = new TauriDesktopApi();
    const account = {
      homeserver: "https://example.invalid",
      userId: "@user:example.invalid",
      deviceId: "DEVICE"
    };
    const document = {
      version: 2 as const,
      inlines: [
        {
          kind: "mention" as const,
          target: {
            kind: "user" as const,
            user_id: "@alice:example.invalid",
            display_label: "Same Name"
          },
          display_label: "Same Name"
        }
      ]
    };

    await api.sendText(
      account,
      "lease",
      "renderer",
      "submission",
      "!room:example.invalid",
      document,
      parseComposerDraftRevision("1")
    );
    await api.editMessage("!room:example.invalid", "$event", document);

    expect(invoke).toHaveBeenCalledWith(
      "send_text",
      expect.objectContaining({ document })
    );
    expect(invoke).toHaveBeenCalledWith("edit_message", {
      roomId: "!room:example.invalid",
      eventId: "$event",
      document
    });
    expect(
      vi.mocked(invoke).mock.calls.flatMap(([, args]) => Object.keys(args ?? {}))
    ).not.toContain("mentions");
  });

  test("passes E2EE trust actions to Rust-owned commands", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = new TauriDesktopApi();
    await api.bootstrapCrossSigning();
    await api.enableKeyBackup();
    await api.acceptVerification(41);
    await api.confirmSasVerification(42);
    await api.cancelVerification(43);
    await api.resetIdentity();
    await api.cancelIdentityReset(44);
    await api.submitIdentityResetPassword(44, "synthetic-password");
    await api.submitIdentityResetOAuth(45);

    expect(invoke).toHaveBeenCalledWith("bootstrap_cross_signing");
    expect(invoke).toHaveBeenCalledWith("enable_key_backup");
    expect(invoke).toHaveBeenCalledWith("accept_verification", { flowId: 41 });
    expect(invoke).toHaveBeenCalledWith("confirm_sas_verification", { flowId: 42 });
    expect(invoke).toHaveBeenCalledWith("cancel_verification", { flowId: 43 });
    expect(invoke).toHaveBeenCalledWith("reset_identity");
    expect(invoke).toHaveBeenCalledWith("cancel_identity_reset", { flowId: 44 });
    expect(invoke).toHaveBeenCalledWith("submit_identity_reset_password", {
      flowId: 44,
      password: "synthetic-password"
    });
    expect(invoke).toHaveBeenCalledWith("submit_identity_reset_oauth", { flowId: 45 });
  });

  test("passes E2EE key-management actions to Rust-owned commands", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = new TauriDesktopApi();
    await api.exportRoomKeys("/tmp/export.txt", "room-key-passphrase");
    await api.importRoomKeys("/tmp/import.txt", "room-key-passphrase");
    await api.bootstrapSecureBackup(
      "secure-backup-passphrase",
      "/tmp/recovery.txt",
      { kind: "initialSetup" }
    );
    await api.changeSecureBackupPassphrase(
      "old-secure-backup-passphrase",
      "new-secure-backup-passphrase",
      "/tmp/recovery.txt"
    );

    expect(invoke).toHaveBeenCalledWith("export_room_keys", {
      destinationPath: "/tmp/export.txt",
      passphrase: "room-key-passphrase"
    });
    expect(invoke).toHaveBeenCalledWith("import_room_keys", {
      sourcePath: "/tmp/import.txt",
      passphrase: "room-key-passphrase"
    });
    expect(invoke).toHaveBeenCalledWith("bootstrap_secure_backup", {
      passphrase: "secure-backup-passphrase",
      recoveryKeyDestinationPath: "/tmp/recovery.txt",
      intent: { kind: "initialSetup" }
    });
    expect(invoke).toHaveBeenCalledWith("change_secure_backup_passphrase", {
      oldSecret: "old-secure-backup-passphrase",
      newPassphrase: "new-secure-backup-passphrase",
      recoveryKeyDestinationPath: "/tmp/recovery.txt"
    });
  });

  test("passes secure backup gate actions to their dedicated Rust commands", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = new TauriDesktopApi();
    await api.recoverSecureBackup("secure-backup-recovery-key");
    await api.bootstrapSecureBackup(
      "secure-backup-passphrase",
      "/tmp/recovery.txt",
      { kind: "initialSetup" }
    );
    await api.bootstrapSecureBackup(
      "reenable-passphrase",
      "/tmp/reenable-recovery.txt",
      { kind: "reenable", confirmed: true }
    );
    await api.retrySecureBackupInspection();

    expect(invoke).toHaveBeenCalledWith("recover_secure_backup", {
      secret: "secure-backup-recovery-key"
    });
    expect(invoke).toHaveBeenCalledWith("bootstrap_secure_backup", {
      passphrase: "secure-backup-passphrase",
      recoveryKeyDestinationPath: "/tmp/recovery.txt",
      intent: { kind: "initialSetup" }
    });
    expect(invoke).toHaveBeenCalledWith("bootstrap_secure_backup", {
      passphrase: "reenable-passphrase",
      recoveryKeyDestinationPath: "/tmp/reenable-recovery.txt",
      intent: { kind: "reenable", confirmed: true }
    });
    expect(invoke).toHaveBeenCalledWith("retry_secure_backup_inspection");
  });

  test("passes reaction actions to Rust-owned timeline commands", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = new TauriDesktopApi();
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

    const api = new TauriDesktopApi();
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

    const api = new TauriDesktopApi();
    await api.setDisplayName("Alice");
    await api.setAvatar("image/png", [1, 2, 3, 4]);
    await api.setLocalUserAlias("@target:example.invalid", "Desk Alias");
    await api.setLocalUserAlias("@target:example.invalid", null);

    expect(invoke).toHaveBeenCalledWith("set_display_name", { displayName: "Alice" });
    expect(invoke).toHaveBeenCalledWith("set_avatar", {
      mimeType: "image/png",
      bytes: [1, 2, 3, 4]
    });
    expect(invoke).toHaveBeenCalledWith("set_local_user_alias", {
      userId: "@target:example.invalid",
      alias: "Desk Alias"
    });
    expect(invoke).toHaveBeenCalledWith("set_local_user_alias", {
      userId: "@target:example.invalid",
      alias: null
    });
  });

  test("passes invite and DM actions to Rust-owned room commands", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = new TauriDesktopApi();
    await api.acceptInvite("!invite:example.invalid");
    await api.declineInvite("!decline:example.invalid");
    await api.joinRoom("!child:example.invalid");
    await api.startDirectMessage("@target:example.invalid");
    await api.inviteUser("!room:example.invalid", "@target:example.invalid");

    expect(invoke).toHaveBeenCalledWith("accept_invite", {
      roomId: "!invite:example.invalid"
    });
    expect(invoke).toHaveBeenCalledWith("decline_invite", {
      roomId: "!decline:example.invalid"
    });
    expect(invoke).toHaveBeenCalledWith("join_room", {
      roomId: "!child:example.invalid"
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

    const api = new TauriDesktopApi();
    await api.queryDirectory({
      term: "public rooms",
      server_name: "example.invalid",
      limit: 20,
      since: "page-2"
    });
    await api.joinDirectoryRoom("#public:example.invalid", ["example.invalid"]);

    expect(invoke).toHaveBeenCalledWith("query_directory", {
      term: "public rooms",
      serverName: "example.invalid",
      limit: 20,
      since: "page-2"
    });
    expect(invoke).toHaveBeenCalledWith("join_directory_room", {
      roomIdOrAlias: "#public:example.invalid",
      viaServers: ["example.invalid"]
    });
  });

  test("passes room management actions to Rust-owned room commands", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = new TauriDesktopApi();
    await api.loadRoomSettings("!room:example.invalid");
    await api.queryMentionCandidates("!room:example.invalid", "thread", "ali");
    await api.repairRoomTimeline("!room:example.invalid");
    await api.forceRotateOutboundSession("!room:example.invalid");
    await api.updateRoomSetting("!room:example.invalid", {
      topic: "Private topic"
    });
    await api.moderateRoomMember(
      "!room:example.invalid",
      "@target:example.invalid",
      "kick",
      "Private reason"
    );
    await api.updateRoomMemberRole("!room:example.invalid", "@target:example.invalid", 50);

    expect(invoke).toHaveBeenCalledWith("load_room_settings", {
      roomId: "!room:example.invalid"
    });
    expect(invoke).toHaveBeenCalledWith("query_mention_candidates", {
      roomId: "!room:example.invalid",
      surface: "thread",
      query: "ali"
    });
    expect(invoke).toHaveBeenCalledWith("repair_room_timeline", {
      roomId: "!room:example.invalid"
    });
    expect(invoke).toHaveBeenCalledWith("force_rotate_outbound_session", {
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
    expect(invoke).toHaveBeenCalledWith("update_room_member_role", {
      roomId: "!room:example.invalid",
      targetUserId: "@target:example.invalid",
      powerLevel: 50
    });
  });

  test("passes Space member audit actions with the generation fence", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = new TauriDesktopApi();
    await api.loadSpaceMembers("!space:example.invalid", 4);
    await api.inviteUserToSpace(
      "!space:example.invalid",
      "@target:example.invalid",
      4
    );

    expect(invoke).toHaveBeenCalledWith("load_space_members", {
      spaceId: "!space:example.invalid",
      generation: 4
    });
    expect(invoke).toHaveBeenCalledWith("invite_user_to_space", {
      spaceId: "!space:example.invalid",
      userId: "@target:example.invalid",
      generation: 4
    });

    await api.cancelSpaceInvite("!space:example.invalid", "@target:example.invalid", 4);

    expect(invoke).toHaveBeenCalledWith("cancel_space_invite", {
      spaceId: "!space:example.invalid",
      userId: "@target:example.invalid",
      generation: 4
    });

    await api.updateSpaceMemberRole(
      "!space:example.invalid",
      "@target:example.invalid",
      4,
      "revision-1",
      0,
      50,
      false
    );
    expect(invoke).toHaveBeenCalledWith("update_space_member_role", {
      spaceId: "!space:example.invalid",
      userId: "@target:example.invalid",
      generation: 4,
      expectedPowerLevelsRevision: "revision-1",
      expectedPowerLevel: 0,
      powerLevel: 50,
      confirmed: false
    });
  });

  test("reads receipt resources through the scoped installed revision", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = new TauriDesktopApi();
    await api.readReceiptReaderResource(
      "scope-7",
      "revision-9",
      "avatar/0000000000000001"
    );

    expect(invoke).toHaveBeenCalledWith("read_receipt_reader_resource", {
      scope: "scope-7",
      revision: "revision-9",
      sourceRef: "avatar/0000000000000001"
    });
  });

  test("passes activity actions to Rust-owned activity commands", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = new TauriDesktopApi();
    await api.openActivity();
    await api.setActivityTab("unread");
    await api.paginateActivity("recent", "recent-page-2");
    await api.retryActivityResolution();
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
    expect(invoke).toHaveBeenCalledWith("retry_activity_resolution");
    expect(invoke).toHaveBeenCalledWith("mark_activity_read", {
      target: {
        kind: "room",
        room_id: "!room:example.invalid",
        up_to_event_id: "$event:example.invalid"
      }
    });
    expect(invoke).toHaveBeenCalledWith("close_activity");
  });

  test("passes credential health probe to Rust-owned account command", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = new TauriDesktopApi();
    await api.probeLocalEncryptionHealth();

    expect(invoke).toHaveBeenCalledWith("probe_local_encryption_health");
  });
});
