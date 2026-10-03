/* @vitest-environment jsdom */

import { afterEach, describe, expect, test, vi } from "vitest";

async function loadRuntime(tauriRuntime: boolean) {
  vi.resetModules();
  const port = { kind: "tauri-attention-port" };
  const createTauriDesktopAttentionPort = vi.fn(() => port);
  vi.doMock("./runtimeEnvironment", () => ({ isTauriRuntime: () => tauriRuntime }));
  vi.doMock("./tauri/desktopAttentionPort", () => ({ createTauriDesktopAttentionPort }));

  const runtime = await import("./desktopAttentionRuntime");
  return { runtime, port, createTauriDesktopAttentionPort };
}

afterEach(() => {
  vi.clearAllMocks();
});

describe("desktop attention platform selection", () => {
  test("creates a Tauri attention port for the owning account tab", async () => {
    const { runtime, port, createTauriDesktopAttentionPort } = await loadRuntime(true);

    expect(runtime.desktopAttentionPortForAccount("account:alice")).toBe(port);
    expect(createTauriDesktopAttentionPort).toHaveBeenCalledOnce();
    expect(createTauriDesktopAttentionPort).toHaveBeenCalledWith("account:alice");
  });

  test("keeps browser attention native operations absent", async () => {
    const { runtime, createTauriDesktopAttentionPort } = await loadRuntime(false);

    expect(runtime.desktopAttentionPortForAccount("account:alice")).toBeNull();
    expect(createTauriDesktopAttentionPort).not.toHaveBeenCalled();
  });
});
