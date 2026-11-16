// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";

import type { CoreEventPayload, TimelineDiff, TimelineItem } from "../domain/coreEvents";
import { KEY, baseTransport, message, mockTimelineRects } from "./timelineViewTestSupport";
import { TimelineView, clearTimelineViewportSessionMemoryForTests } from "./TimelineView";

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  vi.restoreAllMocks();
});

function messages(count: number): TimelineItem[] {
  return Array.from({ length: count }, (_, index) => message(`$item${index}`, `message ${index}`));
}

// #1916: at the very top of a scrollable timeline, wheel-up input changes
// nothing, so the browser fires no `scroll` event. That input is still the
// reader asking for older history: it must reach the backfill policy as a
// genuine user scroll, or a request fenced on "a new user scroll" never retries.
it("retries fenced back-pagination on wheel-up at the top without a scroll event", async () => {
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
  render(
    <TimelineView timelineKey={KEY} roomId="!room:example.invalid" transport={transport} onReply={vi.fn()} />
  );

  // Sitting at the top. Underfilled, so the first request is automatic.
  const timeline = screen.getByTestId("timeline-view");
  Object.defineProperty(timeline, "scrollHeight", { value: 320, configurable: true });
  Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
  Object.defineProperty(timeline, "scrollTop", { value: 0, writable: true, configurable: true });
  act(() => {
    emit({
      kind: "Timeline",
      event: { InitialItems: { request_id: null, key: KEY, generation: 1, items: messages(1) } }
    });
  });

  // The first request fails, which fences automatic retries until a new
  // state transition such as a genuine user scroll.
  await waitFor(() => expect(paginateBackwards).toHaveBeenCalledTimes(1));
  await act(async () => Promise.resolve());
  expect(paginateBackwards).toHaveBeenCalledTimes(1);

  // Wheel up at the top: no scroll event follows. Well inside the first
  // automatic backoff retry, so only the wheel can explain a second request.
  fireEvent.wheel(timeline, { deltaY: -120 });
  await waitFor(() => expect(paginateBackwards).toHaveBeenCalledTimes(2), { timeout: 500 });
});

it("retries fenced back-pagination on PageUp at the top without a scroll event", async () => {
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
  render(
    <TimelineView timelineKey={KEY} roomId="!room:example.invalid" transport={transport} onReply={vi.fn()} />
  );
  const timeline = screen.getByTestId("timeline-view");
  Object.defineProperty(timeline, "scrollHeight", { value: 320, configurable: true });
  Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
  Object.defineProperty(timeline, "scrollTop", { value: 0, writable: true, configurable: true });
  act(() => {
    emit({
      kind: "Timeline",
      event: { InitialItems: { request_id: null, key: KEY, generation: 1, items: messages(1) } }
    });
  });
  await waitFor(() => expect(paginateBackwards).toHaveBeenCalledTimes(1));
  await act(async () => Promise.resolve());

  fireEvent.keyDown(timeline, { key: "PageUp" });
  await waitFor(() => expect(paginateBackwards).toHaveBeenCalledTimes(2), { timeout: 500 });
});

it("does not treat wheel-down at the top as a request for older history", async () => {
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
  render(
    <TimelineView timelineKey={KEY} roomId="!room:example.invalid" transport={transport} onReply={vi.fn()} />
  );
  const timeline = screen.getByTestId("timeline-view");
  Object.defineProperty(timeline, "scrollHeight", { value: 320, configurable: true });
  Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
  Object.defineProperty(timeline, "scrollTop", { value: 0, writable: true, configurable: true });
  act(() => {
    emit({
      kind: "Timeline",
      event: { InitialItems: { request_id: null, key: KEY, generation: 1, items: messages(1) } }
    });
  });
  await waitFor(() => expect(paginateBackwards).toHaveBeenCalledTimes(1));
  await act(async () => Promise.resolve());

  fireEvent.wheel(timeline, { deltaY: 120 });
  await new Promise((resolve) => setTimeout(resolve, 300));
  expect(paginateBackwards).toHaveBeenCalledTimes(1);
});

// #1916: the core now reads the unskipped item list, so a backward page that
// did reach the network reports `Idle, prepend_expected: false` before its rows
// arrive. The renderer closes the request epoch on that terminal, which
// re-evaluates pagination -- while the prepend that must hold the reader's
// position is still in flight. Pin both halves of that window.
it("holds the anchor when a prepend lands after the terminal has closed the epoch", async () => {
  let emit: (payload: CoreEventPayload) => void = () => undefined;
  const paginateBackwards = vi.fn<() => Promise<void>>().mockResolvedValue(undefined);
  const transport = baseTransport({
    listenCoreEvents(nextListener) {
      emit = nextListener;
      return () => undefined;
    },
    paginateBackwards
  });
  const rects: Record<string, { top: number; height: number }> = {};
  for (let index = 0; index < 30; index += 1) {
    rects[`$item${index}`] = { top: index * 72, height: 72 };
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

  const timeline = screen.getByTestId("timeline-view");
  const sized = (value: number) => ({
    value,
    writable: true,
    configurable: true
  });
  Object.defineProperty(timeline, "scrollTop", sized(0));
  Object.defineProperty(timeline, "scrollHeight", sized(30 * 72));
  Object.defineProperty(timeline, "clientHeight", sized(600));
  act(() => {
    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: messages(30)
        }
      }
    });
  });

  // The room opens at the live edge; scroll up to the top and read there.
  await act(async () => {
    timeline.scrollTop = 0;
  });
  fireEvent.wheel(timeline, { deltaY: -120 });
  fireEvent.scroll(timeline);
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

  const anchor = Array.from(timeline.querySelectorAll<HTMLElement>("[data-item-id]")).find(
    (node) => node.getBoundingClientRect().bottom > 0
  );
  expect(anchor).toBeDefined();
  const anchorId = anchor?.dataset["itemId"] ?? "";
  const anchorTop = anchor?.getBoundingClientRect().top ?? 0;

  // The page reached the network, so the terminal says no prepend was promised
  // -- while the rows it produced are still on their way.
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
  const prepended = Array.from({ length: 10 }, (_, index) =>
    message(`$older${index}`, `older ${index}`)
  );
  act(() => {
    emit({
      kind: "Timeline",
      event: {
        ItemsUpdated: {
          key: KEY,
          generation: 1,
          batch_id: 2,
          diffs: prepended.map((item): TimelineDiff => ({ PushFront: { item } }))
        }
      }
    });
    for (let index = 0; index < 30; index += 1) {
      rects[`$item${index}`] = { top: 10 * 72 + index * 72, height: 72 };
    }
    for (const [index] of prepended.entries()) {
      rects[`$older${index}`] = { top: (9 - index) * 72, height: 72 };
    }
  });

  await waitFor(() => {
    const restored = timeline.querySelector<HTMLElement>(`[data-item-id="${anchorId}"]`);
    expect(restored).not.toBeNull();
    expect(restored!.getBoundingClientRect().top).toBe(anchorTop);
  });
  // The closed epoch lets the deferred re-evaluation ask once more; the late
  // prepend must neither suppress that nor turn it into a retry loop.
  await waitFor(() => expect(paginateBackwards).toHaveBeenCalledTimes(2));
  await new Promise((resolve) => setTimeout(resolve, 300));
  expect(paginateBackwards).toHaveBeenCalledTimes(2);
});
