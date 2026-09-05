// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  threadTimelineKey,
  type CoreEventPayload,
  type TimelineDiff,
  type TimelineItem
} from "../domain/coreEvents";
import { setActiveLocaleProfile } from "../i18n/messages";
import { createManualTimelineViewportScheduler } from "./timeline/TimelineViewportScheduler";
import {
  KEY,
  baseTransport,
  message,
  mockTimelineRects,
  navigationSnapshot
} from "./timelineViewTestSupport";
import {
  TimelineView,
  clearTimelineViewportSessionMemoryForTests,
  timelineBackfillThresholdForTests,
  timelineRowsArePurePrependForTests
} from "./TimelineView";

function messages(count: number, prefix = "$item"): TimelineItem[] {
  return Array.from({ length: count }, (_, index) =>
    message(`${prefix}${index}`, `message ${index}`)
  );
}

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  setActiveLocaleProfile("en", "none");
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("TimelineView", () => {
  it.each([5, 700])("retains live-edge resize correction after a passive %ipx clamp notification", (gap) => {
    const scheduler = createManualTimelineViewportScheduler();
    const resizeCallbacks: Array<() => void> = [];
    vi.stubGlobal("ResizeObserver", class {
      constructor(callback: ResizeObserverCallback) {
        resizeCallbacks.push(() => callback([], this as unknown as ResizeObserver));
      }
      observe() {}
      disconnect() {}
    });
    let listener: ((payload: CoreEventPayload) => void) | null = null;
    render(<TimelineView timelineKey={KEY} roomId="!room:example.invalid"
      transport={baseTransport({ listenCoreEvents(callback) { listener = callback; return () => undefined; } })}
      onReply={() => undefined} viewportScheduler={scheduler} />);
    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperties(timeline, {
      scrollHeight: { value: 2000, configurable: true },
      clientHeight: { value: 600, configurable: true },
      scrollTop: { value: 0, writable: true, configurable: true }
    });
    act(() => listener?.({ kind: "Timeline", event: { InitialItems: {
      request_id: null, key: KEY, generation: 1, items: [message("$latest", "Latest")]
    } } }));
    act(() => scheduler.flushAll());
    expect(timeline.scrollTop).toBe(1400);
    fireEvent.scroll(timeline); // Consume the preceding owned write's echo.
    // A transient shorter layout clamps scrollTop; the final layout has grown
    // again before the browser delivers that clamp's scroll notification.
    timeline.scrollTop = 1400 - gap;
    act(() => resizeCallbacks.at(-1)?.());
    fireEvent.scroll(timeline); // No wheel, key, touch or scrollbar intent.
    act(() => scheduler.flushAll());
    expect(timeline.scrollTop).toBe(1400);
  });
  it("drops a stale live-edge follow-up after user viewport input", () => {
    const scheduler = createManualTimelineViewportScheduler();
    let listener: ((payload: CoreEventPayload) => void) | null = null;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        listener = nextListener;
        return () => undefined;
      }
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={() => undefined}
        viewportScheduler={scheduler}
      />
    );

    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollHeight", { value: 2_000, configurable: true });
    Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
    Object.defineProperty(timeline, "scrollTop", {
      value: 0,
      writable: true,
      configurable: true
    });

    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [message("$latest", "Latest")]
          }
        }
      });
      listener?.({
        kind: "Timeline",
        event: {
          NavigationUpdated: {
            key: KEY,
            snapshot: navigationSnapshot({
              can_jump_to_bottom: true,
              newer_event_count: 1
            })
          }
        }
      });
    });
    act(() => scheduler.flushAll());

    fireEvent.click(screen.getByRole("button", { name: /Jump to bottom/ }));
    timeline.scrollTop = 700;
    fireEvent.wheel(timeline, { deltaY: -1 });
    fireEvent.scroll(timeline);
    act(() => scheduler.flushAll());

    expect(timeline.scrollTop).toBe(700);
  });

  it("classifies only stable suffix growth as a pure prepend", () => {
    expect(timelineRowsArePurePrependForTests(["b", "c"], ["a", "b", "c"])).toBe(true);
    expect(timelineRowsArePurePrependForTests(["b", "c"], ["b", "x", "c"])).toBe(false);
    expect(timelineRowsArePurePrependForTests([], ["a"])).toBe(false);
  });

  it("bounds near-top prefetch to two viewport heights", () => {
    expect(timelineBackfillThresholdForTests(900, true)).toBe(1800);
    expect(timelineBackfillThresholdForTests(20, true)).toBe(80);
    expect(timelineBackfillThresholdForTests(900, false)).toBe(0);
  });

  it("emits private-data-free scroll diagnostics for the mounted timeline", async () => {
    const onScrollDiagnosticsChange = vi.fn();
    let listener: ((payload: CoreEventPayload) => void) | null = null;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        listener = nextListener;
        return () => undefined;
      }
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={() => undefined}
        onScrollDiagnosticsChange={onScrollDiagnosticsChange}
      />
    );

    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: Array.from({ length: 700 }, (_, index) =>
              message(`$item${index}`, `message ${index}`)
            )
          }
        }
      });
    });

    await waitFor(() => {
      const latest = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
      expect(latest?.latestFrame?.endIndex ?? 0).toBeGreaterThan(0);
    });
    const latest = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    expect(latest.renderCommits).toBeGreaterThan(0);
    expect(latest.scrollFrames).toBeGreaterThan(0);
    expect(JSON.stringify(latest)).not.toContain("!room:example.invalid");
    expect(JSON.stringify(latest)).not.toContain("$item");
  });

  it("defers virtual height commits during active scroll and flushes once after idle", async () => {
    vi.useFakeTimers();
    let listener: ((payload: CoreEventPayload) => void) | null = null;
    const onScrollDiagnosticsChange = vi.fn();
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        listener = nextListener;
        return () => undefined;
      }
    });

    const rects: Record<string, { top: number; height: number }> = {};
    for (let index = 0; index < 700; index += 1) {
      rects[`$item${index}`] = { top: index * 72, height: 72 };
    }
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    mockTimelineRects(rects, { top: 0, height: 600 }, scrollContainerRef);

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={() => undefined}
        onScrollDiagnosticsChange={onScrollDiagnosticsChange}
        listRefCallback={(element) => {
          scrollContainerRef.current =
            element?.closest<HTMLElement>("[data-testid=timeline-view]") ?? null;
        }}
      />
    );

    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: Array.from({ length: 700 }, (_, index) =>
              message(`$item${index}`, `message ${index}`)
            )
          }
        }
      });
    });

    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollTop", {
      value: 3000,
      writable: true,
      configurable: true
    });
    Object.defineProperty(timeline, "scrollHeight", {
      value: 700 * 72,
      writable: true,
      configurable: true
    });
    Object.defineProperty(timeline, "clientHeight", {
      value: 600,
      writable: true,
      configurable: true
    });
    const baselineDiagnostics = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    const baselineHeightModelCommits = baselineDiagnostics?.heightModelCommits ?? 0;
    const baselineMeasurementFlushes = baselineDiagnostics?.measurementFlushes ?? 0;

    fireEvent.wheel(timeline, { deltaY: 40 });
    fireEvent.scroll(timeline);

    rects.$item50 = { top: 50 * 72, height: 180 };
    fireEvent.scroll(timeline);
    act(() => {
      vi.advanceTimersByTime(16);
    });
    await act(async () => {
      await Promise.resolve();
    });

    const activeDiagnostics = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    expect(activeDiagnostics.heightModelCommits - baselineHeightModelCommits).toBe(0);
    expect(activeDiagnostics.pendingMeasuredRows).toBeGreaterThan(0);

    act(() => {
      vi.advanceTimersByTime(100);
    });

    const idleDiagnostics = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    expect(idleDiagnostics.measurementFlushes - baselineMeasurementFlushes).toBe(1);
    expect(idleDiagnostics.heightModelCommits - baselineHeightModelCommits).toBeGreaterThan(0);
  });

  it("does not hide non-flushed changed rows in the post-flush measurement pass", async () => {
    vi.useFakeTimers();
    let listener: ((payload: CoreEventPayload) => void) | null = null;
    const rects: Record<string, { top: number; height: number }> = {};
    let mutateAfterFlush = false;
    let baselineMeasurementFlushes = 0;
    const onScrollDiagnosticsChange = vi.fn((diagnostics) => {
      if (
        mutateAfterFlush &&
        diagnostics.measurementFlushes > baselineMeasurementFlushes
      ) {
        mutateAfterFlush = false;
        rects.$item52 = { top: 52 * 72, height: 216 };
      }
    });
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        listener = nextListener;
        return () => undefined;
      }
    });

    for (let index = 0; index < 700; index += 1) {
      rects[`$item${index}`] = { top: index * 72, height: 72 };
    }
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    mockTimelineRects(rects, { top: 0, height: 600 }, scrollContainerRef);

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={() => undefined}
        onScrollDiagnosticsChange={onScrollDiagnosticsChange}
        listRefCallback={(element) => {
          scrollContainerRef.current =
            element?.closest<HTMLElement>("[data-testid=timeline-view]") ?? null;
        }}
      />
    );

    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: messages(700)
          }
        }
      });
    });

    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollTop", {
      value: 3000,
      writable: true,
      configurable: true
    });
    Object.defineProperty(timeline, "scrollHeight", {
      value: 700 * 72,
      writable: true,
      configurable: true
    });
    Object.defineProperty(timeline, "clientHeight", {
      value: 600,
      writable: true,
      configurable: true
    });
    const baselineDiagnostics = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    const baselineHeightModelCommits = baselineDiagnostics?.heightModelCommits ?? 0;
    baselineMeasurementFlushes = baselineDiagnostics?.measurementFlushes ?? 0;

    fireEvent.wheel(timeline, { deltaY: 40 });
    fireEvent.scroll(timeline);

    rects.$item50 = { top: 50 * 72, height: 180 };
    fireEvent.scroll(timeline);
    act(() => {
      vi.advanceTimersByTime(16);
    });
    await act(async () => {
      await Promise.resolve();
    });

    const activeDiagnostics = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    expect(activeDiagnostics.pendingMeasuredRows).toBeGreaterThan(0);

    mutateAfterFlush = true;
    act(() => {
      vi.advanceTimersByTime(100);
    });

    const idleDiagnostics = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    expect(idleDiagnostics.measurementFlushes - baselineMeasurementFlushes).toBe(1);
    expect(idleDiagnostics.heightModelCommits - baselineHeightModelCommits).toBe(2);
  });

  it("does not defer measurements from a programmatic scroll echo", async () => {
    vi.useFakeTimers();
    let listener: ((payload: CoreEventPayload) => void) | null = null;
    const onScrollDiagnosticsChange = vi.fn();
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        listener = nextListener;
        return () => undefined;
      }
    });

    const rects: Record<string, { top: number; height: number }> = {};
    for (let index = 0; index < 700; index += 1) {
      rects[`$item${index}`] = { top: index * 72, height: 72 };
    }
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    mockTimelineRects(rects, { top: 0, height: 600 }, scrollContainerRef);

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={() => undefined}
        onScrollDiagnosticsChange={onScrollDiagnosticsChange}
        listRefCallback={(element) => {
          scrollContainerRef.current =
            element?.closest<HTMLElement>("[data-testid=timeline-view]") ?? null;
        }}
      />
    );

    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: messages(700)
          }
        }
      });
      listener?.({
        kind: "Timeline",
        event: {
          NavigationUpdated: {
            key: KEY,
            snapshot: navigationSnapshot({
              unread_event_count: 1,
              newer_event_count: 2,
              can_jump_to_bottom: true
            })
          }
        }
      });
    });

    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollTop", {
      value: 3000,
      writable: true,
      configurable: true
    });
    Object.defineProperty(timeline, "scrollHeight", {
      value: 700 * 72,
      writable: true,
      configurable: true
    });
    Object.defineProperty(timeline, "clientHeight", {
      value: 600,
      writable: true,
      configurable: true
    });
    await act(async () => {
      await Promise.resolve();
    });
    const jumpToBottomButton = screen.getByRole("button", { name: /Jump to bottom/i });
    const baselineDiagnostics = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    const baselineHeightModelCommits = baselineDiagnostics?.heightModelCommits ?? 0;
    const baselineMeasurementFlushes = baselineDiagnostics?.measurementFlushes ?? 0;

    fireEvent.click(jumpToBottomButton);
    rects.$item699 = { top: 699 * 72, height: 180 };
    fireEvent.scroll(timeline);
    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 2,
            items: messages(700)
          }
        }
      });
    });
    await act(async () => {
      await Promise.resolve();
    });

    const afterEchoDiagnostics = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    expect(afterEchoDiagnostics.pendingMeasuredRows).toBe(0);
    expect(afterEchoDiagnostics.heightModelCommits - baselineHeightModelCommits).toBeGreaterThan(0);

    act(() => {
      vi.advanceTimersByTime(100);
    });

    const idleDiagnostics = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    expect(idleDiagnostics.measurementFlushes - baselineMeasurementFlushes).toBe(0);
  });

  it("classifies programmatic scroll writes by reason and suppresses their scroll echo", async () => {
    const onScrollDiagnosticsChange = vi.fn();
    let listener: ((payload: CoreEventPayload) => void) | null = null;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        listener = nextListener;
        return () => undefined;
      }
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={() => undefined}
        onScrollDiagnosticsChange={onScrollDiagnosticsChange}
      />
    );

    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollTop", {
      value: 1000,
      writable: true,
      configurable: true
    });
    Object.defineProperty(timeline, "scrollHeight", {
      value: 700 * 72,
      configurable: true
    });
    Object.defineProperty(timeline, "clientHeight", {
      value: 600,
      configurable: true
    });

    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: Array.from({ length: 700 }, (_, index) =>
              message(`$item${index}`, `message ${index}`)
            )
          }
        }
      });
    });

    timeline.scrollTop = 1000;
    fireEvent.wheel(timeline, { deltaY: -40 });
    fireEvent.scroll(timeline);
    onScrollDiagnosticsChange.mockClear();

    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          NavigationUpdated: {
            key: KEY,
            snapshot: navigationSnapshot({
              can_jump_to_bottom: true,
              newer_event_count: 4
            })
          }
        }
      });
    });

    fireEvent.click(screen.getByRole("button", { name: /Jump to bottom/ }));
    fireEvent.scroll(timeline);

    const diagnostics = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    expect(diagnostics.scrollWrites.jumpToBottom).toBe(1);
    expect(diagnostics.latestFrame?.userInputPending).toBe(false);
  });

  it("drops stale pending measurements after same timeline ItemsUpdated reset", async () => {
    vi.useFakeTimers();
    let listener: ((payload: CoreEventPayload) => void) | null = null;
    const onScrollDiagnosticsChange = vi.fn();
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        listener = nextListener;
        return () => undefined;
      }
    });

    const rects: Record<string, { top: number; height: number }> = {};
    for (let index = 0; index < 700; index += 1) {
      rects[`$item${index}`] = { top: index * 72, height: 72 };
    }
    for (let index = 0; index < 20; index += 1) {
      rects[`$reset${index}`] = { top: index * 72, height: 72 };
    }
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    mockTimelineRects(rects, { top: 0, height: 600 }, scrollContainerRef);

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={() => undefined}
        onScrollDiagnosticsChange={onScrollDiagnosticsChange}
        listRefCallback={(element) => {
          scrollContainerRef.current =
            element?.closest<HTMLElement>("[data-testid=timeline-view]") ?? null;
        }}
      />
    );

    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: messages(700)
          }
        }
      });
    });

    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollTop", {
      value: 3000,
      writable: true,
      configurable: true
    });
    Object.defineProperty(timeline, "scrollHeight", {
      value: 700 * 72,
      writable: true,
      configurable: true
    });
    Object.defineProperty(timeline, "clientHeight", {
      value: 600,
      writable: true,
      configurable: true
    });

    fireEvent.wheel(timeline, { deltaY: 40 });
    fireEvent.scroll(timeline);
    rects.$item50 = { top: 50 * 72, height: 180 };
    fireEvent.scroll(timeline);
    act(() => {
      vi.advanceTimersByTime(16);
    });
    await act(async () => {
      await Promise.resolve();
    });
    expect(onScrollDiagnosticsChange.mock.calls.at(-1)?.[0].pendingMeasuredRows).toBeGreaterThan(0);

    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key: KEY,
            generation: 1,
            batch_id: 2,
            diffs: [
              {
                Reset: {
                  items: messages(20, "$reset")
                }
              }
            ]
          }
        }
      });
    });
    expect(timeline.getAttribute("data-total-items")).toBe("20");
    const resetDiagnostics = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    const resetHeightModelCommits = resetDiagnostics?.heightModelCommits ?? 0;
    const resetMeasurementFlushes = resetDiagnostics?.measurementFlushes ?? 0;

    act(() => {
      vi.advanceTimersByTime(100);
    });

    const idleDiagnostics = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    expect(idleDiagnostics.measurementFlushes - resetMeasurementFlushes).toBe(0);
    expect(idleDiagnostics.heightModelCommits - resetHeightModelCommits).toBe(0);
    expect(idleDiagnostics.pendingMeasuredRows).toBe(0);
  });

  it("drops pending scroll frame diagnostics after the timeline key changes", async () => {
    const onScrollDiagnosticsChange = vi.fn();
    let listener: ((payload: CoreEventPayload) => void) | null = null;
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
    const flushFrames = () => {
      const queued = [...frames.entries()];
      frames.clear();
      for (const [, callback] of queued) {
        callback(0);
      }
    };
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        listener = nextListener;
        return () => undefined;
      }
    });
    const threadKey = threadTimelineKey(
      "@alice:example.invalid",
      "!room:example.invalid",
      "$root:example.invalid"
    );
    const renderView = (timelineKey = KEY) => (
      <TimelineView
        timelineKey={timelineKey}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={() => undefined}
        onScrollDiagnosticsChange={onScrollDiagnosticsChange}
      />
    );

    const { rerender } = render(renderView());
    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperty(timeline, "clientHeight", { value: 500, configurable: true });
    Object.defineProperty(timeline, "scrollHeight", { value: 700 * 72, configurable: true });
    Object.defineProperty(timeline, "scrollTop", {
      value: 20_000,
      writable: true,
      configurable: true
    });

    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: Array.from({ length: 700 }, (_, index) =>
              message(`$item${index}`, `message ${index}`)
            )
          }
        }
      });
    });

    await waitFor(() => expect(timeline.getAttribute("data-virtualized")).toBe("true"));
    act(() => {
      flushFrames();
    });
    onScrollDiagnosticsChange.mockClear();

    act(() => {
      fireEvent.wheel(timeline, { deltaY: 4 });
      timeline.scrollTop += 4;
      fireEvent.scroll(timeline);
    });
    expect(frames.size).toBe(1);

    act(() => {
      rerender(renderView(threadKey));
    });
    act(() => {
      flushFrames();
    });

    const activeFrames = onScrollDiagnosticsChange.mock.calls
      .map(([diagnostics]) => diagnostics.latestFrame)
      .filter((frame) => frame?.scrollActivity === "active");
    expect(activeFrames).toEqual([]);
  });

  it("drops pending scroll frame diagnostics after same timeline items reset", async () => {
    const onScrollDiagnosticsChange = vi.fn();
    let listener: ((payload: CoreEventPayload) => void) | null = null;
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
    const flushFrames = () => {
      const queued = [...frames.entries()];
      frames.clear();
      for (const [, callback] of queued) {
        callback(0);
      }
    };
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        listener = nextListener;
        return () => undefined;
      }
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={() => undefined}
        onScrollDiagnosticsChange={onScrollDiagnosticsChange}
      />
    );
    const timeline = await screen.findByTestId("timeline-view");
    Object.defineProperty(timeline, "clientHeight", { value: 500, configurable: true });
    Object.defineProperty(timeline, "scrollHeight", { value: 700 * 72, configurable: true });
    Object.defineProperty(timeline, "scrollTop", {
      value: 20_000,
      writable: true,
      configurable: true
    });

    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: Array.from({ length: 700 }, (_, index) =>
              message(`$item${index}`, `message ${index}`)
            )
          }
        }
      });
    });

    await waitFor(() => expect(timeline.getAttribute("data-virtualized")).toBe("true"));
    act(() => {
      flushFrames();
    });
    onScrollDiagnosticsChange.mockClear();

    act(() => {
      fireEvent.wheel(timeline, { deltaY: 4 });
      timeline.scrollTop += 4;
      fireEvent.scroll(timeline);
    });
    expect(frames.size).toBe(1);

    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 2,
            items: Array.from({ length: 20 }, (_, index) =>
              message(`$reset${index}`, `reset message ${index}`)
            )
          }
        }
      });
    });
    await waitFor(() => {
      expect(timeline.getAttribute("data-timeline-generation")).toBe("2");
      expect(timeline.getAttribute("data-total-items")).toBe("20");
    });
    onScrollDiagnosticsChange.mockClear();

    act(() => {
      flushFrames();
    });

    const activeFrames = onScrollDiagnosticsChange.mock.calls
      .map(([diagnostics]) => diagnostics.latestFrame)
      .filter((frame) => frame?.scrollActivity === "active");
    expect(activeFrames).toEqual([]);
  });

  it("cancels delayed programmatic scroll follow-ups after timeline key changes", async () => {
    vi.useFakeTimers();
    const onScrollDiagnosticsChange = vi.fn();
    let listener: ((payload: CoreEventPayload) => void) | null = null;
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
    const flushFrames = () => {
      const queued = [...frames.entries()];
      frames.clear();
      for (const [, callback] of queued) {
        callback(0);
      }
    };
    const threadKey = threadTimelineKey(
      "@alice:example.invalid",
      "!room:example.invalid",
      "$root:example.invalid"
    );
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        listener = nextListener;
        return () => undefined;
      }
    });
    const renderView = (timelineKey: typeof KEY | typeof threadKey) => (
      <TimelineView
        timelineKey={timelineKey}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={() => undefined}
        onScrollDiagnosticsChange={onScrollDiagnosticsChange}
      />
    );

    const { rerender } = render(renderView(KEY));
    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollTop", {
      value: 1000,
      writable: true,
      configurable: true
    });
    Object.defineProperty(timeline, "scrollHeight", {
      value: 700 * 72,
      configurable: true
    });
    Object.defineProperty(timeline, "clientHeight", {
      value: 600,
      configurable: true
    });

    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: Array.from({ length: 700 }, (_, index) =>
              message(`$item${index}`, `message ${index}`)
            )
          }
        }
      });
    });
    act(() => {
      flushFrames();
    });
    timeline.scrollTop = 1000;
    fireEvent.wheel(timeline, { deltaY: -40 });
    fireEvent.scroll(timeline);
    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          NavigationUpdated: {
            key: KEY,
            snapshot: navigationSnapshot({
              can_jump_to_bottom: true,
              newer_event_count: 4
            })
          }
        }
      });
    });

    fireEvent.click(screen.getByRole("button", { name: /Jump to bottom/ }));
    expect(frames.size).toBeGreaterThan(0);

    act(() => {
      rerender(renderView(threadKey));
    });
    timeline.scrollTop = 1000;
    onScrollDiagnosticsChange.mockClear();

    act(() => {
      flushFrames();
    });

    const jumpWritesAfterKeyChange = onScrollDiagnosticsChange.mock.calls
      .map(([diagnostics]) => diagnostics.scrollWrites.jumpToBottom)
      .filter((count) => count > 0);
    expect(jumpWritesAfterKeyChange).toEqual([]);
  });

  it("does not re-emit scroll diagnostics from parent state commits", async () => {
    const onScrollDiagnosticsChange = vi.fn();

    function Parent() {
      const [, setDiagnostics] = useState<unknown>(null);
      return (
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({})}
          onReply={() => undefined}
          onScrollDiagnosticsChange={(diagnostics) => {
            onScrollDiagnosticsChange(diagnostics);
            if (onScrollDiagnosticsChange.mock.calls.length <= 4) {
              setDiagnostics(diagnostics);
            }
          }}
        />
      );
    }

    render(<Parent />);

    await waitFor(() => expect(onScrollDiagnosticsChange).toHaveBeenCalled());
    await act(async () => undefined);

    expect(onScrollDiagnosticsChange.mock.calls.length).toBeLessThanOrEqual(2);
  });

  it("paginates older history when the user scrolls to the top even if prefetch is disabled", async () => {
    const scheduler = createManualTimelineViewportScheduler();
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const paginateBackwards = vi.fn(async () => undefined);
    const onDiagnosticLogEntry = vi.fn();
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      paginateBackwards
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        autoLoadOlderMessages={false}
        onReply={vi.fn()}
        onDiagnosticLogEntry={onDiagnosticLogEntry}
        viewportScheduler={scheduler}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [message("$latest", "Latest")]
        }
      }
    });

    const timeline = await screen.findByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollTop", {
      value: 0,
      writable: true,
      configurable: true
    });
    fireEvent.wheel(timeline, { deltaY: -120 });
    fireEvent.scroll(timeline);
    act(() => scheduler.flushAll());

    expect(paginateBackwards).toHaveBeenCalledWith(KEY);

    emit({
      kind: "Timeline",
      event: {
        PaginationStateChanged: {
          request_id: null,
          key: KEY,
          direction: "Backward",
          state: "Idle"
        }
      }
    });

    await waitFor(() => {
      const backfillMessages = onDiagnosticLogEntry.mock.calls
        .map(([entry]) => entry)
        .filter((entry) => entry.source === "timeline.backfill")
        .map((entry) => entry.message);
      expect(backfillMessages[0]).toContain("stage=request trigger=scroll");
      expect(backfillMessages[0]).toContain("threshold_px=0");
      expect(backfillMessages).toEqual(
        expect.arrayContaining([expect.stringContaining("stage=complete reason=pagination_idle")])
      );
    });
  });

  it("does not treat a programmatic top scroll as explicit demand when prefetch is disabled", async () => {
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
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        autoLoadOlderMessages={false}
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
            items: [message("$latest", "Latest")]
          }
        }
      });
    });

    const timeline = await screen.findByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollHeight", { value: 1_200, configurable: true });
    Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
    Object.defineProperty(timeline, "scrollTop", {
      value: 0,
      writable: true,
      configurable: true
    });

    fireEvent.scroll(timeline);
    await act(async () => Promise.resolve());

    expect(paginateBackwards).not.toHaveBeenCalled();
  });

  it.each([
    {
      projection: "prepend",
      diffs: [{ PushFront: { item: message("$older", "Older") } }]
    },
    {
      projection: "reset",
      diffs: [
        {
          Reset: {
            items: [message("$older", "Older"), message("$latest", "Latest")]
          }
        }
      ]
    }
  ] satisfies Array<{ projection: string; diffs: TimelineDiff[] }>) (
    "keeps a backfill request active through $projection until pagination reaches a terminal state",
    async ({ diffs }) => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const paginateBackwards = vi.fn(async () => undefined);
    const onDiagnosticLogEntry = vi.fn();
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      paginateBackwards
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        autoLoadOlderMessages
        onReply={vi.fn()}
        onDiagnosticLogEntry={onDiagnosticLogEntry}
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
            items: [message("$latest", "Latest")]
          }
        }
      });
    });

    const timeline = await screen.findByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollHeight", { value: 2_000, configurable: true });
    Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
    Object.defineProperty(timeline, "scrollTop", {
      value: 0,
      writable: true,
      configurable: true
    });
    fireEvent.wheel(timeline, { deltaY: -120 });
    fireEvent.scroll(timeline);

    await waitFor(() => expect(paginateBackwards).toHaveBeenCalledTimes(1));

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key: KEY,
            generation: 1,
            batch_id: 2,
            diffs
          }
        }
      });
    });
    timeline.scrollTop = 0;
    fireEvent.wheel(timeline, { deltaY: -120 });
    fireEvent.scroll(timeline);
    await act(async () => Promise.resolve());

    expect(paginateBackwards).toHaveBeenCalledTimes(1);
    const evaluationMessages = onDiagnosticLogEntry.mock.calls
      .map(([entry]) => entry)
      .filter((entry) => entry.source === "timeline.backfill_evaluation")
      .map((entry) => entry.message);
    expect(evaluationMessages).toEqual(
      expect.arrayContaining([
        expect.stringMatching(/decision=blocked .*request_epoch=1/)
      ])
    );
    expect(evaluationMessages.join("\n")).not.toMatch(/!room:|\$latest|\$older|@alice|@bob/);

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          PaginationStateChanged: {
            request_id: null,
            key: KEY,
            direction: "Backward",
            state: "Idle"
          }
        }
      });
    });

    await waitFor(() => expect(paginateBackwards).toHaveBeenCalledTimes(2));
    }
  );

  it.each([
    {
      projection: "prepend",
      diffs: [{ PushFront: { item: message("$older", "Older") } }]
    },
    {
      projection: "reset",
      diffs: [
        {
          Reset: {
            items: [message("$older", "Older"), message("$latest", "Latest")]
          }
        }
      ]
    }
  ] satisfies Array<{ projection: string; diffs: TimelineDiff[] }>) (
    "keeps a terminal-first backfill request active until its $projection settles",
    async ({ diffs }) => {
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
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        autoLoadOlderMessages
        onReply={vi.fn()}
      />
    );

    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollHeight", { value: 320, configurable: true });
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
            items: [message("$latest", "Latest")]
          }
        }
      });
    });
    await waitFor(() => expect(paginateBackwards).toHaveBeenCalledTimes(1));
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          PaginationStateChanged: {
            request_id: null,
            key: KEY,
            direction: "Backward",
            state: "Paginating"
          }
        }
      });
    });
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          PaginationStateChanged: {
            request_id: null,
            key: KEY,
            direction: "Backward",
            state: "Idle"
          }
        }
      });
    });
    await act(async () => Promise.resolve());
    expect(paginateBackwards).toHaveBeenCalledTimes(1);

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key: KEY,
            generation: 1,
            batch_id: 2,
            diffs
          }
        }
      });
    });

    await waitFor(() => expect(paginateBackwards).toHaveBeenCalledTimes(2));
    }
  );

  it("keeps an unaccepted Idle fenced through gap projection until repair release", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const paginateBackwards = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      paginateBackwards
    });
    const renderView = (autoLoadOlderMessages: boolean) => (
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        autoLoadOlderMessages={autoLoadOlderMessages}
        onReply={vi.fn()}
      />
    );
    render(renderView(true));

    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollHeight", { value: 320, configurable: true });
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
            items: [message("$latest", "Latest")]
          }
        }
      });
    });
    await waitFor(() => expect(paginateBackwards).toHaveBeenCalledTimes(1));

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          PaginationStateChanged: {
            request_id: null,
            key: KEY,
            direction: "Backward",
            state: "Idle"
          }
        }
      });
    });
    await act(async () => Promise.resolve());
    expect(paginateBackwards).toHaveBeenCalledTimes(1);

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          GapPositionsUpdated: {
            key: KEY,
            actor_generation: 1,
            generation: 2,
            positions: []
          }
        }
      });
    });
    await act(async () => Promise.resolve());
    expect(paginateBackwards).toHaveBeenCalledTimes(1);

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          GapRepairReleased: {
            key: KEY,
            actor_generation: 1,
            generation: 3
          }
        }
      });
    });
    await waitFor(() => expect(paginateBackwards).toHaveBeenCalledTimes(2));
  });

  it("retries after gap repair releases an Idle request rejected during repair", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const paginateBackwards = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      paginateBackwards
    });
    const renderView = (autoLoadOlderMessages: boolean) => (
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        autoLoadOlderMessages={autoLoadOlderMessages}
        onReply={vi.fn()}
      />
    );
    const { rerender } = render(renderView(false));

    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollHeight", { value: 320, configurable: true });
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
            items: [message("$latest", "Latest")]
          }
        }
      });
    });
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          GapPositionsUpdated: {
            key: KEY,
            actor_generation: 1,
            generation: 2,
            positions: []
          }
        }
      });
    });
    await act(async () => Promise.resolve());
    expect(paginateBackwards).not.toHaveBeenCalled();

    rerender(renderView(true));
    await waitFor(() => expect(paginateBackwards).toHaveBeenCalledTimes(1));

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          PaginationStateChanged: {
            request_id: null,
            key: KEY,
            direction: "Backward",
            state: "Idle",
            prepend_expected: null
          }
        }
      });
    });
    await act(async () => Promise.resolve());
    expect(paginateBackwards).toHaveBeenCalledTimes(1);

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          GapRepairReleased: {
            key: KEY,
            actor_generation: 1,
            generation: 3
          }
        }
      } as CoreEventPayload);
    });

    await waitFor(() => expect(paginateBackwards).toHaveBeenCalledTimes(2));
  });

  it("continues after an accepted Idle page with no prepend projection", async () => {
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
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        autoLoadOlderMessages
        onReply={vi.fn()}
      />
    );

    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollHeight", { value: 320, configurable: true });
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
            items: [message("$latest", "Latest")]
          }
        }
      });
    });
    await waitFor(() => expect(paginateBackwards).toHaveBeenCalledTimes(1));

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          PaginationStateChanged: {
            request_id: null,
            key: KEY,
            direction: "Backward",
            state: "Paginating",
            prepend_expected: null
          }
        }
      });
    });
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          PaginationStateChanged: {
            request_id: null,
            key: KEY,
            direction: "Backward",
            state: "Idle",
            prepend_expected: false
          }
        }
      });
    });

    await waitFor(() => expect(paginateBackwards).toHaveBeenCalledTimes(2));
  });

  it("waits for a new state transition after backfill transport rejection", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const paginateBackwards = vi
      .fn<() => Promise<void>>()
      .mockRejectedValueOnce(new Error("transport rejected"))
      .mockResolvedValue(undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      paginateBackwards
    });

    const renderView = (autoLoadOlderMessages: boolean) => (
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        autoLoadOlderMessages={autoLoadOlderMessages}
        onReply={vi.fn()}
      />
    );
    const { rerender } = render(renderView(true));

    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollHeight", { value: 320, configurable: true });
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
            items: [message("$latest", "Latest")]
          }
        }
      });
    });

    await waitFor(() => expect(paginateBackwards).toHaveBeenCalledTimes(1));
    await act(async () => Promise.resolve());
    expect(paginateBackwards).toHaveBeenCalledTimes(1);

    act(() => rerender(renderView(false)));
    act(() => rerender(renderView(true)));
    await waitFor(() => expect(paginateBackwards).toHaveBeenCalledTimes(2));
  });

  it("backfills an underfilled room timeline after short initial items arrive", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const paginateBackwards = vi.fn(async () => undefined);
    const onDiagnosticLogEntry = vi.fn();
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      paginateBackwards
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        autoLoadOlderMessages={true}
        onReply={vi.fn()}
        onDiagnosticLogEntry={onDiagnosticLogEntry}
      />
    );

    const timeline = await screen.findByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollHeight", { value: 320, configurable: true });
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
            items: [message("$latest", "Latest")]
          }
        }
      });
    });

    await waitFor(() => {
      const underfilledLogs = onDiagnosticLogEntry.mock.calls
        .map(([entry]) => entry)
        .filter((entry) => entry.source === "timeline.backfill")
        .map((entry) => entry.message)
        .filter((message) => message.includes("trigger=underfilled_initial"));
      expect(underfilledLogs).toEqual([
        expect.stringContaining("stage=request trigger=underfilled_initial")
      ]);
      expect(underfilledLogs[0]).toContain("items=1");
      expect(underfilledLogs[0]).toContain("scroll_height_px=320");
      expect(underfilledLogs[0]).toContain("client_height_px=600");
      expect(underfilledLogs[0]).toContain("overflow_px=0");
      expect(underfilledLogs[0]).toContain("auto_load=true");
      expect(underfilledLogs[0]).toContain("state=Idle");
    });
    expect(paginateBackwards).toHaveBeenCalledWith(KEY);
    expect(paginateBackwards).toHaveBeenCalledTimes(1);
  });

  it("re-evaluates an underfilled timeline when automatic loading is enabled", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const paginateBackwards = vi.fn(async () => undefined);
    const onDiagnosticLogEntry = vi.fn();
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      paginateBackwards
    });
    const renderView = (autoLoadOlderMessages: boolean) => (
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        autoLoadOlderMessages={autoLoadOlderMessages}
        onReply={vi.fn()}
        onDiagnosticLogEntry={onDiagnosticLogEntry}
      />
    );
    const { rerender } = render(renderView(false));
    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollHeight", { value: 320, configurable: true });
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
            items: [message("$latest", "Latest")]
          }
        }
      });
    });
    expect(paginateBackwards).not.toHaveBeenCalled();

    act(() => rerender(renderView(true)));

    await waitFor(() => expect(paginateBackwards).toHaveBeenCalledTimes(1));
    expect(
      onDiagnosticLogEntry.mock.calls
        .map(([entry]) => entry)
        .filter((entry) => entry.source === "timeline.backfill_evaluation")
        .map((entry) => entry.message)
    ).toEqual(expect.arrayContaining([expect.stringContaining("trigger=setting_changed")]));
  });

  it("does not backfill a 3,234-item virtual timeline from transient DOM underfill", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const paginateBackwards = vi.fn(async () => undefined);
    const onDiagnosticLogEntry = vi.fn();
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      paginateBackwards
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        autoLoadOlderMessages={true}
        onReply={vi.fn()}
        onDiagnosticLogEntry={onDiagnosticLogEntry}
      />
    );
    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollHeight", { value: 367, configurable: true });
    Object.defineProperty(timeline, "clientHeight", { value: 367, configurable: true });
    Object.defineProperty(timeline, "scrollTop", { value: 0, writable: true, configurable: true });

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: messages(3_234, "$diagnostic")
          }
        }
      });
    });

    await waitFor(() => {
      expect(screen.getByTestId("timeline-view").getAttribute("data-virtualized")).toBe("true");
    });
    expect(paginateBackwards).not.toHaveBeenCalled();
    expect(
      onDiagnosticLogEntry.mock.calls
        .map(([entry]) => entry.message)
        .filter((message) => message.includes("trigger=underfilled_initial"))
    ).toEqual([]);
  });

  it("auto-backfills after an in-session room anchor settles without a user scroll", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const updateScrollAnchor = vi.fn(async () => undefined);
    const paginateBackwards = vi.fn(async () => undefined);
    const onDiagnosticLogEntry = vi.fn();
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      paginateBackwards,
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
    expect(paginateBackwards).not.toHaveBeenCalled();

    unmount();
    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        autoLoadOlderMessages
        onReply={vi.fn()}
        onDiagnosticLogEntry={onDiagnosticLogEntry}
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
            message("$anchor:example.invalid", "Anchor"),
            message("$after:example.invalid", "After")
          ]
        }
      }
    });

    await waitFor(() => {
      expect(restoredTimeline.scrollTop).toBe(48);
    });
    await act(async () => {
      await new Promise<void>((resolve) => {
        requestAnimationFrame(() => resolve());
      });
    });

    await waitFor(() => {
      expect(paginateBackwards).toHaveBeenCalledWith(KEY);
    });
    expect(paginateBackwards).toHaveBeenCalledTimes(1);
  });
});
