import { beforeEach, describe, expect, test } from "vitest";

import {
  getTimelineTransportStats,
  recordTimelineEventReceived,
  recordTimelineInitialItems,
  recordTimelineKeyMismatch,
  resetTimelineTransportStats
} from "./timelineTransportStats";

describe("timeline transport stats", () => {
  beforeEach(() => {
    resetTimelineTransportStats();
  });

  test("starts at zero", () => {
    expect(getTimelineTransportStats()).toEqual({
      received: 0,
      keyMismatchDropped: 0,
      initialItemsApplied: 0,
      lastInitialItemsCount: 0
    });
  });

  test("counts received events, key-mismatch drops, and applied initial items", () => {
    recordTimelineEventReceived();
    recordTimelineEventReceived();
    recordTimelineKeyMismatch();
    recordTimelineInitialItems(42);

    expect(getTimelineTransportStats()).toEqual({
      received: 2,
      keyMismatchDropped: 1,
      initialItemsApplied: 1,
      lastInitialItemsCount: 42
    });
  });

  test("keeps the most recent initial-items count", () => {
    recordTimelineInitialItems(10);
    recordTimelineInitialItems(0);

    const stats = getTimelineTransportStats();
    expect(stats.initialItemsApplied).toBe(2);
    expect(stats.lastInitialItemsCount).toBe(0);
  });
});
