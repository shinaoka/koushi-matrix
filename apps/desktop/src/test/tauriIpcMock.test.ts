import { describe, expect, test } from "vitest";

import type { DesktopSnapshot } from "../domain/types";
import { TauriIpcMock } from "./tauriIpcMock";

describe("TauriIpcMock command responses", () => {
  test("supports static and functional command responses", async () => {
    const mock = new TauriIpcMock();
    let current: { kind: "ready" | "reply" } = { kind: "ready" };

    mock.setCommandResponse("get_snapshot", () => current);
    mock.setCommandResponse("set_composer_reply_target", ({ roomId }: { roomId: string }) => {
      current = { kind: roomId === "!room:test" ? "reply" : "ready" };
      return current;
    });

    mock.setCommandResponse("static_command", { ok: true });

    await expect(mock.invoke("get_snapshot")).resolves.toEqual({ kind: "ready" });
    await expect(
      mock.invoke("set_composer_reply_target", { roomId: "!room:test" })
    ).resolves.toEqual({ kind: "reply" });
    await expect(mock.invoke("get_snapshot")).resolves.toEqual({ kind: "reply" });
    await expect(mock.invoke("static_command")).resolves.toEqual({ ok: true });
  });

  test("redacts key-management secrets and paths from recorded invocations", async () => {
    const mock = new TauriIpcMock();

    await mock.invoke("export_room_keys", {
      destinationPath: "/tmp/private-export.txt",
      passphrase: "private-room-key-passphrase"
    });
    await mock.invoke("change_secure_backup_passphrase", {
      oldSecret: "private-old-secret",
      newPassphrase: "private-new-passphrase",
      recoveryKeyDestinationPath: "/tmp/private-recovery.txt"
    });

    const recorded = JSON.stringify(mock.recordedInvocations());
    expect(recorded).not.toContain("private-export");
    expect(recorded).not.toContain("private-room-key-passphrase");
    expect(recorded).not.toContain("private-old-secret");
    expect(recorded).not.toContain("private-new-passphrase");
    expect(recorded).not.toContain("private-recovery");
    expect(recorded).toContain("[REDACTED]");
  });

  test("default get_snapshot contains the complete nested app-state contract", async () => {
    const mock = new TauriIpcMock();
    const snapshot = await mock.invoke<DesktopSnapshot>("get_snapshot");
    const domain = snapshot.state.domain as unknown as Record<string, unknown>;
    const ui = snapshot.state.ui as unknown as Record<string, unknown>;
    const requiredDomainKeys = [
      "session",
      "session_lock_reason",
      "secure_backup_gate",
      "current_session_status",
      "device_cleanup",
      "auth",
      "account_management_url",
      "account_management",
      "account_management_capabilities",
      "soft_logout_reauth",
      "qr_login",
      "settings",
      "link_preview_settings",
      "room_preferences",
      "locale_profile",
      "typography_profile",
      "profile",
      "space_members",
      "sync",
      "spaces",
      "rooms",
      "invites",
      "invite_workflow",
      "room_notification_settings",
      "room_interactions",
      "directory",
      "room_management",
      "mention_candidates",
      "activity",
      "thread_attention",
      "search",
      "search_crawler",
      "live_signals",
      "e2ee_trust",
      "local_encryption",
      "native_attention",
      "cjk_text_policy"
    ] as const;

    for (const key of requiredDomainKeys) {
      expect(domain).toHaveProperty(key);
    }

    for (const key of [
      "navigation",
      "room_list",
      "timeline",
      "thread",
      "focused_context",
      "files_view",
      "threads_list",
      "basic_operation",
      "errors"
    ]) {
      expect(ui).toHaveProperty(key);
      expect(domain).not.toHaveProperty(key);
    }

    expect(domain.secure_backup_gate).toEqual({ kind: "inactive" });
    expect(domain.current_session_status).toEqual({ status: "idle" });
    expect(domain.device_cleanup).toEqual({ kind: "idle" });
    expect(domain.space_members).toEqual({
      selected_space_id: null,
      generation: 0,
      power_levels_revision: null,
      can_edit_roles: false,
      space_joined: [],
      space_invited: [],
      child_room_only: [],
      child_room_count: 0,
      complete_child_room_count: 0,
      incomplete_child_room_count: 0,
      operation: { kind: "idle" }
    });
    expect(domain.mention_candidates).toEqual({ targets: [] });
    expect(domain.search_crawler).toEqual({ rooms: {}, last_active: null });
    expect(domain.settings).toMatchObject({
      values: {
        search_crawler: {
          speed: "standard",
          include_media_captions: true,
          include_filenames: true
        },
        thread_list_order: { kind: "latestReply" },
        room_list_sort: { kind: "activity" }
      }
    });
  });
});
