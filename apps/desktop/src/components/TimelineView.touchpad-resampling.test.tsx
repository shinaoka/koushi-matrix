// @vitest-environment jsdom

import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import type { CoreEventPayload, TimelineItem } from "../domain/coreEvents";
import type { TimelineScrollDiagnostics } from "../domain/timelineScrollDiagnostics";
import { TOUCHPAD_SCROLL } from "./timeline/touchpadScrollResampler";
import { KEY, baseTransport, message } from "./timelineViewTestSupport";
import { TimelineView, clearTimelineViewportSessionMemoryForTests } from "./TimelineView";

// Linux WebKitGTK delivers touchpad deltas at the touchpad's own ~250 Hz with
// no frame alignment, so 4 or 5 events land in a frame, or none: the page moved
// 1 px in one frame and 11 px in the next (measured 2026-11-20). With
// resampling on, the timeline owns that movement, moves an even amount per
// frame at Chromium's distance, and every write is the reader's own scroll.

beforeEach(() => {
  vi.useFakeTimers({
    toFake: [
      "setTimeout",
      "clearTimeout",
      "setInterval",
      "clearInterval",
      "requestAnimationFrame",
      "cancelAnimationFrame",
      "performance",
      "Date"
    ]
  });
});

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

function messages(count: number): TimelineItem[] {
  return Array.from({ length: count }, (_, index) => message(`$item${index}`, `message ${index}`));
}

function swipeDelta(index: number): number {
  return -(5.3 + 0.6 * Math.sin(index * 1.7));
}

function setup(touchpadScrollResampling: boolean) {
  let emit: (payload: CoreEventPayload) => void = () => undefined;
  let diagnostics: TimelineScrollDiagnostics | null = null;
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
      touchpadScrollResampling={touchpadScrollResampling}
      onScrollDiagnosticsChange={(next) => {
        diagnostics = next;
      }}
    />
  );
  const timeline = screen.getByTestId("timeline-view");
  let scrollTop = 0;
  const writes: Array<{ at: number; top: number }> = [];
  Object.defineProperty(timeline, "scrollHeight", { value: 40_000, configurable: true });
  Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
  Object.defineProperty(timeline, "scrollTop", {
    configurable: true,
    get: () => scrollTop,
    set: (value: number) => {
      const changed = value !== scrollTop;
      scrollTop = value;
      writes.push({ at: performance.now(), top: value });
      // jsdom fires no scroll event for a scrollTop write; a browser does, and
      // the timeline classifies the write there (user input or programmatic).
      if (changed) timeline.dispatchEvent(new Event("scroll"));
    }
  });
  act(() => {
    emit({
      kind: "Timeline",
      event: { InitialItems: { request_id: null, key: KEY, generation: 1, items: messages(60) } }
    });
  });
  act(() => {
    vi.advanceTimersByTime(500);
  });
  // Mid-history, away from both edges.
  scrollTop = 20_000;
  writes.length = 0;
  return {
    timeline,
    writes,
    get scrollTop() {
      return scrollTop;
    },
    get diagnostics() {
      return diagnostics;
    }
  };
}

function swipe(timeline: HTMLElement, events: number): { prevented: boolean[]; deltas: number[] } {
  const prevented: boolean[] = [];
  const deltas: number[] = [];
  for (let index = 0; index < events; index += 1) {
    const deltaY = swipeDelta(index);
    const event = new WheelEvent("wheel", { deltaY, deltaMode: 0, bubbles: true, cancelable: true });
    act(() => {
      timeline.dispatchEvent(event);
    });
    prevented.push(event.defaultPrevented);
    deltas.push(deltaY);
    act(() => {
      vi.advanceTimersByTime(4);
    });
  }
  return { prevented, deltas };
}

it("resamples a 250 Hz touchpad swipe into even, scaled, user-owned frames", () => {
  const view = setup(true);
  const before = view.scrollTop;
  const writesBefore = { ...view.diagnostics!.scrollWrites };

  const { prevented, deltas } = swipe(view.timeline, 75);
  act(() => {
    vi.advanceTimersByTime(300);
  });

  // The first delta proves nothing yet and scrolls natively; the timeline owns
  // every later one.
  expect(prevented[0]).toBe(false);
  expect(prevented.slice(1).every(Boolean)).toBe(true);

  // Chromium's distance for the resampled part of the swipe.
  const resampledInput = deltas.slice(1).reduce((sum, delta) => sum + delta, 0);
  expect(view.scrollTop - before).toBeCloseTo(resampledInput * TOUCHPAD_SCROLL.speed, 0);

  // One write per frame, each moving about the same amount.
  const steps = view.writes.map((write, index) => write.top - (index === 0 ? before : view.writes[index - 1].top));
  const steady = steps.slice(3, 14).map(Math.abs);
  expect(Math.max(...steady) / Math.min(...steady)).toBeLessThan(1.3);
  const frameGaps = view.writes.slice(1, 15).map((write, index) => write.at - view.writes[index].at);
  expect(frameGaps.every((gap) => gap >= 15 && gap <= 18)).toBe(true);

  // The reader's own scroll: free-scroll intent, no programmatic write.
  expect(view.diagnostics!.latestFrame?.viewportIntent).toBe("freeScroll");
  expect(view.diagnostics!.scrollWrites).toEqual(writesBefore);
});

it("leaves touchpad input to the browser when resampling is off", () => {
  const view = setup(false);
  const { prevented } = swipe(view.timeline, 20);
  expect(prevented.some(Boolean)).toBe(false);
});

it("leaves whole-pixel wheel input to the browser", () => {
  const view = setup(true);
  for (let index = 0; index < 10; index += 1) {
    const event = new WheelEvent("wheel", { deltaY: -40, deltaMode: 0, bubbles: true, cancelable: true });
    act(() => {
      view.timeline.dispatchEvent(event);
    });
    expect(event.defaultPrevented).toBe(false);
    act(() => {
      vi.advanceTimersByTime(4);
    });
  }
});

it("stops a glide when a jump takes the viewport", () => {
  const view = setup(true);
  // A flick: fractional deltas ramping up at 4 ms.
  for (let index = 0; index < 12; index += 1) {
    const event = new WheelEvent("wheel", {
      deltaY: -(15 + index * 3.5) - 0.3,
      deltaMode: 0,
      bubbles: true,
      cancelable: true
    });
    act(() => {
      view.timeline.dispatchEvent(event);
      vi.advanceTimersByTime(4);
    });
  }
  act(() => {
    vi.advanceTimersByTime(120);
  });
  const gliding = view.scrollTop;
  act(() => {
    vi.advanceTimersByTime(34);
  });
  expect(view.scrollTop).toBeLessThan(gliding);

  // The reader grabs the scrollbar: the glide stops at once.
  act(() => {
    view.timeline.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
  });
  const grabbed = view.scrollTop;
  act(() => {
    vi.advanceTimersByTime(300);
  });
  expect(view.scrollTop).toBe(grabbed);
});
