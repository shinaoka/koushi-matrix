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
import { timelineKeyEquals } from "./coreEvents";

// ---------------------------------------------------------------------------
// Per-key state
// ---------------------------------------------------------------------------

export interface TimelineKeyState {
  /** Current known generation.  0 = not yet received InitialItems. */
  generation: number;
  /** Render list maintained by applying diffs. */
  items: TimelineItem[];
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

function keyStr(key: TimelineKey): string {
  return JSON.stringify(key);
}

function emptyKeyState(): TimelineKeyState {
  return {
    generation: 0,
    items: [],
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

/** Called on EventStreamLag (ResyncMarker): clear all keys. */
export function applyGlobalResync(store: TimelineStoreState): TimelineStoreState {
  const next = new Map<string, TimelineKeyState>();
  for (const [k, state] of store.keys) {
    next.set(k, {
      ...state,
      items: [],
      awaitingResync: true,
      mediaUploadProgress: new Map()
    });
  }
  return { keys: next };
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
  const next = new Map(store.keys);
  next.set(k, {
    ...existing,
    generation: payload.generation,
    items: [...payload.items],
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
      awaitingResync: false
    };
    const updatedItems = applyDiffs(initialized.items, payload.diffs);
    const next = new Map(store.keys);
    next.set(k, { ...initialized, items: updatedItems });
    return { keys: next };
  }

  // Stale generation: discard silently.
  if (existing.generation !== payload.generation) {
    return store;
  }

  // Awaiting resync: discard diffs; we need a fresh InitialItems first.
  if (existing.awaitingResync) {
    return store;
  }

  const updatedItems = applyDiffs(existing.items, payload.diffs);
  const next = new Map(store.keys);
  next.set(k, { ...existing, items: updatedItems });
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
    awaitingResync: true,
    mediaUploadProgress: new Map()
  });
  return { keys: next };
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
  return store.keys.get(keyStr(key))?.items ?? [];
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
