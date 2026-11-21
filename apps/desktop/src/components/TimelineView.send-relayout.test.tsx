// @vitest-environment jsdom
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { CoreEventPayload, TimelineItem, TimelineItemId } from "../domain/coreEvents";
import { KEY, baseTransport, message, navigationSnapshot } from "./timelineViewTestSupport";
import { setActiveLocaleProfile } from "../i18n/messages";
import { TimelineView, clearTimelineViewportSessionMemoryForTests } from "./TimelineView";

const OWN = "@alice:example.invalid";
const READ = "$read:example.invalid";
const SENT = "$sent:example.invalid";

function echo(
  transactionId: string,
  sendState: "sending" | "notSent" = "sending"
): TimelineItem {
  return {
    ...message(`$echo-${transactionId}:example.invalid`, "Just sent"),
    id: { Transaction: { transaction_id: transactionId } } as TimelineItemId,
    sender: OWN,
    timestamp_ms: 1_800_000_002_000,
    send_state: sendState === "sending" ? { kind: "sending" } : { kind: "notSent", reason: "recoverable" }
  };
}

function ownRead(): TimelineItem {
  return { ...message(READ, "Previous own message"), sender: OWN };
}

function confirmed(): TimelineItem {
  return { ...message(SENT, "Just sent"), sender: OWN, timestamp_ms: 1_800_000_002_000 };
}

/** The row the "Read up to here" divider follows, by DOM order. */
function markerRowId(): string | null {
  return (
    screen.queryByRole("separator", { name: "Read up to here" })?.previousElementSibling?.getAttribute(
      "data-item-id"
    ) ?? null
  );
}

function renderTimeline() {
  let emit: (payload: CoreEventPayload) => void = () => undefined;
  const transport = baseTransport({
    listenCoreEvents(listener) {
      emit = listener;
      return () => undefined;
    }
  });
  render(
    <TimelineView
      timelineKey={KEY}
      roomId="!room:example.invalid"
      transport={transport}
      currentUserId={OWN}
      onReply={vi.fn()}
    />
  );
  return (payload: CoreEventPayload) => act(() => emit(payload));
}

const nav = (anchor: string): CoreEventPayload => ({
  kind: "Timeline",
  event: {
    NavigationUpdated: {
      key: KEY,
      snapshot: navigationSnapshot({
        read_marker_event_id: anchor,
        read_marker_display_event_id: anchor,
        local_viewed_event_id: anchor,
        server_confirmed_read_event_id: anchor
      })
    }
  }
});

const diffs = (batch: number, list: unknown[]): CoreEventPayload => ({
  kind: "Timeline",
  event: {
    ItemsUpdated: {
      key: KEY,
      generation: 1,
      batch_id: batch,
      diffs: list
    } as never
  }
});

/** A presentation-only row: the day heading Core inserts at a local-day boundary. */
function dateDivider(timestampMs: number): TimelineItem {
  return {
    ...message(`$date-source:example.invalid`, ""),
    id: { Synthetic: { synthetic_id: `date-divider-${timestampMs}` } },
    sender: null,
    body: null
  };
}

function initial(items: TimelineItem[]): CoreEventPayload {
  return {
    kind: "Timeline",
    event: { InitialItems: { request_id: null, key: KEY, generation: 1, items } } as never
  };
}

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  setActiveLocaleProfile("en", "none");
});

it("keeps the read-marker divider on the user's in-flight message, so a send lays out once", async () => {
  const emit = renderTimeline();
  await screen.findByTestId("timeline-view");
  emit(initial([ownRead()]));
  emit(nav(READ));
  expect(markerRowId()).toBe(READ);

  // The local echo cannot take the anchor until the server names it. The
  // divider must still sit below it, or the row renders once below the divider
  // and re-renders above it on confirmation (#2012).
  emit(diffs(2, [{ PushBack: { item: echo("1") } }]));
  expect(markerRowId()).toBe("txn:1");
  expect(screen.getByText("Just sent").closest(".message")?.className).toContain("is-continuation");

  // Confirmation arrives as two Core events: the item diff, then the navigation
  // snapshot that names the new event. Core publishes them in one batch, so the
  // frames in between are not painted; the e2e spec samples those frames.
  emit(diffs(3, [{ Set: { index: 1, item: confirmed() } }]));
  emit(nav(SENT));
  expect(markerRowId()).toBe(SENT);
  expect(screen.getByText("Just sent").closest(".message")?.className).toContain("is-continuation");
});

it("stops at another sender's message instead of jumping over it", async () => {
  const emit = renderTimeline();
  await screen.findByTestId("timeline-view");
  emit(initial([ownRead(), message("$other:example.invalid", "Someone else")]));
  emit(nav(READ));
  expect(markerRowId()).toBe(READ);
  // The walk stays inside the send: it advances onto own in-flight messages and
  // past presentation rows, not across other people's messages. Core moves the
  // divider onto the confirmed echo; the renderer leaves that commit to Core
  // rather than guessing a position Core may not take.
  emit(diffs(2, [{ PushBack: { item: echo("1") } }]));
  expect(markerRowId()).toBe(READ);
  // Once Core names that message as the anchor, the walk resumes and the echo
  // takes the divider as usual.
  emit(nav("$other:example.invalid"));
  expect(markerRowId()).toBe("txn:1");
});

it("advances past a run of in-flight messages", async () => {
  const emit = renderTimeline();
  await screen.findByTestId("timeline-view");
  emit(initial([ownRead()]));
  emit(nav(READ));
  emit(diffs(2, [{ PushBack: { item: echo("1") } }, { PushBack: { item: echo("2") } }]));
  expect(markerRowId()).toBe("txn:2");
});

it("leaves a failed send on the previous message, which never takes the anchor", async () => {
  const emit = renderTimeline();
  await screen.findByTestId("timeline-view");
  emit(initial([ownRead()]));
  emit(nav(READ));
  emit(diffs(2, [{ PushBack: { item: echo("1", "notSent") } }]));
  expect(markerRowId()).toBe(READ);
});

it("skips a date divider between the marker and the in-flight send (#2021)", async () => {
  const emit = renderTimeline();
  await screen.findByTestId("timeline-view");
  // The first message of a new local day: the heading sits between the old
  // anchor and the echo, and neither is a candidate for the confirmed anchor.
  emit(initial([ownRead(), dateDivider(1_800_000_002_000)]));
  emit(nav(READ));
  expect(markerRowId()).toBe(READ);
  expect(document.querySelectorAll(".timeline-date-divider").length).toBeGreaterThan(0);

  emit(diffs(2, [{ PushBack: { item: echo("1") } }]));
  // The day heading still breaks the continuation run (a separate rule); what
  // matters here is that it does not capture the read-marker anchor.
  expect(markerRowId()).toBe("txn:1");
});

it("skips a timeline gap between the marker and the in-flight send (#2021)", async () => {
  const emit = renderTimeline();
  await screen.findByTestId("timeline-view");
  emit(initial([ownRead()]));
  emit(nav(READ));
  emit({
    kind: "Timeline",
    event: {
      GapPositionsUpdated: {
        key: KEY,
        actor_generation: 1,
        generation: 1,
        positions: [{ id: { topology_revision: 1, ordinal: 0 }, before_item_index: 1 }]
      }
    } as never
  });
  expect(markerRowId()).toBe(READ);
  expect(screen.getByTestId("timeline-gap-row")).toBeDefined();

  emit(diffs(2, [{ PushBack: { item: echo("1") } }]));
  expect(markerRowId()).toBe("txn:1");
});
