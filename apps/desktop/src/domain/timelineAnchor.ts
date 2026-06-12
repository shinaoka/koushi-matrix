import type { TimelineMessage } from "./types";

export interface TimelineAnchorRoot {
  querySelector(selector: string): TimelineAnchorElement | null;
}

export interface TimelineAnchorElement {
  scrollIntoView(options?: ScrollIntoViewOptions): void;
}

export function timelinePaginationAnchorEventId(
  messages: TimelineMessage[]
): string | null {
  return messages[0]?.event_id ?? null;
}

export function restoreTimelineAnchor(
  root: TimelineAnchorRoot,
  eventId: string | null
): boolean {
  if (!eventId) {
    return false;
  }

  const target = root.querySelector(`[data-event-id="${cssEscape(eventId)}"]`);
  if (!target) {
    return false;
  }

  target.scrollIntoView({ block: "start" });
  return true;
}

function cssEscape(value: string): string {
  return value.replace(/["\\]/g, "\\$&");
}
