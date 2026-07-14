/**
 * Presentation-only room-timeline projection.
 *
 * The timeline store owns the SDK VectorDiff order. This module deliberately
 * receives that order as readonly input and derives a separate row list for
 * rendering; it must never be used as a VectorDiff target.
 */

import type {
  ThreadRootProjectionDto,
  TimelineGapPosition,
  TimelineItem,
  TimelineKey
} from "./coreEvents";
import { timelineItemDomId } from "./coreEvents";
import type { TimelineThreadRootOrder } from "./types";

export type TimelineDisplayRow = {
  /** Stable presentation/virtualization identity, distinct from event identity. */
  row_id: string;
  /** The canonical item that supplies content, actions, and metadata. */
  item: TimelineItem;
  /** Event identity for content actions. */
  content_event_id: string | null;
  /** Event identity for activity placement and viewport facts. */
  activity_event_id: string | null;
  /** Timestamp rendered in the message heading. */
  content_timestamp_ms: number | null;
  /** Timestamp used for presentation placement and date grouping. */
  display_timestamp_ms: number | null;
  kind:
    | "event"
    | "threadRoot"
    | "threadRootPending"
    | "threadRootFailed"
    | "dateDivider"
    | "timelineGap";
};

export function insertTimelineGapRows(
  rows: readonly TimelineDisplayRow[],
  positions: readonly TimelineGapPosition[],
  generation: number
): TimelineDisplayRow[] {
  if (positions.length === 0) {
    return [...rows];
  }
  const result = [...rows];
  for (const gap of [...positions].sort((left, right) => right.before_item_index - left.before_item_index)) {
    const insertionIndex = Math.min(gap.before_item_index, result.length);
    result.splice(insertionIndex, 0, {
      row_id: `timeline-gap-${generation}-${gap.ordinal}`,
      item: timelineGapPlaceholderItem(generation, gap.ordinal),
      content_event_id: null,
      activity_event_id: null,
      content_timestamp_ms: null,
      display_timestamp_ms: null,
      kind: "timelineGap"
    });
  }
  return result;
}

function timelineGapPlaceholderItem(generation: number, ordinal: number): TimelineItem {
  return {
    id: { Synthetic: { synthetic_id: `timeline-gap-${generation}-${ordinal}` } },
    sender: null,
    body: null,
    timestamp_ms: null,
    in_reply_to_event_id: null,
    thread_root: null,
    thread_summary: null,
    reactions: [],
    can_react: false,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false
  };
}

type CanonicalEntry = {
  index: number;
  item: TimelineItem;
  eventId: string | null;
};

type ThreadRoot = CanonicalEntry & {
  eventId: string;
  latestEventId: string | null;
  latestTimestampMs: number | null;
};

type MoveCandidate = {
  root: ThreadRoot;
  activityIndex: number;
  activityEventId: string;
  displayTimestampMs: number;
};

type SummaryFallbackCandidate = {
  row: TimelineDisplayRow;
  displayTimestampMs: number;
  originalIndex: number;
  rootEventId: string;
};

/**
 * Derives render rows from canonical timeline items.
 *
 * RootEvent keeps a Room root/summary at its canonical origin while suppressing
 * standalone thread-reply rows. LatestReply replaces a loaded exact
 * thread-reply slot with the root row. When an SDK room window exposes only a
 * root summary, the same root row is inserted by its summary activity time.
 * Thread, Focused, and non-Room projections remain identity projections.
 */
export function projectTimelineDisplayRows(
  canonicalItems: readonly TimelineItem[],
  key: TimelineKey,
  order: TimelineThreadRootOrder,
  threadRootProjections: readonly ThreadRootProjectionDto[] = []
): TimelineDisplayRow[] {
  if (!("Room" in key.kind)) {
    return canonicalItems.map((item) => canonicalRow(item));
  }

  if (order.kind === "rootEvent") {
    return projectRootEventRoomRows(canonicalItems);
  }

  return projectLatestReplyRoomRows(canonicalItems, threadRootProjections);
}

function projectRootEventRoomRows(canonicalItems: readonly TimelineItem[]): TimelineDisplayRow[] {
  const rows: TimelineDisplayRow[] = [];
  let pendingDateDivider: TimelineItem | null = null;

  for (const item of canonicalItems) {
    if (isCanonicalDateDivider(item)) {
      // Canonical divider rows precede their date's first item. Keep one only
      // when that next visible Room item survives reply suppression; do not
      // manufacture dividers for ordinary timelines that did not have one.
      pendingDateDivider = item;
      continue;
    }
    if (isThreadReply(item)) {
      continue;
    }
    if (pendingDateDivider !== null) {
      rows.push(canonicalRow(pendingDateDivider));
      pendingDateDivider = null;
    }
    rows.push(canonicalRow(item));
  }

  return rows;
}

function projectLatestReplyRoomRows(
  canonicalItems: readonly TimelineItem[],
  threadRootProjections: readonly ThreadRootProjectionDto[]
): TimelineDisplayRow[] {
  const entries = canonicalItems.map((item, index) => ({
    index,
    item,
    eventId: eventIdFor(item)
  }));
  const rootsByIndex = new Map<number, ThreadRoot>();
  const duplicateRootIndexes = new Set<number>();
  const rootIds = new Set<string>();

  for (const entry of entries) {
    const root = asThreadRoot(entry);
    if (root === null) {
      continue;
    }
    if (rootIds.has(root.eventId)) {
      // A healthy canonical store deduplicates item IDs. If a malformed input
      // contains a second root, retain the first canonical occurrence only.
      duplicateRootIndexes.add(entry.index);
      continue;
    }
    rootIds.add(root.eventId);
    rootsByIndex.set(entry.index, root);
  }

  const movedRootByActivityIndex = chooseMoves(rootsByIndex, entries);
  const hydratedRootByActivityIndex = chooseHydratedRoots(
    entries,
    rootIds,
    threadRootProjections
  );
  const exactMovedRootIds = new Set(
    [...movedRootByActivityIndex.values()].map((candidate) => candidate.root.eventId)
  );
  const exactHydratedRootIds = new Set(
    [...hydratedRootByActivityIndex.values()].flatMap((row) =>
      row.content_event_id === null ? [] : [row.content_event_id]
    )
  );
  const summaryFallbacks = [
    ...chooseSummaryFallbackRoots(rootsByIndex, exactMovedRootIds),
    ...chooseHydratedSummaryFallbacks(
      entries,
      rootIds,
      exactHydratedRootIds,
      threadRootProjections
    )
  ];
  const summaryFallbackRootIds = new Set(summaryFallbacks.map((candidate) => candidate.rootEventId));
  const movedRootIndexes = new Set(
    [...movedRootByActivityIndex.values()].map((candidate) => candidate.root.index)
  );
  const projectedRows: TimelineDisplayRow[] = [];

  for (const entry of entries) {
    if (isCanonicalDateDivider(entry.item)) {
      // SDK dividers describe canonical chronology; presentation dividers are
      // rebuilt below using a moved root's activity timestamp.
      continue;
    }
    if (duplicateRootIndexes.has(entry.index)) {
      continue;
    }
    if (isThreadReply(entry.item)) {
      const moved = movedRootByActivityIndex.get(entry.index);
      if (moved !== undefined) {
        projectedRows.push(movedRootRow(moved));
      } else {
        const hydrated = hydratedRootByActivityIndex.get(entry.index);
        if (hydrated !== undefined) {
          projectedRows.push(hydrated);
        }
      }
      // Room presentation represents a thread by its root/summary block, not
      // by any standalone reply row, including an unmatched stale reply.
      continue;
    }
    if (movedRootIndexes.has(entry.index) || summaryFallbackRootIds.has(entry.eventId ?? "")) {
      continue;
    }

    const root = rootsByIndex.get(entry.index);
    projectedRows.push(root === undefined ? canonicalRow(entry.item) : rootAtOriginRow(root));
  }

  return rebuildDateDividers(insertSummaryFallbackRows(projectedRows, summaryFallbacks, entries));
}

function chooseHydratedRoots(
  entries: readonly CanonicalEntry[],
  loadedRootIds: ReadonlySet<string>,
  projections: readonly ThreadRootProjectionDto[]
): Map<number, TimelineDisplayRow> {
  const selected = new Map<number, TimelineDisplayRow>();
  const canonicalEventIds = new Set(
    entries.flatMap((entry) => (entry.eventId === null ? [] : [entry.eventId]))
  );

  for (const projection of projections) {
    if (projection.state.kind === "cleared") {
      continue;
    }
    if (loadedRootIds.has(projection.root_event_id) || canonicalEventIds.has(projection.root_event_id)) {
      continue;
    }
    const activity = entries.find(
      (entry) =>
        entry.eventId === projection.activity_event_id &&
        entry.item.thread_root === projection.root_event_id
    );
    if (activity === undefined || selected.has(activity.index)) {
      continue;
    }
    const displayTimestampMs =
      finiteTimestamp(activity.item.timestamp_ms) ??
      finiteTimestamp(projection.activity_timestamp_ms);
    if (displayTimestampMs === null) {
      continue;
    }
    selected.set(activity.index, hydratedRootRow(projection, displayTimestampMs));
  }
  return selected;
}

function chooseSummaryFallbackRoots(
  rootsByIndex: ReadonlyMap<number, ThreadRoot>,
  exactMovedRootIds: ReadonlySet<string>
): SummaryFallbackCandidate[] {
  const fallbacks: SummaryFallbackCandidate[] = [];
  for (const root of rootsByIndex.values()) {
    if (exactMovedRootIds.has(root.eventId) || root.latestEventId === null) {
      continue;
    }
    const displayTimestampMs = finiteTimestamp(root.latestTimestampMs);
    if (displayTimestampMs === null) {
      continue;
    }
    fallbacks.push({
      row: summaryOnlyRootRow(root, displayTimestampMs),
      displayTimestampMs,
      originalIndex: root.index,
      rootEventId: root.eventId
    });
  }
  return fallbacks;
}

function chooseHydratedSummaryFallbacks(
  entries: readonly CanonicalEntry[],
  loadedRootIds: ReadonlySet<string>,
  exactHydratedRootIds: ReadonlySet<string>,
  projections: readonly ThreadRootProjectionDto[]
): SummaryFallbackCandidate[] {
  const canonicalEventIds = new Set(
    entries.flatMap((entry) => (entry.eventId === null ? [] : [entry.eventId]))
  );
  const fallbacks: SummaryFallbackCandidate[] = [];

  for (const projection of projections) {
    if (
      projection.state.kind !== "ready" ||
      !isReplayKnownSummaryFallback(projection) ||
      loadedRootIds.has(projection.root_event_id) ||
      canonicalEventIds.has(projection.root_event_id) ||
      exactHydratedRootIds.has(projection.root_event_id) ||
      !projection.activity_event_id.trim()
    ) {
      continue;
    }
    const displayTimestampMs =
      finiteTimestamp(projection.activity_timestamp_ms) ??
      finiteTimestamp(projection.state.item.thread_summary?.latest_timestamp_ms ?? null);
    if (displayTimestampMs === null) {
      continue;
    }
    fallbacks.push({
      row: hydratedRootRow(projection, displayTimestampMs),
      displayTimestampMs,
      originalIndex: Number.MAX_SAFE_INTEGER,
      rootEventId: projection.root_event_id
    });
  }
  return fallbacks;
}

/**
 * Only the bounded replay path may create a root row when there is no
 * canonical reply slot. Hydration Ready records remain valid for replacing an
 * exact reply, but must not invent an out-of-band summary row after a global
 * resync or in any other canonical-empty interval.
 */
function isReplayKnownSummaryFallback(projection: ThreadRootProjectionDto): boolean {
  const source = projection.source;
  return (
    projection.retain_without_reply === true &&
    source?.kind === "replayKnown" &&
    Number.isSafeInteger(source.epoch) &&
    source.epoch > 0
  );
}

function chooseMoves(
  rootsByIndex: ReadonlyMap<number, ThreadRoot>,
  entries: readonly CanonicalEntry[]
): Map<number, MoveCandidate> {
  const candidatesByActivityIndex = new Map<number, MoveCandidate[]>();

  for (const root of rootsByIndex.values()) {
    if (root.latestEventId === null) {
      continue;
    }
    const activity = entries.find(
      (entry) =>
        entry.eventId === root.latestEventId && entry.item.thread_root === root.eventId
    );
    if (activity === undefined) {
      continue;
    }
    const displayTimestampMs =
      finiteTimestamp(activity.item.timestamp_ms) ?? finiteTimestamp(root.latestTimestampMs);
    if (displayTimestampMs === null) {
      // A latest event ID without a usable activity timestamp is incomplete
      // data. Keeping the root at its origin avoids a partial-summary flicker.
      continue;
    }
    const candidate: MoveCandidate = {
      root,
      activityIndex: activity.index,
      activityEventId: root.latestEventId,
      displayTimestampMs
    };
    const candidates = candidatesByActivityIndex.get(activity.index) ?? [];
    candidates.push(candidate);
    candidatesByActivityIndex.set(activity.index, candidates);
  }

  const selected = new Map<number, MoveCandidate>();
  for (const [activityIndex, candidates] of candidatesByActivityIndex) {
    candidates.sort(compareMoveCandidates);
    selected.set(activityIndex, candidates[0]);
  }
  return selected;
}

function compareMoveCandidates(left: MoveCandidate, right: MoveCandidate): number {
  return (
    left.activityIndex - right.activityIndex ||
    left.displayTimestampMs - right.displayTimestampMs ||
    left.root.index - right.root.index ||
    left.root.eventId.localeCompare(right.root.eventId)
  );
}

function canonicalRow(item: TimelineItem): TimelineDisplayRow {
  if (isCanonicalDateDivider(item)) {
    return {
      row_id: timelineItemDomId(item.id),
      item,
      kind: "dateDivider",
      content_event_id: null,
      activity_event_id: null,
      content_timestamp_ms: null,
      display_timestamp_ms: finiteTimestamp(item.timestamp_ms)
    };
  }

  const eventId = eventIdFor(item);
  const root = hasThreadSummaryRoot(item, eventId);
  const timestampMs = finiteTimestamp(item.timestamp_ms);
  return {
    row_id: root && eventId !== null ? `thread-root:${eventId}` : timelineItemDomId(item.id),
    item,
    kind: root ? "threadRoot" : "event",
    content_event_id: eventId,
    activity_event_id: eventId,
    content_timestamp_ms: timestampMs,
    display_timestamp_ms: timestampMs
  };
}

function rootAtOriginRow(root: ThreadRoot): TimelineDisplayRow {
  const timestampMs = finiteTimestamp(root.item.timestamp_ms);
  return {
    row_id: `thread-root:${root.eventId}`,
    item: root.item,
    kind: "threadRoot",
    content_event_id: root.eventId,
    activity_event_id: root.eventId,
    content_timestamp_ms: timestampMs,
    display_timestamp_ms: timestampMs
  };
}

function movedRootRow(candidate: MoveCandidate): TimelineDisplayRow {
  return {
    row_id: `thread-root:${candidate.root.eventId}`,
    item: candidate.root.item,
    kind: "threadRoot",
    content_event_id: candidate.root.eventId,
    activity_event_id: candidate.activityEventId,
    content_timestamp_ms: finiteTimestamp(candidate.root.item.timestamp_ms),
    display_timestamp_ms: candidate.displayTimestampMs
  };
}

function summaryOnlyRootRow(root: ThreadRoot, displayTimestampMs: number): TimelineDisplayRow {
  return {
    row_id: `thread-root:${root.eventId}`,
    item: root.item,
    kind: "threadRoot",
    content_event_id: root.eventId,
    activity_event_id: root.latestEventId,
    content_timestamp_ms: finiteTimestamp(root.item.timestamp_ms),
    display_timestamp_ms: displayTimestampMs
  };
}

function hydratedRootRow(
  projection: ThreadRootProjectionDto,
  displayTimestampMs: number
): TimelineDisplayRow {
  const state = projection.state;
  const item = state.kind === "ready" ? state.item : hydratedPlaceholderItem(projection);
  return {
    row_id: `thread-root:${projection.root_event_id}`,
    item,
    kind:
      state.kind === "pending"
        ? "threadRootPending"
        : state.kind === "failed"
          ? "threadRootFailed"
          : "threadRoot",
    content_event_id: projection.root_event_id,
    activity_event_id: projection.activity_event_id,
    content_timestamp_ms: finiteTimestamp(item.timestamp_ms),
    display_timestamp_ms: displayTimestampMs
  };
}

function hydratedPlaceholderItem(projection: ThreadRootProjectionDto): TimelineItem {
  return {
    id: { Event: { event_id: projection.root_event_id } },
    sender: null,
    body: null,
    timestamp_ms: null,
    in_reply_to_event_id: null,
    thread_root: null,
    thread_summary: {
      reply_count: 1,
      latest_event_id: projection.activity_event_id,
      latest_sender: null,
      latest_body_preview: null,
      latest_timestamp_ms: projection.activity_timestamp_ms
    },
    reactions: [],
    can_react: false,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false
  };
}

function insertSummaryFallbackRows(
  rows: TimelineDisplayRow[],
  fallbacks: readonly SummaryFallbackCandidate[],
  entries: readonly CanonicalEntry[]
): TimelineDisplayRow[] {
  const originalIndexByRowId = new Map<string, number>();
  for (const entry of entries) {
    const root = asThreadRoot(entry);
    const rowId = root === null ? timelineItemDomId(entry.item.id) : `thread-root:${root.eventId}`;
    originalIndexByRowId.set(rowId, entry.index);
  }
  const inserted = [...rows];
  for (const fallback of [...fallbacks].sort(compareSummaryFallbackCandidates)) {
    const insertionIndex = inserted.findIndex((row) =>
      compareSummaryFallbackToRow(fallback, row, originalIndexByRowId) <= 0
    );
    inserted.splice(insertionIndex < 0 ? inserted.length : insertionIndex, 0, fallback.row);
  }
  return inserted;
}

function compareSummaryFallbackCandidates(
  left: SummaryFallbackCandidate,
  right: SummaryFallbackCandidate
): number {
  return (
    left.displayTimestampMs - right.displayTimestampMs ||
    left.originalIndex - right.originalIndex ||
    left.rootEventId.localeCompare(right.rootEventId)
  );
}

function compareSummaryFallbackToRow(
  fallback: SummaryFallbackCandidate,
  row: TimelineDisplayRow,
  originalIndexByRowId: ReadonlyMap<string, number>
): number {
  const rowTimestampMs = row.display_timestamp_ms;
  if (rowTimestampMs === null) {
    return 1;
  }
  const rowOriginalIndex = originalIndexByRowId.get(row.row_id) ?? Number.MAX_SAFE_INTEGER;
  return (
    fallback.displayTimestampMs - rowTimestampMs ||
    fallback.originalIndex - rowOriginalIndex ||
    fallback.rootEventId.localeCompare(row.content_event_id ?? row.row_id)
  );
}

function rebuildDateDividers(rows: readonly TimelineDisplayRow[]): TimelineDisplayRow[] {
  const rebuilt: TimelineDisplayRow[] = [];
  let previousDateKey: string | null = null;
  let dividerOrdinal = 0;

  for (const row of rows) {
    const timestampMs = row.display_timestamp_ms;
    if (isDateDividerSource(row) && timestampMs !== null) {
      const dateKey = localDateKey(timestampMs);
      if (dateKey !== previousDateKey) {
        rebuilt.push(dateDividerRow(timestampMs, dividerOrdinal));
        dividerOrdinal += 1;
        previousDateKey = dateKey;
      }
    }
    rebuilt.push(row);
  }

  return rebuilt;
}

function isDateDividerSource(row: TimelineDisplayRow): boolean {
  return (
    row.kind !== "dateDivider" &&
    !row.item.is_hidden &&
    ("Event" in row.item.id || "Transaction" in row.item.id)
  );
}

function dateDividerRow(timestampMs: number, ordinal: number): TimelineDisplayRow {
  const item: TimelineItem = {
    id: { Synthetic: { synthetic_id: `date-divider-${timestampMs}` } },
    sender: null,
    body: null,
    timestamp_ms: timestampMs,
    in_reply_to_event_id: null,
    thread_root: null,
    thread_summary: null,
    reactions: [],
    can_react: false,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false
  };
  return {
    row_id: `date-divider:${localDateKey(timestampMs)}:${ordinal}`,
    item,
    kind: "dateDivider",
    content_event_id: null,
    activity_event_id: null,
    content_timestamp_ms: null,
    display_timestamp_ms: timestampMs
  };
}

function asThreadRoot(entry: CanonicalEntry): ThreadRoot | null {
  if (!hasThreadSummaryRoot(entry.item, entry.eventId) || entry.eventId === null) {
    return null;
  }
  const summary = entry.item.thread_summary;
  if (summary === null) {
    return null;
  }
  return {
    ...entry,
    eventId: entry.eventId,
    latestEventId: nonEmptyTrimmedEventId(summary.latest_event_id),
    latestTimestampMs: summary.latest_timestamp_ms
  };
}

function hasThreadSummaryRoot(item: TimelineItem, eventId: string | null): boolean {
  return eventId !== null && item.thread_summary !== null && item.thread_root === null;
}

function isThreadReply(item: TimelineItem): boolean {
  return item.thread_root !== null;
}

function isCanonicalDateDivider(item: TimelineItem): boolean {
  return "Synthetic" in item.id && item.id.Synthetic.synthetic_id.startsWith("date-divider-");
}

function eventIdFor(item: TimelineItem): string | null {
  return "Event" in item.id ? item.id.Event.event_id : null;
}

function finiteTimestamp(timestampMs: number | null): number | null {
  return timestampMs !== null && Number.isFinite(timestampMs) ? timestampMs : null;
}

function nonEmptyTrimmedEventId(eventId: string | null): string | null {
  const trimmed = eventId?.trim() ?? "";
  return trimmed || null;
}

function localDateKey(timestampMs: number): string {
  const date = new Date(timestampMs);
  return `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
}
