/* @vitest-environment jsdom */

import { afterEach, describe, expect, test, vi } from "vitest";

function port() {
  return {
    openHttpUrl: vi.fn(async () => undefined),
    mediaSourceUrl: vi.fn((sourceUrl: string) => `converted:${sourceUrl}`),
    saveMediaFile: vi.fn(async () => undefined)
  };
}

async function loadRuntime(tauriRuntime: boolean) {
  vi.resetModules();
  const tauriLinkMediaPort = port();
  const browserLinkMediaPort = port();
  vi.doMock("./runtimeEnvironment", () => ({ isTauriRuntime: () => tauriRuntime }));
  vi.doMock("./tauri/linkMediaPort", () => ({ tauriLinkMediaPort }));
  vi.doMock("./browser/linkMediaPort", () => ({ browserLinkMediaPort }));
  const runtime = await import("./linkMediaRuntime");
  return { runtime, tauriLinkMediaPort, browserLinkMediaPort };
}

afterEach(() => {
  vi.clearAllMocks();
});

describe("link/media platform selection", () => {
  test("routes valid operations only to the Tauri port", async () => {
    const { runtime, tauriLinkMediaPort, browserLinkMediaPort } = await loadRuntime(true);

    await runtime.openExternalHttpUrl("https://example.com/path");
    expect(runtime.mediaSourceUrl("/tmp/media.png")).toBe("converted:/tmp/media.png");
    await runtime.saveReadyMediaFile("asset://media", "media.png");

    expect(tauriLinkMediaPort.openHttpUrl).toHaveBeenCalledWith("https://example.com/path");
    expect(tauriLinkMediaPort.mediaSourceUrl).toHaveBeenCalledWith("/tmp/media.png");
    expect(tauriLinkMediaPort.saveMediaFile).toHaveBeenCalledWith(
      "asset://media",
      "media.png"
    );
    expect(browserLinkMediaPort.openHttpUrl).not.toHaveBeenCalled();
    expect(browserLinkMediaPort.mediaSourceUrl).not.toHaveBeenCalled();
    expect(browserLinkMediaPort.saveMediaFile).not.toHaveBeenCalled();
  });

  test("routes browser operations and rejects invalid external links before the port", async () => {
    const { runtime, tauriLinkMediaPort, browserLinkMediaPort } = await loadRuntime(false);

    await runtime.openExternalHttpUrl("javascript:alert(1)");
    await runtime.openExternalHttpUrl("https://example.com/path");
    expect(runtime.mediaSourceUrl("/tmp/media.png")).toBe("converted:/tmp/media.png");
    await runtime.saveReadyMediaFile("/tmp/media.png", "media.png");

    expect(browserLinkMediaPort.openHttpUrl).toHaveBeenCalledTimes(1);
    expect(browserLinkMediaPort.openHttpUrl).toHaveBeenCalledWith("https://example.com/path");
    expect(browserLinkMediaPort.mediaSourceUrl).toHaveBeenCalledWith("/tmp/media.png");
    expect(browserLinkMediaPort.saveMediaFile).toHaveBeenCalledWith(
      "/tmp/media.png",
      "media.png"
    );
    expect(tauriLinkMediaPort.openHttpUrl).not.toHaveBeenCalled();
    expect(tauriLinkMediaPort.mediaSourceUrl).not.toHaveBeenCalled();
    expect(tauriLinkMediaPort.saveMediaFile).not.toHaveBeenCalled();
  });
});
