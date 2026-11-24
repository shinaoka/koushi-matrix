// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { CoreEventPayload } from "../domain/coreEvents";
import { KEY, baseTransport, message, mockTimelineRects } from "./timelineViewTestSupport";
import { createManualTimelineViewportScheduler } from "./timeline/TimelineViewportScheduler";
import { TimelineView, clearTimelineViewportSessionMemoryForTests } from "./TimelineView";

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  vi.restoreAllMocks();
});

it.each([true, false])("reports historical bottom only after the window boundaries commit (%s)", async (boundariesCommitted) => {
  const scheduler = createManualTimelineViewportScheduler();
  let emit: (payload: CoreEventPayload) => void = () => undefined;
  const returnToLive = vi.fn().mockResolvedValue(undefined);
  const onReturnToLive = vi.fn().mockResolvedValue(undefined);
  const observeViewport = vi.fn().mockResolvedValue(undefined);
  const transport = baseTransport({
    listenCoreEvents(listener) { emit = listener; return () => undefined; },
    observeViewport,
    returnToLive
  });
  const containerRef: { current: HTMLElement | null } = { current: null };
  const rectSpy = mockTimelineRects({
    "$older:example.invalid": { top: 1540, height: 80 },
    "$newer:example.invalid": { top: 1640, height: 80 },
    "$latest:example.invalid": { top: 1950, height: 80 }
  }, { top: 0, height: 500 }, containerRef);
  try {
    render(<TimelineView timelineKey={KEY} roomId="!room:example.invalid" transport={transport} onReply={vi.fn()} onReturnToLive={onReturnToLive} viewportScheduler={scheduler} />);
    const timeline = await screen.findByTestId("timeline-view");
    containerRef.current = timeline;
    Object.defineProperties(timeline, {
      clientHeight: { value: 500, configurable: true },
      scrollHeight: { value: 2000, configurable: true },
      scrollTop: { value: 1500, writable: true, configurable: true }
    });
    act(() => {
      emit({ kind: "Timeline", event: { InitialItems: { request_id: null, key: KEY, generation: 1,
        items: [message("$older:example.invalid", "Older"), message("$newer:example.invalid", "Newer")]
      } } });
      emit({ kind: "Timeline", event: { NavigationUpdated: { key: KEY, snapshot: {
        read_marker_event_id: null, read_marker_display_event_id: null,
        first_unread_event_id: null, first_unread_display_event_id: null,
        unread_event_count: 0, unread_position: "none", newer_event_count: 10,
        can_jump_to_bottom: true, local_viewed_event_id: null,
        server_confirmed_read_event_id: null, read_state_sync: "notRequested",
        display_window: { revision: 7, first_event_id: "$older:example.invalid", last_event_id: boundariesCommitted ? "$newer:example.invalid" : "$future:example.invalid", reaches_live_end: false }
      } } } });
    });
    act(() => { scheduler.flushAll(); });
    timeline.scrollTop = 1500;
    fireEvent.wheel(timeline, { deltaY: 10 });
    fireEvent.scroll(timeline);
    act(() => { scheduler.flushAll(); });
    if (!boundariesCommitted) {
      expect(observeViewport).not.toHaveBeenCalled();
      return;
    }
    await waitFor(() => expect(observeViewport).toHaveBeenCalled());
    expect(observeViewport.mock.calls.every((call) => call[4] === false)).toBe(true);
    expect(observeViewport.mock.calls.at(-1)?.[7]).toBe(7);
    fireEvent.click(screen.getByRole("button", { name: "Jump to bottom, 10 new messages" }));
    expect(returnToLive).toHaveBeenCalledWith(KEY);
    expect(onReturnToLive).not.toHaveBeenCalled();
    expect(timeline.scrollTop).toBe(1500);
    Object.defineProperty(timeline, "scrollHeight", { value: 2400, configurable: true });
    act(() => {
      emit({ kind: "Timeline", event: { NavigationUpdated: { key: KEY, snapshot: {
        read_marker_event_id: null, read_marker_display_event_id: null,
        first_unread_event_id: null, first_unread_display_event_id: null,
        unread_event_count: 0, unread_position: "none", newer_event_count: 0,
        can_jump_to_bottom: false, local_viewed_event_id: null,
        server_confirmed_read_event_id: null, read_state_sync: "notRequested",
        display_window: { revision: 8, first_event_id: "$latest:example.invalid", last_event_id: "$latest:example.invalid", reaches_live_end: true }
      } } } });
      emit({ kind: "Timeline", event: { ItemsUpdated: { key: KEY, generation: 1, batch_id: 1,
        diffs: [{ Reset: { items: [message("$latest:example.invalid", "Latest message")] } }]
      } } });
    });
    act(() => { scheduler.flushAll(); });
    expect(screen.getByText("Latest message")).toBeTruthy();
    expect(timeline.scrollTop).toBe(1900);
  } finally { rectSpy.mockRestore(); }
});

// #2092: opening a room at the live edge leaves the unread badge and "Jump to
// bottom" bar in place because nothing asks again once the rows and the display
// window agree. The invariant is the one the guards already encode: report only
// when the committed rows span the display window. Here the window moves ahead
// of the rows (a newer message lands), the report is suppressed, and the rows
// then catch up with no scroll to drive another ask.
it("asks again when the display window lands after the rows (#2092)", async () => {
  const scheduler = createManualTimelineViewportScheduler();
  let emit: (payload: CoreEventPayload) => void = () => undefined;
  const observeViewport = vi.fn().mockResolvedValue(undefined);
  const transport = baseTransport({
    listenCoreEvents(listener) { emit = listener; return () => undefined; },
    observeViewport
  });
  const containerRef: { current: HTMLElement | null } = { current: null };
  const rectSpy = mockTimelineRects({
    "$older:example.invalid": { top: 0, height: 80 },
    "$newer:example.invalid": { top: 80, height: 80 },
    "$newest:example.invalid": { top: 160, height: 80 }
  }, { top: 0, height: 500 }, containerRef);
  try {
    render(<TimelineView timelineKey={KEY} roomId="!room:example.invalid" transport={transport} onReply={vi.fn()} onReturnToLive={vi.fn()} viewportScheduler={scheduler} />);
    const timeline = await screen.findByTestId("timeline-view");
    containerRef.current = timeline;
    Object.defineProperties(timeline, {
      clientHeight: { value: 500, configurable: true },
      scrollHeight: { value: 240, configurable: true },
      scrollTop: { value: 0, writable: true, configurable: true }
    });
    act(() => {
      emit({ kind: "Timeline", event: { InitialItems: { request_id: null, key: KEY, generation: 1,
        items: [message("$older:example.invalid", "Older"), message("$newer:example.invalid", "Newer")]
      } } });
      emit({ kind: "Timeline", event: { NavigationUpdated: { key: KEY, snapshot: {
        read_marker_event_id: null, read_marker_display_event_id: null,
        first_unread_event_id: null, first_unread_display_event_id: null,
        unread_event_count: 0, unread_position: "none", newer_event_count: 0,
        can_jump_to_bottom: false, local_viewed_event_id: null,
        server_confirmed_read_event_id: null, read_state_sync: "notRequested",
        display_window: { revision: 7, first_event_id: "$older:example.invalid", last_event_id: "$newer:example.invalid", reaches_live_end: true }
      } } } });
    });
    act(() => { scheduler.flushAll(); });
    await act(async () => { await Promise.resolve(); });
    observeViewport.mockClear();
    // The rows move ahead of the window: the same suppression, other order.
    act(() => {
      emit({ kind: "Timeline", event: { ItemsUpdated: { key: KEY, generation: 1, batch_id: 1,
        diffs: [{ Insert: { index: 2, item: message("$newest:example.invalid", "Newest message") } }]
      } } });
    });
    act(() => { scheduler.flushAll(); });
    expect(observeViewport).not.toHaveBeenCalled();
    // The window catches up and the reader never scrolls.
    act(() => {
      emit({ kind: "Timeline", event: { NavigationUpdated: { key: KEY, snapshot: {
        read_marker_event_id: null, read_marker_display_event_id: null,
        first_unread_event_id: null, first_unread_display_event_id: null,
        unread_event_count: 0, unread_position: "none", newer_event_count: 0,
        can_jump_to_bottom: false, local_viewed_event_id: null,
        server_confirmed_read_event_id: null, read_state_sync: "notRequested",
        display_window: { revision: 8, first_event_id: "$older:example.invalid", last_event_id: "$newest:example.invalid", reaches_live_end: true }
      } } } });
    });
    act(() => { scheduler.flushAll(); });
    await waitFor(() => expect(observeViewport).toHaveBeenCalled());
    const account = observeViewport.mock.calls.at(-1);
    expect(account?.[7]).toBe(8);
    expect(account?.[4]).toBe(true);
  } finally { rectSpy.mockRestore(); }
});
