// @vitest-environment jsdom

import { describe, expect, it, vi } from "vitest";

import type { TimelineItem } from "../../domain/coreEvents";
import {
  buildTimelineHeightModel,
  calculateTimelineVirtualRange,
  scheduleTimelineFrame,
  TIMELINE_ESTIMATED_ITEM_HEIGHT_PX,
  TIMELINE_VIRTUALIZATION_THRESHOLD
} from "./TimelineViewportVirtualization";

describe("timeline viewport virtualization", () => {
  it("keeps a 100,000-event history bounded to one rendered window", () => {
    const rows = Array.from({ length: 100_000 }, (_, index) => ({
      row_id: `row-${index}`,
      item: {} as TimelineItem,
      content_event_id: null,
      activity_event_id: null,
      gap_id: null,
      content_timestamp_ms: null,
      display_timestamp_ms: null,
      kind: "event" as const
    }));
    const model = buildTimelineHeightModel(
      rows,
      new Map(),
      TIMELINE_ESTIMATED_ITEM_HEIGHT_PX
    );
    const range = calculateTimelineVirtualRange({
      visibleItemsLength: rows.length,
      metrics: { scrollTop: 0, clientHeight: 600, listOffsetTop: 0 },
      model
    });

    expect(rows.length).toBe(100_000);
    expect(range.virtualized).toBe(true);
    expect(range.endIndex - range.startIndex).toBeLessThanOrEqual(
      TIMELINE_VIRTUALIZATION_THRESHOLD
    );
    expect(range.paddingBottom).toBeGreaterThan(0);
  });
});

describe("scheduleTimelineFrame teardown", () => {
  it("uses captured browser capabilities after window teardown", () => {
    const windowDescriptor = Object.getOwnPropertyDescriptor(globalThis, "window");
    if (!windowDescriptor) {
      throw new Error("expected jsdom window descriptor");
    }

    let firstRafHandler: FrameRequestCallback | undefined;
    let secondRafHandler: FrameRequestCallback | undefined;
    let firstTimeoutHandler: (() => void) | undefined;
    let secondTimeoutHandler: (() => void) | undefined;
    const requestAnimationFrame = vi
      .spyOn(window, "requestAnimationFrame")
      .mockImplementationOnce((handler) => {
        firstRafHandler = handler;
        return 1;
      })
      .mockImplementationOnce((handler) => {
        secondRafHandler = handler;
        return 2;
      });
    const cancelAnimationFrame = vi.spyOn(window, "cancelAnimationFrame");
    const setTimeout = vi
      .spyOn(window, "setTimeout")
      .mockImplementationOnce((handler) => {
        firstTimeoutHandler = handler as () => void;
        return 3 as unknown as ReturnType<typeof window.setTimeout>;
      })
      .mockImplementationOnce((handler) => {
        secondTimeoutHandler = handler as () => void;
        return 4 as unknown as ReturnType<typeof window.setTimeout>;
      });
    const clearTimeout = vi.spyOn(window, "clearTimeout");
    const performanceNow = vi.spyOn(window.performance, "now").mockReturnValue(1234);
    const firstCallback = vi.fn();
    const secondCallback = vi.fn();

    try {
      const firstHandle = scheduleTimelineFrame(firstCallback);
      const secondHandle = scheduleTimelineFrame(secondCallback);
      expect(requestAnimationFrame).toHaveBeenCalledTimes(2);
      expect(setTimeout).toHaveBeenCalledTimes(2);
      expect(firstRafHandler).toBeDefined();
      expect(secondRafHandler).toBeDefined();
      expect(firstTimeoutHandler).toBeDefined();
      expect(secondTimeoutHandler).toBeDefined();

      expect(Reflect.deleteProperty(globalThis, "window")).toBe(true);

      expect(() => firstTimeoutHandler?.()).not.toThrow();
      expect(firstCallback).toHaveBeenCalledTimes(1);
      expect(firstCallback).toHaveBeenCalledWith(1234);
      expect(cancelAnimationFrame).toHaveBeenCalledWith(1);
      expect(clearTimeout).toHaveBeenCalledWith(3);
      expect(performanceNow).toHaveBeenCalledTimes(1);
      expect(() => firstHandle.cancel()).not.toThrow();
      expect(cancelAnimationFrame).toHaveBeenCalledTimes(1);
      expect(clearTimeout).toHaveBeenCalledTimes(1);

      expect(() => secondHandle.cancel()).not.toThrow();
      secondHandle.cancel();
      expect(secondCallback).not.toHaveBeenCalled();
      expect(cancelAnimationFrame).toHaveBeenCalledWith(2);
      expect(clearTimeout).toHaveBeenCalledWith(4);
      expect(cancelAnimationFrame).toHaveBeenCalledTimes(2);
      expect(clearTimeout).toHaveBeenCalledTimes(2);
    } finally {
      Object.defineProperty(globalThis, "window", windowDescriptor);
      vi.restoreAllMocks();
    }
  });
});
