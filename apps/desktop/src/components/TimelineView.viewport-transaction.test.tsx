// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CoreEventPayload, TimelineItem } from "../domain/coreEvents";
import { createManualTimelineViewportScheduler } from "./timeline/TimelineViewportScheduler";
import { KEY, baseTransport, message, mockTimelineRects } from "./timelineViewTestSupport";
import { TimelineView, clearTimelineViewportSessionMemoryForTests } from "./TimelineView";

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

function setup(count: number, roundWrites = false) {
  vi.useFakeTimers();
  const scheduler = createManualTimelineViewportScheduler();
  let listener: ((event: CoreEventPayload) => void) | null = null;
  const rects: Record<string, { top: number; height: number }> = {};
  for (let i = 0; i < count; i++) rects[`$item${i}`] = { top: i * 72, height: 72 };
  const containerRef: { current: HTMLElement | null } = { current: null };
  mockTimelineRects(rects, { top: 0, height: 600 }, containerRef);
  const diagnostics = vi.fn();
  render(<TimelineView timelineKey={KEY} roomId="!room:example.invalid"
    transport={baseTransport({ listenCoreEvents(next) { listener = next; return () => undefined; } })}
    onReply={() => undefined} enableAvatarThumbnailDownloads={false} viewportScheduler={scheduler}
    onDiagnosticLogEntry={diagnostics}
    listRefCallback={(node) => { containerRef.current = node?.closest<HTMLElement>("[data-testid=timeline-view]") ?? null; }}
  />);
  const emit = (event: CoreEventPayload) => listener?.(event);
  const view = screen.getByTestId("timeline-view");
  let scrollTop = count * 72 - 600;
  let scrollHeight = count * 72;
  const writes: number[] = [];
  Object.defineProperties(view, {
    scrollTop: { configurable: true, get: () => scrollTop, set: (value: number) => { writes.push(value); scrollTop = roundWrites ? Math.round(value) : value; } },
    scrollHeight: { configurable: true, get: () => scrollHeight },
    clientHeight: { configurable: true, value: 600 }
  });
  act(() => emit({ kind: "Timeline", event: { InitialItems: {
    request_id: null, key: KEY, generation: 1,
    items: Array.from({ length: count }, (_, i) => message(`$item${i}`, "Synthetic row"))
  } } }));
  const drain = () => {
    // Flush React commits between scheduler phases; the scheduler itself bounds
    // recursive callbacks. No wall-clock sleeps or renderer product fake.
    for (let phase = 0; phase < 3; phase++) act(() => { vi.advanceTimersByTime(100); scheduler.flushAll(); });
  };
  fireEvent.scroll(view);
  drain();
  scrollTop = 1000;
  fireEvent.wheel(view, { deltaY: -40 });
  fireEvent.scroll(view);
  act(() => scheduler.flushAll());
  const row = [...view.querySelectorAll<HTMLElement>("[data-item-id]")].find(node => {
    const rect = node.getBoundingClientRect();
    return rect.bottom > 0 && rect.top < 600;
  })!;
  expect(row).toBeDefined();
  const anchor = { id: row.dataset.itemId!, offset: row.getBoundingClientRect().top };
  writes.length = 0;
  diagnostics.mockClear();
  return { view, emit, rects, writes, diagnostics, anchor, drain,
    userScroll(delta: number) {
      scrollTop += delta; // Browser-owned input: do not count it as a compensation write.
      fireEvent.wheel(view, { deltaY: delta });
      fireEvent.scroll(view);
    },
    prepend(amount: number, delayedHeight: number) {
      act(() => {
        emit({ kind: "Timeline", event: { ItemsUpdated: {
          key: KEY, generation: 1, batch_id: 1,
          diffs: [
            ...Array.from({ length: amount }, (_, i) => ({ PushFront: { item: message(`$old${i}`, "Older synthetic row") } })),
            { Set: { index: count + amount - 1, item: { ...message(`$item${count - 1}`, "Synthetic row"), is_hidden: true } as TimelineItem } }
          ]
        } } });
        for (let i = 0; i < count; i++) rects[`$item${i}`] = {
          top: (amount + i) * 72 + (i > 5 ? delayedHeight : 0), height: i === 5 ? 72 + delayedHeight : 72
        };
        for (let i = 0; i < amount; i++) rects[`$old${i}`] = { top: (amount - i - 1) * 72, height: 72 };
        scrollHeight += amount * 72 + delayedHeight;
      });
    }
  };
}

describe("TimelineView unified viewport transaction", () => {
  for (const { count, withPrepend } of [{ count: 120, withPrepend: false }, { count: 601, withPrepend: false }, { count: 601, withPrepend: true }]) {
    it(`settles a native resize before its observer returns (${count} rows, prepend=${withPrepend})`, () => {
      let resize: () => void = () => undefined;
      vi.stubGlobal("ResizeObserver", class {
        constructor(private callback: ResizeObserverCallback) {}
        observe(node: Element) {
          if (node.classList.contains("timeline-item-list")) resize = () => this.callback([], this as unknown as ResizeObserver);
        }
        disconnect() {}
        unobserve() {}
      });
      const fixture = setup(count);
      fixture.drain();
      if (withPrepend) {
        fixture.prepend(80, 64);
        fixture.userScroll(-10);
      }
      fixture.rects.$item5.height += 80;
      for (let i = 6; i < count; i++) fixture.rects[`$item${i}`].top += 80;
      // Re-arm the native observer; the injected viewport scheduler stays held.
      act(() => vi.advanceTimersToNextFrame());
      act(() => {
        resize();
        const row = [...fixture.view.querySelectorAll<HTMLElement>("[data-item-id]")].find(node => node.dataset.itemId === fixture.anchor.id)!;
        expect(row.getBoundingClientRect().top).toBe(fixture.anchor.offset + (withPrepend ? 10 : 0));
      });
      expect(fixture.writes.length).toBeGreaterThan(0);
      expect(fixture.writes.length).toBeLessThanOrEqual(withPrepend ? 2 : 1);
      const writeCount = fixture.writes.length;
      const beginCount = fixture.diagnostics.mock.calls.filter(([entry]) => entry.source === "timeline.viewport_transaction" && entry.message.includes("stage=begin")).length;
      fixture.drain(); // Let the observer re-arm before delivering an unchanged size.
      act(() => resize());
      fixture.drain();
      expect(fixture.writes).toHaveLength(writeCount);
      expect(fixture.diagnostics.mock.calls.filter(([entry]) => entry.source === "timeline.viewport_transaction" && entry.message.includes("stage=begin"))).toHaveLength(beginCount);
    });
  }
  for (const withInput of [false, true]) {
  it(`does not accumulate subpixel drift across resize observations (input=${withInput})`, () => {
    let resize: () => void = () => undefined;
    vi.stubGlobal("ResizeObserver", class {
      constructor(private callback: ResizeObserverCallback) {}
      observe(node: Element) {
        if (node.classList.contains("timeline-item-list")) resize = () => this.callback([], this as unknown as ResizeObserver);
      }
      disconnect() {}
      unobserve() {}
    });
    const fixture = setup(120, true);
    fixture.drain();
    for (let observation = 0; observation < 9; observation++) {
      if (withInput) fixture.userScroll(-2);
      fixture.rects.$item5.height += 0.25;
      for (let i = 6; i < 120; i++) fixture.rects[`$item${i}`].top += 0.25;
      act(() => resize());
      fixture.drain();
      const row = [...fixture.view.querySelectorAll<HTMLElement>("[data-item-id]")].find(node => node.dataset.itemId === fixture.anchor.id)!;
      const expectedOffset = fixture.anchor.offset + (withInput ? (observation + 1) * 2 : 0);
      expect(Math.abs(row.getBoundingClientRect().top - expectedOffset)).toBeLessThanOrEqual(0.5);
    }
  });
  }
  it("does not re-defer a released prepend when input resumes before its render commits", () => {
    const fixture = setup(120);
    act(() => fixture.emit({ kind: "Timeline", event: { ItemsUpdated: {
      key: KEY, generation: 1, batch_id: 1,
      diffs: Array.from({ length: 10 }, (_, i) => ({ PushFront: { item: message(`$pending${i}`, "Older synthetic row") } }))
    } } }));
    expect(fixture.view.dataset.totalItems).toBe("120");
    act(() => {
      vi.advanceTimersByTime(100);
      fixture.userScroll(-2);
    });
    fixture.drain();
    expect(fixture.view.dataset.totalItems).toBe("130");
  });
  it("ignores a stale-generation prepend while the current transaction is pending", () => {
    const fixture = setup(120);
    fixture.prepend(3, 64);
    act(() => fixture.emit({ kind: "Timeline", event: { ItemsUpdated: {
      key: KEY, generation: 0, batch_id: 99,
      diffs: [{ PushFront: { item: message("$stale", "Stale synthetic row") } }]
    } } }));
    fixture.drain();
    const row = [...fixture.view.querySelectorAll<HTMLElement>("[data-item-id]")].find(node => node.dataset.itemId === fixture.anchor.id)!;
    expect(row.getBoundingClientRect().top).toBe(fixture.anchor.offset);
    expect(fixture.writes).toHaveLength(1);
  });
  it("invalidates a pending transaction on a new generation with identical row structure", () => {
    const fixture = setup(120);
    fixture.prepend(3, 64);
    const items = [
      ...[2, 1, 0].map(i => message(`$old${i}`, "Older synthetic row")),
      ...Array.from({ length: 120 }, (_, i) => ({ ...message(`$item${i}`, "Synthetic row"), is_hidden: i === 119 }))
    ];
    act(() => fixture.emit({ kind: "Timeline", event: { InitialItems: {
      request_id: null, key: KEY, generation: 2, items
    } } }));
    fixture.drain();
    expect(fixture.view.scrollTop).toBe(fixture.view.scrollHeight - fixture.view.clientHeight);
    expect(fixture.writes).not.toContain(1280); // The retired generation's anchor target.
    expect(fixture.diagnostics.mock.calls.some(([entry]) => entry.source === "timeline.viewport_transaction" && entry.message.includes("reason=generation"))).toBe(true);
  });
  for (const count of [120, 601]) {
    it(`rebases input before the queued layout and folds delayed heights (${count} rows)`, () => {
      const fixture = setup(count);
      fixture.prepend(count > 500 ? 80 : 3, 64);
      fixture.userScroll(-10);
      fixture.drain();
      const row = [...fixture.view.querySelectorAll<HTMLElement>("[data-item-id]")].find(node => node.dataset.itemId === fixture.anchor.id);
      expect(row).toBeDefined();
      expect(row!.getBoundingClientRect().top, JSON.stringify({ writes: fixture.writes, lifecycle: fixture.diagnostics.mock.calls.map(([entry]) => entry).filter(entry => entry.source === "timeline.viewport_transaction") })).toBe(fixture.anchor.offset + 10);
      expect(fixture.writes.length).toBeGreaterThan(0);
      expect(fixture.writes.length).toBeLessThanOrEqual(count > 500 ? 2 : 1);
      const lifecycle = fixture.diagnostics.mock.calls.map(([entry]) => entry).filter(entry => entry.source === "timeline.viewport_transaction");
      expect(lifecycle.some(entry => entry.message.includes("stage=rebase"))).toBe(true);
      expect(lifecycle.filter(entry => entry.message.includes("reason=settled"))).toHaveLength(1);
      expect(JSON.stringify(lifecycle)).not.toMatch(/example\.invalid|\$item|\$old/);
    });
  }
});
