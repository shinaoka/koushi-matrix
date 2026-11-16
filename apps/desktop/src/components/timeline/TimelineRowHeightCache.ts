import type { TimelineItem } from "../../domain/coreEvents";
import type { TimelineDisplayRow } from "../../domain/timelineDisplayProjection";
import { measuredItemHeight } from "./TimelineViewportVirtualization";

/**
 * #1890: the timeline's measured row heights, keyed by `row_id` — the identity
 * `buildTimelineHeightModel` already indexes and `data-frame-item-id` carries, so
 * no new key space is introduced.
 *
 * This exists because measuring a row requires mounting it: a row outside the
 * mounted window has no rect to read. On main that meant a row's height was
 * whatever the 72 px estimate said until it was mounted again, and a measurement
 * taken during active scrolling was thrown away when the row unmounted
 * (`flushPendingMeasurements`' `!mountedDomIds.has(domId)` filter). A reader
 * wheeling through history therefore measured rows and discarded the result, so
 * the spacer above kept estimates and every remount moved the reader.
 *
 * The entry keeps the item it was measured on, the list width and a font epoch:
 * an entry that no longer matches any of those is *stale*. A stale entry is
 * still returned — an old height is a better estimate than 72 px — and the next
 * mount re-measures it. That is exactly main's behaviour for a row that was
 * never measured, and never worse than it.
 *
 * Audited for chrome that changes a row's height without changing its `item` or
 * its `row_id` (`domain/timelineDisplayProjection.ts` builds rows from the store's
 * items, so an edit, a reaction, a receipt or a new activity event does replace the
 * item). Two inputs are not in the row:
 *
 * - Sender continuation is decided from the *previous* row (`TimelineView.tsx`'s
 *   `isContinuation`, which hides the sender line in compact density), so a prepend
 *   can flip it. It is deliberately not part of freshness: the flip is computed
 *   while the row is rendered, that render changes the row's rect and the
 *   ResizeObserver re-records it, and a row that is off-window is re-measured at its
 *   next mount anyway. It cannot leave a wrong height behind.
 * - Date dividers get a fresh `TimelineItem` on every projection rebuild, so their
 *   entries read as stale almost immediately — over-staleness, never over-freshness.
 *
 * So freshness is an estimates-only signal: main's re-measure-on-mount is
 * unconditional, nothing in the product reads `isFresh`, and it exists as the
 * contract the design's deferred stale-hit diagnostic will read.
 */

/**
 * #1890: has anything the model reads moved since the last commit? A record can be
 * a no-op (an entry keeps its previous height when it moved by a pixel or less)
 * while the model has still never seen that value: a reset cancels an owed commit,
 * and the next measurement pass re-reads what the cache already holds.
 */
export function rowHeightsUnpublished(
  next: ReadonlyMap<string, number>,
  committed: ReadonlyMap<string, number>
): boolean {
  if (next.size !== committed.size) {
    return true;
  }
  for (const [rowId, height] of next) {
    if (committed.get(rowId) !== height) {
      return true;
    }
  }
  return false;
}

/** Hard cap, so a residency that grows without a bound cannot grow this. */
export const TIMELINE_ROW_HEIGHT_CACHE_MAX_ENTRIES = 4096;

interface Entry {
  height: number;
  item: TimelineItem;
  width: number;
  fontEpoch: number;
}

/**
 * Bumped when the document's fonts finish loading. Heights measured under the
 * previous metrics are stale, so every entry recorded before the bump is
 * re-measured on its next mount. Module-level because the loaded font set is
 * shared by every timeline on screen.
 */
let fontEpoch = 0;

export function noteTimelineFontsLoaded(): void {
  fontEpoch += 1;
}

export class TimelineRowHeightCache {
  /** Insertion order is recency: the last key is the most recently recorded. */
  private readonly entries = new Map<string, Entry>();
  private width = 0;
  private readonly maxEntries: number;

  constructor() {
    this.maxEntries = TIMELINE_ROW_HEIGHT_CACHE_MAX_ENTRIES;
  }

  /** Whether the entry was measured on this row's current item, width and fonts. */
  isFresh(row: TimelineDisplayRow): boolean {
    const entry = this.entries.get(row.row_id);
    return (
      entry !== undefined &&
      entry.item === row.item &&
      entry.width === this.width &&
      entry.fontEpoch === fontEpoch
    );
  }

  /**
   * Record a rect height, mounted or not. Callers record on every mount pass, so
   * a row keeps its entry while the reader is around it and its recency follows
   * the viewport — which is also what the eviction below uses.
   *
   * Returns whether the stored height moved by more than a pixel, which is the
   * only case the height model has to be rebuilt for.
   */
  record(row: TimelineDisplayRow, height: number, width: number): boolean {
    if (!(height > 0)) {
      // A zero rect means the row was not laid out at all — display:none, or a DOM
      // without layout (jsdom). Keeping the estimate is what main did, and
      // charging it 1 px would shrink the model and expand the window until
      // everything mounted.
      return false;
    }
    const rounded = measuredItemHeight(height);
    this.width = width;
    const previous = this.entries.get(row.row_id);
    // Within a pixel the model keeps what it has, so repeated measurements of the
    // same row do not churn the layout; the freshness facts still refresh.
    const changed = previous === undefined || Math.abs(previous.height - rounded) > 1;
    // Re-inserting moves the key to the most-recent end of the iteration order.
    this.entries.delete(row.row_id);
    this.entries.set(row.row_id, {
      height: changed ? rounded : previous.height,
      item: row.item,
      width,
      fontEpoch
    });
    this.evictOverflow();
    return changed;
  }

  /**
   * Drop every entry whose row is no longer resident. `rows` is the timeline's
   * resident set (`visibleRows`), which is what replaced main's two ad-hoc prunes.
   * Dropping changes nothing the model can see — it is only ever handed the
   * resident rows — so this is memory hygiene, not a commit.
   */
  retainResident(rows: readonly TimelineDisplayRow[]): void {
    if (this.entries.size === 0) {
      return;
    }
    const resident = new Set(rows.map((row) => row.row_id));
    for (const rowId of this.entries.keys()) {
      if (!resident.has(rowId)) {
        this.entries.delete(rowId);
      }
    }
  }

  /**
   * The model's input: every stored height for the rows given, stale or fresh.
   * `buildTimelineHeightModel` keeps its `ReadonlyMap` signature, so the
   * freshness decision stays here instead of spreading into the pure module.
   */
  heightsFor(rows: readonly TimelineDisplayRow[]): Map<string, number> {
    const heights = new Map<string, number>();
    for (const row of rows) {
      const entry = this.entries.get(row.row_id);
      if (entry !== undefined) {
        heights.set(row.row_id, entry.height);
      }
    }
    return heights;
  }

  /** Key change: the rows belong to a different timeline. */
  clear(): void {
    this.entries.clear();
  }

  /**
   * Keep the newest `maxEntries`. Every mount pass re-records the mounted rows, so
   * the oldest record is the row the reader has been away from longest — the one
   * farthest from the viewport.
   */
  private evictOverflow(): void {
    while (this.entries.size > this.maxEntries) {
      const oldest = this.entries.keys().next();
      if (oldest.done) {
        return;
      }
      this.entries.delete(oldest.value);
    }
  }
}
