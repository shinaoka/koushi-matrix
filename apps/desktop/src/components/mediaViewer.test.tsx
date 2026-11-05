// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { MediaViewer, mediaViewerEntryFromGallery } from "./mediaViewer";
import type { TimelineMediaDownloadState, TimelineMediaGalleryItem } from "../domain/types";

afterEach(cleanup);

/** Three images in the room sequence, each with a ready download. */
const IMAGES = [
  { eventId: "$nav-1:example.invalid", filename: "nav-1.png", width: 320 },
  { eventId: "$nav-2:example.invalid", filename: "nav-2.png", width: 480 },
  { eventId: "$nav-3:example.invalid", filename: "nav-3.png", width: 640 }
];

function galleryItem(
  eventId: string,
  filename: string,
  width: number
): TimelineMediaGalleryItem {
  return {
    event_id: eventId,
    room_id: "!room:example.invalid",
    sender: null,
    timestamp_ms: 1_800_000_000_000,
    media: {
      kind: "Image",
      filename,
      source: {
        mxc_uri: `mxc://example.invalid/${width}`,
        encrypted: false,
        encryption_version: null
      },
      mimetype: "image/png",
      size: 2048,
      width,
      height: 200,
      thumbnail: null
    }
  };
}

const items = IMAGES.map((image) =>
  mediaViewerEntryFromGallery(galleryItem(image.eventId, image.filename, image.width))
);

const mediaDownloads: Record<string, TimelineMediaDownloadState> = Object.fromEntries(
  IMAGES.map((image) => [
    image.eventId,
    {
      kind: "ready" as const,
      source_ref: `data:image/png;base64,${image.width}`,
      width: image.width,
      height: 200,
      mime_type: "image/png"
    }
  ])
);

function renderViewer(index: number, overrides: Partial<Parameters<typeof MediaViewer>[0]> = {}) {
  const onSelectIndex = vi.fn();
  const view = render(
    <MediaViewer
      index={index}
      items={items}
      mediaDownloads={mediaDownloads}
      onClose={vi.fn()}
      onSelectIndex={onSelectIndex}
      {...overrides}
    />
  );
  return { ...view, onSelectIndex };
}

const stage = () => document.querySelector<HTMLElement>(".media-viewer-stage")!;
const shownSource = () =>
  document.querySelector<HTMLImageElement>(".media-viewer-image")?.src ?? null;

describe("media viewer image surface", () => {
  // #1140/#1133: the viewer owns no custom context menu, so the platform
  // webview's own image menu (WebKitGTK "Copy Image") must stay reachable.
  it("never cancels contextmenu on the full-size image", () => {
    renderViewer(0);

    const image = screen.getByRole("img", { name: "nav-1.png" });
    const notCancelled = fireEvent.contextMenu(image);

    expect(notCancelled).toBe(true);
  });
});

describe("media viewer media kinds (#1810)", () => {
  it("plays a ready video in the stage the retired timeline viewer used", () => {
    renderViewer(0, {
      items: [{ ...items[0], kind: "Video", mimetype: "video/mp4" }]
    });

    const video = document.querySelector<HTMLVideoElement>(".media-viewer-video");
    expect(video?.src).toContain("data:image/png;base64,320");
  });

  it("keeps a not-ready video on the file placeholder instead of downloading it", () => {
    const onRequestDownload = vi.fn();
    renderViewer(0, {
      items: [{ ...items[0], kind: "Video", mimetype: "video/mp4" }],
      mediaDownloads: {},
      onRequestDownload
    });

    expect(document.querySelector(".media-viewer-video")).toBeNull();
    expect(document.querySelector(".media-viewer-file-placeholder")).not.toBeNull();
    // #1810 asked for the image sequence's downloads only, never for files.
    expect(onRequestDownload).not.toHaveBeenCalled();
  });
});

describe("media viewer timeline footer (#1810)", () => {
  it("offers Save even when no actions resolve for the shown image", () => {
    renderViewer(0, { actionsFor: () => null });

    expect(screen.getByRole("button", { name: "Download nav-1.png" })).toBeTruthy();
  });

  it("keeps an action-less footer where the mount supplies no resolver", () => {
    renderViewer(0);

    expect(screen.queryByRole("button", { name: /Download/ })).toBeNull();
    expect(screen.queryByRole("button", { name: "Message actions" })).toBeNull();
  });

  it("retries a failed download once instead of on every state update", () => {
    const onRequestDownload = vi.fn();
    const { rerender } = renderViewer(0, {
      mediaDownloads: {
        [IMAGES[0].eventId]: { kind: "failed", failure_kind: "network" }
      },
      onRequestDownload
    });

    rerender(
      <MediaViewer
        index={0}
        items={items}
        mediaDownloads={{
          ...mediaDownloads,
          [IMAGES[0].eventId]: { kind: "failed", failure_kind: "network" }
        }}
        onClose={vi.fn()}
        onSelectIndex={vi.fn()}
        onRequestDownload={onRequestDownload}
      />
    );

    expect(onRequestDownload).toHaveBeenCalledTimes(1);
  });
});

describe("media viewer position (#1810)", () => {
  it("shows the position in the room sequence", () => {
    renderViewer(1);

    expect(document.querySelector(".media-viewer-counter")?.textContent).toBe("2/3");
  });

  it("hides navigation and the counter for a single image (#163)", () => {
    renderViewer(0, { items: [items[0]] });

    expect(screen.queryByRole("button", { name: "Next media" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Previous media" })).toBeNull();
    expect(document.querySelector(".media-viewer-counter")).toBeNull();
  });

  it("stops at the first image instead of wrapping to the last", () => {
    const { onSelectIndex } = renderViewer(0);

    const previous = screen.getByRole("button", { name: "Previous media" });
    expect(previous.hasAttribute("disabled")).toBe(true);
    fireEvent.click(previous);
    expect(onSelectIndex).not.toHaveBeenCalled();
  });

  it("stops at the last image instead of wrapping to the first", () => {
    const { onSelectIndex } = renderViewer(IMAGES.length - 1);

    const next = screen.getByRole("button", { name: "Next media" });
    expect(next.hasAttribute("disabled")).toBe(true);
    fireEvent.click(next);
    expect(onSelectIndex).not.toHaveBeenCalled();
  });
});

describe("media viewer keyboard navigation (#1810)", () => {
  it("navigates with ArrowRight when the image cannot pan", () => {
    const { onSelectIndex } = renderViewer(0);

    fireEvent.keyDown(stage(), { key: "ArrowRight" });

    expect(onSelectIndex).toHaveBeenCalledWith(1);
  });

  it("navigates with ArrowLeft when the image cannot pan", () => {
    const { onSelectIndex } = renderViewer(1);

    fireEvent.keyDown(stage(), { key: "ArrowLeft" });

    expect(onSelectIndex).toHaveBeenCalledWith(0);
  });

  it("jumps to the first and last image with Home and End", () => {
    const { onSelectIndex } = renderViewer(1);

    fireEvent.keyDown(stage(), { key: "End" });
    expect(onSelectIndex).toHaveBeenCalledWith(IMAGES.length - 1);

    fireEvent.keyDown(stage(), { key: "Home" });
    expect(onSelectIndex).toHaveBeenCalledWith(0);
  });

  it("leaves the arrows inert when there is nothing to navigate to", () => {
    const { onSelectIndex } = renderViewer(0, { items: [items[0]] });

    fireEvent.keyDown(stage(), { key: "ArrowRight" });

    expect(onSelectIndex).not.toHaveBeenCalled();
  });
});

describe("media viewer image switching (#1810)", () => {
  it("shows the switched image from its own download", () => {
    const { rerender } = renderViewer(0);
    expect(shownSource()).toContain(IMAGES[0].width.toString());

    rerender(
      <MediaViewer
        index={1}
        items={items}
        mediaDownloads={mediaDownloads}
        onClose={vi.fn()}
        onSelectIndex={vi.fn()}
      />
    );

    expect(shownSource()).toContain(IMAGES[1].width.toString());
  });

  it("resets zoom and pan when the image changes", () => {
    const { rerender } = renderViewer(0);
    fireEvent.click(screen.getByRole("button", { name: "Zoom in" }));
    fireEvent.click(screen.getByRole("button", { name: "Zoom in" }));
    expect(document.querySelector<HTMLElement>(".media-viewer-image")!.style.transform).toContain(
      "scale(1.5)"
    );

    rerender(
      <MediaViewer
        index={1}
        items={items}
        mediaDownloads={mediaDownloads}
        onClose={vi.fn()}
        onSelectIndex={vi.fn()}
      />
    );

    expect(document.querySelector<HTMLElement>(".media-viewer-image")!.style.transform).toContain(
      "scale(1)"
    );
  });
});
