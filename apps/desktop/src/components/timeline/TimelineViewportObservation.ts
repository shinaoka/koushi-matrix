import type { KeyboardEvent } from "react";
import type { TimelineGapId } from "../../domain/coreEvents";
import { readTimelineViewportFacts } from "./TimelineViewportRows";

export function visibleTimelineViewportFacts(container: HTMLElement): {
  firstVisibleEventId: string | null;
  lastVisibleEventId: string | null;
  visibleGapIds: TimelineGapId[];
} {
  const facts = readTimelineViewportFacts(container);
  return {
    firstVisibleEventId: facts.firstVisibleEventId,
    lastVisibleEventId: facts.lastVisibleEventId,
    visibleGapIds: facts.visibleGapIds
  };
}

export function isScrolledToBottom(container: HTMLElement): boolean {
  return (
    container.scrollHeight - container.clientHeight - container.scrollTop <=
    SCROLL_EDGE_TOLERANCE_PX
  );
}

export function scrollContainerToBottom(container: HTMLElement): void {
  container.scrollTop = container.scrollHeight - container.clientHeight;
}

export function timelineKeyShouldReleaseViewportIntent(event: KeyboardEvent<HTMLDivElement>): boolean {
  if (event.altKey || event.ctrlKey || event.metaKey) {
    return false;
  }
  // #1001: keys typed into an editor inside the timeline (inline edit) move
  // its caret, not the timeline, so they must not drop the live edge.
  if (keyTargetEditsText(event.target)) {
    return false;
  }
  switch (event.key) {
    case "ArrowDown":
    case "ArrowUp":
    case "End":
    case "Home":
    case "PageDown":
    case "PageUp":
    case " ":
      return true;
    default:
      return false;
  }
}

function keyTargetEditsText(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return (
    target.isContentEditable ||
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target.closest("[contenteditable='true'], [contenteditable='']") !== null
  );
}

/** Distance (px) from the top edge that triggers automatic backfill. */
const AUTO_BACKFILL_THRESHOLD_PX = 80;
const AUTO_BACKFILL_VIEWPORTS = 2;
export const SCROLL_EDGE_TOLERANCE_PX = 2;

export function timelineBackfillThreshold(clientHeight: number): number {
  return Math.max(
    AUTO_BACKFILL_THRESHOLD_PX,
    Math.max(0, clientHeight) * AUTO_BACKFILL_VIEWPORTS
  );
}

export const timelineBackfillThresholdForTests = timelineBackfillThreshold;
