import { describe, expect, it } from "vitest";

import type { TimelineItem } from "../../domain/coreEvents";
import type { TimelineDisplayRow } from "../../domain/timelineDisplayProjection";
import { message } from "../timelineViewTestSupport";
import {
  TIMELINE_ROW_HEIGHT_CACHE_MAX_ENTRIES,
  TimelineRowHeightCache,
  noteTimelineFontsLoaded,
  rowHeightsSettledAbove,
  rowHeightsUnpublished
} from "./TimelineRowHeightCache";

const ROW_HEIGHT = 40;
const ESTIMATE = 72;
const WIDTH = 720;

function row(eventId: string, item: TimelineItem = message(eventId, `body ${eventId}`)): TimelineDisplayRow {
  return {
    row_id: `event:${eventId}`,
    item,
    content_event_id: eventId,
    activity_event_id: eventId,
    gap_id: null,
    content_timestamp_ms: null,
    display_timestamp_ms: null,
    kind: "event"
  };
}

function rows(count: number, prefix = "$row"): TimelineDisplayRow[] {
  return Array.from({ length: count }, (_, index) => row(`${prefix}${index}`));
}

/** The height a row would be laid out with, or `undefined` if it was never measured. */
function heights(cache: TimelineRowHeightCache, target: TimelineDisplayRow): number | undefined {
  return cache.heightsFor([target]).get(target.row_id);
}

describe("TimelineRowHeightCache", () => {
  it("keeps a measurement while its row is resident and drops it once it leaves", () => {
    const cache = new TimelineRowHeightCache();
    const [resident, leaving] = rows(2);
    cache.record(resident!, ROW_HEIGHT, WIDTH);
    cache.record(leaving!, ROW_HEIGHT, WIDTH);
    expect(heights(cache, resident!)).toBe(ROW_HEIGHT);
    expect(cache.heightsFor([resident!]).size).toBe(1);

    cache.retainResident([resident!]);
    expect(heights(cache, leaving!)).toBeUndefined();
    expect(cache.heightsFor([resident!, leaving!]).size).toBe(1);
  });

  it("reports a height change only when the stored height actually moved", () => {
    const cache = new TimelineRowHeightCache();
    const [first] = rows(1);
    expect(cache.record(first!, ROW_HEIGHT, WIDTH)).toBe(true);
    expect(cache.record(first!, ROW_HEIGHT, WIDTH)).toBe(false);
    // Rounding noise inside one pixel is not a model change.
    expect(cache.record(first!, ROW_HEIGHT + 0.6, WIDTH)).toBe(false);
    expect(cache.record(first!, ROW_HEIGHT + 2, WIDTH)).toBe(true);
    expect(heights(cache, first!)).toBe(ROW_HEIGHT + 2);
  });

  it("keeps the old value as the estimate for a replaced item, and marks it stale", () => {
    const cache = new TimelineRowHeightCache();
    const [before] = rows(1);
    cache.record(before!, ROW_HEIGHT, WIDTH);
    expect(cache.isFresh(before!)).toBe(true);

    // The store replaces the item object on an edit, a reaction or a receipt.
    const edited: TimelineDisplayRow = { ...before!, item: message("$row0", "edited body") };
    expect(cache.isFresh(edited)).toBe(false);
    // Stale is still a better estimate than the 72 px fallback.
    expect(heights(cache, edited)).toBe(ROW_HEIGHT);
  });

  it("marks an entry stale when the list width or the loaded fonts change", () => {
    const cache = new TimelineRowHeightCache();
    const [first] = rows(1);
    cache.record(first!, ROW_HEIGHT, WIDTH);
    expect(cache.isFresh(first!)).toBe(true);

    cache.record(row("$other"), ROW_HEIGHT, WIDTH + 120);
    expect(cache.isFresh(first!)).toBe(false);
    expect(heights(cache, first!)).toBe(ROW_HEIGHT);

    cache.record(first!, ROW_HEIGHT, WIDTH + 120);
    expect(cache.isFresh(first!)).toBe(true);
    noteTimelineFontsLoaded();
    expect(cache.isFresh(first!)).toBe(false);
    expect(heights(cache, first!)).toBe(ROW_HEIGHT);
  });

  it("evicts the entries recorded longest ago, which are the ones far from the viewport", () => {
    const cache = new TimelineRowHeightCache();
    const all = rows(TIMELINE_ROW_HEIGHT_CACHE_MAX_ENTRIES);
    for (const resident of all) {
      cache.record(resident, ROW_HEIGHT, WIDTH);
    }
    // Re-recording the viewport's own row makes it the most recent again, so the
    // newcomer pushes out the row the reader has been away from longest.
    cache.record(all[0]!, ROW_HEIGHT, WIDTH);
    cache.record(row("$newcomer"), ROW_HEIGHT, WIDTH);

    expect(cache.heightsFor(all).size).toBe(TIMELINE_ROW_HEIGHT_CACHE_MAX_ENTRIES - 1);
    expect(heights(cache, all[1]!)).toBeUndefined();
    expect(heights(cache, all[0]!)).toBe(ROW_HEIGHT);
    expect(heights(cache, all[all.length - 1]!)).toBe(ROW_HEIGHT);
  });

  it("holds the entry cap at the documented bound", () => {
    const cache = new TimelineRowHeightCache();
    const all = rows(TIMELINE_ROW_HEIGHT_CACHE_MAX_ENTRIES + 24);
    for (const resident of all) {
      cache.record(resident, ROW_HEIGHT, WIDTH);
    }
    expect(cache.heightsFor(all).size).toBe(TIMELINE_ROW_HEIGHT_CACHE_MAX_ENTRIES);
  });

  it("clears on a timeline key change", () => {
    const cache = new TimelineRowHeightCache();
    const [first] = rows(1);
    cache.record(first!, ROW_HEIGHT, WIDTH);
    cache.clear();
    expect(heights(cache, first!)).toBeUndefined();
    expect(cache.heightsFor([first!]).size).toBe(0);
  });

  it("keeps a measured height below the fallback clamp as measured", () => {
    const cache = new TimelineRowHeightCache();
    const [shortRow] = rows(1);
    // A collapsed state event is genuinely shorter than the fallback clamp (36).
    cache.record(shortRow!, 12, WIDTH);
    expect(heights(cache, shortRow!)).toBe(12);
    expect(heights(cache, shortRow!)).toBeLessThan(ESTIMATE);
  });
});

describe("rowHeightsUnpublished", () => {
  it("is false only when the model has already seen every resident height", () => {
    const committed = new Map([
      ["event:$a", 40],
      ["event:$b", 60]
    ]);
    expect(rowHeightsUnpublished(new Map(committed), committed)).toBe(false);
    // A value the model has not seen, even if the cache calls it "no change".
    expect(rowHeightsUnpublished(new Map([["event:$a", 40], ["event:$b", 64]]), committed)).toBe(true);
    // A row that left residency, and a row that joined it.
    expect(rowHeightsUnpublished(new Map([["event:$a", 40]]), committed)).toBe(true);
    expect(rowHeightsUnpublished(new Map([["event:$a", 40], ["event:$b", 60], ["event:$c", 40]]), committed)).toBe(true);
  });
});

describe("rowHeightsSettledAbove", () => {
  it("is false while a row above the viewport has an uncommitted measurement", () => {
    const cache = new TimelineRowHeightCache();
    const [first, second, third] = rows(3);
    cache.record(first!, ROW_HEIGHT, WIDTH);
    cache.record(second!, ROW_HEIGHT + 20, WIDTH);
    cache.record(third!, ROW_HEIGHT, WIDTH);
    const committed = new Map([
      [first!.row_id, ROW_HEIGHT],
      [third!.row_id, ROW_HEIGHT]
    ]);

    // The two rows before index 2 are above the viewport: the second one grew
    // past what the model committed, so the spacer above is about to change.
    expect(rowHeightsSettledAbove(cache, [first!, second!, third!], 2, committed)).toBe(false);
    // Only the row before index 1 is above it, and that one is committed.
    expect(rowHeightsSettledAbove(cache, [first!, second!, third!], 1, committed)).toBe(true);
    // Rows at or after the boundary never count, so an unsettled row below the
    // viewport does not withhold a request.
    expect(rowHeightsSettledAbove(cache, [first!, second!, third!], 0, committed)).toBe(true);
  });

  it("is true for an unmeasured row above the viewport", () => {
    // No measurement exists, so the model's estimate is all there is and nothing
    // is owed: waiting for one would strand a room whose history is not mounted.
    const cache = new TimelineRowHeightCache();
    const [first, second] = rows(2);
    expect(rowHeightsSettledAbove(cache, [first!, second!], 2, new Map())).toBe(true);
    cache.record(first!, ROW_HEIGHT, WIDTH);
    expect(rowHeightsSettledAbove(cache, [first!, second!], 2, new Map())).toBe(false);
  });
});
