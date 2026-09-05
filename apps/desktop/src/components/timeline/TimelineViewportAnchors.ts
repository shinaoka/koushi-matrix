import type { TimelineItem, TimelineKey } from "../../domain/coreEvents";
import type { TimelineScrollAnchor } from "../../domain/types";
import { timelineStoreKeyId } from "../../domain/timelineStore";

// ---------------------------------------------------------------------------
// Scroll anchor
// ---------------------------------------------------------------------------

export interface ScrollAnchor {
  /** Stable item id of the anchor element. */
  itemId: string;
  /** Pixel offset of the anchor element top from the container's top edge. */
  offsetTop: number;
}

type ScrollAnchorCaptureOptions = {
  /**
   * Lets a caller exclude a row whose presentation position is itself being
   * changed. This is essential for the latest-reply projection: restoring a
   * moved root would preserve the wrong visual intent.
   */
  isEligible?: (node: HTMLElement) => boolean;
};

type TimelineEventIdentity = "content" | "activity";

/** Capture the first eligible visible item as the anchor (id + pixel offset). */
export function captureAnchor(
  container: HTMLElement,
  options: ScrollAnchorCaptureOptions = {}
): ScrollAnchor | null {
  const containerRect = container.getBoundingClientRect();
  const containerTop = containerRect.top;
  const containerBottom = containerTop + (container.clientHeight || containerRect.height);
  const nodes = container.querySelectorAll<HTMLElement>("[data-item-id]");
  for (const node of nodes) {
    if (options.isEligible && !options.isEligible(node)) {
      continue;
    }
    const rect = node.getBoundingClientRect();
    if (rect.bottom > containerTop && rect.top < containerBottom) {
      return {
        itemId: node.dataset["itemId"] ?? "",
        offsetTop: rect.top - containerTop
      };
    }
  }
  return null;
}

/**
 * A root shown at a reply's activity position is not a stable free-scroll
 * anchor: the next reply/redaction can relocate it again. Prefer a normal
 * material row and leave the anchor empty when no such row is mounted.
 */
export function captureFreeScrollAnchor(container: HTMLElement): ScrollAnchor | null {
  return captureAnchor(container, {
    isEligible: (node) => {
      const contentEventId = node.dataset["contentEventId"] ?? null;
      const activityEventId = node.dataset["activityEventId"] ?? null;
      return contentEventId === null || activityEventId === null || contentEventId === activityEventId;
    }
  });
}

/** Measure the anchor's local delta; only the viewport transaction may apply it. */
export function measureAnchorDelta(
  container: HTMLElement,
  anchor: ScrollAnchor
): number | null {
  const node = container.querySelector<HTMLElement>(
    `[data-item-id="${cssEscape(anchor.itemId)}"]`
  );
  if (!node) {
    return null;
  }
  const containerTop = container.getBoundingClientRect().top;
  const currentOffset = node.getBoundingClientRect().top - containerTop;
  return currentOffset - anchor.offsetTop;
}

type CapturedTimelineScrollAnchor = {
  event_id: string;
  edge: "bottom";
  offset_px: number;
};

type TimelineViewportSessionMemory =
  | { mode: "live-edge" }
  | { mode: "anchor"; anchor: TimelineScrollAnchor };

export type TimelineSessionAnchorAgeBucket = "none" | "fresh" | "recent" | "stale";

export function timelineSessionAnchorAgeBucket(
  anchor: TimelineScrollAnchor | null,
  nowMs = Date.now()
): TimelineSessionAnchorAgeBucket {
  if (!anchor) {
    return "none";
  }
  const ageMs = Math.max(0, nowMs - anchor.updated_at_ms);
  if (ageMs < 30_000) {
    return "fresh";
  }
  if (ageMs < 5 * 60_000) {
    return "recent";
  }
  return "stale";
}

// UI-only memory for this JavaScript session. It intentionally resets on app
// restart: first entry into a room starts at live edge, while room switches
// during the same process can restore the user's last free-scroll anchor.
export const timelineViewportSessionMemory = new Map<string, TimelineViewportSessionMemory>();

export function clearTimelineViewportSessionMemoryForTests(): void {
  timelineViewportSessionMemory.clear();
}

export function setTimelineViewportSessionAnchorForTests(
  timelineKey: TimelineKey,
  anchor: TimelineScrollAnchor
): void {
  timelineViewportSessionMemory.set(timelineStoreKeyId(timelineKey), {
    mode: "anchor",
    anchor
  });
}

export function captureRoomScrollAnchor(container: HTMLElement): CapturedTimelineScrollAnchor | null {
  const containerRect = container.getBoundingClientRect();
  const nodes = container.querySelectorAll<HTMLElement>("[data-activity-event-id]");
  let captured: CapturedTimelineScrollAnchor | null = null;
  for (const node of nodes) {
    const rect = node.getBoundingClientRect();
    if (rect.bottom <= containerRect.top || rect.top >= containerRect.bottom) {
      continue;
    }
    const eventId = eventIdForTimelineIdentity(node, "activity");
    if (!eventId) {
      continue;
    }
    captured = {
      event_id: eventId,
      edge: "bottom",
      offset_px: Math.round(rect.bottom - containerRect.bottom)
    };
  }
  return captured;
}

export function restoreRoomScrollAnchor(container: HTMLElement, anchor: TimelineScrollAnchor): boolean {
  const node = findRoomScrollAnchorNode(container, anchor);
  if (!node) {
    return false;
  }
  const currentOffset = currentRoomScrollAnchorOffset(container, node, anchor);
  container.scrollTop += currentOffset - anchor.offset_px;
  return true;
}

function currentRoomScrollAnchorOffset(
  container: HTMLElement,
  node: HTMLElement,
  anchor: TimelineScrollAnchor
): number {
  const containerRect = container.getBoundingClientRect();
  const nodeRect = node.getBoundingClientRect();
  return (anchor.edge ?? "top") === "bottom"
    ? nodeRect.bottom - containerRect.bottom
    : nodeRect.top - containerRect.top;
}

function findRoomScrollAnchorNode(
  container: HTMLElement,
  anchor: TimelineScrollAnchor
): HTMLElement | null {
  return findTimelineEventNode(container, "activity", anchor.event_id);
}

export function roomScrollAnchorSignature(roomId: string, anchor: TimelineScrollAnchor): string {
  return [
    roomId,
    anchor.event_id,
    anchor.edge ?? "top",
    anchor.offset_px,
    anchor.updated_at_ms
  ].join("\u0000");
}

export function roomScrollAnchorStableSignature(
  roomId: string,
  anchor: Pick<TimelineScrollAnchor, "event_id" | "edge" | "offset_px">
): string {
  return [
    roomId,
    anchor.event_id,
    anchor.edge ?? "top",
    anchor.offset_px
  ].join("\u0000");
}

export function canonicalTimelineContainsActivityEventId(
  items: readonly TimelineItem[],
  eventId: string
): boolean {
  return items.some(
    (item) => "Event" in item.id && item.id.Event.event_id === eventId
  );
}

function timelineEventIdentityAttribute(identity: TimelineEventIdentity): string {
  return identity === "activity" ? "data-activity-event-id" : "data-content-event-id";
}

export function eventIdForTimelineIdentity(
  node: HTMLElement,
  identity: TimelineEventIdentity
): string | null {
  return identity === "activity"
    ? node.dataset["activityEventId"] ?? null
    : node.dataset["contentEventId"] ?? null;
}

export function findTimelineEventNode(
  container: HTMLElement,
  identity: TimelineEventIdentity,
  eventId: string
): HTMLElement | null {
  return container.querySelector<HTMLElement>(
    `[${timelineEventIdentityAttribute(identity)}="${cssEscape(eventId)}"]`
  );
}

function cssEscape(value: string): string {
  return value.replace(/["\\]/g, "\\$&");
}
