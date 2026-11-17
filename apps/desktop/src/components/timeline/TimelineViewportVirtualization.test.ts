// @vitest-environment jsdom

import { describe, expect, it, vi } from "vitest";

import type { TimelineItem } from "../../domain/coreEvents";
import {
  buildTimelineHeightModel,
  calculateTimelineVirtualRange,
  scheduleTimelineFrame,
  TIMELINE_ESTIMATED_ITEM_HEIGHT_PX,
  TIMELINE_VIRTUAL_OVERSCAN_SCREENS
} from "./TimelineViewportVirtualization";

function rows(count: number) {
  return Array.from({ length: count }, (_, index) => ({
    row_id: `row-${index}`,
    item: {} as TimelineItem,
    content_event_id: null,
    activity_event_id: null,
    gap_id: null,
    content_timestamp_ms: null,
    display_timestamp_ms: null,
    kind: "event" as const
  }));
}

describe("timeline viewport virtualization", () => {
  it("bounds mounted rows for a room with 147 visible timeline items", () => {
    const timelineRows = rows(147);
    const model = buildTimelineHeightModel(
      timelineRows,
      new Map(),
      TIMELINE_ESTIMATED_ITEM_HEIGHT_PX
    );
    const range = calculateTimelineVirtualRange({
      visibleItemsLength: timelineRows.length,
      metrics: { scrollTop: model.totalHeight - 500, clientHeight: 500, listOffsetTop: 0 },
      model
    });

    expect(range.virtualized).toBe(true);
    expect(range.endIndex - range.startIndex).toBeLessThanOrEqual(80);
  });

  it("keeps a 100,000-event history bounded to one rendered window", () => {
    const timelineRows = rows(100_000);
    const model = buildTimelineHeightModel(
      timelineRows,
      new Map(),
      TIMELINE_ESTIMATED_ITEM_HEIGHT_PX
    );
    const range = calculateTimelineVirtualRange({
      visibleItemsLength: timelineRows.length,
      metrics: { scrollTop: 0, clientHeight: 600, listOffsetTop: 0 },
      model
    });

    expect(timelineRows.length).toBe(100_000);
    expect(range.virtualized).toBe(true);
    expect(range.endIndex - range.startIndex).toBeLessThanOrEqual(40);
    expect(range.paddingBottom).toBeGreaterThan(0);
  });

  it("mounts a 60-row room within one screen of overscan each side", () => {
    const timelineRows = rows(60);
    const model = buildTimelineHeightModel(
      timelineRows,
      new Map(),
      TIMELINE_ESTIMATED_ITEM_HEIGHT_PX
    );
    const range = calculateTimelineVirtualRange({
      visibleItemsLength: timelineRows.length,
      metrics: { scrollTop: model.totalHeight - 600, clientHeight: 600, listOffsetTop: 0 },
      model
    });

    expect(range.virtualized).toBe(true);
    // 60 rows of 72 px are 4,320 px. The tail of a 600 px viewport plus one
    // screen of overscan above it covers well under two thirds of the room.
    expect(range.endIndex - range.startIndex).toBeLessThan(60);
    expect(range.startIndex).toBeGreaterThan(0);
    expect(range.paddingTop).toBeGreaterThan(0);
    expect(range.paddingBottom).toBe(0);
  });

  it("sizes the overscan in pixels, not rows", () => {
    const timelineRows = rows(60);
    const heights = new Map(
      timelineRows.map((row, index) => [row.row_id, index % 5 === 0 ? 480 : 36])
    );
    const model = buildTimelineHeightModel(
      timelineRows,
      heights,
      TIMELINE_ESTIMATED_ITEM_HEIGHT_PX
    );
    const viewportHeight = 600;
    const overscanPx = viewportHeight * TIMELINE_VIRTUAL_OVERSCAN_SCREENS;
    const range = calculateTimelineVirtualRange({
      visibleItemsLength: timelineRows.length,
      metrics: {
        scrollTop: model.totalHeight - viewportHeight,
        clientHeight: viewportHeight,
        listOffsetTop: 0
      },
      model
    });
    const mountedAboveViewport =
      model.totalHeight - viewportHeight - model.offsets[range.startIndex]!;

    // A row-count overscan would stop short of a screen once tall rows are in
    // the window; a pixel overscan always reaches one screen back and no more.
    expect(range.endIndex).toBe(timelineRows.length);
    expect(mountedAboveViewport).toBeGreaterThanOrEqual(overscanPx);
    expect(mountedAboveViewport).toBeLessThan(overscanPx + 480);
  });

  it("keeps the tail mounted when the physical scroll outruns the model", () => {
    const timelineRows = rows(40);
    const model = buildTimelineHeightModel(
      timelineRows,
      new Map(),
      TIMELINE_ESTIMATED_ITEM_HEIGHT_PX
    );
    const range = calculateTimelineVirtualRange({
      visibleItemsLength: timelineRows.length,
      metrics: { scrollTop: 5_000, clientHeight: 600, listOffsetTop: 0 },
      model
    });
    const modelledBottom = calculateTimelineVirtualRange({
      visibleItemsLength: timelineRows.length,
      metrics: {
        scrollTop: model.totalHeight - 600,
        clientHeight: 600,
        listOffsetTop: 0
      },
      model
    });

    // Measured heights can lag a live layout. The window still covers the end
    // of the model instead of collapsing to its last row.
    expect(range).toEqual(modelledBottom);
    expect(range.endIndex).toBe(timelineRows.length);
    expect(range.paddingBottom).toBe(0);
  });

  it("keeps a list shorter than the viewport fully mounted", () => {
    const timelineRows = rows(5);
    const model = buildTimelineHeightModel(
      timelineRows,
      new Map(),
      TIMELINE_ESTIMATED_ITEM_HEIGHT_PX
    );
    const range = calculateTimelineVirtualRange({
      visibleItemsLength: timelineRows.length,
      metrics: { scrollTop: 0, clientHeight: 600, listOffsetTop: 0 },
      model
    });

    expect(range.virtualized).toBe(true);
    expect(range.startIndex).toBe(0);
    expect(range.endIndex).toBe(5);
    expect(range.paddingTop).toBe(0);
    expect(range.paddingBottom).toBe(0);
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
