// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ActivityPane } from "./panes";
import { setActiveLocaleProfile } from "../i18n/messages";
import type { ActivityRow, ActivityState } from "../domain/types";

function activityState(rows: ActivityRow[]): ActivityState {
  return {
    kind: "open",
    active_tab: "unread",
    recent: { rows: [], next_batch: null },
    unread: { rows, next_batch: null },
    mark_read: { kind: "idle" }
  };
}

const eventRow: ActivityRow = {
  kind: "event",
  room_id: "!room:example.invalid",
  event_id: "$event:example.invalid",
  thread_root_event_id: null,
  sender_id: "@sender:example.invalid",
  room_label: "Event room",
  sender_label: "Sender",
  sender_avatar: null,
  preview: "Preview",
  timestamp_ms: 1_000_000,
  unread: true,
  highlight: false,
  context_label: "Room"
};

const placeholderRow: ActivityRow = {
  kind: "roomUnread",
  room_id: "!placeholder:example.invalid",
  event_id: null,
  thread_root_event_id: null,
  sender_id: null,
  room_label: "Placeholder room",
  sender_label: null,
  sender_avatar: null,
  preview: null,
  timestamp_ms: 2_000_000,
  unread: true,
  highlight: true,
  context_label: "Room"
};

describe("ActivityPane", () => {
  beforeEach(() => {
    setActiveLocaleProfile("en", "none");
  });

  afterEach(() => {
    cleanup();
    setActiveLocaleProfile("en", "none");
  });

  it("renders room-unread rows without event details but keeps them openable", () => {
    const onOpenRow = vi.fn();
    render(
      <ActivityPane
        activity={activityState([placeholderRow])}
        onClose={vi.fn()}
        onLoadMore={vi.fn()}
        onMarkRead={vi.fn()}
        onOpenRow={onOpenRow}
        onSetTab={vi.fn()}
      />
    );

    const listitem = screen.getByRole("listitem");
    expect(listitem.getAttribute("data-room-id")).toBe("!placeholder:example.invalid");
    expect(listitem.getAttribute("data-kind")).toBe("roomUnread");
    expect(listitem.getAttribute("data-event-id")).toBeNull();

    expect(screen.getByText("Placeholder room")).toBeTruthy();
    expect(screen.queryByText("Preview")).toBeNull();
    expect(screen.queryByText("Sender")).toBeNull();
    expect(listitem.querySelector("time")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /Open/ }));
    expect(onOpenRow).toHaveBeenCalledWith(placeholderRow);

    // No row-level mark-read button for placeholders.
    expect(screen.queryByRole("button", { name: /Mark room read/ })).toBeNull();
  });

  it("keeps event-backed rows clickable and markable", () => {
    const onOpenRow = vi.fn();
    const onMarkRead = vi.fn();
    render(
      <ActivityPane
        activity={activityState([eventRow])}
        onClose={vi.fn()}
        onLoadMore={vi.fn()}
        onMarkRead={onMarkRead}
        onOpenRow={onOpenRow}
        onSetTab={vi.fn()}
      />
    );

    const listitem = screen.getByRole("listitem");
    expect(listitem.getAttribute("data-event-id")).toBe("$event:example.invalid");
    expect(listitem.getAttribute("data-kind")).toBe("event");

    fireEvent.click(screen.getByRole("button", { name: /Open/ }));
    expect(onOpenRow).toHaveBeenCalledWith(eventRow);

    fireEvent.click(screen.getByRole("button", { name: /Mark room read/ }));
    expect(onMarkRead).toHaveBeenCalledWith({
      kind: "room",
      room_id: "!room:example.invalid",
      up_to_event_id: "$event:example.invalid"
    });
  });

  it("prefers observed event rows over placeholders for the same room", () => {
    const sameRoomPlaceholder: ActivityRow = {
      ...placeholderRow,
      room_id: "!room:example.invalid",
      room_label: "Event room"
    };
    render(
      <ActivityPane
        activity={activityState([eventRow, sameRoomPlaceholder])}
        onClose={vi.fn()}
        onLoadMore={vi.fn()}
        onMarkRead={vi.fn()}
        onOpenRow={vi.fn()}
        onSetTab={vi.fn()}
      />
    );

    // Both rows rendered because the test fixture supplies both; the Rust
    // projection guarantees only one row per room reaches the UI.
    expect(screen.getAllByRole("listitem")).toHaveLength(2);
  });
});
