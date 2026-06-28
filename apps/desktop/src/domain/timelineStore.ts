/**
 * Timeline store: applies CoreEvent::Timeline diffs to maintain a per-key
 * render list of TimelineItems. Operates on the WIRE shapes defined in
 * coreEvents.ts (externally tagged serde enums).
 *
 * Contract (docs/architecture/overview.md — "Timeline Viewport And Scrollback"):
 *
 * - InitialItems: replaces the current list for the given (key, generation).
 * - ItemsUpdated: applies diffs if generation matches; if the view mounted
 *   after InitialItems was emitted, a missing key is initialized from an empty
 *   list for the live diff. Stale generations are still dropped silently
 *   (Async rule 4: after reset/resync the UI discards diffs from older
 *   generations).
 * - ResyncRequired: clears the list and marks the store as awaiting the next
 *   InitialItems for that key.
 * - ResyncMarker (from EventStreamLag): same as ResyncRequired but global —
 *   all keys are cleared and await InitialItems.
 *
 * Scroll anchoring responsibilities (UI layer; not core):
 *   Before a prepend batch affects the viewport, the component captures an
 *   anchor (stable item id + pixel offset). After the diff is applied and
 *   React commits, it restores the anchor in a layout effect and only then
 *   allows the next automatic fill request. This store does no DOM work; it
 *   only tracks the item list.
 *
 * Pagination suppression:
 *   The store exposes paginationState per (key, direction). Callers must not
 *   issue a new Paginate command if the state is "Paginating" or
 *   "EndReached".
 *
 * This is a pure in-memory reducer: no side effects, no Tauri calls.
 */

import type {
  MediaTransferProgress,
  PaginationDirection,
  PaginationState,
  TimelineDiff,
  TimelineEvent,
  TimelineItem,
  TimelineKey
} from "./coreEvents";
import { timelineItemDomId, timelineKeyEquals } from "./coreEvents";
import type { TimelineThreadRootOrder } from "./types";

// ---------------------------------------------------------------------------
// Per-key state
// ---------------------------------------------------------------------------

export interface TimelineKeyState {
  /** Current known Core timeline generation. 0 is a valid first generation. */
  generation: number;
  /** Render list maintained by applying diffs. */
  items: TimelineItem[];
  /** Stable item id -> render-list index for O(1) duplicate checks. */
  itemIndexById: Map<string, number>;
  /** Timestamp -> item ids, kept in sync with the render list for diagnostics and fast updates. */
  itemIdsByTimestamp: Map<number, Set<string>>;
  /** Last applied SDK VectorDiff batch id for this generation. */
  lastAppliedBatchId: number | null;
  /** True while awaiting InitialItems after ResyncRequired / ResyncMarker. */
  awaitingResync: boolean;
  paginationBackward: PaginationState;
  paginationForward: PaginationState;
  mediaUploadProgress: Map<string, MediaTransferProgress>;
}

// ---------------------------------------------------------------------------
// Store type
// ---------------------------------------------------------------------------

export interface TimelineStoreState {
  /** Keyed by JSON.stringify(TimelineKey) for simple equality. */
  keys: Map<string, TimelineKeyState>;
}

export interface TimelineDisplayOptions {
  threadRootOrder?: TimelineThreadRootOrder;
  /** When available, non-Room timelines (Thread, Focused) skip thread-root
   *  latest-reply projection and preserve SDK/actor item order. */
  timelineKey?: TimelineKey;
}

export const TIMELINE_STORE_INACTIVE_RETAIN_LIMIT = 8;

function keyStr(key: TimelineKey): string {
  return JSON.stringify(key);
}

export function timelineStoreKeyId(key: TimelineKey): string {
  return keyStr(key);
}

function emptyKeyState(): TimelineKeyState {
  return {
    generation: 0,
    items: [],
    itemIndexById: new Map(),
    itemIdsByTimestamp: new Map(),
    lastAppliedBatchId: null,
    awaitingResync: true,
    paginationBackward: "Idle",
    paginationForward: "Idle",
    mediaUploadProgress: new Map()
  };
}

export function createTimelineStore(): TimelineStoreState {
  return { keys: new Map() };
}

// ---------------------------------------------------------------------------
// Apply a single TimelineEvent to the store; returns a new store (immutable).
// ---------------------------------------------------------------------------

export function applyTimelineEvent(
  store: TimelineStoreState,
  event: TimelineEvent
): TimelineStoreState {
  if ("InitialItems" in event) {
    return applyInitialItems(store, event.InitialItems);
  }
  if ("ItemsUpdated" in event) {
    return applyItemsUpdated(store, event.ItemsUpdated);
  }
  if ("DisplayLabelsUpdated" in event) {
    return applyDisplayLabelsUpdated(store, event.DisplayLabelsUpdated);
  }
  if ("DisplayPolicyUpdated" in event) {
    return applyDisplayPolicyUpdated(store, event.DisplayPolicyUpdated);
  }
  if ("PaginationStateChanged" in event) {
    return applyPaginationStateChanged(store, event.PaginationStateChanged);
  }
  if ("ResyncRequired" in event) {
    return applyResyncRequired(store, event.ResyncRequired.key);
  }
  if ("MediaUploadProgress" in event) {
    return applyMediaUploadProgress(store, event.MediaUploadProgress);
  }
  if ("SendCompleted" in event) {
    return applySendCompleted(store, event.SendCompleted);
  }
  // MediaDownloadCompleted does not change the render list; native persistence
  // is handled by the Rust adapter and future UI state will arrive as events.
  return store;
}

export function applyTimelineEventWithRetention(
  store: TimelineStoreState,
  event: TimelineEvent,
  retainedKeyIds: ReadonlySet<string>,
  inactiveLimit = TIMELINE_STORE_INACTIVE_RETAIN_LIMIT
): TimelineStoreState {
  const next = applyTimelineEvent(store, event);
  return pruneTimelineStore(next, retainedKeyIds, timelineEventKeyId(event), inactiveLimit);
}

/** Called on EventStreamLag (ResyncMarker): clear all keys. */
export function applyGlobalResync(store: TimelineStoreState): TimelineStoreState {
  const next = new Map<string, TimelineKeyState>();
  for (const [k, state] of store.keys) {
    next.set(k, {
      ...state,
      items: [],
      itemIndexById: new Map(),
      itemIdsByTimestamp: new Map(),
      lastAppliedBatchId: null,
      awaitingResync: true,
      mediaUploadProgress: new Map()
    });
  }
  return { keys: next };
}

export function pruneTimelineStore(
  store: TimelineStoreState,
  retainedKeyIds: ReadonlySet<string>,
  touchedKeyId: string | null = null,
  inactiveLimit = TIMELINE_STORE_INACTIVE_RETAIN_LIMIT
): TimelineStoreState {
  const retainLimit = Math.max(0, Math.trunc(inactiveLimit));
  const next = new Map(store.keys);
  let movedTouchedKey = false;
  if (touchedKeyId !== null && next.has(touchedKeyId)) {
    const touched = next.get(touchedKeyId)!;
    next.delete(touchedKeyId);
    next.set(touchedKeyId, touched);
    movedTouchedKey = true;
  }

  let inactiveCount = 0;
  for (const keyId of next.keys()) {
    if (!retainedKeyIds.has(keyId)) {
      inactiveCount += 1;
    }
  }
  if (inactiveCount <= retainLimit) {
    return movedTouchedKey ? { keys: next } : store;
  }

  let evictCount = inactiveCount - retainLimit;
  for (const keyId of next.keys()) {
    if (evictCount === 0) {
      break;
    }
    if (retainedKeyIds.has(keyId)) {
      continue;
    }
    next.delete(keyId);
    evictCount -= 1;
  }
  return { keys: next };
}

function timelineEventKeyId(event: TimelineEvent): string | null {
  if ("InitialItems" in event) {
    return keyStr(event.InitialItems.key);
  }
  if ("ItemsUpdated" in event) {
    return keyStr(event.ItemsUpdated.key);
  }
  if ("PaginationStateChanged" in event) {
    return keyStr(event.PaginationStateChanged.key);
  }
  if ("AnchorMaterializeFinished" in event) {
    return keyStr(event.AnchorMaterializeFinished.key);
  }
  if ("NavigationUpdated" in event) {
    return keyStr(event.NavigationUpdated.key);
  }
  if ("SendCompleted" in event) {
    return keyStr(event.SendCompleted.key);
  }
  if ("MessageForwarded" in event) {
    return keyStr(event.MessageForwarded.key);
  }
  if ("MessageSourceLoaded" in event) {
    return keyStr(event.MessageSourceLoaded.key);
  }
  if ("MediaUploadProgress" in event) {
    return keyStr(event.MediaUploadProgress.key);
  }
  if ("MediaDownloadProgress" in event) {
    return keyStr(event.MediaDownloadProgress.key);
  }
  if ("MediaDownloadCompleted" in event) {
    return keyStr(event.MediaDownloadCompleted.key);
  }
  if ("MediaDownloadFailed" in event) {
    return keyStr(event.MediaDownloadFailed.key);
  }
  if ("ResyncRequired" in event) {
    return keyStr(event.ResyncRequired.key);
  }
  return null;
}

// ---------------------------------------------------------------------------
// Internal reducers
// ---------------------------------------------------------------------------

function applyInitialItems(
  store: TimelineStoreState,
  payload: Extract<TimelineEvent, { InitialItems: unknown }>["InitialItems"]
): TimelineStoreState {
  const k = keyStr(payload.key);
  const existing = store.keys.get(k) ?? emptyKeyState();
  const indexed = indexedTimelineItems(payload.items);
  const next = new Map(store.keys);
  next.set(k, {
    ...existing,
    generation: payload.generation,
    items: indexed.items,
    itemIndexById: indexed.itemIndexById,
    itemIdsByTimestamp: indexed.itemIdsByTimestamp,
    lastAppliedBatchId: null,
    awaitingResync: false
  });
  return { keys: next };
}

function applyMediaUploadProgress(
  store: TimelineStoreState,
  payload: Extract<TimelineEvent, { MediaUploadProgress: unknown }>["MediaUploadProgress"]
): TimelineStoreState {
  const k = keyStr(payload.key);
  const existing = store.keys.get(k) ?? emptyKeyState();
  const progress = new Map(existing.mediaUploadProgress);
  progress.set(payload.transaction_id, payload.progress);
  const next = new Map(store.keys);
  next.set(k, { ...existing, mediaUploadProgress: progress });
  return { keys: next };
}

function applySendCompleted(
  store: TimelineStoreState,
  payload: Extract<TimelineEvent, { SendCompleted: unknown }>["SendCompleted"]
): TimelineStoreState {
  const k = keyStr(payload.key);
  const existing = store.keys.get(k);
  if (!existing || !existing.mediaUploadProgress.has(payload.transaction_id)) {
    return store;
  }
  const progress = new Map(existing.mediaUploadProgress);
  progress.delete(payload.transaction_id);
  const next = new Map(store.keys);
  next.set(k, { ...existing, mediaUploadProgress: progress });
  return { keys: next };
}

function applyItemsUpdated(
  store: TimelineStoreState,
  payload: Extract<TimelineEvent, { ItemsUpdated: unknown }>["ItemsUpdated"]
): TimelineStoreState {
  const k = keyStr(payload.key);
  const existing = store.keys.get(k);
  if (!existing) {
    const initialized = {
      ...emptyKeyState(),
      generation: payload.generation,
      lastAppliedBatchId: payload.batch_id,
      awaitingResync: false
    };
    const updated = applyDiffsForRender(
      initialized.items,
      initialized.itemIndexById,
      initialized.itemIdsByTimestamp,
      payload.diffs
    );
    const next = new Map(store.keys);
    next.set(k, {
      ...initialized,
      items: updated.items,
      itemIndexById: updated.itemIndexById,
      itemIdsByTimestamp: updated.itemIdsByTimestamp
    });
    return { keys: next };
  }

  // Stale generation: discard silently.
  if (existing.generation !== payload.generation) {
    return store;
  }

  if (existing.lastAppliedBatchId !== null && payload.batch_id <= existing.lastAppliedBatchId) {
    return store;
  }

  // Awaiting resync: discard diffs; we need a fresh InitialItems first.
  if (existing.awaitingResync) {
    return store;
  }

  const updated = applyDiffsForRender(
    existing.items,
    existing.itemIndexById,
    existing.itemIdsByTimestamp,
    payload.diffs
  );
  const next = new Map(store.keys);
  next.set(k, {
    ...existing,
    items: updated.items,
    itemIndexById: updated.itemIndexById,
    itemIdsByTimestamp: updated.itemIdsByTimestamp,
    lastAppliedBatchId: payload.batch_id
  });
  return { keys: next };
}

function applyPaginationStateChanged(
  store: TimelineStoreState,
  payload: Extract<
    TimelineEvent,
    { PaginationStateChanged: unknown }
  >["PaginationStateChanged"]
): TimelineStoreState {
  const k = keyStr(payload.key);
  const existing = store.keys.get(k) ?? emptyKeyState();
  const next = new Map(store.keys);
  const updated: TimelineKeyState =
    payload.direction === "Backward"
      ? { ...existing, paginationBackward: payload.state }
      : { ...existing, paginationForward: payload.state };
  next.set(k, updated);
  return { keys: next };
}

function applyResyncRequired(
  store: TimelineStoreState,
  key: TimelineKey
): TimelineStoreState {
  const k = keyStr(key);
  const existing = store.keys.get(k);
  if (!existing) {
    return store;
  }
  const next = new Map(store.keys);
  next.set(k, {
    ...existing,
    items: [],
    itemIndexById: new Map(),
    itemIdsByTimestamp: new Map(),
    awaitingResync: true,
    mediaUploadProgress: new Map()
  });
  return { keys: next };
}

function applyDisplayPolicyUpdated(
  store: TimelineStoreState,
  payload: Extract<TimelineEvent, { DisplayPolicyUpdated: unknown }>["DisplayPolicyUpdated"]
): TimelineStoreState {
  if (store.keys.size === 0) {
    return store;
  }

  let changed = false;
  const next = new Map<string, TimelineKeyState>();
  for (const [key, state] of store.keys) {
    let itemsChanged = false;
    const items = state.items.map((item) => {
      const isHidden = payload.hide_redacted && item.is_redacted;
      if (item.is_hidden === isHidden) {
        return item;
      }
      itemsChanged = true;
      return { ...item, is_hidden: isHidden };
    });

    if (itemsChanged) {
      changed = true;
      next.set(key, { ...state, items });
    } else {
      next.set(key, state);
    }
  }

  return changed ? { keys: next } : store;
}

function applyDisplayLabelsUpdated(
  store: TimelineStoreState,
  payload: Extract<TimelineEvent, { DisplayLabelsUpdated: unknown }>["DisplayLabelsUpdated"]
): TimelineStoreState {
  if (payload.labels.length === 0 || store.keys.size === 0) {
    return store;
  }

  const labels = new Map<string, string | null>(
    payload.labels.map((label) => [
      label.user_id,
      label.display_label.trim().length === 0 ? null : label.display_label
    ])
  );
  let changed = false;
  const next = new Map<string, TimelineKeyState>();

  for (const [key, state] of store.keys) {
    let itemsChanged = false;
    const items = state.items.map((item) => {
      const updated = applyDisplayLabelUpdateToItem(item, labels);
      if (updated !== item) {
        itemsChanged = true;
      }
      return updated;
    });

    if (itemsChanged) {
      changed = true;
      next.set(key, { ...state, items });
    } else {
      next.set(key, state);
    }
  }

  return changed ? { keys: next } : store;
}

function applyDisplayLabelUpdateToItem(
  item: TimelineItem,
  labels: Map<string, string | null>
): TimelineItem {
  let updated: TimelineItem = item;

  const senderLabel = labelUpdateFor(item.sender, labels);
  if (senderLabel !== undefined && item.sender_label !== senderLabel) {
    updated = { ...updated, sender_label: senderLabel };
  }

  if (updated.reply_quote) {
    const quoteLabel = labelUpdateFor(updated.reply_quote.sender, labels);
    if (quoteLabel !== undefined && updated.reply_quote.sender_label !== quoteLabel) {
      updated = {
        ...updated,
        reply_quote: { ...updated.reply_quote, sender_label: quoteLabel }
      };
    }
  }

  if (updated.thread_summary) {
    const latestSenderLabel = labelUpdateFor(updated.thread_summary.latest_sender, labels);
    if (
      latestSenderLabel !== undefined &&
      updated.thread_summary.latest_sender_label !== latestSenderLabel
    ) {
      updated = {
        ...updated,
        thread_summary: {
          ...updated.thread_summary,
          latest_sender_label: latestSenderLabel
        }
      };
    }
  }

  return updated;
}

function labelUpdateFor(
  userId: string | null | undefined,
  labels: Map<string, string | null>
): string | null | undefined {
  if (!userId || !labels.has(userId)) {
    return undefined;
  }
  return labels.get(userId) ?? null;
}

// ---------------------------------------------------------------------------
// VectorDiff application
// ---------------------------------------------------------------------------

export function applyDiffs(
  items: TimelineItem[],
  diffs: TimelineDiff[]
): TimelineItem[] {
  let current = [...items];
  for (const diff of diffs) {
    current = applyOneDiff(current, diff);
  }
  return current;
}

function applyOneDiff(items: TimelineItem[], diff: TimelineDiff): TimelineItem[] {
  if (diff === "Clear") {
    return [];
  }
  if ("PushFront" in diff) {
    return [diff.PushFront.item, ...items];
  }
  if ("PushBack" in diff) {
    return [...items, diff.PushBack.item];
  }
  if ("Insert" in diff) {
    const { index, item } = diff.Insert;
    const result = [...items];
    result.splice(index, 0, item);
    return result;
  }
  if ("Set" in diff) {
    const { index, item } = diff.Set;
    const result = [...items];
    result[index] = item;
    return result;
  }
  if ("Remove" in diff) {
    const result = [...items];
    result.splice(diff.Remove.index, 1);
    return result;
  }
  if ("Truncate" in diff) {
    return items.slice(0, diff.Truncate.length);
  }
  if ("Reset" in diff) {
    return [...diff.Reset.items];
  }
  return items;
}

interface IndexedTimelineItems {
  items: TimelineItem[];
  itemIndexById: Map<string, number>;
  itemIdsByTimestamp: Map<number, Set<string>>;
}

function applyDiffsForRender(
  items: readonly TimelineItem[],
  itemIndexById: ReadonlyMap<string, number>,
  itemIdsByTimestamp: ReadonlyMap<number, ReadonlySet<string>>,
  diffs: readonly TimelineDiff[]
): IndexedTimelineItems {
  let current = [...items];
  const indexById = new Map(itemIndexById);
  let idsByTimestamp = cloneTimestampIndex(itemIdsByTimestamp);

  for (const diff of diffs) {
    if (diff === "Clear") {
      current = [];
      indexById.clear();
      idsByTimestamp.clear();
    } else if ("PushFront" in diff) {
      insertTimelineItem(current, indexById, idsByTimestamp, diff.PushFront.item, 0);
    } else if ("PushBack" in diff) {
      insertTimelineItem(current, indexById, idsByTimestamp, diff.PushBack.item, current.length);
    } else if ("Insert" in diff) {
      insertTimelineItem(current, indexById, idsByTimestamp, diff.Insert.item, diff.Insert.index);
    } else if ("Set" in diff) {
      setTimelineItem(current, indexById, idsByTimestamp, diff.Set.index, diff.Set.item);
    } else if ("Remove" in diff) {
      removeTimelineItemAt(current, indexById, idsByTimestamp, diff.Remove.index);
    } else if ("Truncate" in diff) {
      const indexed = indexedTimelineItems(current.slice(0, diff.Truncate.length));
      current = indexed.items;
      indexById.clear();
      idsByTimestamp.clear();
      copyIndex(indexById, indexed.itemIndexById);
      idsByTimestamp = indexed.itemIdsByTimestamp;
    } else if ("Reset" in diff) {
      const indexed = indexedTimelineItems(diff.Reset.items);
      current = indexed.items;
      indexById.clear();
      idsByTimestamp.clear();
      copyIndex(indexById, indexed.itemIndexById);
      idsByTimestamp = indexed.itemIdsByTimestamp;
    }
  }
  return { items: current, itemIndexById: indexById, itemIdsByTimestamp: idsByTimestamp };
}

function indexedTimelineItems(items: readonly TimelineItem[]): IndexedTimelineItems {
  const result: TimelineItem[] = [];
  const itemIndexById = new Map<string, number>();
  const itemIdsByTimestamp = new Map<number, Set<string>>();

  for (const item of items) {
    const id = timelineItemDomId(item.id);
    if (itemIndexById.has(id)) {
      continue;
    }
    itemIndexById.set(id, result.length);
    addTimestampIndex(itemIdsByTimestamp, item);
    result.push(item);
  }

  return { items: result, itemIndexById, itemIdsByTimestamp };
}

function insertTimelineItem(
  items: TimelineItem[],
  itemIndexById: Map<string, number>,
  itemIdsByTimestamp: Map<number, Set<string>>,
  item: TimelineItem,
  preferredIndex: number
): void {
  const id = timelineItemDomId(item.id);
  if (itemIndexById.has(id)) {
    return;
  }
  const insertIndex = clampIndex(preferredIndex, items.length);
  items.splice(insertIndex, 0, item);
  reindexItemsFrom(items, itemIndexById, insertIndex);
  addTimestampIndex(itemIdsByTimestamp, item);
}

function setTimelineItem(
  items: TimelineItem[],
  itemIndexById: Map<string, number>,
  itemIdsByTimestamp: Map<number, Set<string>>,
  index: number,
  item: TimelineItem
): void {
  if (index < 0 || index >= items.length) {
    return;
  }
  const id = timelineItemDomId(item.id);
  const existingIndex = itemIndexById.get(id);
  let targetIndex = index;
  if (existingIndex !== undefined && existingIndex !== index) {
    removeTimelineItemAt(items, itemIndexById, itemIdsByTimestamp, existingIndex);
    if (existingIndex < targetIndex) {
      targetIndex -= 1;
    }
  }
  removeTimestampIndex(itemIdsByTimestamp, items[targetIndex]);
  items[targetIndex] = item;
  itemIndexById.set(id, targetIndex);
  addTimestampIndex(itemIdsByTimestamp, item);
}

function removeTimelineItemAt(
  items: TimelineItem[],
  itemIndexById: Map<string, number>,
  itemIdsByTimestamp: Map<number, Set<string>>,
  index: number
): void {
  if (index < 0 || index >= items.length) {
    return;
  }
  const [removed] = items.splice(index, 1);
  if (!removed) {
    return;
  }
  itemIndexById.delete(timelineItemDomId(removed.id));
  removeTimestampIndex(itemIdsByTimestamp, removed);
  reindexItemsFrom(items, itemIndexById, index);
}

function reindexItemsFrom(
  items: readonly TimelineItem[],
  itemIndexById: Map<string, number>,
  startIndex: number
): void {
  for (let index = Math.max(0, startIndex); index < items.length; index += 1) {
    itemIndexById.set(timelineItemDomId(items[index].id), index);
  }
}

function addTimestampIndex(
  itemIdsByTimestamp: Map<number, Set<string>>,
  item: TimelineItem
): void {
  const timestamp = item.timestamp_ms;
  if (timestamp === null || timestamp === undefined) {
    return;
  }
  let ids = itemIdsByTimestamp.get(timestamp);
  if (!ids) {
    ids = new Set();
    itemIdsByTimestamp.set(timestamp, ids);
  }
  ids.add(timelineItemDomId(item.id));
}

function removeTimestampIndex(
  itemIdsByTimestamp: Map<number, Set<string>>,
  item: TimelineItem
): void {
  const timestamp = item.timestamp_ms;
  if (timestamp === null || timestamp === undefined) {
    return;
  }
  const ids = itemIdsByTimestamp.get(timestamp);
  if (!ids) {
    return;
  }
  ids.delete(timelineItemDomId(item.id));
  if (ids.size === 0) {
    itemIdsByTimestamp.delete(timestamp);
  }
}

function cloneTimestampIndex(
  itemIdsByTimestamp: ReadonlyMap<number, ReadonlySet<string>>
): Map<number, Set<string>> {
  return new Map(
    [...itemIdsByTimestamp.entries()].map(([timestamp, ids]) => [timestamp, new Set(ids)])
  );
}

function copyIndex(target: Map<string, number>, source: ReadonlyMap<string, number>): void {
  for (const [id, index] of source) {
    target.set(id, index);
  }
}

function clampIndex(index: number, length: number): number {
  if (!Number.isFinite(index)) {
    return length;
  }
  return Math.max(0, Math.min(Math.trunc(index), length));
}

/** True if any diff in the batch prepends items (scroll-anchor relevant). */
export function batchContainsPrepend(diffs: TimelineDiff[]): boolean {
  return diffs.some(
    (diff) =>
      diff !== "Clear" &&
      ("PushFront" in diff || ("Insert" in diff && diff.Insert.index === 0))
  );
}

// ---------------------------------------------------------------------------
// Selector helpers
// ---------------------------------------------------------------------------

const EMPTY_TIMELINE_ITEMS = Object.freeze([]) as unknown as TimelineItem[];

export function getKeyState(
  store: TimelineStoreState,
  key: TimelineKey
): TimelineKeyState | undefined {
  return store.keys.get(keyStr(key));
}

export function getItems(
  store: TimelineStoreState,
  key: TimelineKey
): TimelineItem[] {
  return store.keys.get(keyStr(key))?.items ?? EMPTY_TIMELINE_ITEMS;
}

export function getTimelineDisplayItems(
  store: TimelineStoreState,
  key: TimelineKey,
  options?: TimelineDisplayOptions
): TimelineItem[] {
  return projectTimelineItemsForDisplay(getItems(store, key), {
    ...options,
    timelineKey: key
  });
}

export function projectTimelineItemsForDisplay(
  items: TimelineItem[],
  options?: TimelineDisplayOptions
): TimelineItem[] {
  // Only Room timelines reposition thread roots by latest reply.
  // Thread and Focused timelines must preserve SDK/actor order so that the
  // thread root stays at the top and replies follow in canonical order.
  if (options?.timelineKey && !("Room" in options.timelineKey.kind)) {
    return items;
  }
  if ((options?.threadRootOrder?.kind ?? "latestReply") === "rootEvent") {
    return items;
  }
  return projectThreadRootsByLatestReply(items);
}

function projectThreadRootsByLatestReply(items: TimelineItem[]): TimelineItem[] {
  const fixedItems: TimelineItem[] = [];
  const movableThreadRoots: Array<{
    item: TimelineItem;
    index: number;
    displayTimestamp: number;
  }> = [];

  for (const [index, item] of items.entries()) {
    const displayTimestamp = threadRootLatestReplyTimestamp(item);
    if (displayTimestamp === null) {
      fixedItems.push(item);
      continue;
    }
    movableThreadRoots.push({ item, index, displayTimestamp });
  }

  if (movableThreadRoots.length === 0) {
    return items;
  }

  movableThreadRoots.sort((left, right) => {
    if (left.displayTimestamp !== right.displayTimestamp) {
      return left.displayTimestamp - right.displayTimestamp;
    }
    return left.index - right.index;
  });

  const insertionIndex = buildFixedTimelineInsertionIndex(fixedItems);
  const buckets = Array.from(
    { length: fixedItems.length + 1 },
    () => [] as TimelineItem[]
  );
  for (const root of movableThreadRoots) {
    buckets[insertionIndex(root.displayTimestamp)].push(root.item);
  }

  const projected: TimelineItem[] = [];
  projected.push(...buckets[0]);
  for (const [index, item] of fixedItems.entries()) {
    projected.push(item);
    projected.push(...buckets[index + 1]);
  }

  if (projected.every((item, index) => item === items[index])) {
    return items;
  }
  return projected;
}

function buildFixedTimelineInsertionIndex(
  fixedItems: readonly TimelineItem[]
): (targetTimestamp: number) => number {
  let nullTimestampMaxIndex = -1;
  const timestampIndexes: Array<{ timestamp: number; index: number }> = [];

  fixedItems.forEach((item, index) => {
    const timestamp = timelineProjectionTimestamp(item);
    if (timestamp === null) {
      nullTimestampMaxIndex = index;
      return;
    }
    timestampIndexes.push({ timestamp, index });
  });

  timestampIndexes.sort((left, right) =>
    left.timestamp === right.timestamp
      ? left.index - right.index
      : left.timestamp - right.timestamp
  );

  const prefixMaxIndexes: number[] = [];
  let maxIndex = -1;
  for (const entry of timestampIndexes) {
    maxIndex = Math.max(maxIndex, entry.index);
    prefixMaxIndexes.push(maxIndex);
  }

  return (targetTimestamp: number) => {
    let low = 0;
    let high = timestampIndexes.length;
    while (low < high) {
      const mid = Math.floor((low + high) / 2);
      if (timestampIndexes[mid].timestamp <= targetTimestamp) {
        low = mid + 1;
      } else {
        high = mid;
      }
    }
    const timestampMaxIndex = low > 0 ? prefixMaxIndexes[low - 1] : -1;
    return Math.max(nullTimestampMaxIndex, timestampMaxIndex) + 1;
  };
}

function timelineProjectionTimestamp(item: TimelineItem): number | null {
  return threadRootLatestReplyTimestamp(item) ?? item.timestamp_ms ?? null;
}

function threadRootLatestReplyTimestamp(item: TimelineItem): number | null {
  if (
    item.thread_summary?.latest_timestamp_ms !== null &&
    item.thread_summary?.latest_timestamp_ms !== undefined
  ) {
    return item.thread_summary.latest_timestamp_ms;
  }
  return null;
}

export function getMediaUploadProgress(
  store: TimelineStoreState,
  key: TimelineKey,
  transactionId: string
): MediaTransferProgress | null {
  return store.keys.get(keyStr(key))?.mediaUploadProgress.get(transactionId) ?? null;
}

export function getPaginationState(
  store: TimelineStoreState,
  key: TimelineKey,
  direction: PaginationDirection
): PaginationState {
  const state = store.keys.get(keyStr(key));
  if (!state) return "Idle";
  return direction === "Backward"
    ? state.paginationBackward
    : state.paginationForward;
}

export function isAwaitingResync(
  store: TimelineStoreState,
  key: TimelineKey
): boolean {
  return store.keys.get(keyStr(key))?.awaitingResync ?? true;
}

// ---------------------------------------------------------------------------
// Convenience: check if auto-backward pagination should be suppressed
// ---------------------------------------------------------------------------

export function shouldSuppressAutoBackfill(
  store: TimelineStoreState,
  key: TimelineKey
): boolean {
  const state = getPaginationState(store, key, "Backward");
  return state === "Paginating" || state === "EndReached";
}

// ---------------------------------------------------------------------------
// Re-export key equality for callers that build TimelineKey objects
// ---------------------------------------------------------------------------

export { timelineKeyEquals };
