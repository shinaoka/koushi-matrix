/* @vitest-environment jsdom */

import { expect, test, vi } from "vitest";

import { browserLinkMediaPort } from "./linkMediaPort";

test("browser link/media operations use native web behavior", async () => {
  const windowOpen = vi.spyOn(window, "open").mockImplementation(() => null);

  await browserLinkMediaPort.openHttpUrl("https://example.com/");
  expect(windowOpen).toHaveBeenCalledWith(
    "https://example.com/",
    "_blank",
    "noopener,noreferrer"
  );
  expect(browserLinkMediaPort.mediaSourceUrl("/tmp/media.png")).toBe("/tmp/media.png");
  expect(browserLinkMediaPort.renderableThumbnailSourceUrl("data:image/gif;base64,R0lGODlh")).toBe(
    "data:image/gif;base64,R0lGODlh"
  );
  expect(browserLinkMediaPort.renderableThumbnailSourceUrl("asset://fixture/avatar.png")).toBe(
    "asset://fixture/avatar.png"
  );
  expect(
    browserLinkMediaPort.renderableThumbnailSourceUrl("avatar/0123456789abcdef")
  ).toBeNull();
  await expect(
    browserLinkMediaPort.saveMediaFile("/tmp/media.png", "media.png", null)
  ).resolves.toBe(undefined);
});
