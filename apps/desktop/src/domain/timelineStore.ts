/**
 * Timeline store: applies CoreEvent::Timeline diffs to maintain a per-key
 * render list of TimelineItems.
 *
 * Contract (docs/architecture/overview.md — "Timeline Viewport And Scrollback"):
 *
 * - InitialItems: replaces the current list for the given (key, generation).
 * - ItemsUpdated: applies diffs if generation matches; silently drops if
 *   generation is stale (Async rule 4: after reset/resync the UI discards
 *   diffs from older generations).
 * - ResyncRequired: clears the list and marks the store as awaiting the next
 *   InitialItems for that key.
 * - ResyncMarker (from EventStreamLag): same as ResyncRequired but global —
 *   all keys are cleared and await InitialItems.
 *
 * Scroll anchoring responsibilities (UI layer; not core):
 *   Before issuing a backward Paginate command, callers capture an anchor
 *   (timelinePaginationAnchorEventId).  After the diff is applied and React
 *   commits to the DOM, callers restore the anchor in a layout effect or RAF.
 *   This store does not do DOM work; it only tracks the item list.
 *
 * Pagination suppression:
 *   The store exposes paginationState per (key, direction).  Callers must
 *   not issue a new Paginate command if the state is "Paginating" or
 *   "EndReached".
 *
 * This is a pure in-memory reducer: no side effects, no Tauri calls.
 */

import type {
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
    paginationForward: "Idle"
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
  switch (event.kind) {
    case "InitialItems":
      return applyInitialItems(store, event);
    case "ItemsUpdated":
      return applyItemsUpdated(store, event);
    case "PaginationStateChanged":
      return applyPaginationStateChanged(store, event);
    case "ResyncRequired":
      return applyResyncRequired(store, event.key);
  }
}

/** Called on EventStreamLag (ResyncMarker): clear all keys. */
export function applyGlobalResync(store: TimelineStoreState): TimelineStoreState {
  const next = new Map<string, TimelineKeyState>();
  for (const [k, state] of store.keys) {
    next.set(k, { ...state, items: [], awaitingResync: true });
  }
  return { keys: next };
}

// ---------------------------------------------------------------------------
// Internal reducers
// ---------------------------------------------------------------------------

function applyInitialItems(
  store: TimelineStoreState,
  event: Extract<TimelineEvent, { kind: "InitialItems" }>
): TimelineStoreState {
  const k = keyStr(event.key);
  const existing = store.keys.get(k) ?? emptyKeyState();
  const next = new Map(store.keys);
  next.set(k, {
    ...existing,
    generation: event.generation,
    items: [...event.items],
    awaitingResync: false
  });
  return { keys: next };
}

function applyItemsUpdated(
  store: TimelineStoreState,
  event: Extract<TimelineEvent, { kind: "ItemsUpdated" }>
): TimelineStoreState {
  const k = keyStr(event.key);
  const existing = store.keys.get(k);

  // Stale generation: discard silently.
  if (!existing || existing.generation !== event.generation) {
    return store;
  }

  // Awaiting resync: discard diffs; we need a fresh InitialItems first.
  if (existing.awaitingResync) {
    return store;
  }

  const updatedItems = applyDiffs(existing.items, event.diffs);
  const next = new Map(store.keys);
  next.set(k, { ...existing, items: updatedItems });
  return { keys: next };
}

function applyPaginationStateChanged(
  store: TimelineStoreState,
  event: Extract<TimelineEvent, { kind: "PaginationStateChanged" }>
): TimelineStoreState {
  const k = keyStr(event.key);
  const existing = store.keys.get(k) ?? emptyKeyState();
  const next = new Map(store.keys);
  const updated: TimelineKeyState =
    event.direction === "Backward"
      ? { ...existing, paginationBackward: event.state }
      : { ...existing, paginationForward: event.state };
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
  next.set(k, { ...existing, items: [], awaitingResync: true });
  return { keys: next };
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
  if ("Clear" in diff) {
    return [];
  }
  if ("Reset" in diff) {
    return [...diff.Reset.items];
  }
  return items;
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
