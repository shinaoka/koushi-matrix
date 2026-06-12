import { describe, expect, test } from "vitest";

import {
  restoreTimelineAnchor,
  timelinePaginationAnchorEventId
} from "./timelineAnchor";
import type { TimelineMessage } from "./types";

describe("timeline pagination anchors", () => {
  test("captures the current top event and scrolls it back after prepending history", () => {
    const anchor = timelinePaginationAnchorEventId([
      timelineMessage('$event"top'),
      timelineMessage("$event-next")
    ]);
    const calls: unknown[] = [];

    const restored = restoreTimelineAnchor(
      {
        querySelector(selector) {
          calls.push(selector);
          return {
            scrollIntoView(options) {
              calls.push(options);
            }
          };
        }
      },
      anchor
    );

    expect(restored).toBe(true);
    expect(calls).toEqual([
      '[data-event-id="$event\\"top"]',
      { block: "start" }
    ]);
  });

  test("does not query the document when there is no pagination anchor", () => {
    let queried = false;

    expect(timelinePaginationAnchorEventId([])).toBeNull();
    expect(
      restoreTimelineAnchor(
        {
          querySelector() {
            queried = true;
            return null;
          }
        },
        null
      )
    ).toBe(false);
    expect(queried).toBe(false);
  });
});

function timelineMessage(eventId: string): TimelineMessage {
  return {
    attachment_filename: null,
    body: "Synthetic body",
    event_id: eventId,
    reply_count: 0,
    room_id: "!room:example.invalid",
    sender: "@user:example.invalid",
    timestamp_ms: 1_820_000_000_000
  };
}
