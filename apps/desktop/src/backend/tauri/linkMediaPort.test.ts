/* @vitest-environment jsdom */

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { afterEach, describe, expect, it, vi } from "vitest";

import { setRendererSelectedAccountTabId } from "../client";
import { tauriLinkMediaPort } from "./linkMediaPort";

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: vi.fn((path: string) => `asset://${path}`),
  invoke: vi.fn(async (command: string) =>
    command === "default_media_save_path" ? "/downloads/default.png" : undefined
  )
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: vi.fn(async () => "/downloads/chosen.png")
}));
vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: vi.fn(async () => undefined)
}));

afterEach(() => {
  setRendererSelectedAccountTabId(null);
  vi.clearAllMocks();
});

describe("Tauri link/media port", () => {
  it("opens through Tauri and falls back to an isolated browser window", async () => {
    await tauriLinkMediaPort.openHttpUrl("https://example.com/");
    expect(openUrl).toHaveBeenCalledWith("https://example.com/");

    const windowOpen = vi.spyOn(window, "open").mockImplementation(() => null);
    vi.mocked(openUrl).mockRejectedValueOnce(new Error("unavailable"));
    await tauriLinkMediaPort.openHttpUrl("https://fallback.example/");
    expect(windowOpen).toHaveBeenCalledWith(
      "https://fallback.example/",
      "_blank",
      "noopener,noreferrer"
    );
  });

  it("passes web URLs through and converts local media paths", () => {
    expect(tauriLinkMediaPort.mediaSourceUrl("https://example.invalid/image.png")).toBe(
      "https://example.invalid/image.png"
    );
    expect(tauriLinkMediaPort.mediaSourceUrl("asset://localhost/avatar.png")).toBe(
      "asset://localhost/avatar.png"
    );
    expect(tauriLinkMediaPort.mediaSourceUrl("file:///tmp/avatar%20image.png")).toBe(
      "asset:///tmp/avatar image.png"
    );
    expect(convertFileSrc).toHaveBeenCalledWith("/tmp/avatar image.png");
    expect(tauriLinkMediaPort.mediaSourceUrl("/tmp/media-downloads/report.pdf")).toBe(
      "asset:///tmp/media-downloads/report.pdf"
    );
  });

  it("mints the desktop thumbnail URI only for a validated opaque Core reference", () => {
    expect(tauriLinkMediaPort.renderableThumbnailSourceUrl("avatar/0123456789abcdef")).toBe(
      "koushi-thumbnail://localhost/avatar/0123456789abcdef"
    );
    expect(
      tauriLinkMediaPort.renderableThumbnailSourceUrl("link-preview/fedcba9876543210")
    ).toBe("koushi-thumbnail://localhost/link-preview/fedcba9876543210");
    expect(tauriLinkMediaPort.renderableThumbnailSourceUrl("data:image/gif;base64,R0lGODlh")).toBe(
      "data:image/gif;base64,R0lGODlh"
    );
    expect(tauriLinkMediaPort.renderableThumbnailSourceUrl("../private.bin")).toBeNull();
    expect(
      tauriLinkMediaPort.renderableThumbnailSourceUrl(
        "koushi-thumbnail://localhost/avatar/already-minted"
      )
    ).toBeNull();
  });

  it("mints the Windows localhost thumbnail URI on Windows", () => {
    const platform = Object.getOwnPropertyDescriptor(window.navigator, "platform");
    Object.defineProperty(window.navigator, "platform", {
      value: "Win32",
      configurable: true
    });
    try {
      expect(tauriLinkMediaPort.renderableThumbnailSourceUrl("avatar/0123456789abcdef")).toBe(
        "http://koushi-thumbnail.localhost/avatar/0123456789abcdef"
      );
      expect(
        tauriLinkMediaPort.renderableThumbnailSourceUrl("link-preview/fedcba9876543210")
      ).toBe("http://koushi-thumbnail.localhost/link-preview/fedcba9876543210");
    } finally {
      if (platform) {
        Object.defineProperty(window.navigator, "platform", platform);
      }
    }
  });

  it("preserves the default-path, dialog and save command contract", async () => {
    await tauriLinkMediaPort.saveMediaFile("asset://media", ' report:*?.png ', null);

    // #1135: Core owns the naming policy, so it receives the attachment's own
    // filename and the facts the renderer resolved.
    expect(invoke).toHaveBeenCalledWith("default_media_save_path", {
      filename: " report:*?.png ",
      mediaKind: "file",
      timestampMs: null,
      utcOffsetMinutes: 0,
      localNamePrefix: "Koushi_Image"
    });
    expect(saveDialog).toHaveBeenCalledWith({
      title: "Download report_.png",
      defaultPath: "/downloads/default.png"
    });
    expect(invoke).toHaveBeenCalledWith("save_downloaded_media", {
      sourceUrl: "asset://media",
      destinationPath: "/downloads/chosen.png"
    });
  });

  it("passes the generic-image facts a timestamped name needs (#1135)", async () => {
    // 2026-10-05T09:19:00Z; the offset the platform resolves for that instant is
    // forwarded as minutes ahead of UTC, which is what Core's policy expects.
    const timestampMs = 1_791_191_940_000;
    await tauriLinkMediaPort.saveMediaFile("asset://media", "image.png", {
      kind: "image",
      timestampMs
    });

    expect(invoke).toHaveBeenCalledWith("default_media_save_path", {
      filename: "image.png",
      mediaKind: "image",
      timestampMs,
      utcOffsetMinutes: -new Date(timestampMs).getTimezoneOffset(),
      localNamePrefix: "Koushi_Image"
    });
  });

  it("forwards an empty filename to the naming policy instead of renaming it first", async () => {
    await tauriLinkMediaPort.saveMediaFile("asset://media", "   ", {
      kind: "image",
      timestampMs: null
    });

    expect(invoke).toHaveBeenCalledWith("default_media_save_path", {
      filename: "   ",
      mediaKind: "image",
      timestampMs: null,
      utcOffsetMinutes: 0,
      localNamePrefix: "Koushi_Image"
    });
  });

  it("binds account media saves to the selected tab", async () => {
    const accountTabId = "account:alice";
    setRendererSelectedAccountTabId(accountTabId);
    await tauriLinkMediaPort.saveMediaFile(
      "/accounts/alice/media-downloads/item.bin",
      "media.png",
      null,
      accountTabId
    );

    expect(invoke).toHaveBeenCalledWith("save_downloaded_media", {
      sourceUrl: "/accounts/alice/media-downloads/item.bin",
      destinationPath: "/downloads/chosen.png",
      accountTabId
    });
  });

  it("rejects an account media save if its tab changes while the dialog is open", async () => {
    const accountTabId = "account:alice";
    setRendererSelectedAccountTabId(accountTabId);
    vi.mocked(saveDialog).mockImplementationOnce(async () => {
      setRendererSelectedAccountTabId("account:bob");
      return "/downloads/chosen.png";
    });

    await expect(
      tauriLinkMediaPort.saveMediaFile(
        "/accounts/alice/media-downloads/item.bin",
        "media.png",
        null,
        accountTabId
      )
    ).rejects.toThrow("account tab is no longer selected");
    expect(invoke).not.toHaveBeenCalledWith("save_downloaded_media", expect.anything());
  });

  it("does not save when the dialog is cancelled", async () => {
    vi.mocked(saveDialog).mockResolvedValueOnce(null);
    await tauriLinkMediaPort.saveMediaFile("asset://media", "media.png", null);

    expect(invoke).not.toHaveBeenCalledWith("save_downloaded_media", expect.anything());
  });
});
