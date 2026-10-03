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
  test("constructs and selects exactly one Tauri port", async () => {
    const { runtime, port, createTauriDesktopAttentionPort } = await loadRuntime(true);

    expect(runtime.desktopAttentionPort).toBe(port);
    expect(createTauriDesktopAttentionPort).toHaveBeenCalledOnce();
  });

  test("keeps browser attention native operations absent", async () => {
    const { runtime, createTauriDesktopAttentionPort } = await loadRuntime(false);

    expect(runtime.desktopAttentionPort).toBeNull();
    expect(createTauriDesktopAttentionPort).not.toHaveBeenCalled();
  });
});
