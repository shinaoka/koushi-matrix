// @vitest-environment jsdom
/**
 * #2041 P2: consecutive same-kind notices share one display line plus a count.
 *
 * Rust decides which rows form a group and carries `group_count`,
 * `group_position` and `group_id`; these tests pin what the renderer does with
 * them — draw the head row alone, open the group on the disclosure button, and
 * forget the open group when the view is replaced (the pane keys this view by
 * room, so a room switch is a remount).
 */
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { CoreEventPayload, TimelineItem } from "../domain/coreEvents";
import { roomTimelineKey } from "../domain/coreEvents";
import { setActiveLocaleProfile, t } from "../i18n/messages";
import { TimelineView, clearTimelineViewportSessionMemoryForTests } from "./TimelineView";
import { KEY, baseTransport, message } from "./timelineViewTestSupport";

// jsdom has no layout: a navigation target still asks its row to scroll.
beforeEach(() => {
  Element.prototype.scrollIntoView = vi.fn();
});

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  setActiveLocaleProfile("en", "none");
});

const GROUP_ID = "notice-group:$profile-0:example.invalid";
const ROOM_ID = "!room:example.invalid";
const OTHER_ROOM_ID = "!other:example.invalid";

/** Four consecutive profile-avatar notices, as Core groups them. */
function noticeGroup(): TimelineItem[] {
  return [0, 1, 2, 3].map((index) => ({
    ...message(`$profile-${index}:example.invalid`, "changed their profile picture"),
    sender_label: "Member 1",
    message_kind: "notice" as const,
    notice_i18n: { key: "timeline.notice.profileAvatarChanged" as const },
    timestamp_ms: 1_800_000_000_000 + index * 1_000,
    display_metadata: {
      row_id: `$profile-${index}:example.invalid`,
      kind: { kind: "event" as const },
      content_event_id: `$profile-${index}:example.invalid`,
      activity_event_id: `$profile-${index}:example.invalid`,
      display_timestamp_ms: 1_800_000_000_000 + index * 1_000,
      group_count: 4,
      group_position: index,
      group_id: GROUP_ID
    }
  }));
}

function timelineHarness() {
  let emit: (payload: CoreEventPayload) => void = () => undefined;
  const transport = baseTransport({
    listenCoreEvents(listener) {
      emit = listener;
      return () => undefined;
    }
  });
  const view = (timelineKey: typeof KEY, roomId: string, key?: string) => (
    <TimelineView
      key={key}
      timelineKey={timelineKey}
      roomId={roomId}
      transport={transport}
      onReply={vi.fn()}
    />
  );
  const seed = (items: TimelineItem[], timelineKey: typeof KEY) =>
    act(() =>
      emit({
        kind: "Timeline",
        event: { InitialItems: { request_id: null, key: timelineKey, generation: 1, items } }
      })
    );
  return { transport, view, seed };
}

function mount(items: TimelineItem[], key = KEY) {
  const harness = timelineHarness();
  const rendered = render(harness.view(key, ROOM_ID, ROOM_ID));
  harness.seed(items, key);
  return rendered;
}

function visibleNoticeRows(): HTMLElement[] {
  return Array.from(document.querySelectorAll<HTMLElement>(".message-notice-row"));
}

describe("#2041 notice groups", () => {
  it("draws one line with the count and opens to one row per notice", () => {
    mount(noticeGroup());

    const rows = visibleNoticeRows();
    expect(rows).toHaveLength(1);
    const head = rows[0];
    expect(head.getAttribute("data-event-id")).toBe("$profile-0:example.invalid");

    const toggle = head.querySelector<HTMLElement>(".message-notice-group-toggle");
    expect(toggle).toBeTruthy();
    expect(toggle!.textContent).toContain("4");
    expect(toggle!.getAttribute("aria-expanded")).toBe("false");
    expect(toggle!.getAttribute("aria-label")).toBe(
      t("timeline.noticeGroupToggle", { count: 4 })
    );

    fireEvent.click(toggle!);

    const expanded = visibleNoticeRows();
    expect(expanded).toHaveLength(4);
    expect(expanded.map((row) => row.getAttribute("data-event-id"))).toEqual([
      "$profile-0:example.invalid",
      "$profile-1:example.invalid",
      "$profile-2:example.invalid",
      "$profile-3:example.invalid"
    ]);
    // Each event keeps its own time, so three notices in one burst are still
    // told apart. A collapsed line shows none: it is one information line.
    for (const [index, row] of expanded.entries()) {
      const timestamp = row.querySelector("time");
      expect(timestamp).toBeTruthy();
      expect(timestamp!.getAttribute("datetime")).toBe(
        new Date(1_800_000_000_000 + index * 1_000).toISOString()
      );
    }

    fireEvent.click(toggle!);
    expect(visibleNoticeRows()).toHaveLength(1);
    expect(
      visibleNoticeRows()[0]
        .querySelector(".message-notice-group-toggle")!
        .getAttribute("aria-expanded")
    ).toBe("false");
  });

  it("starts collapsed again when the view is replaced, as a room switch does", () => {
    const harness = timelineHarness();
    const rendered = render(harness.view(KEY, ROOM_ID, ROOM_ID));
    harness.seed(noticeGroup(), KEY);
    fireEvent.click(
      visibleNoticeRows()[0].querySelector<HTMLElement>(".message-notice-group-toggle")!
    );
    expect(visibleNoticeRows()).toHaveLength(4);

    // The pane renders this view with `key={roomId}`, so entering another room
    // replaces the component and its session-only expansion state with it. The
    // other room holds the same four notices, so only the reset can collapse
    // them.
    const otherKey = roomTimelineKey("@alice:example.invalid", OTHER_ROOM_ID);
    // The pane keeps this view as a single child and keys it by room, so the
    // switch replaces the component instance — and its expansion state with it.
    const host = (timelineKey: typeof KEY, roomId: string) => (
      <div>{harness.view(timelineKey, roomId, roomId)}</div>
    );
    rendered.rerender(host(otherKey, OTHER_ROOM_ID));
    harness.seed(noticeGroup(), otherKey);

    expect(visibleNoticeRows()).toHaveLength(1);
    expect(
      visibleNoticeRows()[0]
        .querySelector(".message-notice-group-toggle")!
        .getAttribute("aria-expanded")
    ).toBe("false");
  });

  it("reports a group open when a navigation target sits inside it", () => {
    const harness = timelineHarness();
    render(
      <TimelineView
        key={ROOM_ID}
        timelineKey={KEY}
        roomId={ROOM_ID}
        transport={harness.transport}
        inPlaceTarget={{ eventId: "$profile-2:example.invalid", generation: 1 }}
        onReply={vi.fn()}
      />
    );
    harness.seed(noticeGroup(), KEY);

    // The target row has to exist to be scrolled to, so the group is drawn open
    // — and its head must say so, or the disclosure button would answer a click
    // with no visible change.
    expect(visibleNoticeRows()).toHaveLength(4);
    const toggle = visibleNoticeRows()[0].querySelector<HTMLElement>(
      ".message-notice-group-toggle"
    );
    expect(toggle!.getAttribute("aria-expanded")).toBe("true");
    expect(
      visibleNoticeRows()[2].querySelector("time")!.getAttribute("datetime")
    ).toBe(new Date(1_800_000_002_000).toISOString());
  });

  it("shows no count on a single notice", () => {
    mount([
      {
        ...message("$single:example.invalid", "set the room topic to Topic"),
        sender_label: "Member 1",
        message_kind: "notice" as const,
        notice_i18n: { key: "timeline.notice.roomTopicSet" as const, value: "Topic" }
      }
    ]);

    expect(visibleNoticeRows()).toHaveLength(1);
    expect(screen.queryByRole("button", { name: /similar notices/ })).toBeNull();
  });
});
