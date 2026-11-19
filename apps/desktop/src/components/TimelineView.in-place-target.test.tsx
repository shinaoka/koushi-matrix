// @vitest-environment jsdom

// #1981: Core settles a reply-quote jump in place when the quoted original is
// already a row of the live room timeline. The live view scrolls that row into
// view and emphasizes it, once per navigation generation, and never leaves the
// live timeline. Everything here is synthetic.

import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { roomTimelineKey, type CoreEventPayload } from "../domain/coreEvents";
import { baseTransport, message } from "./timelineViewTestSupport";
import { TimelineView, clearTimelineViewportSessionMemoryForTests } from "./TimelineView";

const ROOM = "!reply-in-place-room:example.invalid";
const ROOM_KEY = roomTimelineKey("@alice:example.invalid", ROOM);
const ORIGINAL = "$quoted-original:example.invalid";

let restoreScrollIntoView = () => undefined;
let restoreLayout = () => undefined;

afterEach(() => {
  restoreLayout();
  restoreLayout = () => undefined;
  restoreScrollIntoView();
  restoreScrollIntoView = () => undefined;
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  vi.restoreAllMocks();
});

type InPlaceTarget = { eventId: string; generation: number } | null;

async function renderLiveRoom(target: InPlaceTarget, items = defaultItems()) {
  let emit: (payload: CoreEventPayload) => void = () => undefined;
  const onReturnToLive = vi.fn();
  const scrolled: string[] = [];
  // jsdom has no layout, so it does not implement scrollIntoView.
  const original = Element.prototype.scrollIntoView;
  Element.prototype.scrollIntoView = function (this: Element) {
    scrolled.push(this.getAttribute("data-content-event-id") ?? "?");
  };
  restoreScrollIntoView = () => {
    Element.prototype.scrollIntoView = original;
  };
  const props = {
    timelineKey: ROOM_KEY,
    roomId: ROOM,
    transport: baseTransport({
      listenCoreEvents(listener) {
        emit = listener;
        return () => undefined;
      }
    }),
    onReply: vi.fn(),
    onReturnToLive
  };
  const view = render(<TimelineView {...props} inPlaceTarget={target} />);
  await screen.findByTestId("timeline-view");
  act(() => {
    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          cause_request_id: null,
          key: ROOM_KEY,
          actor_generation: 1,
          generation: 1,
          items
        }
      }
    });
  });
  return {
    scrolled,
    onReturnToLive,
    emit: (payload: CoreEventPayload) => act(() => emit(payload)),
    rerender: (next: InPlaceTarget) =>
      view.rerender(<TimelineView {...props} inPlaceTarget={next} />)
  };
}

function defaultItems() {
  return [
    message("$earlier:example.invalid", "Earlier message"),
    message(ORIGINAL, "Quoted original"),
    message("$reply:example.invalid", "The reply that quotes it")
  ];
}

function originalRow(): HTMLElement {
  const row = document.querySelector<HTMLElement>(`[data-content-event-id="${ORIGINAL}"]`);
  if (!row) {
    throw new Error("the quoted original row is not mounted");
  }
  return row;
}

describe("in-place reply-quote target (#1981)", () => {
  it("scrolls the live row into view and emphasizes it without leaving the live timeline", async () => {
    const view = await renderLiveRoom({ eventId: ORIGINAL, generation: 1 });
    await screen.findByText("Quoted original");

    expect(view.scrolled).toEqual([ORIGINAL]);
    expect(originalRow().classList.contains("pinned-target")).toBe(true);
    expect(view.onReturnToLive).not.toHaveBeenCalled();
  });

  it("scrolls once per navigation generation", async () => {
    const view = await renderLiveRoom({ eventId: ORIGINAL, generation: 1 });
    await screen.findByText("Quoted original");
    view.rerender({ eventId: ORIGINAL, generation: 1 });
    view.emit({
      kind: "Timeline",
      event: {
        ItemsUpdated: {
          key: ROOM_KEY,
          generation: 1,
          batch_id: 1,
          diffs: [{ PushBack: { item: message("$newer:example.invalid", "A newer message") } }]
        }
      }
    });
    await screen.findByText("A newer message");
    expect(view.scrolled).toEqual([ORIGINAL]);

    // A second click on the same quote is a newer navigation and scrolls again.
    view.rerender({ eventId: ORIGINAL, generation: 2 });
    expect(view.scrolled).toEqual([ORIGINAL, ORIGINAL]);
  });

  it("waits for a target row the renderer has not applied yet", async () => {
    const view = await renderLiveRoom({ eventId: ORIGINAL, generation: 1 }, [
      message("$earlier:example.invalid", "Earlier message")
    ]);
    await screen.findByText("Earlier message");
    expect(view.scrolled).toEqual([]);

    view.emit({
      kind: "Timeline",
      event: {
        ItemsUpdated: {
          key: ROOM_KEY,
          generation: 1,
          batch_id: 1,
          diffs: [{ PushBack: { item: message(ORIGINAL, "Quoted original") } }]
        }
      }
    });
    await screen.findByText("Quoted original");
    expect(view.scrolled).toEqual([ORIGINAL]);
  });

  // #1987: jsdom has no layout, so this case supplies one. The row lives at content
  // offset 1000 inside a 200 px band, and its box follows the container's scroll, so
  // the settle has to keep correcting until the whole row is inside the band instead
  // of stopping one row above it.
  it("keeps recentring until the whole row is inside the visible band (#1987)", async () => {
    const ROW_CONTENT_TOP = 1000;
    const BAND_HEIGHT = 200;
    const ROW_HEIGHT = 72;
    const originalRect = Element.prototype.getBoundingClientRect;
    const originalClientHeight = Object.getOwnPropertyDescriptor(
      HTMLElement.prototype,
      "clientHeight"
    );
    Element.prototype.getBoundingClientRect = function (this: Element) {
      if (this.getAttribute("data-content-event-id") === ORIGINAL) {
        const container = document.querySelector<HTMLElement>('[data-testid="timeline-view"]');
        const top = ROW_CONTENT_TOP - (container?.scrollTop ?? 0);
        return { top, bottom: top + ROW_HEIGHT, height: ROW_HEIGHT } as DOMRect;
      }
      return { top: 0, bottom: BAND_HEIGHT, height: BAND_HEIGHT } as DOMRect;
    };
    Object.defineProperty(HTMLElement.prototype, "clientHeight", {
      configurable: true,
      get() {
        return BAND_HEIGHT;
      }
    });
    restoreLayout = () => {
      Element.prototype.getBoundingClientRect = originalRect;
      if (originalClientHeight) {
        Object.defineProperty(HTMLElement.prototype, "clientHeight", originalClientHeight);
      }
    };

    const view = await renderLiveRoom({ eventId: ORIGINAL, generation: 1 });
    await screen.findByText("Quoted original");

    const container = screen.getByTestId("timeline-view");
    // Centred: the row's top moves to (band - row) / 2 = 64.
    expect(container.scrollTop).toBe(ROW_CONTENT_TOP - 64);
    expect(
      Math.abs(originalRow().getBoundingClientRect().top - (BAND_HEIGHT - ROW_HEIGHT) / 2)
    ).toBeLessThan(1);

    // The generation is settled, so a later arrival does not move the reader again.
    const settled = container.scrollTop;
    view.emit({
      kind: "Timeline",
      event: {
        ItemsUpdated: {
          key: ROOM_KEY,
          generation: 1,
          batch_id: 2,
          diffs: [{ PushBack: { item: message("$newer-after-settle:example.invalid", "Later") } }]
        }
      }
    });
    await screen.findByText("Later");
    expect(container.scrollTop).toBe(settled);
  });

  it("does nothing without an in-place target", async () => {
    const view = await renderLiveRoom(null);
    await screen.findByText("Quoted original");
    expect(view.scrolled).toEqual([]);
    expect(originalRow().classList.contains("pinned-target")).toBe(false);
  });
});
