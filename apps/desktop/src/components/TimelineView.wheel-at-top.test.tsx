// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";

import type { CoreEventPayload, TimelineItem } from "../domain/coreEvents";
import { KEY, baseTransport, message } from "./timelineViewTestSupport";
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
