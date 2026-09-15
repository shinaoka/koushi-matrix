// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { StrictMode, Suspense, startTransition, useEffect, useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  focusedTimelineKey,
  roomTimelineKey,
  threadTimelineKey,
  timelineItemDomId,
  type CoreEventPayload,
  type TimelineGapId,
  type TimelineItem
} from "../domain/coreEvents";
import { setActiveLocaleProfile } from "../i18n/messages";
import {
  KEY,
  baseTransport,
  message,
  mockTimelineRects,
  navigationSnapshot
} from "./timelineViewTestSupport";
import {
  applyTimelineEvent,
  createTimelineStore,
  type TimelineStoreState
} from "../domain/timelineStore";
import { TimelineStoreContext } from "./timelineStoreContext";
import { TimelineView, clearTimelineViewportSessionMemoryForTests } from "./TimelineView";
import type { TimelineContinuityState } from "../domain/types";
import { resetTimelineTransportStats } from "../domain/timelineTransportStats";

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  setActiveLocaleProfile("en", "none");
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

/**
 * Gives every rendered timeline row a deterministic block position based on
 * its presentation order. Unlike `mockTimelineRects`, this intentionally
 * follows DOM reordering so a test can observe the viewport correction that a
 * display-projection transaction must make.
 */
function mockPresentationOrderRects(
  scrollContainerRef: { current: HTMLElement | null },
  options: { rowHeight?: number; viewportHeight?: number } = {}
) {
  const rowHeight = options.rowHeight ?? 100;
  const viewportHeight = options.viewportHeight ?? 200;
  return vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement
  ) {
    const testId = this.getAttribute("data-testid");
    if (testId === "timeline-view") {
      return {
        x: 0,
        y: 0,
        top: 0,
        left: 0,
        right: 0,
        width: 0,
        height: viewportHeight,
        bottom: viewportHeight,
        toJSON: () => ({})
      } as DOMRect;
    }

    const row = this.matches("article[data-item-id]")
      ? this
      : this.querySelector<HTMLElement>("article[data-item-id]");
    if (!row) {
      return {
        x: 0,
        y: 0,
        top: 0,
        left: 0,
        right: 0,
        width: 0,
        height: 0,
        bottom: 0,
        toJSON: () => ({})
      } as DOMRect;
    }
    const rows = Array.from(document.querySelectorAll<HTMLElement>("article[data-item-id]"));
    const index = rows.indexOf(row);
    const top = index * rowHeight - (scrollContainerRef.current?.scrollTop ?? 0);
    return {
      x: 0,
      y: top,
      top,
      left: 0,
      right: 0,
      width: 0,
      height: rowHeight,
      bottom: top + rowHeight,
      toJSON: () => ({})
    } as DOMRect;
  });
}

/**
 * Wraps a root item with the Rust-owned threadRoot display metadata the
 * renderer now consumes: a stable `thread-root:<event-id>` row identity plus
 * content/activity identities and the display timestamp. The accompanying
 * standalone reply row is suppressed upstream, so fixtures must not include
 * it either.
 */
function threadRootDisplayItem(
  item: TimelineItem,
  options: { activityEventId: string; displayTimestampMs?: number | null }
): TimelineItem {
  const contentEventId = "Event" in item.id ? item.id.Event.event_id : null;
  return {
    ...item,
    display_metadata: {
      row_id:
        contentEventId === null
          ? timelineItemDomId(item.id)
          : `thread-root:${contentEventId}`,
      kind: { kind: "threadRoot" },
      content_event_id: contentEventId,
      activity_event_id: options.activityEventId,
      display_timestamp_ms: options.displayTimestampMs ?? null
    }
  };
}


describe("TimelineView", () => {

  it("omits reply in thread from focused presentation while preserving ordinary reply", () => {
    const key = focusedTimelineKey(
      "@alice:example.invalid",
      "!room:example.invalid",
      "$focused:example.invalid"
    );
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key,
        generation: 1,
        items: [message("$focused-reply", "focused reply")]
      }
    });

    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          presentationContext="focused"
          timelineKey={key}
          roomId="!room:example.invalid"
          transport={baseTransport({})}
          onReply={vi.fn()}
          onOpenThread={vi.fn()}
        />
      </TimelineStoreContext.Provider>
    );

    const row = screen.getByText("focused reply").closest("article");
    expect(row).not.toBeNull();
    expect(within(row!).getByRole("button", { name: "Reply to message" })).not.toBeNull();
    expect(within(row!).queryByRole("button", { name: "Reply in thread" })).toBeNull();
  });


  it("omits every reply-composition affordance from thread presentation", () => {
    const key = threadTimelineKey(
      "@alice:example.invalid",
      "!room:example.invalid",
      "$thread-root:example.invalid"
    );
    const onOpenContextMenu = vi.fn();
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key,
        generation: 1,
        items: [
          {
            ...message("$thread-reply", "thread reply"),
            thread_root: "$thread-root:example.invalid",
            thread_summary: {
              reply_count: 2,
              latest_event_id: "$thread-latest:example.invalid",
              latest_sender: "@bob:example.invalid",
              latest_sender_label: null,
              latest_body_preview: "Latest",
              latest_timestamp_ms: 1_800_000_000_100
            }
          }
        ]
      }
    });

    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          presentationContext="thread"
          timelineKey={key}
          roomId="!room:example.invalid"
          currentUserId="@alice:example.invalid"
          transport={baseTransport({})}
          onReply={vi.fn()}
          onOpenThread={vi.fn()}
          onOpenContextMenu={onOpenContextMenu}
        />
      </TimelineStoreContext.Provider>
    );

    const row = screen.getByText("thread reply").closest("article");
    expect(row).not.toBeNull();
    expect(within(row!).queryByRole("button", { name: "Reply to message" })).toBeNull();
    expect(within(row!).queryByRole("button", { name: "Reply in thread" })).toBeNull();

    fireEvent.contextMenu(row!);
    expect(onOpenContextMenu).toHaveBeenCalledTimes(1);
    const menuItems = onOpenContextMenu.mock.calls[0][2] as Array<{ id: string }>;
    expect(menuItems.map((item) => item.id)).not.toContain("replyToMessage");
    expect(menuItems.map((item) => item.id)).not.toContain("openThread");
    // The menu still has to be useful for the remaining thread-event actions.
    expect(menuItems.length).toBeGreaterThan(0);
  });


  it("renders an incoming rich reply quote inside thread presentation", () => {
    const key = threadTimelineKey(
      "@alice:example.invalid",
      "!room:example.invalid",
      "$thread-root:example.invalid"
    );
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key,
        generation: 1,
        items: [
          {
            ...message("$thread-rich-reply", "Rich reply from another client"),
            thread_root: "$thread-root:example.invalid",
            in_reply_to_event_id: "$thread-earlier:example.invalid",
            reply_quote: {
              event_id: "$thread-earlier:example.invalid",
              sender: "@bob:example.invalid",
              sender_label: "Bob",
              body_preview: "Earlier thread event",
              state: "ready"
            }
          }
        ]
      }
    });

    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          presentationContext="thread"
          timelineKey={key}
          roomId="!room:example.invalid"
          transport={baseTransport({})}
          onReply={vi.fn()}
          onOpenThread={vi.fn()}
        />
      </TimelineStoreContext.Provider>
    );

    const row = screen.getByText("Rich reply from another client").closest("article");
    expect(row).not.toBeNull();
    const quote = row!.querySelector<HTMLElement>(".reply-quote");
    expect(quote?.getAttribute("data-reply-state")).toBe("ready");
    expect(quote?.textContent).toContain("Bob");
    expect(quote?.textContent).toContain("Earlier thread event");
  });


  it("preserves gap identity when the same thread root crosses it in latestReply mode", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const observeViewport = vi.fn().mockResolvedValue(undefined);
    const fullRangeGapId = {
      topology_revision: "14695981039346656037",
      ordinal: 0
    };
    const rootAtOrigin = threadRootDisplayItem(
      {
        ...message("$thread-root:example.invalid", "Thread root"),
        thread_summary: {
          reply_count: 1,
          latest_event_id: "$thread-reply:example.invalid",
          latest_sender: "@bob:example.invalid",
          latest_sender_label: "Bob",
          latest_body_preview: "Latest reply",
          latest_timestamp_ms: 1_800_000_010_000
        }
      },
      { activityEventId: "$thread-root:example.invalid" }
    );
    const rootAtActivity = threadRootDisplayItem(rootAtOrigin, {
      activityEventId: "$thread-reply:example.invalid",
      displayTimestampMs: 1_800_000_010_000
    });
    const beforeItem = message("$before:example.invalid", "Before");
    const betweenItem = message("$between:example.invalid", "Between");
    const afterItem = message("$after:example.invalid", "After");
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    const rectSpy = mockTimelineRects(
      {
        "$before:example.invalid": { top: -200, height: 40 },
        "$thread-root:example.invalid": { top: 40, height: 40 },
        "$between:example.invalid": { top: -200, height: 40 },
        "$thread-reply:example.invalid": { top: 160, height: 40 },
        "$after:example.invalid": { top: 800, height: 40 },
        "timeline-gap-row": { top: 100, height: 40 }
      },
      { top: 0, height: 500 },
      scrollContainerRef
    );
    const transport = baseTransport({
      observeViewport,
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });

    try {
      const view = () => (
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={transport}
          onReply={vi.fn()}
          continuity={{
            kind: "repairing",
            generation: 3,
            gap_count: 1,
            batches_processed: 0,
            minimum_batch_id: null
          }}
        />
      );
      render(view());

      const timeline = await screen.findByTestId("timeline-view");
      scrollContainerRef.current = timeline;
      Object.defineProperty(timeline, "clientHeight", { value: 500, configurable: true });
      Object.defineProperty(timeline, "scrollHeight", { value: 2_000, configurable: true });
      Object.defineProperty(timeline, "scrollTop", {
        value: 0,
        writable: true,
        configurable: true
      });

      act(() => {
        emit({
          kind: "Timeline",
          event: {
            InitialItems: {
              request_id: null,
              key: KEY,
              generation: 1,
              items: [beforeItem, rootAtOrigin, betweenItem, afterItem]
            }
          }
        });
        emit({
          kind: "Timeline",
          event: {
            GapPositionsUpdated: {
              key: KEY,
              actor_generation: 0,
              generation: 3,
              positions: [
                {
                  id: fullRangeGapId,
                  before_item_index: 3
                }
              ]
            }
          }
        });
      });

      timeline.scrollTop = 0;
      fireEvent.wheel(timeline, { deltaY: 1 });
      fireEvent.scroll(timeline);

      await waitFor(() => {
        expect(observeViewport).toHaveBeenCalledWith(
          "!room:example.invalid",
          "$thread-root:example.invalid",
          "$thread-root:example.invalid",
          [fullRangeGapId],
          false,
          // #872: the observation reports how the viewport arrived.
          "user",
          null
        );
      });
      const gap = screen.getByTestId("timeline-gap-row");
      const root = screen.getByText("Thread root").closest<HTMLElement>("article");
      expect(root).not.toBeNull();
      expect(root?.dataset["rowId"]).toBe("thread-root:$thread-root:example.invalid");
      expect(root?.dataset["contentEventId"]).toBe("$thread-root:example.invalid");
      expect(root?.dataset["activityEventId"]).toBe("$thread-root:example.invalid");
      expect(root!.compareDocumentPosition(gap) & Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(0);
      expect(gap.dataset["gapTopologyRevision"]).toBe(fullRangeGapId.topology_revision);
      expect(gap.dataset["gapOrdinal"]).toBe(String(fullRangeGapId.ordinal));

      observeViewport.mockClear();
      act(() => {
        emit({
          kind: "Timeline",
          event: {
            ItemsUpdated: {
              key: KEY,
              generation: 1,
              batch_id: 1,
              diffs: [
                {
                  Reset: {
                    items: [beforeItem, betweenItem, rootAtActivity, afterItem]
                  }
                }
              ]
            }
          }
        });
        // The same gap moves one display slot later: the root row now occupies
        // the activity slot after it, so the renderer keeps the identical gap
        // placeholder (same generation) while the root crosses it.
        emit({
          kind: "Timeline",
          event: {
            GapPositionsUpdated: {
              key: KEY,
              actor_generation: 0,
              generation: 3,
              positions: [{ id: fullRangeGapId, before_item_index: 2 }]
            }
          }
        });
      });
      fireEvent.scroll(timeline);

      await waitFor(() => {
        const movedRoot = screen.getByText("Thread root").closest<HTMLElement>("article");
        expect(movedRoot).toBe(root);
        expect(gap.compareDocumentPosition(movedRoot!) & Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(
          0
        );
        expect(movedRoot?.dataset["activityEventId"]).toBe("$thread-reply:example.invalid");
      });
      expect(screen.getByTestId("timeline-gap-row")).toBe(gap);
      await waitFor(() => {
        expect(observeViewport).toHaveBeenCalledWith(
          "!room:example.invalid",
          "$thread-reply:example.invalid",
          "$thread-reply:example.invalid",
          [fullRangeGapId],
          false,
          // #872: the observation reports how the viewport arrived.
          "user",
          null
        );
      });
    } finally {
      rectSpy.mockRestore();
    }
  });


  it("covers selected-room persisted gap recovery through live history and room switch", async () => {
    const observeViewport = vi.fn().mockResolvedValue(undefined);
    const gapId = { topology_revision: "14695981039346656037", ordinal: 0 };
    const otherRoomId = "!other-room:example.invalid";
    const otherKey = roomTimelineKey("@alice:example.invalid", otherRoomId);
    const rootAtOrigin = threadRootDisplayItem(
      {
        ...message("$persisted-thread-root:example.invalid", "Persisted thread root"),
        thread_summary: {
          reply_count: 1,
          latest_event_id: "$persisted-thread-reply:example.invalid",
          latest_sender: "@bob:example.invalid",
          latest_sender_label: "Bob",
          latest_body_preview: "Latest persisted reply",
          latest_timestamp_ms: 1_800_000_010_000
        }
      },
      { activityEventId: "$persisted-thread-root:example.invalid" }
    );
    const rootAtActivity = threadRootDisplayItem(rootAtOrigin, {
      activityEventId: "$persisted-thread-reply:example.invalid",
      displayTimestampMs: 1_800_000_010_000
    });
    const beforeItem = message("$persisted-before:example.invalid", "Before persisted gap");
    const betweenItem = message(
      "$persisted-between:example.invalid",
      "Between root and gap"
    );
    const liveEvent = message("$persisted-live:example.invalid", "New live event");
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    const rectSpy = mockTimelineRects(
      {
        "$persisted-before:example.invalid": { top: -200, height: 40 },
        "$persisted-thread-root:example.invalid": { top: 40, height: 40 },
        "$persisted-between:example.invalid": { top: -200, height: 40 },
        "$persisted-thread-reply:example.invalid": { top: 160, height: 40 },
        "$persisted-live:example.invalid": { top: 600, height: 40 },
        "$other-room-event:example.invalid": { top: 40, height: 40 },
        "timeline-gap-row": { top: 100, height: 40 }
      },
      { top: 0, height: 500 },
      scrollContainerRef
    );
    const transport = baseTransport({ observeViewport });
    const repairing = {
      kind: "repairing" as const,
      generation: 31,
      gap_count: 1,
      batches_processed: 0,
      minimum_batch_id: null
    };
    let store = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        actor_generation: 0,
        generation: 1,
        items: [beforeItem, rootAtOrigin, betweenItem]
      }
    });
    store = applyTimelineEvent(store, {
      GapPositionsUpdated: {
        key: KEY,
        actor_generation: 0,
        generation: 31,
        positions: [{ id: gapId, before_item_index: 3 }]
      }
    });
    const setStore = vi.fn();
    const view = (
      timelineKey: typeof KEY,
      roomId: string,
      continuity: TimelineContinuityState
    ) => (
      <TimelineView
        timelineKey={timelineKey}
        roomId={roomId}
        transport={transport}
        onReply={vi.fn()}
        continuity={continuity}
        timelineStore={store}
        setTimelineStore={setStore}
      />
    );
    const oldRoomGapObservations = () =>
      observeViewport.mock.calls.filter(
        ([roomId, , , visibleGapIds]) =>
          roomId === "!room:example.invalid" &&
          (visibleGapIds as TimelineGapId[]).some(
            (id) =>
              id.topology_revision === gapId.topology_revision && id.ordinal === gapId.ordinal
          )
      );

    try {
      const { rerender } = render(view(KEY, "!room:example.invalid", repairing));
      const timeline = await screen.findByTestId("timeline-view");
      scrollContainerRef.current = timeline;
      Object.defineProperty(timeline, "clientHeight", { value: 500, configurable: true });
      Object.defineProperty(timeline, "scrollHeight", { value: 1_000, configurable: true });
      Object.defineProperty(timeline, "scrollTop", {
        value: 0,
        writable: true,
        configurable: true
      });
      // Mount observations happen before jsdom receives stable dimensions.
      observeViewport.mockClear();

      fireEvent.wheel(timeline, { deltaY: 1 });
      fireEvent.scroll(timeline);
      await waitFor(() => {
        expect(observeViewport).toHaveBeenCalledWith(
          "!room:example.invalid",
          "$persisted-thread-root:example.invalid",
          "$persisted-thread-root:example.invalid",
          [gapId],
          false,
          // #872: the observation reports how the viewport arrived.
          "user",
          null
        );
      });
      const gap = screen.getByTestId("timeline-gap-row");
      const root = screen.getByText("Persisted thread root").closest<HTMLElement>("article");
      expect(root).not.toBeNull();
      expect(root!.compareDocumentPosition(gap) & Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(0);
      expect(oldRoomGapObservations()).toHaveLength(1);
      fireEvent.scroll(timeline);
      await act(async () => Promise.resolve());
      expect(oldRoomGapObservations()).toHaveLength(1);

      // The relocated projection is a fresh Rust display reset: the root row
      // moves to the activity slot (same stable row identity) and the gap's
      // display index follows it, still on the same gap generation.
      store = applyTimelineEvent(store, {
        ItemsUpdated: {
          key: KEY,
          generation: 1,
          batch_id: 1,
          diffs: [
            {
              Reset: {
                items: [beforeItem, betweenItem, rootAtActivity]
              }
            }
          ]
        }
      });
      store = applyTimelineEvent(store, {
        GapPositionsUpdated: {
          key: KEY,
          actor_generation: 0,
          generation: 31,
          positions: [{ id: gapId, before_item_index: 2 }]
        }
      });
      rerender(view(KEY, "!room:example.invalid", repairing));
      await waitFor(() => {
        const movedRoot = screen
          .getByText("Persisted thread root")
          .closest<HTMLElement>("article");
        expect(movedRoot).toBe(root);
        expect(screen.getByTestId("timeline-gap-row")).toBe(gap);
        expect(gap.compareDocumentPosition(movedRoot!) & Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(
          0
        );
        expect(movedRoot?.dataset["activityEventId"]).toBe(
          "$persisted-thread-reply:example.invalid"
        );
      });

      store = applyTimelineEvent(store, {
        ItemsUpdated: {
          key: KEY,
          generation: 1,
          batch_id: 6,
          diffs: [{ PushBack: { item: liveEvent } }]
        }
      });
      const repairingAfterBatch = {
        ...repairing,
        batches_processed: 1,
        minimum_batch_id: 6
      };
      rerender(view(KEY, "!room:example.invalid", repairingAfterBatch));
      const liveRow = await screen.findByText("New live event").then((node) =>
        node.closest<HTMLElement>("article")
      );
      expect(liveRow).not.toBeNull();
      expect(gap.compareDocumentPosition(liveRow!) & Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(0);

      timeline.scrollTop = 500;
      fireEvent.wheel(timeline, { deltaY: 1 });
      fireEvent.scroll(timeline);
      await waitFor(() => {
        expect(observeViewport).toHaveBeenCalledWith(
          "!room:example.invalid",
          "$persisted-live:example.invalid",
          "$persisted-live:example.invalid",
          [],
          true,
          // #872: the wheel was followed by the client's own live-edge placement,
          // which is what moved the viewport to the bottom.
          "programmatic",
          null
        );
      });
      rerender(view(KEY, "!room:example.invalid", repairingAfterBatch));
      fireEvent.scroll(timeline);
      await act(async () => Promise.resolve());

      rerender(view(KEY, "!room:example.invalid", repairingAfterBatch));
      timeline.scrollTop = 0;
      fireEvent.wheel(timeline, { deltaY: -1 });
      fireEvent.scroll(timeline);
      await waitFor(() => expect(oldRoomGapObservations()).toHaveLength(2));
      expect(screen.getByTestId("timeline-gap-row")).toBe(gap);
      expect(oldRoomGapObservations().at(-1)?.[3]).toEqual([gapId]);
      expect(oldRoomGapObservations().every((call) => call[3].length === 1)).toBe(true);

      store = applyTimelineEvent(store, {
        GapPositionsUpdated: {
          key: KEY,
          actor_generation: 0,
          generation: 32,
          positions: []
        }
      });
      rerender(view(KEY, "!room:example.invalid", repairingAfterBatch));
      await waitFor(() => expect(screen.queryByTestId("timeline-gap-row")).toBeNull());
      await waitFor(() => {
        expect(observeViewport).toHaveBeenCalledWith(
          "!room:example.invalid",
          "$persisted-thread-reply:example.invalid",
          "$persisted-thread-reply:example.invalid",
          [],
          false,
          // #872: the observation reports how the viewport arrived.
          "user",
          null
        );
      });

      const oldGapObservationCount = oldRoomGapObservations().length;
      store = applyTimelineEvent(store, {
        InitialItems: {
          request_id: null,
          key: otherKey,
          actor_generation: 10,
          generation: 1,
          items: [message("$other-room-event:example.invalid", "Other room event")]
        }
      });
      store = applyTimelineEvent(store, {
        GapPositionsUpdated: {
          key: KEY,
          actor_generation: 0,
          generation: 33,
          positions: [{ id: gapId, before_item_index: 3 }]
        }
      });
      rerender(
        view(otherKey, otherRoomId, {
          kind: "healthy",
          generation: 1,
          authoritative_start: false
        })
      );
      timeline.scrollTop = 0;
      fireEvent.scroll(timeline);
      await waitFor(() => {
        expect(observeViewport).toHaveBeenCalledWith(
          otherRoomId,
          "$other-room-event:example.invalid",
          "$other-room-event:example.invalid",
          [],
          true,
          // #872: switching rooms placed this viewport, so no reader arrived.
          "programmatic",
          null
        );
      });
      expect(screen.queryByTestId("timeline-gap-row")).toBeNull();
      expect(oldRoomGapObservations()).toHaveLength(oldGapObservationCount);
    } finally {
      rectSpy.mockRestore();
    }
  });


  it("emits safe timestamped timeline event diagnostics for thread timelines", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const onDiagnosticLogEntry = vi.fn();
    const threadKey = threadTimelineKey(
      "@alice:example.invalid",
      "!room:example.invalid",
      "$root:example.invalid"
    );
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });

    render(
      <TimelineView
        timelineKey={threadKey}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        onDiagnosticLogEntry={onDiagnosticLogEntry}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: threadKey,
          generation: 3,
          items: [message("$root:example.invalid", "Thread root")]
        }
      }
    });
    emit({
      kind: "Timeline",
      event: {
        PaginationStateChanged: {
          request_id: null,
          key: threadKey,
          direction: "Backward",
          state: "EndReached"
        }
      }
    });

    await waitFor(() => {
      expect(onDiagnosticLogEntry).toHaveBeenCalledWith(
        expect.objectContaining({
          source: "timeline.event",
          message: "kind=thread initial items=1 generation=3"
        })
      );
      expect(onDiagnosticLogEntry).toHaveBeenCalledWith(
        expect.objectContaining({
          source: "timeline.event",
          message: "kind=thread pagination direction=Backward state=EndReached"
        })
      );
    });
    expect(onDiagnosticLogEntry.mock.calls.map(([entry]) => entry.message).join("\n")).not.toContain(
      "$root"
    );
  });


  it("emits privacy-safe focused store lookup and event-key mismatch diagnostics", async () => {
    resetTimelineTransportStats();
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const onDiagnosticLogEntry = vi.fn();
    const focusedKey = focusedTimelineKey(
      "@alice:example.invalid",
      "!room:example.invalid",
      "$target:example.invalid"
    );
    const otherKey = focusedTimelineKey(
      "@alice:example.invalid",
      "!room:example.invalid",
      "$other:example.invalid"
    );
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });

    render(
      <TimelineView
        timelineKey={focusedKey}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        onDiagnosticLogEntry={onDiagnosticLogEntry}
        timelineStore={createTimelineStore()}
      />
    );

    await waitFor(() => {
      expect(onDiagnosticLogEntry).toHaveBeenCalledWith(
        expect.objectContaining({
          source: "timeline.store",
          message: expect.stringContaining(
            "stage=lookup kind=focused"
          ) as unknown as string
        })
      );
    });

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: { connection_id: 9, sequence: 1 },
          key: otherKey,
          actor_generation: 1,
          generation: 1,
          items: []
        }
      }
    });

    await waitFor(() => {
      expect(onDiagnosticLogEntry).toHaveBeenCalledWith(
        expect.objectContaining({
          source: "timeline.key",
          message: expect.stringContaining(
            "stage=event_dropped_summary current_kind=focused event_kind=focused"
          ) as unknown as string
        })
      );
    });
    const diagnostics = onDiagnosticLogEntry.mock.calls
      .map(([entry]) => `${entry.source} ${entry.message}`)
      .join("\n");
    expect(diagnostics).toContain("account_match=true");
    expect(diagnostics).toContain("room_match=true");
    expect(diagnostics).not.toContain("@alice:example.invalid");
    expect(diagnostics).not.toContain("!room:example.invalid");
    expect(diagnostics).not.toContain("$target:example.invalid");
    expect(diagnostics).not.toContain("$other:example.invalid");
  });


  it("centers the focused target instead of restoring the focused window to live edge", async () => {
    const originalScrollIntoView = Element.prototype.scrollIntoView;
    const scrollIntoView = vi.fn();
    Element.prototype.scrollIntoView = scrollIntoView;
    try {
      let emit: (payload: CoreEventPayload) => void = () => undefined;
      const onDiagnosticLogEntry = vi.fn();
      const focusedKey = focusedTimelineKey(
        "@alice:example.invalid",
        "!room:example.invalid",
        "$focused-target:example.invalid"
      );
      const transport = baseTransport({
        listenCoreEvents(nextListener) {
          emit = nextListener;
          return () => undefined;
        }
      });

      render(
        <TimelineView
          timelineKey={focusedKey}
          roomId="!room:example.invalid"
          transport={transport}
          onReply={vi.fn()}
          onDiagnosticLogEntry={onDiagnosticLogEntry}
        />
      );

      const timeline = screen.getByTestId("timeline-view");
      Object.defineProperty(timeline, "clientHeight", { value: 400, configurable: true });
      Object.defineProperty(timeline, "scrollHeight", { value: 1_800, configurable: true });
      Object.defineProperty(timeline, "scrollTop", {
        value: 0,
        writable: true,
        configurable: true
      });

      act(() => {
        emit({
          kind: "Timeline",
          event: {
            InitialItems: {
              request_id: null,
              key: focusedKey,
              generation: 1,
              items: [
                message("$focused-older:example.invalid", "Older"),
                message("$focused-target:example.invalid", "Target"),
                message("$focused-newer:example.invalid", "Newer")
              ]
            }
          }
        });
      });

      await waitFor(() => expect(scrollIntoView).toHaveBeenCalledTimes(1));
      const targetRow = scrollIntoView.mock.instances[0] as HTMLElement | undefined;
      expect(targetRow?.getAttribute("data-activity-event-id")).toBe(
        "$focused-target:example.invalid"
      );
      expect(onDiagnosticLogEntry).toHaveBeenCalledWith(
        expect.objectContaining({
          source: "timeline.scroll",
          message: "stage=focused_target_restore path=dom target_present=true"
        })
      );
      expect(
        onDiagnosticLogEntry.mock.calls.some(
          ([entry]) =>
            entry.source === "timeline.scroll" &&
            entry.message.includes("stage=room_reentry_restore") &&
            entry.message.includes("path=live_edge")
        )
      ).toBe(false);
    } finally {
      Element.prototype.scrollIntoView = originalScrollIntoView;
    }
  });


  it("records a deduplicated committed thread projection", async () => {
    const onDiagnosticLogEntry = vi.fn();
    const threadKey = threadTimelineKey(
      "@alice:example.invalid",
      "!room:example.invalid",
      "$root:example.invalid"
    );
    let store = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: threadKey,
        actor_generation: 5,
        generation: 3,
        items: [message("$root:example.invalid", "Thread root")]
      }
    });
    store = applyTimelineEvent(store, {
      ItemsUpdated: {
        key: threadKey,
        generation: 3,
        batch_id: 7,
        diffs: [{ PushBack: { item: message("$reply:example.invalid", "Reply") } }]
      }
    });

    const view = (
      <TimelineView
        timelineKey={threadKey}
        roomId="!room:example.invalid"
        transport={baseTransport({})}
        onReply={vi.fn()}
        onDiagnosticLogEntry={onDiagnosticLogEntry}
        timelineStore={store}
      />
    );
    const { rerender } = render(view);

    await waitFor(() => {
      expect(onDiagnosticLogEntry).toHaveBeenCalledWith(
        expect.objectContaining({
          source: "thread.timeline",
          message: "stage=committed actor=5 generation=3 batch=7 items=2"
        })
      );
    });
    rerender(view);
    expect(
      onDiagnosticLogEntry.mock.calls.filter(
        ([entry]) =>
          entry.source === "thread.timeline" &&
          entry.message === "stage=committed actor=5 generation=3 batch=7 items=2"
      )
    ).toHaveLength(1);
  });


  it("may request renderer auto-load after Core confirms an authoritative empty thread", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const threadKey = threadTimelineKey(
      "@alice:example.invalid",
      "!room:example.invalid",
      "$root:example.invalid"
    );
    const paginateBackwards = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      paginateBackwards
    });

    render(
      <TimelineView
        timelineKey={threadKey}
        roomId="!room:example.invalid"
        transport={transport}
        autoLoadOlderMessages
        onReply={vi.fn()}
      />
    );
    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperty(timeline, "clientHeight", {
      value: 600,
      configurable: true
    });
    Object.defineProperty(timeline, "scrollHeight", {
      value: 0,
      configurable: true
    });

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: threadKey,
            generation: 0,
            items: []
          }
        }
      });
    });

    await waitFor(() => {
      expect(paginateBackwards).toHaveBeenCalledWith(threadKey);
    });
    expect(paginateBackwards).toHaveBeenCalledTimes(1);
  });


  it("keeps a new-thread draft out of backfill and hides stale pagination state", async () => {
    const threadKey = threadTimelineKey(
      "@alice:example.invalid",
      "!room:example.invalid",
      "$root:example.invalid"
    );
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const paginateBackwards = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      paginateBackwards
    });

    render(
      <TimelineView
        timelineKey={threadKey}
        roomId="!room:example.invalid"
        transport={transport}
        autoLoadOlderMessages
        automaticBackfillEligible={false}
        onReply={vi.fn()}
      />
    );

    expect(paginateBackwards).not.toHaveBeenCalled();

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: threadKey,
          generation: 1,
          items: []
        }
      }
    });

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          PaginationStateChanged: {
            request_id: null,
            key: threadKey,
            direction: "Backward",
            state: "Paginating"
          }
        }
      });
      emit({
        kind: "Timeline",
        event: {
          PaginationStateChanged: {
            request_id: null,
            key: threadKey,
            direction: "Backward",
            state: "Idle"
          }
        }
      });
      emit({
        kind: "Timeline",
        event: {
          GapPositionsUpdated: {
            key: threadKey,
            actor_generation: 1,
            generation: 2,
            positions: []
          }
        }
      });
      emit({
        kind: "Timeline",
        event: {
          GapRepairReleased: {
            key: threadKey,
            actor_generation: 1,
            generation: 3
          }
        }
      });
    });

    await act(async () => Promise.resolve());
    expect(paginateBackwards).not.toHaveBeenCalled();
    expect(screen.queryByTestId("timeline-spinner")).toBeNull();
  });


  it("keeps a committed projection compensation when StrictMode abandons a later render", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    let controls: {
      setShouldSuspend: (shouldSuspend: boolean) => void;
      refresh: () => void;
    } | null = null;
    const suspended = new Promise<never>(() => undefined);
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    const rectMock = mockPresentationOrderRects(scrollContainerRef);
    const rootAtOrigin = threadRootDisplayItem(
      {
        ...message("$thread-root:example.invalid", "Thread root"),
        thread_summary: {
          reply_count: 1,
          latest_event_id: "$latest-thread-reply:example.invalid",
          latest_sender: "@bob:example.invalid",
          latest_sender_label: "Bob",
          latest_body_preview: "Latest reply",
          latest_timestamp_ms: 1_800_000_001_000
        }
      },
      { activityEventId: "$thread-root:example.invalid" }
    );
    const rootAtActivity = threadRootDisplayItem(rootAtOrigin, {
      activityEventId: "$latest-thread-reply:example.invalid",
      displayTimestampMs: 1_800_000_001_000
    });
    const beforeItem = message("$before:example.invalid", "Before");
    const betweenItem = message("$between:example.invalid", "Between");
    const afterItem = message("$after:example.invalid", "After");
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    function SuspendsAfterTimeline({ shouldSuspend }: { shouldSuspend: boolean }) {
      if (shouldSuspend) {
        throw suspended;
      }
      return null;
    }
    function Harness() {
      const [shouldSuspend, setShouldSuspend] = useState(false);
      const [, setVersion] = useState(0);
      useEffect(() => {
        controls = {
          setShouldSuspend,
          refresh: () => setVersion((current) => current + 1)
        };
      });
      return (
        <Suspense fallback={null}>
          <TimelineView
            timelineKey={KEY}
            roomId="!room:example.invalid"
            transport={transport}
            onReply={vi.fn()}
          />
          <SuspendsAfterTimeline shouldSuspend={shouldSuspend} />
        </Suspense>
      );
    }

    render(
      <StrictMode>
        <Harness />
      </StrictMode>
    );
    await waitFor(() => expect(controls).not.toBeNull());
    const timeline = await screen.findByTestId("timeline-view");
    scrollContainerRef.current = timeline;
    Object.defineProperty(timeline, "clientHeight", { value: 200, configurable: true });
    Object.defineProperty(timeline, "scrollHeight", { value: 1_000, configurable: true });
    Object.defineProperty(timeline, "scrollTop", {
      value: 0,
      writable: true,
      configurable: true
    });
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [beforeItem, rootAtOrigin, betweenItem, afterItem]
          }
        }
      });
    });

    timeline.scrollTop = 800;
    timeline.scrollTop = 190;
    fireEvent.wheel(timeline, { deltaY: -1 });

    vi.useFakeTimers();
    const frames = new Map<number, FrameRequestCallback>();
    let nextFrameId = 0;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      nextFrameId += 1;
      frames.set(nextFrameId, callback);
      return nextFrameId;
    });
    vi.spyOn(window, "cancelAnimationFrame").mockImplementation((frameId) => {
      frames.delete(frameId);
    });

    // B commits (the Rust display Reset relocates the root to its activity
    // slot) and queues its free-scroll correction. C starts afterwards, but
    // suspends before it can commit; B remains the visible projection.
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key: KEY,
            generation: 1,
            batch_id: 1,
            diffs: [
              {
                Reset: {
                  items: [beforeItem, betweenItem, rootAtActivity, afterItem]
                }
              }
            ]
          }
        }
      });
    });
    expect(
      document
        .querySelector('[data-content-event-id="$thread-root:example.invalid"]')
        ?.getAttribute("data-activity-event-id")
    ).toBe("$latest-thread-reply:example.invalid");
    act(() => {
      startTransition(() => {
        controls!.setShouldSuspend(true);
      });
    });
    expect(
      document
        .querySelector('[data-content-event-id="$thread-root:example.invalid"]')
        ?.getAttribute("data-activity-event-id")
    ).toBe("$latest-thread-reply:example.invalid");

    act(() => {
      const queued = [...frames.values()];
      frames.clear();
      for (const callback of queued) {
        callback(0);
      }
    });

    expect(timeline.scrollTop).toBe(90);
    rectMock.mockRestore();
  });


  it("does not overwrite a user scroll that happens after projection compensation is queued", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const onScrollDiagnosticsChange = vi.fn();
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    const rectMock = mockPresentationOrderRects(scrollContainerRef);
    const rootAtOrigin = threadRootDisplayItem(
      {
        ...message("$thread-root:example.invalid", "Thread root"),
        thread_summary: {
          reply_count: 1,
          latest_event_id: "$latest-thread-reply:example.invalid",
          latest_sender: "@bob:example.invalid",
          latest_sender_label: "Bob",
          latest_body_preview: "Latest reply",
          latest_timestamp_ms: 1_800_000_001_000
        }
      },
      { activityEventId: "$thread-root:example.invalid" }
    );
    const rootAtActivity = threadRootDisplayItem(rootAtOrigin, {
      activityEventId: "$latest-thread-reply:example.invalid",
      displayTimestampMs: 1_800_000_001_000
    });
    const beforeItem = message("$before:example.invalid", "Before");
    const betweenItem = message("$between:example.invalid", "Between");
    const afterItem = message("$after:example.invalid", "After");
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const renderView = () => (
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        onScrollDiagnosticsChange={onScrollDiagnosticsChange}
      />
    );
    render(renderView());

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [beforeItem, rootAtOrigin, betweenItem, afterItem]
          }
        }
      });
    });

    await screen.findByText("Between");
    const timeline = screen.getByTestId("timeline-view");
    scrollContainerRef.current = timeline;
    Object.defineProperty(timeline, "clientHeight", { value: 200, configurable: true });
    Object.defineProperty(timeline, "scrollHeight", { value: 1_000, configurable: true });
    Object.defineProperty(timeline, "scrollTop", {
      value: 800,
      writable: true,
      configurable: true
    });
    timeline.scrollTop = 190;
    fireEvent.wheel(timeline, { deltaY: -1 });

    vi.useFakeTimers();
    const frames = new Map<number, FrameRequestCallback>();
    let nextFrameId = 0;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      nextFrameId += 1;
      frames.set(nextFrameId, callback);
      return nextFrameId;
    });
    vi.spyOn(window, "cancelAnimationFrame").mockImplementation((frameId) => {
      frames.delete(frameId);
    });
    // The Rust display Reset relocates the root to its activity slot and
    // queues the projection's free-scroll correction.
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key: KEY,
            generation: 1,
            batch_id: 1,
            diffs: [
              {
                Reset: {
                  items: [beforeItem, betweenItem, rootAtActivity, afterItem]
                }
              }
            ]
          }
        }
      });
    });

    // A real user scroll takes ownership while the projection's frame is held.
    timeline.scrollTop = 250;
    fireEvent.wheel(timeline, { deltaY: -1 });
    fireEvent.scroll(timeline);
    act(() => {
      const queued = [...frames.values()];
      frames.clear();
      for (const callback of queued) {
        callback(0);
      }
    });

    expect(timeline.scrollTop).toBe(250);
    expect(
      onScrollDiagnosticsChange.mock.calls.some(
        ([diagnostics]) => diagnostics.scrollWrites.projectionCompensation > 0
      )
    ).toBe(false);
    rectMock.mockRestore();
  });


  it("does not apply queued projection compensation after a jump takes viewport ownership", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    let jumpToLatest: (() => void) | null = null;
    const onScrollDiagnosticsChange = vi.fn();
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    const rectMock = mockPresentationOrderRects(scrollContainerRef);
    const rootAtOrigin = threadRootDisplayItem(
      {
        ...message("$thread-root:example.invalid", "Thread root"),
        thread_summary: {
          reply_count: 1,
          latest_event_id: "$latest-thread-reply:example.invalid",
          latest_sender: "@bob:example.invalid",
          latest_sender_label: "Bob",
          latest_body_preview: "Latest reply",
          latest_timestamp_ms: 1_800_000_001_000
        }
      },
      { activityEventId: "$thread-root:example.invalid" }
    );
    const rootAtActivity = threadRootDisplayItem(rootAtOrigin, {
      activityEventId: "$latest-thread-reply:example.invalid",
      displayTimestampMs: 1_800_000_001_000
    });
    const beforeItem = message("$before:example.invalid", "Before");
    const betweenItem = message("$between:example.invalid", "Between");
    const afterItem = message("$after:example.invalid", "After");
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const renderView = () => (
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        onRegisterJumpToLatest={(handler) => {
          jumpToLatest = handler;
        }}
        onScrollDiagnosticsChange={onScrollDiagnosticsChange}
      />
    );
    render(renderView());

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [beforeItem, rootAtOrigin, betweenItem, afterItem]
          }
        }
      });
    });

    await screen.findByText("Between");
    const timeline = screen.getByTestId("timeline-view");
    scrollContainerRef.current = timeline;
    Object.defineProperty(timeline, "clientHeight", { value: 200, configurable: true });
    Object.defineProperty(timeline, "scrollHeight", { value: 1_000, configurable: true });
    Object.defineProperty(timeline, "scrollTop", {
      value: 800,
      writable: true,
      configurable: true
    });
    timeline.scrollTop = 190;
    fireEvent.wheel(timeline, { deltaY: -1 });

    vi.useFakeTimers();
    const frames = new Map<number, FrameRequestCallback>();
    let nextFrameId = 0;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      nextFrameId += 1;
      frames.set(nextFrameId, callback);
      return nextFrameId;
    });
    vi.spyOn(window, "cancelAnimationFrame").mockImplementation((frameId) => {
      frames.delete(frameId);
    });
    // The Rust display Reset relocates the root to its activity slot and
    // queues the projection's free-scroll correction.
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key: KEY,
            generation: 1,
            batch_id: 1,
            diffs: [
              {
                Reset: {
                  items: [beforeItem, betweenItem, rootAtActivity, afterItem]
                }
              }
            ]
          }
        }
      });
    });

    act(() => {
      jumpToLatest?.();
    });
    expect(timeline.scrollTop).toBe(800);
    act(() => {
      const queued = [...frames.values()];
      frames.clear();
      for (const callback of queued) {
        callback(0);
      }
    });

    expect(timeline.scrollTop).toBe(800);
    expect(
      onScrollDiagnosticsChange.mock.calls.some(
        ([diagnostics]) => diagnostics.scrollWrites.projectionCompensation > 0
      )
    ).toBe(false);
    rectMock.mockRestore();
  });


  it("renders an unread latest-reply marker before the root block that represents it", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const rootAtActivity = threadRootDisplayItem(
      {
        ...message("$thread-root:example.invalid", "Thread root"),
        thread_summary: {
          reply_count: 1,
          latest_event_id: "$latest-thread-reply:example.invalid",
          latest_sender: "@bob:example.invalid",
          latest_sender_label: "Bob",
          latest_body_preview: "Latest reply",
          latest_timestamp_ms: 1_800_000_001_000
        }
      },
      {
        activityEventId: "$latest-thread-reply:example.invalid",
        displayTimestampMs: 1_800_000_001_000
      }
    );
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          NavigationUpdated: {
            key: KEY,
            snapshot: navigationSnapshot({
              first_unread_event_id: "$latest-thread-reply:example.invalid",
              unread_event_count: 1,
              unread_position: "insideViewport"
            })
          }
        }
      });
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [message("$before:example.invalid", "Before"), rootAtActivity]
          }
        }
      });
    });

    const marker = await screen.findByRole("separator", { name: "Unread messages" });
    const rootRow = marker.nextElementSibling;
    expect(rootRow?.getAttribute("data-content-event-id")).toBe("$thread-root:example.invalid");
    expect(rootRow?.getAttribute("data-activity-event-id")).toBe(
      "$latest-thread-reply:example.invalid"
    );
  });


  it("renders an unread marker before a moved root by its latest activity identity", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const rootAtActivity = threadRootDisplayItem(
      {
        ...message("$thread-root:example.invalid", "Thread root"),
        thread_summary: {
          reply_count: 1,
          latest_event_id: "$latest-thread-reply:example.invalid",
          latest_sender: "@bob:example.invalid",
          latest_sender_label: "Bob",
          latest_body_preview: "Latest reply",
          latest_timestamp_ms: 1_800_000_001_000
        }
      },
      {
        activityEventId: "$latest-thread-reply:example.invalid",
        displayTimestampMs: 1_800_000_001_000
      }
    );
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          NavigationUpdated: {
            key: KEY,
            snapshot: navigationSnapshot({
              first_unread_event_id: "$latest-thread-reply:example.invalid",
              unread_event_count: 1,
              unread_position: "belowViewport"
            })
          }
        }
      });
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [message("$before:example.invalid", "Before"), rootAtActivity]
          }
        }
      });
    });

    const marker = await screen.findByRole("separator", { name: "Unread messages" });
    const rootRow = marker.nextElementSibling;
    expect(rootRow?.getAttribute("data-content-event-id")).toBe("$thread-root:example.invalid");
    expect(rootRow?.getAttribute("data-activity-event-id")).toBe(
      "$latest-thread-reply:example.invalid"
    );
  });


  it("keeps live edge pinned when a summary Set relocates its root block", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const oldRootAtActivity = threadRootDisplayItem(
      {
        ...message("$thread-root:example.invalid", "Thread root"),
        thread_summary: {
          reply_count: 1,
          latest_event_id: "$older-thread-reply:example.invalid",
          latest_sender: "@bob:example.invalid",
          latest_sender_label: "Bob",
          latest_body_preview: "Older reply",
          latest_timestamp_ms: 1_800_000_001_000
        }
      },
      {
        activityEventId: "$older-thread-reply:example.invalid",
        displayTimestampMs: 1_800_000_001_000
      }
    );
    const newRootAtActivity = threadRootDisplayItem(
      {
        ...oldRootAtActivity,
        thread_summary: {
          ...oldRootAtActivity.thread_summary!,
          latest_event_id: "$newer-thread-reply:example.invalid",
          latest_body_preview: "Newer reply",
          latest_timestamp_ms: 1_800_000_003_000
        }
      },
      {
        activityEventId: "$newer-thread-reply:example.invalid",
        displayTimestampMs: 1_800_000_003_000
      }
    );
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const renderView = () => (
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );
    render(renderView());
    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperty(timeline, "clientHeight", { value: 200, configurable: true });
    Object.defineProperty(timeline, "scrollHeight", { value: 1_200, configurable: true });
    Object.defineProperty(timeline, "scrollTop", {
      value: 0,
      writable: true,
      configurable: true
    });

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [oldRootAtActivity, message("$between:example.invalid", "Between")]
          }
        }
      });
    });

    await screen.findByText("Thread root");
    timeline.scrollTop = 1_000;

    act(() => {
      // The relocated Rust display projection moves the root block to the
      // newer activity slot (stable row identity) while the live edge stays
      // pinned; the summary change itself rides the moved row's metadata.
      emit({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key: KEY,
            generation: 1,
            batch_id: 1,
            diffs: [
              { Remove: { index: 0 } },
              { Insert: { index: 1, item: newRootAtActivity } }
            ]
          }
        }
      });
    });

    await waitFor(() => {
      expect(timeline.scrollTop).toBe(1_000);
    });
  });


  it("falls back to the virtual height model when a projection anchor unmounts", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const onScrollDiagnosticsChange = vi.fn();
    const rowHeight = 72;
    const normalCount = 620;
    const rootAtActivity = threadRootDisplayItem(
      {
        ...message("$thread-root:example.invalid", "Thread root"),
        thread_summary: {
          reply_count: 1,
          latest_event_id: "$older-thread-reply:example.invalid",
          latest_sender: "@bob:example.invalid",
          latest_sender_label: "Bob",
          latest_body_preview: "Older reply",
          latest_timestamp_ms: 1_800_000_001_000
        }
      },
      {
        activityEventId: "$older-thread-reply:example.invalid",
        displayTimestampMs: 1_800_000_001_000
      }
    );
    const updatedRootAtNewerActivity = threadRootDisplayItem(
      {
        ...rootAtActivity,
        thread_summary: {
          ...rootAtActivity.thread_summary!,
          latest_event_id: "$newer-thread-reply:example.invalid",
          latest_body_preview: "Newer reply",
          latest_timestamp_ms: 1_800_000_003_000
        }
      },
      {
        activityEventId: "$newer-thread-reply:example.invalid",
        displayTimestampMs: 1_800_000_003_000
      }
    );
    const normals = Array.from({ length: normalCount }, (_, index) =>
      message(`$normal${index}:example.invalid`, `Normal ${index}`)
    );
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    let rootMovedToNewReply = false;
    const rectMock = vi
      .spyOn(HTMLElement.prototype, "getBoundingClientRect")
      .mockImplementation(function (this: HTMLElement) {
        const timeline = scrollContainerRef.current;
        if (this.getAttribute("data-testid") === "timeline-view") {
          return {
            x: 0,
            y: 0,
            top: 0,
            left: 0,
            right: 0,
            width: 0,
            height: 200,
            bottom: 200,
            toJSON: () => ({})
          } as DOMRect;
        }
        const row = this.matches(".timeline-item-frame")
          ? this
          : this.closest<HTMLElement>(".timeline-item-frame");
        const rowId =
          row?.dataset["frameItemId"] ??
          row?.querySelector<HTMLElement>("[data-item-id]")?.dataset["itemId"] ??
          "";
        let rowIndex = -1;
        if (rowId.startsWith("date-divider:")) {
          rowIndex = 0;
        } else if (rowId === "thread-root:$thread-root:example.invalid") {
          rowIndex = rootMovedToNewReply ? normalCount + 1 : 1;
        } else {
          const match = /^\$normal(\d+):example\.invalid$/.exec(rowId);
          if (match) {
            rowIndex = Number(match[1]) + (rootMovedToNewReply ? 1 : 2);
          }
        }
        const top =
          rowIndex >= 0 ? rowIndex * rowHeight - (timeline?.scrollTop ?? 0) : 0;
        return {
          x: 0,
          y: top,
          top,
          left: 0,
          right: 0,
          width: 0,
          height: rowIndex >= 0 ? rowHeight : 0,
          bottom: top + (rowIndex >= 0 ? rowHeight : 0),
          toJSON: () => ({})
        } as DOMRect;
      });
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const renderView = () => (
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        onScrollDiagnosticsChange={onScrollDiagnosticsChange}
      />
    );
    render(renderView());
    const timeline = screen.getByTestId("timeline-view");
    scrollContainerRef.current = timeline;
    Object.defineProperty(timeline, "clientHeight", { value: 200, configurable: true });
    Object.defineProperty(timeline, "scrollHeight", { value: 50_000, configurable: true });
    Object.defineProperty(timeline, "scrollTop", {
      value: 0,
      writable: true,
      configurable: true
    });

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [rootAtActivity, ...normals]
          }
        }
      });
    });

    expect(timeline.getAttribute("data-virtualized")).toBe("true");

    // The previous presentation puts Normal 300 after a date divider and the
    // root block. Its first-visible offset is +10px.
    timeline.scrollTop = 302 * rowHeight - 10;
    fireEvent.wheel(timeline, { deltaY: -1 });
    fireEvent.scroll(timeline);
    await act(async () => Promise.resolve());
    await waitFor(() => {
      expect(
        document.querySelector('[data-content-event-id="$normal300:example.invalid"]')
      ).not.toBeNull();
    });
    vi.useFakeTimers();
    const frames = new Map<number, FrameRequestCallback>();
    let nextFrameId = 0;
    let executedFrameCount = 0;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      nextFrameId += 1;
      frames.set(nextFrameId, callback);
      return nextFrameId;
    });
    vi.spyOn(window, "cancelAnimationFrame").mockImplementation((frameId) => {
      frames.delete(frameId);
    });
    act(() => {
      // The relocated Rust display projection moves the root row to the newest
      // activity slot (stable row identity) while the summary update rides the
      // moved row's metadata.
      emit({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key: KEY,
            generation: 1,
            batch_id: 1,
            diffs: [
              { Remove: { index: 0 } },
              { Insert: { index: normalCount + 1, item: updatedRootAtNewerActivity } }
            ]
          }
        }
      });
    });
    rootMovedToNewReply = true;
    const transactionFrameScheduled = frames.size > 0;

    // Model a virtual-window turnover between commit and the coalesced frame:
    // the stable anchor is no longer mounted, so DOM restoration must fail and
    // the height-model offset is the only valid correction path.
    document
      .querySelector('[data-content-event-id="$normal300:example.invalid"]')
      ?.closest(".timeline-item-frame")
      ?.remove();
    act(() => {
      const queued = [...frames.values()];
      frames.clear();
      for (const callback of queued) {
        executedFrameCount += 1;
        callback(0);
      }
    });

    expect({
      transactionFrameScheduled,
      executedFrameCount: executedFrameCount > 0,
      projectionWriteRecorded: onScrollDiagnosticsChange.mock.calls.some(
        ([diagnostics]) => diagnostics.scrollWrites.projectionCompensation > 0
      ),
      scrollTop: timeline.scrollTop
    }).toEqual({
      transactionFrameScheduled: true,
      executedFrameCount: true,
      projectionWriteRecorded: true,
      scrollTop: 301 * rowHeight - 10
    });
    rectMock.mockRestore();
  });


  it("does not reorder Thread timeline rows when latest placement is enabled", async () => {
    const threadKey = threadTimelineKey(
      "@alice:example.invalid",
      "!room:example.invalid",
      "$thread-root:example.invalid"
    );
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const root = {
      ...message("$thread-root:example.invalid", "Thread root"),
      thread_summary: {
        reply_count: 1,
        latest_event_id: "$latest-thread-reply:example.invalid",
        latest_sender: "@bob:example.invalid",
        latest_sender_label: null,
        latest_body_preview: "Latest reply",
        latest_timestamp_ms: 1_800_000_001_000
      }
    };
    const latestReply = {
      ...message("$latest-thread-reply:example.invalid", "Thread reply"),
      thread_root: "$thread-root:example.invalid"
    };
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });

    render(
      <TimelineView
        timelineKey={threadKey}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: threadKey,
            generation: 1,
            items: [root, latestReply]
          }
        }
      });
    });

    await screen.findByText("Thread reply");
    expect(
      Array.from(document.querySelectorAll("article[data-row-id]")).map((row) =>
        row.getAttribute("data-content-event-id")
      )
    ).toEqual(["$thread-root:example.invalid", "$latest-thread-reply:example.invalid"]);
  });


  it("shows notification count on the matching root row without moving timeline rows", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const onOpenThread = vi.fn();
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const root = {
      ...message("$thread-root:example.invalid", "Thread root"),
      thread_summary: {
        reply_count: 4,
        latest_event_id: "$latest-thread-reply:example.invalid",
        latest_sender: "@bob:example.invalid",
        latest_sender_label: "Bob",
        latest_body_preview: "latest reply",
        latest_timestamp_ms: 1_800_000_000_500
      }
    };

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        onOpenThread={onOpenThread}
        threadAttention={{
          rootEventId: "$thread-root:example.invalid",
          notificationCount: 3,
          highlightCount: 0,
          liveEventMarkerCount: 0
        }}
      />
    );

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [
              message("$before:example.invalid", "Before"),
              root,
              message("$after:example.invalid", "After")
            ]
          }
        }
      });
    });

    const notifications = await screen.findByRole("button", {
      name: /Thread notifications · 3/
    });
    expect(notifications.closest("[data-event-id]")?.getAttribute("data-event-id")).toBe(
      "$thread-root:example.invalid"
    );
    const eventOrder = Array.from(document.querySelectorAll("article[data-event-id]")).map(
      (row) => row.getAttribute("data-event-id")
    );
    expect(eventOrder).toEqual([
      "$before:example.invalid",
      "$thread-root:example.invalid",
      "$after:example.invalid"
    ]);

    fireEvent.click(notifications);
    expect(onOpenThread).toHaveBeenCalledWith(
      "!room:example.invalid",
      "$thread-root:example.invalid",
      "existingThread"
    );
  });


  it("does not show a thread notification badge when notification count is zero", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const root = {
      ...message("$quiet-thread-root:example.invalid", "Quiet thread root"),
      thread_summary: {
        reply_count: 4,
        latest_event_id: "$quiet-thread-reply:example.invalid",
        latest_sender: "@bob:example.invalid",
        latest_sender_label: "Bob",
        latest_body_preview: "latest reply",
        latest_timestamp_ms: 1_800_000_000_500
      }
    };
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        threadAttention={{
          rootEventId: "$quiet-thread-root:example.invalid",
          notificationCount: 0,
          highlightCount: 4,
          liveEventMarkerCount: 2
        }}
      />
    );

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [root]
          }
        }
      });
    });

    const row = await screen.findByText("Quiet thread root").then((node) =>
      node.closest<HTMLElement>("article")
    );
    expect(row).not.toBeNull();
    expect(
      within(row!).queryByRole("button", { name: /Thread notifications|View new replies/ })
    ).toBeNull();
    expect(within(row!).getByRole("button", { name: /Open thread/ })).toBeTruthy();
  });


  it("keeps notification badges with their root when roots reorder", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const rootAAtOrigin = threadRootDisplayItem(
      {
        ...message("$thread-root-a:example.invalid", "Thread root A"),
        timestamp_ms: 1_800_000_000_100,
        thread_summary: {
          reply_count: 1,
          latest_event_id: "$thread-reply-a:example.invalid",
          latest_sender: "@bob:example.invalid",
          latest_sender_label: "Bob",
          latest_body_preview: "Reply A",
          latest_timestamp_ms: 1_800_000_002_100
        }
      },
      {
        activityEventId: "$thread-root-a:example.invalid",
        displayTimestampMs: 1_800_000_000_100
      }
    );
    const rootAAtActivity = threadRootDisplayItem(rootAAtOrigin, {
      activityEventId: "$thread-reply-a:example.invalid",
      displayTimestampMs: 1_800_000_002_100
    });
    const rootBAtOrigin = threadRootDisplayItem(
      {
        ...message("$thread-root-b:example.invalid", "Thread root B"),
        timestamp_ms: 1_800_000_001_100,
        thread_summary: {
          reply_count: 1,
          latest_event_id: "$thread-reply-b:example.invalid",
          latest_sender: "@carol:example.invalid",
          latest_sender_label: "Carol",
          latest_body_preview: "Reply B",
          latest_timestamp_ms: 1_800_000_003_100
        }
      },
      {
        activityEventId: "$thread-root-b:example.invalid",
        displayTimestampMs: 1_800_000_001_100
      }
    );
    const rootBAtActivity = threadRootDisplayItem(rootBAtOrigin, {
      activityEventId: "$thread-reply-b:example.invalid",
      displayTimestampMs: 1_800_000_003_100
    });
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const view = () => (
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        threadAttention={{
          rootEventId: "$thread-root-a:example.invalid",
          notificationCount: 3,
          highlightCount: 0,
          liveEventMarkerCount: 0
        }}
      />
    );

    render(view());
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [rootAAtOrigin, rootBAtOrigin]
          }
        }
      });
    });

    const badge = await screen.findByRole("button", { name: /Thread notifications · 3/ });
    const rootRow = badge.closest<HTMLElement>("article");
    expect(rootRow?.getAttribute("data-event-id")).toBe("$thread-root-a:example.invalid");
    const rootOrder = Array.from(document.querySelectorAll("article[data-event-id]")).map((row) =>
      row.getAttribute("data-event-id")
    );

    act(() => {
      // The incoming Rust display reset reorders the roots; the renderer must
      // keep every row (and its badge) by stable row identity and render the
      // exact incoming order with its activity metadata.
      emit({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key: KEY,
            generation: 1,
            batch_id: 1,
            diffs: [{ Reset: { items: [rootBAtActivity, rootAAtActivity] } }]
          }
        }
      });
    });
    await waitFor(() => {
      const movedBadge = screen.getByRole("button", { name: /Thread notifications · 3/ });
      expect(movedBadge.closest<HTMLElement>("article")).toBe(rootRow);
      expect(
        movedBadge.closest<HTMLElement>("article")?.getAttribute("data-content-event-id")
      ).toBe("$thread-root-a:example.invalid");
      expect(
        Array.from(document.querySelectorAll("article[data-event-id]")).map((row) =>
          row.getAttribute("data-event-id")
        )
      ).toEqual(["$thread-reply-b:example.invalid", "$thread-reply-a:example.invalid"]);
      expect(
        Array.from(document.querySelectorAll("article[data-event-id]")).map((row) =>
          row.getAttribute("data-event-id")
        )
      ).not.toEqual(rootOrder);
    });
  });

});
