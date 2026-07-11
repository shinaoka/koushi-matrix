import { describe, expect, test } from "vitest";

import {
  focusedTimelineKey,
  roomTimelineKey,
  threadTimelineKey,
  timelineItemDomId,
  type TimelineItem
} from "./coreEvents";
import type { ThreadRootProjectionDto } from "./coreEvents";
import { projectTimelineDisplayRows } from "./timelineDisplayProjection";
import type { TimelineThreadRootOrder } from "./types";

const ACCOUNT_KEY = "@projection:example.invalid";
const ROOM_ID = "!projection:example.invalid";
const ROOM_KEY = roomTimelineKey(ACCOUNT_KEY, ROOM_ID);
const LATEST_REPLY: TimelineThreadRootOrder = { kind: "latestReply" };
const ROOT_EVENT: TimelineThreadRootOrder = { kind: "rootEvent" };

function event(
  eventId: string,
  timestampMs: number,
  overrides: Partial<TimelineItem> = {}
): TimelineItem {
  return {
    id: { Event: { event_id: eventId } },
    sender: "@sender:example.invalid",
    body: eventId,
    timestamp_ms: timestampMs,
    in_reply_to_event_id: null,
    thread_root: null,
    thread_summary: null,
    reactions: [],
    can_react: true,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false,
    ...overrides
  };
}

function root(
  eventId: string,
  timestampMs: number,
  latestEventId: string | null,
  latestTimestampMs: number | null,
  overrides: Partial<TimelineItem> = {}
): TimelineItem {
  return event(eventId, timestampMs, {
    ...overrides,
    thread_summary: {
      reply_count: latestEventId === null ? 0 : 1,
      latest_event_id: latestEventId,
      latest_sender: "@reply:example.invalid",
      latest_body_preview: "latest reply",
      latest_timestamp_ms: latestTimestampMs
    }
  });
}

function reply(
  eventId: string,
  rootEventId: string,
  timestampMs: number,
  overrides: Partial<TimelineItem> = {}
): TimelineItem {
  return event(eventId, timestampMs, { ...overrides, thread_root: rootEventId });
}

function transaction(transactionId: string, timestampMs: number): TimelineItem {
  return {
    ...event(`local:${transactionId}`, timestampMs),
    id: { Transaction: { transaction_id: transactionId } }
  };
}

function canonicalDateDivider(timestampMs: number): TimelineItem {
  return {
    ...event(`divider:${timestampMs}`, timestampMs),
    id: { Synthetic: { synthetic_id: `date-divider-${timestampMs}` } },
    sender: null,
    body: null,
    can_react: false
  };
}

function materialRows(rows: ReturnType<typeof projectTimelineDisplayRows>) {
  return rows.filter((row) => row.kind !== "dateDivider");
}

function materialRowIds(rows: ReturnType<typeof projectTimelineDisplayRows>) {
  return materialRows(rows).map((row) => row.row_id);
}

describe("timeline display projection", () => {
  test("uses one stable summary-only row while an old root is pending, then replaces it in place", () => {
    const latestReply = reply("$latest", "$old-root", 1_800_000_010_000);
    const canonical = [event("$normal", 1_800_000_000_000), latestReply];
    const pending: ThreadRootProjectionDto = {
      root_event_id: "$old-root",
      activity_event_id: "$latest",
      activity_timestamp_ms: 1_800_000_010_000,
      state: { kind: "pending" }
    };
    const ready: ThreadRootProjectionDto = {
      ...pending,
      state: { kind: "ready", item: root("$old-root", 1_700_000_000_000, "$latest", 1_800_000_010_000) }
    };
    const failed: ThreadRootProjectionDto = {
      ...pending,
      state: { kind: "failed", failure_kind: "notFound" }
    };

    const pendingRows = projectTimelineDisplayRows(canonical, ROOM_KEY, LATEST_REPLY, [pending]);
    const readyRows = projectTimelineDisplayRows(canonical, ROOM_KEY, LATEST_REPLY, [ready]);
    const failedRows = projectTimelineDisplayRows(canonical, ROOM_KEY, LATEST_REPLY, [failed]);

    for (const rows of [pendingRows, readyRows, failedRows]) {
      expect(materialRowIds(rows)).toEqual(["$normal", "thread-root:$old-root"]);
      expect(rows.find((row) => row.row_id === "thread-root:$old-root")?.activity_event_id).toBe(
        "$latest"
      );
    }
    expect(pendingRows.find((row) => row.row_id === "thread-root:$old-root")?.kind).toBe(
      "threadRootPending"
    );
    expect(readyRows.find((row) => row.row_id === "thread-root:$old-root")?.item.body).toBe(
      "$old-root"
    );
    expect(failedRows.find((row) => row.row_id === "thread-root:$old-root")?.kind).toBe(
      "threadRootFailed"
    );
    expect(canonical).toEqual([event("$normal", 1_800_000_000_000), latestReply]);
  });

  test("uses the remaining older reply when a ready or failed root activity moves backward", () => {
    const olderReply = reply("$older", "$old-root", 1_800_000_005_000);
    const canonical = [event("$normal", 1_800_000_000_000), olderReply];
    for (const state of [
      { kind: "ready" as const, item: root("$old-root", 1_700_000_000_000, "$older", 1_800_000_005_000) },
      { kind: "failed" as const, failure_kind: "notFound" as const }
    ]) {
      const rows = projectTimelineDisplayRows(canonical, ROOM_KEY, LATEST_REPLY, [
        {
          root_event_id: "$old-root",
          activity_event_id: "$older",
          activity_timestamp_ms: 1_800_000_005_000,
          state
        }
      ]);
      expect(materialRowIds(rows)).toEqual(["$normal", "thread-root:$old-root"]);
      expect(rows.find((row) => row.row_id === "thread-root:$old-root")?.activity_event_id).toBe(
        "$older"
      );
    }
  });

  test("RootEvent preserves the exact canonical presentation and item references", () => {
    const threadRoot = root("$root", 1_800_000_000_000, "$reply", 1_800_000_020_000);
    const latestReply = reply("$reply", "$root", 1_800_000_020_000);
    const items = [canonicalDateDivider(1_800_000_000_000), threadRoot, latestReply];
    const beforeIds = items.map((item) => timelineItemDomId(item.id));

    const rows = projectTimelineDisplayRows(items, ROOM_KEY, ROOT_EVENT);

    expect(rows).toHaveLength(items.length);
    expect(rows.map((row) => row.item)).toEqual(items);
    expect(rows.map((row) => row.item)).toStrictEqual(items);
    expect(rows[1].item).toBe(threadRoot);
    expect(rows[2].item).toBe(latestReply);
    expect(items.map((item) => timelineItemDomId(item.id))).toEqual(beforeIds);
  });

  test("LatestReply replaces the exact reply slot with one unchanged root block", () => {
    const threadRoot = root("$root", 1_800_000_000_000, "$latest", 1_800_000_030_000, {
      body: "original root body",
      reactions: [
        {
          key: "👍",
          count: 2,
          reacted_by_me: false,
          my_reaction_event_id: null,
          sender_preview: ["@one:example.invalid", "@two:example.invalid"]
        }
      ]
    });
    const earlierReply = reply("$earlier", "$root", 1_800_000_010_000);
    const normal = event("$normal", 1_800_000_020_000);
    const latestReply = reply("$latest", "$root", 1_800_000_030_000);
    const items = [threadRoot, earlierReply, normal, latestReply];

    const rows = projectTimelineDisplayRows(items, ROOM_KEY, LATEST_REPLY);
    const rootRow = rows.find((row) => row.row_id === "thread-root:$root");

    expect(materialRowIds(rows)).toEqual(["$normal", "thread-root:$root"]);
    expect(rootRow).toMatchObject({
      kind: "threadRoot",
      content_event_id: "$root",
      activity_event_id: "$latest",
      content_timestamp_ms: 1_800_000_000_000,
      display_timestamp_ms: 1_800_000_030_000
    });
    expect(rootRow?.item).toBe(threadRoot);
    expect(rootRow?.item.body).toBe("original root body");
    expect(rootRow?.item.reactions).toBe(threadRoot.reactions);
    expect(rows.some((row) => row.content_event_id === "$earlier")).toBe(false);
    expect(rows.some((row) => row.content_event_id === "$latest")).toBe(false);
  });

  test("LatestReply keeps normal rows in canonical relative order and does not duplicate roots", () => {
    const rootA = root("$root-a", 100, "$reply-a", 400);
    const rootB = root("$root-b", 200, "$reply-b", 600);
    const normalA = event("$normal-a", 300);
    const replyA = reply("$reply-a", "$root-a", 400);
    const normalB = event("$normal-b", 500);
    const replyB = reply("$reply-b", "$root-b", 600);

    const rows = projectTimelineDisplayRows(
      [rootA, rootB, normalA, replyA, normalB, replyB],
      ROOM_KEY,
      LATEST_REPLY
    );

    expect(materialRowIds(rows)).toEqual([
      "$normal-a",
      "thread-root:$root-a",
      "$normal-b",
      "thread-root:$root-b"
    ]);
    expect(rows.filter((row) => row.row_id === "thread-root:$root-a")).toHaveLength(1);
    expect(rows.filter((row) => row.row_id === "thread-root:$root-b")).toHaveLength(1);
  });

  test("Thread and Focused keys remain canonical, including individual reply rows", () => {
    const threadRoot = root("$root", 100, "$reply", 200);
    const threadReply = reply("$reply", "$root", 200);
    const items = [threadRoot, threadReply];

    for (const key of [
      threadTimelineKey(ACCOUNT_KEY, ROOM_ID, "$root"),
      focusedTimelineKey(ACCOUNT_KEY, ROOM_ID, "$reply")
    ]) {
      const rows = projectTimelineDisplayRows(items, key, LATEST_REPLY);
      expect(rows.map((row) => row.item)).toStrictEqual(items);
      expect(rows.map((row) => row.kind)).toEqual(["threadRoot", "event"]);
      expect(rows[1].activity_event_id).toBe("$reply");
    }
  });

  test("moves backward after a summary update and keeps no reply row at either position", () => {
    const rootAtLatest = root("$root", 100, "$newer", 300);
    const oldReply = reply("$older", "$root", 200);
    const normal = event("$normal", 250);
    const newReply = reply("$newer", "$root", 300);
    const canonical = [rootAtLatest, oldReply, normal, newReply];

    const beforeRedaction = projectTimelineDisplayRows(canonical, ROOM_KEY, LATEST_REPLY);
    const afterRedaction = projectTimelineDisplayRows(
      [
        root("$root", 100, "$older", 200),
        oldReply,
        normal,
        newReply
      ],
      ROOM_KEY,
      LATEST_REPLY
    );

    expect(materialRowIds(beforeRedaction)).toEqual(["$normal", "thread-root:$root"]);
    expect(materialRowIds(afterRedaction)).toEqual(["thread-root:$root", "$normal"]);
    expect(afterRedaction.filter((row) => row.kind === "threadRoot")).toHaveLength(1);
    expect(afterRedaction.some((row) => row.content_event_id === "$newer")).toBe(false);
    expect(afterRedaction.some((row) => row.content_event_id === "$older")).toBe(false);
  });

  test("keeps a root at its canonical slot until a complete exact latest activity is available", () => {
    const threadRoot = root("$root", 100, "$reply", null);
    const canonical = [
      threadRoot,
      reply("$reply", "$root", 200, { timestamp_ms: null }),
      event("$normal", 300)
    ];

    const rows = projectTimelineDisplayRows(canonical, ROOM_KEY, LATEST_REPLY);

    expect(materialRowIds(rows)).toEqual(["thread-root:$root", "$normal"]);
    expect(rows.find((row) => row.row_id === "thread-root:$root")).toMatchObject({
      content_event_id: "$root",
      activity_event_id: "$root",
      display_timestamp_ms: 100
    });
  });

  test("uses the exact redacted reply identity and never infers an activity slot from text or time", () => {
    const threadRoot = root("$root", 100, "$redacted", 200);
    const sameTimestampButWrongThread = reply("$wrong", "$other-root", 200);
    const redactedLatest = reply("$redacted", "$root", 200, { is_redacted: true, body: null });

    const rows = projectTimelineDisplayRows(
      [threadRoot, sameTimestampButWrongThread, redactedLatest],
      ROOM_KEY,
      LATEST_REPLY
    );

    expect(materialRowIds(rows)).toEqual(["thread-root:$root"]);
    expect(rows.find((row) => row.row_id === "thread-root:$root")?.activity_event_id).toBe(
      "$redacted"
    );
  });

  test("is deterministic for equal timestamps and malformed duplicate activity identities", () => {
    const rootA = root("$root-a", 100, "$shared", 500);
    const rootB = root("$root-b", 200, "$shared", 500);
    const sharedForA = reply("$shared", "$root-a", 500);
    const sharedForB = reply("$shared", "$root-b", 500);
    const canonical = [rootB, rootA, sharedForB, sharedForA];

    const first = projectTimelineDisplayRows(canonical, ROOM_KEY, LATEST_REPLY);
    const second = projectTimelineDisplayRows(canonical, ROOM_KEY, LATEST_REPLY);

    expect(materialRowIds(first)).toEqual(["thread-root:$root-b", "thread-root:$root-a"]);
    expect(first.map((row) => row.row_id)).toEqual(second.map((row) => row.row_id));
    expect(first.filter((row) => row.row_id === "thread-root:$root-a")).toHaveLength(1);
    expect(first.filter((row) => row.row_id === "thread-root:$root-b")).toHaveLength(1);
  });

  test("does not invent an event identity for transactions", () => {
    const localEcho = transaction("local-echo", 100);

    const rows = projectTimelineDisplayRows([localEcho], ROOM_KEY, LATEST_REPLY);
    const [localEchoRow] = materialRows(rows);

    expect(materialRows(rows)).toHaveLength(1);
    expect(localEchoRow).toMatchObject({
      row_id: "txn:local-echo",
      kind: "event",
      content_event_id: null,
      activity_event_id: null,
      content_timestamp_ms: 100,
      display_timestamp_ms: 100
    });
    expect(localEchoRow.item).toBe(localEcho);
  });

  test("regenerates Room date dividers from display timestamps while retaining the root timestamp", () => {
    const rootTimestamp = Date.UTC(2026, 6, 8, 12);
    const normalTimestamp = Date.UTC(2026, 6, 9, 12);
    const replyTimestamp = Date.UTC(2026, 6, 10, 12);
    const threadRoot = root("$root", rootTimestamp, "$reply", replyTimestamp);
    const normal = event("$normal", normalTimestamp);
    const latestReply = reply("$reply", "$root", replyTimestamp);

    const rows = projectTimelineDisplayRows(
      [
        canonicalDateDivider(rootTimestamp),
        threadRoot,
        canonicalDateDivider(normalTimestamp),
        normal,
        canonicalDateDivider(replyTimestamp),
        latestReply
      ],
      ROOM_KEY,
      LATEST_REPLY
    );
    const dates = rows.filter((row) => row.kind === "dateDivider");
    const rootRow = rows.find((row) => row.row_id === "thread-root:$root");

    expect(dates.map((row) => row.display_timestamp_ms)).toEqual([normalTimestamp, replyTimestamp]);
    expect(rootRow).toMatchObject({
      content_timestamp_ms: rootTimestamp,
      display_timestamp_ms: replyTimestamp
    });
    expect(rows.map((row) => row.row_id)).not.toContain(`syn:date-divider-${rootTimestamp}`);
  });

  test("prefers a loaded latest reply timestamp over conflicting summary metadata", () => {
    const rootTimestamp = Date.UTC(2026, 6, 8, 12);
    const normalTimestamp = Date.UTC(2026, 6, 9, 12);
    const loadedReplyTimestamp = Date.UTC(2026, 6, 10, 12);
    const staleSummaryTimestamp = Date.UTC(2026, 6, 11, 12);
    const threadRoot = root("$root", rootTimestamp, "$reply", staleSummaryTimestamp);
    const normal = event("$normal", normalTimestamp);
    const latestReply = reply("$reply", "$root", loadedReplyTimestamp);

    const rows = projectTimelineDisplayRows(
      [threadRoot, normal, latestReply],
      ROOM_KEY,
      LATEST_REPLY
    );
    const dates = rows.filter((row) => row.kind === "dateDivider");
    const rootRow = rows.find((row) => row.row_id === "thread-root:$root");

    expect(rootRow?.display_timestamp_ms).toBe(loadedReplyTimestamp);
    expect(dates.map((row) => row.display_timestamp_ms)).toEqual([
      normalTimestamp,
      loadedReplyTimestamp
    ]);
  });

  test("does not mutate the canonical array or item objects", () => {
    const threadRoot = root("$root", 100, "$reply", 200);
    const latestReply = reply("$reply", "$root", 200);
    const items = Object.freeze([threadRoot, latestReply]);
    const originalItemIds = items.map((item) => item.id);
    Object.freeze(threadRoot);
    Object.freeze(latestReply);

    expect(() => projectTimelineDisplayRows(items, ROOM_KEY, LATEST_REPLY)).not.toThrow();
    expect(items).toEqual([threadRoot, latestReply]);
    expect(items[0].id).toBe(originalItemIds[0]);
    expect(items[1].id).toBe(originalItemIds[1]);
  });
});
