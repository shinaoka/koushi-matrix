import type { TimelineGapId } from "../../domain/coreEvents";

/**
 * #1889: geometry for the vertical timeline list, shared by the scroll anchor
 * capture and the viewport observation so both read the same rows once.
 *
 * Every read here is a forced layout, so the pass is bounded by the viewport:
 * the visible run is found by binary search over the DOM-ordered rows rather
 * than by measuring every mounted row.
 */

const CANONICAL_UNSIGNED_DECIMAL = /^(?:0|[1-9][0-9]*)$/;

type TimelineEventIdentity = "content" | "activity";

/** Box of one scrolling viewport, as the vertical list sees it. */
export interface ViewportBox {
  top: number;
  bottom: number;
}

/** Inclusive index range of the DOM-ordered nodes that intersect the viewport. */
export interface VisibleNodeSpan {
  start: number;
  end: number;
}

/** What the viewport observation and the room scroll anchor both need. */
export interface TimelineViewportFacts {
  firstVisibleEventId: string | null;
  lastVisibleEventId: string | null;
  /** Rounded bottom offset of the last visible row, for a `bottom` anchor. */
  lastVisibleBottomOffset: number | null;
  visibleGapIds: TimelineGapId[];
}

const ROW_SELECTOR = "[data-activity-event-id]";
const GAP_SELECTOR = "[data-gap-topology-revision][data-gap-ordinal]";

/**
 * The viewport's own box, read once per pass. `clientHeight` wins when present
 * so a scrolled container is measured by its scrollport rather than by its own
 * (whole-content) rect; `.timeline-view` carries no border, so this is the same
 * edge the border box gives.
 */
export function timelineViewportBox(container: HTMLElement): ViewportBox {
  const rect = container.getBoundingClientRect();
  const top = rect.top;
  return { top, bottom: top + (container.clientHeight || rect.height) };
}

function intersects(box: ViewportBox, rect: { top: number; bottom: number }): boolean {
  return rect.bottom > box.top && rect.top < box.bottom;
}

/**
 * #1889: rows are laid out in DOM order, so the visible run is contiguous and
 * binary-searching it reads log N rows instead of all N.
 *
 * shortcut: the search assumes bottoms never decrease down the list. A row
 * moved out of order (the latest-reply projection) breaks that, so the span and
 * its neighbours are verified and the exact scan runs when the check fails; a
 * list short enough to scan for free is always scanned exactly. Upgrade to an
 * exact index if a layout ever displaces two or more rows without the
 * neighbour check noticing.
 */
const EXACT_SCAN_MAX_ROWS = 32;

export function visibleNodeSpan(
  box: ViewportBox,
  nodes: readonly HTMLElement[]
): VisibleNodeSpan | null {
  const count = nodes.length;
  if (count <= EXACT_SCAN_MAX_ROWS) {
    return linearVisibleNodeSpan(box, nodes);
  }
  let low = 0;
  let high = count - 1;
  let start = count;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (nodes[middle].getBoundingClientRect().bottom > box.top) {
      start = middle;
      high = middle - 1;
    } else {
      low = middle + 1;
    }
  }
  if (start >= count) {
    return null;
  }
  let end = start - 1;
  for (let index = start; index < count; index += 1) {
    if (nodes[index].getBoundingClientRect().top >= box.bottom) {
      break;
    }
    end = index;
  }
  if (end < start) {
    // The candidate sits wholly below the viewport, so the list is not in
    // layout order; the exact scan may still find a visible row.
    return linearVisibleNodeSpan(box, nodes);
  }
  let previousBottom = -Infinity;
  for (let index = Math.max(0, start - 1); index <= Math.min(count - 1, end + 1); index += 1) {
    const bottom = nodes[index].getBoundingClientRect().bottom;
    if (bottom < previousBottom) {
      return linearVisibleNodeSpan(box, nodes);
    }
    previousBottom = bottom;
  }
  return { start, end };
}

/** The pre-#1889 full scan, kept as the oracle for a non-monotone list. */
export function linearVisibleNodeSpan(
  box: ViewportBox,
  nodes: readonly HTMLElement[]
): VisibleNodeSpan | null {
  let start = -1;
  let end = -1;
  for (const [index, node] of nodes.entries()) {
    if (!intersects(box, node.getBoundingClientRect())) {
      continue;
    }
    if (start === -1) {
      start = index;
    }
    end = index;
  }
  return start === -1 ? null : { start, end };
}

/**
 * One pass over the visible rows and gaps. The scroll anchor's room capture and
 * the viewport observation used to walk the whole list separately for the same
 * answer; both read this.
 */
export function readTimelineViewportFacts(container: HTMLElement): TimelineViewportFacts {
  const box = timelineViewportBox(container);
  const nodes = Array.from(container.querySelectorAll<HTMLElement>(ROW_SELECTOR));
  const span = visibleNodeSpan(box, nodes);
  let firstVisibleEventId: string | null = null;
  let lastVisibleEventId: string | null = null;
  let lastVisibleBottomOffset: number | null = null;
  if (span) {
    for (let index = span.start; index <= span.end; index += 1) {
      const node = nodes[index];
      const eventId = eventIdForTimelineIdentity(node, "activity");
      if (!eventId) {
        continue;
      }
      if (firstVisibleEventId === null) {
        firstVisibleEventId = eventId;
      }
      lastVisibleEventId = eventId;
      lastVisibleBottomOffset = Math.round(node.getBoundingClientRect().bottom - box.bottom);
    }
  }
  return {
    firstVisibleEventId,
    lastVisibleEventId,
    lastVisibleBottomOffset,
    visibleGapIds: visibleGapIds(container, box)
  };
}

function visibleGapIds(container: HTMLElement, box: ViewportBox): TimelineGapId[] {
  const gapNodes = Array.from(container.querySelectorAll<HTMLElement>(GAP_SELECTOR));
  const span = visibleNodeSpan(box, gapNodes);
  if (!span) {
    return [];
  }
  const visibleGapIds: TimelineGapId[] = [];
  const seenGapIds = new Set<string>();
  for (let index = span.start; index <= span.end; index += 1) {
    const node = gapNodes[index];
    const topologyRevision = node.dataset["gapTopologyRevision"];
    const ordinal = parseCanonicalU32(node.dataset["gapOrdinal"]);
    if (!isCanonicalUnsignedDecimal(topologyRevision) || ordinal === null) {
      continue;
    }
    const signature = `${topologyRevision}\u0000${ordinal}`;
    if (seenGapIds.has(signature)) {
      continue;
    }
    seenGapIds.add(signature);
    visibleGapIds.push({ topology_revision: topologyRevision, ordinal });
  }
  return visibleGapIds;
}

function isCanonicalUnsignedDecimal(value: string | undefined): value is string {
  return value !== undefined && CANONICAL_UNSIGNED_DECIMAL.test(value);
}

function parseCanonicalU32(value: string | undefined): number | null {
  if (!isCanonicalUnsignedDecimal(value)) {
    return null;
  }
  let parsed = 0;
  for (const digit of value) {
    parsed = parsed * 10 + digit.charCodeAt(0) - "0".charCodeAt(0);
    if (parsed > MAX_U32) {
      return null;
    }
  }
  return parsed;
}

const MAX_U32 = 4_294_967_295;

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

export function timelineEventIdentitySelector(
  identity: TimelineEventIdentity,
  eventId: string
): string {
  return `[${timelineEventIdentityAttribute(identity)}="${cssEscape(eventId)}"]`;
}

export function cssEscape(value: string): string {
  return value.replace(/["\\]/g, "\\$&");
}
