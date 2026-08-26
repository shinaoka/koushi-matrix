// @vitest-environment jsdom

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { createBrowserFakeApi } from "../backend/browserFakeApi";
import { computeBrowserRoomListProjection } from "../backend/roomListProjection";
import type { CurrentSessionStatusState, RoomSummary } from "../domain/types";
import { elementAvatarColorIndex, elementAvatarInitial } from "../app/uiShared";
import {
  EntityAvatar,
  Sidebar,
  TopBar,
  WorkspaceRail,
  avatarColorClass,
  type RuntimeAlert
} from "./Shell";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  window.localStorage.clear();
});

describe("EntityAvatar", () => {
  it("matches Compound's first non-sigil grapheme without splitting Unicode clusters", () => {
    expect(elementAvatarInitial("Research")).toBe("R");
    expect(elementAvatarInitial("研究室")).toBe("研");
    expect(elementAvatarInitial("#general")).toBe("g");
    expect(elementAvatarInitial("+Physics")).toBe("P");
    expect(elementAvatarInitial("@alice")).toBe("a");
    expect(elementAvatarInitial("👩‍🔬 Lab")).toBe("👩‍🔬");
    expect(elementAvatarInitial("👍🏽 team")).toBe("👍🏽");
    expect(elementAvatarInitial("équipe")).toBe("é");
    expect(elementAvatarInitial("#")).toBe("");
    expect(elementAvatarInitial("")).toBe("");
  });

  it("matches Compound's six-bucket UTF-16 room-id color hash", () => {
    expect(elementAvatarColorIndex("!abc:example.org")).toBe(2);
    expect(elementAvatarColorIndex("!space:matrix.org")).toBe(1);
    expect(elementAvatarColorIndex("!koushi:matrix.org")).toBe(4);
  });

  it("renders a Space-only Compound fallback without changing default avatar colors", () => {
    const { rerender } = render(
      <EntityAvatar
        avatar={null}
        className="workspace-button-avatar is-space"
        colorSeed="!abc:example.org"
        fallback="R"
        fallbackMode="elementSpace"
      />
    );

    const spaceFallback = screen.getByText("R");
    expect(spaceFallback.classList.contains("element-space")).toBe(true);
    expect(spaceFallback.classList.contains("compact-label")).toBe(false);
    expect(spaceFallback.getAttribute("data-color")).toBe("2");

    rerender(
      <EntityAvatar
        avatar={null}
        className="room-avatar"
        colorSeed="!abc:example.org"
        fallback="R"
      />
    );
    expect(screen.getByText("R").className).toContain(avatarColorClass("!abc:example.org"));
  });

  it("maps stable avatar seeds to one of the eight palette classes", () => {
    expect(avatarColorClass("!alpha:example.invalid")).toMatch(/^avatar-c[1-8]$/);
    expect(avatarColorClass("!alpha:example.invalid")).toBe(
      avatarColorClass("!alpha:example.invalid")
    );
  });

  it("applies a stable palette class to fallback avatars", () => {
    render(
      <EntityAvatar
        avatar={null}
        className="room-avatar"
        colorSeed="!alpha:example.invalid"
        fallback="AL"
      />
    );

    const fallback = screen.getByText("AL");
    expect(fallback.className).toContain(avatarColorClass("!alpha:example.invalid"));
  });

  it("renders a ready avatar image", () => {
    render(
      <EntityAvatar
        avatar={{
          mxc_uri: "mxc://matrix.org/avatar",
          thumbnail: {
            kind: "ready",
            source_url: "asset://avatar.bin",
            width: null,
            height: null,
            mime_type: null
          }
        }}
        className="room-avatar"
        fallback="AL"
      />
    );

    expect(document.querySelector<HTMLImageElement>(".room-avatar img")?.getAttribute("src")).toBe(
      "asset://avatar.bin"
    );
  });

  it("falls back to initials when a ready avatar image fails to render", () => {
    render(
      <EntityAvatar
        avatar={{
          mxc_uri: "mxc://matrix.org/avatar",
          thumbnail: {
            kind: "ready",
            source_url: "asset://avatar.bin",
            width: null,
            height: null,
            mime_type: null
          }
        }}
        className="room-avatar"
        fallback="AL"
      />
    );

    const image = document.querySelector<HTMLImageElement>(".room-avatar img");
    expect(image?.getAttribute("src")).toBe("asset://avatar.bin");
    fireEvent.error(image!);

    expect(document.querySelector(".room-avatar img")).toBeNull();
    expect(screen.getByText("AL")).toBeTruthy();
  });

  it("retries the same avatar URL after a transient image load failure", () => {
    vi.useFakeTimers();
    render(
      <EntityAvatar
        avatar={{
          mxc_uri: "mxc://matrix.org/avatar",
          thumbnail: {
            kind: "ready",
            source_url: "asset://avatar.bin",
            width: null,
            height: null,
            mime_type: null
          }
        }}
        className="room-avatar"
        fallback="AL"
      />
    );

    const image = document.querySelector<HTMLImageElement>(".room-avatar img");
    fireEvent.error(image!);
    expect(document.querySelector(".room-avatar img")).toBeNull();

    act(() => {
      vi.advanceTimersByTime(10_000);
    });

    expect(document.querySelector<HTMLImageElement>(".room-avatar img")?.getAttribute("src")).toBe(
      "asset://avatar.bin"
    );
  });
});

describe("Sidebar", () => {
  it("shows bootstrap loading without presenting an authoritative zero room list", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.selectSpace(null);
    snapshot.state.ui.room_list.readiness = {
      kind: "loading",
      source: "live",
      generation: 7
    };
    snapshot.sidebar.space_rooms = [];
    snapshot.sidebar.global_dms = [];
    const view = render(
      <Sidebar
        activeRoomId={snapshot.state.ui.navigation.active_room_id}
        activeView="timeline"
        snapshot={snapshot}
        onCreateRoom={() => undefined}
        onNewDm={() => undefined}
        onOpenContextMenu={() => undefined}
        onOpenActivity={() => undefined}
        onOpenExplore={() => undefined}
        onOpenInvites={() => undefined}
        onOpenSpaceInfo={() => undefined}
        onSelectRoom={() => undefined}
      />
    );

    expect(screen.getByRole("status").textContent).toBe("Loading rooms…");
    expect(screen.queryByRole("group", { name: "Room list category" })).toBeNull();

    snapshot.state.ui.room_list.readiness = {
      kind: "ready",
      source: "live",
      generation: 7
    };
    view.rerender(
      <Sidebar
        activeRoomId={snapshot.state.ui.navigation.active_room_id}
        activeView="timeline"
        snapshot={snapshot}
        onCreateRoom={() => undefined}
        onNewDm={() => undefined}
        onOpenContextMenu={() => undefined}
        onOpenActivity={() => undefined}
        onOpenExplore={() => undefined}
        onOpenInvites={() => undefined}
        onOpenSpaceInfo={() => undefined}
        onSelectRoom={() => undefined}
      />
    );

    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.getByRole("group", { name: "Room list category" })).toBeTruthy();
  });

  it("keeps the Space title and complete action group in separate rows", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.selectSpace("!space-alpha:example.invalid");

    render(
      <Sidebar
        activeRoomId={snapshot.state.ui.navigation.active_room_id}
        activeView="timeline"
        snapshot={snapshot}
        onCreateRoom={() => undefined}
        onNewDm={() => undefined}
        onOpenContextMenu={() => undefined}
        onOpenActivity={() => undefined}
        onOpenExplore={() => undefined}
        onOpenInvites={() => undefined}
        onOpenSpaceInfo={() => undefined}
        onSelectRoom={() => undefined}
      />
    );

    const header = document.querySelector<HTMLElement>(".workspace-header");
    const titleRow = header?.querySelector<HTMLElement>(".workspace-header-title");
    const actionRow = header?.querySelector<HTMLElement>(".workspace-header-actions");
    expect(titleRow?.textContent).toContain("Synthetic Workspace");
    expect(actionRow?.querySelectorAll("button")).toHaveLength(5);
    expect(titleRow?.contains(actionRow!)).toBe(false);
    expect(actionRow?.classList.contains("no-wrap")).toBe(true);
  });

  it("renders Home as Activity, Explore, Invites, all Rooms, and Direct Messages", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.selectSpace(null);
    const unspacedRoom: RoomSummary = {
      room_id: "!room-unspaced:example.invalid",
      display_name: "unspaced-room",
      display_label: "unspaced-room",
      original_display_label: "unspaced-room",
      avatar: null,
      is_dm: false,
      dm_user_ids: [],
      tags: { favourite: null, low_priority: null },
      unread_count: 0,
      parent_space_ids: [],
      dm_space_ids: [],
      is_encrypted: false
    };
    snapshot.state.domain.rooms = [...snapshot.state.domain.rooms, unspacedRoom];
    snapshot.state.ui.room_list = computeBrowserRoomListProjection(
      snapshot.state.ui.room_list.active_filter,
      snapshot.state.ui.room_list.sort,
      snapshot.state.ui.navigation.active_space_id,
      snapshot.state.domain.spaces,
      snapshot.state.domain.rooms,
      snapshot.state.domain.invites
    );

    render(
      <Sidebar
        activeRoomId={snapshot.state.ui.navigation.active_room_id}
        activeView="activity"
        snapshot={snapshot}
        onCreateRoom={() => undefined}
        onNewDm={() => undefined}
        onOpenContextMenu={() => undefined}
        onOpenActivity={() => undefined}
        onOpenExplore={() => undefined}
        onOpenInvites={() => undefined}
        onOpenSpaceInfo={() => undefined}
        onSelectRoom={() => undefined}
      />
    );

    expect(screen.getByRole("button", { name: "Activity" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Explore" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Invites" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Threads" })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Rooms/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /DMs/ })).toBeTruthy();
    expect(screen.getByRole("region", { name: "Rooms" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "unspaced-room" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "synthetic-room" })).toBeTruthy();
    expect(screen.queryByRole("region", { name: "Direct Messages" })).toBeNull();
  });

  it("switches between DMs and Rooms and persists the selected category", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.selectSpace(null);

    const renderSidebar = () =>
      render(
        <Sidebar
          activeRoomId={snapshot.state.ui.navigation.active_room_id}
          activeView="activity"
          snapshot={snapshot}
          onCreateRoom={() => undefined}
          onNewDm={() => undefined}
          onOpenContextMenu={() => undefined}
          onOpenActivity={() => undefined}
          onOpenExplore={() => undefined}
          onOpenInvites={() => undefined}
          onOpenSpaceInfo={() => undefined}
          onSelectRoom={() => undefined}
        />
      );

    renderSidebar();
    expect(screen.getByRole("region", { name: "Rooms" })).toBeTruthy();
    expect(screen.queryByRole("region", { name: "Direct Messages" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /DMs/ }));
    expect(screen.getByRole("region", { name: "Direct Messages" })).toBeTruthy();
    expect(screen.queryByRole("region", { name: "Rooms" })).toBeNull();
    expect(window.localStorage.getItem("koushi.sidebarRoomCategory.v1")).toBe("dms");

    cleanup();
    renderSidebar();
    expect(screen.getByRole("region", { name: "Direct Messages" })).toBeTruthy();
    expect(screen.queryByRole("region", { name: "Rooms" })).toBeNull();
  });

  it("keeps category totals visually neutral compared with unread badges", () => {
    const css = readFileSync(join(process.cwd(), "src/styles.css"), "utf8");
    const selectedTotalRule = /\.room-list-chip\.is-selected \.room-list-chip-total\s*\{(?<body>[^}]+)\}/s.exec(
      css
    )?.groups?.body;

    expect(selectedTotalRule).toBeTruthy();
    expect(selectedTotalRule).toContain("background: var(--surface-raised)");
    expect(selectedTotalRule).not.toContain("background: var(--brand-contrast)");
    expect(selectedTotalRule).not.toContain("color: var(--unread)");
    expect(selectedTotalRule).not.toContain("color: var(--mention)");
  });

  it("renders plain unread content as a dot and notifications as a number", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.selectSpace("!space-alpha:example.invalid");
    const plain = snapshot.state.domain.rooms.find((room) => room.room_id === "!room-alpha:example.invalid");
    const notified = snapshot.state.domain.rooms.find(
      (room) => room.room_id === "!room-planning:example.invalid"
    );
    if (!plain || !notified) {
      throw new Error("expected fake room fixtures");
    }
    plain.unread_count = 3;
    plain.notification_count = 0;
    plain.highlight_count = 0;
    plain.marked_unread = false;
    notified.unread_count = 3;
    notified.notification_count = 2;
    notified.highlight_count = 0;
    notified.marked_unread = false;

    render(
      <Sidebar
        activeRoomId={snapshot.state.ui.navigation.active_room_id}
        activeView="timeline"
        snapshot={snapshot}
        onCreateRoom={() => undefined}
        onNewDm={() => undefined}
        onOpenContextMenu={() => undefined}
        onOpenActivity={() => undefined}
        onOpenExplore={() => undefined}
        onOpenInvites={() => undefined}
        onOpenSpaceInfo={() => undefined}
        onSelectRoom={() => undefined}
      />
    );

    const plainButton = screen.getByRole("button", { name: "synthetic-room" });
    expect(plainButton.querySelector(".room-unread-dot")).not.toBeNull();
    expect(plainButton.querySelector(".room-count")).toBeNull();
    const notifiedButton = screen.getByRole("button", { name: "planning-room" });
    expect(notifiedButton.querySelector(".room-count")?.textContent).toBe("2");
    expect(notifiedButton.querySelector(".room-count")?.className).toContain("is-attention");
  });

  it("does not render unresolved child room ids as not joined rooms", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.selectSpace("!space-alpha:example.invalid");
    const activeSpace = snapshot.state.domain.spaces.find(
      (space) => space.space_id === snapshot.state.ui.navigation.active_space_id
    );
    activeSpace?.child_room_ids.push("!not-joined:example.invalid");
    const onJoinRoom = vi.fn();

    render(
      <Sidebar
        activeRoomId={snapshot.state.ui.navigation.active_room_id}
        activeView="timeline"
        snapshot={snapshot}
        onCreateRoom={() => undefined}
        onJoinRoom={onJoinRoom}
        onNewDm={() => undefined}
        onOpenContextMenu={() => undefined}
        onOpenActivity={() => undefined}
        onOpenExplore={() => undefined}
        onOpenInvites={() => undefined}
        onOpenSpaceInfo={() => undefined}
        onSelectRoom={() => undefined}
      />
    );

    expect(screen.queryByRole("region", { name: "Not joined" })).toBeNull();
    expect(screen.queryByRole("button", { name: "!not-joined:example.invalid" })).toBeNull();
    expect(onJoinRoom).not.toHaveBeenCalled();
  });

  it("sorts the selected category by active order or display name and persists the sort", async () => {
    const api = createBrowserFakeApi();
    let snapshot = await api.selectSpace("!space-alpha:example.invalid");

    const renderSidebar = () =>
      render(
        <Sidebar
          activeRoomId={snapshot.state.ui.navigation.active_room_id}
          activeView="timeline"
          snapshot={snapshot}
          onCreateRoom={() => undefined}
          onNewDm={() => undefined}
          onOpenContextMenu={() => undefined}
          onOpenActivity={() => undefined}
          onOpenExplore={() => undefined}
          onOpenInvites={() => undefined}
          onOpenSpaceInfo={() => undefined}
          onSelectRoom={() => undefined}
        />
      );

    renderSidebar();
    const activityOrder = Array.from(
      document.querySelectorAll('[data-room-section="rooms"] [data-testid="room-item"]')
    ).map((button) => button.getAttribute("aria-label"));
    expect(activityOrder).toEqual(["synthetic-room", "planning-room"]);

    const sortGroup = screen.getByRole("group", { name: "Room list sort" });
    expect(sortGroup.querySelector(".room-list-sort-label")?.textContent).toBe("Sort");
    expect(
      screen.getByRole("group", { name: "Room list category" }).querySelector(".room-list-sort-label")
    ).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Name" }));
    const nameOrder = Array.from(
      document.querySelectorAll('[data-room-section="rooms"] [data-testid="room-item"]')
    ).map((button) => button.getAttribute("aria-label"));
    expect(nameOrder).toEqual(["planning-room", "synthetic-room"]);
    expect(window.localStorage.getItem("koushi.sidebarRoomSort.v1")).toBe("name");

    cleanup();
    renderSidebar();
    const persistedOrder = Array.from(
      document.querySelectorAll('[data-room-section="rooms"] [data-testid="room-item"]')
    ).map((button) => button.getAttribute("aria-label"));
    expect(persistedOrder).toEqual(["planning-room", "synthetic-room"]);
  });

  it("renders Direct Messages in the Rust sidebar order for Active sort", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.selectSpace(null);
    const dmRooms = snapshot.state.domain.rooms.filter((room) => room.is_dm).slice(0, 2);
    if (dmRooms.length < 2) {
      throw new Error("expected at least two fake direct messages");
    }
    const [statusNewer, messageNewer] = dmRooms;
    statusNewer.recency_stamp = 300;
    statusNewer.conversation_activity = null;
    statusNewer.latest_event = {
      event_id: "$status-newer:example.invalid",
      is_redacted: false,
      sender_id: "@sender:example.invalid",
      sender_label: "Sender",
      sender_avatar: null,
      preview: null,
      timestamp_ms: 300
    };
    messageNewer.recency_stamp = 200;
    messageNewer.conversation_activity = {
      timestamp_ms: 250,
      source: "message"
    };
    messageNewer.latest_event = {
      event_id: "$message-newer:example.invalid",
      is_redacted: false,
      sender_id: "@sender:example.invalid",
      sender_label: "Sender",
      sender_avatar: null,
      preview: "newer latest message",
      timestamp_ms: 250
    };
    const projectedDms = snapshot.state.domain.rooms
      .filter((room) => room.is_dm)
      .map((room) => ({
        room_id: room.room_id,
        display_name: room.display_label,
        avatar: room.avatar,
        tags: room.tags,
        unread_count: room.notification_count ?? room.unread_count,
        highlight_count: room.highlight_count ?? 0
      }));
    snapshot.sidebar.global_dms = [projectedDms[1], projectedDms[0], ...projectedDms.slice(2)];

    render(
      <Sidebar
        activeRoomId={snapshot.state.ui.navigation.active_room_id}
        activeView="activity"
        snapshot={snapshot}
        onCreateRoom={() => undefined}
        onNewDm={() => undefined}
        onOpenContextMenu={() => undefined}
        onOpenActivity={() => undefined}
        onOpenExplore={() => undefined}
        onOpenInvites={() => undefined}
        onOpenSpaceInfo={() => undefined}
        onSelectRoom={() => undefined}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: /DMs/ }));
    const dmOrder = Array.from(
      document.querySelectorAll('[data-room-section="people"] [data-testid="room-item"]')
    ).map((button) => button.getAttribute("aria-label"));

    expect(dmOrder.slice(0, 2)).toEqual([
      messageNewer.display_label,
      statusNewer.display_label
    ]);
  });

  it("keeps Rooms and Direct Messages separate inside a normal space", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.selectSpace("!space-alpha:example.invalid");

    render(
      <Sidebar
        activeRoomId={snapshot.state.ui.navigation.active_room_id}
        activeView="timeline"
        snapshot={snapshot}
        onCreateRoom={() => undefined}
        onNewDm={() => undefined}
        onOpenContextMenu={() => undefined}
        onOpenActivity={() => undefined}
        onOpenExplore={() => undefined}
        onOpenInvites={() => undefined}
        onOpenSpaceInfo={() => undefined}
        onSelectRoom={() => undefined}
      />
    );

    expect(screen.queryByRole("button", { name: "Home" })).toBeNull();
    // Threads remains available from the workspace header; the room list is
    // still scoped to the selected space.
    expect(screen.getByRole("button", { name: "Threads" })).toBeTruthy();
    expect(screen.getByRole("region", { name: "Rooms" })).toBeTruthy();
    expect(screen.queryByRole("region", { name: "Direct Messages" })).toBeNull();
  });

  it("filters direct messages by trimmed display name without changing category totals", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.selectSpace(null);
    const dmRows = snapshot.sidebar.global_dms.slice(0, 2);
    if (dmRows.length < 2) {
      throw new Error("expected at least two fake direct messages");
    }
    dmRows[0].display_name = "Alice Example";
    dmRows[1].display_name = "Bob Example";

    render(
      <Sidebar
        activeRoomId={snapshot.state.ui.navigation.active_room_id}
        activeView="activity"
        snapshot={snapshot}
        onCreateRoom={() => undefined}
        onNewDm={() => undefined}
        onOpenContextMenu={() => undefined}
        onOpenActivity={() => undefined}
        onOpenExplore={() => undefined}
        onOpenInvites={() => undefined}
        onOpenSpaceInfo={() => undefined}
        onSelectRoom={() => undefined}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: /DMs/ }));
    const total = snapshot.sidebar.global_dms.length;
    const filter = screen.getByRole("searchbox", { name: "Filter direct messages" });
    fireEvent.change(filter, { target: { value: "  ALICE  " } });

    expect(screen.getByRole("button", { name: "Alice Example" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Bob Example" })).toBeNull();
    expect(screen.getByRole("button", { name: new RegExp(`DMs.*${total}`) })).toBeTruthy();
  });

  it("shows a distinct no-match state and clears the query with Escape or the clear button", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.selectSpace(null);
    snapshot.sidebar.global_dms = snapshot.sidebar.global_dms.slice(0, 2).map((room, index) => ({
      ...room,
      display_name: index === 0 ? "Alice Example" : "Bob Example"
    }));

    render(
      <Sidebar
        activeRoomId={snapshot.state.ui.navigation.active_room_id}
        activeView="activity"
        snapshot={snapshot}
        onCreateRoom={() => undefined}
        onNewDm={() => undefined}
        onOpenContextMenu={() => undefined}
        onOpenActivity={() => undefined}
        onOpenExplore={() => undefined}
        onOpenInvites={() => undefined}
        onOpenSpaceInfo={() => undefined}
        onSelectRoom={() => undefined}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: /DMs/ }));
    const filter = screen.getByRole("searchbox", { name: "Filter direct messages" });
    fireEvent.change(filter, { target: { value: "missing" } });
    expect(screen.getByRole("status").textContent).toBe("No matching direct messages");

    fireEvent.keyDown(filter, { key: "Escape" });
    expect(screen.getByRole("button", { name: "Alice Example" })).toBeTruthy();

    fireEvent.change(filter, { target: { value: "bob" } });
    fireEvent.click(screen.getByRole("button", { name: "Clear room list filter" }));
    expect(screen.getByRole("button", { name: "Alice Example" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Bob Example" })).toBeTruthy();
  });

  it("clears the filter when switching categories or active spaces", async () => {
    const api = createBrowserFakeApi();
    const homeSnapshot = await api.selectSpace(null);
    const view = render(
      <Sidebar
        activeRoomId={homeSnapshot.state.ui.navigation.active_room_id}
        activeView="activity"
        snapshot={homeSnapshot}
        onCreateRoom={() => undefined}
        onNewDm={() => undefined}
        onOpenContextMenu={() => undefined}
        onOpenActivity={() => undefined}
        onOpenExplore={() => undefined}
        onOpenInvites={() => undefined}
        onOpenSpaceInfo={() => undefined}
        onSelectRoom={() => undefined}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: /DMs/ }));
    const dmFilter = screen.getByRole("searchbox", { name: "Filter direct messages" });
    fireEvent.change(dmFilter, { target: { value: "alice" } });
    fireEvent.click(screen.getByRole("button", { name: /Rooms/ }));
    const roomFilter = screen.getByRole("searchbox", { name: "Filter rooms" });
    expect((roomFilter as HTMLInputElement).value).toBe("");

    fireEvent.change(roomFilter, { target: { value: "room" } });
    const spaceSnapshot = await api.selectSpace("!space-alpha:example.invalid");
    view.rerender(
      <Sidebar
        activeRoomId={spaceSnapshot.state.ui.navigation.active_room_id}
        activeView="activity"
        snapshot={spaceSnapshot}
        onCreateRoom={() => undefined}
        onNewDm={() => undefined}
        onOpenContextMenu={() => undefined}
        onOpenActivity={() => undefined}
        onOpenExplore={() => undefined}
        onOpenInvites={() => undefined}
        onOpenSpaceInfo={() => undefined}
        onSelectRoom={() => undefined}
      />
    );
    expect(
      (screen.getByRole("searchbox", { name: "Filter rooms" }) as HTMLInputElement).value
    ).toBe("");
  });

  it("shows online presence only on Direct Messages rows", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.selectSpace(null);
    const dm = snapshot.sidebar.global_dms[0];
    const dmRoom = snapshot.state.domain.rooms.find((room) => room.room_id === dm?.room_id);
    const dmUserId = dmRoom?.dm_user_ids[0];
    if (!dm || !dmUserId) {
      throw new Error("expected fake account home to include a direct message");
    }

    snapshot.state.domain.live_signals.presence[dmUserId] = "online";

    render(
      <Sidebar
        activeRoomId={snapshot.state.ui.navigation.active_room_id}
        activeView="timeline"
        snapshot={snapshot}
        onCreateRoom={() => undefined}
        onNewDm={() => undefined}
        onOpenContextMenu={() => undefined}
        onOpenActivity={() => undefined}
        onOpenExplore={() => undefined}
        onOpenInvites={() => undefined}
        onOpenSpaceInfo={() => undefined}
        onSelectRoom={() => undefined}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: /DMs/ }));
    const dmRow = screen.getByRole("button", { name: dm.display_name });
    expect(dmRow.querySelector(".room-presence-dot")).toBeTruthy();
  });

  it("passes one-to-one DM user info through the room context menu", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.selectSpace(null);
    const dm = snapshot.sidebar.global_dms[0];
    const dmRoom = snapshot.state.domain.rooms.find((room) => room.room_id === dm?.room_id);
    const dmUserId = dmRoom?.dm_user_ids[0];
    if (!dm || !dmUserId) {
      throw new Error("expected fake account home to include a direct message");
    }
    const onOpenContextMenu = vi.fn();

    render(
      <Sidebar
        activeRoomId={snapshot.state.ui.navigation.active_room_id}
        activeView="timeline"
        snapshot={snapshot}
        onCreateRoom={() => undefined}
        onNewDm={() => undefined}
        onOpenContextMenu={onOpenContextMenu}
        onOpenActivity={() => undefined}
        onOpenExplore={() => undefined}
        onOpenInvites={() => undefined}
        onOpenSpaceInfo={() => undefined}
        onSelectRoom={() => undefined}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: /DMs/ }));
    fireEvent.contextMenu(screen.getByRole("button", { name: dm.display_name }));

    expect(onOpenContextMenu).toHaveBeenCalledTimes(1);
    expect(onOpenContextMenu.mock.calls[0][1]).toEqual({
      kind: "room",
      roomId: dm.room_id,
      dmUserId
    });
    const items = onOpenContextMenu.mock.calls[0][2] as Array<{ id: string }>;
    expect(items.map((item) => item.id)).toContain("openUserInfo");
  });
});

describe("WorkspaceRail", () => {
  it("keeps Home accessible without a visual tooltip while preserving Space tooltips", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();
    const firstSpace = snapshot.sidebar.space_rail[0];
    if (!firstSpace) {
      throw new Error("expected fake snapshot to include a space");
    }
    snapshot.sidebar.account_home.unread_count = 3;
    snapshot.sidebar.account_home.invite_count = 1;
    snapshot.sidebar.account_home.attention_count = 4;

    render(
      <WorkspaceRail
        snapshot={snapshot}
        onCreateSpace={() => undefined}
        onOpenContextMenu={() => undefined}
        onOpenUserSettings={() => undefined}
        onReorderSpaces={() => undefined}
        onSelectSpace={() => undefined}
      />
    );

    const home = screen.getByRole("button", {
      name: "Home, 3 unread messages, 1 invites"
    });
    fireEvent.mouseEnter(home);
    fireEvent.focus(home);
    expect(
      Array.from(document.querySelectorAll(".tooltip-bubble")).some(
        (tooltip) => tooltip.textContent === "Home"
      )
    ).toBe(false);
    expect(home.getAttribute("title")).toBeNull();
    expect(home.getAttribute("aria-describedby")).toBeNull();

    const space = screen.getByRole("button", { name: firstSpace.display_name });
    fireEvent.focus(space);
    expect(screen.getByRole("tooltip", { name: firstSpace.display_name })).toBeTruthy();
  });

  it("uses Home as the only top-level system entry and does not render Activity bell", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.selectSpace(null);

    render(
      <WorkspaceRail
        snapshot={snapshot}
        onCreateSpace={() => undefined}
        onOpenContextMenu={() => undefined}
        onOpenUserSettings={() => undefined}
        onReorderSpaces={() => undefined}
        onSelectSpace={() => undefined}
      />
    );

    expect(screen.getByRole("button", { name: /^Home/ })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Activity" })).toBeNull();
  });

  it("uses the effective Space name for one generated grapheme and preserves local icon precedence", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();
    const firstSpace = snapshot.sidebar.space_rail[0];
    if (!firstSpace) {
      throw new Error("expected fake snapshot to include a space");
    }
    firstSpace.display_name = "Original";

    const view = render(
      <WorkspaceRail
        snapshot={snapshot}
        spaceOverrides={{ [firstSpace.space_id]: { name: "👩‍🔬 Laboratory" } }}
        onCreateSpace={() => undefined}
        onOpenContextMenu={() => undefined}
        onOpenUserSettings={() => undefined}
        onReorderSpaces={() => undefined}
        onSelectSpace={() => undefined}
      />
    );

    const generated = within(screen.getByRole("button", { name: "👩‍🔬 Laboratory" })).getByText(
      "👩‍🔬"
    );
    expect(generated.classList.contains("element-space")).toBe(true);
    expect(generated.getAttribute("data-color")).toBe(
      String(elementAvatarColorIndex(firstSpace.space_id))
    );

    view.rerender(
      <WorkspaceRail
        snapshot={snapshot}
        spaceOverrides={{ [firstSpace.space_id]: { name: "Renamed", icon: "LOCAL" } }}
        onCreateSpace={() => undefined}
        onOpenContextMenu={() => undefined}
        onOpenUserSettings={() => undefined}
        onReorderSpaces={() => undefined}
        onSelectSpace={() => undefined}
      />
    );
    const local = within(screen.getByRole("button", { name: "Renamed" })).getByText("LOCAL");
    expect(local.classList.contains("compact-label")).toBe(true);
    expect(local.classList.contains("element-space")).toBe(false);
  });

  it("uses Compound Space geometry and light/dark/high-contrast decorative tokens", () => {
    const css = readFileSync(join(process.cwd(), "src/styles.css"), "utf8");
    expect(css).toMatch(/\.workspace-button-avatar\.is-space\s*\{[^}]*border-radius:\s*25%/s);
    expect(css).toMatch(/\.avatar-fallback\.element-space\s*\{[^}]*text-transform:\s*uppercase/s);
    for (let index = 1; index <= 6; index += 1) {
      expect(css).toContain(`--space-avatar-bg-${index}:`);
      expect(css).toContain(`--space-avatar-text-${index}:`);
      expect(css).toContain(`.avatar-fallback.element-space[data-color="${index}"]`);
    }
    expect(css).toMatch(/@media \(forced-colors: active\)[\s\S]*\.avatar-fallback\.element-space/);
  });

  it("does not render mention or online-style dots on space rail buttons", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();
    const firstSpace = snapshot.sidebar.space_rail[0];
    if (!firstSpace) {
      throw new Error("expected fake snapshot to include a space");
    }
    firstSpace.highlight_count = 2;

    render(
      <WorkspaceRail
        snapshot={snapshot}
        onCreateSpace={() => undefined}
        onOpenContextMenu={() => undefined}
        onOpenUserSettings={() => undefined}
        onReorderSpaces={() => undefined}
        onSelectSpace={() => undefined}
      />
    );

    const spaceButton = screen.getByRole("button", { name: firstSpace.display_name });
    expect(spaceButton.getAttribute("data-mention-count")).toBeNull();
  });
});

describe("Space Members navigation", () => {
  it("shows joined and child-only counts for an active Space and opens on click", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.selectSpace("!space-alpha:example.invalid");
    const onOpenSpaceMembers = vi.fn();

    render(
      <Sidebar
        activeRoomId={snapshot.state.ui.navigation.active_room_id}
        activeView="timeline"
        snapshot={snapshot}
        onCreateRoom={() => undefined}
        onNewDm={() => undefined}
        onOpenContextMenu={() => undefined}
        onOpenActivity={() => undefined}
        onOpenExplore={() => undefined}
        onOpenInvites={() => undefined}
        onOpenSpaceInfo={() => undefined}
        onOpenSpaceMembers={onOpenSpaceMembers}
        spaceMemberCounts={{ joined: 26, childOnly: 3 }}
        onSelectRoom={() => undefined}
      />
    );

    const members = screen.getByRole("button", {
      name: "Members, 26 joined, 3 only in child rooms"
    });
    const actionRow = document.querySelector(".workspace-header-actions");
    expect(actionRow?.firstElementChild).toBe(members);
    expect(members.textContent).toBe("26 · +3");
    expect(members.querySelector(".space-members-nav-warning")?.textContent).toBe(" · +3");
    expect(document.querySelector(".sidebar-scroll .space-members-nav")).toBeNull();
    expect(actionRow?.querySelectorAll("button")).toHaveLength(5);
    fireEvent.click(members);
    expect(onOpenSpaceMembers).toHaveBeenCalledTimes(1);
  });

  it("shows only the joined count when there are no child-room-only users", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.selectSpace("!space-alpha:example.invalid");

    render(
      <Sidebar
        activeRoomId={snapshot.state.ui.navigation.active_room_id}
        activeView="timeline"
        snapshot={snapshot}
        onCreateRoom={() => undefined}
        onNewDm={() => undefined}
        onOpenContextMenu={() => undefined}
        onOpenActivity={() => undefined}
        onOpenExplore={() => undefined}
        onOpenInvites={() => undefined}
        onOpenSpaceInfo={() => undefined}
        spaceMemberCounts={{ joined: 26, childOnly: 0 }}
        onSelectRoom={() => undefined}
      />
    );

    const members = screen.getByRole("button", { name: /Members/ });
    expect(members.textContent).toBe("26");
    expect(members.querySelector(".space-members-nav-warning")).toBeNull();
  });

  it("does not show the Space-only Members entry on account Home", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.selectSpace(null);

    render(
      <Sidebar
        activeRoomId={snapshot.state.ui.navigation.active_room_id}
        activeView="timeline"
        snapshot={snapshot}
        onCreateRoom={() => undefined}
        onNewDm={() => undefined}
        onOpenContextMenu={() => undefined}
        onOpenActivity={() => undefined}
        onOpenExplore={() => undefined}
        onOpenInvites={() => undefined}
        onOpenSpaceInfo={() => undefined}
        onOpenSpaceMembers={() => undefined}
        spaceMemberCounts={{ joined: 26, childOnly: 3 }}
        onSelectRoom={() => undefined}
      />
    );

    expect(screen.queryByRole("button", { name: /Members/ })).toBeNull();
  });

  it("does not show the entry without a real active Space", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.selectSpace("!space-alpha:example.invalid");
    snapshot.sidebar.account_home.is_active = false;
    snapshot.sidebar.space_rail.forEach((space) => {
      space.is_active = false;
    });
    snapshot.state.ui.navigation.active_space_id = null;

    render(
      <Sidebar
        activeRoomId={snapshot.state.ui.navigation.active_room_id}
        activeView="timeline"
        snapshot={snapshot}
        onCreateRoom={() => undefined}
        onNewDm={() => undefined}
        onOpenContextMenu={() => undefined}
        onOpenActivity={() => undefined}
        onOpenExplore={() => undefined}
        onOpenInvites={() => undefined}
        onOpenSpaceInfo={() => undefined}
        spaceMemberCounts={{ joined: 26, childOnly: 3 }}
        onSelectRoom={() => undefined}
      />
    );

    expect(screen.queryByRole("button", { name: /Members/ })).toBeNull();
  });
});

describe("TopBar search placeholder", () => {
  function placeholderFor(
    searchScope: "allRooms" | "currentSpace" | "currentRoom",
    activeRoomName: string | null = "Design"
  ): string {
    cleanup();
    render(
      <TopBar
        activeRoomName={activeRoomName}
        activeSpaceName="Matrix"
        isBusy={false}
        searchInputRef={{ current: null }}
        searchQuery=""
        searchScope={searchScope}
        sync="running"
        onOpenKeyboardSettings={() => undefined}
        onRestartSync={() => undefined}
        onSearchQueryChange={() => undefined}
        onSearchScopeChange={() => undefined}
      />
    );
    return screen.getByLabelText("Search").getAttribute("placeholder") ?? "";
  }

  it("names the search target from the selected scope, not the active space", () => {
    // Saying "Search in Matrix" while the scope is All told the user the search
    // was narrower than it actually was.
    expect(placeholderFor("allRooms")).toBe("Search everywhere");
    expect(placeholderFor("currentSpace")).toBe("Search in Matrix");
    expect(placeholderFor("currentRoom")).toBe("Search in Design");
  });

  it("offers Room/DM as the conversation scope and keeps global search explicit", () => {
    cleanup();
    render(
      <TopBar
        activeRoomName="Design"
        activeSpaceName="Matrix"
        isBusy={false}
        searchInputRef={{ current: null }}
        searchQuery=""
        searchScope="currentRoom"
        sync="running"
        onOpenKeyboardSettings={() => undefined}
        onRestartSync={() => undefined}
        onSearchQueryChange={() => undefined}
        onSearchScopeChange={() => undefined}
      />
    );

    const scope = screen.getByRole("combobox", { name: "Search scope" });
    expect((scope as HTMLSelectElement).value).toBe("currentRoom");
    expect(screen.getByRole("option", { name: "Room/DM" }).getAttribute("value")).toBe("currentRoom");
    expect(screen.getByRole("option", { name: "All" }).getAttribute("value")).toBe("allRooms");
    expect(screen.queryByRole("option", { name: "DM" })).toBeNull();
  });

  it("falls back to the generic label when the scope has no target to name", () => {
    expect(placeholderFor("currentRoom", null)).toBe("Search");
  });
});

describe("TopBar navigation controls", () => {
  it("does not render unused history navigation controls", () => {
    render(
      <TopBar
        activeSpaceName="Matrix"
        isBusy={false}
        searchInputRef={{ current: null }}
        searchQuery=""
        searchScope="allRooms"
        sync="running"
        onOpenKeyboardSettings={() => undefined}
        onRestartSync={() => undefined}
        onSearchQueryChange={() => undefined}
        onSearchScopeChange={() => undefined}
      />
    );

    expect(screen.queryByRole("button", { name: "Back" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Forward" })).toBeNull();
    expect(screen.queryByRole("button", { name: "History" })).toBeNull();
    expect(screen.getByRole("textbox", { name: "Search" })).toBeTruthy();
  });
});

describe("TopBar window dragging", () => {
  it("starts window dragging from the titlebar background", () => {
    const onStartWindowDrag = vi.fn();

    render(
      <TopBar
        activeSpaceName="Matrix"
        isBusy={false}
        searchInputRef={{ current: null }}
        searchQuery=""
        searchScope="allRooms"
        sync="running"
        onOpenKeyboardSettings={() => undefined}
        onRestartSync={() => undefined}
        onSearchQueryChange={() => undefined}
        onSearchScopeChange={() => undefined}
        onStartWindowDrag={onStartWindowDrag}
      />
    );

    const titlebar = document.querySelector<HTMLElement>(".titlebar");
    expect(titlebar).not.toBeNull();
    fireEvent.mouseDown(titlebar!, { button: 0, buttons: 1 });
    fireEvent.mouseDown(titlebar!, { button: 1, buttons: 2 });

    expect(onStartWindowDrag).toHaveBeenCalledTimes(1);
  });

  it("does not start window dragging from titlebar controls", () => {
    const onStartWindowDrag = vi.fn();

    render(
      <TopBar
        activeSpaceName="Matrix"
        isBusy={false}
        searchInputRef={{ current: null }}
        searchQuery=""
        searchScope="allRooms"
        sync="running"
        onOpenKeyboardSettings={() => undefined}
        onRestartSync={() => undefined}
        onSearchQueryChange={() => undefined}
        onSearchScopeChange={() => undefined}
        onStartWindowDrag={onStartWindowDrag}
      />
    );

    const button = document.querySelector<HTMLButtonElement>(".top-actions .icon-button");
    const search = document.querySelector<HTMLElement>(".top-search");
    expect(button).not.toBeNull();
    expect(search).not.toBeNull();

    fireEvent.mouseDown(button!, { button: 0, buttons: 1 });
    fireEvent.mouseDown(search!, { button: 0, buttons: 1 });

    expect(onStartWindowDrag).not.toHaveBeenCalled();
  });
});

describe("TopBar Matrix connection status", () => {
  it("shows the Matrix server while reconnecting", () => {
    render(
      <TopBar
        activeSpaceName="Matrix"
        homeserver="https://matrix.org"
        isBusy={false}
        searchInputRef={{ current: null }}
        searchQuery=""
        searchScope="allRooms"
        sync={{ reconnecting: "network_offline" }}
        onOpenKeyboardSettings={() => undefined}
        onRestartSync={() => undefined}
        onSearchQueryChange={() => undefined}
        onSearchScopeChange={() => undefined}
      />
    );

    const status = screen.getByRole("status", { name: /Sync reconnecting/ });
    expect(status.textContent).toContain("matrix.org");
    expect(status.textContent).toContain("Reconnecting");
    expect(status.getAttribute("data-sync-state")).toBe("reconnecting");
  });

  it("updates the Matrix connection status after reconnect recovery", () => {
    const props = {
      activeSpaceName: "Matrix",
      homeserver: "https://matrix.org",
      isBusy: false,
      searchInputRef: { current: null },
      searchQuery: "",
      searchScope: "allRooms" as const,
      onOpenKeyboardSettings: () => undefined,
      onRestartSync: () => undefined,
      onSearchQueryChange: () => undefined,
      onSearchScopeChange: () => undefined
    };
    const { rerender } = render(<TopBar {...props} sync={{ reconnecting: "network_offline" }} />);

    expect(screen.getByRole("status", { name: /Sync reconnecting/ }).textContent).toContain(
      "Reconnecting"
    );

    rerender(<TopBar {...props} sync="running" />);

    const status = screen.getByRole("status", { name: /matrix\.org.*Running/ });
    expect(status.textContent).toContain("matrix.org");
    expect(status.textContent).toContain("Running");
    expect(status.textContent).not.toContain("Reconnecting");
    expect(status.getAttribute("data-sync-state")).toBe("running");
    expect(screen.queryByRole("button", { name: "Restart sync" })).toBeNull();
  });

  it("shows restart control when Matrix connection failed", () => {
    const onRestartSync = vi.fn();

    render(
      <TopBar
        activeSpaceName="Matrix"
        homeserver="https://matrix.org"
        isBusy={false}
        searchInputRef={{ current: null }}
        searchQuery=""
        searchScope="allRooms"
        sync={{ failed: "sync_failed_http" }}
        onOpenKeyboardSettings={() => undefined}
        onRestartSync={onRestartSync}
        onSearchQueryChange={() => undefined}
        onSearchScopeChange={() => undefined}
      />
    );

    expect(screen.getByRole("status", { name: /Sync failed/ }).textContent).toContain(
      "matrix.org"
    );
    fireEvent.click(screen.getByRole("button", { name: "Restart sync" }));

    expect(onRestartSync).toHaveBeenCalledTimes(1);
  });

  it("does not show restart control when Matrix auth is required", () => {
    render(
      <TopBar
        activeSpaceName="Matrix"
        homeserver="https://matrix.org"
        isBusy={false}
        searchInputRef={{ current: null }}
        searchQuery=""
        searchScope="allRooms"
        sync={{ failed: "sync_failed_auth" }}
        onOpenKeyboardSettings={() => undefined}
        onRestartSync={() => undefined}
        onSearchQueryChange={() => undefined}
        onSearchScopeChange={() => undefined}
      />
    );

    const status = screen.getByRole("status", { name: /Sign-in required/ });
    expect(status.textContent).toContain("Sign-in required");
    expect(screen.queryByRole("button", { name: "Restart sync" })).toBeNull();
  });
});

describe("TopBar current session status", () => {
  const readyStatus: CurrentSessionStatusState = {
    status: "ready",
    request_id: 369,
    details: {
      device_display_name: "Koushi on Linux",
      device_id: "DEVICE369",
      authentication_method: "oauth",
      sync_state: "running",
      is_cross_signed_by_owner: true,
      own_identity_verification: "verified",
      key_backup: "ready",
      verification: "verified",
      checked_at_ms: Date.UTC(2026, 6, 30, 12, 0, 0)
    }
  };

  function renderStatus(
    status: CurrentSessionStatusState = readyStatus,
    overrides: {
      accountManagementUrl?: string | null;
      onRefresh?: (trigger: "open" | "manual") => void;
      onManage?: (url: string | null) => void;
      onDiagnostics?: () => void;
      runtimeAlerts?: RuntimeAlert[];
      onRetryRuntimeAlert?: (kind: RuntimeAlert["kind"]) => void;
      onCopyDiagnostics?: () => Promise<void>;
    } = {}
  ) {
    return render(
      <TopBar
        activeSpaceName="Matrix"
        accountManagementUrl={overrides.accountManagementUrl ?? "https://account.example/manage"}
        currentSessionStatus={status}
        deviceId="DEVICE369"
        homeserver="https://matrix.example"
        isBusy={false}
        searchInputRef={{ current: null }}
        searchQuery=""
        searchScope="allRooms"
        sync="running"
        userId="@alice:matrix.example"
        onManageAccount={overrides.onManage ?? (() => undefined)}
        runtimeAlerts={overrides.runtimeAlerts}
        onRetryRuntimeAlert={overrides.onRetryRuntimeAlert}
        onCopyDiagnostics={overrides.onCopyDiagnostics}
        onOpenDiagnostics={overrides.onDiagnostics ?? (() => undefined)}
        onOpenKeyboardSettings={() => undefined}
        onRefreshCurrentSessionStatus={overrides.onRefresh ?? (() => undefined)}
        onRestartSync={() => undefined}
        onSearchQueryChange={() => undefined}
        onSearchScopeChange={() => undefined}
      />
    );
  }

  it("opens by pointer, refreshes immediately, and renders every Rust-owned field", () => {
    const onRefresh = vi.fn();
    renderStatus(readyStatus, { onRefresh });

    fireEvent.click(screen.getByRole("button", { name: "Open session status" }));

    expect(onRefresh).toHaveBeenCalledWith("open");
    const dialog = screen.getByRole("dialog", { name: "Current session" });
    expect(dialog.textContent).toContain("matrix.example");
    expect(dialog.textContent).toContain("@alice:matrix.example");
    expect(dialog.textContent).toContain("Koushi on Linux");
    expect(dialog.textContent).toContain("DEVICE369");
    expect(dialog.textContent).toContain("OAuth");
    expect(dialog.textContent).toContain("Running");
    expect(dialog.textContent).toContain("Verified");
    expect(dialog.textContent).toContain("Cross-signed");
    expect(dialog.textContent).toContain("Identity verified");
    expect(dialog.textContent).toContain("Ready");
    expect(dialog.textContent).toContain("Last checked");
  });

  it("supports keyboard opening, manual recheck, and failure replacing stale facts", () => {
    const onRefresh = vi.fn();
    const { rerender } = renderStatus(readyStatus, { onRefresh });
    const trigger = screen.getByRole("button", { name: "Open session status" });

    trigger.focus();
    fireEvent.keyDown(trigger, { key: "Enter" });
    fireEvent.click(trigger);
    fireEvent.click(screen.getByRole("button", { name: "Recheck" }));
    expect(onRefresh).toHaveBeenLastCalledWith("manual");

    rerender(
      <TopBar
        activeSpaceName="Matrix"
        accountManagementUrl={null}
        currentSessionStatus={{
          status: "failed",
          request_id: 370,
          kind: "timed_out",
          checked_at_ms: Date.UTC(2026, 6, 30, 12, 1, 0)
        }}
        deviceId="DEVICE369"
        homeserver="https://matrix.example"
        isBusy={false}
        searchInputRef={{ current: null }}
        searchQuery=""
        searchScope="allRooms"
        sync="running"
        userId="@alice:matrix.example"
        onManageAccount={() => undefined}
        onOpenKeyboardSettings={() => undefined}
        onRefreshCurrentSessionStatus={onRefresh}
        onRestartSync={() => undefined}
        onSearchQueryChange={() => undefined}
        onSearchScopeChange={() => undefined}
      />
    );

    const dialog = screen.getByRole("dialog", { name: "Current session" });
    expect(dialog.textContent).toContain("Session check timed out");
    expect(dialog.textContent).not.toContain("Koushi on Linux");
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(onRefresh).toHaveBeenLastCalledWith("manual");
  });

  it("copies only Device ID and renders only safe management and diagnostics actions", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText }
    });
    const onManage = vi.fn();
    const onDiagnostics = vi.fn();
    const { rerender } = renderStatus(readyStatus, { onManage, onDiagnostics });
    fireEvent.click(screen.getByRole("button", { name: "Open session status" }));

    fireEvent.click(screen.getByRole("button", { name: "Copy Device ID" }));
    expect(writeText).toHaveBeenCalledWith("DEVICE369");
    fireEvent.click(screen.getByRole("button", { name: "Manage account and devices" }));
    expect(onManage).toHaveBeenCalledWith("https://account.example/manage");

    fireEvent.click(
      within(screen.getByRole("dialog", { name: "Current session" })).getByRole("button", {
        name: "Open diagnostics"
      })
    );
    expect(onDiagnostics).toHaveBeenCalledTimes(1);

    rerender(
      <TopBar
        activeSpaceName="Matrix"
        accountManagementUrl="javascript:alert(1)"
        currentSessionStatus={readyStatus}
        deviceId="DEVICE369"
        homeserver="https://matrix.example"
        isBusy={false}
        searchInputRef={{ current: null }}
        searchQuery=""
        searchScope="allRooms"
        sync="running"
        userId="@alice:matrix.example"
        onManageAccount={onManage}
        onOpenKeyboardSettings={() => undefined}
        onRefreshCurrentSessionStatus={() => undefined}
        onRestartSync={() => undefined}
        onSearchQueryChange={() => undefined}
        onSearchScopeChange={() => undefined}
      />
    );
    expect(screen.queryByText(/did not advertise a safe external account destination/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Open local account settings" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Manage account and devices" })).toBeNull();
    expect(onManage).toHaveBeenCalledTimes(1);
  });

  it("shows the highest runtime-alert severity, lists every warning, and retries Secure Backup", () => {
    const onRetryRuntimeAlert = vi.fn();
    const runtimeAlerts: RuntimeAlert[] = [
      {
        kind: "secureBackup",
        severity: "warning",
        title: "Secure Backup unavailable",
        detail: "Encrypted sending is paused.",
        retryable: true
      },
      {
        kind: "sync",
        severity: "error",
        title: "Sync failed",
        detail: "Sign-in required.",
        retryable: false
      }
    ];
    renderStatus(readyStatus, { runtimeAlerts, onRetryRuntimeAlert });

    const trigger = screen.getByRole("button", {
      name: "Open session status, 2 runtime warnings"
    });
    expect(
      screen.getByRole("img", { name: "2 runtime warnings" }).getAttribute(
        "data-runtime-alert-severity"
      )
    ).toBe("error");

    fireEvent.click(trigger);

    const dialog = screen.getByRole("dialog", { name: "Current session" });
    expect(dialog.textContent).toContain("Runtime warnings");
    expect(dialog.textContent).toContain("Secure Backup unavailable");
    expect(dialog.textContent).toContain("Encrypted sending is paused.");
    expect(dialog.textContent).toContain("Sync failed");
    expect(dialog.textContent).toContain("Sign-in required.");
    fireEvent.click(within(dialog).getByRole("button", { name: "Retry secure backup" }));
    expect(onRetryRuntimeAlert).toHaveBeenCalledWith("secureBackup");
  });

  it("uses singular accessibility text for one runtime warning", () => {
    renderStatus(readyStatus, {
      runtimeAlerts: [
        {
          kind: "sync",
          severity: "warning",
          title: "Sync reconnecting",
          detail: "Network unavailable.",
          retryable: false
        }
      ]
    });

    expect(
      screen.getByRole("button", { name: "Open session status, 1 runtime warning" })
    ).toBeTruthy();
    expect(screen.getByRole("img", { name: "1 runtime warning" })).toBeTruthy();
  });

  it("announces successful diagnostic copying from the status popover", async () => {
    const onCopyDiagnostics = vi.fn().mockResolvedValue(undefined);
    renderStatus(readyStatus, { onCopyDiagnostics });
    fireEvent.click(screen.getByRole("button", { name: "Open session status" }));

    fireEvent.click(screen.getByRole("button", { name: "Copy diagnostics" }));

    expect(onCopyDiagnostics).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(screen.getByText("Diagnostics copied.")).toBeTruthy());
  });

  it("keeps the popover open and offers a retryable copy failure", async () => {
    const onCopyDiagnostics = vi
      .fn<() => Promise<void>>()
      .mockRejectedValueOnce(new Error("clipboard unavailable"))
      .mockResolvedValueOnce(undefined);
    renderStatus(readyStatus, { onCopyDiagnostics });
    fireEvent.click(screen.getByRole("button", { name: "Open session status" }));

    fireEvent.click(screen.getByRole("button", { name: "Copy diagnostics" }));

    await waitFor(() =>
      expect(screen.getByText("Could not copy diagnostics. Try again.")).toBeTruthy()
    );
    expect(screen.getByRole("dialog", { name: "Current session" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Copy diagnostics" }).hasAttribute("disabled")).toBe(
      false
    );

    fireEvent.click(screen.getByRole("button", { name: "Copy diagnostics" }));
    await waitFor(() => expect(screen.getByText("Diagnostics copied.")).toBeTruthy());
    expect(onCopyDiagnostics).toHaveBeenCalledTimes(2);
  });

  it("dismisses on Escape and outside pointer input and returns focus", () => {
    renderStatus();
    const trigger = screen.getByRole("button", { name: "Open session status" });
    fireEvent.click(trigger);
    expect(screen.getByRole("dialog", { name: "Current session" })).toBeTruthy();

    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "Current session" })).toBeNull();
    expect(document.activeElement).toBe(trigger);

    fireEvent.click(trigger);
    fireEvent.pointerDown(document.body);
    expect(screen.queryByRole("dialog", { name: "Current session" })).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });
});
