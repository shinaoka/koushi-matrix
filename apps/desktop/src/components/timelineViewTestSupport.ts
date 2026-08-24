import { vi } from "vitest";

import {
  roomTimelineKey,
  type TimelineItem,
  type TimelineReadStateSync
} from "../domain/coreEvents";
import type { TimelineTransport } from "./TimelineView";

export const KEY = roomTimelineKey("@alice:example.invalid", "!room:example.invalid");

export function message(eventId: string, body: string): TimelineItem {
  return {
    id: { Event: { event_id: eventId } },
    sender: "@bob:example.invalid",
    body,
    timestamp_ms: 1_800_000_000_000,
    in_reply_to_event_id: null,
    thread_root: null,
    thread_summary: null,
    can_react: true,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false,
    reactions: [],
    actions: {
      can_copy: false,
      can_forward: false,
      can_reply: true,
      can_permalink: false,
      can_view_source: false
    }
  };
}

export function imageMessage(eventId: string, encrypted = false): TimelineItem {
  return {
    ...message(eventId, "Image body"),
    media: {
      kind: "Image",
      filename: "photo.png",
      source: {
        mxc_uri: "mxc://example.invalid/photo",
        encrypted,
        encryption_version: encrypted ? "v2" : null
      },
      mimetype: "image/png",
      size: 416_768,
      width: 2048,
      height: 1188,
      thumbnail: null
    }
  };
}

export function fileMessage(eventId: string): TimelineItem {
  return {
    ...message(eventId, "File body"),
    media: {
      kind: "File",
      filename: "notes.pdf",
      source: {
        mxc_uri: "mxc://example.invalid/notes",
        encrypted: false,
        encryption_version: null
      },
      mimetype: "application/pdf",
      size: 12_288,
      width: null,
      height: null,
      thumbnail: null
    }
  };
}

export function navigationSnapshot(overrides: Partial<{
  read_marker_event_id: string | null;
  read_marker_display_event_id: string | null;
  first_unread_event_id: string | null;
  unread_event_count: number;
  unread_position: "none" | "aboveViewport" | "insideViewport" | "belowViewport" | "unknown";
  newer_event_count: number;
  can_jump_to_bottom: boolean;
  local_viewed_event_id: string | null;
  server_confirmed_read_event_id: string | null;
  read_state_sync: TimelineReadStateSync;
}> = {}) {
  return {
    read_marker_event_id: null,
    read_marker_display_event_id: null,
    first_unread_event_id: null,
    unread_event_count: 0,
    unread_position: "none" as const,
    newer_event_count: 0,
    can_jump_to_bottom: false,
    local_viewed_event_id: null,
    server_confirmed_read_event_id: null,
    read_state_sync: "synced" as const,
    ...overrides
  };
}

export function baseTransport(
  overrides: Partial<TimelineTransport>
): TimelineTransport {
  return {
    listenCoreEvents: () => () => undefined,
    paginateBackwards: async () => undefined,
    sendReaction: async () => undefined,
    retrySend: async () => undefined,
    cancelSend: async () => undefined,
    redactReaction: async () => undefined,
    sendReadReceipt: async () => undefined,
    setFullyRead: async () => undefined,
    setTyping: async () => undefined,
    editMessage: async () => undefined,
    redactMessage: async () => undefined,
    pinEvent: async () => undefined,
    unpinEvent: async () => undefined,
    downloadMedia: async () => undefined,
    downloadAvatarThumbnail: async () => undefined,
    loadMessageSource: async () => undefined,
    requestRoomKey: async () => undefined,
    forwardMessage: async () => undefined,
    loadLinkPreviews: async () => undefined,
    hideLinkPreview: async () => undefined,
    updateScrollAnchor: async () => undefined,
    ...overrides
  };
}

export function mockTimelineRects(
  rects: Record<string, { top: number; height: number }>,
  container: { top?: number; height?: number } = {},
  scrollContainerRef?: { current: HTMLElement | null }
) {
  return vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement
  ) {
    const eventId =
      this.getAttribute("data-event-id") ??
      this.getAttribute("data-frame-item-id") ??
      this.getAttribute("data-item-id") ??
      this.getAttribute("data-testid");
    const testId = this.getAttribute("data-testid");
    const scrollTop = scrollContainerRef?.current?.scrollTop ?? 0;
    const top =
      testId === "timeline-view"
        ? container.top ?? 0
        : (rects[eventId ?? ""]?.top ?? 0) - scrollTop;
    const height =
      testId === "timeline-view"
        ? container.height ?? 600
        : rects[eventId ?? ""]?.height ?? 0;
    const bottom = top + height;
    return {
      x: 0,
      y: top,
      top,
      left: 0,
      right: 0,
      width: 0,
      height,
      bottom,
      toJSON: () => ({})
    } as DOMRect;
  });
}
