// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  focusedTimelineKey,
  roomTimelineKey,
  threadTimelineKey,
  type CoreEventPayload,
  type PaginationState,
  type TimelineGapId,
  type TimelineKey
} from "../domain/coreEvents";
import { setActiveLocaleProfile } from "../i18n/messages";
import { KEY, baseTransport, message, mockTimelineRects } from "./timelineViewTestSupport";
import { TimelineView, clearTimelineViewportSessionMemoryForTests } from "./TimelineView";

const TIMED_ROOM_ROW_HEIGHT = 72;

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  setActiveLocaleProfile("en", "none");
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

/** A room that opens at the live edge, with the container a real scroller. */
async function mountTimedRoom(rowCount: number, rowHeight = TIMED_ROOM_ROW_HEIGHT) {
  const onScrollDiagnosticsChange = vi.fn();
  let listener: ((payload: CoreEventPayload) => void) | null = null;
  const transport = baseTransport({
    listenCoreEvents(nextListener) {
      listener = nextListener;
      return () => undefined;
    }
  });
  const rects: Record<string, { top: number; height: number }> = {};
  for (let index = 0; index < rowCount; index += 1) {
    rects[`$row${index}`] = { top: index * rowHeight, height: rowHeight };
  }
  const scrollContainerRef: { current: HTMLElement | null } = { current: null };
  mockTimelineRects(rects, { top: 0, height: 600 }, scrollContainerRef);
  render(
    <TimelineView
      timelineKey={KEY}
      roomId="!room:example.invalid"
      transport={transport}
      onReply={vi.fn()}
      onScrollDiagnosticsChange={onScrollDiagnosticsChange}
      listRefCallback={(element) => {
        scrollContainerRef.current =
          element?.closest<HTMLElement>("[data-testid=timeline-view]") ?? null;
      }}
    />
  );
  // `getBy` rather than `findBy`: fake timers are installed by the caller.
  const timeline = screen.getByTestId("timeline-view");
  Object.defineProperties(timeline, {
    scrollHeight: { value: rowCount * rowHeight, writable: true, configurable: true },
    clientHeight: { value: 600, configurable: true },
    scrollTop: { value: 0, writable: true, configurable: true }
  });
  act(() => {
    listener?.({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: Array.from({ length: rowCount }, (_, index) =>
            message(`$row${index}`, `row ${index}`)
          )
        }
      }
    });
  });
  return {
    timeline,
    onScrollDiagnosticsChange,
    /** A reader's own scroll, one frame in, so the scroll counts as active. */
    async scrollTo(scrollTop: number) {
      act(() => {
        fireEvent.wheel(timeline, { deltaY: -40 });
      });
      timeline.scrollTop = scrollTop;
      act(() => {
        fireEvent.scroll(timeline);
      });
      await act(async () => {
        vi.advanceTimersByTime(40);
        await Promise.resolve();
      });
    },
    setRowHeight(index: number, height: number) {
      rects[`$row${index}`] = { top: index * rowHeight, height };
    }
  };
}

function forwardPagination(key: TimelineKey, state: PaginationState): CoreEventPayload {
  return {
    kind: "Timeline",
    event: {
      PaginationStateChanged: {
        request_id: null,
        key,
        direction: "Forward",
        state,
        prepend_expected: null
      }
    }
  };
}

/** #1676: Core's live-tail report for the current (default) actor. */
function focusedLiveTail(key: TimelineKey, tailEventId: string): CoreEventPayload {
  return {
    kind: "Timeline",
    event: {
      FocusedLiveTailReached: {
        key,
        actor_generation: 0,
        generation: 1,
        tail_event_id: tailEventId
      }
    }
  };
}

function installResizeObserverMock() {
  const originalResizeObserver = window.ResizeObserver;
  const observers: Array<{ trigger: () => void }> = [];

  class MockResizeObserver {
    private readonly callback: ResizeObserverCallback;

    constructor(callback: ResizeObserverCallback) {
      this.callback = callback;
      observers.push({
        trigger: () => {
          this.callback([] as ResizeObserverEntry[], this as unknown as ResizeObserver);
        }
      });
    }

    observe = vi.fn();
    unobserve = vi.fn();
    disconnect = vi.fn();
  }

  Object.defineProperty(window, "ResizeObserver", {
    configurable: true,
    writable: true,
    value: MockResizeObserver
  });

  return {
    triggerAll() {
      for (const observer of observers) {
        observer.trigger();
      }
    },
    restore() {
      Object.defineProperty(window, "ResizeObserver", {
        configurable: true,
        writable: true,
        value: originalResizeObserver
      });
    }
  };
}

describe("TimelineView", () => {
  it.each(["read", "unread"] as const)("places the %s divider on a moved thread root's content identity", async (state) => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(listener) {
        emit = listener;
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
    const boundary = state === "read" ? "$root:example.invalid" : "$before:example.invalid";
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [{
              ...message("$root:example.invalid", "Moved thread root"),
              display_metadata: {
                row_id: "thread-root:$root:example.invalid",
                kind: { kind: "threadRoot" },
                content_event_id: "$root:example.invalid",
                activity_event_id: "$reply:example.invalid",
                display_timestamp_ms: null
              }
            }]
          }
        }
      });
      emit({
        kind: "Timeline",
        event: {
          NavigationUpdated: {
            key: KEY,
            snapshot: {
              read_marker_event_id: boundary,
              read_marker_display_event_id: boundary,
              first_unread_event_id: state === "unread" ? "$root:example.invalid" : null,
              first_unread_display_event_id: state === "unread" ? "$reply:example.invalid" : null,
              unread_event_count: state === "unread" ? 1 : 0,
              unread_position: state === "unread" ? "insideViewport" : "none",
              newer_event_count: 0,
              can_jump_to_bottom: false,
              local_viewed_event_id: boundary,
              server_confirmed_read_event_id: boundary,
              read_state_sync: "synced"
            }
          }
        }
      });
    });
    const marker = await screen.findByRole("separator", {
      name: state === "read" ? "Read up to here" : "Unread messages"
    });
    const row = state === "read" ? marker.previousElementSibling : marker.nextElementSibling;
    expect(row?.getAttribute("data-content-event-id")).toBe("$root:example.invalid");
    expect(row?.getAttribute("data-activity-event-id")).toBe("$reply:example.invalid");
  });

  it("reports the visible latest thread root as the live edge using its activity identity", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const observeViewport = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(listener) {
        emit = listener;
        return () => undefined;
      },
      observeViewport
    });
    const root = {
      ...message("$root:example.invalid", "Thread root"),
      display_metadata: {
        row_id: "thread-root:$root:example.invalid",
        kind: { kind: "threadRoot" as const },
        content_event_id: "$root:example.invalid",
        activity_event_id: "$reply:example.invalid",
        display_timestamp_ms: null
      }
    };
    const rects = { "$reply:example.invalid": { top: 900, height: 100 } };
    const rectSpy = mockTimelineRects(
      rects,
      { top: 0, height: 500 }
    );
    try {
      render(
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={transport}
          onReply={vi.fn()}
        />
      );
      const timeline = await screen.findByTestId("timeline-view");
      Object.defineProperty(timeline, "clientHeight", { value: 500, configurable: true });
      Object.defineProperty(timeline, "scrollHeight", { value: 1000, configurable: true });
      Object.defineProperty(timeline, "scrollTop", { value: 400, writable: true, configurable: true });
      act(() => {
        emit({
          kind: "Timeline",
          event: {
            InitialItems: { request_id: null, key: KEY, generation: 1, items: [root] }
          }
        });
      });
      await screen.findByText("Thread root");
      // All material content is visible even while trailing layout space is
      // outside the viewport. Rust must receive the same bottom fact as for an
      // ordinary message, then resolve the root without reading its hidden reply.
      timeline.scrollTop = 400;
      rects["$reply:example.invalid"].top = 300;
      observeViewport.mockClear();
      fireEvent.scroll(timeline);
      await waitFor(() => {
        expect(observeViewport).toHaveBeenCalledWith(
          "!room:example.invalid",
          "$reply:example.invalid",
          "$reply:example.invalid",
          [],
          true,
          expect.any(String),
          null
        );
      });
    } finally {
      rectSpy.mockRestore();
    }
  });

  it("automatically returns an anchored timeline once its focused bottom matches the live edge", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const onReturnToLive = vi.fn();
    const observeViewport = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      observeViewport
    });
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    const rectSpy = mockTimelineRects(
      { "$live:example.invalid": { top: 520, height: 80 } },
      { top: 0, height: 500 },
      scrollContainerRef
    );

    try {
      render(
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={transport}
          onReply={vi.fn()}
          initialTargetEventId="$anchor:example.invalid"
          isAnchored
          onReturnToLive={onReturnToLive}
          liveLatestEventId="$live:example.invalid"
        />
      );

      const timeline = await screen.findByTestId("timeline-view");
      scrollContainerRef.current = timeline;
      Object.defineProperty(timeline, "clientHeight", { value: 500, configurable: true });
      Object.defineProperty(timeline, "scrollHeight", { value: 1_000, configurable: true });
      Object.defineProperty(timeline, "scrollTop", {
        value: 500,
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
              items: [message("$live:example.invalid", "Live message")]
            }
          }
        });
      });

      fireEvent.wheel(timeline, { deltaY: 1 });
      fireEvent.scroll(timeline);

      await waitFor(() => {
        expect(onReturnToLive).toHaveBeenCalledTimes(1);
      });
      fireEvent.scroll(timeline);
      expect(onReturnToLive).toHaveBeenCalledTimes(1);
      expect(screen.getByRole("button", { name: /jump to latest message/i })).toBeTruthy();
    } finally {
      rectSpy.mockRestore();
    }
  });


  it("automatically returns a focused anchored timeline at the live edge", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const onReturnToLive = vi.fn();
    const observeViewport = vi.fn(async () => undefined);
    const paginateForwards = vi.fn(async (_key: TimelineKey) => undefined);
    const focusedKey = focusedTimelineKey(
      "@alice:example.invalid",
      "!room:example.invalid",
      "$anchor:example.invalid"
    );
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      observeViewport,
      paginateForwards
    });
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    const rectSpy = mockTimelineRects(
      { "$live:example.invalid": { top: 520, height: 80 } },
      { top: 0, height: 500 },
      scrollContainerRef
    );

    try {
      render(
        <TimelineView
          timelineKey={focusedKey}
          roomId="!room:example.invalid"
          transport={transport}
          onReply={vi.fn()}
          initialTargetEventId="$anchor:example.invalid"
          isAnchored
          onReturnToLive={onReturnToLive}
          liveLatestEventId="$live:example.invalid"
        />
      );

      const timeline = await screen.findByTestId("timeline-view");
      scrollContainerRef.current = timeline;
      Object.defineProperty(timeline, "clientHeight", { value: 500, configurable: true });
      Object.defineProperty(timeline, "scrollHeight", { value: 1_000, configurable: true });
      Object.defineProperty(timeline, "scrollTop", {
        value: 500,
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
              items: [message("$live:example.invalid", "Live message")]
            }
          }
        });
      });

      fireEvent.wheel(timeline, { deltaY: 1 });
      fireEvent.scroll(timeline);

      // #1668: the focused bottom is the live end only once Rust reports it.
      await waitFor(() => {
        expect(paginateForwards).toHaveBeenCalledWith(focusedKey);
      });
      expect(onReturnToLive).not.toHaveBeenCalled();
      act(() => {
        emit(forwardPagination(focusedKey, "EndReached"));
      });
      fireEvent.scroll(timeline);
      // #1676: the end alone is not the proof; Core reports the live tail.
      expect(onReturnToLive).not.toHaveBeenCalled();
      act(() => {
        emit(focusedLiveTail(focusedKey, "$live:example.invalid"));
      });

      await waitFor(() => {
        expect(onReturnToLive).toHaveBeenCalledTimes(1);
      });
      expect(observeViewport).not.toHaveBeenCalled();
    } finally {
      rectSpy.mockRestore();
    }
  });

  it("loads newer focused history at the anchored bottom without moving the reader (#1668)", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const onReturnToLive = vi.fn();
    const paginateForwards = vi.fn(async (_key: TimelineKey) => undefined);
    const focusedKey = focusedTimelineKey(
      "@alice:example.invalid",
      "!room:example.invalid",
      "$anchor:example.invalid"
    );
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      paginateForwards
    });
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    const rectSpy = mockTimelineRects(
      {
        "$context:example.invalid": { top: 800, height: 80 },
        "$tail:example.invalid": { top: 920, height: 80 }
      },
      { top: 0, height: 500 },
      scrollContainerRef
    );

    try {
      render(
        <TimelineView
          timelineKey={focusedKey}
          roomId="!room:example.invalid"
          transport={transport}
          onReply={vi.fn()}
          initialTargetEventId="$anchor:example.invalid"
          isAnchored
          onReturnToLive={onReturnToLive}
          liveLatestEventId="$live:example.invalid"
        />
      );

      const timeline = await screen.findByTestId("timeline-view");
      scrollContainerRef.current = timeline;
      Object.defineProperty(timeline, "clientHeight", { value: 500, configurable: true });
      Object.defineProperty(timeline, "scrollHeight", { value: 1_000, configurable: true });
      Object.defineProperty(timeline, "scrollTop", {
        value: 500,
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
                message("$context:example.invalid", "Context message"),
                message("$tail:example.invalid", "End of the context page")
              ]
            }
          }
        });
      });
      fireEvent.wheel(timeline, { deltaY: 1 });
      fireEvent.scroll(timeline);

      await waitFor(() => {
        expect(paginateForwards).toHaveBeenCalledWith(focusedKey);
      });
      expect(paginateForwards).toHaveBeenCalledTimes(1);

      act(() => {
        emit(forwardPagination(focusedKey, "Paginating"));
        emit({
          kind: "Timeline",
          event: {
            ItemsUpdated: {
              key: focusedKey,
              generation: 1,
              batch_id: 1,
              diffs: [{ PushBack: { item: message("$newer:example.invalid", "Newer message") } }]
            }
          }
        });
      });
      await screen.findByText("Newer message");
      // The appended page lands below the reader; the anchored history is not
      // the live edge, so nothing may pin the viewport to the new bottom.
      expect(timeline.scrollTop).toBe(500);
      expect(onReturnToLive).not.toHaveBeenCalled();

      // One page in flight at a time; the Idle terminal releases the next.
      fireEvent.scroll(timeline);
      expect(paginateForwards).toHaveBeenCalledTimes(1);
      act(() => {
        emit(forwardPagination(focusedKey, "Idle"));
      });
      fireEvent.wheel(timeline, { deltaY: 1 });
      fireEvent.scroll(timeline);
      await waitFor(() => {
        expect(paginateForwards).toHaveBeenCalledTimes(2);
      });
    } finally {
      rectSpy.mockRestore();
    }
  });

  it("returns an anchored focused timeline to live only after its final page is committed (#1668)", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const onReturnToLive = vi.fn();
    const paginateForwards = vi.fn(async (_key: TimelineKey) => undefined);
    const focusedKey = focusedTimelineKey(
      "@alice:example.invalid",
      "!room:example.invalid",
      "$anchor:example.invalid"
    );
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      paginateForwards
    });
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    const rectSpy = mockTimelineRects(
      {
        "$tail:example.invalid": { top: 920, height: 80 },
        "$live:example.invalid": { top: 920, height: 80 }
      },
      { top: 0, height: 500 },
      scrollContainerRef
    );

    try {
      render(
        <TimelineView
          timelineKey={focusedKey}
          roomId="!room:example.invalid"
          transport={transport}
          onReply={vi.fn()}
          initialTargetEventId="$anchor:example.invalid"
          isAnchored
          onReturnToLive={onReturnToLive}
          liveLatestEventId="$live:example.invalid"
        />
      );

      const timeline = await screen.findByTestId("timeline-view");
      scrollContainerRef.current = timeline;
      Object.defineProperty(timeline, "clientHeight", { value: 500, configurable: true });
      Object.defineProperty(timeline, "scrollHeight", { value: 1_000, configurable: true });
      Object.defineProperty(timeline, "scrollTop", {
        value: 500,
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
              items: [message("$tail:example.invalid", "End of the context page")]
            }
          }
        });
      });
      fireEvent.wheel(timeline, { deltaY: 1 });
      fireEvent.scroll(timeline);
      await waitFor(() => {
        expect(paginateForwards).toHaveBeenCalledWith(focusedKey);
      });

      // The SDK can report the end before the final page reaches the renderer.
      act(() => {
        emit(forwardPagination(focusedKey, "EndReached"));
      });
      fireEvent.wheel(timeline, { deltaY: 1 });
      fireEvent.scroll(timeline);
      expect(onReturnToLive).not.toHaveBeenCalled();

      act(() => {
        emit({
          kind: "Timeline",
          event: {
            ItemsUpdated: {
              key: focusedKey,
              generation: 1,
              batch_id: 1,
              diffs: [{ PushBack: { item: message("$live:example.invalid", "Live message") } }]
            }
          }
        });
        // #1676: Core reports the live tail right after the committing batch.
        emit(focusedLiveTail(focusedKey, "$live:example.invalid"));
      });
      // The reader rests at the bottom: committing the final page alone
      // completes the return, without another scroll.
      await waitFor(() => {
        expect(onReturnToLive).toHaveBeenCalledTimes(1);
      });
    } finally {
      rectSpy.mockRestore();
    }
  });

  it("asks for a declined forward page again once gap repair releases the timeline (#1668)", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const paginateForwards = vi.fn(async (_key: TimelineKey) => undefined);
    const focusedKey = focusedTimelineKey(
      "@alice:example.invalid",
      "!room:example.invalid",
      "$anchor:example.invalid"
    );
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      paginateForwards
    });
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    const rectSpy = mockTimelineRects(
      { "$tail:example.invalid": { top: 920, height: 80 } },
      { top: 0, height: 500 },
      scrollContainerRef
    );

    try {
      render(
        <TimelineView
          timelineKey={focusedKey}
          roomId="!room:example.invalid"
          transport={transport}
          onReply={vi.fn()}
        />
      );
      const timeline = await screen.findByTestId("timeline-view");
      scrollContainerRef.current = timeline;
      Object.defineProperty(timeline, "clientHeight", { value: 500, configurable: true });
      Object.defineProperty(timeline, "scrollHeight", { value: 1_000, configurable: true });
      Object.defineProperty(timeline, "scrollTop", {
        value: 500,
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
              items: [message("$tail:example.invalid", "End of the context page")]
            }
          }
        });
      });
      fireEvent.wheel(timeline, { deltaY: 1 });
      fireEvent.scroll(timeline);
      await waitFor(() => {
        expect(paginateForwards).toHaveBeenCalledTimes(1);
      });

      // Core declined the page: an Idle terminal without Paginating.
      act(() => {
        emit(forwardPagination(focusedKey, "Idle"));
      });
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 50));
      });
      expect(paginateForwards).toHaveBeenCalledTimes(1);

      act(() => {
        emit({
          kind: "Timeline",
          event: {
            GapRepairReleased: { key: focusedKey, actor_generation: 1, generation: 1 }
          }
        });
      });
      await waitFor(() => {
        expect(paginateForwards).toHaveBeenCalledTimes(2);
      });
    } finally {
      rectSpy.mockRestore();
    }
  });

  it("does not re-request live mode after a transient loss of bottom proof", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const onReturnToLive = vi.fn(() => new Promise<void>(() => undefined));
    const observeViewport = vi.fn(
      async (
        _roomId: string,
        _firstVisibleEventId: string | null,
        _lastVisibleEventId: string | null,
        _visibleGapIds: TimelineGapId[],
        _atBottom: boolean
      ) => undefined
    );
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      observeViewport
    });
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    const rectSpy = mockTimelineRects(
      {
        "$older:example.invalid": { top: 100, height: 80 },
        "$live:example.invalid": { top: 900, height: 80 }
      },
      { top: 0, height: 500 },
      scrollContainerRef
    );

    try {
      const props = {
        timelineKey: KEY,
        roomId: "!room:example.invalid",
        transport,
        onReply: vi.fn(),
        initialTargetEventId: "$anchor:example.invalid",
        isAnchored: true,
        onReturnToLive,
        liveLatestEventId: null
      };
      const { rerender } = render(<TimelineView {...props} />);
      const timeline = await screen.findByTestId("timeline-view");
      scrollContainerRef.current = timeline;
      Object.defineProperty(timeline, "clientHeight", { value: 500, configurable: true });
      Object.defineProperty(timeline, "scrollHeight", { value: 1_000, configurable: true });
      Object.defineProperty(timeline, "scrollTop", {
        value: 500,
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
              items: [
                message("$older:example.invalid", "Older message"),
                message("$live:example.invalid", "Live message")
              ]
            }
          }
        });
      });

      timeline.scrollTop = 500;
      fireEvent.wheel(timeline, { deltaY: 1 });
      fireEvent.scroll(timeline);
      rerender(<TimelineView {...props} liveLatestEventId="$live:example.invalid" />);
      await waitFor(() => {
        expect(onReturnToLive).toHaveBeenCalledTimes(1);
      });

      timeline.scrollTop = 0;
      fireEvent.wheel(timeline, { deltaY: -1 });
      fireEvent.scroll(timeline);
      await waitFor(() => {
        expect(observeViewport.mock.calls.some((call) => call[4] === false)).toBe(true);
      });

      timeline.scrollTop = 500;
      fireEvent.wheel(timeline, { deltaY: 1 });
      fireEvent.scroll(timeline);
      await waitFor(() => {
        expect(observeViewport.mock.calls.some((call) => call[4] === true)).toBe(true);
      });
      expect(onReturnToLive).toHaveBeenCalledTimes(1);
    } finally {
      rectSpy.mockRestore();
    }
  });


  it("retries automatic live return after a rejected close operation", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const onReturnToLive = vi
      .fn<() => Promise<void>>()
      .mockRejectedValueOnce(new Error("close failed"))
      .mockResolvedValue(undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    const rectSpy = mockTimelineRects(
      { "$live:example.invalid": { top: 520, height: 80 } },
      { top: 0, height: 500 },
      scrollContainerRef
    );

    try {
      const props = {
        timelineKey: KEY,
        roomId: "!room:example.invalid",
        transport,
        onReply: vi.fn(),
        initialTargetEventId: "$anchor:example.invalid",
        isAnchored: true,
        onReturnToLive,
        liveLatestEventId: "$live:example.invalid"
      };
      const { rerender } = render(<TimelineView {...props} />);
      const timeline = await screen.findByTestId("timeline-view");
      scrollContainerRef.current = timeline;
      Object.defineProperty(timeline, "clientHeight", { value: 500, configurable: true });
      Object.defineProperty(timeline, "scrollHeight", { value: 1_000, configurable: true });
      Object.defineProperty(timeline, "scrollTop", {
        value: 500,
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
              items: [message("$live:example.invalid", "Live message")]
            }
          }
        });
      });
      fireEvent.wheel(timeline, { deltaY: 1 });
      fireEvent.scroll(timeline);
      await waitFor(() => expect(onReturnToLive).toHaveBeenCalledTimes(1));

      rerender(<TimelineView {...props} liveLatestEventId="$other:example.invalid" />);
      rerender(<TimelineView {...props} liveLatestEventId="$live:example.invalid" />);
      await waitFor(() => expect(onReturnToLive).toHaveBeenCalledTimes(2));
    } finally {
      rectSpy.mockRestore();
    }
  });


  it("keeps the explicit return control when the focused latest readable ID is unknown", async () => {
    const onReturnToLive = vi.fn();
    const transport = baseTransport({
      observeViewport: vi.fn(async () => undefined)
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        initialTargetEventId="$anchor:example.invalid"
        isAnchored
        onReturnToLive={onReturnToLive}
        liveLatestEventId="$live:example.invalid"
      />
    );

    const timeline = await screen.findByTestId("timeline-view");
    Object.defineProperty(timeline, "clientHeight", { value: 500, configurable: true });
    Object.defineProperty(timeline, "scrollHeight", { value: 1_000, configurable: true });
    Object.defineProperty(timeline, "scrollTop", {
      value: 500,
      writable: true,
      configurable: true
    });
    fireEvent.wheel(timeline, { deltaY: 1 });
    fireEvent.scroll(timeline);
    await act(async () => {
      await Promise.resolve();
    });

    expect(onReturnToLive).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: /jump to latest message/i })).toBeTruthy();
  });


  it.each([
    ["different event IDs", "$other:example.invalid", 500],
    ["unknown authoritative event ID", null, 500],
    ["viewport above the bottom", "$live:example.invalid", 0]
  ])(
    "keeps the explicit return control when the live-edge proof is missing: %s",
    async (_label, liveLatestEventId, scrollTop) => {
      let emit: (payload: CoreEventPayload) => void = () => undefined;
      const onReturnToLive = vi.fn();
      const observeViewport = vi.fn(async () => undefined);
      const transport = baseTransport({
        listenCoreEvents(nextListener) {
          emit = nextListener;
          return () => undefined;
        },
        observeViewport
      });
      const scrollContainerRef: { current: HTMLElement | null } = { current: null };
      const rectSpy = mockTimelineRects(
        {
          "$older:example.invalid": { top: 100, height: 80 },
          "$live:example.invalid": { top: 900, height: 80 }
        },
        { top: 0, height: 500 },
        scrollContainerRef
      );

      try {
        const props = {
          timelineKey: KEY,
          roomId: "!room:example.invalid",
          transport,
          onReply: vi.fn(),
          initialTargetEventId: "$anchor:example.invalid",
          isAnchored: true,
          onReturnToLive,
          liveLatestEventId: null
        };
        const { rerender } = render(
          <TimelineView
            {...props}
          />
        );

        const timeline = await screen.findByTestId("timeline-view");
        scrollContainerRef.current = timeline;
        Object.defineProperty(timeline, "clientHeight", { value: 500, configurable: true });
        Object.defineProperty(timeline, "scrollHeight", { value: 1_000, configurable: true });
        Object.defineProperty(timeline, "scrollTop", {
          value: scrollTop,
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
                items: [
                  message("$older:example.invalid", "Older message"),
                  message("$live:example.invalid", "Live message")
                ]
              }
            }
          }
        );
        });

        timeline.scrollTop = scrollTop;
        fireEvent.wheel(timeline, { deltaY: scrollTop > 0 ? 1 : -1 });
        fireEvent.scroll(timeline);
        await act(async () => {
          await Promise.resolve();
        });
        rerender(<TimelineView {...props} liveLatestEventId={liveLatestEventId} />);
        await act(async () => {
          await Promise.resolve();
        });

        expect(onReturnToLive).not.toHaveBeenCalled();
        expect(screen.getByRole("button", { name: /jump to latest message/i })).toBeTruthy();
      } finally {
        rectSpy.mockRestore();
      }
    }
  );


  it("captures the bottom-most visible event as the persisted room scroll anchor", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const updateScrollAnchor = vi.fn(async () => undefined);
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      callback(0);
      return 0;
    });
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      updateScrollAnchor
    });

    mockTimelineRects({
      "$first:example.invalid": { top: 120, height: 48 },
      "$second:example.invalid": { top: 420, height: 48 }
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
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [
              message("$first:example.invalid", "First"),
              message("$second:example.invalid", "Second")
            ]
          }
        }
      });
    });

    const timeline = screen.getByTestId("timeline-view");

    act(() => {
      fireEvent.scroll(timeline);
    });

    // #1892: persisted once the scroll settles, not from the scroll event.
    await waitFor(() => {
      expect(updateScrollAnchor).toHaveBeenCalledTimes(1);
    });
    expect(updateScrollAnchor).toHaveBeenCalledWith(
      "!room:example.invalid",
      expect.objectContaining({
        event_id: "$second:example.invalid",
        edge: "bottom",
        offset_px: -132,
        updated_at_ms: expect.any(Number)
      })
    );

    act(() => {
      fireEvent.scroll(timeline);
    });
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 400));
    });

    expect(updateScrollAnchor).toHaveBeenCalledTimes(1);
  });

  async function renderTwoRowTimelineForAnchor(updateScrollAnchor: () => Promise<void>) {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      callback(0);
      return 0;
    });
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      updateScrollAnchor
    });
    // Rows follow scrollTop, so each scroll step observes a new anchor. The
    // rows are 2,400 px tall together because the mounted window is derived
    // from those measured heights: a fixture that places a 48 px row at
    // offset 1,900 would model 96 px of content in a 2,400 px scroller.
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    mockTimelineRects(
      {
        "$first:example.invalid": { top: 0, height: 2350 },
        "$second:example.invalid": { top: 2350, height: 50 }
      },
      { top: 0, height: 600 },
      scrollContainerRef
    );
    const rendered = render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );
    const timeline = await screen.findByTestId("timeline-view");
    scrollContainerRef.current = timeline;
    Object.defineProperty(timeline, "scrollHeight", { value: 2400, configurable: true });
    Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
    Object.defineProperty(timeline, "scrollTop", { value: 0, writable: true, configurable: true });
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [
              message("$first:example.invalid", "First"),
              message("$second:example.invalid", "Second")
            ]
          }
        }
      });
    });
    await waitFor(() => {
      expect(timeline.scrollTop).toBe(1800);
    });
    // Arriving at the live edge persists once, immediately; only scrolling is
    // under test from here on.
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 400));
    });
    vi.mocked(updateScrollAnchor).mockClear();
    return { ...rendered, timeline };
  }

  function userScroll(timeline: HTMLElement, steps: number) {
    for (let step = 0; step < steps; step += 1) {
      act(() => {
        fireEvent.wheel(timeline, { deltaY: -30 });
        timeline.scrollTop -= 30;
        fireEvent.scroll(timeline);
      });
    }
  }

  it("sends no scroll anchor to the core while the user is still scrolling (#1892)", async () => {
    // Each anchor update is a core command whose snapshot re-renders the
    // shell. Sending one per scroll frame held scrolling under 20 fps.
    const updateScrollAnchor = vi.fn(async () => undefined);
    const { timeline } = await renderTwoRowTimelineForAnchor(updateScrollAnchor);

    userScroll(timeline, 5);

    expect(updateScrollAnchor).not.toHaveBeenCalled();
    await waitFor(() => {
      expect(updateScrollAnchor).toHaveBeenCalledTimes(1);
    });
    // The one update carries where the scroll ended (150px up), not where it
    // began: the tail row has left the viewport and the first row's bottom now
    // sits 100 px below it.
    expect(updateScrollAnchor).toHaveBeenCalledWith(
      "!room:example.invalid",
      expect.objectContaining({ event_id: "$first:example.invalid", edge: "bottom", offset_px: 100 })
    );
  });

  it("still persists a pending scroll anchor when the timeline unmounts mid-scroll (#1892)", async () => {
    const updateScrollAnchor = vi.fn(async () => undefined);
    const { timeline, unmount } = await renderTwoRowTimelineForAnchor(updateScrollAnchor);

    userScroll(timeline, 2);
    expect(updateScrollAnchor).not.toHaveBeenCalled();

    unmount();

    expect(updateScrollAnchor).toHaveBeenCalledTimes(1);
    expect(updateScrollAnchor).toHaveBeenCalledWith(
      "!room:example.invalid",
      expect.objectContaining({ event_id: "$first:example.invalid", edge: "bottom" })
    );
  });

  it("persists a pending scroll anchor for the room it was taken in when the room changes (#1892)", async () => {
    const updateScrollAnchor = vi.fn(async () => undefined);
    const { timeline, rerender } = await renderTwoRowTimelineForAnchor(updateScrollAnchor);

    userScroll(timeline, 2);
    expect(updateScrollAnchor).not.toHaveBeenCalled();

    rerender(
      <TimelineView
        timelineKey={roomTimelineKey("@alice:example.invalid", "!other:example.invalid")}
        roomId="!other:example.invalid"
        transport={baseTransport({ updateScrollAnchor })}
        onReply={vi.fn()}
      />
    );

    expect(updateScrollAnchor).toHaveBeenCalledTimes(1);
    expect(updateScrollAnchor).toHaveBeenCalledWith(
      "!room:example.invalid",
      expect.objectContaining({ event_id: "$first:example.invalid" })
    );
  });


  it("persists the sent message as the room anchor after a programmatic live-edge scroll", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const updateScrollAnchor = vi.fn(async () => undefined);
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      callback(0);
      return 0;
    });
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      updateScrollAnchor
    });
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };

    mockTimelineRects(
      {
        "$older:example.invalid": { top: 2100, height: 80 },
        "$sent:example.invalid": { top: 2320, height: 60 }
      },
      { top: 0, height: 600 },
      scrollContainerRef
    );

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        currentUserId="@alice:example.invalid"
        onReply={vi.fn()}
      />
    );

    const timeline = await screen.findByTestId("timeline-view");
    scrollContainerRef.current = timeline;
    Object.defineProperty(timeline, "scrollHeight", { value: 2400, configurable: true });
    Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
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
            items: [message("$older:example.invalid", "Older message")]
          }
        }
      });
    });

    await waitFor(() => {
      expect(timeline.scrollTop).toBe(1800);
    });

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
                PushBack: {
                  item: {
                    ...message("$sent:example.invalid", "Message I just sent"),
                    sender: "@alice:example.invalid",
                    send_state: { kind: "sending" }
                  }
                }
              }
            ]
          }
        }
      });
    });

    await waitFor(() => {
      expect(screen.getByText("Message I just sent")).toBeTruthy();
      expect(updateScrollAnchor).toHaveBeenLastCalledWith(
        "!room:example.invalid",
        expect.objectContaining({
          event_id: "$sent:example.invalid",
          edge: "bottom",
          offset_px: -20,
          updated_at_ms: expect.any(Number)
        })
      );
    });
  });


  it("restores an in-session bottom-edge room anchor when the event is already rendered", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const updateScrollAnchor = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      updateScrollAnchor
    });
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };

    mockTimelineRects(
      {
        "$anchor:example.invalid": { top: 500, height: 48 },
        "$after:example.invalid": { top: 560, height: 48 }
      },
      { top: 0, height: 600 },
      scrollContainerRef
    );

    const { unmount } = render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    const timeline = await screen.findByTestId("timeline-view");
    scrollContainerRef.current = timeline;
    Object.defineProperty(timeline, "scrollHeight", { value: 2000, configurable: true });
    Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
    Object.defineProperty(timeline, "scrollTop", {
      value: 0,
      writable: true,
      configurable: true
    });

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [
            message("$first:example.invalid", "First"),
            message("$anchor:example.invalid", "Anchor"),
            message("$after:example.invalid", "After")
          ]
        }
      }
    });

    await waitFor(() => {
      expect(timeline.scrollTop).toBe(1400);
    });
    await act(async () => {
      await new Promise<void>((resolve) => {
        requestAnimationFrame(() => resolve());
      });
    });
    timeline.scrollTop = 48;
    fireEvent.wheel(timeline, { deltaY: -120 });
    fireEvent.scroll(timeline);

    await waitFor(() => {
      expect(updateScrollAnchor).toHaveBeenLastCalledWith(
        "!room:example.invalid",
        expect.objectContaining({
          event_id: "$after:example.invalid",
          edge: "bottom",
          offset_px: -40,
          updated_at_ms: expect.any(Number)
        })
      );
    });

    unmount();
    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );
    const restoredTimeline = await screen.findByTestId("timeline-view");
    scrollContainerRef.current = restoredTimeline;
    Object.defineProperty(restoredTimeline, "scrollHeight", { value: 2000, configurable: true });
    Object.defineProperty(restoredTimeline, "clientHeight", { value: 600, configurable: true });
    Object.defineProperty(restoredTimeline, "scrollTop", {
      value: 0,
      writable: true,
      configurable: true
    });

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [
            message("$first:example.invalid", "First"),
            message("$anchor:example.invalid", "Anchor"),
            message("$after:example.invalid", "After")
          ]
        }
      }
    });

    await waitFor(() => {
      expect(restoredTimeline.scrollTop).toBe(48);
    });
  });


  it("falls back to live edge and clears session anchor when the in-session anchor is missing", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const updateScrollAnchor = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      updateScrollAnchor
    });
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    mockTimelineRects(
      {
        "$anchor:example.invalid": { top: 500, height: 48 },
        "$after:example.invalid": { top: 560, height: 48 },
        "$live-top:example.invalid": { top: 1300, height: 48 },
        "$live-bottom:example.invalid": { top: 1900, height: 48 }
      },
      { top: 0, height: 600 },
      scrollContainerRef
    );

    const { unmount } = render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    const timeline = await screen.findByTestId("timeline-view");
    scrollContainerRef.current = timeline;
    Object.defineProperty(timeline, "scrollHeight", { value: 2000, configurable: true });
    Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
    Object.defineProperty(timeline, "scrollTop", {
      value: 0,
      writable: true,
      configurable: true
    });
    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [
            message("$anchor:example.invalid", "Anchor"),
            message("$after:example.invalid", "After")
          ]
        }
      }
    });

    await waitFor(() => {
      expect(timeline.scrollTop).toBe(1400);
    });
    await act(async () => {
      await new Promise<void>((resolve) => {
        requestAnimationFrame(() => resolve());
      });
    });
    timeline.scrollTop = 48;
    fireEvent.wheel(timeline, { deltaY: -120 });
    fireEvent.scroll(timeline);
    await waitFor(() => {
      expect(updateScrollAnchor).toHaveBeenLastCalledWith(
        "!room:example.invalid",
        expect.objectContaining({
          event_id: "$after:example.invalid",
          edge: "bottom"
        })
      );
    });

    unmount();
    const fallbackRender = render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );
    const fallbackTimeline = await screen.findByTestId("timeline-view");
    scrollContainerRef.current = fallbackTimeline;
    Object.defineProperty(fallbackTimeline, "scrollHeight", { value: 2000, configurable: true });
    Object.defineProperty(fallbackTimeline, "clientHeight", { value: 600, configurable: true });
    Object.defineProperty(fallbackTimeline, "scrollTop", {
      value: 0,
      writable: true,
      configurable: true
    });
    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [
            message("$live-top:example.invalid", "Live top"),
            message("$live-bottom:example.invalid", "Live bottom")
          ]
        }
      }
    });

    await waitFor(() => {
      expect(fallbackTimeline.scrollTop).toBe(1400);
    });

    fallbackRender.unmount();
    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );
    const liveEdgeTimeline = await screen.findByTestId("timeline-view");
    scrollContainerRef.current = liveEdgeTimeline;
    Object.defineProperty(liveEdgeTimeline, "scrollHeight", { value: 2000, configurable: true });
    Object.defineProperty(liveEdgeTimeline, "clientHeight", { value: 600, configurable: true });
    Object.defineProperty(liveEdgeTimeline, "scrollTop", {
      value: 0,
      writable: true,
      configurable: true
    });
    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [
            message("$anchor:example.invalid", "Anchor"),
            message("$after:example.invalid", "After")
          ]
        }
      }
    });

    await waitFor(() => {
      expect(liveEdgeTimeline.scrollTop).toBe(1400);
    });
  });

  it("opens a 100-row room at the live edge without mounting its first rows", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const rowHeight = 72;
    const itemCount = 100;
    const rects: Record<string, { top: number; height: number }> = {};
    for (let index = 0; index < itemCount; index += 1) {
      rects[`$row-${index}:example.invalid`] = { top: index * rowHeight, height: rowHeight };
    }
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    mockTimelineRects(rects, { top: 0, height: 600 }, scrollContainerRef);

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        listRefCallback={(element) => {
          scrollContainerRef.current = element?.closest<HTMLElement>("[data-testid=timeline-view]") ?? null;
        }}
      />
    );
    const timeline = await screen.findByTestId("timeline-view");
    scrollContainerRef.current = timeline;
    Object.defineProperty(timeline, "scrollHeight", {
      value: itemCount * rowHeight,
      configurable: true
    });
    Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
    Object.defineProperty(timeline, "scrollTop", {
      value: 0,
      writable: true,
      configurable: true
    });

    // Every row ever mounted, not only the final window: a live-edge open that
    // first mounts the top screen and then snaps to the bottom shows up here.
    const everMounted = new Set<string>();
    const record = () => {
      for (const node of timeline.querySelectorAll<HTMLElement>("[data-event-id]")) {
        everMounted.add(node.dataset["eventId"] ?? "");
      }
    };
    const observer = new MutationObserver(record);
    observer.observe(timeline, { childList: true, subtree: true });

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: Array.from({ length: itemCount }, (_, index) =>
            message(`$row-${index}:example.invalid`, `message ${index}`)
          )
        }
      }
    });

    await waitFor(() => {
      expect(timeline.scrollTop).toBe(itemCount * rowHeight - 600);
    });
    await waitFor(() => {
      expect(
        timeline.querySelector(`[data-event-id="$row-${itemCount - 1}:example.invalid"]`)
      ).not.toBeNull();
    });
    record();
    observer.disconnect();

    expect([...everMounted]).not.toContain("$row-0:example.invalid");
    expect(everMounted.size).toBeLessThanOrEqual(40);
  });


  it("drops a focused target from the window once the reader scrolls away", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const rowHeight = 72;
    const itemCount = 200;
    const targetIndex = 5;
    const targetEventId = `$row-${targetIndex}:example.invalid`;
    const focusedKey = focusedTimelineKey(
      "@alice:example.invalid",
      "!room:example.invalid",
      targetEventId
    );
    const rects: Record<string, { top: number; height: number }> = {};
    for (let index = 0; index < itemCount; index += 1) {
      rects[`$row-${index}:example.invalid`] = { top: index * rowHeight, height: rowHeight };
    }
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    mockTimelineRects(rects, { top: 0, height: 600 }, scrollContainerRef);
    // jsdom implements no scrollIntoView; the placement must not need it to
    // reach the target, so a no-op is enough here.
    const scrollIntoView = vi.fn();
    Element.prototype.scrollIntoView = scrollIntoView;

    render(
      <TimelineView
        timelineKey={focusedKey}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        listRefCallback={(element) => {
          scrollContainerRef.current =
            element?.closest<HTMLElement>("[data-testid=timeline-view]") ?? null;
        }}
      />
    );
    const timeline = await screen.findByTestId("timeline-view");
    scrollContainerRef.current = timeline;
    Object.defineProperty(timeline, "scrollHeight", {
      value: itemCount * rowHeight,
      configurable: true
    });
    Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
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
            items: Array.from({ length: itemCount }, (_, index) =>
              message(`$row-${index}:example.invalid`, `message ${index}`)
            )
          }
        }
      });
    });

    // The jump keeps its target mounted while it owns the viewport.
    await waitFor(() => {
      expect(timeline.querySelector(`[data-event-id="${targetEventId}"]`)).not.toBeNull();
    });

    // The reader then scrolls to the live edge; the focused target is history.
    act(() => {
      timeline.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: 40 }));
      timeline.scrollTop = itemCount * rowHeight - 600;
      fireEvent.scroll(timeline);
    });
    await waitFor(() => {
      expect(
        timeline.querySelector(`[data-event-id="$row-${itemCount - 1}:example.invalid"]`)
      ).not.toBeNull();
    });

    expect(timeline.querySelector(`[data-event-id="${targetEventId}"]`)).toBeNull();
    expect(timeline.querySelectorAll("[data-event-id]").length).toBeLessThanOrEqual(60);
  });


  it("places a thread pane's pinned reply that sits outside the mounted window", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const rowHeight = 72;
    const itemCount = 100;
    const targetIndex = 5;
    const targetEventId = `$row-${targetIndex}:example.invalid`;
    const threadKey = threadTimelineKey(
      "@alice:example.invalid",
      "!room:example.invalid",
      "$thread-root:example.invalid"
    );
    const rects: Record<string, { top: number; height: number }> = {};
    for (let index = 0; index < itemCount; index += 1) {
      rects[`$row-${index}:example.invalid`] = { top: index * rowHeight, height: rowHeight };
    }
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    mockTimelineRects(rects, { top: 0, height: 600 }, scrollContainerRef);
    const scrollIntoView = vi.fn();
    Element.prototype.scrollIntoView = scrollIntoView;

    render(
      <TimelineView
        presentationContext="thread"
        timelineKey={threadKey}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        onOpenThread={vi.fn()}
        initialTargetEventId={targetEventId}
        listRefCallback={(element) => {
          scrollContainerRef.current =
            element?.closest<HTMLElement>("[data-testid=timeline-view]") ?? null;
        }}
      />
    );
    const timeline = await screen.findByTestId("timeline-view");
    scrollContainerRef.current = timeline;
    Object.defineProperty(timeline, "scrollHeight", {
      value: itemCount * rowHeight,
      configurable: true
    });
    Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
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
            key: threadKey,
            generation: 1,
            items: Array.from({ length: itemCount }, (_, index) =>
              message(`$row-${index}:example.invalid`, `reply ${index}`)
            )
          }
        }
      });
    });

    // The pane opens at the live edge, so the pinned reply starts outside the
    // mounted window; the jump has to reach it through the height model.
    await waitFor(() => {
      expect(timeline.querySelector(`[data-event-id="${targetEventId}"]`)).not.toBeNull();
    });
    expect(scrollIntoView).toHaveBeenCalled();
  });


  it("does not reapply a persisted room anchor across later rerenders", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const rects = {
      "$anchor:example.invalid": { top: 500, height: 48 },
      "$after:example.invalid": { top: 560, height: 48 }
    };

    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    mockTimelineRects(rects, { top: 0, height: 600 }, scrollContainerRef);

    const props = {
      timelineKey: KEY,
      roomId: "!room:example.invalid",
      transport,
      roomScrollAnchor: {
        event_id: "$anchor:example.invalid",
        edge: "bottom" as const,
        offset_px: -100,
        updated_at_ms: Date.now()
      },
      onReply: vi.fn()
    };
    const { rerender } = render(<TimelineView {...props} />);

    const timeline = await screen.findByTestId("timeline-view");
    scrollContainerRef.current = timeline;
    Object.defineProperty(timeline, "scrollHeight", { value: 2000, configurable: true });
    Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
    Object.defineProperty(timeline, "scrollTop", {
      value: 0,
      writable: true,
      configurable: true
    });

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [
            message("$anchor:example.invalid", "Anchor"),
            message("$after:example.invalid", "After")
          ]
        }
      }
    });

    await waitFor(() => {
      expect(timeline.scrollTop).toBe(1400);
    });

    rects["$anchor:example.invalid"].top = 530;
    rerender(
      <TimelineView
        {...props}
        liveSignals={{ presence: {}, rooms: {} }}
      />
    );

    expect(timeline.scrollTop).toBe(1400);
  });


  it("does not move a free-scroll viewport when read receipts shift earlier rows", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const rects = {
      "$seen:example.invalid": { top: 440, height: 48 },
      "$anchor:example.invalid": { top: 500, height: 48 },
      "$after:example.invalid": { top: 560, height: 48 }
    };

    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    mockTimelineRects(rects, { top: 0, height: 600 }, scrollContainerRef);

    const props = {
      timelineKey: KEY,
      roomId: "!room:example.invalid",
      transport,
      roomScrollAnchor: {
        event_id: "$anchor:example.invalid",
        edge: "bottom" as const,
        offset_px: -100,
        updated_at_ms: Date.now()
      },
      onReply: vi.fn()
    };
    const { rerender } = render(<TimelineView {...props} />);

    const timeline = await screen.findByTestId("timeline-view");
    scrollContainerRef.current = timeline;
    Object.defineProperty(timeline, "scrollHeight", { value: 2000, configurable: true });
    Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
    Object.defineProperty(timeline, "scrollTop", {
      value: 0,
      writable: true,
      configurable: true
    });

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [
            message("$seen:example.invalid", "Seen"),
            message("$anchor:example.invalid", "Anchor"),
            message("$after:example.invalid", "After")
          ]
        }
      }
    });

    await waitFor(() => {
      expect(timeline.scrollTop).toBe(1400);
    });
    await act(async () => {
      await new Promise<void>((resolve) => {
        requestAnimationFrame(() => resolve());
      });
    });

    rects["$anchor:example.invalid"].top = 530;
    timeline.scrollTop = 58;
    fireEvent.wheel(timeline, { deltaY: -120 });
    fireEvent.scroll(timeline);
    rerender(
      <TimelineView
        {...props}
        liveSignals={{
          presence: {},
          rooms: {
            "!room:example.invalid": {
              fully_read_event_id: null,
              typing_user_ids: [],
              typing_users: [],
              focused_receipts_by_event: {}, thread_receipts_by_event: {}, receipts_by_event: {
                "$seen:example.invalid": {
                  total_count: 1,
                  overflow_count: 0,
                  readers: [
                    {
                      user_id: "@satoshi:example.invalid",
                      display_name: "Satoshi Terasaki",
                      original_display_label: "Satoshi Terasaki",
                      avatar: null,
                      timestamp_ms: null
                    }
                  ]
                }
              }
            }
          }
        }}
      />
    );

    expect(timeline.scrollTop).toBe(58);
  });


  it("ignores persisted anchors on first room entry and opens at the live edge", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const roomId = "!room:example.invalid";
    const anchorEventId = "$anchor:example.invalid";
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });

    mockTimelineRects(
      {
        "$live-top:example.invalid": { top: 120, height: 48 },
        "$live-bottom:example.invalid": { top: 560, height: 48 },
        [anchorEventId]: { top: 500, height: 48 }
      },
      { top: 0, height: 600 }
    );

    render(
      <TimelineView
        timelineKey={KEY}
        roomId={roomId}
        transport={transport}
        roomScrollAnchor={{
          event_id: anchorEventId,
          edge: "bottom",
          offset_px: -100,
          updated_at_ms: Date.now()
        }}
        onReply={vi.fn()}
      />
    );

    const timeline = await screen.findByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollHeight", { value: 2000, configurable: true });
    Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
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
            items: [
              message("$live-top:example.invalid", "Live top"),
              message("$live-bottom:example.invalid", "Live bottom")
            ]
          }
        }
      });
    });

    await waitFor(() => {
      expect(screen.getByText("Live top")).toBeTruthy();
      expect(timeline.getAttribute("data-timeline-generation")).toBe("1");
      expect(timeline.scrollTop).toBe(1400);
    });
  });


  it("does not chase a missing persisted anchor on first room entry", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const roomId = "!room:example.invalid";
    const anchorEventId = "$anchor:example.invalid";
    const updateScrollAnchor = vi.fn(async () => undefined);
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      updateScrollAnchor
    });
    mockTimelineRects(
      {
        "$live-top:example.invalid": { top: 1300, height: 48 },
        "$live-bottom:example.invalid": { top: 1900, height: 48 }
      },
      { top: 0, height: 600 },
      scrollContainerRef
    );

    render(
      <TimelineView
        timelineKey={KEY}
        roomId={roomId}
        transport={transport}
        roomScrollAnchor={{
          event_id: anchorEventId,
          edge: "bottom",
          offset_px: 50,
          updated_at_ms: Date.now()
        }}
        onReply={vi.fn()}
      />
    );

    const timeline = await screen.findByTestId("timeline-view");
    scrollContainerRef.current = timeline;
    Object.defineProperty(timeline, "scrollHeight", { value: 2000, configurable: true });
    Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
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
            items: [
              message("$live-top:example.invalid", "Live top"),
              message("$live-bottom:example.invalid", "Live bottom")
            ]
          }
        }
      });
    });

    await waitFor(() => {
      expect(timeline.scrollTop).toBe(1400);
    });
    await waitFor(() => {
      expect(updateScrollAnchor).toHaveBeenCalledWith(
        roomId,
        expect.objectContaining({
          event_id: "$live-bottom:example.invalid",
          edge: "bottom"
        })
      );
    });
  });


  it("restores the live edge after a same-key timeline resync generation arrives", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
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

    const timeline = await screen.findByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollHeight", { value: 2000, configurable: true });
    Object.defineProperty(timeline, "clientHeight", { value: 500, configurable: true });
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
            items: [message("$first", "First generation")]
          }
        }
      });
    });

    await waitFor(() => {
      expect(timeline.scrollTop).toBe(1500);
    });

    timeline.scrollTop = 100;

    act(() => {
      emit({ kind: "ResyncMarker" });
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 2,
            items: [message("$second", "Second generation")]
          }
        }
      });
    });

    await waitFor(() => {
      expect(screen.getByText("Second generation")).toBeTruthy();
      expect(timeline.scrollTop).toBe(1500);
    });
  });


  it("scrolls to the sent local echo even when the user was reading above the bottom", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
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
        currentUserId="@alice:example.invalid"
        onReply={vi.fn()}
      />
    );

    const timeline = await screen.findByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollHeight", { value: 2400, configurable: true });
    Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
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
            items: [message("$older", "Older message")]
          }
        }
      });
    });

    await waitFor(() => {
      expect(timeline.scrollTop).toBe(1800);
    });

    timeline.scrollTop = 400;

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
                PushBack: {
                  item: {
                    ...message("$local-echo", "Message I just sent"),
                    id: { Transaction: { transaction_id: "txn-1" } },
                    sender: "@alice:example.invalid",
                    send_state: { kind: "sending" }
                  }
                }
              }
            ]
          }
        }
      });
    });

    await waitFor(() => {
      expect(screen.getByText("Message I just sent")).toBeTruthy();
      expect(timeline.scrollTop).toBe(1800);
    });
  });


  it("keeps the live edge pinned when the read marker appears below a sent message", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
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
        currentUserId="@alice:example.invalid"
        onReply={vi.fn()}
      />
    );

    const timeline = await screen.findByTestId("timeline-view");
    let scrollHeight = 2400;
    Object.defineProperty(timeline, "scrollHeight", {
      get: () => scrollHeight,
      configurable: true
    });
    Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
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
            items: [message("$older", "Older message")]
          }
        }
      });
    });

    await waitFor(() => {
      expect(timeline.scrollTop).toBe(1800);
    });

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
                PushBack: {
                  item: {
                    ...message("$sent:example.invalid", "Test"),
                    sender: "@alice:example.invalid",
                    send_state: { kind: "sending" }
                  }
                }
              }
            ]
          }
        }
      });
    });

    await waitFor(() => {
      expect(screen.getByText("Test")).toBeTruthy();
      expect(timeline.scrollTop).toBe(1800);
    });

    scrollHeight = 2440;
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          NavigationUpdated: {
            key: KEY,
            snapshot: {
              read_marker_event_id: "$sent:example.invalid",
              read_marker_display_event_id: "$sent:example.invalid",
              local_viewed_event_id: "$sent:example.invalid",
              server_confirmed_read_event_id: "$sent:example.invalid",
              read_state_sync: "synced",
              first_unread_event_id: null,
              first_unread_display_event_id: null,
              unread_event_count: 0,
              unread_position: "none",
              newer_event_count: 0,
              can_jump_to_bottom: false
            }
          }
        }
      });
    });

    expect(await screen.findByRole("separator", { name: "Read up to here" })).toBeTruthy();
    await waitFor(() => {
      expect(timeline.scrollTop).toBe(1840);
    });
  });


  it("keeps the live edge pinned when rendered content grows without a React commit", async () => {
    const resizeObserver = installResizeObserverMock();
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      callback(0);
      return 0;
    });
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });

    try {
      render(
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={transport}
          currentUserId="@alice:example.invalid"
          onReply={vi.fn()}
        />
      );

      const timeline = await screen.findByTestId("timeline-view");
      let scrollHeight = 2400;
      Object.defineProperty(timeline, "scrollHeight", {
        get: () => scrollHeight,
        configurable: true
      });
      Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
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
              items: [message("$older", "Older message")]
            }
          }
        });
      });

      await waitFor(() => {
        expect(timeline.scrollTop).toBe(1800);
      });

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
                  PushBack: {
                    item: {
                      ...message("$sent:example.invalid", "Test"),
                      sender: "@alice:example.invalid",
                      send_state: { kind: "sending" }
                    }
                  }
                }
              ]
            }
          }
        });
      });

      await waitFor(() => {
        expect(screen.getByText("Test")).toBeTruthy();
        expect(timeline.scrollTop).toBe(1800);
      });

      scrollHeight = 2480;
      act(() => {
        resizeObserver.triggerAll();
      });

      await waitFor(() => {
        expect(timeline.scrollTop).toBe(1880);
      });
    } finally {
      resizeObserver.restore();
    }
  });


  it("does not keep the sent-message live-edge lock after user scroll input", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
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
        currentUserId="@alice:example.invalid"
        onReply={vi.fn()}
      />
    );

    const timeline = await screen.findByTestId("timeline-view");
    let scrollHeight = 2400;
    Object.defineProperty(timeline, "scrollHeight", {
      get: () => scrollHeight,
      configurable: true
    });
    Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
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
            items: [message("$older", "Older message")]
          }
        }
      });
    });

    await waitFor(() => {
      expect(timeline.scrollTop).toBe(1800);
    });

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
                PushBack: {
                  item: {
                    ...message("$sent:example.invalid", "Test"),
                    sender: "@alice:example.invalid",
                    send_state: { kind: "sending" }
                  }
                }
              }
            ]
          }
        }
      });
    });

    await waitFor(() => {
      expect(timeline.scrollTop).toBe(1800);
    });

    act(() => {
      fireEvent.wheel(timeline, { deltaY: -120 });
      timeline.scrollTop = 1700;
      fireEvent.scroll(timeline);
    });

    scrollHeight = 2440;
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          NavigationUpdated: {
            key: KEY,
            snapshot: {
              read_marker_event_id: "$sent:example.invalid",
              read_marker_display_event_id: "$sent:example.invalid",
              local_viewed_event_id: "$sent:example.invalid",
              server_confirmed_read_event_id: "$sent:example.invalid",
              read_state_sync: "synced",
              first_unread_event_id: null,
              first_unread_display_event_id: null,
              unread_event_count: 0,
              unread_position: "none",
              newer_event_count: 0,
              can_jump_to_bottom: false
            }
          }
        }
      });
    });

    expect(await screen.findByRole("separator", { name: "Read up to here" })).toBeTruthy();
    expect(timeline.scrollTop).toBe(1700);
  });


  it("drops the live-edge lock immediately on wheel input before the scroll event settles", async () => {
    const resizeObserver = installResizeObserverMock();
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      callback(0);
      return 0;
    });
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });

    try {
      render(
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={transport}
          currentUserId="@alice:example.invalid"
          onReply={vi.fn()}
        />
      );

      const timeline = await screen.findByTestId("timeline-view");
      let scrollHeight = 2400;
      Object.defineProperty(timeline, "scrollHeight", {
        get: () => scrollHeight,
        configurable: true
      });
      Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
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
              items: [message("$older", "Older message")]
            }
          }
        });
      });

      await waitFor(() => {
        expect(timeline.scrollTop).toBe(1800);
      });

      act(() => {
        fireEvent.wheel(timeline, { deltaY: -120 });
      });

      scrollHeight = 2480;
      act(() => {
        resizeObserver.triggerAll();
      });

      expect(timeline.scrollTop).toBe(1800);
    } finally {
      resizeObserver.restore();
    }
  });


  it("keeps a row's measured height when the row unmounts before the idle flush", async () => {
    // #1890: a reader wheeling through history measures rows. Those measurements
    // used to be discarded when the row left the mounted window before the flush
    // (`!mountedDomIds.has(domId)`), so the spacer above kept the 72 px estimate
    // and every remount moved the reader by the accumulated error.
    vi.useFakeTimers();
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const ROW_HEIGHT = 40;
    const ROW_COUNT = 160;
    const rects: Record<string, { top: number; height: number }> = {};
    for (let index = 0; index < ROW_COUNT; index += 1) {
      rects[`$row${index}`] = { top: index * ROW_HEIGHT, height: ROW_HEIGHT };
    }
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    mockTimelineRects(rects, { top: 0, height: 600 }, scrollContainerRef);

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        listRefCallback={(element) => {
          scrollContainerRef.current =
            element?.closest<HTMLElement>("[data-testid=timeline-view]") ?? null;
        }}
      />
    );
    // `getBy` rather than `findBy`: fake timers are installed above.
    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperties(timeline, {
      scrollHeight: { value: ROW_COUNT * ROW_HEIGHT, writable: true, configurable: true },
      clientHeight: { value: 600, configurable: true },
      scrollTop: { value: 0, writable: true, configurable: true }
    });
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: Array.from({ length: ROW_COUNT }, (_, index) =>
              message(`$row${index}`, `row ${index}`)
            )
          }
        }
      });
    });

    // Settle the opening window, then wheel through history one screen at a
    // time: each step mounts and measures rows and the next step unmounts them,
    // so what the reader passed has to survive in the height cache rather than
    // in the DOM.
    await act(async () => {
      vi.advanceTimersByTime(200);
      await Promise.resolve();
    });
    for (let scrollTop = 10_320; scrollTop >= 0; scrollTop -= 600) {
      act(() => {
        fireEvent.wheel(timeline, { deltaY: 40 });
      });
      timeline.scrollTop = scrollTop;
      act(() => {
        fireEvent.scroll(timeline);
      });
      await act(async () => {
        vi.advanceTimersByTime(200);
        await Promise.resolve();
      });
    }
    // The scroll settles: the deferred commits land.
    await act(async () => {
      vi.advanceTimersByTime(200);
      await Promise.resolve();
    });

    // Back at the top, the rows above the window are exactly the rows the reader
    // passed, so every one of them is a measurement: the timeline's height is the
    // true 160 x 40, not a mixture with the 72 px estimate.
    timeline.scrollTop = 0;
    act(() => {
      fireEvent.scroll(timeline);
    });
    await act(async () => {
      vi.advanceTimersByTime(200);
      await Promise.resolve();
    });

    const list = timeline.querySelector(".timeline-item-list")!;
    const spacers = Array.from(list.querySelectorAll<HTMLElement>(".timeline-virtual-spacer"));
    expect(spacers).toHaveLength(2);
    const paddingTop = Number.parseFloat(spacers[0]!.style.blockSize || "0");
    const paddingBottom = Number.parseFloat(spacers[1]!.style.blockSize || "0");
    const renderedRows = list.querySelectorAll(".timeline-item-frame").length;
    expect(paddingTop).toBe(0);
    expect(renderedRows).toBeGreaterThan(0);
    expect(paddingTop + renderedRows * ROW_HEIGHT + paddingBottom).toBe(ROW_COUNT * ROW_HEIGHT);
  });

  it("commits a measured height above the anchor with the compensation, not on idle", async () => {
    // #1890 §2.2: a row above the reader that measures shorter moves them, so
    // its height has to land in the same pass as the anchor compensation. While
    // the scroll is active the pass used to defer every commit, which is what
    // made the position drift until the idle flush.
    vi.useFakeTimers();
    const { onScrollDiagnosticsChange, scrollTo, setRowHeight } = await mountTimedRoom(160);
    const baseline = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    const baselineCommits = baseline?.heightModelCommits ?? 0;

    // The reader scrolls into the middle of the room while the rows above them
    // measure shorter than the 72 px estimate.
    for (let index = 61; index <= 66; index += 1) {
      setRowHeight(index, 40);
    }
    await scrollTo(4996);

    const active = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    expect(active.heightModelCommits - baselineCommits).toBeGreaterThan(0);
  });

  it("leaves a measured height below the anchor to the idle commit and never moves the reader", async () => {
    vi.useFakeTimers();
    const { timeline, onScrollDiagnosticsChange, scrollTo, setRowHeight } =
      await mountTimedRoom(160);
    const baseline = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    const baselineCommits = baseline?.heightModelCommits ?? 0;

    // The rows that measure shorter sit below the reader, so they only change
    // the extent under them.
    for (let index = 80; index <= 85; index += 1) {
      setRowHeight(index, 40);
    }
    await scrollTo(4996);

    const active = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    expect(active.heightModelCommits - baselineCommits).toBe(0);
    expect(timeline.scrollTop).toBe(4996);

    await act(async () => {
      vi.advanceTimersByTime(200);
      await Promise.resolve();
    });
    const settled = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    expect(settled.heightModelCommits - baselineCommits).toBeGreaterThan(0);
    expect(timeline.scrollTop).toBe(4996);
  });

/**
 * The height model's extent for the whole list, as the DOM shows it: both
 * spacer blocks the model's padding plus the rows it mounted.
 */
function modelledExtent(timeline: HTMLElement): number {
  const padding = Array.from(
    timeline.querySelectorAll<HTMLElement>(".timeline-virtual-spacer")
  ).reduce((total, spacer) => total + Number.parseFloat(spacer.style.blockSize || "0"), 0);
  const measured = Array.from(timeline.querySelectorAll<HTMLElement>("[data-item-id]")).reduce(
    (total, row) => total + row.getBoundingClientRect().height,
    0
  );
  return padding + measured;
}

  it("sizes the room from its first measurements, not from the 72 px constant", async () => {
    // #1890 §2.3: every unmeasured row starts at what the open measured. With
    // the 72 px constant in a 48 px room the model is a third too tall, which is
    // the padding the round-trip test reaches.
    vi.useFakeTimers();
    const rowHeight = 48;
    const rowCount = 160;
    const { timeline } = await mountTimedRoom(rowCount, rowHeight);
    await act(async () => {
      vi.advanceTimersByTime(200);
      await Promise.resolve();
    });

    const extent = modelledExtent(timeline);
    expect(extent).toBeGreaterThan(rowCount * rowHeight * 0.95);
    expect(extent).toBeLessThan(rowCount * rowHeight * 1.05);
  });

  it("freezes a new estimate for a new timeline key", async () => {
    vi.useFakeTimers();
    const ROW_COUNT = 160;
    const SHORT = 48;
    const TALL = 120;
    const roomKey = (roomId: string): TimelineKey =>
      roomTimelineKey(KEY.account_key, roomId);
    const rects: Record<string, { top: number; height: number }> = {};
    for (let index = 0; index < ROW_COUNT; index += 1) {
      rects[`$row${index}`] = { top: index * SHORT, height: SHORT };
    }
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    mockTimelineRects(rects, { top: 0, height: 600 }, scrollContainerRef);
    let listener: ((payload: CoreEventPayload) => void) | null = null;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        listener = nextListener;
        return () => undefined;
      }
    });
    const seed = (key: TimelineKey) =>
      act(() => {
        listener?.({
          kind: "Timeline",
          event: {
            InitialItems: {
              request_id: null,
              key,
              generation: 1,
              items: Array.from({ length: ROW_COUNT }, (_, index) =>
                message(`$row${index}`, `row ${index}`)
              )
            }
          }
        });
      });
    const renderRoom = (roomId: string) => (
      <TimelineView
        timelineKey={roomKey(roomId)}
        roomId={roomId}
        transport={transport}
        onReply={vi.fn()}
        listRefCallback={(element) => {
          scrollContainerRef.current =
            element?.closest<HTMLElement>("[data-testid=timeline-view]") ?? null;
        }}
      />
    );

    const { rerender } = render(renderRoom("!short:example.invalid"));
    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperties(timeline, {
      scrollHeight: { value: ROW_COUNT * SHORT, writable: true, configurable: true },
      clientHeight: { value: 600, configurable: true },
      scrollTop: { value: 0, writable: true, configurable: true }
    });
    seed(roomKey("!short:example.invalid"));
    await act(async () => {
      vi.advanceTimersByTime(200);
      await Promise.resolve();
    });
    expect(modelledExtent(timeline)).toBeLessThan(ROW_COUNT * SHORT * 1.05);

    for (let index = 0; index < ROW_COUNT; index += 1) {
      rects[`$row${index}`] = { top: index * TALL, height: TALL };
    }
    rerender(renderRoom("!tall:example.invalid"));
    seed(roomKey("!tall:example.invalid"));
    await act(async () => {
      vi.advanceTimersByTime(200);
      await Promise.resolve();
    });

    // A key change clears the measurement cache, so the new room has to freeze
    // its own estimate instead of keeping the previous room's.
    const extent = modelledExtent(timeline);
    expect(extent).toBeGreaterThan(ROW_COUNT * TALL * 0.95);
    expect(extent).toBeLessThan(ROW_COUNT * TALL * 1.05);
  });

  it("commits a scroll frame's window before the frame ends", async () => {
    // #1890: a range set from a rAF is a default-lane update, so React renders
    // it in a Scheduler task after paint and the frame shows the old window at
    // the new scrollTop. That is the one-frame gap the round trip in
    // `timeline-scroll-anchor-drift.spec.ts` counts. This runs the scroll and its
    // frame outside `act` on purpose: a flush from `act` would hide the gap.
    vi.useFakeTimers();
    const ROW_HEIGHT = 48;
    const ROW_COUNT = 120;
    const CLIENT_HEIGHT = 600;
    const frames: FrameRequestCallback[] = [];
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      frames.push(callback);
      return frames.length;
    });
    const { timeline } = await mountTimedRoom(ROW_COUNT, ROW_HEIGHT);
    const runFrames = () => {
      for (const frame of frames.splice(0)) {
        frame(0);
      }
    };
    await act(async () => {
      vi.advanceTimersByTime(200);
      runFrames();
      await Promise.resolve();
    });

    const mountedIndices = () =>
      Array.from(timeline.querySelectorAll<HTMLElement>("[data-item-id]"))
        .map((row) => Number.parseInt(row.dataset["itemId"]?.replace("$row", "-") ?? "", 10))
        .map((value) => Math.abs(value))
        .sort((a, b) => a - b);
    const before = mountedIndices();
    expect(before.length).toBeGreaterThan(0);

    const newScrollTop = timeline.scrollTop - CLIENT_HEIGHT;
    frames.length = 0;
    // A wheel first: a bare scrollTop write is not user input, and the viewport
    // would stay pinned to the live edge.
    timeline.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -40 }));
    timeline.scrollTop = newScrollTop;
    timeline.dispatchEvent(new Event("scroll"));
    expect(frames.length).toBeGreaterThan(0);
    for (const frame of frames.splice(0)) {
      frame(0);
    }

    const firstVisible = Math.floor(newScrollTop / ROW_HEIGHT);
    const after = mountedIndices();
    // The frame the reader is looking at has to be the window for the scrollTop
    // it is scrolled to, not the one from before the wheel: the mounted range
    // moves by the screen the scroll moved.
    expect(Math.min(...after)).toBeLessThan(Math.min(...before));
    expect(Math.min(...after)).toBeLessThanOrEqual(firstVisible);
    expect(Math.max(...after)).toBeGreaterThanOrEqual(
      firstVisible + Math.ceil(CLIENT_HEIGHT / ROW_HEIGHT) - 1
    );
  });

  it("re-measures an edited row when it comes back into the window", async () => {
    // #1890: the cache keys freshness on the item object the store hands over, so
    // an edited row keeps its old height only as an estimate until it remounts.
    vi.useFakeTimers();
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const ROW_HEIGHT = 40;
    const EDITED_HEIGHT = 120;
    const ROW_COUNT = 160;
    const rects: Record<string, { top: number; height: number }> = {};
    for (let index = 0; index < ROW_COUNT; index += 1) {
      rects[`$row${index}`] = { top: index * ROW_HEIGHT, height: ROW_HEIGHT };
    }
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    mockTimelineRects(rects, { top: 0, height: 600 }, scrollContainerRef);

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        listRefCallback={(element) => {
          scrollContainerRef.current =
            element?.closest<HTMLElement>("[data-testid=timeline-view]") ?? null;
        }}
      />
    );
    // `getBy` rather than `findBy`: fake timers are installed above.
    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperties(timeline, {
      scrollHeight: { value: ROW_COUNT * ROW_HEIGHT, writable: true, configurable: true },
      clientHeight: { value: 600, configurable: true },
      scrollTop: { value: 0, writable: true, configurable: true }
    });
    const initialItems = Array.from({ length: ROW_COUNT }, (_, index) =>
      message(`$row${index}`, `row ${index}`)
    );
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: { request_id: null, key: KEY, generation: 1, items: initialItems }
        }
      });
    });

    // Measure the middle rows, then leave them off-window.
    for (const scrollTop of [2000, 0]) {
      act(() => {
        fireEvent.wheel(timeline, { deltaY: 40 });
      });
      timeline.scrollTop = scrollTop;
      act(() => {
        fireEvent.scroll(timeline);
      });
      await act(async () => {
        vi.advanceTimersByTime(200);
        await Promise.resolve();
      });
    }

    // The model's total, read back from the DOM: the spacers plus the rows that
    // are mounted at their mocked heights.
    const timelineTotalHeight = (): number => {
      const list = timeline.querySelector(".timeline-item-list")!;
      const spacers = Array.from(list.querySelectorAll<HTMLElement>(".timeline-virtual-spacer"));
      const padding =
        Number.parseFloat(spacers[0]?.style.blockSize || "0") +
        Number.parseFloat(spacers[1]?.style.blockSize || "0");
      let rendered = 0;
      for (const frame of Array.from(list.querySelectorAll<HTMLElement>(".timeline-item-frame"))) {
        const id = frame.dataset["frameItemId"] ?? "";
        rendered += rects[id]?.height ?? 0;
      }
      return padding + rendered;
    };
    const totalBeforeEdit = timelineTotalHeight();

    // A row that is off-window grows: the store replaces its item object.
    const editedIndex = 60;
    rects[`$row${editedIndex}`] = {
      top: editedIndex * ROW_HEIGHT,
      height: EDITED_HEIGHT
    };
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
                Set: {
                  index: editedIndex,
                  item: message(`$row${editedIndex}`, "row 60 edited with a much longer body")
                }
              }
            ]
          }
        }
      });
    });
    await act(async () => {
      vi.advanceTimersByTime(200);
      await Promise.resolve();
    });

    // Coming back to it re-measures, so the timeline grows by the edit's delta
    // rather than keeping the height the row had before it was edited.
    timeline.scrollTop = 2000;
    act(() => {
      fireEvent.scroll(timeline);
    });
    await act(async () => {
      vi.advanceTimersByTime(200);
      await Promise.resolve();
    });

    // Exactly the edit's delta: the row was measured at 40 px before it left the
    // window, and coming back re-measures it at 120 px. Trusting the retained
    // entry instead would leave the total unchanged.
    expect(timelineTotalHeight()).toBe(totalBeforeEdit + (EDITED_HEIGHT - ROW_HEIGHT));
  });

});
