import { beforeEach, describe, expect, test } from "vitest";

import {
  getTimelineTransportStats,
  recordTimelineEventReceived,
  recordTimelineInitialItems,
  recordTimelineKeyMismatch,
  recordTimelineResync,
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
      lastInitialItemsCount: 0,
      resync: 0
    });
  });

  test("counts received events, key-mismatch drops, applied initial items, and resyncs", () => {
    recordTimelineEventReceived();
    recordTimelineEventReceived();
    recordTimelineKeyMismatch();
    recordTimelineInitialItems(42);
    recordTimelineResync();

    expect(getTimelineTransportStats()).toEqual({
      received: 2,
      keyMismatchDropped: 1,
      initialItemsApplied: 1,
      lastInitialItemsCount: 42,
      resync: 1
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
