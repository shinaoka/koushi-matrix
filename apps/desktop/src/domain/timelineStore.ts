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
  TimelineGapPosition,
  TimelineKey,
  RequestId,
  ThreadRootProjectionDto
} from "./coreEvents";
import { timelineItemDomId, timelineKeyEquals } from "./coreEvents";

// ---------------------------------------------------------------------------
// Per-key state
// ---------------------------------------------------------------------------

export interface TimelineKeyState {
  /** Current known Core timeline generation. 0 is a valid first generation. */
  generation: number;
  /** Monotonic Core actor owner generation for replacement fencing. */
  actorGeneration: number;
  /** Stable actor-owned projection identity, preserved across replay. */
  projectionRequestId: RequestId | null;
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
  gapPositions: TimelineGapPosition[];
  gapGeneration: number;
}

// ---------------------------------------------------------------------------
// Store type
// ---------------------------------------------------------------------------

export interface TimelineStoreState {
  /** Keyed by JSON.stringify(TimelineKey) for simple equality. */
  keys: Map<string, TimelineKeyState>;
  /** Bounded root snapshots, keyed independently from canonical SDK items. */
  threadRootProjections: Map<string, ThreadRootProjectionDto>;
}

export const TIMELINE_STORE_INACTIVE_RETAIN_LIMIT = 8;

function keyStr(key: TimelineKey): string {
  return JSON.stringify(key);
}

function requestIdsEqual(left: RequestId, right: RequestId): boolean {
  return left.connection_id === right.connection_id && left.sequence === right.sequence;
}

export function timelineStoreKeyId(key: TimelineKey): string {
  return keyStr(key);
}

function emptyKeyState(): TimelineKeyState {
  return {
    generation: 0,
    actorGeneration: 0,
    projectionRequestId: null,
    items: [],
    itemIndexById: new Map(),
    itemIdsByTimestamp: new Map(),
    lastAppliedBatchId: null,
    awaitingResync: true,
    paginationBackward: "Idle",
    paginationForward: "Idle",
    mediaUploadProgress: new Map(),
    gapPositions: [],
    gapGeneration: 0
  };
}

export function createTimelineStore(): TimelineStoreState {
  return { keys: new Map(), threadRootProjections: new Map() };
}

function withKeys(store: TimelineStoreState, keys: Map<string, TimelineKeyState>): TimelineStoreState {
  return {
    ...store,
    keys,
    threadRootProjections: retainActiveThreadRootProjections(store.threadRootProjections, keys)
  };
}

function withRetainedKeys(
  store: TimelineStoreState,
  keys: Map<string, TimelineKeyState>
): TimelineStoreState {
  const retainedPrefixes = [...keys.keys()].map((key) => `${key}\u0000`);
  const threadRootProjections = new Map(
    [...store.threadRootProjections].filter(([projectionKey]) =>
      retainedPrefixes.some((prefix) => projectionKey.startsWith(prefix))
    )
  );
  // Pruning an unrelated timeline must not invalidate TimelineView's display
  // projection memo for a room whose root snapshots were all retained.
  return withKeys(
    {
      ...store,
      threadRootProjections:
        threadRootProjections.size === store.threadRootProjections.size
          ? store.threadRootProjections
          : threadRootProjections
    },
    keys
  );
}

function threadRootProjectionStoreKey(key: TimelineKey, rootEventId: string): string {
  return `${keyStr(key)}\u0000${rootEventId}`;
}

/**
 * A root projection is useful only while its reply is in the bounded canonical
 * Room window. A Core-marked replay-known **ready** root is the sole exception:
 * it may stay without a reply because the bounded display window intentionally
 * omitted the known root. Pending, failed, and ordinary ready entries are
 * removed as soon as their reply leaves. This never mutates canonical items.
 */
function retainActiveThreadRootProjections(
  projections: Map<string, ThreadRootProjectionDto>,
  keys: ReadonlyMap<string, TimelineKeyState>
): Map<string, ThreadRootProjectionDto> {
  const retained = new Map<string, ThreadRootProjectionDto>();
  for (const [projectionKey, projection] of projections) {
    const separator = projectionKey.lastIndexOf("\u0000");
    const timelineKeyId = separator < 0 ? projectionKey : projectionKey.slice(0, separator);
    const state = keys.get(timelineKeyId);
    if (!state) {
      continue;
    }
    if (isReplayKnownReadyProjection(projection)) {
      retained.set(projectionKey, projection);
      continue;
    }
    if (state.items.some((item) => item.thread_root === projection.root_event_id)) {
      retained.set(projectionKey, projection);
    }
  }
  // Preserve the reference when no projection lifecycle changed. TimelineView
  // memoizes display rows by this map, so cloning an unchanged empty map on a
  // pagination-only event would schedule a spurious projection transaction.
  return retained.size === projections.size ? projections : retained;
}

function isReplayKnownReadyProjection(projection: ThreadRootProjectionDto): boolean {
  return (
    projection.retain_without_reply === true &&
    projection.state.kind === "ready" &&
    replayKnownEpoch(projection) !== null
  );
}

function normalizeThreadRootProjection(
  projection: ThreadRootProjectionDto
): ThreadRootProjectionDto {
  if (projection.retain_without_reply !== true || isReplayKnownReadyProjection(projection)) {
    return projection;
  }
  // `retain_without_reply` is an epoch-scoped Core replay-known Ready contract.
  // Never let an arbitrary Ready or malformed Pending/Failed wire payload
  // manufacture an unbounded retention.
  return { ...projection, retain_without_reply: false };
}

function replayKnownEpoch(projection: ThreadRootProjectionDto): number | null {
  const source = projection.source;
  return source?.kind === "replayKnown" && Number.isSafeInteger(source.epoch) && source.epoch > 0
    ? source.epoch
    : null;
}

function projectionSourcesMatch(
  left: ThreadRootProjectionDto,
  right: ThreadRootProjectionDto
): boolean {
  const leftReplayEpoch = replayKnownEpoch(left);
  const rightReplayEpoch = replayKnownEpoch(right);
  if (leftReplayEpoch !== null || rightReplayEpoch !== null) {
    return leftReplayEpoch !== null && leftReplayEpoch === rightReplayEpoch;
  }
  return (left.source?.kind ?? "hydration") === "hydration" &&
    (right.source?.kind ?? "hydration") === "hydration";
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
  if ("ThreadRootProjection" in event) {
    return applyThreadRootProjection(store, event.ThreadRootProjection);
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
  if ("GapPositionsUpdated" in event) {
    return applyGapPositionsUpdated(store, event.GapPositionsUpdated);
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

export type TimelineProjectionApplication =
  | { kind: "applied"; requestId: RequestId; key: TimelineKey; generation: number }
  | { kind: "rejectedStale" }
  | { kind: "ignored" };

export function applyTimelineEventWithProjectionResult(
  store: TimelineStoreState,
  event: TimelineEvent
): { store: TimelineStoreState; projection: TimelineProjectionApplication } {
  if (!("InitialItems" in event) || event.InitialItems.request_id === null) {
    return { store: applyTimelineEvent(store, event), projection: { kind: "ignored" } };
  }
  const payload = event.InitialItems;
  const actorGeneration = payload.actor_generation ?? 0;
  const requestId = payload.request_id;
  if (requestId === null) {
    return { store: applyInitialItems(store, payload), projection: { kind: "ignored" } };
  }
  const existing = store.keys.get(keyStr(payload.key));
  if (
    existing &&
    (actorGeneration < existing.actorGeneration ||
      (actorGeneration === existing.actorGeneration &&
        payload.generation < existing.generation) ||
      (actorGeneration === existing.actorGeneration &&
        payload.generation === existing.generation &&
        payload.request_id !== null &&
        existing.projectionRequestId !== null &&
        !requestIdsEqual(payload.request_id, existing.projectionRequestId)))
  ) {
    return { store, projection: { kind: "rejectedStale" } };
  }
  return {
    store: applyInitialItems(store, payload),
    projection: {
      kind: "applied",
      requestId,
      key: payload.key,
      generation: payload.generation
    }
  };
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

export function applyTimelineEventWithProjectionResultAndRetention(
  store: TimelineStoreState,
  event: TimelineEvent,
  retainedKeyIds: ReadonlySet<string>,
  inactiveLimit = TIMELINE_STORE_INACTIVE_RETAIN_LIMIT
): { store: TimelineStoreState; projection: TimelineProjectionApplication } {
  const applied = applyTimelineEventWithProjectionResult(store, event);
  return {
    ...applied,
    store: pruneTimelineStore(
      applied.store,
      retainedKeyIds,
      timelineEventKeyId(event),
      inactiveLimit
    )
  };
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
  // EventStreamLag temporarily clears canonical rows while already-subscribed
  // actors replay InitialItems. It is not a canonical-window transition, so
  // retained terminal root snapshots must survive until that replay restores
  // their reply activity. True window exits still flow through `withKeys`.
  return { ...store, keys: next };
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
    return movedTouchedKey ? withKeys(store, next) : store;
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
  return withRetainedKeys(store, next);
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
  if ("AnchorRestoreFinished" in event) {
    return keyStr(event.AnchorRestoreFinished.key);
  }
  if ("NavigationUpdated" in event) {
    return keyStr(event.NavigationUpdated.key);
  }
  if ("GapPositionsUpdated" in event) {
    return keyStr(event.GapPositionsUpdated.key);
  }
  if ("GapRepairReleased" in event) {
    return keyStr(event.GapRepairReleased.key);
  }
  if ("SendCompleted" in event) {
    return keyStr(event.SendCompleted.key);
  }
  if ("MediaSendQueued" in event) {
    return keyStr(event.MediaSendQueued.key);
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
  if ("ThreadRootProjection" in event) {
    return keyStr(event.ThreadRootProjection.key);
  }
  if ("ResyncRequired" in event) {
    return keyStr(event.ResyncRequired.key);
  }
  return null;
}

// ---------------------------------------------------------------------------
// Internal reducers
// ---------------------------------------------------------------------------

function applyThreadRootProjection(
  store: TimelineStoreState,
  payload: Extract<TimelineEvent, { ThreadRootProjection: unknown }>[
    "ThreadRootProjection"
  ]
): TimelineStoreState {
  if (!("Room" in payload.key.kind)) {
    return store;
  }
  const projection = normalizeThreadRootProjection(payload.projection);
  const projectionKey = threadRootProjectionStoreKey(payload.key, projection.root_event_id);
  const projections = new Map(store.threadRootProjections);
  if (projection.state.kind === "cleared") {
    const existing = projections.get(projectionKey);
    if (existing === undefined || !projectionSourcesMatch(existing, projection)) {
      return store;
    }
    projections.delete(projectionKey);
    return { ...store, threadRootProjections: projections };
  }
  // Delete before set so a changed terminal result is the most-recent record
  // should future bounded retention diagnostics need map order.
  projections.delete(projectionKey);
  projections.set(projectionKey, projection);
  // Deliberately does not touch `keys`, canonical items, or either index.
  return {
    ...store,
    threadRootProjections: retainActiveThreadRootProjections(projections, store.keys)
  };
}

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
    actorGeneration: payload.actor_generation ?? 0,
    projectionRequestId: payload.request_id,
    items: indexed.items,
    itemIndexById: indexed.itemIndexById,
    itemIdsByTimestamp: indexed.itemIdsByTimestamp,
    lastAppliedBatchId: null,
    awaitingResync: false,
    gapGeneration: 0,
    gapPositions: []
  });
  return withKeys(store, next);
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
  return withKeys(store, next);
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
  return withKeys(store, next);
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
    return withKeys(store, next);
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
  return withKeys(store, next);
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
  return withKeys(store, next);
}

function applyGapPositionsUpdated(
  store: TimelineStoreState,
  payload: Extract<TimelineEvent, { GapPositionsUpdated: unknown }>["GapPositionsUpdated"]
): TimelineStoreState {
  const k = keyStr(payload.key);
  const existing = store.keys.get(k) ?? emptyKeyState();
  if (
    (existing.actorGeneration !== 0 && payload.actor_generation !== existing.actorGeneration) ||
    payload.generation < existing.gapGeneration
  ) {
    return store;
  }
  const next = new Map(store.keys);
  next.set(k, {
    ...existing,
    gapGeneration: payload.generation,
    gapPositions: payload.positions
  });
  return withKeys(store, next);
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
    mediaUploadProgress: new Map(),
    gapPositions: []
  });
  return withKeys(store, next);
}

function applyDisplayPolicyUpdated(
  store: TimelineStoreState,
  payload: Extract<TimelineEvent, { DisplayPolicyUpdated: unknown }>["DisplayPolicyUpdated"]
): TimelineStoreState {
  if (store.keys.size === 0 && store.threadRootProjections.size === 0) {
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

  const nextStore = changed ? withKeys(store, next) : store;
  const threadRootProjections = mapReadyThreadRootProjectionItems(
    nextStore.threadRootProjections,
    (item) => {
      const isHidden = payload.hide_redacted && item.is_redacted;
      return item.is_hidden === isHidden ? item : { ...item, is_hidden: isHidden };
    }
  );
  return threadRootProjections === nextStore.threadRootProjections
    ? nextStore
    : { ...nextStore, threadRootProjections };
}

function applyDisplayLabelsUpdated(
  store: TimelineStoreState,
  payload: Extract<TimelineEvent, { DisplayLabelsUpdated: unknown }>["DisplayLabelsUpdated"]
): TimelineStoreState {
  if (
    payload.labels.length === 0 ||
    (store.keys.size === 0 && store.threadRootProjections.size === 0)
  ) {
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

  const nextStore = changed ? withKeys(store, next) : store;
  const threadRootProjections = mapReadyThreadRootProjectionItems(
    nextStore.threadRootProjections,
    (item) => applyDisplayLabelUpdateToItem(item, labels)
  );
  return threadRootProjections === nextStore.threadRootProjections
    ? nextStore
    : { ...nextStore, threadRootProjections };
}

/**
 * Root snapshots are a separately-owned display input, but their renderable
 * Ready item must receive the same global presentation updates as canonical
 * timeline rows. This maps only that item, never `keys[*].items`.
 */
function mapReadyThreadRootProjectionItems(
  projections: Map<string, ThreadRootProjectionDto>,
  update: (item: TimelineItem) => TimelineItem
): Map<string, ThreadRootProjectionDto> {
  let changed = false;
  const next = new Map<string, ThreadRootProjectionDto>();
  for (const [projectionKey, projection] of projections) {
    if (projection.state.kind !== "ready") {
      next.set(projectionKey, projection);
      continue;
    }
    const item = update(projection.state.item);
    if (item === projection.state.item) {
      next.set(projectionKey, projection);
      continue;
    }
    changed = true;
    next.set(projectionKey, {
      ...projection,
      state: { ...projection.state, item }
    });
  }
  return changed ? next : projections;
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
  if (existingIndex !== undefined && existingIndex !== index) {
    // Overlapping scrollback can produce a diff index for a duplicate Core slot
    // that this render store already collapsed. Update the canonical row without
    // shifting or replacing later slots such as the live-edge item.
    removeTimestampIndex(itemIdsByTimestamp, items[existingIndex]);
    items[existingIndex] = item;
    itemIndexById.set(id, existingIndex);
    addTimestampIndex(itemIdsByTimestamp, item);
    return;
  }
  const previous = items[index];
  removeTimestampIndex(itemIdsByTimestamp, previous);
  const previousId = timelineItemDomId(previous.id);
  if (previousId !== id) {
    itemIndexById.delete(previousId);
  }
  items[index] = item;
  itemIndexById.set(id, index);
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

/**
 * Returns the separately-owned old-root snapshots for one Room timeline.
 * Callers must use these only as display-projection input; they are never
 * canonical diff items and cannot be applied back to `TimelineKeyState`.
 */
export function getThreadRootProjections(
  store: TimelineStoreState,
  key: TimelineKey
): ThreadRootProjectionDto[] {
  if (!("Room" in key.kind)) {
    return [];
  }
  const prefix = `${keyStr(key)}\u0000`;
  return [...store.threadRootProjections.entries()]
    .filter(([projectionKey]) => projectionKey.startsWith(prefix))
    .map(([, projection]) => projection);
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
