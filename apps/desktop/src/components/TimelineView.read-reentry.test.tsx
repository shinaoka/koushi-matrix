// @vitest-environment jsdom

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { threadTimelineKey, type TimelineKey } from "../domain/coreEvents";
import { applyGlobalResync, applyTimelineEvent, createTimelineStore, type TimelineStoreState } from "../domain/timelineStore";
import { setActiveLocaleProfile } from "../i18n/messages";
import { TimelineView, clearTimelineViewportSessionMemoryForTests } from "./TimelineView";
import { baseTransport, message, navigationSnapshot } from "./timelineViewTestSupport";

const ROOT = "$root:example.invalid";
const REPLY = "$reply:example.invalid";
const LATEST = "$latest:example.invalid";
const KEY = threadTimelineKey("@reader:example.invalid", "!room:example.invalid", ROOT);
const transport = baseTransport({});

function initial(store = createTimelineStore(), key = KEY, actor = 1) {
  const root = "Thread" in key.kind ? key.kind.Thread.root_event_id : ROOT;
  return applyTimelineEvent(store, { InitialItems: {
    key, generation: 1, actor_generation: actor, request_id: null,
    items: [message(root, "Root"), ...[REPLY, LATEST].map((id) => ({
      ...message(id, "Synthetic edited reply"), thread_root: root, is_edited: true,
      reactions: [{ key: "👍", count: 1, reacted_by_me: false, my_reaction_event_id: null, sender_preview: [] }]
    }))]
  }});
}

function navigation(store: TimelineStoreState, eventId: string, key = KEY) {
  return applyTimelineEvent(store, { NavigationUpdated: { key, snapshot: navigationSnapshot({
    read_marker_event_id: eventId, read_marker_display_event_id: eventId,
    server_confirmed_read_event_id: eventId
  }) }});
}

function view(store: TimelineStoreState, key: TimelineKey = KEY) {
  return <TimelineView timelineKey={key} roomId="!room:example.invalid"
    transport={transport} timelineStore={store} onReply={vi.fn()}
    liveSignals={{ presence: {}, rooms: { "!room:example.invalid": {
      fully_read_event_id: ROOT, typing_user_ids: [], typing_users: [], focused_receipts_by_event: {}, thread_receipts_by_event: {}, receipts_by_event: {}
    } } }} />;
}

function markerEventId() {
  return screen.queryByRole("separator", { name: "Read up to here" })
    ?.previousElementSibling?.getAttribute("data-event-id") ?? null;
}

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  setActiveLocaleProfile("en", "none");
});

it("does not substitute the room marker while thread navigation is pending", () => {
  render(view(initial()));
  expect(markerEventId()).toBeNull();
});

it("renders navigation delivered before mounting and after reopening the panel", () => {
  let store = navigation(initial(), REPLY);
  const mounted = render(view(store));
  expect(markerEventId()).toBe(REPLY);
  mounted.unmount();
  // The App listener keeps receiving Rust snapshots while the panel is closed.
  store = navigation(store, LATEST);
  store = initial(store); // unchanged actor replays rows before navigation
  render(view(store));
  expect(markerEventId()).toBe(LATEST);
});

it("keeps each account's navigation separate across key changes", () => {
  const second = threadTimelineKey("@second:example.invalid", "!room:example.invalid", ROOT);
  const store = navigation(initial(navigation(initial(), LATEST), second), REPLY, second);
  const mounted = render(view(store));
  expect(markerEventId()).toBe(LATEST);
  mounted.rerender(view(store, second));
  expect(markerEventId()).toBe(REPLY);
  mounted.rerender(view(store));
  expect(markerEventId()).toBe(LATEST);
});

it("drops the old divider when a replacement actor initializes the same thread", () => {
  const store = initial(navigation(initial(), LATEST), KEY, 2);
  render(view(store));
  expect(markerEventId()).toBeNull();
});

it.each(["key", "global"])("clears navigation on %s resync until Core replays it", (scope) => {
  let store = navigation(initial(), LATEST);
  const mounted = render(view(store));
  expect(markerEventId()).toBe(LATEST);
  store = scope === "global" ? applyGlobalResync(store) : applyTimelineEvent(store, {
    ResyncRequired: { key: KEY, reason: "QueueOverflow" }
  });
  store = navigation(store, LATEST); // navigation cannot initialize a resyncing list
  store = initial(store);
  mounted.rerender(view(store));
  expect(markerEventId()).toBeNull();
  mounted.rerender(view(navigation(store, LATEST)));
  expect(markerEventId()).toBe(LATEST);
});

it("clears navigation on changed timeline generation within the same actor", () => {
  let store = navigation(initial(), LATEST);
  store = applyTimelineEvent(store, { InitialItems: {
    key: KEY, generation: 2, actor_generation: 1, request_id: null,
    items: [message(ROOT, "Root")]
  }});
  render(view(store));
  expect(markerEventId()).toBeNull();
});

it("keeps distinct roots in one room separate on panel switches", () => {
  const otherRoot = "$other-root:example.invalid";
  const otherKey = threadTimelineKey(KEY.account_key, "!room:example.invalid", otherRoot);
  const store = navigation(initial(navigation(initial(), LATEST), otherKey), otherRoot, otherKey);
  const mounted = render(view(store));
  expect(markerEventId()).toBe(LATEST);
  mounted.rerender(view(store, otherKey));
  expect(markerEventId()).toBe(otherRoot);
  mounted.rerender(view(store));
  expect(markerEventId()).toBe(LATEST);
});

it("does not initialize a timeline from a navigation event alone", () => {
  const store = navigation(createTimelineStore(), LATEST);
  expect(store.keys.size).toBe(0);
  render(view(initial(store)));
  expect(markerEventId()).toBeNull();
});
