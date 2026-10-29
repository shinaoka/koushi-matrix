// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { createTauriTimelineTransport } from "../backend/tauriTimelineTransport";
import {
  focusedTimelineKey,
  type CoreEventPayload,
  type PaginationState,
  type TimelineKey
} from "../domain/coreEvents";
import { baseTransport, message } from "./timelineViewTestSupport";
import { TimelineView, clearTimelineViewportSessionMemoryForTests } from "./TimelineView";

// The production desktop transport invokes Tauri; observe its commands here.
const invoke = vi.hoisted(() => vi.fn(async () => ({ accepted: true })));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const FOCUSED_KEY = focusedTimelineKey(
  "@alice:example.invalid",
  "!room:example.invalid",
  "$anchor:example.invalid"
);

// jsdom omits scrollIntoView, which the anchored view uses to reveal its target.
const originalScrollIntoView = Element.prototype.scrollIntoView;

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  Element.prototype.scrollIntoView = originalScrollIntoView;
  delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  vi.clearAllMocks();
  vi.restoreAllMocks();
});

function backwardPagination(key: TimelineKey, state: PaginationState): CoreEventPayload {
  return {
    kind: "Timeline",
    event: {
      PaginationStateChanged: {
        request_id: null,
        key,
        direction: "Backward",
        state,
        prepend_expected: null
      }
    }
  };
}

/** Mounts the anchored main pane an event jump opens, with an overflowing page. */
async function renderAnchoredView(paginateBackwards: (key: TimelineKey) => Promise<void>) {
  Element.prototype.scrollIntoView = () => undefined;
  let emit: (payload: CoreEventPayload) => void = () => undefined;
  const onReturnToLive = vi.fn();
  render(
    <TimelineView
      timelineKey={FOCUSED_KEY}
      roomId="!room:example.invalid"
      transport={baseTransport({
        listenCoreEvents(listener) {
          emit = listener;
          return () => undefined;
        },
        paginateBackwards
      })}
      onReply={vi.fn()}
      initialTargetEventId="$anchor:example.invalid"
      isAnchored
      onReturnToLive={onReturnToLive}
      liveLatestEventId="$live:example.invalid"
    />
  );
  const timeline = await screen.findByTestId("timeline-view");
  Object.defineProperty(timeline, "clientHeight", { value: 500, configurable: true });
  Object.defineProperty(timeline, "scrollHeight", { value: 2_000, configurable: true });
  Object.defineProperty(timeline, "scrollTop", { value: 700, writable: true, configurable: true });
  act(() => {
    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: FOCUSED_KEY,
          generation: 1,
          items: [
            message("$context-start:example.invalid", "Start of the context page"),
            message("$anchor:example.invalid", "Jump target"),
            message("$context-end:example.invalid", "End of the context page")
          ]
        }
      }
    });
  });
  await screen.findByText("Jump target");
  const scrollToTop = () => {
    timeline.scrollTop = 0;
    fireEvent.wheel(timeline, { deltaY: -120 });
    fireEvent.scroll(timeline);
  };
  return { emit: (payload: CoreEventPayload) => act(() => emit(payload)), scrollToTop, onReturnToLive };
}

describe("TimelineView anchored backward pagination", () => {
  it("asks Core for older history when the reader scrolls up from a jump target (#1675)", async () => {
    // The production desktop transport, as the main pane uses it.
    (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
    const transport = createTauriTimelineTransport();
    expect(transport).not.toBeNull();
    const view = await renderAnchoredView((key) => transport!.paginateBackwards(key));

    view.scrollToTop();

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("paginate_timeline", {
        timelineKey: FOCUSED_KEY,
        direction: "Backward"
      });
    });
    expect(view.onReturnToLive).not.toHaveBeenCalled();
  });

  it("keeps one older page in flight and stops at the start of the room", async () => {
    const paginateBackwards = vi.fn(async (_key: TimelineKey) => undefined);
    const view = await renderAnchoredView(paginateBackwards);

    view.scrollToTop();
    await waitFor(() => expect(paginateBackwards).toHaveBeenCalledWith(FOCUSED_KEY));
    view.emit(backwardPagination(FOCUSED_KEY, "Paginating"));
    view.scrollToTop();
    await act(async () => Promise.resolve());
    expect(paginateBackwards).toHaveBeenCalledTimes(1);

    view.emit(backwardPagination(FOCUSED_KEY, "Idle"));
    view.emit({
      kind: "Timeline",
      event: {
        ItemsUpdated: {
          key: FOCUSED_KEY,
          generation: 1,
          batch_id: 1,
          diffs: [{ PushFront: { item: message("$older:example.invalid", "Older message") } }]
        }
      }
    });
    await screen.findByText("Older message");
    view.scrollToTop();
    await waitFor(() => expect(paginateBackwards).toHaveBeenCalledTimes(2));

    view.emit(backwardPagination(FOCUSED_KEY, "Paginating"));
    view.emit(backwardPagination(FOCUSED_KEY, "EndReached"));
    view.scrollToTop();
    await act(async () => Promise.resolve());
    expect(paginateBackwards).toHaveBeenCalledTimes(2);
  });

  it("asks again for a page Core declined during gap repair only after the release", async () => {
    const paginateBackwards = vi.fn(async (_key: TimelineKey) => undefined);
    const view = await renderAnchoredView(paginateBackwards);

    view.scrollToTop();
    await waitFor(() => expect(paginateBackwards).toHaveBeenCalledTimes(1));
    // An Idle terminal without Paginating is Core declining the page.
    view.emit(backwardPagination(FOCUSED_KEY, "Idle"));
    view.scrollToTop();
    await act(async () => Promise.resolve());
    expect(paginateBackwards).toHaveBeenCalledTimes(1);

    view.emit({
      kind: "Timeline",
      event: { GapRepairReleased: { key: FOCUSED_KEY, actor_generation: 1, generation: 2 } }
    });
    view.scrollToTop();
    await waitFor(() => expect(paginateBackwards).toHaveBeenCalledTimes(2));
  });
});
