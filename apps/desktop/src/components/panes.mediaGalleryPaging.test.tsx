// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { createElement } from "react";
import { afterEach, describe, expect, test, vi } from "vitest";

import { clearAppStoreSnapshot, setAppStoreSnapshot } from "../domain/appStore";
import type { DesktopSnapshot, TimelineMediaGalleryItem } from "../domain/types";
import { createTimelineStore } from "../domain/timelineStore";
import { readyDesktopSnapshotFixture } from "../test/desktopApiFixture";
import { TimelineStoreContext } from "./timelineStoreContext";
import { TimelinePane } from "./panes";
import type { TimelineTransport } from "./TimelineView";

const ROOM_ID = "!room-alpha:example.invalid";
const OLDEST = "$media-oldest:example.invalid";

type Pagination = DesktopSnapshot["state"]["ui"]["timeline"]["media_gallery_pagination"];

describe("timeline pane media viewer pagination (#1969)", () => {
  afterEach(() => {
    cleanup();
    clearAppStoreSnapshot();
    vi.restoreAllMocks();
  });

  // #1971: the viewer's near-oldest effect refires whenever the caller rebuilds
  // its closure, so a failed page would retry forever. Pagination must be idle
  // for the viewer to page on its own; a failure waits for the gallery's Retry.
  test("a failed page does not page again from the viewer", () => {
    const transport = pagingTransport();
    const { rerender } = renderPane(snapshotWith("failed"), transport);
    openViewerAtOldestImage();
    expect(transport.paginateMediaGallery).not.toHaveBeenCalled();
    // A live diff batch re-renders the pane with a fresh snapshot object, which
    // is what rebuilt the caller's closure on every page attempt before.
    setAppStoreSnapshot(snapshotWith("failed"));
    rerender(paneTree(snapshotWith("failed"), transport));
    expect(transport.paginateMediaGallery).not.toHaveBeenCalled();
  });

  test("an idle pagination pages from the oldest image", () => {
    const transport = pagingTransport();
    renderPane(snapshotWith("idle"), transport);
    fireEvent.click(screen.getByRole("button", { name: "Open media gallery" }));
    const paginate = vi.mocked(transport.paginateMediaGallery!);
    // The gallery's own end-of-list trigger pages too; the viewer's is what this
    // test isolates.
    paginate.mockClear();
    openOldestImage();
    expect(paginate).toHaveBeenCalledWith(ROOM_ID);
  });
});

function snapshotWith(pagination: Pagination): DesktopSnapshot {
  const snapshot = readyDesktopSnapshotFixture();
  snapshot.state.ui.timeline.room_id = ROOM_ID;
  snapshot.state.ui.timeline.media_gallery = [
    galleryImage("$media-newest:example.invalid", 1_800_000_003_000),
    galleryImage("$media-middle:example.invalid", 1_800_000_002_000),
    galleryImage(OLDEST, 1_800_000_001_000)
  ];
  snapshot.state.ui.timeline.media_gallery_pagination = pagination;
  return snapshot;
}

function galleryImage(eventId: string, timestampMs: number): TimelineMediaGalleryItem {
  const name = eventId.replace(/^\$|:example\.invalid$/g, "");
  return {
    event_id: eventId,
    room_id: ROOM_ID,
    sender: null,
    timestamp_ms: timestampMs,
    media: {
      kind: "Image",
      filename: `${name}.png`,
      source: { mxc_uri: `mxc://example.invalid/${name}`, encrypted: false, encryption_version: null },
      mimetype: "image/png",
      size: 10,
      width: 10,
      height: 10,
      thumbnail: null
    }
  };
}

function pagingTransport(): TimelineTransport {
  const noop = async () => undefined;
  return {
    listenCoreEvents: () => () => undefined,
    paginateBackwards: noop,
    sendReaction: noop,
    retrySend: noop,
    cancelSend: noop,
    redactReaction: noop,
    sendReadReceipt: noop,
    setFullyRead: noop,
    setTyping: noop,
    editMessage: noop,
    redactMessage: noop,
    pinEvent: noop,
    unpinEvent: noop,
    downloadMedia: noop,
    downloadAvatarThumbnail: noop,
    loadMessageSource: noop,
    requestRoomKey: noop,
    forwardMessage: noop,
    loadLinkPreviews: noop,
    hideLinkPreview: noop,
    observeViewport: noop,
    openAtTimestamp: noop,
    paginateMediaGallery: vi.fn(async () => undefined)
  };
}

function paneTree(snapshot: DesktopSnapshot, transport: TimelineTransport) {
  const noop = () => undefined;
  const store = createTimelineStore();
  return createElement(
    TimelineStoreContext.Provider,
    { value: { store, setStore: vi.fn() } },
    createElement(TimelinePane, {
      activeRoomName: "Alpha Room",
      composerDocument: snapshot.state.ui.timeline.composer.document,
      composerMode: { kind: "plain" },
      resolveComposerKeyAction: async () => "noop" as const,
      searchQuery: "",
      searchResults: [],
      showSearchResults: false,
      snapshot,
      timelineTransport: transport,
      onCancelReply: noop,
      onCancelScheduledSend: noop,
      onAttachFiles: noop,
      onClearUploadStaging: noop,
      onComposerMathModeChange: noop,
      onUpdateStagedUploadCaption: noop,
      onSelectStagedUploadOutput: noop,
      onSendStagedAttachments: noop,
      onLoadStagedUploadPreview: async () => [],
      onComposerDocumentChange: noop,
      onOpenContextMenu: noop,
      onOpenThread: noop,
      onRedactMessage: noop,
      onReply: noop,
      onRescheduleScheduledSend: noop,
      onResultSelect: noop,
      onScheduleSend: noop,
      onSendText: noop,
      onSetLocalUserAlias: noop,
      onUnpinPinnedEvent: noop,
      onOpenPeople: noop,
      onOpenThreads: noop,
      onToggleRoomInfo: noop
    })
  );
}

function renderPane(snapshot: DesktopSnapshot, transport: TimelineTransport) {
  setAppStoreSnapshot(snapshot);
  return render(paneTree(snapshot, transport));
}

function openViewerAtOldestImage(): void {
  fireEvent.click(screen.getByRole("button", { name: "Open media gallery" }));
  openOldestImage();
}

function openOldestImage(): void {
  fireEvent.click(screen.getByRole("button", { name: `Open ${OLDEST.replace(/^\$|:example\.invalid$/g, "")}.png` }));
  expect(screen.getByRole("dialog", { name: "Media viewer" })).toBeTruthy();
}
