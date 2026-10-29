// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  focusedTimelineKey,
  type CoreEventPayload,
  type PaginationState
} from "../domain/coreEvents";
import { baseTransport, message, mockTimelineRects } from "./timelineViewTestSupport";
import { TimelineView, clearTimelineViewportSessionMemoryForTests } from "./TimelineView";

const FOCUSED_KEY = focusedTimelineKey(
  "@alice:example.invalid",
  "!room:example.invalid",
  "$anchor:example.invalid"
);

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  vi.restoreAllMocks();
});

function forwardPagination(state: PaginationState): CoreEventPayload {
  return {
    kind: "Timeline",
    event: {
      PaginationStateChanged: {
        request_id: null,
        key: FOCUSED_KEY,
        direction: "Forward",
        state,
        prepend_expected: null
      }
    }
  };
}

function liveTailReached(
  tailEventId: string,
  actorGeneration = 3,
  generation = 1
): CoreEventPayload {
  return {
    kind: "Timeline",
    event: {
      FocusedLiveTailReached: {
        key: FOCUSED_KEY,
        actor_generation: actorGeneration,
        generation,
        tail_event_id: tailEventId
      }
    }
  };
}

function finalPage(): CoreEventPayload {
  return {
    kind: "Timeline",
    event: {
      ItemsUpdated: {
        key: FOCUSED_KEY,
        generation: 1,
        batch_id: 1,
        diffs: [{ PushBack: { item: message("$final:example.invalid", "Final page message") } }]
      }
    }
  };
}

/**
 * The anchored main pane after an event jump. Live messages already moved the
 * room's latest event past the focused cache, which never takes them (#1676).
 */
async function renderAnchoredView(scrollTop: number) {
  let emit: (payload: CoreEventPayload) => void = () => undefined;
  const onReturnToLive = vi.fn();
  const scrollContainerRef: { current: HTMLElement | null } = { current: null };
  const rectSpy = mockTimelineRects(
    {
      "$context:example.invalid": { top: 840, height: 80 },
      "$final:example.invalid": { top: 920, height: 80 }
    },
    { top: 0, height: 500 },
    scrollContainerRef
  );
  render(
    <TimelineView
      timelineKey={FOCUSED_KEY}
      roomId="!room:example.invalid"
      transport={baseTransport({
        listenCoreEvents(listener) {
          emit = listener;
          return () => undefined;
        },
        paginateForwards: vi.fn(async () => undefined)
      })}
      onReply={vi.fn()}
      initialTargetEventId="$anchor:example.invalid"
      isAnchored
      onReturnToLive={onReturnToLive}
      liveLatestEventId="$newer-live:example.invalid"
    />
  );
  const timeline = await screen.findByTestId("timeline-view");
  scrollContainerRef.current = timeline;
  Object.defineProperty(timeline, "clientHeight", { value: 500, configurable: true });
  Object.defineProperty(timeline, "scrollHeight", { value: 1_000, configurable: true });
  Object.defineProperty(timeline, "scrollTop", { value: scrollTop, writable: true, configurable: true });
  act(() => {
    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          cause_request_id: null,
          key: FOCUSED_KEY,
          actor_generation: 3,
          generation: 1,
          items: [message("$context:example.invalid", "End of the context page")]
        }
      }
    });
  });
  await screen.findByText("End of the context page");
  const scrollTo = (top: number) => {
    timeline.scrollTop = top;
    fireEvent.wheel(timeline, { deltaY: 1 });
    fireEvent.scroll(timeline);
  };
  return {
    emit: (payload: CoreEventPayload) => act(() => emit(payload)),
    scrollTo,
    onReturnToLive,
    restore: () => rectSpy.mockRestore()
  };
}

describe("TimelineView anchored live-tail return (#1676)", () => {
  it("returns to live when Core reports the live tail while the reader rests at the bottom", async () => {
    const view = await renderAnchoredView(500);
    try {
      view.emit(forwardPagination("EndReached"));
      view.emit(finalPage());
      await screen.findByText("Final page message");
      view.emit(liveTailReached("$final:example.invalid"));
      view.scrollTo(500);
      await waitFor(() => expect(view.onReturnToLive).toHaveBeenCalledTimes(1));
    } finally {
      view.restore();
    }
  });

  it("keeps a reader above the bottom in place and returns once they reach it", async () => {
    const view = await renderAnchoredView(0);
    try {
      view.emit(forwardPagination("EndReached"));
      view.emit(finalPage());
      await screen.findByText("Final page message");
      view.emit(liveTailReached("$final:example.invalid"));
      view.scrollTo(0);
      await act(async () => Promise.resolve());
      expect(view.onReturnToLive).not.toHaveBeenCalled();

      view.scrollTo(500);
      await waitFor(() => expect(view.onReturnToLive).toHaveBeenCalledTimes(1));
    } finally {
      view.restore();
    }
  });

  it("waits until the reported tail is committed", async () => {
    const view = await renderAnchoredView(500);
    try {
      view.emit(forwardPagination("EndReached"));
      view.emit(liveTailReached("$final:example.invalid"));
      view.scrollTo(500);
      await act(async () => Promise.resolve());
      expect(view.onReturnToLive).not.toHaveBeenCalled();

      view.emit(finalPage());
      await waitFor(() => expect(view.onReturnToLive).toHaveBeenCalledTimes(1));
    } finally {
      view.restore();
    }
  });

  it("ignores a live tail reported by a replaced actor", async () => {
    const view = await renderAnchoredView(500);
    try {
      view.emit(forwardPagination("EndReached"));
      view.emit(finalPage());
      await screen.findByText("Final page message");
      view.emit(liveTailReached("$final:example.invalid", 2));
      view.scrollTo(500);
      await act(async () => Promise.resolve());
      expect(view.onReturnToLive).not.toHaveBeenCalled();
    } finally {
      view.restore();
    }
  });

  it("ignores a live tail reported for another timeline generation", async () => {
    const view = await renderAnchoredView(500);
    try {
      view.emit(forwardPagination("EndReached"));
      view.emit(finalPage());
      await screen.findByText("Final page message");
      view.emit(liveTailReached("$final:example.invalid", 3, 2));
      view.scrollTo(500);
      await act(async () => Promise.resolve());
      expect(view.onReturnToLive).not.toHaveBeenCalled();
    } finally {
      view.restore();
    }
  });
});
