import { describe, expect, test } from "vitest";

import type { TimelineItem } from "./coreEvents";
import { projectNoticeGroups, projectTimelineDisplayRows } from "./timelineDisplayProjection";

function message(id: string, timestamp = 1): TimelineItem {
  return {
    id: { Event: { event_id: id } },
    sender: "@sender:example.invalid",
    body: id,
    timestamp_ms: timestamp,
    in_reply_to_event_id: null,
    thread_root: null,
    thread_summary: null,
    reactions: [],
    can_react: false,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false,
    display_metadata: null
  };
}

describe("Rust timeline display adapter", () => {
  test("preserves Rust-provided order and stable metadata", () => {
    const first = message("$first");
    const root = {
      ...message("$root", 2),
      display_metadata: {
        row_id: "thread-root:$root",
        kind: { kind: "threadRoot" as const },
        content_event_id: "$root",
        activity_event_id: "$reply",
        display_timestamp_ms: 9
      }
    };
    const second = message("$second", 3);

    const eventRows = projectTimelineDisplayRows([second, root, first]).filter(
      (row) => row.kind !== "dateDivider"
    );
    expect(eventRows.map((row) => row.row_id)).toEqual([
      "$second",
      "thread-root:$root",
      "$first"
    ]);
    expect(eventRows[1]).toMatchObject({
      content_event_id: "$root",
      activity_event_id: "$reply",
      display_timestamp_ms: 9,
      kind: "threadRoot"
    });
  });

  test("maps pending and failed Rust rows without synthesizing lifecycle state", () => {
    const pending = {
      ...message("$pending"),
      id: { Synthetic: { synthetic_id: "thread-root-slot:$root" } },
      display_metadata: {
        row_id: "thread-root:$root",
        kind: { kind: "threadRootPending" as const },
        content_event_id: "$root",
        activity_event_id: "$reply",
        display_timestamp_ms: 4
      }
    };
    const failed = {
      ...message("$failed"),
      display_metadata: {
        row_id: "thread-root:$failed",
        kind: { kind: "threadRootFailed" as const, failure_kind: "sdk" as const },
        content_event_id: "$failed",
        activity_event_id: "$reply-failed",
        display_timestamp_ms: 5
      }
    };

    const projectionRows = projectTimelineDisplayRows([pending, failed]).filter(
      (row) => row.kind !== "dateDivider"
    );
    expect(projectionRows.map((row) => row.kind)).toEqual([
      "threadRootPending",
      "threadRootFailed"
    ]);
    expect(projectionRows[0]?.item.id).toEqual({
      Synthetic: { synthetic_id: "thread-root-slot:$root" }
    });
  });

  test("puts a root-less thread placeholder inside its reply's day (#1620)", () => {
    const dayOne = Date.UTC(2027, 0, 1, 12);
    const dayTwo = Date.UTC(2027, 0, 2, 18);
    const before = message("$before", dayOne);
    const after = message("$after", dayTwo + 60_000);
    const divider = (id: string, timestamp: number): TimelineItem => ({
      ...message(id, timestamp),
      id: { Synthetic: { synthetic_id: `date-divider-input-${id}` } }
    });
    const placeholderKinds = [
      { kind: "threadRootPending" as const },
      { kind: "threadRootFailed" as const, failure_kind: "notFound" as const }
    ];

    for (const placeholderKind of placeholderKinds) {
      const placeholder: TimelineItem = {
        ...message("$reply", dayTwo),
        id: { Synthetic: { synthetic_id: "thread-root-slot:$hidden-root" } },
        timestamp_ms: null,
        display_metadata: {
          row_id: "thread-root:$hidden-root",
          kind: placeholderKind,
          content_event_id: "$hidden-root",
          activity_event_id: "$reply",
          display_timestamp_ms: dayTwo
        }
      };

      const rows = projectTimelineDisplayRows([
        divider("$day-one", dayOne),
        before,
        placeholder,
        divider("$day-two", dayTwo),
        after
      ]);

      expect(rows.filter((row) => row.kind === "dateDivider")).toHaveLength(2);
      const dayTwoDivider = rows.findIndex(
        (row) => row.kind === "dateDivider" && row.display_timestamp_ms === dayTwo
      );
      expect(dayTwoDivider, placeholderKind.kind).toBeGreaterThan(-1);
      expect(rows[dayTwoDivider + 1]).toMatchObject({
        kind: placeholderKind.kind,
        row_id: "thread-root:$hidden-root"
      });
      expect(rows[dayTwoDivider + 1]?.display_timestamp_ms).toBe(dayTwo);
    }
  });

  test("preserves renderer date dividers without changing Rust event order", () => {
    const first = message("$a", Date.UTC(2027, 0, 1));
    const second = message("$b", Date.UTC(2027, 0, 2));
    const canonicalDivider = {
      ...message("$divider", Date.UTC(2027, 0, 2)),
      id: { Synthetic: { synthetic_id: "date-divider-input" } }
    };
    const rows = projectTimelineDisplayRows([second, canonicalDivider, first]);
    expect(rows.map((row) => row.kind)).toEqual(["event", "dateDivider", "event"]);
    expect(rows.filter((row) => row.kind === "event").map((row) => row.row_id)).toEqual([
      "$b",
      "$a"
    ]);
  });
});

describe("#2041 notice group collapse", () => {
  function notice(id: string, groupId: string, position: number, count = 4): TimelineItem {
    return {
      ...message(id),
      display_metadata: {
        row_id: id,
        kind: { kind: "event" as const },
        content_event_id: id,
        activity_event_id: id,
        display_timestamp_ms: 1,
        group_count: count,
        group_position: position,
        group_id: groupId
      }
    };
  }

  const notices = () => [
    notice("$a", "group", 0),
    notice("$b", "group", 1),
    notice("$c", "group", 2),
    notice("$d", "group", 3)
  ];
  const rows = () => projectTimelineDisplayRows(notices());

  test("a collapsed group draws its head row only", () => {
    const collapsed = projectNoticeGroups(rows(), new Set(), []);
    expect(collapsed.rows.map((row) => row.row_id)).toEqual(["$a"]);
    expect(collapsed.openGroupIds.size).toBe(0);
  });

  test("an expanded group draws every row", () => {
    const expanded = projectNoticeGroups(rows(), new Set(["group"]), []);
    expect(expanded.rows.map((row) => row.row_id)).toEqual(["$a", "$b", "$c", "$d"]);
    expect([...expanded.openGroupIds]).toEqual(["group"]);
  });

  test("a row the view is pointed at opens its group, and the group reports open", () => {
    const anchored = projectNoticeGroups(rows(), new Set(), ["$c"]);
    expect(anchored.rows.map((row) => row.row_id)).toEqual(["$a", "$b", "$c", "$d"]);
    expect([...anchored.openGroupIds]).toEqual(["group"]);
  });

  test("rows outside a group are never dropped", () => {
    const plain = message("$plain");
    const projected = projectTimelineDisplayRows([...notices(), plain]);
    expect(projectNoticeGroups(projected, new Set(), []).rows.map((row) => row.row_id)).toEqual([
      "$a",
      "$plain"
    ]);
  });
});
