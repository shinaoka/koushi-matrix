// @vitest-environment jsdom

import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { CoreEventPayload, TimelineItem } from "../domain/coreEvents";
import { createManualTimelineViewportScheduler } from "./timeline/TimelineViewportScheduler";
import {
  KEY,
  baseTransport,
  message,
  mockTimelineRects,
} from "./timelineViewTestSupport";
import {
  TimelineView,
  clearTimelineViewportSessionMemoryForTests,
} from "./TimelineView";

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("TimelineView anchor settlement", () => {
  it("preserves the pre-apply anchor when measurement flush wins the restoration race", async () => {
    vi.useFakeTimers();
    const scheduler = createManualTimelineViewportScheduler();
    let listener: ((payload: CoreEventPayload) => void) | null = null;
    const onScrollDiagnosticsChange = vi.fn();
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        listener = nextListener;
        return () => undefined;
      },
    });
    const rects: Record<string, { top: number; height: number }> = {};
    for (let index = 0; index < 700; index += 1) {
      rects[`$item${index}`] = { top: index * 72, height: 72 };
    }
    const scrollContainerRef: { current: HTMLElement | null } = {
      current: null,
    };
    mockTimelineRects(rects, { top: 0, height: 600 }, scrollContainerRef);

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={() => undefined}
        onScrollDiagnosticsChange={onScrollDiagnosticsChange}
        viewportScheduler={scheduler}
        listRefCallback={(element) => {
          scrollContainerRef.current =
            element?.closest<HTMLElement>("[data-testid=timeline-view]") ??
            null;
        }}
      />,
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
              message(`$item${index}`, `message ${index}`),
            ),
          },
        },
      });
    });

    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollTop", {
      value: 0,
      writable: true,
      configurable: true,
    });
    Object.defineProperty(timeline, "scrollHeight", {
      value: 700 * 72,
      writable: true,
      configurable: true,
    });
    Object.defineProperty(timeline, "clientHeight", {
      value: 600,
      writable: true,
      configurable: true,
    });
    timeline.scrollTop = 49_800;
    fireEvent.scroll(timeline);
    act(() => {
      scheduler.flushAll();
      vi.advanceTimersByTime(500);
      scheduler.flushAll();
    });
    timeline.scrollTop = 20_000;
    fireEvent.wheel(timeline, { deltaY: -40 });
    fireEvent.scroll(timeline);
    act(() => scheduler.flushAll());

    const anchor = Array.from(
      timeline.querySelectorAll<HTMLElement>("[data-item-id]"),
    ).find((node) => node.getBoundingClientRect().bottom > 0);
    expect(anchor).toBeDefined();
    const anchorId = anchor?.dataset["itemId"] ?? "";
    const anchorOffset = anchor?.getBoundingClientRect().top ?? 0;

    const prepended = Array.from({ length: 100 }, (_, index) =>
      message(`$old${index}`, `old ${index}`),
    );
    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key: KEY,
            generation: 1,
            batch_id: 1,
            diffs: [
              ...prepended.map((item) => ({ PushFront: { item } })),
              {
                Set: {
                  index: 700,
                  item: {
                    ...message("$item600", "message 600"),
                    is_hidden: true,
                  } as TimelineItem,
                },
              },
            ],
          },
        },
      });
      let extraHeight = 0;
      for (let index = 0; index < 700; index += 1) {
        const height = index >= 117 && index < 123 ? 136 : 72;
        rects[`$item${index}`] = {
          top: 100 * 72 + index * 72 + extraHeight,
          height,
        };
        extraHeight += height - 72;
      }
      for (let index = 0; index < 100; index += 1) {
        rects[`$old${index}`] = { top: (99 - index) * 72, height: 72 };
      }
    });
    act(() => {
      vi.advanceTimersByTime(100);
    });
    for (let attempt = 0; attempt < 3; attempt += 1) {
      await act(async () => {
        scheduler.flushAll();
        await Promise.resolve();
      });
    }

    const restored = timeline.querySelector<HTMLElement>(
      `[data-item-id="${anchorId}"]`,
    );
    expect(restored).not.toBeNull();
    expect(restored!.getBoundingClientRect().top).toBe(anchorOffset);
    expect(
      onScrollDiagnosticsChange.mock.calls.at(-1)?.[0].maxAnchorTopDeltaPx,
    ).toBeGreaterThan(0);
  });
});
