// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import type { TimelineItem } from "../../domain/coreEvents";
import type { TimelineDisplayRow } from "../../domain/timelineDisplayProjection";
import {
  captureAnchor,
  captureFreeScrollAnchor,
  capturePrependAnchor,
  partitionHeightChangesByAnchor,
  timelineRowFullyInsideViewport,
  wholePixelScrollTop
} from "./TimelineViewportAnchors";
import {
  timelineViewportFactsFromModel,
  type TimelineViewportFacts
} from "./TimelineViewportRows";
import {
  buildTimelineHeightModel,
  timelineVisibleItemRange,
  type TimelineViewportModel
} from "./TimelineViewportVirtualization";

const BOX_TOP = 10;

function row(index: number, overrides: Partial<TimelineDisplayRow> = {}): TimelineDisplayRow {
  return {
    row_id: `row-${index}`,
    item: {} as TimelineItem,
    content_event_id: `$row-${index}`,
    activity_event_id: `$row-${index}`,
    gap_id: null,
    content_timestamp_ms: null,
    display_timestamp_ms: null,
    kind: "event",
    ...overrides
  };
}

/**
 * A mounted window whose DOM rects and height model agree, which is the state
 * the model-based reads are allowed to assume: the pass that commits the model
 * is the pass that measured the mounted rows. The container box is 10..10+height.
 */
function view(options: {
  heights?: readonly number[];
  rows?: readonly TimelineDisplayRow[];
  scrollTop?: number;
  clientHeight?: number;
  listOffsetTop?: number;
  /**
   * Draws every row this far above the offset its measured height gives it, which
   * is the disagreement #2101 is about: the model names a row the drawing puts
   * higher (or lower) than the offsets say.
   */
  shift?: number;
} = {}): { container: HTMLElement; view: TimelineViewportModel; reads: () => number } {
  const heights = options.heights ?? [72, 72];
  const rows = options.rows ?? heights.map((_, index) => row(index));
  const clientHeight = options.clientHeight ?? 200;
  const listOffsetTop = options.listOffsetTop ?? 0;
  const scrollTop = options.scrollTop ?? 0;
  const shift = options.shift ?? 0;
  const model = buildTimelineHeightModel(
    rows,
    new Map(rows.map((entry, index) => [entry.row_id, heights[index] ?? 72])),
    heights[0] ?? 72
  );
  const container = document.createElement("div");
  container.getBoundingClientRect = () =>
    ({ top: BOX_TOP, bottom: BOX_TOP + clientHeight, height: clientHeight } as DOMRect);
  Object.defineProperty(container, "clientHeight", { value: clientHeight });
  Object.defineProperty(container, "scrollTop", { value: scrollTop, writable: true });
  let reads = 0;
  rows.forEach((entry, index) => {
    const node = document.createElement("article");
    node.dataset.itemId = entry.row_id;
    const top = BOX_TOP + (model.offsets[index] ?? 0) - shift - (scrollTop - listOffsetTop);
    const height = heights[index] ?? 72;
    node.getBoundingClientRect = () => {
      reads += 1;
      return { top, bottom: top + height, height } as DOMRect;
    };
    container.appendChild(node);
  });
  return {
    container,
    view: { rows, model, metrics: { scrollTop, clientHeight, listOffsetTop } },
    reads: () => reads
  };
}

function threadRootRows(count: number): TimelineDisplayRow[] {
  return Array.from({ length: count }, (_, index) =>
    row(index, {
      content_event_id: `$root-${index}`,
      activity_event_id: `$activity-${index}`
    })
  );
}

describe("viewport anchor capture (#1890 item 5)", () => {
  it("captures nothing when the model holds no rows", () => {
    const empty = view({ heights: [], rows: [] });
    expect(captureAnchor(empty.container, empty.view)).toBeNull();
  });

  it("keeps a partially visible row and its negative offset", () => {
    const mounted = view({ heights: [72, 72], scrollTop: 30 });
    expect(captureAnchor(mounted.container, mounted.view)).toEqual({
      itemId: "row-0",
      offsetTop: -30
    });
  });

  it("leaves the range's first row out when it ends exactly at the top edge", () => {
    const mounted = view({ heights: [62, 72], scrollTop: 62, clientHeight: 60 });
    expect(captureAnchor(mounted.container, mounted.view)).toEqual({
      itemId: "row-1",
      offsetTop: 0
    });
  });

  it("skips a row the range names but the drawing puts entirely above the edge (#2101)", () => {
    // The row the offset 0 names ends at -10: the reader sees nothing of it, so a
    // height change inside it would move every visible row and its own top would
    // not budge.
    const mounted = view({
      heights: Array.from({ length: 4 }, () => 72),
      scrollTop: 82,
      clientHeight: 200,
      shift: 92
    });
    // Row 1 spans -92..-20 and row 2 spans -20..52: the range names row 1, and the
    // anchor is the first row whose box crosses the edge.
    expect(captureAnchor(mounted.container, mounted.view)).toEqual({
      itemId: "row-2",
      offsetTop: -30
    });
  });

  it("keeps the first row as the anchor when the whole window ends above the edge", () => {
    // A drawing that lags the model by more than the range is the state the old
    // capture already covered: it names the offset 0 row, so it still does.
    const mounted = view({
      heights: [72, 72],
      scrollTop: 0,
      clientHeight: 60,
      shift: 1000
    });
    expect(captureAnchor(mounted.container, mounted.view)).toEqual({
      itemId: "row-0",
      offsetTop: -1000
    });
  });

  it("reads one row rect however many rows the window holds", () => {
    const small = view({ heights: Array.from({ length: 20 }, () => 72), clientHeight: 400 });
    const large = view({ heights: Array.from({ length: 600 }, () => 72), clientHeight: 400 });
    expect(captureAnchor(small.container, small.view)).not.toBeNull();
    expect(captureAnchor(large.container, large.view)).not.toBeNull();
    // One rect for the anchor itself: the visible range comes from the model.
    expect(small.reads()).toBe(1);
    expect(large.reads()).toBe(1);
    // A candidate that ends above the edge costs the rect it was read with, and
    // the walk stops at the first row the reader can see.
    const shifted = view({
      heights: Array.from({ length: 20 }, () => 72),
      clientHeight: 400,
      shift: 80
    });
    expect(captureAnchor(shifted.container, shifted.view)).not.toBeNull();
    expect(shifted.reads()).toBe(2);
  });
});

function threadRootViewport(scrollTop: number) {
  const rows = threadRootRows(2);
  return view({ rows, heights: [72, 72], scrollTop });
}

describe("whole-pixel anchor writes (#1507)", () => {
  it("rounds to the nearest pixel instead of leaving truncation to the engine", () => {
    expect(wholePixelScrollTop(20553.71875)).toBe(20554);
    expect(wholePixelScrollTop(20724.40625)).toBe(20724);
  });
});

describe("pure prepend anchor (#1604)", () => {
  it("keeps latest-reply thread roots out of long-lived free-scroll memory", () => {
    const mounted = threadRootViewport(30);
    expect(captureFreeScrollAnchor(mounted.container, mounted.view)).toBeNull();
  });
  it("falls back to an intersecting thread root for prepend compensation", () => {
    const mounted = threadRootViewport(30);
    expect(capturePrependAnchor(mounted.container, mounted.view)).toEqual({
      itemId: "row-0",
      offsetTop: -30
    });
  });
});

describe("visible range from the height model (#1890 item 5)", () => {
  const heights = (count: number) => Array.from({ length: count }, () => 72);

  it("finds the visible run without reading any row", () => {
    const mounted = view({ heights: heights(600), scrollTop: 7000, clientHeight: 200 });
    const range = timelineVisibleItemRange(
      mounted.view.model,
      mounted.view.metrics,
      mounted.view.rows.length
    );
    expect(range.startIndex).toBe(Math.floor(7000 / 72));
    expect(range.endIndex - range.startIndex).toBeLessThan(5);
    expect(mounted.reads()).toBe(0);
  });

  it("names the last rows for a scroll position past the modelled end", () => {
    // A physical list taller than the model leaves scrollTop beyond the modelled
    // extent while a measurement is owed; those rows are still on screen, so the
    // answer is the tail, never nothing.
    const mounted = view({ heights: [72, 72], scrollTop: 900 });
    expect(
      timelineVisibleItemRange(mounted.view.model, mounted.view.metrics, mounted.view.rows.length)
    ).toEqual({ startIndex: 1, endIndex: 2 });
    const empty = view({ heights: [], rows: [] });
    expect(
      timelineVisibleItemRange(empty.view.model, empty.view.metrics, empty.view.rows.length)
    ).toEqual({ startIndex: 0, endIndex: 0 });
  });

  it("counts the offset of a list that sits below the scrollport's top", () => {
    const mounted = view({ heights: heights(10), scrollTop: 160, clientHeight: 200, listOffsetTop: 40 });
    const range = timelineVisibleItemRange(
      mounted.view.model,
      mounted.view.metrics,
      mounted.view.rows.length
    );
    // 160 - 40 = 120 px into the list, which is row 1 of 72 px rows.
    expect(range.startIndex).toBe(1);
  });

  it("names the visible rows and gaps in one pass", () => {
    const rows = [
      row(0),
      row(1),
      row(2, { activity_event_id: null, content_event_id: null, kind: "timelineGap", gap_id: { topology_revision: "3", ordinal: 1 } }),
      row(3),
      row(4)
    ];
    const mounted = view({ rows, heights: [72, 72, 24, 72, 72], scrollTop: 60, clientHeight: 200 });
    const facts: TimelineViewportFacts = timelineViewportFactsFromModel(mounted.view);
    expect(facts.firstVisibleEventId).toBe("$row-0");
    expect(facts.lastVisibleEventId).toBe("$row-4");
    expect(facts.visibleGapIds).toEqual([{ topology_revision: "3", ordinal: 1 }]);
    expect(mounted.reads()).toBe(0);
  });

  it("reports nothing for a container without a layout box", () => {
    const mounted = view({ heights: heights(10), clientHeight: 0 });
    expect(timelineViewportFactsFromModel(mounted.view)).toEqual({
      firstVisibleEventId: null,
      lastVisibleEventId: null,
      visibleGapIds: []
    });
    expect(mounted.reads()).toBe(0);
  });

  it("shows less of the list when the list starts above the scrollport's top", () => {
    // Container at scrollTop 0 with its list 40 px down: 160 of the 200 px
    // viewport show the list, so the row starting at 160 px is below the fold. A
    // band that ignored the offset would take the full 200 px and name it.
    const mounted = view({
      heights: [72, 88, 72],
      scrollTop: 0,
      clientHeight: 200,
      listOffsetTop: 40
    });
    expect(timelineViewportFactsFromModel(mounted.view).lastVisibleEventId).toBe("$row-1");
  });

  it("stops at a row that starts exactly on the bottom edge", () => {
    const rows = [row(0), row(1)];
    const mounted = view({ rows, heights: [72, 72], clientHeight: 72 });
    expect(timelineViewportFactsFromModel(mounted.view).lastVisibleEventId).toBe("$row-0");
  });
});

describe("height-change partition at the anchor (#1890 §2.2)", () => {
  const rows = Array.from({ length: 10 }, (_, index) => ({ row_id: `$row${index}` }));

  it("splits measured-height corrections above the anchor from the ones it may defer", () => {
    const partition = partitionHeightChangesByAnchor(
      rows,
      ["$row2", "$row4", "$row6", "$row9"],
      4
    );
    expect(partition.above).toEqual(["$row2"]);
    expect(partition.below).toEqual(["$row4", "$row6", "$row9"]);
  });

  it("keeps the anchor row with the deferred half, because its own height does not move it", () => {
    expect(partitionHeightChangesByAnchor(rows, ["$row3", "$row4"], 4)).toEqual({
      above: ["$row3"],
      below: ["$row4"]
    });
  });

  it("defers every change when the anchor is unknown or is the first row", () => {
    for (const anchorIndex of [-1, 0]) {
      expect(partitionHeightChangesByAnchor(rows, ["$row1", "$row8"], anchorIndex)).toEqual({
        above: [],
        below: ["$row1", "$row8"]
      });
    }
  });

  it("defers a change whose row is not in the window", () => {
    expect(partitionHeightChangesByAnchor(rows, ["$gone"], 5)).toEqual({
      above: [],
      below: ["$gone"]
    });
  });
});

describe("whole-row containment (#1987)", () => {
  // The container's band is 0..200 here, so a 72 px row is contained only between
  // row top 0 and row bottom 200.
  function band(rowTop: number, rowHeight = 72, clientHeight = 200) {
    const container = document.createElement("div");
    container.getBoundingClientRect = () => ({ top: 0, bottom: clientHeight } as DOMRect);
    Object.defineProperty(container, "clientHeight", { value: clientHeight });
    const row = document.createElement("div");
    row.getBoundingClientRect = () =>
      ({ top: rowTop, bottom: rowTop + rowHeight, height: rowHeight } as DOMRect);
    container.appendChild(row);
    return { container, row };
  }

  it("accepts a row wholly inside the band", () => {
    const { container, row } = band(64);
    expect(timelineRowFullyInsideViewport(container, row)).toBe(true);
  });

  it("rejects a row whose bottom is clipped above the band", () => {
    // The #1987 symptom: the row intersects the band, but the reader cannot see it.
    const { container, row } = band(-62);
    expect(timelineRowFullyInsideViewport(container, row)).toBe(false);
  });

  it("rejects a row whose bottom hangs below the band", () => {
    const { container, row } = band(160);
    expect(timelineRowFullyInsideViewport(container, row)).toBe(false);
  });

  it("falls back to intersection for a row taller than the band", () => {
    const { container, row } = band(-100, 400);
    expect(timelineRowFullyInsideViewport(container, row)).toBe(true);
  });

  it("falls back to intersection for a row without a layout box", () => {
    const { container, row } = band(0, 0);
    expect(timelineRowFullyInsideViewport(container, row)).toBe(true);
  });
});
