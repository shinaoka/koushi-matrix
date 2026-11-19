// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  MessageArticle,
  PinnedEventsList,
  PinnedMessagesEntry,
  RoomMediaGallery,
  ScheduledMessagesList
} from "./mediaLists";
import { MediaViewer, mediaViewerEntryFromGallery } from "./mediaViewer";
import type { TimelineMediaGalleryItem } from "../domain/types";

afterEach(cleanup);

describe("people-facing media list labels", () => {
  it("renders pinned-event sender_label and hides the raw sender id", () => {
    render(
      <PinnedEventsList
        roomId="!room:example.invalid"
        pinnedEvents={[
          {
            event_id: "$pinned:example.invalid",
            sender: "@private:example.invalid",
            sender_label: "Pinned Alias",
            body_preview: "Pinned body",
            redacted: false
          }
        ]}
        onUnpin={() => undefined}
      />
    );

    expect(screen.getByText("Pinned Alias")).toBeTruthy();
    expect(screen.queryByText("@private:example.invalid")).toBeNull();
  });

  it("navigates by the owning room and exact event ID from a pinned row", () => {
    const onOpen = vi.fn();
    render(
      <PinnedEventsList
        roomId="!room:example.invalid"
        pinnedEvents={[
          {
            event_id: "$pinned:example.invalid",
            sender: "@private:example.invalid",
            sender_label: "Pinned Alias",
            body_preview: "Pinned body",
            redacted: false,
            timestamp_ms: 1_800_000_000_000,
            state: "ready",
            thread_root_event_id: null
          }
        ]}
        onOpen={onOpen}
        onUnpin={() => undefined}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: /Pinned body/ }));

    expect(onOpen).toHaveBeenCalledWith(
      "!room:example.invalid",
      "$pinned:example.invalid",
      null
    );
  });

  it("opens the pinned panel from the compact timeline entry", () => {
    const onOpen = vi.fn();
    render(<PinnedMessagesEntry count={3} onOpen={onOpen} />);

    fireEvent.click(screen.getByRole("button", { name: "Pinned · 3" }));

    expect(onOpen).toHaveBeenCalledOnce();
  });

  it("renders a profile label in fixture message metadata and fails closed when absent", () => {
    const message = {
      room_id: "!room:example.invalid",
      event_id: "$message:example.invalid",
      sender: "@private:example.invalid",
      timestamp_ms: 1_800_000_000_000,
      body: "Body",
      attachment_filename: null,
      reply_count: 0
    };
    const props = {
      currentUserId: null,
      message,
      highlights: [],
      onOpenThread: vi.fn(),
      onRedactMessage: vi.fn(),
      isIgnored: false
    };
    const { rerender } = render(
      <MessageArticle
        {...props}
        profileUsers={{
          [message.sender]: {
            user_id: message.sender,
            display_name: "Profile Name",
            display_label: "Local Alias",
            original_display_label: "Profile Name",
            mention_search_terms: [],
            avatar: null
          }
        }}
      />
    );

    expect(screen.getByText("Local Alias")).toBeTruthy();
    expect(screen.queryByText(message.sender)).toBeNull();

    rerender(<MessageArticle {...props} profileUsers={{}} />);
    expect(screen.getByText("Unknown user")).toBeTruthy();
    expect(screen.queryByText(message.sender)).toBeNull();
  });
});

describe("scheduled message editing", () => {
  it("edits the body and time through the shared composer controls", () => {
    const onReschedule = vi.fn();
    render(
      <ScheduledMessagesList
        capability="localFallback"
        items={[
          {
            scheduled_id: "scheduled-1",
            room_id: "!room:example.invalid",
            body: "Original body",
            send_at_ms: Date.UTC(2030, 0, 1, 12, 0),
            handle: { kind: "local" }
          }
        ]}
        onCancel={() => undefined}
        onReschedule={onReschedule}
        canEdit
        mathModeEnabled={false}
        onMathModeChange={() => undefined}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Edit scheduled send" }));
    const body = screen.getByRole("textbox", { name: "Scheduled message" });
    expect(body.textContent).toBe("Original body");
    body.textContent = "Edited **body**";
    fireEvent.input(body);
    expect(screen.getByRole("button", { name: "Save scheduled send" }).textContent).toBe("Save");
    fireEvent.click(screen.getByRole("button", { name: "Save scheduled send" }));

    expect(onReschedule).toHaveBeenCalledWith("scheduled-1", "Edited **body**", expect.any(Number));
  });

  // #1983: the same list renders inside the selected-room pane, so the lock has
  // to hold at the component itself, not only through the right panel's wiring.
  it("refuses to open the editor while sending is locked", () => {
    const onReschedule = vi.fn();
    render(
      <ScheduledMessagesList
        capability="localFallback"
        items={[
          {
            scheduled_id: "scheduled-1",
            room_id: "!room:example.invalid",
            body: "Original body",
            send_at_ms: Date.UTC(2030, 0, 1, 12, 0),
            handle: { kind: "local" }
          }
        ]}
        onCancel={() => undefined}
        onReschedule={onReschedule}
        canEdit={false}
        mathModeEnabled={false}
        onMathModeChange={() => undefined}
      />
    );

    expect(
      (screen.getByRole("button", { name: "Edit scheduled send" }) as HTMLButtonElement).disabled
    ).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Edit scheduled send" }));
    expect(screen.queryByRole("textbox", { name: "Scheduled message" })).toBeNull();
    expect(onReschedule).not.toHaveBeenCalled();
  });
});

describe("media gallery viewer header", () => {
  const galleryItem = (
    media: Partial<TimelineMediaGalleryItem["media"]>
  ): TimelineMediaGalleryItem => ({
    event_id: "$synthetic-media",
    room_id: "!room:example.invalid",
    sender: null,
    timestamp_ms: 0,
    media: {
      kind: "Image",
      filename: "synthetic_photo.png",
      source: {
        mxc_uri: "mxc://example.invalid/synthetic",
        encrypted: false,
        encryption_version: null
      },
      mimetype: "image/png",
      size: 2 * 1024,
      width: null,
      height: null,
      thumbnail: null,
      ...media
    }
  });

  const headerText = (media: Partial<TimelineMediaGalleryItem["media"]>): string | null => {
    render(
      <MediaViewer
        index={0}
        items={[mediaViewerEntryFromGallery(galleryItem(media))]}
        mediaDownloads={{}}
        onClose={vi.fn()}
        onSelectIndex={vi.fn()}
      />
    );
    return document.querySelector(".media-viewer-header p")?.textContent ?? null;
  };

  // #1784: the header shows the size, never the raw MIME type or media kind.
  it("omits the MIME type from the header", () => {
    render(
      <MediaViewer
        index={0}
        items={[
          mediaViewerEntryFromGallery(
            galleryItem({
              kind: "File",
              filename: "synthetic_report.xlsx",
              mimetype: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
              size: 9 * 1024
            })
          )
        ]}
        mediaDownloads={{}}
        onClose={vi.fn()}
        onSelectIndex={vi.fn()}
      />
    );

    const header = document.querySelector(".media-viewer-header p");
    expect(header?.textContent).toBe("9 KB");
  });

  // #1792: an image shows its pixel dimensions next to the size. The timeline
  // and the gallery now share one viewer (#1810), so this is the only header.
  it("shows size and dimensions for an image", () => {
    expect(
      headerText({ kind: "Image", size: 2 * 1024, width: 640, height: 480 })
    ).toBe("2 KB · 640x480");
  });

  // #1792: a video shows its pixel dimensions the same way.
  it("shows size and dimensions for a video", () => {
    expect(
      headerText({ kind: "Video", size: 5 * 1024 * 1024, width: 1920, height: 1080 })
    ).toBe("5.0 MB · 1920x1080");
  });

  // #1792: unknown dimensions stay out of the header instead of rendering
  // `0x0` or `nullxnull`.
  it("omits missing and zero dimensions for an image", () => {
    expect(headerText({ kind: "Image", size: 1024, width: null, height: null })).toBe(
      "1 KB"
    );
    cleanup();
    expect(headerText({ kind: "Video", size: 1024, width: 0, height: 0 })).toBe("1 KB");
  });

  // #1792: dimensions belong to images and videos; #1784 keeps every other
  // kind at the size only even when the event declares a width and height.
  it.each(["File", "Audio"] as const)(
    "keeps a %s at the size only when dimensions are declared",
    (kind) => {
      expect(headerText({ kind, size: 2048, width: 640, height: 480 })).toBe("2 KB");
    }
  );
});

describe("room media gallery older media (#1969)", () => {
  function image(eventId: string): TimelineMediaGalleryItem {
    return {
      event_id: eventId,
      room_id: "!room:example.invalid",
      sender: null,
      timestamp_ms: 1_800_000_000_000,
      media: {
        kind: "Image",
        filename: "fixture.png",
        source: { mxc_uri: "mxc://example.invalid/m", encrypted: false, encryption_version: null },
        mimetype: "image/png",
        size: 10,
        width: 10,
        height: 10,
        thumbnail: null
      }
    };
  }

  it("pages older media while the gallery end is in view and Rust reports idle", () => {
    const onLoadOlder = vi.fn();
    render(
      <RoomMediaGallery
        items={[image("$a:example.invalid")]}
        mediaDownloads={{}}
        onOpenItem={() => undefined}
        pagination="idle"
        onLoadOlder={onLoadOlder}
      />
    );
    expect(onLoadOlder).toHaveBeenCalledTimes(1);
  });

  it("waits while a page is in flight and stops at the room start", () => {
    const onLoadOlder = vi.fn();
    const { rerender } = render(
      <RoomMediaGallery
        items={[image("$a:example.invalid")]}
        mediaDownloads={{}}
        onOpenItem={() => undefined}
        pagination="loading"
        onLoadOlder={onLoadOlder}
      />
    );
    expect(screen.getByText("Loading older media…")).toBeTruthy();
    rerender(
      <RoomMediaGallery
        items={[image("$a:example.invalid")]}
        mediaDownloads={{}}
        onOpenItem={() => undefined}
        pagination="reachedStart"
        onLoadOlder={onLoadOlder}
      />
    );
    expect(onLoadOlder).not.toHaveBeenCalled();
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("offers a retry after a failed page instead of retrying on its own", () => {
    const onLoadOlder = vi.fn();
    render(
      <RoomMediaGallery
        items={[]}
        mediaDownloads={{}}
        onOpenItem={() => undefined}
        pagination="failed"
        onLoadOlder={onLoadOlder}
      />
    );
    expect(onLoadOlder).not.toHaveBeenCalled();
    expect(screen.queryByText("No media in this room")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(onLoadOlder).toHaveBeenCalledTimes(1);
  });

  it("says the room has no media only once the room start is reached", () => {
    render(
      <RoomMediaGallery
        items={[]}
        mediaDownloads={{}}
        onOpenItem={() => undefined}
        pagination="reachedStart"
        onLoadOlder={() => undefined}
      />
    );
    expect(screen.getByText("No media in this room")).toBeTruthy();
  });
});
