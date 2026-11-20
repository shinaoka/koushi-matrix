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

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  setActiveLocaleProfile("en", "none");
});

it("keeps the read-marker divider on the user's in-flight message, so a send lays out once", async () => {
  const emit = renderTimeline();
  await screen.findByTestId("timeline-view");
  emit({
    kind: "Timeline",
    event: {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [ownRead()]
      }
    } as never
  });
  emit(nav(READ));
  expect(markerRowId()).toBe(READ);

  // The local echo cannot take the anchor until the server names it. The
  // divider must still sit below it, or the row renders once below the divider
  // and re-renders above it on confirmation (#2012).
  emit(diffs(2, [{ PushBack: { item: echo("1") } }]));
  expect(markerRowId()).toBe("txn:1");
  expect(screen.getByText("Just sent").closest(".message")?.className).toContain("is-continuation");

  // Confirmation: the row gains an event id and Core moves the anchor onto it.
  emit(diffs(3, [{ Set: { index: 1, item: confirmed() } }]));
  emit(nav(SENT));
  expect(markerRowId()).toBe(SENT);
});

it("advances past a run of in-flight messages", async () => {
  const emit = renderTimeline();
  await screen.findByTestId("timeline-view");
  emit({
    kind: "Timeline",
    event: {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [ownRead()]
      }
    } as never
  });
  emit(nav(READ));
  emit(diffs(2, [{ PushBack: { item: echo("1") } }, { PushBack: { item: echo("2") } }]));
  expect(markerRowId()).toBe("txn:2");
});

it("leaves a failed send on the previous message, which never takes the anchor", async () => {
  const emit = renderTimeline();
  await screen.findByTestId("timeline-view");
  emit({
    kind: "Timeline",
    event: {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [ownRead()]
      }
    } as never
  });
  emit(nav(READ));
  emit(diffs(2, [{ PushBack: { item: echo("1", "notSent") } }]));
  expect(markerRowId()).toBe(READ);
});
