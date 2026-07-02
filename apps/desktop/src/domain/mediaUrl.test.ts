import { convertFileSrc } from "@tauri-apps/api/core";
import { describe, expect, it, vi } from "vitest";

import { mediaSourceUrl } from "./mediaUrl";

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: vi.fn((path: string) => `asset://${path}`)
}));

describe("mediaSourceUrl", () => {
  it("passes web URLs through unchanged", () => {
    expect(mediaSourceUrl("https://example.invalid/image.png")).toBe(
      "https://example.invalid/image.png"
    );
  });

  it("passes already-converted Tauri asset URLs through unchanged", () => {
    expect(mediaSourceUrl("asset://localhost/avatar.png")).toBe("asset://localhost/avatar.png");
  });

  it("passes custom thumbnail protocol URLs through unchanged", () => {
    expect(mediaSourceUrl("koushi-thumbnail://localhost/avatar/abc123")).toBe(
      "koushi-thumbnail://localhost/avatar/abc123"
    );
  });

  it("converts file URLs to Tauri asset URLs using the local path", () => {
    expect(mediaSourceUrl("file:///tmp/avatar%20image.png")).toBe(
      "asset:///tmp/avatar image.png"
    );
    expect(convertFileSrc).toHaveBeenCalledWith("/tmp/avatar image.png");
  });

  it("converts local filesystem paths to Tauri asset URLs", () => {
    expect(mediaSourceUrl("/tmp/media-downloads/report.pdf")).toBe(
      "asset:///tmp/media-downloads/report.pdf"
    );
    expect(convertFileSrc).toHaveBeenLastCalledWith("/tmp/media-downloads/report.pdf");
  });
});
