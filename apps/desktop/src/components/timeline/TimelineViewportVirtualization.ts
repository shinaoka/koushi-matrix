import type { TimelineDisplayRow } from "../../domain/timelineDisplayProjection";

export const TIMELINE_VIRTUALIZATION_THRESHOLD = 120;
export const TIMELINE_VIRTUAL_OVERSCAN_ITEMS = 24;
export const TIMELINE_ESTIMATED_ITEM_HEIGHT_PX = 72;
export const TIMELINE_MIN_ITEM_HEIGHT_PX = 36;
const TIMELINE_MAX_ITEM_HEIGHT_PX = 480;

export type TimelineViewportMetrics = {
  scrollTop: number;
  clientHeight: number;
  listOffsetTop: number;
};

export type TimelineVirtualRangeState = {
  virtualized: boolean;
  startIndex: number;
  endIndex: number;
  paddingTop: number;
  paddingBottom: number;
};

export type TimelineItemIndexRange = {
  startIndex: number;
  endIndex: number;
};

export type TimelineVirtualWindow = TimelineVirtualRangeState & {
  items: readonly TimelineDisplayRow[];
};

export const EMPTY_TIMELINE_RANGE: TimelineVirtualRangeState = {
  virtualized: false,
  startIndex: 0,
  endIndex: 0,
  paddingTop: 0,
  paddingBottom: 0
};

export const EMPTY_TIMELINE_ITEM_INDEX_RANGE: TimelineItemIndexRange = {
  startIndex: 0,
  endIndex: 0
};

type TimelineHeightModel = {
  fallbackHeight: number;
  offsets: number[];
  totalHeight: number;
};

function estimatedItemHeight(height: number): number {
  return Math.max(
    TIMELINE_MIN_ITEM_HEIGHT_PX,
    Math.min(TIMELINE_MAX_ITEM_HEIGHT_PX, height)
  );
}

export function measuredItemHeight(height: number): number {
  return Math.max(1, Math.round(height));
}

const TIMELINE_FRAME_FALLBACK_MS = 16;

export type TimelineScheduledFrame = {
  cancel: () => void;
};

export function scheduleTimelineFrame(callback: FrameRequestCallback): TimelineScheduledFrame {
  const requestAnimationFrame =
    typeof window.requestAnimationFrame === "function"
      ? window.requestAnimationFrame.bind(window)
      : null;
  const cancelAnimationFrame =
    typeof window.cancelAnimationFrame === "function"
      ? window.cancelAnimationFrame.bind(window)
      : null;
  const setTimeout = window.setTimeout.bind(window);
  const clearTimeout = window.clearTimeout.bind(window);
  const performanceNow = window.performance.now.bind(window.performance);
  let cancelled = false;
  let frameId: number | null = null;
  let timeoutId: number | null = null;
  const run = (timestamp: number) => {
    if (cancelled) {
      return;
    }
    cancelled = true;
    if (frameId !== null && cancelAnimationFrame !== null) {
      cancelAnimationFrame(frameId);
      frameId = null;
    }
    if (timeoutId !== null) {
      clearTimeout(timeoutId);
      timeoutId = null;
    }
    callback(timestamp);
  };

  if (requestAnimationFrame !== null) {
    frameId = requestAnimationFrame(run);
  }
  timeoutId = setTimeout(() => run(performanceNow()), TIMELINE_FRAME_FALLBACK_MS);

  return {
    cancel() {
      if (cancelled) {
        return;
      }
      cancelled = true;
      if (frameId !== null && cancelAnimationFrame !== null) {
        cancelAnimationFrame(frameId);
      }
      if (timeoutId !== null) {
        clearTimeout(timeoutId);
      }
    }
  };
}

export function buildTimelineHeightModel(
  rows: readonly TimelineDisplayRow[],
  measuredHeights: ReadonlyMap<string, number>,
  fallbackHeight: number
): TimelineHeightModel {
  const fallback = estimatedItemHeight(fallbackHeight);
  const offsets = new Array<number>(rows.length + 1);
  offsets[0] = 0;
  for (const [index, row] of rows.entries()) {
    offsets[index + 1] = offsets[index] + (measuredHeights.get(row.row_id) ?? fallback);
  }
  return {
    fallbackHeight: fallback,
    offsets,
    totalHeight: offsets[rows.length] ?? 0
  };
}

function timelineIndexAtOffset(offsets: readonly number[], offset: number): number {
  if (offsets.length <= 1) {
    return 0;
  }
  const boundedOffset = Math.max(0, offset);
  let low = 0;
  let high = offsets.length - 2;
  while (low <= high) {
    const mid = Math.floor((low + high) / 2);
    if (offsets[mid + 1] <= boundedOffset) {
      low = mid + 1;
      continue;
    }
    if (offsets[mid] > boundedOffset) {
      high = mid - 1;
      continue;
    }
    return mid;
  }
  return Math.max(0, Math.min(offsets.length - 2, low));
}

export function virtualRangeEquals(
  left: TimelineVirtualRangeState,
  right: TimelineVirtualRangeState
): boolean {
  return (
    left.virtualized === right.virtualized &&
    left.startIndex === right.startIndex &&
    left.endIndex === right.endIndex &&
    left.paddingTop === right.paddingTop &&
    left.paddingBottom === right.paddingBottom
  );
}

export function timelineItemIndexRangeEquals(
  left: TimelineItemIndexRange,
  right: TimelineItemIndexRange
): boolean {
  return left.startIndex === right.startIndex && left.endIndex === right.endIndex;
}

export function timelineItemIndexInRange(index: number, range: TimelineItemIndexRange): boolean {
  return index >= range.startIndex && index < range.endIndex;
}

export function calculateTimelineItemIndexRange({
  visibleItemsLength,
  metrics,
  model,
  overscanItems
}: {
  visibleItemsLength: number;
  metrics: TimelineViewportMetrics;
  model: TimelineHeightModel;
  overscanItems: number;
}): TimelineItemIndexRange {
  if (visibleItemsLength <= 0) {
    return EMPTY_TIMELINE_ITEM_INDEX_RANGE;
  }

  const viewportHeight = metrics.clientHeight || 600;
  const relativeScrollTop = Math.max(0, metrics.scrollTop - metrics.listOffsetTop);
  const firstVisibleIndex = timelineIndexAtOffset(model.offsets, relativeScrollTop);
  const lastVisibleIndex = timelineIndexAtOffset(
    model.offsets,
    relativeScrollTop + viewportHeight
  );
  const startIndex = Math.max(0, firstVisibleIndex - overscanItems);
  const endIndex = Math.min(
    visibleItemsLength,
    Math.max(startIndex + 1, lastVisibleIndex + overscanItems + 1)
  );

  return { startIndex, endIndex };
}

export function calculateTimelineVirtualRange({
  visibleItemsLength,
  metrics,
  model
}: {
  visibleItemsLength: number;
  metrics: TimelineViewportMetrics;
  model: TimelineHeightModel;
}): TimelineVirtualRangeState {
  if (visibleItemsLength <= TIMELINE_VIRTUALIZATION_THRESHOLD) {
    return {
      virtualized: false,
      startIndex: 0,
      endIndex: visibleItemsLength,
      paddingTop: 0,
      paddingBottom: 0
    };
  }

  const { startIndex, endIndex } = calculateTimelineItemIndexRange({
    visibleItemsLength,
    metrics,
    model,
    overscanItems: TIMELINE_VIRTUAL_OVERSCAN_ITEMS
  });

  return {
    virtualized: true,
    startIndex,
    endIndex,
    paddingTop: Math.round(model.offsets[startIndex] ?? 0),
    paddingBottom: Math.round(model.totalHeight - (model.offsets[endIndex] ?? 0))
  };
}

export function timelineItemHeightAtIndex(model: TimelineHeightModel, index: number): number {
  return model.offsets[index + 1] - model.offsets[index] || model.fallbackHeight;
}
