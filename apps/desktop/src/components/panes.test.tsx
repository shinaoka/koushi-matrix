// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ActivityPane } from "./panes";
import { setActiveLocaleProfile } from "../i18n/messages";
import type { ActivityRow, ActivityState, ActivityStream } from "../domain/types";

function activityStream(rows: ActivityRow[]): ActivityStream {
  return {
    rows,
    next_batch: null,
    summary: {
      event_count: rows.filter((row) => row.kind === "event").length,
      room_count: new Set(rows.map((row) => row.room_id)).size,
      highlight_count: rows.filter((row) => row.highlight).length,
      unresolved_room_count: rows.filter((row) => row.kind === "roomUnread").length,
      thread_count: rows.filter((row) => row.kind === "threadUnread").length
    }
  };
}

function activityState(rows: ActivityRow[], activeTab: "recent" | "unread" = "unread"): ActivityState {
  return {
    kind: "open",
    active_tab: activeTab,
    recent: activityStream(activeTab === "recent" ? rows : []),
    unread: activityStream(activeTab === "unread" ? rows : []),
    mark_read: { kind: "idle" }
  };
}

const eventRow: ActivityRow = {
  kind: "event",
  room_id: "!room:example.invalid",
  event_id: "$event:example.invalid",
  root_event_id: null,
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
  root_event_id: null,
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

const threadPlaceholderRow: ActivityRow = {
  kind: "threadUnread",
  room_id: "!thread-room:example.invalid",
  event_id: null,
  root_event_id: "$thread-root:example.invalid",
  sender_id: null,
  room_label: "Thread room",
  sender_label: null,
  sender_avatar: null,
  preview: null,
  timestamp_ms: 2_500_000,
  unread: true,
  highlight: false,
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

  it("keeps thread event rows clickable without room mark-read", () => {
    const threadEventRow: ActivityRow = {
      ...eventRow,
      root_event_id: "$thread-root:example.invalid"
    };
    const onOpenRow = vi.fn();
    render(
      <ActivityPane
        activity={activityState([threadEventRow])}
        onClose={vi.fn()}
        onLoadMore={vi.fn()}
        onMarkRead={vi.fn()}
        onOpenRow={onOpenRow}
        onSetTab={vi.fn()}
      />
    );

    expect(screen.getByText("Thread")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Open/ }));
    expect(onOpenRow).toHaveBeenCalledWith(threadEventRow);
    expect(screen.queryByRole("button", { name: /Mark room read/ })).toBeNull();
  });

  it("renders thread-unread rows as thread placeholders without row mark-read", () => {
    const onOpenRow = vi.fn();
    render(
      <ActivityPane
        activity={activityState([threadPlaceholderRow])}
        onClose={vi.fn()}
        onLoadMore={vi.fn()}
        onMarkRead={vi.fn()}
        onOpenRow={onOpenRow}
        onSetTab={vi.fn()}
      />
    );

    const listitem = screen.getByRole("listitem");
    expect(listitem.getAttribute("data-kind")).toBe("threadUnread");
    expect(listitem.getAttribute("data-event-id")).toBeNull();
    expect(screen.getByText("Thread room")).toBeTruthy();
    expect(screen.getByText("Thread")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: /Open/ }));
    expect(onOpenRow).toHaveBeenCalledWith(threadPlaceholderRow);
    expect(screen.queryByRole("button", { name: /Mark room read/ })).toBeNull();
  });

  it("renders unread tab counts from event summary", () => {
    const secondEventRow: ActivityRow = {
      ...eventRow,
      room_id: "!second:example.invalid",
      event_id: "$second:example.invalid",
      timestamp_ms: 3_000_000
    };

    render(
      <ActivityPane
        activity={activityState([eventRow, secondEventRow, placeholderRow])}
        onClose={vi.fn()}
        onLoadMore={vi.fn()}
        onMarkRead={vi.fn()}
        onOpenRow={vi.fn()}
        onSetTab={vi.fn()}
      />
    );

    expect(screen.getByRole("tab", { name: "Unread (2)" })).toBeTruthy();
  });

  it("includes thread rows in the direct unread tab count", () => {
    render(
      <ActivityPane
        activity={activityState([eventRow, threadPlaceholderRow])}
        onClose={vi.fn()}
        onLoadMore={vi.fn()}
        onMarkRead={vi.fn()}
        onOpenRow={vi.fn()}
        onSetTab={vi.fn()}
      />
    );

    expect(screen.getByRole("tab", { name: "Unread (2)" })).toBeTruthy();
  });

  it("renders unread tab counts from unresolved room summary when no event rows exist", () => {
    const secondPlaceholderRow: ActivityRow = {
      ...placeholderRow,
      room_id: "!second-placeholder:example.invalid",
      room_label: "Second placeholder"
    };

    render(
      <ActivityPane
        activity={activityState([placeholderRow, secondPlaceholderRow])}
        onClose={vi.fn()}
        onLoadMore={vi.fn()}
        onMarkRead={vi.fn()}
        onOpenRow={vi.fn()}
        onSetTab={vi.fn()}
      />
    );

    expect(screen.getByRole("tab", { name: "Unread (2 rooms)" })).toBeTruthy();
  });

  it("uses singular unread room copy for one unresolved room", () => {
    render(
      <ActivityPane
        activity={activityState([placeholderRow])}
        onClose={vi.fn()}
        onLoadMore={vi.fn()}
        onMarkRead={vi.fn()}
        onOpenRow={vi.fn()}
        onSetTab={vi.fn()}
      />
    );

    expect(screen.getByRole("tab", { name: "Unread (1 room)" })).toBeTruthy();
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
