/**
 * Render adapter for Rust-owned timeline display items.
 *
 * Rust owns thread placement, suppression, row identity and display timestamps.
 * This module only maps those fields to the renderer and adds presentation-only
 * date dividers and gap placeholders.
 */

import type {
  TimelineGapId,
  TimelineGapPosition,
  TimelineItem
} from "./coreEvents";
import { timelineItemDomId } from "./coreEvents";

export type TimelineDisplayRow = {
  row_id: string;
  item: TimelineItem;
  content_event_id: string | null;
  activity_event_id: string | null;
  gap_id: TimelineGapId | null;
  content_timestamp_ms: number | null;
  display_timestamp_ms: number | null;
  kind:
    | "event"
    | "threadRoot"
    | "threadRootPending"
    | "threadRootFailed"
    | "dateDivider"
    | "timelineGap";
};

export function insertTimelineGapItems(
  items: readonly TimelineItem[],
  positions: readonly TimelineGapPosition[],
  generation: number
): TimelineItem[] {
  if (positions.length === 0) return [...items];
  const result = [...items];
  for (const gap of [...positions].sort(
    (left, right) => right.before_item_index - left.before_item_index
  )) {
    result.splice(
      Math.min(gap.before_item_index, result.length),
      0,
      timelineGapPlaceholderItem(generation, gap.id)
    );
  }
  return result;
}

type TimelineGapPlaceholderItem = TimelineItem & { gap_id: TimelineGapId };

function timelineGapPlaceholderItem(
  generation: number,
  gapId: TimelineGapId
): TimelineGapPlaceholderItem {
  return {
    id: {
      Synthetic: {
        synthetic_id: `timeline-gap-${generation}-${gapId.topology_revision}-${gapId.ordinal}`
      }
    },
    gap_id: gapId,
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

/** #2041: which notice groups a view draws open, and the rows that follow. */
export type NoticeGroupProjection = {
  rows: TimelineDisplayRow[];
  openGroupIds: ReadonlySet<string>;
};

/**
 * #2041: the rows a collapsed notice group draws.
 *
 * Core decides which rows form a group and carries the count; the renderer only
 * picks how many of them are drawn. A collapsed group keeps its head row (the
 * one with the count) and drops the rest, so a burst of four profile notices is
 * one line.
 *
 * A group is open when the user expanded it, and also when it holds a row the
 * view is being pointed at — a navigation target — because that row must exist
 * to be scrolled to. The open set is returned with the rows so the head row and
 * its members report the same state: a group that is drawn open but reports
 * itself closed would answer the disclosure button with no visible change.
 */
export function projectNoticeGroups(
  rows: readonly TimelineDisplayRow[],
  expandedGroupIds: ReadonlySet<string>,
  targetEventIds: readonly (string | null)[]
): NoticeGroupProjection {
  const targets = new Set(targetEventIds.filter((id): id is string => id !== null));
  const openGroupIds = new Set(expandedGroupIds);
  for (const row of rows) {
    const groupId = row.item.display_metadata?.group_id;
    if (!groupId) continue;
    if (
      (row.content_event_id !== null && targets.has(row.content_event_id)) ||
      (row.activity_event_id !== null && targets.has(row.activity_event_id))
    ) {
      openGroupIds.add(groupId);
    }
  }
  return {
    rows: rows.filter((row) => {
      const metadata = row.item.display_metadata;
      const groupId = metadata?.group_id;
      if (!groupId || (metadata?.group_position ?? 0) === 0) return true;
      return openGroupIds.has(groupId);
    }),
    openGroupIds
  };
}

export function projectTimelineDisplayRows(items: readonly TimelineItem[]): TimelineDisplayRow[] {
  const rows = items.map((item) => displayRowFromItem(item));
  const latestReplyPlacement = rows.some(
    (row) =>
      (row.kind === "threadRoot" ||
        row.kind === "threadRootPending" ||
        row.kind === "threadRootFailed") &&
      row.content_event_id !== null &&
      row.activity_event_id !== null &&
      row.content_event_id !== row.activity_event_id
  );
  return latestReplyPlacement && rows.some((row) => row.kind === "dateDivider")
    ? rebuildDateDividers(rows.filter((row) => row.kind !== "dateDivider"))
    : rows;
}

function displayRowFromItem(item: TimelineItem): TimelineDisplayRow {
  const metadata = item.display_metadata;
  const gapId = isTimelineGapPlaceholder(item) ? timelineGapIdForPlaceholder(item) : null;
  if (gapId !== null) {
    return {
      row_id: metadata?.row_id ?? timelineItemDomId(item.id),
      item,
      kind: "timelineGap",
      content_event_id: null,
      activity_event_id: null,
      gap_id: gapId,
      content_timestamp_ms: null,
      display_timestamp_ms: null
    };
  }
  if (isDateDivider(item)) {
    return {
      row_id: metadata?.row_id ?? timelineItemDomId(item.id),
      item,
      kind: "dateDivider",
      content_event_id: null,
      activity_event_id: null,
      gap_id: null,
      content_timestamp_ms: null,
      display_timestamp_ms: item.timestamp_ms
    };
  }

  const eventId = eventIdFor(item);
  const kind = metadata?.kind.kind ?? "event";
  return {
    row_id: metadata?.row_id ?? timelineItemDomId(item.id),
    item,
    kind:
      kind === "threadRootPending"
        ? "threadRootPending"
        : kind === "threadRootFailed"
          ? "threadRootFailed"
          : kind === "threadRoot"
            ? "threadRoot"
            : "event",
    content_event_id: metadata?.content_event_id ?? eventId,
    activity_event_id: metadata?.activity_event_id ?? eventId,
    gap_id: null,
    content_timestamp_ms: item.timestamp_ms,
    display_timestamp_ms: metadata?.display_timestamp_ms ?? item.timestamp_ms
  };
}

function rebuildDateDividers(rows: readonly TimelineDisplayRow[]): TimelineDisplayRow[] {
  const rebuilt: TimelineDisplayRow[] = [];
  let previousDateKey: string | null = null;
  let dividerOrdinal = 0;

  for (const row of rows) {
    const timestampMs = finiteTimestamp(row.display_timestamp_ms);
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
    !row.item.is_hidden &&
    ("Event" in row.item.id ||
      "Transaction" in row.item.id ||
      // A root-less thread placeholder is a Synthetic row positioned at its
      // latest reply, and Rust gives it that reply's display timestamp. It
      // must start its reply's day (#1620) instead of staying under the
      // previous day until the next real event supplies a separator.
      row.kind === "threadRoot" ||
      row.kind === "threadRootPending" ||
      row.kind === "threadRootFailed")
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
    gap_id: null,
    content_timestamp_ms: null,
    display_timestamp_ms: timestampMs
  };
}

function localDateKey(timestampMs: number): string {
  const date = new Date(timestampMs);
  return `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
}

function isDateDivider(item: TimelineItem): boolean {
  return (
    "Synthetic" in item.id &&
    item.id.Synthetic.synthetic_id.startsWith("date-divider-")
  );
}

function isTimelineGapPlaceholder(item: TimelineItem): boolean {
  return (
    "Synthetic" in item.id &&
    item.id.Synthetic.synthetic_id.startsWith("timeline-gap-")
  );
}

function timelineGapIdForPlaceholder(item: TimelineItem): TimelineGapId | null {
  if (!isTimelineGapPlaceholder(item) || !("gap_id" in item)) return null;
  return (item as TimelineGapPlaceholderItem).gap_id;
}

function eventIdFor(item: TimelineItem): string | null {
  return "Event" in item.id ? item.id.Event.event_id : null;
}

export function finiteTimestamp(timestampMs: number | null): number | null {
  return typeof timestampMs === "number" && Number.isFinite(timestampMs)
    ? timestampMs
    : null;
}
