// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { openExternalHttpUrl } from "../backend/linkMediaRuntime";

vi.mock("../backend/linkMediaRuntime", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../backend/linkMediaRuntime")>()),
  openExternalHttpUrl: vi.fn(async () => undefined)
}));

import { type CoreEventPayload, type TimelineItem } from "../domain/coreEvents";
import { setActiveLocaleProfile } from "../i18n/messages";
import { KEY, baseTransport, fileMessage, imageMessage, message } from "./timelineViewTestSupport";
import {
  TimelineView,
  clearTimelineViewportSessionMemoryForTests,
  timelineMediaDisplayBoxForTests
} from "./TimelineView";

const AVATAR_DATA_URL_B = "data:image/gif;base64,R0lGODlhAQABAIAAAAD/AP///ywAAAAAAQABAAACAUwAOw==";

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  setActiveLocaleProfile("en", "none");
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("TimelineView", () => {

  it("computes a stable clamped media box for known image dimensions", () => {
    expect(timelineMediaDisplayBoxForTests(2048, 1188)).toEqual({
      inlineSize: 420,
      blockSize: 244
    });
    expect(timelineMediaDisplayBoxForTests(800, 1600)).toEqual({
      inlineSize: 130,
      blockSize: 260
    });
    expect(timelineMediaDisplayBoxForTests(null, 1600)).toEqual({
      inlineSize: 347,
      blockSize: 260
    });
    expect(timelineMediaDisplayBoxForTests(800, null)).toEqual({
      inlineSize: 347,
      blockSize: 260
    });
  });

  it("automatically requests previews for encrypted image attachments", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const downloadMedia = vi.fn(async () => undefined);
    const transport = baseTransport({
      downloadMedia,
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [imageMessage("$encrypted-image", true)]
          }
        }
      });
    });

    await waitFor(() => {
      expect(downloadMedia).toHaveBeenCalledWith(
        "!room:example.invalid",
        "$encrypted-image"
      );
    });
  });

  it("renders ready image with image-first layout and hover download overlay", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        mediaDownloads={{
          "$ready-image": {
            kind: "ready",
            source_url: "appmedia://synthetic-image",
            width: 2048,
            height: 1188,
            mime_type: "image/png"
          }
        }}
        onReply={vi.fn()}
      />
    );

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [imageMessage("$ready-image", false)]
          }
        }
      });
    });

    await waitFor(() => {
      const media = document.querySelector('[data-event-id="$ready-image"] .message-media');
      expect(media).not.toBeNull();
      expect(media?.getAttribute("data-download-state")).toBe("ready");
      // #163: image-first layout — the preview is the primary block. The
      // filename lives on the image (alt), not as text laid over the preview,
      // and download appears in the hover/focus action overlay.
      const image = media?.querySelector<HTMLImageElement>(".message-media-image");
      expect(image).not.toBeNull();
      expect(image?.getAttribute("alt")).toBe("photo.png");
      const actionButtons = Array.from(
        media?.querySelectorAll<HTMLButtonElement>(
          ".message-media-hover-actions .message-media-hover-action"
        ) ?? []
      );
      const actionLabels = actionButtons.map((button) => button.getAttribute("aria-label"));
      expect(actionLabels).toEqual(["Show media details for photo.png", "Download photo.png"]);
      const downloadButton = actionButtons.find(
        (button) => button.getAttribute("aria-label") === "Download photo.png"
      );
      expect(downloadButton).not.toBeNull();
      expect(downloadButton?.tagName).toBe("BUTTON");
      expect(media?.textContent).not.toContain("image/png");
      expect(media?.textContent).not.toContain("407 KB");
    });
  });

  it("renders ready file downloads as navigation-safe buttons", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        mediaDownloads={{
          "$ready-file": {
            kind: "ready",
            source_url: "asset://localhost/notes.pdf",
            width: null,
            height: null,
            mime_type: "application/pdf"
          }
        }}
        onReply={vi.fn()}
      />
    );

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [fileMessage("$ready-file")]
          }
        }
      });
    });

    await waitFor(() => {
      const downloadButton = document.querySelector<HTMLButtonElement>(
        '[data-event-id="$ready-file"] button.message-media-download'
      );
      expect(downloadButton).not.toBeNull();
      expect(downloadButton?.getAttribute("aria-label")).toBe("Download notes.pdf");
    });
  });

  it("saves an initially idle file after its first download becomes ready", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const downloadMedia = vi.fn(async () => undefined);
    const saveMediaFile = vi.fn(async () => undefined);
    const transport = baseTransport({
      downloadMedia,
      saveMediaFile,
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const props = {
      timelineKey: KEY,
      roomId: "!room:example.invalid",
      transport,
      onReply: vi.fn()
    };

    const { rerender } = render(<TimelineView {...props} onReply={vi.fn()} />);
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [fileMessage("$idle-file")]
          }
        }
      });
    });

    fireEvent.click(await screen.findByRole("button", { name: "Download notes.pdf" }));
    await waitFor(() =>
      expect(downloadMedia).toHaveBeenCalledWith("!room:example.invalid", "$idle-file")
    );

    rerender(
      <TimelineView
        {...props}
        onReply={vi.fn()}
        mediaDownloads={{
          "$idle-file": {
            kind: "ready",
            source_url: "asset://localhost/notes.pdf",
            width: null,
            height: null,
            mime_type: "application/pdf"
          }
        }}
      />
    );

    await waitFor(() =>
      expect(saveMediaFile).toHaveBeenCalledWith("asset://localhost/notes.pdf", "notes.pdf")
    );
    expect(saveMediaFile).toHaveBeenCalledTimes(1);
  });

  it("does not save a file that becomes ready without a download click", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const saveMediaFile = vi.fn(async () => undefined);
    const transport = baseTransport({
      saveMediaFile,
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const { rerender } = render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [fileMessage("$ready-without-click")]
          }
        }
      });
    });
    rerender(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        mediaDownloads={{
          "$ready-without-click": {
            kind: "ready",
            source_url: "asset://localhost/notes.pdf",
            width: null,
            height: null,
            mime_type: "application/pdf"
          }
        }}
        onReply={vi.fn()}
      />
    );

    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(saveMediaFile).not.toHaveBeenCalled();
  });

  it("clears save intent on failure and rearms it for a retry", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const downloadMedia = vi.fn(async () => undefined);
    const saveMediaFile = vi.fn(async () => undefined);
    const transport = baseTransport({
      downloadMedia,
      saveMediaFile,
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const props = {
      timelineKey: KEY,
      roomId: "!room:example.invalid",
      transport,
      onReply: vi.fn()
    };
    const { rerender } = render(<TimelineView {...props} onReply={vi.fn()} />);
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [fileMessage("$retry-file")]
          }
        }
      });
    });

    fireEvent.click(await screen.findByRole("button", { name: "Download notes.pdf" }));
    await waitFor(() => expect(downloadMedia).toHaveBeenCalledTimes(1));
    rerender(
      <TimelineView
        {...props}
        onReply={vi.fn()}
        mediaDownloads={{
          "$retry-file": { kind: "failed", failure_kind: "network" }
        }}
      />
    );
    expect(saveMediaFile).not.toHaveBeenCalled();

    fireEvent.click(await screen.findByRole("button", { name: "Retry" }));
    expect(downloadMedia).toHaveBeenCalledTimes(2);
    rerender(
      <TimelineView
        {...props}
        onReply={vi.fn()}
        mediaDownloads={{
          "$retry-file": {
            kind: "ready",
            source_url: "asset://localhost/notes.pdf",
            width: null,
            height: null,
            mime_type: "application/pdf"
          }
        }}
      />
    );

    await waitFor(() =>
      expect(saveMediaFile).toHaveBeenCalledWith("asset://localhost/notes.pdf", "notes.pdf")
    );
    expect(saveMediaFile).toHaveBeenCalledTimes(1);
  });

  it("routes ready image preview downloads through the transport when available", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const fetchMock = vi.fn(async () => new Response(new Blob(["image"], { type: "image/png" })));
    const createObjectURL = vi.fn(() => "blob:downloaded-image");
    const revokeObjectURL = vi.fn();
    const OriginalURL = URL;
    class MockURL extends OriginalURL {
      static override createObjectURL = createObjectURL;
      static override revokeObjectURL = revokeObjectURL;
    }
    const clickedAnchors: HTMLAnchorElement[] = [];
    vi.stubGlobal("fetch", fetchMock);
    vi.stubGlobal("URL", MockURL);
    vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(function (
      this: HTMLAnchorElement
    ) {
      clickedAnchors.push(this);
    });
    const saveMediaFile = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      saveMediaFile
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        mediaDownloads={{
          "$ready-image": {
            kind: "ready",
            source_url: "asset://localhost/original-photo.png",
            width: 2048,
            height: 1188,
            mime_type: "image/png"
          }
        }}
        onReply={vi.fn()}
      />
    );

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [imageMessage("$ready-image", false)]
          }
        }
      });
    });

    const downloadButton = await screen.findByRole("button", { name: "Download photo.png" });
    fireEvent.click(downloadButton);

    await waitFor(() => {
      expect(saveMediaFile).toHaveBeenCalledWith(
        "asset://localhost/original-photo.png",
        "photo.png"
      );
    });
    expect(fetchMock).not.toHaveBeenCalled();
    expect(createObjectURL).not.toHaveBeenCalled();
    expect(clickedAnchors).toHaveLength(0);
    expect(screen.queryByRole("dialog", { name: "Media viewer" })).toBeNull();
  });

  it("does not request encrypted image previews for off-window initial virtualized items", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const downloadMedia = vi.fn(async () => undefined);
    const transport = baseTransport({
      downloadMedia,
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const items = Array.from({ length: 700 }, (_, index) =>
      index === 350
        ? imageMessage("$offscreen-image", true)
        : message(`$plain-${index}`, `Plain ${index}`)
    );

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items
          }
        }
      });
    });

    await waitFor(() => {
      const renderedItems = Number(
        screen.getByTestId("timeline-view").getAttribute("data-rendered-items")
      );
      expect(renderedItems).toBeGreaterThan(0);
      expect(renderedItems).toBeLessThan(items.length);
    });
    expect(downloadMedia).not.toHaveBeenCalledWith(
      "!room:example.invalid",
      "$offscreen-image"
    );
    expect(downloadMedia).not.toHaveBeenCalled();
  });

  it("opens ready image previews in an in-app media viewer", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        mediaDownloads={{
          "$ready-image": {
            kind: "ready",
            source_url: "asset://localhost/original-photo.png",
            width: 2048,
            height: 1188,
            mime_type: "image/png"
          }
        }}
        onReply={vi.fn()}
      />
    );

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [imageMessage("$ready-image", true)]
          }
        }
      });
    });

    await waitFor(() => {
      const image = screen.getByRole("img", { name: "photo.png" });
      const previewButton = image.closest("button");
      expect(previewButton?.getAttribute("aria-label")).toBe("Open file");
      const media = document.querySelector(".message-media");
      // #163: image-first layout. The encrypted badge stays visible as a
      // security signal and the download sits in the hover overlay, but
      // filename/mimetype/size no longer occupy layout over the preview.
      expect(media?.querySelector(".message-media-image-badge")?.textContent).toContain(
        "Encrypted"
      );
      expect(media?.querySelector(".message-media-hover-actions")).not.toBeNull();
      expect(media?.textContent).not.toContain("image/png");
      expect(media?.textContent).not.toContain("407 KB");
    });

    fireEvent.click(screen.getByRole("button", { name: "Open file" }));

    const viewer = await screen.findByRole("dialog", { name: "Media viewer" });
    expect(viewer.textContent).toContain("photo.png");
    expect(viewer.textContent).toContain("407 KB");
    expect(viewer.querySelector<HTMLImageElement>(".timeline-media-viewer-image")?.src).toContain(
      "asset://localhost/original-photo.png"
    );

    fireEvent.click(screen.getByRole("button", { name: "Close media viewer" }));
    await waitFor(() => {
      expect(screen.queryByRole("dialog", { name: "Media viewer" })).toBeNull();
    });
  });

  it("keeps ready image metadata behind an inline details action", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        mediaDownloads={{
          "$ready-image": {
            kind: "ready",
            source_url: "asset://localhost/original-photo.png",
            width: 2048,
            height: 1188,
            mime_type: "image/png"
          }
        }}
        onReply={vi.fn()}
      />
    );

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [imageMessage("$ready-image", true)]
          }
        }
      });
    });

    const detailsButton = await screen.findByRole("button", {
      name: "Show media details for photo.png"
    });
    const media = document.querySelector(".message-media");
    expect(media?.textContent).not.toContain("image/png");
    expect(media?.textContent).not.toContain("407 KB");

    fireEvent.click(detailsButton);

    const details = await screen.findByRole("dialog", { name: "Media details" });
    expect(details.textContent).toContain("photo.png");
    expect(details.textContent).toContain("image/png");
    expect(details.textContent).toContain("407 KB");
    expect(details.textContent).toContain("2048x1188");
    expect(details.textContent).toContain("Encrypted");
  });

  it("focuses the media viewer close control and returns focus to the clicked image", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        mediaDownloads={{
          "$ready-image": {
            kind: "ready",
            source_url: "asset://localhost/original-photo.png",
            width: 2048,
            height: 1188,
            mime_type: "image/png"
          }
        }}
        onReply={vi.fn()}
      />
    );

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [imageMessage("$ready-image", false)]
          }
        }
      });
    });

    const openButton = await screen.findByRole("button", { name: "Open file" });
    openButton.focus();
    fireEvent.click(openButton);

    const viewer = await screen.findByRole("dialog", { name: "Media viewer" });
    const closeButton = within(viewer).getByRole("button", { name: "Close media viewer" });
    await waitFor(() => {
      expect(document.activeElement).toBe(closeButton);
    });

    const tabEvent = new KeyboardEvent("keydown", {
      key: "Tab",
      bubbles: true,
      cancelable: true
    });
    document.dispatchEvent(tabEvent);
    expect(tabEvent.defaultPrevented).toBe(true);
    expect(viewer.contains(document.activeElement)).toBe(true);

    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => {
      expect(screen.queryByRole("dialog", { name: "Media viewer" })).toBeNull();
    });
    expect(document.activeElement).toBe(openButton);
  });

  it("routes media viewer message actions through the event transport", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const loadMessageSource = vi.fn(async () => undefined);
    const redactMessage = vi.fn(async () => undefined);
    const forwardMessage = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      loadMessageSource,
      redactMessage,
      forwardMessage
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        mediaDownloads={{
          "$ready-image": {
            kind: "ready",
            source_url: "asset://localhost/original-photo.png",
            width: 2048,
            height: 1188,
            mime_type: "image/png"
          }
        }}
        forwardDestinations={[
          {
            room_id: "!destination:example.invalid",
            display_name: "Destination room"
          }
        ]}
        onReply={vi.fn()}
      />
    );

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [
              {
                ...imageMessage("$ready-image", false),
                can_redact: true,
                actions: {
                  can_copy: false,
                  can_forward: true,
                  can_reply: true,
                  can_permalink: false,
                  can_view_source: true
                }
              }
            ]
          }
        }
      });
    });

    fireEvent.click(await screen.findByRole("button", { name: "Open file" }));
    let viewer = await screen.findByRole("dialog", { name: "Media viewer" });
    fireEvent.click(within(viewer).getByRole("button", { name: "Message actions" }));
    expect(within(viewer).getByRole("menu", { name: "Message actions" })).not.toBeNull();
    fireEvent.click(within(viewer).getByRole("menuitem", { name: "Forward" }));
    fireEvent.click(within(viewer).getByRole("menuitem", { name: "Destination room" }));
    await waitFor(() => {
      expect(forwardMessage).toHaveBeenCalledWith(
        "!room:example.invalid",
        "$ready-image",
        "!destination:example.invalid"
      );
      expect(screen.queryByRole("dialog", { name: "Media viewer" })).toBeNull();
    });

    fireEvent.click(screen.getByRole("button", { name: "Open file" }));
    viewer = await screen.findByRole("dialog", { name: "Media viewer" });
    fireEvent.click(within(viewer).getByRole("button", { name: "Message actions" }));
    fireEvent.click(within(viewer).getByRole("menuitem", { name: "View source" }));
    await waitFor(() => {
      expect(loadMessageSource).toHaveBeenCalledWith("!room:example.invalid", "$ready-image");
      expect(screen.queryByRole("dialog", { name: "Media viewer" })).toBeNull();
    });

    fireEvent.click(screen.getByRole("button", { name: "Open file" }));
    viewer = await screen.findByRole("dialog", { name: "Media viewer" });
    fireEvent.click(within(viewer).getByRole("button", { name: "Message actions" }));
    fireEvent.click(within(viewer).getByRole("menuitem", { name: "Remove" }));

    await waitFor(() => {
      expect(redactMessage).toHaveBeenCalledWith("!room:example.invalid", "$ready-image");
      expect(screen.queryByRole("dialog", { name: "Media viewer" })).toBeNull();
    });
  });

  it("requests visible sender avatar thumbnails that are not yet downloaded", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const downloadAvatarThumbnail = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      downloadAvatarThumbnail
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        enableAvatarThumbnailDownloads={true}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [
            {
              ...message("$avatar", "Avatar row"),
              sender_avatar: {
                mxc_uri: "mxc://matrix.org/avatar",
                thumbnail: { kind: "notRequested" }
              }
            }
          ]
        }
      }
    });

    await waitFor(() => {
      expect(downloadAvatarThumbnail).toHaveBeenCalledWith("mxc://matrix.org/avatar");
    });
    expect(downloadAvatarThumbnail).toHaveBeenCalledTimes(1);
  });

  it("limits initial avatar thumbnail requests to the current viewport window", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const downloadAvatarThumbnail = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      downloadAvatarThumbnail
    });
    const items = Array.from({ length: 40 }, (_, index) => ({
      ...message(`$avatar-window-${index}`, `Avatar row ${index}`),
      sender_avatar: {
        mxc_uri: `mxc://matrix.org/avatar-window-${index}`,
        thumbnail: { kind: "notRequested" as const }
      }
    }));

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        enableAvatarThumbnailDownloads={true}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items
        }
      }
    });

    await waitFor(() => {
      expect(downloadAvatarThumbnail).toHaveBeenCalledWith(
        "mxc://matrix.org/avatar-window-0"
      );
    });
    expect(downloadAvatarThumbnail).not.toHaveBeenCalledWith(
      "mxc://matrix.org/avatar-window-39"
    );
    expect(downloadAvatarThumbnail.mock.calls.length).toBeLessThan(items.length);
  });

  it("requests profile avatar thumbnails when the timeline item has no sender avatar", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const downloadAvatarThumbnail = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      downloadAvatarThumbnail
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        profileUsers={{
          "@bob:example.invalid": {
            user_id: "@bob:example.invalid",
            display_name: "Bob",
            display_label: "Bob",
            original_display_label: "Bob",
            mention_search_terms: ["bob"],
            avatar: {
              mxc_uri: "mxc://matrix.org/profile-avatar",
              thumbnail: { kind: "notRequested" }
            }
          }
        }}
        onReply={vi.fn()}
        enableAvatarThumbnailDownloads={true}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [message("$profile-avatar", "Profile avatar row")]
        }
      }
    });

    await waitFor(() => {
      expect(downloadAvatarThumbnail).toHaveBeenCalledWith("mxc://matrix.org/profile-avatar");
    });
    expect(downloadAvatarThumbnail).toHaveBeenCalledTimes(1);
  });

  it("does NOT call downloadAvatarThumbnail when enableAvatarThumbnailDownloads is explicitly false (kill-switch)", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const downloadAvatarThumbnail = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      downloadAvatarThumbnail
    });

    // Explicitly disable via the kill-switch prop (#116 Stage F1a: default is now ON).
    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        enableAvatarThumbnailDownloads={false}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [
            {
              ...message("$avatar-gated", "Avatar row (kill-switch off)"),
              sender_avatar: {
                mxc_uri: "mxc://matrix.org/avatar-gated",
                thumbnail: { kind: "notRequested" }
              }
            }
          ]
        }
      }
    });

    // Give React time to flush any effects that might fire.
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(downloadAvatarThumbnail).not.toHaveBeenCalled();
  });

  it("ignores avatar thumbnail events that are not relevant to the mounted timeline", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const onDiagnosticLogEntry = vi.fn();
    const onDiagnosticsChange = vi.fn();
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onDiagnosticsChange={onDiagnosticsChange}
        onDiagnosticLogEntry={onDiagnosticLogEntry}
        onReply={vi.fn()}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [
            {
              ...message("$avatar-relevant", "Avatar row"),
              sender_avatar: {
                mxc_uri: "mxc://matrix.org/relevant-avatar",
                thumbnail: { kind: "notRequested" }
              }
            }
          ]
        }
      }
    });
    await waitFor(() =>
      expect(onDiagnosticsChange).toHaveBeenCalledWith(
        expect.objectContaining({
          avatarMxcItems: 1,
          avatarPendingItems: 1,
          visibleItems: 1
        })
      )
    );
    onDiagnosticLogEntry.mockClear();
    onDiagnosticsChange.mockClear();

    emit({
      kind: "Account",
      event: {
        AvatarThumbnailDownloaded: {
          request_id: { connection_id: 1, sequence: 2 },
          mxc_uri: "mxc://matrix.org/unrelated-avatar",
          thumbnail: {
            kind: "ready",
            source_ref: AVATAR_DATA_URL_B,
            width: null,
            height: null,
            mime_type: null
          }
        }
      }
    });

    await new Promise((resolve) => window.setTimeout(resolve, 0));
    expect(onDiagnosticLogEntry).not.toHaveBeenCalledWith(
      expect.objectContaining({
        source: "timeline.avatar",
        message: "avatar thumbnail ready"
      })
    );
    expect(onDiagnosticsChange).not.toHaveBeenCalled();
    expect(document.querySelector(".message .avatar img")).toBeNull();
  });

  it("falls back to sender initials when a downloaded sender avatar image is broken", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [
            {
              ...message("$avatar-broken", "Avatar row"),
              sender_label: "Ken Inayoshi",
              sender_avatar: {
                mxc_uri: "mxc://matrix.org/avatar-broken",
                thumbnail: {
                  kind: "ready",
                  source_ref: "asset://missing-avatar.bin",
                  width: null,
                  height: null,
                  mime_type: null
                }
              }
            }
          ]
        }
      }
    });

    const image = await waitFor(() => {
      const element = document.querySelector<HTMLImageElement>(".message .avatar img");
      expect(element?.getAttribute("src")).toBe("asset://missing-avatar.bin");
      return element!;
    });
    fireEvent.error(image);

    expect(document.querySelector(".message .avatar img")).toBeNull();
    expect(document.querySelector(".message .avatar")?.textContent).toBe("KE");
  });

  it("retries a transiently broken sender avatar image URL", async () => {
    vi.useFakeTimers();
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [
              {
                ...message("$avatar-retry-render", "Avatar row"),
                sender_label: "Ken Inayoshi",
                sender_avatar: {
                  mxc_uri: "mxc://matrix.org/avatar-retry-render",
                  thumbnail: {
                    kind: "ready",
                    source_ref: "asset://transient-avatar.bin",
                    width: null,
                    height: null,
                    mime_type: null
                  }
                }
              }
            ]
          }
        }
      });
    });

    const image = document.querySelector<HTMLImageElement>(".message .avatar img");
    expect(image).not.toBeNull();
    expect(image?.getAttribute("src")).toBe("asset://transient-avatar.bin");
    fireEvent.error(image!);
    expect(document.querySelector(".message .avatar img")).toBeNull();

    act(() => {
      vi.advanceTimersByTime(10_000);
    });

    expect(document.querySelector<HTMLImageElement>(".message .avatar img")?.getAttribute("src")).toBe(
      "asset://transient-avatar.bin"
    );
  });

  it("renders link preview cards as clickable anchors", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const hideLinkPreview = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      hideLinkPreview
    });
    const item: TimelineItem = {
      ...message("$preview:example.invalid", "look at this"),
      link_previews: [
        {
          url: "https://example.com/article",
          title: "An article",
          state: "ready"
        }
      ]
    };

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [item]
        }
      }
    });

    const card = await screen.findByRole("link", { name: /An article/ });
    expect(card.getAttribute("href")).toBe("https://example.com/article");
    fireEvent.click(card);
    await waitFor(() => {
      expect(openExternalHttpUrl).toHaveBeenCalledWith("https://example.com/article");
    });

    const hide = screen.getByRole("button", { name: "Hide preview" });
    fireEvent.click(hide);
    await waitFor(() => {
      expect(hideLinkPreview).toHaveBeenCalledWith("!room:example.invalid", "$preview:example.invalid");
    });
  });

  it("emits private-data-free diagnostics when viewport pending link previews load", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const loadLinkPreviews = vi.fn(async () => undefined);
    const onDiagnosticLogEntry = vi.fn();
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      loadLinkPreviews
    });
    const item: TimelineItem = {
      ...message("$pending-preview:example.invalid", "look at https://secret.example/article"),
      link_previews: [
        {
          url: "https://secret.example/article",
          state: "pending"
        }
      ]
    };

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        onDiagnosticLogEntry={onDiagnosticLogEntry}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [item]
        }
      }
    });

    await waitFor(() => {
      expect(loadLinkPreviews).toHaveBeenCalledWith(
        "!room:example.invalid",
        "$pending-preview:example.invalid"
      );
      expect(onDiagnosticLogEntry).toHaveBeenCalledWith(
        expect.objectContaining({
          source: "timeline.preview",
          message: "kind=room stage=request trigger=viewport_pending pending=1"
        })
      );
    });

    emit({
      kind: "Timeline",
      event: {
        ItemsUpdated: {
          key: KEY,
          generation: 1,
          batch_id: 1,
          diffs: [
            {
              Set: {
                index: 0,
                item: {
                  ...item,
                  link_previews: [
                    {
                      url: "https://secret.example/article",
                      title: "Loaded",
                      state: "ready"
                    }
                  ]
                }
              }
            }
          ]
        }
      }
    });

    await waitFor(() => {
      expect(onDiagnosticLogEntry).toHaveBeenCalledWith(
        expect.objectContaining({
          source: "timeline.preview",
          message: "kind=room stage=update items=1 pending=0 loading=0 ready=1 failed=0"
        })
      );
    });

    const diagnosticText = onDiagnosticLogEntry.mock.calls
      .map(([entry]) => `${entry.source} ${entry.message}`)
      .join("\n");
    expect(diagnosticText).not.toContain("$pending-preview");
    expect(diagnosticText).not.toContain("secret.example");
  });

  it("limits initial link preview requests to the current viewport window", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const loadLinkPreviews = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      loadLinkPreviews
    });
    const items = Array.from({ length: 40 }, (_, index) => ({
      ...message(`$preview-window-${index}`, `Preview row ${index}`),
      link_previews: [
        {
          url: `https://example.invalid/preview-window-${index}`,
          state: "pending" as const
        }
      ]
    }));

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items
        }
      }
    });

    await waitFor(() => {
      expect(loadLinkPreviews).toHaveBeenCalledWith(
        "!room:example.invalid",
        "$preview-window-0"
      );
    });
    expect(loadLinkPreviews).not.toHaveBeenCalledWith(
      "!room:example.invalid",
      "$preview-window-39"
    );
    expect(loadLinkPreviews.mock.calls.length).toBeLessThan(items.length);
  });
});
