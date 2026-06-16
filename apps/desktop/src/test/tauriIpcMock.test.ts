import { describe, expect, test } from "vitest";

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
});
