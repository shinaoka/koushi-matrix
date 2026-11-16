// @vitest-environment jsdom
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { CoreEventPayload, TimelineItem } from "../domain/coreEvents";
import type { LiveSignalsState } from "../domain/types";
import { KEY, baseTransport, message } from "./timelineViewTestSupport";
import { TimelineView, clearTimelineViewportSessionMemoryForTests } from "./TimelineView";

// Every TimelineItemRow render (and every timestamp label) calls this hook, so
// the call count is a render counter for the timeline rows (#1892).
const renders = vi.hoisted(() => ({ count: 0 }));
vi.mock("./timeline/useCalendarPresentation", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./timeline/useCalendarPresentation")>();
  return {
    ...actual,
    useCalendarPresentation: () => {
      renders.count += 1;
      actual.useCalendarPresentation();
    }
  };
});

const ROOM = "!room:example.invalid";
const ROW_COUNT = 40;

function items(): TimelineItem[] {
  return Array.from({ length: ROW_COUNT }, (_, index) => ({
    ...message(`$row-${index}:example.invalid`, `Synthetic row ${index}`),
    sender: index % 2 === 0 ? "@bob:example.invalid" : "@carol:example.invalid",
    timestamp_ms: 1_800_000_000_000 + index * 60_000
  }));
}

function liveSignals(typing: string[], presence: LiveSignalsState["presence"] = {}): LiveSignalsState {
  return {
    presence,
    rooms: {
      [ROOM]: {
        receipts_by_event: {},
        focused_receipts_by_event: {},
        thread_receipts_by_event: {},
        fully_read_event_id: null,
        typing_user_ids: typing,
        typing_users: []
      }
    }
  } as unknown as LiveSignalsState;
}

let emit: (payload: CoreEventPayload) => void = () => undefined;
const transport = baseTransport({
  listenCoreEvents(listener) {
    emit = listener;
    return () => undefined;
  }
});
const onReply = vi.fn();

function view(signals: LiveSignalsState) {
  return <TimelineView timelineKey={KEY} roomId={ROOM} transport={transport} onReply={onReply} liveSignals={signals} />;
}

async function mountRows() {
  const result = render(view(liveSignals([])));
  act(() => emit({ kind: "Timeline", event: { InitialItems: { request_id: null, key: KEY, generation: 1, items: items() } } }));
  await screen.findByText(`Synthetic row ${ROW_COUNT - 1}`);
  return result;
}

beforeEach(() => {
  renders.count = 0;
});

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
});

it("re-renders no timeline row when a live signal unrelated to the rows changes", async () => {
  const { rerender } = await mountRows();
  renders.count = 0;

  // Someone starts typing and an unrelated user's presence changes: the
  // shell passes new liveSignals, but no row's own inputs changed.
  rerender(view(liveSignals(["@dave:example.invalid"], { "@dave:example.invalid": "online" })));

  expect(renders.count).toBe(0);
});

it("re-renders only the row whose item changed", async () => {
  await mountRows();
  renders.count = 0;

  // Edit a row next to the live edge: it is mounted whether or not the
  // timeline windows its rows (#1890).
  const index = ROW_COUNT - 2;
  const updated: TimelineItem = { ...items()[index], body: `Synthetic row ${index} edited`, is_edited: true };
  act(() =>
    emit({
      kind: "Timeline",
      event: { ItemsUpdated: { key: KEY, generation: 1, batch_id: 1, diffs: [{ Set: { index, item: updated } }] } }
    })
  );

  await screen.findByText(`Synthetic row ${index} edited`);
  // One row (its body plus at most its own timestamp labels), not all ROW_COUNT rows.
  expect(renders.count).toBeGreaterThan(0);
  expect(renders.count).toBeLessThanOrEqual(3);
});
