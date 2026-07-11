/**
 * Headless UI tests — QA Model layer 4 (logic tier).
 *
 * Tests the timeline store (diff application, generation handling, scroll
 * anchoring, pagination suppression, send command shape, login credential
 * safety) without any browser process or Vite dev server. Real-DOM scroll
 * behaviour is covered by the Playwright spec (e2e/timeline-scrollback.spec.ts)
 * driving headless Chromium against the harness page.
 *
 * All event payloads here use the WIRE shapes pinned by the Rust contract
 * test `core_event_wire_format_matches_typescript_contract` in src-tauri
 * lib.rs (externally tagged serde enums).
 *
 * References:
 *   docs/architecture/overview.md — "Timeline Viewport And Scrollback"
 *   docs/superpowers/plans/2026-06-12-headless-core-runtime-implementation.md
 *     — Phase 7 gate: test:ui-headless
 */

import { readFileSync } from "node:fs";

import { describe, expect, test } from "vitest";

import type { TimelineItem, TimelineKey } from "./coreEvents";
import { roomTimelineKey, timelineItemDomId } from "./coreEvents";
import { projectTimelineDisplayRows } from "./timelineDisplayProjection";
import type { TimelineThreadRootOrder } from "./types";
import {
  applyDiffs,
  applyGlobalResync,
  applyTimelineEvent,
  batchContainsPrepend,
  createTimelineStore,
  getThreadRootProjections,
  getMediaUploadProgress,
  getItems,
  getKeyState,
  getPaginationState,
  isAwaitingResync,
  applyTimelineEventWithRetention,
  pruneTimelineStore,
  shouldSuppressAutoBackfill,
  timelineStoreKeyId,
  type TimelineStoreState
} from "./timelineStore";
import { TauriIpcMock } from "../test/tauriIpcMock";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const ACCOUNT_KEY = "@qa-user:example.invalid";
const KEY: TimelineKey = roomTimelineKey(ACCOUNT_KEY, "!room:example.invalid");
const LATEST_REPLY: TimelineThreadRootOrder = { kind: "latestReply" };

function makeMsg(id: string, body: string): TimelineItem {
  return {
    id: { Event: { event_id: id } },
    sender: "@sender:example.invalid",
    body,
    timestamp_ms: 1_800_000_000_000,
    in_reply_to_event_id: null,
    thread_root: null,
    thread_summary: null,
    can_react: true,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false,
    reactions: [
      {
        key: "👍",
        count: 1,
        reacted_by_me: false,
        my_reaction_event_id: null,
        sender_preview: ["@alice:example.invalid"]
      }
    ]
  };
}

function makeMsgAt(id: string, body: string, timestampMs: number): TimelineItem {
  return {
    ...makeMsg(id, body),
    timestamp_ms: timestampMs
  };
}

function makeLocalEcho(txnId: string, body: string): TimelineItem {
  return {
    id: { Transaction: { transaction_id: txnId } },
    sender: "@qa-user:example.invalid",
    body,
    timestamp_ms: 1_820_000_000_000,
    in_reply_to_event_id: null,
    thread_root: null,
    thread_summary: null,
    can_react: false,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false,
    reactions: []
  };
}

function itemId(item: TimelineItem): string {
  return timelineItemDomId(item.id);
}

// ---------------------------------------------------------------------------
// (1) Timeline renders InitialItems then applies append/Set/Remove diffs
// ---------------------------------------------------------------------------

describe("timeline store — diff application", () => {
  test("preserves an unchanged thread-root projection map identity across canonical-only updates", () => {
    let store = createTimelineStore();
    const emptyProjections = store.threadRootProjections;

    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [makeMsg("$latest", "Latest")]
      }
    });
    expect(store.threadRootProjections).toBe(emptyProjections);

    store = applyTimelineEvent(store, {
      PaginationStateChanged: {
        request_id: null,
        key: KEY,
        direction: "Backward",
        state: "Idle"
      }
    });
    expect(store.threadRootProjections).toBe(emptyProjections);
  });

  test("stores thread-root projection snapshots outside canonical items", () => {
    let store = createTimelineStore();
    const canonicalReply = {
      ...makeMsg("$latest-reply", "reply"),
      thread_root: "$old-root"
    };
    store = applyTimelineEvent(store, {
      InitialItems: { request_id: null, key: KEY, generation: 1, items: [canonicalReply] }
    });
    const canonicalBefore = getItems(store, KEY);

    store = applyTimelineEvent(store, {
      ThreadRootProjection: {
        key: KEY,
        projection: {
          root_event_id: "$old-root",
          activity_event_id: "$latest-reply",
          activity_timestamp_ms: 1_800_000_010_000,
          state: { kind: "pending" }
        }
      }
    });

    expect(getItems(store, KEY)).toBe(canonicalBefore);
    expect(getItems(store, KEY)).toEqual([canonicalReply]);
    expect(getThreadRootProjections(store, KEY)).toEqual([
      {
        root_event_id: "$old-root",
        activity_event_id: "$latest-reply",
        activity_timestamp_ms: 1_800_000_010_000,
        state: { kind: "pending" }
      }
    ]);

    const root = makeMsg("$old-root", "old root body");
    store = applyTimelineEvent(store, {
      ThreadRootProjection: {
        key: KEY,
        projection: {
          root_event_id: "$old-root",
          activity_event_id: "$latest-reply",
          activity_timestamp_ms: 1_800_000_010_000,
          state: { kind: "ready", item: root }
        }
      }
    });

    expect(getItems(store, KEY)).toEqual([canonicalReply]);
    expect(getThreadRootProjections(store, KEY)[0]?.state).toEqual({ kind: "ready", item: root });

    store = applyTimelineEvent(store, {
      ThreadRootProjection: {
        key: KEY,
        projection: {
          root_event_id: "$old-root",
          activity_event_id: "$latest-reply",
          activity_timestamp_ms: 1_800_000_010_000,
          state: { kind: "cleared" }
        }
      }
    });
    expect(getThreadRootProjections(store, KEY)).toEqual([]);
  });

  test("retains an active terminal root projection then evicts it after its reply leaves the canonical window", () => {
    const reply = { ...makeMsg("$latest-reply", "reply"), thread_root: "$old-root" };
    let store = applyTimelineEvent(createTimelineStore(), {
      InitialItems: { request_id: null, key: KEY, generation: 1, items: [reply] }
    });
    const canonicalBefore = getItems(store, KEY);

    store = applyTimelineEvent(store, {
      ThreadRootProjection: {
        key: KEY,
        projection: {
          root_event_id: "$old-root",
          activity_event_id: "$latest-reply",
          activity_timestamp_ms: 1,
          state: { kind: "failed", failure_kind: "notFound" }
        }
      }
    });
    expect(getItems(store, KEY)).toBe(canonicalBefore);
    expect(getThreadRootProjections(store, KEY)).toHaveLength(1);

    store = applyTimelineEvent(store, {
      InitialItems: { request_id: null, key: KEY, generation: 2, items: [] }
    });
    expect(getItems(store, KEY)).toEqual([]);
    expect(getThreadRootProjections(store, KEY)).toEqual([]);
  });

  test("keeps the first-reply hydration pending and then failed placeholder after canonical diff arrives first", () => {
    const reply = { ...makeMsg("$first-reply", "reply"), thread_root: "$old-root" };
    let store = applyTimelineEvent(createTimelineStore(), {
      InitialItems: { request_id: null, key: KEY, generation: 1, items: [] }
    });
    store = applyTimelineEvent(store, {
      ItemsUpdated: {
        key: KEY,
        generation: 1,
        batch_id: 1,
        diffs: [{ PushBack: { item: reply } }]
      }
    });
    store = applyTimelineEvent(store, {
      ThreadRootProjection: {
        key: KEY,
        projection: {
          root_event_id: "$old-root",
          activity_event_id: "$first-reply",
          activity_timestamp_ms: reply.timestamp_ms,
          state: { kind: "pending" }
        }
      }
    });
    expect(getThreadRootProjections(store, KEY)).toEqual([
      expect.objectContaining({ state: { kind: "pending" } })
    ]);

    store = applyTimelineEvent(store, {
      ThreadRootProjection: {
        key: KEY,
        projection: {
          root_event_id: "$old-root",
          activity_event_id: "$first-reply",
          activity_timestamp_ms: reply.timestamp_ms,
          state: { kind: "failed", failure_kind: "network" }
        }
      }
    });
    expect(getThreadRootProjections(store, KEY)).toEqual([
      expect.objectContaining({ state: { kind: "failed", failure_kind: "network" } })
    ]);

    store = applyTimelineEvent(store, {
      ItemsUpdated: {
        key: KEY,
        generation: 1,
        batch_id: 2,
        diffs: [{ PushBack: { item: makeMsg("$unrelated", "unrelated") } }]
      }
    });
    expect(getThreadRootProjections(store, KEY)).toHaveLength(1);
  });

  test("retains a replay-known ready root through GlobalResync without a canonical reply row", () => {
    const normal = makeMsg("$normal", "normal");
    const knownRoot = {
      ...makeMsg("$known-root", "known root"),
      thread_summary: {
        reply_count: 1,
        latest_event_id: "$summary-activity",
        latest_sender: "@reply:example.invalid",
        latest_body_preview: "summary activity",
        latest_timestamp_ms: 1_800_000_010_000
      }
    };
    let store = applyTimelineEvent(createTimelineStore(), {
      InitialItems: { request_id: null, key: KEY, generation: 1, items: [normal] }
    });

    store = applyTimelineEvent(store, {
      ThreadRootProjection: {
        key: KEY,
        projection: {
          root_event_id: "$known-root",
          activity_event_id: "$summary-activity",
          activity_timestamp_ms: 1_800_000_010_000,
          retain_without_reply: true,
          source: { kind: "replayKnown", epoch: 7 },
          state: { kind: "ready", item: knownRoot }
        }
      }
    });
    expect(getThreadRootProjections(store, KEY)).toHaveLength(1);

    store = applyGlobalResync(store);
    store = applyTimelineEvent(store, {
      InitialItems: { request_id: null, key: KEY, generation: 1, items: [normal] }
    });
    expect(getItems(store, KEY)).toEqual([normal]);
    expect(getThreadRootProjections(store, KEY)).toEqual([
      expect.objectContaining({
        root_event_id: "$known-root",
        retain_without_reply: true,
        source: { kind: "replayKnown", epoch: 7 },
        state: { kind: "ready", item: knownRoot }
      })
    ]);

    store = applyTimelineEvent(store, {
      ThreadRootProjection: {
        key: KEY,
        projection: {
          root_event_id: "$known-root",
          activity_event_id: "$summary-activity",
          activity_timestamp_ms: 1_800_000_010_000,
          retain_without_reply: false,
          source: { kind: "replayKnown", epoch: 7 },
          state: { kind: "cleared" }
        }
      }
    });
    expect(getThreadRootProjections(store, KEY)).toEqual([]);
  });

  test("replaces a replay-known ready root when its renderable revision changes at the same activity", () => {
    const initialRoot = {
      ...makeMsg("$known-root", "original root"),
      thread_summary: {
        reply_count: 2,
        latest_event_id: "$latest",
        latest_sender: "@reply:example.invalid",
        latest_body_preview: "latest reply",
        latest_timestamp_ms: 1_800_000_010_000
      }
    };
    const revisedRoot = {
      ...initialRoot,
      body: "redacted replacement",
      is_redacted: true,
      reactions: [
        {
          key: "👍",
          count: 2,
          reacted_by_me: true,
          my_reaction_event_id: "$reaction",
          sender_preview: ["@sender:example.invalid", "@reply:example.invalid"]
        }
      ]
    };
    let store = applyTimelineEvent(createTimelineStore(), {
      InitialItems: { request_id: null, key: KEY, generation: 1, items: [makeMsg("$normal", "normal")] }
    });

    for (const item of [initialRoot, revisedRoot]) {
      store = applyTimelineEvent(store, {
        ThreadRootProjection: {
          key: KEY,
          projection: {
            root_event_id: "$known-root",
            activity_event_id: "$latest",
            activity_timestamp_ms: 1_800_000_010_000,
            retain_without_reply: true,
            source: { kind: "replayKnown", epoch: 1 },
            state: { kind: "ready", item }
          }
        }
      });
    }

    expect(getThreadRootProjections(store, KEY)).toEqual([
      expect.objectContaining({
        source: { kind: "replayKnown", epoch: 1 },
        state: { kind: "ready", item: revisedRoot }
      })
    ]);
  });

  test("never retains a non-ready projection solely from retain_without_reply", () => {
    const normal = makeMsg("$normal", "normal");
    const readyRoot = makeMsg("$ready-root", "ready root");
    let store = applyTimelineEvent(createTimelineStore(), {
      InitialItems: { request_id: null, key: KEY, generation: 1, items: [normal] }
    });

    for (const projection of [
      {
        root_event_id: "$pending-root",
        activity_event_id: "$pending-activity",
        activity_timestamp_ms: 1,
        retain_without_reply: true,
        state: { kind: "pending" as const }
      },
      {
        root_event_id: "$failed-root",
        activity_event_id: "$failed-activity",
        activity_timestamp_ms: 2,
        retain_without_reply: true,
        state: { kind: "failed" as const, failure_kind: "notFound" as const }
      },
      {
        root_event_id: "$ordinary-ready-root",
        activity_event_id: "$ordinary-ready-activity",
        activity_timestamp_ms: 3,
        state: { kind: "ready" as const, item: readyRoot }
      }
    ]) {
      store = applyTimelineEvent(store, {
        ThreadRootProjection: { key: KEY, projection }
      });
      expect(getThreadRootProjections(store, KEY)).toEqual([]);
    }
  });

  test("normalizes retain_without_reply away from pending and failed wire payloads", () => {
    const reply = { ...makeMsg("$reply", "reply"), thread_root: "$old-root" };
    let store = applyTimelineEvent(createTimelineStore(), {
      InitialItems: { request_id: null, key: KEY, generation: 1, items: [reply] }
    });

    for (const state of [
      { kind: "pending" as const },
      { kind: "failed" as const, failure_kind: "notFound" as const }
    ]) {
      store = applyTimelineEvent(store, {
        ThreadRootProjection: {
          key: KEY,
          projection: {
            root_event_id: "$old-root",
            activity_event_id: "$reply",
            activity_timestamp_ms: 1,
            retain_without_reply: true,
            source: { kind: "hydration" },
            state
          }
        }
      });
      expect(getThreadRootProjections(store, KEY)).toEqual([
        expect.objectContaining({ retain_without_reply: false, state })
      ]);
    }
  });

  test("scopes a stale replay-known clear to its epoch instead of deleting a hydration ready root", () => {
    const normal = makeMsg("$normal", "normal");
    const reply = { ...makeMsg("$live-reply", "live reply"), thread_root: "$shared-root" };
    const replayRoot = makeMsg("$shared-root", "replay root");
    const hydratedRoot = makeMsg("$shared-root", "hydrated root");
    let store = applyTimelineEvent(createTimelineStore(), {
      InitialItems: { request_id: null, key: KEY, generation: 1, items: [normal] }
    });

    store = applyTimelineEvent(store, {
      ThreadRootProjection: {
        key: KEY,
        projection: {
          root_event_id: "$shared-root",
          activity_event_id: "$summary-activity",
          activity_timestamp_ms: 10,
          retain_without_reply: true,
          source: { kind: "replayKnown", epoch: 41 },
          state: { kind: "ready", item: replayRoot }
        }
      }
    });
    store = applyTimelineEvent(store, {
      ItemsUpdated: {
        key: KEY,
        generation: 1,
        batch_id: 1,
        diffs: [{ PushBack: { item: reply } }]
      }
    });
    store = applyTimelineEvent(store, {
      ThreadRootProjection: {
        key: KEY,
        projection: {
          root_event_id: "$shared-root",
          activity_event_id: "$live-reply",
          activity_timestamp_ms: 11,
          retain_without_reply: false,
          source: { kind: "hydration" },
          state: { kind: "ready", item: hydratedRoot }
        }
      }
    });
    store = applyTimelineEvent(store, {
      ThreadRootProjection: {
        key: KEY,
        projection: {
          root_event_id: "$shared-root",
          activity_event_id: "$summary-activity",
          activity_timestamp_ms: 10,
          retain_without_reply: false,
          source: { kind: "replayKnown", epoch: 41 },
          state: { kind: "cleared" }
        }
      }
    });

    expect(getThreadRootProjections(store, KEY)).toEqual([
      expect.objectContaining({
        source: { kind: "hydration" },
        state: { kind: "ready", item: hydratedRoot }
      })
    ]);
  });

  test("rejects retain_without_reply on an arbitrary hydration ready payload", () => {
    let store = applyTimelineEvent(createTimelineStore(), {
      InitialItems: { request_id: null, key: KEY, generation: 1, items: [makeMsg("$normal", "normal")] }
    });
    store = applyTimelineEvent(store, {
      ThreadRootProjection: {
        key: KEY,
        projection: {
          root_event_id: "$untrusted-root",
          activity_event_id: "$untrusted-activity",
          activity_timestamp_ms: 1,
          retain_without_reply: true,
          source: { kind: "hydration" },
          state: { kind: "ready", item: makeMsg("$untrusted-root", "root") }
        }
      }
    });

    expect(getThreadRootProjections(store, KEY)).toEqual([]);
  });

  test("keeps a ready root snapshot through temporary canonical overlap for the later absent-root projection", () => {
    const root = makeMsg("$old-root", "old root body");
    const reply = { ...makeMsg("$latest-reply", "reply"), thread_root: "$old-root" };
    let store = applyTimelineEvent(createTimelineStore(), {
      InitialItems: { request_id: null, key: KEY, generation: 1, items: [root, reply] }
    });

    store = applyTimelineEvent(store, {
      ThreadRootProjection: {
        key: KEY,
        projection: {
          root_event_id: "$old-root",
          activity_event_id: "$latest-reply",
          activity_timestamp_ms: 2,
          state: { kind: "ready", item: root }
        }
      }
    });
    expect(getThreadRootProjections(store, KEY)).toHaveLength(1);

    store = applyTimelineEvent(store, {
      InitialItems: { request_id: null, key: KEY, generation: 2, items: [reply] }
    });
    expect(getItems(store, KEY)).toEqual([reply]);
    expect(getThreadRootProjections(store, KEY)[0]?.state).toEqual({ kind: "ready", item: root });
  });

  test("prunes an inactive pending root before a terminal completion arrives", () => {
    const reply = { ...makeMsg("$latest-reply", "reply"), thread_root: "$old-root" };
    let store = applyTimelineEvent(createTimelineStore(), {
      InitialItems: { request_id: null, key: KEY, generation: 1, items: [reply] }
    });
    store = applyTimelineEvent(store, {
      ThreadRootProjection: {
        key: KEY,
        projection: {
          root_event_id: "$old-root",
          activity_event_id: "$latest-reply",
          activity_timestamp_ms: 1,
          state: { kind: "pending" }
        }
      }
    });
    store = applyTimelineEvent(store, {
      InitialItems: { request_id: null, key: KEY, generation: 2, items: [] }
    });
    expect(getThreadRootProjections(store, KEY)).toEqual([]);

    store = applyTimelineEvent(store, {
      ThreadRootProjection: {
        key: KEY,
        projection: {
          root_event_id: "$old-root",
          activity_event_id: "$latest-reply",
          activity_timestamp_ms: 1,
          state: { kind: "failed", failure_kind: "notFound" }
        }
      }
    });
    expect(getItems(store, KEY)).toEqual([]);
    expect(getThreadRootProjections(store, KEY)).toEqual([]);
  });

  test("InitialItems populates the render list for a key", () => {
    const store = createTimelineStore();
    const items = [makeMsg("$a", "hello"), makeMsg("$b", "world")];
    const next = applyTimelineEvent(store, {
      InitialItems: { request_id: null, key: KEY, generation: 1, items }
    });

    expect(getItems(next, KEY)).toHaveLength(2);
    expect(itemId(getItems(next, KEY)[0])).toBe("$a");
    expect(itemId(getItems(next, KEY)[1])).toBe("$b");
    expect(isAwaitingResync(next, KEY)).toBe(false);
  });

  test("PushBack diff appends an item", () => {
    let store = createTimelineStore();
    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [makeMsg("$a", "hello")]
      }
    });
    store = applyTimelineEvent(store, {
      ItemsUpdated: {
        key: KEY,
        generation: 1,
        batch_id: 1,
        diffs: [{ PushBack: { item: makeMsg("$b", "world") } }]
      }
    });

    const items = getItems(store, KEY);
    expect(items).toHaveLength(2);
    expect(itemId(items[1])).toBe("$b");
  });

  test("PushBack preserves SDK VectorDiff order even when timestamps are older", () => {
    let store = createTimelineStore();
    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [
          makeMsgAt("$jun17", "Jun 17", 1_797_460_000_000),
          makeMsgAt("$jun20", "Jun 20", 1_797_720_000_000)
        ]
      }
    });

    store = applyTimelineEvent(store, {
      ItemsUpdated: {
        key: KEY,
        generation: 1,
        batch_id: 2,
        diffs: [{ PushBack: { item: makeMsgAt("$jun13", "Jun 13", 1_797_120_000_000) } }]
      }
    });

    expect(getItems(store, KEY).map((item) => item.body)).toEqual(["Jun 17", "Jun 20", "Jun 13"]);
  });

  test("duplicate ItemsUpdated batch ids are ignored instead of reapplying index diffs", () => {
    let store = createTimelineStore();
    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [makeMsg("$a", "a"), makeMsg("$b", "b"), makeMsg("$c", "c")]
      }
    });

    const repeatedBatch = {
      key: KEY,
      generation: 1,
      batch_id: 2,
      diffs: [{ Remove: { index: 1 } }]
    };
    store = applyTimelineEvent(store, { ItemsUpdated: repeatedBatch });
    store = applyTimelineEvent(store, { ItemsUpdated: repeatedBatch });

    expect(getItems(store, KEY).map((item) => itemId(item))).toEqual(["$a", "$c"]);
  });

  test("deduplicates repeated event ids from overlapping scrollback batches", () => {
    let store = createTimelineStore();
    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [
          makeMsgAt("$a", "first copy", 1_797_460_000_000),
          makeMsgAt("$b", "neighbor", 1_797_720_000_000)
        ]
      }
    });

    store = applyTimelineEvent(store, {
      ItemsUpdated: {
        key: KEY,
        generation: 1,
        batch_id: 2,
        diffs: [{ PushBack: { item: makeMsgAt("$a", "second copy", 1_797_460_000_000) } }]
      }
    });

    const items = getItems(store, KEY);
    expect(items.map((item) => itemId(item))).toEqual(["$a", "$b"]);
    expect(items[0].body).toBe("first copy");
  });

  test("maintains item id and timestamp indexes alongside render items", () => {
    const store = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [
          makeMsgAt("$a", "first", 1_797_460_000_000),
          makeMsgAt("$b", "second", 1_797_460_000_000)
        ]
      }
    });

    const state = getKeyState(store, KEY);
    expect(state?.itemIndexById.get("$a")).toBe(0);
    expect(state?.itemIndexById.get("$b")).toBe(1);
    expect([...(state?.itemIdsByTimestamp.get(1_797_460_000_000) ?? [])]).toEqual(["$a", "$b"]);
  });

  test("render reducer avoids full-list sort and linear id scans", () => {
    const source = readFileSync(new URL("./timelineStore.ts", import.meta.url), "utf8");
    const renderReducerStart = source.indexOf("function applyDiffsForRender");
    const renderReducerEnd = source.indexOf("/** True if any diff", renderReducerStart);
    const renderReducer = source.slice(renderReducerStart, renderReducerEnd);

    expect(renderReducer).toContain("itemIdsByTimestamp");
    expect(renderReducer).toContain("itemIndexById");
    expect(renderReducer).not.toContain(".sort(");
    expect(renderReducer).not.toContain("findIndex(");
    expect(renderReducer).not.toContain("insertionIndexForTimelineItem");
  });

  test("Set diff updates an existing item in-place", () => {
    let store = createTimelineStore();
    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [makeMsg("$a", "original"), makeMsg("$b", "keep")]
      }
    });
    store = applyTimelineEvent(store, {
      ItemsUpdated: {
        key: KEY,
        generation: 1,
        batch_id: 2,
        diffs: [{ Set: { index: 0, item: makeMsg("$a", "edited body") } }]
      }
    });

    const items = getItems(store, KEY);
    expect(itemId(items[0])).toBe("$a");
    expect(items[0].body).toBe("edited body");
    expect(itemId(items[1])).toBe("$b");
  });

  test("a replay-external local mutation cannot replace the first bounded display row", () => {
    let store = createTimelineStore();
    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [makeMsg("$before", "before"), makeMsg("$after", "after")]
      }
    });

    // A root at canonical navigation index 0 is outside this bounded replay
    // window. Rust projects its local Set to no display diff; the separately
    // emitted replay Ready revision owns rendering that root.
    store = applyTimelineEvent(store, {
      ItemsUpdated: {
        key: KEY,
        generation: 1,
        batch_id: 2,
        diffs: []
      }
    });

    expect(getItems(store, KEY).map((item) => [itemId(item), item.body])).toEqual([
      ["$before", "before"],
      ["$after", "after"]
    ]);
  });

  test("Set diff for a collapsed duplicate scrollback item updates the canonical row without moving the latest item", () => {
    let store = createTimelineStore();
    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [
          makeMsgAt("$old", "older copy", 1_797_460_000_000),
          makeMsgAt("$second", "second from bottom", 1_797_720_000_000),
          makeMsgAt("$latest", "latest message", 1_798_000_000_000)
        ]
      }
    });

    store = applyTimelineEvent(store, {
      ItemsUpdated: {
        key: KEY,
        generation: 1,
        batch_id: 2,
        diffs: [
          {
            Insert: {
              index: 2,
              item: makeMsgAt("$old", "overlapping duplicate", 1_797_460_000_000)
            }
          }
        ]
      }
    });
    store = applyTimelineEvent(store, {
      ItemsUpdated: {
        key: KEY,
        generation: 1,
        batch_id: 3,
        diffs: [
          {
            Set: {
              index: 2,
              item: makeMsgAt("$old", "older copy updated", 1_797_460_000_000)
            }
          }
        ]
      }
    });

    const items = getItems(store, KEY);
    expect(items.map((item) => itemId(item))).toEqual(["$old", "$second", "$latest"]);
    expect(items[0].body).toBe("older copy updated");
    expect(items[2].body).toBe("latest message");
  });

  test("reaction groups survive InitialItems and Set diff application", () => {
    let store = createTimelineStore();
    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [makeMsg("$a", "hello")]
      }
    });
    store = applyTimelineEvent(store, {
      ItemsUpdated: {
        key: KEY,
        generation: 1,
        batch_id: 2,
        diffs: [
          {
            Set: {
              index: 0,
              item: {
                ...makeMsg("$a", "edited body"),
                reactions: [
                  {
                    key: "🔥",
                    count: 3,
                    reacted_by_me: true,
                    my_reaction_event_id: "$reaction:test",
                    sender_preview: ["@alice:example.invalid", "@bob:example.invalid"]
                  }
                ],
                can_react: true,
                is_redacted: false,
                is_hidden: false,
                can_redact: false,
                is_edited: true,
                can_edit: false
              }
            }
          }
        ]
      }
    });

    const items = getItems(store, KEY);
    expect(items[0].reactions).toHaveLength(1);
    expect(items[0].reactions[0]).toEqual({
      key: "🔥",
      count: 3,
      reacted_by_me: true,
      my_reaction_event_id: "$reaction:test",
      sender_preview: ["@alice:example.invalid", "@bob:example.invalid"]
    });
  });

  test("Remove diff removes an item by index", () => {
    let store = createTimelineStore();
    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [makeMsg("$a", "a"), makeMsg("$b", "b"), makeMsg("$c", "c")]
      }
    });
    store = applyTimelineEvent(store, {
      ItemsUpdated: {
        key: KEY,
        generation: 1,
        batch_id: 3,
        diffs: [{ Remove: { index: 1 } }]
      }
    });

    const items = getItems(store, KEY);
    expect(items).toHaveLength(2);
    expect(itemId(items[0])).toBe("$a");
    expect(itemId(items[1])).toBe("$c");
  });

  test("PushFront diff prepends an item", () => {
    let store = createTimelineStore();
    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [makeMsg("$b", "b")]
      }
    });
    store = applyTimelineEvent(store, {
      ItemsUpdated: {
        key: KEY,
        generation: 1,
        batch_id: 4,
        diffs: [{ PushFront: { item: makeMsg("$a", "a") } }]
      }
    });

    const items = getItems(store, KEY);
    expect(items).toHaveLength(2);
    expect(itemId(items[0])).toBe("$a");
    expect(itemId(items[1])).toBe("$b");
  });

  test("Reset diff replaces the entire list", () => {
    let store = createTimelineStore();
    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [makeMsg("$old", "old")]
      }
    });
    store = applyTimelineEvent(store, {
      ItemsUpdated: {
        key: KEY,
        generation: 1,
        batch_id: 5,
        diffs: [{ Reset: { items: [makeMsg("$new1", "n1"), makeMsg("$new2", "n2")] } }]
      }
    });

    const items = getItems(store, KEY);
    expect(items).toHaveLength(2);
    expect(itemId(items[0])).toBe("$new1");
  });

  test("Clear diff (wire shape: bare string) empties the list", () => {
    let store = createTimelineStore();
    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [makeMsg("$a", "a"), makeMsg("$b", "b")]
      }
    });
    store = applyTimelineEvent(store, {
      ItemsUpdated: { key: KEY, generation: 1, batch_id: 6, diffs: ["Clear"] }
    });

    expect(getItems(store, KEY)).toHaveLength(0);
  });

  test("multiple diffs in one batch are applied in order", () => {
    let store = createTimelineStore();
    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [makeMsg("$a", "a")]
      }
    });
    store = applyTimelineEvent(store, {
      ItemsUpdated: {
        key: KEY,
        generation: 1,
        batch_id: 7,
        diffs: [
          { PushBack: { item: makeMsg("$b", "b") } },
          { PushBack: { item: makeMsg("$c", "c") } },
          { Remove: { index: 0 } } // remove $a
        ]
      }
    });

    const items = getItems(store, KEY);
    expect(items).toHaveLength(2);
    expect(itemId(items[0])).toBe("$b");
    expect(itemId(items[1])).toBe("$c");
  });
});

// ---------------------------------------------------------------------------
// (2) Stale-generation diffs discarded; ResyncRequired clears + re-renders
// ---------------------------------------------------------------------------

describe("timeline store — generation handling", () => {
  test("diff with stale generation is silently discarded", () => {
    let store = createTimelineStore();
    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 2,
        items: [makeMsg("$a", "a")]
      }
    });
    // Diff from old generation (1 < 2) — must be discarded.
    store = applyTimelineEvent(store, {
      ItemsUpdated: {
        key: KEY,
        generation: 1,
        batch_id: 1,
        diffs: [{ PushBack: { item: makeMsg("$stale", "should not appear") } }]
      }
    });

    const items = getItems(store, KEY);
    expect(items).toHaveLength(1);
    expect(itemId(items[0])).toBe("$a");
  });

  test("ResyncRequired clears the list and marks awaiting resync", () => {
    let store = createTimelineStore();
    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [makeMsg("$a", "a"), makeMsg("$b", "b")]
      }
    });
    store = applyTimelineEvent(store, {
      ResyncRequired: { key: KEY, reason: "QueueOverflow" }
    });

    expect(getItems(store, KEY)).toHaveLength(0);
    expect(isAwaitingResync(store, KEY)).toBe(true);
  });

  test("after ResyncRequired, diffs are discarded until next InitialItems", () => {
    let store = createTimelineStore();
    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [makeMsg("$a", "a")]
      }
    });
    store = applyTimelineEvent(store, {
      ResyncRequired: { key: KEY, reason: "QueueOverflow" }
    });
    // Discarded because awaitingResync=true.
    store = applyTimelineEvent(store, {
      ItemsUpdated: {
        key: KEY,
        generation: 1,
        batch_id: 10,
        diffs: [{ PushBack: { item: makeMsg("$ghost", "ghost") } }]
      }
    });

    expect(getItems(store, KEY)).toHaveLength(0);
  });

  test("new InitialItems after ResyncRequired resumes normal diff application", () => {
    let store = createTimelineStore();
    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [makeMsg("$old", "old")]
      }
    });
    store = applyTimelineEvent(store, {
      ResyncRequired: { key: KEY, reason: "QueueOverflow" }
    });
    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 2,
        items: [makeMsg("$fresh", "fresh")]
      }
    });
    store = applyTimelineEvent(store, {
      ItemsUpdated: {
        key: KEY,
        generation: 2,
        batch_id: 11,
        diffs: [{ PushBack: { item: makeMsg("$extra", "extra") } }]
      }
    });

    const items = getItems(store, KEY);
    expect(items).toHaveLength(2);
    expect(itemId(items[0])).toBe("$fresh");
    expect(itemId(items[1])).toBe("$extra");
  });

  test("global ResyncMarker clears all keys and awaits InitialItems", () => {
    let store = createTimelineStore();
    const keyB = roomTimelineKey(ACCOUNT_KEY, "!room-b:example.invalid");
    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [makeMsg("$a", "a")]
      }
    });
    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: keyB,
        generation: 1,
        items: [makeMsg("$b", "b")]
      }
    });
    store = applyGlobalResync(store);

    expect(getItems(store, KEY)).toHaveLength(0);
    expect(getItems(store, keyB)).toHaveLength(0);
    expect(isAwaitingResync(store, KEY)).toBe(true);
    expect(isAwaitingResync(store, keyB)).toBe(true);
  });

  test("global ResyncMarker keeps an active terminal root projection through InitialItems replay", () => {
    const reply = { ...makeMsg("$reply", "reply"), thread_root: "$old-root" };
    const root = makeMsg("$old-root", "old root");
    for (const [state, expectedKind] of [
      [{ kind: "ready" as const, item: root }, "threadRoot"],
      [{ kind: "failed" as const, failure_kind: "notFound" as const }, "threadRootFailed"]
    ] as const) {
      let store = applyTimelineEvent(createTimelineStore(), {
        InitialItems: { request_id: null, key: KEY, generation: 1, items: [reply] }
      });
      store = applyTimelineEvent(store, {
        ThreadRootProjection: {
          key: KEY,
          projection: {
            root_event_id: "$old-root",
            activity_event_id: "$reply",
            activity_timestamp_ms: reply.timestamp_ms,
            state
          }
        }
      });

      store = applyGlobalResync(store);
      expect(getThreadRootProjections(store, KEY)).toHaveLength(1);

      store = applyTimelineEvent(store, {
        InitialItems: { request_id: null, key: KEY, generation: 1, items: [reply] }
      });
      const rows = projectTimelineDisplayRows(
        getItems(store, KEY),
        KEY,
        LATEST_REPLY,
        getThreadRootProjections(store, KEY)
      );
      expect(rows.find((row) => row.row_id === "thread-root:$old-root")?.kind).toBe(expectedKind);
      expect(rows.some((row) => row.row_id === "$reply")).toBe(false);
    }
  });
});

// ---------------------------------------------------------------------------
// (3) Rust-owned media upload progress is retained per timeline transaction
// ---------------------------------------------------------------------------

describe("timeline store — media upload progress", () => {
  test("MediaUploadProgress is stored by transaction id for the matching key", () => {
    let store = createTimelineStore();
    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [makeLocalEcho("txn-media", "")]
      }
    });
    store = applyTimelineEvent(store, {
      MediaUploadProgress: {
        request_id: null,
        key: KEY,
        transaction_id: "txn-media",
        index: 0,
        progress: { current: 15, total: 30 },
        source: null
      }
    });

    expect(getMediaUploadProgress(store, KEY, "txn-media")).toEqual({
      current: 15,
      total: 30
    });
    expect(
      getMediaUploadProgress(
        store,
        roomTimelineKey(ACCOUNT_KEY, "!other:example.invalid"),
        "txn-media"
      )
    ).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// (4) Prepend batch keeps the anchored item visually stable (scroll anchor)
//     (Logic tier; real-DOM pixel assertion lives in the Playwright spec.)
// ---------------------------------------------------------------------------

describe("scroll anchoring — prepend keeps anchor stable", () => {
  test("batchContainsPrepend detects PushFront and Insert-at-0 batches", () => {
    expect(batchContainsPrepend([{ PushFront: { item: makeMsg("$x", "x") } }])).toBe(true);
    expect(batchContainsPrepend([{ Insert: { index: 0, item: makeMsg("$x", "x") } }])).toBe(
      true
    );
    expect(batchContainsPrepend([{ PushBack: { item: makeMsg("$x", "x") } }])).toBe(false);
    expect(batchContainsPrepend([{ Insert: { index: 3, item: makeMsg("$x", "x") } }])).toBe(
      false
    );
    expect(batchContainsPrepend(["Clear"])).toBe(false);
  });

  test("prepend places new items before the anchor; anchor id stays findable", () => {
    const items = [makeMsg("$first", "first"), makeMsg("$second", "second")];
    const prepended = applyDiffs(items, [{ PushFront: { item: makeMsg("$older", "older") } }]);
    expect(itemId(prepended[1])).toBe("$first");
    expect(itemId(prepended[0])).toBe("$older");
  });

});

// ---------------------------------------------------------------------------
// (4) EndReached stops further auto-pagination requests
// ---------------------------------------------------------------------------

describe("pagination suppression — EndReached stops auto-backfill", () => {
  function withPaginationState(
    state: "Idle" | "Paginating" | "EndReached" | { Failed: { kind: "Network" } }
  ) {
    let store = createTimelineStore();
    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [makeMsg("$a", "a")]
      }
    });
    return applyTimelineEvent(store, {
      PaginationStateChanged: {
        request_id: null,
        key: KEY,
        direction: "Backward",
        state
      }
    });
  }

  test("Paginating state suppresses auto-backfill", () => {
    const store = withPaginationState("Paginating");
    expect(shouldSuppressAutoBackfill(store, KEY)).toBe(true);
    expect(getPaginationState(store, KEY, "Backward")).toBe("Paginating");
  });

  test("EndReached state suppresses auto-backfill", () => {
    const store = withPaginationState("EndReached");
    expect(shouldSuppressAutoBackfill(store, KEY)).toBe(true);
    expect(getPaginationState(store, KEY, "Backward")).toBe("EndReached");
  });

  test("Idle state allows auto-backfill", () => {
    const store = withPaginationState("Idle");
    expect(shouldSuppressAutoBackfill(store, KEY)).toBe(false);
  });

  test("Failed state does not suppress auto-backfill (user may retry)", () => {
    const store = withPaginationState({ Failed: { kind: "Network" } });
    expect(shouldSuppressAutoBackfill(store, KEY)).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// (5) Send path invokes the right command shape + renders local echo
// ---------------------------------------------------------------------------

describe("send path — command shape and local echo from diff", () => {
  test("mock IPC records send_text invocation with correct command shape", async () => {
    const ipc = new TauriIpcMock();
    await ipc.invoke("send_text", {
      roomId: "!room:example.invalid",
      body: "hello world"
    });

    const calls = ipc.invocationsOf("send_text");
    expect(calls).toHaveLength(1);
    expect(calls[0].args["roomId"]).toBe("!room:example.invalid");
    expect(calls[0].args["body"]).toBe("hello world");
  });

  test("mock IPC records restart_sync invocation", async () => {
    const ipc = new TauriIpcMock();
    await ipc.invoke("restart_sync");

    const calls = ipc.invocationsOf("restart_sync");
    expect(calls).toHaveLength(1);
    expect(calls[0].command).toBe("restart_sync");
  });

  test("local echo from PushBack diff appears in store after send", () => {
    let store = createTimelineStore();
    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [makeMsg("$existing", "existing")]
      }
    });

    // What core emits as local echo after SendText (Transaction identity):
    store = applyTimelineEvent(store, {
      ItemsUpdated: {
        key: KEY,
        generation: 1,
        batch_id: 20,
        diffs: [{ PushBack: { item: makeLocalEcho("desktop-1", "hello world") } }]
      }
    });

    const items = getItems(store, KEY);
    expect(items).toHaveLength(2);
    expect(items[1].body).toBe("hello world");
    expect(itemId(items[1])).toBe("txn:desktop-1");
    expect("Transaction" in items[1].id).toBe(true);
  });

  test("late-mounted store applies live PushBack diff when InitialItems was missed", () => {
    let store = createTimelineStore();

    store = applyTimelineEvent(store, {
      ItemsUpdated: {
        key: KEY,
        generation: 0,
        batch_id: 20,
        diffs: [{ PushBack: { item: makeLocalEcho("desktop-1", "hello world") } }]
      }
    });

    const items = getItems(store, KEY);
    expect(items).toHaveLength(1);
    expect(items[0].body).toBe("hello world");
    expect(itemId(items[0])).toBe("txn:desktop-1");
  });

  test("remote echo replaces local echo via Set diff (identity transition)", () => {
    let store = createTimelineStore();
    store = applyTimelineEvent(store, {
      InitialItems: { request_id: null, key: KEY, generation: 1, items: [] }
    });
    store = applyTimelineEvent(store, {
      ItemsUpdated: {
        key: KEY,
        generation: 1,
        batch_id: 21,
        diffs: [{ PushBack: { item: makeLocalEcho("desktop-1", "hello") } }]
      }
    });
    // Remote echo replaces the Transaction identity with an Event identity
    // through an explicit Set diff (Viewport/Scrollback contract).
    store = applyTimelineEvent(store, {
      ItemsUpdated: {
        key: KEY,
        generation: 1,
        batch_id: 22,
        diffs: [{ Set: { index: 0, item: makeMsg("$remote-event-id", "hello") } }]
      }
    });

    const items = getItems(store, KEY);
    expect(items).toHaveLength(1);
    expect(itemId(items[0])).toBe("$remote-event-id");
    expect("Event" in items[0].id).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// (6) Login form submits credentials via command and never logs them
// ---------------------------------------------------------------------------

describe("login credential safety", () => {
  test("mock IPC redacts password from recorded invocation args", async () => {
    const ipc = new TauriIpcMock();
    await ipc.invoke("submit_login", {
      homeserver: "https://matrix.example.org",
      username: "fixture-user",
      password: "synthetic-password-123",
      deviceDisplayName: "Koushi"
    });

    const calls = ipc.invocationsOf("submit_login");
    expect(calls).toHaveLength(1);
    expect(calls[0].args["homeserver"]).toBe("https://matrix.example.org");
    expect(calls[0].args["username"]).toBe("fixture-user");
    expect(calls[0].args["password"]).toBe("[REDACTED]");
    expect(JSON.stringify(calls[0].args)).not.toContain("synthetic-password-123");
  });

  test("mock IPC redacts recovery secret from recorded invocation args", async () => {
    const ipc = new TauriIpcMock();
    await ipc.invoke("submit_recovery", { secret: "synthetic-recovery-secret" });

    const calls = ipc.invocationsOf("submit_recovery");
    expect(calls).toHaveLength(1);
    expect(calls[0].args["secret"]).toBe("[REDACTED]");
    expect(JSON.stringify(calls[0].args)).not.toContain("synthetic-recovery-secret");
  });

  test("mock IPC debug representation never contains raw credential strings", async () => {
    const ipc = new TauriIpcMock();
    await ipc.invoke("submit_login", {
      homeserver: "https://matrix.example.org",
      username: "fixture-user",
      password: "do-not-log-this-password",
      deviceDisplayName: "Test Device"
    });

    const allRecorded = JSON.stringify(ipc.recordedInvocations());
    expect(allRecorded).not.toContain("do-not-log-this-password");
  });

  test("mock IPC event emission does not expose secret-bearing payloads", () => {
    const ipc = new TauriIpcMock();
    const received: unknown[] = [];
    ipc.listen("koushi-desktop://event", (evt) => received.push(evt));

    ipc.emitCoreEvent({
      kind: "Account",
      event: {
        LoggedIn: {
          request_id: { connection_id: 1, sequence: 1 },
          account_key: "@user:example.org"
        }
      }
    });

    expect(received).toHaveLength(1);
    expect(JSON.stringify(received)).toContain("@user:example.org");
    expect(JSON.stringify(received)).not.toContain("password");
  });
});

// ---------------------------------------------------------------------------
// Additional: PaginationStateChanged is tracked per direction independently
// ---------------------------------------------------------------------------

describe("pagination state — per direction tracking", () => {
  test("backward and forward states are tracked independently", () => {
    let store = createTimelineStore();
    store = applyTimelineEvent(store, {
      InitialItems: { request_id: null, key: KEY, generation: 1, items: [] }
    });
    store = applyTimelineEvent(store, {
      PaginationStateChanged: {
        request_id: null,
        key: KEY,
        direction: "Backward",
        state: "EndReached"
      }
    });
    store = applyTimelineEvent(store, {
      PaginationStateChanged: {
        request_id: null,
        key: KEY,
        direction: "Forward",
        state: "Paginating"
      }
    });

    expect(getPaginationState(store, KEY, "Backward")).toBe("EndReached");
    expect(getPaginationState(store, KEY, "Forward")).toBe("Paginating");
    expect(shouldSuppressAutoBackfill(store, KEY)).toBe(true);
  });

  test("spinner from PaginationStateChanged: Paginating → Idle transition", () => {
    let store = createTimelineStore();
    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [makeMsg("$a", "a")]
      }
    });
    store = applyTimelineEvent(store, {
      PaginationStateChanged: {
        request_id: null,
        key: KEY,
        direction: "Backward",
        state: "Paginating"
      }
    });
    expect(getPaginationState(store, KEY, "Backward")).toBe("Paginating");

    store = applyTimelineEvent(store, {
      PaginationStateChanged: {
        request_id: null,
        key: KEY,
        direction: "Backward",
        state: "Idle"
      }
    });
    expect(getPaginationState(store, KEY, "Backward")).toBe("Idle");
    expect(shouldSuppressAutoBackfill(store, KEY)).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// DisplayLabelsUpdated relabels existing rows across all timelines
// ---------------------------------------------------------------------------

describe("DisplayLabelsUpdated", () => {
  test("patches sender_label, reply_quote sender_label, and thread_summary latest_sender_label while preserving raw ids", () => {
    let store = createTimelineStore();

    // Seed two timelines with items that have raw senders but no labels
    const keyA = roomTimelineKey(ACCOUNT_KEY, "!room-a:example.invalid");
    const keyB = roomTimelineKey(ACCOUNT_KEY, "!room-b:example.invalid");

    const itemWithReply: TimelineItem = {
      ...makeMsg("$a", "hello"),
      sender: "@alice:example.invalid",
      sender_label: null,
      reply_quote: {
        event_id: "$quoted:example.invalid",
        sender: "@bob:example.invalid",
        sender_label: null,
        body_preview: "quoted text",
        state: "ready",
      },
    };
    const itemWithThread: TimelineItem = {
      ...makeMsg("$b", "thread root"),
      sender: "@carol:example.invalid",
      sender_label: null,
      thread_root: "$b",
      thread_summary: {
        reply_count: 3,
        latest_event_id: "$latest-thread-reply:example.invalid",
        latest_sender: "@dave:example.invalid",
        latest_sender_label: null,
        latest_body_preview: "latest reply text",
        latest_timestamp_ms: 3000,
      },
    };

    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: keyA,
        generation: 0,
        items: [itemWithReply],
      },
    });
    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: keyB,
        generation: 0,
        items: [itemWithThread],
      },
    });

    // Apply DisplayLabelsUpdated across both timelines
    store = applyTimelineEvent(store, {
      DisplayLabelsUpdated: {
        labels: [
          { user_id: "@alice:example.invalid", display_label: "Alice Alias" },
          { user_id: "@bob:example.invalid", display_label: "Bobby" },
          { user_id: "@carol:example.invalid", display_label: "Carol Alias" },
          { user_id: "@dave:example.invalid", display_label: "Davey" },
        ],
      },
    });

    // Timeline A: sender_label patched, raw sender unchanged
    const itemsA = getItems(store, keyA);
    expect(itemsA).toHaveLength(1);
    expect(itemsA[0].sender).toBe("@alice:example.invalid");
    expect(itemsA[0].sender_label).toBe("Alice Alias");
    expect(itemsA[0].reply_quote?.sender).toBe("@bob:example.invalid");
    expect(itemsA[0].reply_quote?.sender_label).toBe("Bobby");

    // Timeline B: sender_label + thread_summary.latest_sender_label patched
    const itemsB = getItems(store, keyB);
    expect(itemsB).toHaveLength(1);
    expect(itemsB[0].sender).toBe("@carol:example.invalid");
    expect(itemsB[0].sender_label).toBe("Carol Alias");
    expect(itemsB[0].thread_summary?.latest_sender).toBe("@dave:example.invalid");
    expect(itemsB[0].thread_summary?.latest_sender_label).toBe("Davey");
  });

  test("clears sender_label when display_label is empty", () => {
    let store = createTimelineStore();

    const item: TimelineItem = {
      ...makeMsg("$c", "msg"),
      sender: "@eve:example.invalid",
      sender_label: "Old Eve Label",
    };

    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 0,
        items: [item],
      },
    });

    // Update with empty display_label -> clears sender_label
    store = applyTimelineEvent(store, {
      DisplayLabelsUpdated: {
        labels: [
          { user_id: "@eve:example.invalid", display_label: "" },
        ],
      },
    });

    const items = getItems(store, KEY);
    expect(items[0].sender).toBe("@eve:example.invalid");
    expect(items[0].sender_label).toBeNull();
  });

  test("patches a replay-known ready root snapshot just like its canonical root", () => {
    const canonical = {
      ...makeMsg("$canonical-root", "canonical root"),
      sender: "@root:example.invalid",
      sender_label: null,
      reply_quote: {
        event_id: "$quoted:example.invalid",
        sender: "@quoted:example.invalid",
        sender_label: null,
        body_preview: "quoted text",
        state: "ready" as const
      },
      thread_summary: {
        reply_count: 1,
        latest_event_id: "$latest:example.invalid",
        latest_sender: "@latest:example.invalid",
        latest_sender_label: null,
        latest_body_preview: "latest reply",
        latest_timestamp_ms: 3_000
      }
    };
    let store = applyTimelineEvent(createTimelineStore(), {
      InitialItems: { request_id: null, key: KEY, generation: 0, items: [canonical] }
    });
    store = applyTimelineEvent(store, {
      ThreadRootProjection: {
        key: KEY,
        projection: {
          root_event_id: "$replay-root",
          activity_event_id: "$latest:example.invalid",
          activity_timestamp_ms: 3_000,
          retain_without_reply: true,
          source: { kind: "replayKnown", epoch: 1 },
          state: { kind: "ready", item: { ...canonical, id: { Event: { event_id: "$replay-root" } } } }
        }
      }
    });

    store = applyTimelineEvent(store, {
      DisplayLabelsUpdated: {
        labels: [
          { user_id: "@root:example.invalid", display_label: "Root" },
          { user_id: "@quoted:example.invalid", display_label: "Quoted" },
          { user_id: "@latest:example.invalid", display_label: "Latest" }
        ]
      }
    });

    const canonicalUpdated = getItems(store, KEY)[0];
    const projection = getThreadRootProjections(store, KEY)[0];
    expect(projection?.state).toMatchObject({
      kind: "ready",
      item: {
        sender_label: canonicalUpdated.sender_label,
        reply_quote: { sender_label: canonicalUpdated.reply_quote?.sender_label },
        thread_summary: {
          latest_sender_label: canonicalUpdated.thread_summary?.latest_sender_label
        }
      }
    });
  });
});

// ---------------------------------------------------------------------------
// DisplayPolicyUpdated reprojects hidden redacted rows across timelines
// ---------------------------------------------------------------------------

describe("DisplayPolicyUpdated", () => {
  test("marks only redacted rows hidden while preserving non-redacted rows", () => {
    let store = createTimelineStore();
    const redacted: TimelineItem = {
      ...makeMsg("$redacted", ""),
      body: null,
      is_redacted: true,
      is_hidden: false
    };
    const visible = makeMsg("$visible", "Visible message");

    store = applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 0,
        items: [redacted, visible],
      },
    });

    store = applyTimelineEvent(store, {
      DisplayPolicyUpdated: {
        hide_redacted: true,
      },
    });

    let items = getItems(store, KEY);
    expect(items[0]).toMatchObject({ is_redacted: true, is_hidden: true });
    expect(items[1]).toMatchObject({ is_redacted: false, is_hidden: false });

    store = applyTimelineEvent(store, {
      DisplayPolicyUpdated: {
        hide_redacted: false,
      },
    });

    items = getItems(store, KEY);
    expect(items[0]).toMatchObject({ is_redacted: true, is_hidden: false });
    expect(items[1]).toMatchObject({ is_redacted: false, is_hidden: false });
  });

  test("reprojects a replay-known ready root snapshot just like a canonical redacted root", () => {
    const redacted: TimelineItem = {
      ...makeMsg("$canonical-redacted", ""),
      body: null,
      is_redacted: true,
      is_hidden: false
    };
    let store = applyTimelineEvent(createTimelineStore(), {
      InitialItems: { request_id: null, key: KEY, generation: 0, items: [redacted] }
    });
    store = applyTimelineEvent(store, {
      ThreadRootProjection: {
        key: KEY,
        projection: {
          root_event_id: "$replay-redacted",
          activity_event_id: "$latest:example.invalid",
          activity_timestamp_ms: 3_000,
          retain_without_reply: true,
          source: { kind: "replayKnown", epoch: 2 },
          state: {
            kind: "ready",
            item: { ...redacted, id: { Event: { event_id: "$replay-redacted" } } }
          }
        }
      }
    });

    store = applyTimelineEvent(store, { DisplayPolicyUpdated: { hide_redacted: true } });

    expect(getItems(store, KEY)[0]?.is_hidden).toBe(true);
    expect(getThreadRootProjections(store, KEY)[0]?.state).toMatchObject({
      kind: "ready",
      item: { is_redacted: true, is_hidden: true }
    });

    store = applyTimelineEvent(store, { DisplayPolicyUpdated: { hide_redacted: false } });
    expect(getThreadRootProjections(store, KEY)[0]?.state).toMatchObject({
      kind: "ready",
      item: { is_hidden: false }
    });
  });
});

// ---------------------------------------------------------------------------
// App-level store retention keeps active timelines and bounds inactive keys
// ---------------------------------------------------------------------------

describe("timeline store — retention", () => {
  function seedTimeline(store: TimelineStoreState, key: TimelineKey): TimelineStoreState {
    const roomId = "Room" in key.kind ? key.kind.Room.room_id : "timeline";
    return applyTimelineEvent(store, {
      InitialItems: {
        request_id: null,
        key,
        generation: 1,
        items: [makeMsg(`$${roomId}`, "seed")]
      }
    });
  }

  test("prunes oldest inactive keys while preserving retained keys", () => {
    let store = createTimelineStore();
    const keys = Array.from({ length: 6 }, (_value, index) =>
      roomTimelineKey(ACCOUNT_KEY, `!room-${index}:example.invalid`)
    );
    for (const key of keys) {
      store = seedTimeline(store, key);
    }

    const retainedKeyId = timelineStoreKeyId(keys[0]);
    store = pruneTimelineStore(store, new Set([retainedKeyId]), null, 2);

    const retained = [...store.keys.keys()];
    expect(retained).toContain(retainedKeyId);
    expect(retained.filter((keyId) => keyId !== retainedKeyId)).toEqual([
      timelineStoreKeyId(keys[4]),
      timelineStoreKeyId(keys[5])
    ]);
  });

  test("treats the event key as recently used before pruning", () => {
    let store = createTimelineStore();
    const keyA = roomTimelineKey(ACCOUNT_KEY, "!room-a:example.invalid");
    const keyB = roomTimelineKey(ACCOUNT_KEY, "!room-b:example.invalid");
    const keyC = roomTimelineKey(ACCOUNT_KEY, "!room-c:example.invalid");
    for (const key of [keyA, keyB, keyC]) {
      store = seedTimeline(store, key);
    }

    store = applyTimelineEventWithRetention(
      store,
      {
        ItemsUpdated: {
          key: keyA,
          generation: 1,
          batch_id: 2,
          diffs: [{ PushBack: { item: makeMsg("$a-live", "live") } }]
        }
      },
      new Set(),
      2
    );

    expect([...store.keys.keys()]).toEqual([
      timelineStoreKeyId(keyC),
      timelineStoreKeyId(keyA)
    ]);
    expect(getItems(store, keyA).map((item) => itemId(item))).toContain("$a-live");
  });

  test("evicts out-of-band root projections with their inactive timeline key", () => {
    let store = createTimelineStore();
    store = seedTimeline(store, KEY);
    store = applyTimelineEvent(store, {
      ThreadRootProjection: {
        key: KEY,
        projection: {
          root_event_id: "$evicted-root",
          activity_event_id: "$evicted-reply",
          activity_timestamp_ms: 1,
          state: { kind: "pending" }
        }
      }
    });

    store = pruneTimelineStore(store, new Set(), null, 0);

    expect(store.keys.has(timelineStoreKeyId(KEY))).toBe(false);
    expect(getThreadRootProjections(store, KEY)).toEqual([]);
  });

  test("keeps the projection map identity when pruning only an unrelated timeline key", () => {
    const retainedKey = roomTimelineKey(ACCOUNT_KEY, "!retained:example.invalid");
    const evictedKey = roomTimelineKey(ACCOUNT_KEY, "!evicted:example.invalid");
    let store = seedTimeline(createTimelineStore(), retainedKey);
    store = seedTimeline(store, evictedKey);
    const retainedReply = { ...makeMsg("$retained-reply", "reply"), thread_root: "$retained-root" };
    store = applyTimelineEvent(store, {
      InitialItems: { request_id: null, key: retainedKey, generation: 2, items: [retainedReply] }
    });
    store = applyTimelineEvent(store, {
      ThreadRootProjection: {
        key: retainedKey,
        projection: {
          root_event_id: "$retained-root",
          activity_event_id: "$retained-reply",
          activity_timestamp_ms: 2,
          state: { kind: "pending" }
        }
      }
    });
    const projectionsBeforePrune = store.threadRootProjections;

    store = pruneTimelineStore(store, new Set([timelineStoreKeyId(retainedKey)]), null, 0);

    expect(store.keys.has(timelineStoreKeyId(evictedKey))).toBe(false);
    expect(store.threadRootProjections).toBe(projectionsBeforePrune);
  });
});

// ---------------------------------------------------------------------------
// Additional: applyDiffs is a pure function (no mutation)
// ---------------------------------------------------------------------------

describe("applyDiffs — immutability", () => {
  test("original array is not mutated by applyDiffs", () => {
    const original = [makeMsg("$a", "a"), makeMsg("$b", "b")];
    const snapshot = [...original];

    applyDiffs(original, [{ PushBack: { item: makeMsg("$c", "c") } }]);

    expect(original).toHaveLength(2);
    expect(itemId(original[0])).toBe(itemId(snapshot[0]));
  });
});
