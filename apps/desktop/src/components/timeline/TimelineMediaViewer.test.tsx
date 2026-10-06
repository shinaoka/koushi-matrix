// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { TimelineMediaViewer, type TimelineMediaViewerItem } from "./TimelineMedia";

afterEach(cleanup);

const item: TimelineMediaViewerItem = {
  sourceUrl: "blob:viewer-image",
  downloadSourceUrl: "blob:viewer-image",
  filename: "viewer-image.png",
  size: 2048,
  mimeType: "image/png",
  width: 640,
  height: 480,
  encrypted: false,
  actions: {
    canForward: false,
    forwardDestinations: [],
    onForward: vi.fn(),
    canViewSource: false,
    onViewSource: vi.fn(),
    canRedact: false,
    onRedact: vi.fn()
  },
  saveName: null
};

describe("TimelineMediaViewer native context menu", () => {
  // #1140/#1133: the viewer owns no custom context menu, so the platform
  // webview's own image menu (WebKitGTK "Copy Image") must stay reachable.
  it("never cancels contextmenu on the full-size image", () => {
    render(<TimelineMediaViewer item={item} onClose={vi.fn()} />);

    const image = screen.getByRole("img", { name: "viewer-image.png" });
    const notCancelled = fireEvent.contextMenu(image);

    expect(notCancelled).toBe(true);
  });
});
