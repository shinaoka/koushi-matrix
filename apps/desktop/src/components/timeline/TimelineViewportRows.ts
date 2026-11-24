import type { TimelineGapId } from "../../domain/coreEvents";
import {
  timelineVisibleItemRange,
  type TimelineViewportMetrics,
  type TimelineViewportModel
} from "./TimelineViewportVirtualization";

/**
 * #1890 item 5: geometry for the vertical timeline list.
 *
 * Every question here ("which rows are on screen?", "where is this row?") is
 * answered from the committed height model and the container metrics the caller
 * already read. #1889 answered them by binary-searching the mounted rows, which
 * was bounded by the viewport but still one rect per probe and one per visible
 * row; the model makes them free of row reads altogether, so a scroll frame can
 * no longer be made to pay for a forced layout per row by anything that dirties
 * the layout first.
 *
 * The single container read that remains is `timelineViewportBox`, used by the
 * anchor capture for the coordinate space its rect is reported in.
 */

type TimelineEventIdentity = "content" | "activity";

/** Box of one scrolling viewport, as the vertical list sees it. */
export interface ViewportBox {
  top: number;
  bottom: number;
}

/** What the viewport observation and the room scroll anchor both need. */
export interface TimelineViewportFacts {
  firstVisibleEventId: string | null;
  lastVisibleEventId: string | null;
  visibleGapIds: TimelineGapId[];
}

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

/**
 * The container read an anchor capture needs, and the metrics that go with it.
 * `scrollTop` and `clientHeight` are read here rather than taken from a cached
 * frame, so a capture cannot lag the scroll it was taken on.
 */
export function timelineCaptureMetrics(
  container: HTMLElement,
  listOffsetTop: number
): { box: ViewportBox; metrics: TimelineViewportMetrics } {
  const box = timelineViewportBox(container);
  return {
    box,
    metrics: {
      scrollTop: container.scrollTop,
      clientHeight: box.bottom - box.top,
      listOffsetTop
    }
  };
}

/**
 * #1890 item 5: the visible range and the gap ids in it, from the height model.
 *
 * This is one observation pass, so it reads no layout at all: `view.metrics` is
 * the read the frame already made for its range commit, and the model's offsets
 * are the same ones its spacers were written from.
 */
export function timelineViewportFactsFromModel(
  view: TimelineViewportModel
): TimelineViewportFacts {
  const range = timelineVisibleItemRange(view.model, view.metrics, view.rows.length);
  let firstVisibleEventId: string | null = null;
  let lastVisibleEventId: string | null = null;
  const visibleGapIds: TimelineGapId[] = [];
  const seenGapIds = new Set<string>();
  for (let index = range.startIndex; index < range.endIndex; index += 1) {
    const row = view.rows[index];
    if (row === undefined) {
      continue;
    }
    if (row.gap_id !== null) {
      const signature = `${row.gap_id.topology_revision}\u0000${row.gap_id.ordinal}`;
      if (!seenGapIds.has(signature)) {
        seenGapIds.add(signature);
        visibleGapIds.push(row.gap_id);
      }
    }
    const eventId = row.activity_event_id;
    if (!eventId) {
      continue;
    }
    if (firstVisibleEventId === null) {
      firstVisibleEventId = eventId;
    }
    lastVisibleEventId = eventId;
  }
  return { firstVisibleEventId, lastVisibleEventId, visibleGapIds };
}

function timelineEventIdentityAttribute(identity: TimelineEventIdentity): string {
  return identity === "activity" ? "data-activity-event-id" : "data-content-event-id";
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
