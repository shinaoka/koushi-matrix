// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { createBrowserFakeApi } from "../backend/browserFakeApi";
import { ActivityPane, ExplorePane } from "./panes";
import { setActiveLocaleProfile } from "../i18n/messages";
import type {
  ActivityRow,
  ActivityState,
  DesktopSnapshot,
  DirectoryRoomSummary
} from "../domain/types";

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

  it("keeps retry and mark-all available when resolution is partial or failed", () => {
    const failed = activityState([eventRow, placeholderRow]);
    if (failed.kind === "open") {
      failed.unread.resolution = { kind: "failed", generation: 3, unresolved_room_count: 1, failure_kind: "network" };
    }
    const onRetryResolution = vi.fn();
    const onMarkRead = vi.fn();
    render(
      <ActivityPane
        activity={failed}
        onClose={vi.fn()}
        onLoadMore={vi.fn()}
        onMarkRead={onMarkRead}
        onOpenRow={vi.fn()}
        onRetryResolution={onRetryResolution}
        onSetTab={vi.fn()}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: /Retry/ }));
    expect(onRetryResolution).toHaveBeenCalledOnce();
    fireEvent.click(screen.getByRole("button", { name: /Mark all read/ }));
    expect(onMarkRead).toHaveBeenCalledWith({ kind: "all" });
    expect(screen.getAllByRole("listitem")).toHaveLength(1);
  });

  it("keeps mark-all available when every unread row is unresolved", () => {
    const failed = activityState([placeholderRow]);
    if (failed.kind === "open") {
      failed.unread.resolution = { kind: "failed", generation: 3, unresolved_room_count: 1, failure_kind: "network" };
    }
    const onMarkRead = vi.fn();
    render(
      <ActivityPane
        activity={failed}
        onClose={vi.fn()}
        onLoadMore={vi.fn()}
        onMarkRead={onMarkRead}
        onOpenRow={vi.fn()}
        onRetryResolution={vi.fn()}
        onSetTab={vi.fn()}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: /Mark all read/ }));
    expect(onMarkRead).toHaveBeenCalledWith({ kind: "all" });
  });
});

function directoryEntry(overrides: Partial<DirectoryRoomSummary>): DirectoryRoomSummary {
  return {
    room_id: "!entry:example.invalid",
    canonical_alias: null,
    room_type: null,
    name: "",
    topic: null,
    avatar_url: null,
    joined_members: 0,
    world_readable: false,
    guest_can_join: false,
    ...overrides
  };
}

async function snapshotWithDirectoryResults(
  rooms: DirectoryRoomSummary[]
): Promise<DesktopSnapshot> {
  const snapshot = await createBrowserFakeApi().getSnapshot();
  snapshot.state.domain.directory.query = {
    kind: "results",
    request_id: 1,
    query: { term: "anything", server_name: null, limit: 20, since: null },
    rooms,
    next_batch: null
  };
  return snapshot;
}

describe("ExplorePane public discovery", () => {
  afterEach(cleanup);

  it("marks a space result so it is not mistaken for an ordinary room", async () => {
    const snapshot = await snapshotWithDirectoryResults([
      directoryEntry({
        room_id: "!space:example.invalid",
        room_type: "m.space",
        name: "Community"
      })
    ]);

    render(
      <ExplorePane
        isBusy={false}
        queryDraft=""
        serverDraft=""
        snapshot={snapshot}
        onJoinRoom={vi.fn()}
        onQueryChange={vi.fn()}
        onServerChange={vi.fn()}
        onSearch={vi.fn()}
      />
    );

    expect(screen.getByText("Space")).toBeTruthy();
  });

  it("joins an aliasless space, which would otherwise be findable but unjoinable", async () => {
    const space = directoryEntry({
      room_id: "!aliasless:example.invalid",
      room_type: "m.space",
      name: "No alias space"
    });
    const snapshot = await snapshotWithDirectoryResults([space]);
    const onJoinRoom = vi.fn();

    render(
      <ExplorePane
        isBusy={false}
        queryDraft=""
        serverDraft=""
        snapshot={snapshot}
        onJoinRoom={onJoinRoom}
        onQueryChange={vi.fn()}
        onServerChange={vi.fn()}
        onSearch={vi.fn()}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: /No alias space/ }));

    expect(onJoinRoom).toHaveBeenCalledWith(space);
  });

  it("labels an unnamed entry by its type rather than leaving the row blank", async () => {
    const snapshot = await snapshotWithDirectoryResults([
      directoryEntry({ room_id: "!unnamed:example.invalid", room_type: "m.space" }),
      directoryEntry({ room_id: "!plain:example.invalid" })
    ]);

    render(
      <ExplorePane
        isBusy={false}
        queryDraft=""
        serverDraft=""
        snapshot={snapshot}
        onJoinRoom={vi.fn()}
        onQueryChange={vi.fn()}
        onServerChange={vi.fn()}
        onSearch={vi.fn()}
      />
    );

    expect(screen.getByText("Unnamed space")).toBeTruthy();
    expect(screen.getByText("Unnamed room")).toBeTruthy();
  });
});
