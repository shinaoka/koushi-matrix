// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { createBrowserFakeApi } from "../backend/browserFakeApi";
import { computeBrowserRoomListProjection } from "../backend/roomListProjection";
import type { RoomSummary } from "../domain/types";
import { EntityAvatar, Sidebar, TopBar, WorkspaceRail } from "./Shell";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  window.localStorage.clear();
});

describe("EntityAvatar", () => {
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
  it("renders Home as Activity, Explore, Invites, unspaced Rooms, and Direct Messages", async () => {
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
        onOpenHome={() => undefined}
        onOpenInvites={() => undefined}
        onOpenSpaceInfo={() => undefined}
        onOpenThreads={() => undefined}
        onSelectRoom={() => undefined}
      />
    );

    expect(screen.getByRole("button", { name: "Activity" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Explore" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Invites" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Threads" })).toBeNull();
    expect(screen.getByRole("button", { name: /Rooms/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /DMs/ })).toBeTruthy();
    expect(screen.getByRole("region", { name: "Rooms" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "unspaced-room" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "synthetic-room" })).toBeNull();
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
          onOpenHome={() => undefined}
          onOpenInvites={() => undefined}
          onOpenSpaceInfo={() => undefined}
          onOpenThreads={() => undefined}
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
          onOpenHome={() => undefined}
          onOpenInvites={() => undefined}
          onOpenSpaceInfo={() => undefined}
          onOpenThreads={() => undefined}
          onSelectRoom={() => undefined}
        />
      );

    renderSidebar();
    const activityOrder = Array.from(
      document.querySelectorAll('[data-room-section="rooms"] [data-testid="room-item"]')
    ).map((button) => button.getAttribute("aria-label"));
    expect(activityOrder).toEqual(["synthetic-room", "planning-room"]);

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
        onOpenHome={() => undefined}
        onOpenInvites={() => undefined}
        onOpenSpaceInfo={() => undefined}
        onOpenThreads={() => undefined}
        onSelectRoom={() => undefined}
      />
    );

    expect(screen.getByRole("button", { name: "Home" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Threads" })).toBeTruthy();
    expect(screen.getByRole("region", { name: "Rooms" })).toBeTruthy();
    expect(screen.queryByRole("region", { name: "Direct Messages" })).toBeNull();
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
        onOpenHome={() => undefined}
        onOpenInvites={() => undefined}
        onOpenSpaceInfo={() => undefined}
        onOpenThreads={() => undefined}
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
        onOpenHome={() => undefined}
        onOpenInvites={() => undefined}
        onOpenSpaceInfo={() => undefined}
        onOpenThreads={() => undefined}
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

    expect(screen.getByRole("button", { name: "Home" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Activity" })).toBeNull();
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

    const button = document.querySelector<HTMLButtonElement>(".history .icon-button");
    const search = document.querySelector<HTMLElement>(".top-search");
    expect(button).not.toBeNull();
    expect(search).not.toBeNull();

    fireEvent.mouseDown(button!, { button: 0, buttons: 1 });
    fireEvent.mouseDown(search!, { button: 0, buttons: 1 });

    expect(onStartWindowDrag).not.toHaveBeenCalled();
  });
});
