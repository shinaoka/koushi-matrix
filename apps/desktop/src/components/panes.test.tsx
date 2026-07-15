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
    recent: { rows: [], next_batch: null, resolution: { kind: "idle" } },
    unread: { rows, next_batch: null, resolution: { kind: "idle" } },
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

  it("replaces resolving placeholders with status instead of terminal room rows", () => {
    const resolving = activityState([placeholderRow]);
    if (resolving.kind === "open") {
      resolving.unread.resolution = { kind: "resolving", generation: 2, unresolved_room_count: 1 };
    }
    render(
      <ActivityPane
        activity={resolving}
        onClose={vi.fn()}
        onLoadMore={vi.fn()}
        onMarkRead={vi.fn()}
        onOpenRow={vi.fn()}
        onRetryResolution={vi.fn()}
        onSetTab={vi.fn()}
      />
    );
    expect(screen.getByRole("status").textContent).toContain("Resolving");
    expect(screen.queryByRole("listitem")).toBeNull();
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
        onRetryResolution={vi.fn()}
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
        onRetryResolution={vi.fn()}
        onSetTab={vi.fn()}
      />
    );

    expect(screen.getAllByRole("listitem")).toHaveLength(1);
  });

  it("offers typed retry for failed resolution", () => {
    const failed = activityState([placeholderRow]);
    if (failed.kind === "open") {
      failed.unread.resolution = { kind: "failed", generation: 3, unresolved_room_count: 1, failure_kind: "network" };
    }
    const onRetryResolution = vi.fn();
    render(
      <ActivityPane
        activity={failed}
        onClose={vi.fn()}
        onLoadMore={vi.fn()}
        onMarkRead={vi.fn()}
        onOpenRow={vi.fn()}
        onRetryResolution={onRetryResolution}
        onSetTab={vi.fn()}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: /Retry/ }));
    expect(onRetryResolution).toHaveBeenCalledOnce();
    expect(screen.queryByRole("listitem")).toBeNull();
  });
});
