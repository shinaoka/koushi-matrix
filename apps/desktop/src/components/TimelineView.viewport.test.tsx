// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  focusedTimelineKey,
  type CoreEventPayload,
  type TimelineGapId
} from "../domain/coreEvents";
import { setActiveLocaleProfile } from "../i18n/messages";
import { KEY, baseTransport, message, mockTimelineRects } from "./timelineViewTestSupport";
import { TimelineView, clearTimelineViewportSessionMemoryForTests } from "./TimelineView";

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  setActiveLocaleProfile("en", "none");
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

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

      await waitFor(() => {
        expect(onReturnToLive).toHaveBeenCalledTimes(1);
      });
      expect(observeViewport).not.toHaveBeenCalled();
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

    expect(updateScrollAnchor).toHaveBeenCalledTimes(1);
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

    expect(updateScrollAnchor).toHaveBeenCalledTimes(1);
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
              receipts_by_event: {
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

});
