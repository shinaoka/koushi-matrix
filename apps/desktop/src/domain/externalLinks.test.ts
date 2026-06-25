import { openUrl } from "@tauri-apps/plugin-opener";
import { afterEach, describe, expect, it, vi } from "vitest";

import { openExternalHttpUrl, toExternalHttpUrl } from "./externalLinks";

vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: vi.fn(async () => undefined)
}));

afterEach(() => {
  vi.clearAllMocks();
});

describe("externalLinks", () => {
  it("normalizes and opens http URLs through the Tauri opener", async () => {
    await openExternalHttpUrl("https://example.com/page");

    expect(openUrl).toHaveBeenCalledWith("https://example.com/page");
  });

  it("does not open non-http URLs", async () => {
    expect(toExternalHttpUrl("javascript:alert(1)")).toBeNull();
    expect(toExternalHttpUrl("file:///tmp/secret.txt")).toBeNull();

    await openExternalHttpUrl("javascript:alert(1)");

    expect(openUrl).not.toHaveBeenCalled();
  });
});
