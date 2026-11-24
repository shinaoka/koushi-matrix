import type { TimelineDisplayRow } from "../../domain/timelineDisplayProjection";

/**
 * Screens of overscan mounted on each side of the viewport.
 *
 * Measured, not assumed (#1890). Over a 50-step wheel sweep from the live edge
 * to the top of a 120-row room and back in one-screen steps, a 422 px viewport
 * showed a non-empty spacer in 16 frames on Chromium and WebKit before the scroll
 * frame committed its window synchronously, in none after: with one screen of
 * overscan there is no margin left to hide a one-frame lag in the commit. Two
 * screens would hide it with extra mounted rows instead, which is what #1890
 * removes, and they exceed the window bounds the drift specs assert.
 */
export const TIMELINE_VIRTUAL_OVERSCAN_SCREENS = 1;
/** How often a focused navigation re-places a target that is still estimated. */
export const FOCUSED_TARGET_PLACEMENT_ATTEMPTS = 4;
export const TIMELINE_ESTIMATED_ITEM_HEIGHT_PX = 72;
export const TIMELINE_MIN_ITEM_HEIGHT_PX = 36;
const TIMELINE_MAX_ITEM_HEIGHT_PX = 480;

/**
 * #1890 §2.3: the height every unmeasured row starts at, frozen from the first
 * measurements of an open.
 *
 * The estimate is what makes a window's own padding believable: while
 * unmeasured rows still carry 72 px for a 48 px row, a screen of overscan covers
 * two thirds of a screen and the reader can reach the padding. One clamped mean
 * over what the open actually measured is close enough for the rest of it, and
 * it must not track later measurements — those arrive row by row, so following
 * them would move the viewport under the reader.
 */
export function freezeEstimatedItemHeight(measured: readonly number[]): number {
  const heights = measured.filter((height) => Number.isFinite(height) && height > 0);
  if (heights.length === 0) {
    return TIMELINE_ESTIMATED_ITEM_HEIGHT_PX;
  }
  const mean = heights.reduce((total, height) => total + height, 0) / heights.length;
  return estimatedItemHeight(mean);
}

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

export type TimelineHeightModel = {
  fallbackHeight: number;
  offsets: number[];
  totalHeight: number;
};

/**
 * #1890 item 5: the committed rows, their height model, and the container
 * metrics — everything a viewport question needs without touching a row.
 *
 * `metrics` is what the caller already read for this frame. An anchor capture
 * re-reads `scrollTop`/`clientHeight` from the container instead, because a
 * capture must not lag the scroll it was taken on; `listOffsetTop` is a layout
 * constant between layout changes, so it may come from the cached read.
 */
export type TimelineViewportModel = {
  rows: readonly TimelineDisplayRow[];
  model: TimelineHeightModel;
  metrics: TimelineViewportMetrics;
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

/** The row whose box contains `offset`, for the model's committed offsets. */
export function timelineIndexAtOffset(offsets: readonly number[], offset: number): number {
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

export function timelineVisibleBounds(metrics: TimelineViewportMetrics): {
  relativeScrollTop: number;
  viewportHeight: number;
} {
  return {
    relativeScrollTop: Math.max(0, metrics.scrollTop - metrics.listOffsetTop),
    viewportHeight: metrics.clientHeight || 600
  };
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

  const { relativeScrollTop, viewportHeight } = timelineVisibleBounds(metrics);
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

/**
 * Mounts the visible rows plus whole screens of overscan, so the mounted window
 * follows screens rather than a fixed row count whose footprint depends on the
 * reader's message lengths and font metrics.
 */
export function calculateTimelineIndexRangeForPixels({
  visibleItemsLength,
  metrics,
  model,
  overscanScreens
}: {
  visibleItemsLength: number;
  metrics: TimelineViewportMetrics;
  model: TimelineHeightModel;
  overscanScreens: number;
}): TimelineItemIndexRange {
  if (visibleItemsLength <= 0) {
    return EMPTY_TIMELINE_ITEM_INDEX_RANGE;
  }

  const { relativeScrollTop, viewportHeight } = timelineVisibleBounds(metrics);
  // Measured heights can lag the physical layout, and then the real scrollTop
  // sits past the modelled end. Clamp so the window still covers the tail of
  // the model instead of collapsing to its last row.
  const modelledScrollTop = Math.min(
    relativeScrollTop,
    Math.max(0, model.totalHeight - viewportHeight)
  );
  const overscanPx = Math.max(0, viewportHeight * overscanScreens);
  const startIndex = timelineIndexAtOffset(model.offsets, modelledScrollTop - overscanPx);
  const endIndex = Math.min(
    visibleItemsLength,
    timelineIndexAtOffset(model.offsets, modelledScrollTop + viewportHeight + overscanPx) + 1
  );

  return { startIndex, endIndex: Math.max(startIndex + 1, endIndex) };
}

/**
 * #1890 item 5: the rows the committed model says intersect the viewport, as a
 * half-open index range. The range comes from the model's cumulative offsets and
 * the scroll position — the same binary search Telegram Desktop uses over its
 * cached heights — so it costs no layout read and never touches a row.
 *
 * Positions are exact when the model is: `paddingTop` is the model's offset for
 * the first mounted row, and every mounted row is measured by the same pass that
 * commits the model, so a row's modelled top is where it physically is. Rounded
 * heights can move a boundary row in or out by a pixel, which is why the reader
 * of these facts treats the edges as +-1 row, not as exact.
 */
export function timelineVisibleItemRange(
  model: TimelineHeightModel,
  metrics: TimelineViewportMetrics,
  itemCount: number
): TimelineItemIndexRange {
  // Nothing is visible in a container without a box, which is the truth the
  // placement fallback below deliberately does not use (it mounts a screen so a
  // layoutless render still has rows to measure).
  if (itemCount <= 0 || metrics.clientHeight <= 0) {
    return EMPTY_TIMELINE_ITEM_INDEX_RANGE;
  }
  const { bandTop, bandBottom } = timelineVisibleBand(metrics);
  // The band is the browser's, not the model's: a physical list taller than the
  // model (unmeasured rows, chrome) leaves scrollTop beyond the modelled extent,
  // and the rows there are still on screen. The binary search clamps to the last
  // row on its own, so the answer is the tail rather than nothing.
  const startIndex = Math.max(0, timelineIndexAtOffset(model.offsets, bandTop));
  let endIndex = Math.min(itemCount, timelineIndexAtOffset(model.offsets, bandBottom) + 1);
  // A row that starts exactly at the bottom edge is not on screen; the binary
  // search returns the row *containing* the edge, which is that row.
  while (endIndex > startIndex + 1 && (model.offsets[endIndex - 1] ?? 0) >= bandBottom) {
    endIndex -= 1;
  }
  return { startIndex, endIndex: Math.max(startIndex + 1, endIndex) };
}

/**
 * #1890 item 5: the band of the list the container shows, in list coordinates.
 *
 * The bottom edge is not `bandTop + clientHeight` when the list is scrolled
 * above its own chrome: with the container at `scrollTop` 0 and the list 48 px
 * down, the list's first 452 px of a 500 px viewport are visible. Placement
 * keeps the `timelineVisibleBounds` form because mounting a screen too many only
 * costs a measurement; the facts are what the read state is derived from, so
 * they use the exact band.
 */
function timelineVisibleBand(metrics: TimelineViewportMetrics): {
  bandTop: number;
  bandBottom: number;
} {
  const relativeTop = metrics.scrollTop - metrics.listOffsetTop;
  const bandTop = Math.max(0, relativeTop);
  return {
    bandTop,
    bandBottom: bandTop + Math.max(0, metrics.clientHeight + Math.min(0, relativeTop))
  };
}

/** Scroll top that puts the live edge at the bottom of the viewport. */
export function timelineLiveEdgeScrollTop(
  model: TimelineHeightModel,
  metrics: TimelineViewportMetrics
): number {
  const viewportHeight = metrics.clientHeight || 600;
  return Math.max(0, metrics.listOffsetTop + model.totalHeight - viewportHeight);
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
  const { startIndex, endIndex } = calculateTimelineIndexRangeForPixels({
    visibleItemsLength,
    metrics,
    model,
    overscanScreens: TIMELINE_VIRTUAL_OVERSCAN_SCREENS
  });

  return {
    virtualized: true,
    startIndex,
    endIndex,
    paddingTop: Math.round(model.offsets[startIndex] ?? 0),
    paddingBottom: Math.round(model.totalHeight - (model.offsets[endIndex] ?? 0))
  };
}

/**
 * Widen a virtual range so `index` stays mounted. A focused timeline must keep
 * its navigation target in the DOM: measurements of the rows a placement
 * reached can otherwise shrink the window past it again.
 */
export function timelineRangeIncludingIndex(
  model: TimelineHeightModel,
  range: TimelineVirtualRangeState,
  index: number
): TimelineVirtualRangeState {
  if (!range.virtualized || index < 0 || (index >= range.startIndex && index < range.endIndex)) {
    return range;
  }
  const startIndex = Math.min(range.startIndex, index);
  const endIndex = Math.max(range.endIndex, index + 1);
  return {
    ...range,
    startIndex,
    endIndex,
    paddingTop: Math.round(model.offsets[startIndex] ?? 0),
    paddingBottom: Math.round(model.totalHeight - (model.offsets[endIndex] ?? 0))
  };
}

export function timelineItemHeightAtIndex(model: TimelineHeightModel, index: number): number {
  return model.offsets[index + 1] - model.offsets[index] || model.fallbackHeight;
}
