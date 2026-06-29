// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { openExternalHttpUrl } from "../domain/externalLinks";

vi.mock("../domain/externalLinks", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../domain/externalLinks")>()),
  openExternalHttpUrl: vi.fn(async () => undefined)
}));

import {
  roomTimelineKey,
  threadTimelineKey,
  type CoreEventPayload,
  type TimelineItem,
  type TimelineMessageSource
} from "../domain/coreEvents";
import { setActiveLocaleProfile } from "../i18n/messages";
import {
  applyTimelineEvent,
  createTimelineStore,
  type TimelineStoreState
} from "../domain/timelineStore";
import { TimelineStoreContext } from "./timelineStoreContext";
import {
  MessageSourceDialog,
  TimelineView,
  clearTimelineViewportSessionMemoryForTests,
  type TimelineTransport
} from "./TimelineView";
import type { LiveSignalsState } from "../domain/types";

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  setActiveLocaleProfile("en", "none");
  vi.useRealTimers();
  vi.restoreAllMocks();
});

const KEY = roomTimelineKey("@alice:example.invalid", "!room:example.invalid");

function message(eventId: string, body: string): TimelineItem {
  return {
    id: { Event: { event_id: eventId } },
    sender: "@bob:example.invalid",
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
    reactions: []
  };
}

function imageMessage(eventId: string, encrypted = false): TimelineItem {
  return {
    ...message(eventId, "Image body"),
    media: {
      kind: "Image",
      filename: "photo.png",
      source: {
        mxc_uri: "mxc://example.invalid/photo",
        encrypted,
        encryption_version: encrypted ? "v2" : null
      },
      mimetype: "image/png",
      size: 416_768,
      width: 2048,
      height: 1188,
      thumbnail: null
    }
  };
}

function baseTransport(
  overrides: Partial<TimelineTransport>
): TimelineTransport {
  return {
    listenCoreEvents: () => () => undefined,
    paginateBackwards: async () => undefined,
    sendReaction: async () => undefined,
    retrySend: async () => undefined,
    cancelSend: async () => undefined,
    redactReaction: async () => undefined,
    sendReadReceipt: async () => undefined,
    setFullyRead: async () => undefined,
    setTyping: async () => undefined,
    editMessage: async () => undefined,
    redactMessage: async () => undefined,
    pinEvent: async () => undefined,
    unpinEvent: async () => undefined,
    downloadMedia: async () => undefined,
    downloadAvatarThumbnail: async () => undefined,
    loadMessageSource: async () => undefined,
    requestRoomKey: async () => undefined,
    forwardMessage: async () => undefined,
    loadLinkPreviews: async () => undefined,
    hideLinkPreview: async () => undefined,
    updateScrollAnchor: async () => undefined,
    ...overrides
  };
}

function mockTimelineRects(
  rects: Record<string, { top: number; height: number }>,
  container: { top?: number; height?: number } = {},
  scrollContainerRef?: { current: HTMLElement | null }
) {
  return vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement
  ) {
    const eventId =
      this.getAttribute("data-event-id") ??
      this.getAttribute("data-frame-item-id") ??
      this.getAttribute("data-item-id");
    const testId = this.getAttribute("data-testid");
    const scrollTop = scrollContainerRef?.current?.scrollTop ?? 0;
    const top =
      testId === "timeline-view"
        ? container.top ?? 0
        : (rects[eventId ?? ""]?.top ?? 0) - scrollTop;
    const height =
      testId === "timeline-view"
        ? container.height ?? 600
        : rects[eventId ?? ""]?.height ?? 0;
    const bottom = top + height;
    return {
      x: 0,
      y: top,
      top,
      left: 0,
      right: 0,
      width: 0,
      height,
      bottom,
      toJSON: () => ({})
    } as DOMRect;
  });
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
  it("ensures the timeline subscription after registering the CoreEvent listener", async () => {
    const calls: string[] = [];
    let listener: ((payload: CoreEventPayload) => void) | null = null;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        calls.push("listen");
        listener = nextListener;
        return () => undefined;
      },
      async ensureSubscribed(timelineKey) {
        calls.push("ensure");
        expect(timelineKey).toEqual(KEY);
        listener?.({
          kind: "Timeline",
          event: {
            InitialItems: {
              request_id: null,
              key: KEY,
              generation: 1,
              items: [message("$latest", "Latest after listener")]
            }
          }
        });
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

    await waitFor(() => {
      expect(screen.getByText("Latest after listener")).toBeTruthy();
    });
    expect(calls).toEqual(["listen", "ensure"]);
  });

  it("renders from a prepopulated App-level store while keeping view-local event handling", async () => {
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [message("$app-store:example.invalid", "From app store")]
      }
    });
    const ensureSubscribed = vi.fn().mockResolvedValue(undefined);
    const setStore = vi.fn();
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const listenCoreEvents = vi.fn((nextListener: (payload: CoreEventPayload) => void) => {
      emit = nextListener;
      return () => undefined;
    });
    const transport = baseTransport({
      listenCoreEvents,
      ensureSubscribed
    });

    render(
      <TimelineStoreContext.Provider value={{ store, setStore }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={transport}
          onReply={vi.fn()}
        />
      </TimelineStoreContext.Provider>
    );

    expect(await screen.findByText("From app store")).toBeTruthy();
    expect(listenCoreEvents).toHaveBeenCalledTimes(1);
    expect(ensureSubscribed).toHaveBeenCalledWith(KEY);
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          MessageSourceLoaded: {
            request_id: { connection_id: 1, sequence: 1 },
            key: KEY,
            source: {
              event_id: "$source:example.invalid",
              sender: "@alice:example.invalid",
              timestamp_ms: 1_800_000_000_000,
              body: "source body",
              in_reply_to_event_id: null,
              thread_root: null,
              is_redacted: false,
              is_edited: false,
              has_media: false,
              original_json: {
                type: "m.room.message",
                content: { body: "source body", msgtype: "m.text" }
              }
            }
          }
        }
      });
    });
    expect(await screen.findByText("$source:example.invalid")).toBeTruthy();
    expect(setStore).not.toHaveBeenCalled();
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
    expect(idleDiagnostics.heightModelCommits - baselineHeightModelCommits).toBe(1);
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
    Object.defineProperty(timeline, "scrollTop", { value: 0, configurable: true });
    fireEvent.scroll(timeline);

    await waitFor(() => {
      expect(paginateBackwards).toHaveBeenCalledWith(KEY);
    });
  });

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

  it("renders read receipts as a compact avatar stack without an inline text label", async () => {
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
        liveSignals={{
          presence: {},
          rooms: {
            "!room:example.invalid": {
              fully_read_event_id: null,
              typing_user_ids: [],
              receipts_by_event: {
                "$seen": {
                  total_count: 2,
                  overflow_count: 0,
                  readers: [
                    {
                      user_id: "@ken:example.invalid",
                      display_name: "Ken Inayoshi",
                      original_display_label: "Ken Inayoshi",
                      avatar: null,
                      timestamp_ms: null
                    },
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
            items: [message("$seen", "Seen message")]
          }
        }
      });
    });

    await waitFor(() => {
      const receipts = document.querySelector(".message-receipts");
      expect(receipts).not.toBeNull();
      expect(receipts?.textContent).toContain("KE");
      expect(receipts?.textContent).toContain("SA");
      expect(receipts?.textContent).not.toContain("Read by 2");
      expect(receipts?.getAttribute("aria-label")).toContain("Read by 2");
      expect(receipts?.getAttribute("title")).toBe("Ken Inayoshi\nSatoshi Terasaki");
    });
  });

  it("surfaces reaction senders in a hoverable tooltip using profile labels", async () => {
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
        profileUsers={{
          "@ken:example.invalid": {
            user_id: "@ken:example.invalid",
            display_name: "Ken Inayoshi",
            display_label: "Ken Inayoshi",
            original_display_label: "Ken Inayoshi",
            mention_search_terms: [],
            avatar: null
          },
          "@satoshi:example.invalid": {
            user_id: "@satoshi:example.invalid",
            display_name: "Satoshi Terasaki",
            display_label: "Satoshi Terasaki",
            original_display_label: "Satoshi Terasaki",
            mention_search_terms: [],
            avatar: null
          }
        }}
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
              {
                ...message("$reacted", "Reacted message"),
                reactions: [
                  {
                    key: "😢",
                    count: 2,
                    reacted_by_me: false,
                    my_reaction_event_id: null,
                    sender_preview: ["@ken:example.invalid", "@satoshi:example.invalid"]
                  }
                ]
              }
            ]
          }
        }
      });
    });

    await waitFor(() => {
      expect(screen.getByText("😢")).toBeTruthy();
      expect(screen.getByText("Ken Inayoshi and Satoshi Terasaki reacted with 😢")).toBeTruthy();
    });
  });

  it("places reactions and read receipts in one status row", async () => {
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
        liveSignals={{
          presence: {},
          rooms: {
            "!room:example.invalid": {
              fully_read_event_id: null,
              typing_user_ids: [],
              receipts_by_event: {
                "$reacted-seen": {
                  total_count: 1,
                  overflow_count: 0,
                  readers: [
                    {
                      user_id: "@ken:example.invalid",
                      display_name: "Ken Inayoshi",
                      original_display_label: "Ken Inayoshi",
                      avatar: null,
                      timestamp_ms: null
                    }
                  ]
                }
              }
            }
          }
        }}
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
              {
                ...message("$reacted-seen", "Reacted and seen"),
                reactions: [
                  {
                    key: "✈️",
                    count: 1,
                    reacted_by_me: false,
                    my_reaction_event_id: null,
                    sender_preview: ["@ken:example.invalid"]
                  }
                ]
              }
            ]
          }
        }
      });
    });

    await waitFor(() => {
      const reactions = document.querySelector(".message-reactions");
      const receipts = document.querySelector(".message-receipts");
      const statusRow = document.querySelector(".message-status-row");

      expect(reactions).not.toBeNull();
      expect(receipts).not.toBeNull();
      expect(statusRow).not.toBeNull();
      expect(reactions?.parentElement).toBe(statusRow);
      expect(receipts?.parentElement).toBe(statusRow);
      expect(Array.from(statusRow?.children ?? [])).toEqual([reactions, receipts]);
    });
  });

  it("automatically requests previews for encrypted image attachments", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const downloadMedia = vi.fn(async () => undefined);
    const transport = baseTransport({
      downloadMedia,
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
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [imageMessage("$encrypted-image", true)]
          }
        }
      });
    });

    await waitFor(() => {
      expect(downloadMedia).toHaveBeenCalledWith(
        "!room:example.invalid",
        "$encrypted-image"
      );
    });
  });

  it("does not request encrypted image previews for off-window initial virtualized items", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const downloadMedia = vi.fn(async () => undefined);
    const transport = baseTransport({
      downloadMedia,
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const items = Array.from({ length: 700 }, (_, index) =>
      index === 350
        ? imageMessage("$offscreen-image", true)
        : message(`$plain-${index}`, `Plain ${index}`)
    );

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
            items
          }
        }
      });
    });

    await waitFor(() => {
      const renderedItems = Number(
        screen.getByTestId("timeline-view").getAttribute("data-rendered-items")
      );
      expect(renderedItems).toBeGreaterThan(0);
      expect(renderedItems).toBeLessThan(items.length);
    });
    expect(downloadMedia).not.toHaveBeenCalledWith(
      "!room:example.invalid",
      "$offscreen-image"
    );
    expect(downloadMedia).not.toHaveBeenCalled();
  });

  it("renders ready image previews without technical metadata and opens the original source", async () => {
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
        mediaDownloads={{
          "$ready-image": {
            kind: "ready",
            source_url: "asset://localhost/original-photo.png",
            width: 2048,
            height: 1188,
            mime_type: "image/png"
          }
        }}
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
            items: [imageMessage("$ready-image", true)]
          }
        }
      });
    });

    await waitFor(() => {
      const image = screen.getByRole("img", { name: "photo.png" });
      const previewLink = image.closest("a");
      expect(previewLink?.getAttribute("href")).toBe("asset://localhost/original-photo.png");
      expect(previewLink?.getAttribute("target")).toBe("_blank");
      expect(previewLink?.hasAttribute("download")).toBe(false);
      const media = document.querySelector(".message-media");
      expect(media?.textContent).not.toContain("image/png");
      expect(media?.textContent).not.toContain("407 KB");
      expect(media?.textContent).not.toContain("2048x1188");
      expect(media?.textContent).not.toContain("Encrypted");
    });
  });

  it("requests visible sender avatar thumbnails that are not yet downloaded", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const downloadAvatarThumbnail = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      downloadAvatarThumbnail
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        enableAvatarThumbnailDownloads={true}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [
            {
              ...message("$avatar", "Avatar row"),
              sender_avatar: {
                mxc_uri: "mxc://matrix.org/avatar",
                thumbnail: { kind: "notRequested" }
              }
            }
          ]
        }
      }
    });

    await waitFor(() => {
      expect(downloadAvatarThumbnail).toHaveBeenCalledWith("mxc://matrix.org/avatar");
    });
    expect(downloadAvatarThumbnail).toHaveBeenCalledTimes(1);
  });

  it("emits timestamped avatar diagnostics for request, success, and retryable failure", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const downloadAvatarThumbnail = vi.fn(async () => undefined);
    const onDiagnosticLogEntry = vi.fn();
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      downloadAvatarThumbnail
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        onDiagnosticLogEntry={onDiagnosticLogEntry}
        enableAvatarThumbnailDownloads={true}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [
            {
              ...message("$avatar-retry", "Avatar row"),
              sender_avatar: {
                mxc_uri: "mxc://matrix.org/avatar-retry",
                thumbnail: { kind: "notRequested" }
              }
            }
          ]
        }
      }
    });

    await waitFor(() => {
      expect(downloadAvatarThumbnail).toHaveBeenCalledWith("mxc://matrix.org/avatar-retry");
    });
    expect(onDiagnosticLogEntry).toHaveBeenCalledWith(
      expect.objectContaining({
        source: "timeline.avatar",
        message: "avatar thumbnail request queued"
      })
    );

    emit({
      kind: "Account",
      event: {
        AvatarThumbnailDownloaded: {
          request_id: { connection_id: 1, sequence: 3 },
          mxc_uri: "mxc://matrix.org/avatar-retry",
          thumbnail: {
            kind: "failed",
            request_id: 3,
            failureKind: "network"
          }
        }
      }
    });

    await waitFor(() => {
      expect(downloadAvatarThumbnail).toHaveBeenCalledTimes(2);
    });
    expect(onDiagnosticLogEntry).toHaveBeenCalledWith(
      expect.objectContaining({
        source: "timeline.avatar",
        message: "avatar thumbnail failed kind=network"
      })
    );

    emit({
      kind: "Account",
      event: {
        AvatarThumbnailDownloaded: {
          request_id: { connection_id: 1, sequence: 4 },
          mxc_uri: "mxc://matrix.org/avatar-retry",
          thumbnail: {
            kind: "ready",
            source_url: "koushi-thumbnail://localhost/avatar/retry",
            width: null,
            height: null,
            mime_type: null
          }
        }
      }
    });

    await waitFor(() => {
      expect(onDiagnosticLogEntry).toHaveBeenCalledWith(
        expect.objectContaining({
          source: "timeline.avatar",
          message: "avatar thumbnail ready"
        })
      );
    });
    expect(onDiagnosticLogEntry.mock.calls.every(([entry]) => Number.isFinite(entry.timestampMs)))
      .toBe(true);
  });

  it("requests profile avatar thumbnails when the timeline item has no sender avatar", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const downloadAvatarThumbnail = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      downloadAvatarThumbnail
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        profileUsers={{
          "@bob:example.invalid": {
            user_id: "@bob:example.invalid",
            display_name: "Bob",
            display_label: "Bob",
            original_display_label: "Bob",
            mention_search_terms: ["bob"],
            avatar: {
              mxc_uri: "mxc://matrix.org/profile-avatar",
              thumbnail: { kind: "notRequested" }
            }
          }
        }}
        onReply={vi.fn()}
        enableAvatarThumbnailDownloads={true}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [message("$profile-avatar", "Profile avatar row")]
        }
      }
    });

    await waitFor(() => {
      expect(downloadAvatarThumbnail).toHaveBeenCalledWith("mxc://matrix.org/profile-avatar");
    });
    expect(downloadAvatarThumbnail).toHaveBeenCalledTimes(1);
  });

  it("does NOT call downloadAvatarThumbnail when enableAvatarThumbnailDownloads is explicitly false (kill-switch)", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const downloadAvatarThumbnail = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      downloadAvatarThumbnail
    });

    // Explicitly disable via the kill-switch prop (#116 Stage F1a: default is now ON).
    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        enableAvatarThumbnailDownloads={false}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [
            {
              ...message("$avatar-gated", "Avatar row (kill-switch off)"),
              sender_avatar: {
                mxc_uri: "mxc://matrix.org/avatar-gated",
                thumbnail: { kind: "notRequested" }
              }
            }
          ]
        }
      }
    });

    // Give React time to flush any effects that might fire.
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(downloadAvatarThumbnail).not.toHaveBeenCalled();
  });

  it("renders a downloaded sender avatar thumbnail from account events", async () => {
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

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [
            {
              ...message("$avatar-ready", "Avatar row"),
              sender_avatar: {
                mxc_uri: "mxc://matrix.org/avatar",
                thumbnail: { kind: "notRequested" }
              }
            }
          ]
        }
      }
    });
    emit({
      kind: "Account",
      event: {
        AvatarThumbnailDownloaded: {
          request_id: { connection_id: 1, sequence: 2 },
          mxc_uri: "mxc://matrix.org/avatar",
          thumbnail: {
            kind: "ready",
            source_url: "koushi-thumbnail://localhost/avatar/sender",
            width: null,
            height: null,
            mime_type: null
          }
        }
      }
    });

    await waitFor(() => {
      const image = document.querySelector<HTMLImageElement>(".message .avatar img");
      expect(image?.getAttribute("src")).toBe("koushi-thumbnail://localhost/avatar/sender");
    });
  });

  it("ignores avatar thumbnail events that are not relevant to the mounted timeline", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const onDiagnosticLogEntry = vi.fn();
    const onDiagnosticsChange = vi.fn();
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
        onDiagnosticsChange={onDiagnosticsChange}
        onDiagnosticLogEntry={onDiagnosticLogEntry}
        onReply={vi.fn()}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [
            {
              ...message("$avatar-relevant", "Avatar row"),
              sender_avatar: {
                mxc_uri: "mxc://matrix.org/relevant-avatar",
                thumbnail: { kind: "notRequested" }
              }
            }
          ]
        }
      }
    });
    await waitFor(() =>
      expect(onDiagnosticsChange).toHaveBeenCalledWith(
        expect.objectContaining({
          avatarMxcItems: 1,
          avatarPendingItems: 1,
          visibleItems: 1
        })
      )
    );
    onDiagnosticLogEntry.mockClear();
    onDiagnosticsChange.mockClear();

    emit({
      kind: "Account",
      event: {
        AvatarThumbnailDownloaded: {
          request_id: { connection_id: 1, sequence: 2 },
          mxc_uri: "mxc://matrix.org/unrelated-avatar",
          thumbnail: {
            kind: "ready",
            source_url: "koushi-thumbnail://localhost/avatar/unrelated",
            width: null,
            height: null,
            mime_type: null
          }
        }
      }
    });

    await new Promise((resolve) => window.setTimeout(resolve, 0));
    expect(onDiagnosticLogEntry).not.toHaveBeenCalledWith(
      expect.objectContaining({
        source: "timeline.avatar",
        message: "avatar thumbnail ready"
      })
    );
    expect(onDiagnosticsChange).not.toHaveBeenCalled();
    expect(document.querySelector(".message .avatar img")).toBeNull();
  });

  it("renders downloaded thumbnails for multiple visible sender avatars", async () => {
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

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [
            {
              ...message("$avatar-ready-a", "Avatar row A"),
              sender_avatar: {
                mxc_uri: "mxc://matrix.org/avatar-a",
                thumbnail: { kind: "notRequested" }
              }
            },
            {
              ...message("$avatar-ready-b", "Avatar row B"),
              sender: "@carol:example.invalid",
              sender_avatar: {
                mxc_uri: "mxc://matrix.org/avatar-b",
                thumbnail: { kind: "notRequested" }
              }
            }
          ]
        }
      }
    });
    emit({
      kind: "Account",
      event: {
        AvatarThumbnailDownloaded: {
          request_id: { connection_id: 1, sequence: 2 },
          mxc_uri: "mxc://matrix.org/avatar-a",
          thumbnail: {
            kind: "ready",
            source_url: "koushi-thumbnail://localhost/avatar/a",
            width: null,
            height: null,
            mime_type: null
          }
        }
      }
    });
    emit({
      kind: "Account",
      event: {
        AvatarThumbnailDownloaded: {
          request_id: { connection_id: 1, sequence: 3 },
          mxc_uri: "mxc://matrix.org/avatar-b",
          thumbnail: {
            kind: "ready",
            source_url: "koushi-thumbnail://localhost/avatar/b",
            width: null,
            height: null,
            mime_type: null
          }
        }
      }
    });

    await waitFor(() => {
      const firstImage = document.querySelector<HTMLImageElement>(
        '[data-event-id="$avatar-ready-a"] .avatar img'
      );
      const secondImage = document.querySelector<HTMLImageElement>(
        '[data-event-id="$avatar-ready-b"] .avatar img'
      );
      expect(firstImage?.getAttribute("src")).toBe("koushi-thumbnail://localhost/avatar/a");
      expect(secondImage?.getAttribute("src")).toBe("koushi-thumbnail://localhost/avatar/b");
    });
  });

  it("falls back to sender initials when a downloaded sender avatar image is broken", async () => {
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

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [
            {
              ...message("$avatar-broken", "Avatar row"),
              sender_label: "Ken Inayoshi",
              sender_avatar: {
                mxc_uri: "mxc://matrix.org/avatar-broken",
                thumbnail: {
                  kind: "ready",
                  source_url: "asset://missing-avatar.bin",
                  width: null,
                  height: null,
                  mime_type: null
                }
              }
            }
          ]
        }
      }
    });

    const image = await waitFor(() => {
      const element = document.querySelector<HTMLImageElement>(".message .avatar img");
      expect(element?.getAttribute("src")).toBe("asset://missing-avatar.bin");
      return element!;
    });
    fireEvent.error(image);

    expect(document.querySelector(".message .avatar img")).toBeNull();
    expect(document.querySelector(".message .avatar")?.textContent).toBe("KE");
  });

  it("retries a transiently broken sender avatar image URL", async () => {
    vi.useFakeTimers();
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

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [
              {
                ...message("$avatar-retry-render", "Avatar row"),
                sender_label: "Ken Inayoshi",
                sender_avatar: {
                  mxc_uri: "mxc://matrix.org/avatar-retry-render",
                  thumbnail: {
                    kind: "ready",
                    source_url: "asset://transient-avatar.bin",
                    width: null,
                    height: null,
                    mime_type: null
                  }
                }
              }
            ]
          }
        }
      });
    });

    const image = document.querySelector<HTMLImageElement>(".message .avatar img");
    expect(image).not.toBeNull();
    expect(image?.getAttribute("src")).toBe("asset://transient-avatar.bin");
    fireEvent.error(image!);
    expect(document.querySelector(".message .avatar img")).toBeNull();

    act(() => {
      vi.advanceTimersByTime(10_000);
    });

    expect(document.querySelector<HTMLImageElement>(".message .avatar img")?.getAttribute("src")).toBe(
      "asset://transient-avatar.bin"
    );
  });

  it("jumps to an unread event outside the mounted virtual window", async () => {
    const originalScrollIntoView = Element.prototype.scrollIntoView;
    const scrollIntoView = vi.fn();
    Element.prototype.scrollIntoView = scrollIntoView;
    try {
      let emit: (payload: CoreEventPayload) => void = () => undefined;
      const transport = baseTransport({
        listenCoreEvents(nextListener) {
          emit = nextListener;
          return () => undefined;
        }
      });
      const items = Array.from({ length: 650 }, (_, index) =>
        message(`$virtual-${index}:example.invalid`, `Virtual message ${index}`)
      );

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
      Object.defineProperty(timeline, "scrollHeight", { value: 650 * 72, configurable: true });
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
              items
            }
          }
        });
        emit({
          kind: "Timeline",
          event: {
            NavigationUpdated: {
              key: KEY,
              snapshot: {
                can_jump_to_bottom: false,
                first_unread_event_id: "$virtual-500:example.invalid",
                newer_event_count: 0,
                read_marker_display_event_id: null,
                read_marker_event_id: null,
                unread_event_count: 3,
                unread_position: "belowViewport"
              }
            }
          }
        });
      });

      await waitFor(() => {
        expect(timeline.getAttribute("data-virtualized")).toBe("true");
        expect(screen.getByRole("button", { name: /Jump to first unread/ })).toBeTruthy();
        expect(document.querySelector('[data-event-id="$virtual-500:example.invalid"]')).toBeNull();
      });

      fireEvent.click(screen.getByRole("button", { name: /Jump to first unread/ }));

      expect(timeline.scrollTop).toBeGreaterThan(30_000);
      await waitFor(() => expect(scrollIntoView).toHaveBeenCalled());
    } finally {
      Element.prototype.scrollIntoView = originalScrollIntoView;
    }
  });

  it("backfills an empty thread timeline even when the first Core generation is zero", async () => {
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
        onReply={vi.fn()}
      />
    );

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

    await waitFor(() => {
      expect(paginateBackwards).toHaveBeenCalledWith(threadKey);
    });
    expect(paginateBackwards).toHaveBeenCalledTimes(1);
  });

  it("renders timeline notice i18n keys in the active locale", async () => {
    setActiveLocaleProfile("ja", "none");
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

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [
            {
              ...message("$create", "created the room"),
              notice_i18n_key: "timeline.notice.roomCreate",
              message_kind: "notice"
            }
          ]
        }
      }
    });

    expect(await screen.findByText("ルームを作成しました")).toBeTruthy();
    expect(screen.queryByText("created the room")).toBeNull();
  });

  it("paginates an empty thread timeline once after initial items arrive", async () => {
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

    await waitFor(() => {
      expect(paginateBackwards).toHaveBeenCalledWith(threadKey);
    });
    expect(paginateBackwards).toHaveBeenCalledTimes(1);
  });

  it("lets users request missing room keys from undecryptable events", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const requestRoomKey = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      requestRoomKey
    });
    const encrypted = {
      ...message("$encrypted", "Unable to decrypt message"),
      unable_to_decrypt: {
        session_id: "session-1",
        reason: "missingRoomKey" as const,
        can_request_keys: true
      }
    };

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [encrypted]
        }
      }
    });

    const button = await screen.findByRole("button", { name: "Request keys and retry" });
    fireEvent.click(button);

    expect(requestRoomKey).toHaveBeenCalledWith("!room:example.invalid", "$encrypted");
  });

  it("shows visible copy controls in the message source dialog", () => {
    const source: TimelineMessageSource = {
      event_id: "$source:example.invalid",
      sender: "@alice:example.invalid",
      timestamp_ms: 1_800_000_000_000,
      body: "source body",
      in_reply_to_event_id: null,
      thread_root: null,
      is_redacted: false,
      is_edited: false,
      has_media: false,
      original_json: {
        type: "m.room.message",
        content: { body: "source body", msgtype: "m.text" }
      }
    };

    render(<MessageSourceDialog source={source} onClose={vi.fn()} />);

    expect(screen.getByRole("button", { name: "Copy event ID" }).textContent).toContain(
      "Copy event ID"
    );
    expect(
      screen.getByRole("button", { name: "Copy original event source" }).textContent
    ).toContain("Copy original event source");
  });

  it("renders the read marker after the Rust-derived display anchor for own messages after the marker", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const ownMessage = (eventId: string): TimelineItem => ({
      ...message(eventId, "own"),
      sender: "@alice:example.invalid"
    });
    const other = message("$other:example.invalid", "hello");
    const own1 = ownMessage("$own1:example.invalid");
    const own2 = ownMessage("$own2:example.invalid");

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        NavigationUpdated: {
          key: KEY,
          snapshot: {
            read_marker_event_id: "$other:example.invalid",
            read_marker_display_event_id: "$own2:example.invalid",
            first_unread_event_id: null,
            unread_event_count: 0,
            unread_position: "none",
            newer_event_count: 0,
            can_jump_to_bottom: false
          }
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
          items: [other, own1, own2]
        }
      }
    });

    const marker = await screen.findByRole("separator", { name: "Read up to here" });
    expect(marker.previousElementSibling?.getAttribute("data-event-id")).toBe(
      "$own2:example.invalid"
    );
  });

  it("renders the read marker after the current user's latest own message when the marker starts on an own message", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const ownMessage = (eventId: string): TimelineItem => ({
      ...message(eventId, "own"),
      sender: "@alice:example.invalid"
    });
    const own1 = ownMessage("$own1:example.invalid");
    const own2 = ownMessage("$own2:example.invalid");

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        NavigationUpdated: {
          key: KEY,
          snapshot: {
            read_marker_event_id: "$own1:example.invalid",
            read_marker_display_event_id: "$own2:example.invalid",
            first_unread_event_id: null,
            unread_event_count: 0,
            unread_position: "none",
            newer_event_count: 0,
            can_jump_to_bottom: false
          }
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
          items: [own1, own2]
        }
      }
    });

    const marker = await screen.findByRole("separator", { name: "Read up to here" });
    expect(marker.previousElementSibling?.getAttribute("data-event-id")).toBe(
      "$own2:example.invalid"
    );
  });

  it("renders the unread marker before the first unread event", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const other = message("$other:example.invalid", "hello");
    const unread = message("$unread:example.invalid", "new message");
    const own1 = { ...message("$own1:example.invalid", "own"), sender: "@alice:example.invalid" };

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        NavigationUpdated: {
          key: KEY,
          snapshot: {
            read_marker_event_id: "$other:example.invalid",
            read_marker_display_event_id: null,
            first_unread_event_id: "$unread:example.invalid",
            unread_event_count: 1,
            unread_position: "insideViewport",
            newer_event_count: 0,
            can_jump_to_bottom: false
          }
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
          items: [other, unread, own1]
        }
      }
    });

    const marker = await screen.findByRole("separator", { name: "Unread messages" });
    expect(marker.nextElementSibling?.getAttribute("data-event-id")).toBe(
      "$unread:example.invalid"
    );
  });

  it("renders plain-text URLs as anchors from Rust-projected link ranges", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const text = "Check https://example.com/page and https://example.com/page out";
    const item: TimelineItem = {
      ...message("$url:example.invalid", text),
      link_ranges: [
        {
          url: "https://example.com/page",
          start_utf16: 6,
          end_utf16: 30
        },
        {
          url: "https://example.com/page",
          start_utf16: 35,
          end_utf16: 59
        }
      ]
    };

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [item]
        }
      }
    });

    const links = await screen.findAllByRole("link", { name: "https://example.com/page" });
    expect(links).toHaveLength(2);
    for (const link of links) {
      expect(link.getAttribute("href")).toBe("https://example.com/page");
      expect(link.getAttribute("target")).toBe("_blank");
    }

    fireEvent.click(links[0]);
    await waitFor(() => {
      expect(openExternalHttpUrl).toHaveBeenCalledWith("https://example.com/page");
    });
  });

  it("preserves formatted HTML when adding Rust-projected link anchors", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const plainText = "fn main() {}Visit https://example.com/page";
    const item: TimelineItem = {
      ...message("$formatted-url:example.invalid", plainText),
      formatted: {
        html: "<pre><code>fn main() {}</code></pre><strong>Visit https://example.com/page</strong>",
        plain_text: plainText,
        code_blocks: [{ language: null, body: "fn main() {}" }]
      },
      link_ranges: [
        {
          url: "https://example.com/page",
          start_utf16: "fn main() {}Visit ".length,
          end_utf16: plainText.length
        }
      ]
    };

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [item]
        }
      }
    });

    const link = await screen.findByRole("link", { name: "https://example.com/page" });
    expect(link.getAttribute("href")).toBe("https://example.com/page");
    expect(link.closest("strong")).not.toBeNull();
    expect(screen.getByRole("button", { name: "Copy code" })).toBeTruthy();
  });

  it("renders link preview cards as clickable anchors", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const hideLinkPreview = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      hideLinkPreview
    });
    const item: TimelineItem = {
      ...message("$preview:example.invalid", "look at this"),
      link_previews: [
        {
          url: "https://example.com/article",
          title: "An article",
          state: "ready"
        }
      ]
    };

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [item]
        }
      }
    });

    const card = await screen.findByRole("link", { name: /An article/ });
    expect(card.getAttribute("href")).toBe("https://example.com/article");
    fireEvent.click(card);
    await waitFor(() => {
      expect(openExternalHttpUrl).toHaveBeenCalledWith("https://example.com/article");
    });

    const hide = screen.getByRole("button", { name: "Hide preview" });
    fireEvent.click(hide);
    await waitFor(() => {
      expect(hideLinkPreview).toHaveBeenCalledWith("!room:example.invalid", "$preview:example.invalid");
    });
  });

  it("keeps reactions and read receipts in one footer status row", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const item: TimelineItem = {
      ...message("$reacted:example.invalid", "hello"),
      reactions: [
        {
          key: "👍",
          count: 1,
          reacted_by_me: false,
          my_reaction_event_id: null,
          sender_preview: ["@bob:example.invalid"]
        }
      ],
      can_react: true
    };
    const liveSignals: LiveSignalsState = {
      rooms: {
        "!room:example.invalid": {
          receipts_by_event: {
            "$reacted:example.invalid": {
              readers: [
                {
                  user_id: "@bob:example.invalid",
                  display_name: "Bob",
                  original_display_label: "Bob",
                  avatar: null,
                  timestamp_ms: 1_800_000_000_000
                }
              ],
              total_count: 1,
              overflow_count: 0
            }
          },
          fully_read_event_id: null,
          typing_user_ids: []
        }
      },
      presence: {}
    };

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        liveSignals={liveSignals}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [item]
        }
      }
    });

    const statusRow = await waitFor(() => {
      const row = document.querySelector(".message-status-row");
      if (!row) {
        throw new Error("message-status-row not found");
      }
      return row;
    });
    expect(statusRow.querySelector(".message-reactions")).toBeTruthy();
    expect(statusRow.querySelector(".message-receipts")).toBeTruthy();
  });
});
