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
  RoomKeyRequestStage,
  RoomKeyRequestStateDto,
  RoomKeyRequestWithheldCode,
  TimelineDiff,
  TimelineEvent,
  TimelineItem,
  TimelineGapPosition,
  TimelineKey,
  RequestId
} from "./coreEvents";
import {
  timelineItemDomId,
  timelineKeyEquals,
  timelineKeyIdentity
} from "./coreEvents";

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
  /** Keyed by the canonical semantic TimelineKey identity. */
  keys: Map<string, TimelineKeyState>;
}

export const TIMELINE_STORE_INACTIVE_RETAIN_LIMIT = 8;

function keyStr(key: TimelineKey): string {
  return timelineKeyIdentity(key);
}

function timelineKeyKindDiagnosticLabel(key: TimelineKey): "room" | "thread" | "focused" {
  if ("Room" in key.kind) {
    return "room";
  }
  if ("Thread" in key.kind) {
    return "thread";
  }
  return "focused";
}

function stableDiagnosticFingerprint(value: string): string {
  let hash = 0x811c9dc5;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(16).padStart(8, "0");
}

/**
 * Privacy-safe identity shared by the store/application/view diagnostics.
 * Matrix identifiers and message content are never emitted verbatim.
 */
export function timelineKeyDiagnosticIdentity(key: TimelineKey): string {
  const account = stableDiagnosticFingerprint(key.account_key);
  if ("Room" in key.kind) {
    return `kind=room account=${account} room=${stableDiagnosticFingerprint(key.kind.Room.room_id)}`;
  }
  if ("Thread" in key.kind) {
    return `kind=thread account=${account} room=${stableDiagnosticFingerprint(key.kind.Thread.room_id)} target=${stableDiagnosticFingerprint(key.kind.Thread.root_event_id)}`;
  }
  return `kind=focused account=${account} room=${stableDiagnosticFingerprint(key.kind.Focused.room_id)} target=${stableDiagnosticFingerprint(key.kind.Focused.event_id)}`;
}

export function timelineKeyDiagnosticFingerprint(key: TimelineKey): string {
  return stableDiagnosticFingerprint(timelineKeyIdentity(key));
}

function canonicalKeyKindLabel(keyId: string): string | null {
  try {
    const parsed = JSON.parse(keyId) as unknown;
    if (
      Array.isArray(parsed) &&
      parsed.length >= 2 &&
      (parsed[1] === "Room" || parsed[1] === "Thread" || parsed[1] === "Focused")
    ) {
      return parsed[1].toLowerCase();
    }
  } catch {
    // A malformed/legacy key is still counted in store_keys below. Diagnostics
    // deliberately avoid logging the raw value.
  }
  return null;
}

export function timelineStoreInitialItemsDiagnosticMessage(
  before: TimelineStoreState,
  after: TimelineStoreState,
  payload: Extract<TimelineEvent, { InitialItems: unknown }>["InitialItems"],
  retainedKeyIds: ReadonlySet<string>
): string {
  const keyId = keyStr(payload.key);
  const beforeState = before.keys.get(keyId);
  const afterState = after.keys.get(keyId);
  const expectedSizeAfterApply = before.keys.size + (beforeState ? 0 : 1);
  const pruned = Math.max(0, expectedSizeAfterApply - after.keys.size);
  const request = payload.request_id
    ? `${payload.request_id.connection_id}:${payload.request_id.sequence}`
    : "none";
  const targetPresent = timelineProjectionEvidence(payload.key, payload.items).targetPresent;
  return [
    "stage=initial_apply",
    timelineKeyDiagnosticIdentity(payload.key),
    `request=${request}`,
    `actor=${payload.actor_generation ?? 0}`,
    `generation=${payload.generation}`,
    `incoming_items=${payload.items.length}`,
    `before_items=${beforeState?.items.length ?? 0}`,
    `after_items=${afterState?.items.length ?? 0}`,
    `key_outcome=${beforeState ? "replaced" : "created"}`,
    `retained=${retainedKeyIds.has(keyId)}`,
    `pruned=${pruned}`,
    `store_before=${before.keys.size}`,
    `store_after=${after.keys.size}`,
    `target_present=${targetPresent}`
  ].join(" ");
}

export function timelineStoreLookupDiagnosticMessage(
  store: TimelineStoreState,
  key: TimelineKey
): string {
  const state = store.keys.get(keyStr(key));
  const kind = timelineKeyKindDiagnosticLabel(key);
  let sameKindKeys = 0;
  for (const keyId of store.keys.keys()) {
    if (canonicalKeyKindLabel(keyId) === kind) {
      sameKindKeys += 1;
    }
  }
  return [
    "stage=lookup",
    timelineKeyDiagnosticIdentity(key),
    `found=${state !== undefined}`,
    `items=${state?.items.length ?? 0}`,
    `actor=${state?.actorGeneration ?? 0}`,
    `generation=${state?.generation ?? 0}`,
    `awaiting_resync=${state?.awaitingResync ?? false}`,
    `store_keys=${store.keys.size}`,
    `same_kind_keys=${sameKindKeys}`
  ].join(" ");
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
  return { keys: new Map() };
}

function withKeys(store: TimelineStoreState, keys: Map<string, TimelineKeyState>): TimelineStoreState {
  return { ...store, keys };
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
  | {
      kind: "applied";
      requestId: RequestId;
      key: TimelineKey;
      actorGeneration: number;
      generation: number;
      itemCount: number;
      targetPresent: boolean;
    }
  | { kind: "rejectedStale" }
  | { kind: "ignored" };

export function timelineProjectionEvidence(
  key: TimelineKey,
  items: readonly TimelineItem[]
): { itemCount: number; targetPresent: boolean } {
  if (!("Focused" in key.kind)) {
    return { itemCount: items.length, targetPresent: true };
  }
  const target = key.kind.Focused.event_id;
  return {
    itemCount: items.length,
    targetPresent: items.some(
      (item) => "Event" in item.id && item.id.Event.event_id === target
    )
  };
}

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
      actorGeneration,
      generation: payload.generation,
      ...timelineProjectionEvidence(payload.key, payload.items)
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
  // actors replay InitialItems.
  return { ...store, keys: next };
}

/**
 * Apply a Rust-published room-key request state transition (issue #460) to
 * exactly the timeline the event names. The event carries the authoritative
 * `TimelineKey` (including the account), so a transition published for one
 * account can never mutate another account's same room/event. The DTO is
 * Rust-owned and closed; this only updates the displayed item.
 */
export function applyRoomKeyRequestStateChanged(
  store: TimelineStoreState,
  key: TimelineKey,
  eventId: string,
  stage: RoomKeyRequestStage,
  withheldCode: RoomKeyRequestWithheldCode | null
): TimelineStoreState {
  const targetKey = keyStr(key);
  const keyState = store.keys.get(targetKey);
  if (!keyState) {
    return store;
  }
  const requestState: RoomKeyRequestStateDto = { stage, withheldCode };
  let changed = false;
  const items = keyState.items.map((item) => {
    if (timelineItemDomId(item.id) !== eventId) {
      return item;
    }
    changed = true;
    return { ...item, request_state: requestState };
  });
  if (!changed) {
    return store;
  }
  const next = new Map(store.keys);
  next.set(targetKey, { ...keyState, items });
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
  return withKeys(store, next);
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
  const actorChanged = existing.actorGeneration !== (payload.actor_generation ?? 0);
  const indexed = indexedTimelineItems(payload.items);
  const next = new Map(store.keys);
  next.set(k, {
    ...existing,
    generation: payload.generation,
    actorGeneration: payload.actor_generation ?? 0,
    // EndReached belongs to the previous actor's loaded window, not the key forever.
    paginationBackward: actorChanged ? "Idle" : existing.paginationBackward,
    paginationForward: actorChanged ? "Idle" : existing.paginationForward,
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

export type TimelineItemsUpdatedApplication =
  | "applied"
  | "missing_initial"
  | "generation_mismatch"
  | "duplicate_batch"
  | "awaiting_resync";

export function classifyTimelineItemsUpdatedApplication(
  store: TimelineStoreState,
  payload: Extract<TimelineEvent, { ItemsUpdated: unknown }>["ItemsUpdated"]
): TimelineItemsUpdatedApplication {
  const existing = store.keys.get(keyStr(payload.key));
  if (!existing) {
    return "missing_initial";
  }
  if (existing.generation !== payload.generation) {
    return "generation_mismatch";
  }
  if (existing.lastAppliedBatchId !== null && payload.batch_id <= existing.lastAppliedBatchId) {
    return "duplicate_batch";
  }
  if (existing.awaitingResync) {
    return "awaiting_resync";
  }
  return "applied";
}

export function threadTimelineStoreDiagnosticMessage(
  beforeStore: TimelineStoreState,
  afterStore: TimelineStoreState,
  payload: Extract<TimelineEvent, { ItemsUpdated: unknown }>["ItemsUpdated"]
): string {
  const outcome = classifyTimelineItemsUpdatedApplication(beforeStore, payload);
  const beforeState = beforeStore.keys.get(keyStr(payload.key));
  const afterState = afterStore.keys.get(keyStr(payload.key));
  const actorGeneration = beforeState?.actorGeneration ?? afterState?.actorGeneration ?? 0;
  const before = beforeState?.items.length ?? 0;
  const after = afterState?.items.length ?? 0;
  return (
    `stage=store outcome=${outcome} actor=${actorGeneration} generation=${payload.generation} ` +
    `batch=${payload.batch_id} diffs=${payload.diffs.length} before=${before} after=${after}`
  );
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
  let changed = false;
  const next = new Map<string, TimelineKeyState>();
  for (const [key, state] of store.keys) {
    let itemsChanged = false;
    const items = state.items.map((item) => {
      const isHidden = payload.hide_redacted && item.is_redacted;
      if (item.is_hidden === isHidden) return item;
      itemsChanged = true;
      return { ...item, is_hidden: isHidden };
    });
    changed ||= itemsChanged;
    next.set(key, itemsChanged ? { ...state, items } : state);
  }
  return changed ? withKeys(store, next) : store;
}

function applyDisplayLabelsUpdated(
  store: TimelineStoreState,
  payload: Extract<TimelineEvent, { DisplayLabelsUpdated: unknown }>["DisplayLabelsUpdated"]
): TimelineStoreState {
  if (payload.labels.length === 0 || store.keys.size === 0) return store;
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
      itemsChanged ||= updated !== item;
      return updated;
    });
    changed ||= itemsChanged;
    next.set(key, itemsChanged ? { ...state, items } : state);
  }
  return changed ? withKeys(store, next) : store;
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

/**
 * True if any diff in the batch can carry older items projected by a backward
 * pagination. Room timelines prepend them with PushFront; a focused thread
 * timeline keeps its root pinned at index 0, so older replies arrive as Insert
 * at index >= 1 and never satisfy `batchContainsPrepend`.
 */
export function batchContainsBackfillProjection(diffs: TimelineDiff[]): boolean {
  return diffs.some(
    (diff) => diff !== "Clear" && ("PushFront" in diff || "Insert" in diff)
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
