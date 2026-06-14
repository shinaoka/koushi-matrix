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
});
