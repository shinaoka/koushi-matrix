// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { EntityAvatar, Sidebar, WorkspaceRail } from "./Shell";
import { readyDesktopSnapshotFixture } from "../test/desktopApiFixture";
import type { RoomListItem } from "../domain/types";

function room(room_id: string, display_name: string): RoomListItem {
  return {
    room_id,
    display_name,
    avatar: null,
    tags: { favourite: null, low_priority: null },
    unread_count: 0,
    highlight_count: 0,
    notification_count: 0,
    display_count: 0,
    has_unread_content: false,
    is_attention_highlighted: false,
    has_unread_mention: false,
    is_muted: false
  };
}

class MockIntersectionObserver {
  static instances: MockIntersectionObserver[] = [];
  private readonly callback: IntersectionObserverCallback;
  private readonly observed: Element[] = [];

  constructor(callback: IntersectionObserverCallback) {
    this.callback = callback;
    MockIntersectionObserver.instances.push(this);
  }

  observe(element: Element): void {
    this.observed.push(element);
  }

  disconnect(): void {}
  unobserve(): void {}
  takeRecords(): IntersectionObserverEntry[] { return []; }

  trigger(isIntersecting = true): void {
    this.callback(
      this.observed.map((target) => ({ isIntersecting, target }) as IntersectionObserverEntry),
      this as unknown as IntersectionObserver
    );
  }
}

function sidebarProps() {
  return {
    activeRoomId: null,
    activeView: "activity" as const,
    onCreateRoom: vi.fn(),
    onNewDm: vi.fn(),
    onOpenContextMenu: vi.fn(),
    onOpenActivity: vi.fn(),
    onOpenExplore: vi.fn(),
    onOpenInvites: vi.fn(),
    onOpenSpaceInfo: vi.fn(),
    onSelectRoom: vi.fn()
  };
}

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  MockIntersectionObserver.instances = [];
});

describe("Rust-projected workspace shell", () => {
  it("requests an icon only after its rendered avatar intersects", () => {
    vi.stubGlobal("IntersectionObserver", MockIntersectionObserver);
    const request = vi.fn();
    render(
      <EntityAvatar
        avatar={{ mxc_uri: "mxc://example.invalid/space", thumbnail: { kind: "notRequested" } }}
        className="test-avatar"
        fallback="SP"
        onRequestAvatarThumbnail={request}
      />
    );

    expect(request).not.toHaveBeenCalled();
    MockIntersectionObserver.instances[0]?.trigger();
    expect(request).toHaveBeenCalledOnce();
    expect(request).toHaveBeenCalledWith("mxc://example.invalid/space");
  });

  it("renders the Rust-projected local Space name and icon", () => {
    const snapshot = readyDesktopSnapshotFixture();
    snapshot.sidebar.space_rail = [{
      space_id: "!space:example.invalid",
      display_name: "Local laboratory",
      local_icon: "LAB",
      avatar: null,
      unread_count: 0,
      highlight_count: 0,
      is_active: true
    }];

    render(
      <WorkspaceRail
        snapshot={snapshot}
        onCreateSpace={vi.fn()}
        onOpenContextMenu={vi.fn()}
        onOpenUserSettings={vi.fn()}
        onReorderSpaces={vi.fn()}
        onSelectSpace={vi.fn()}
      />
    );

    expect(within(screen.getByRole("button", { name: "Local laboratory" })).getByText("LAB"))
      .toBeTruthy();
  });

  it("preserves Rust section order and performs only text filtering", () => {
    const snapshot = readyDesktopSnapshotFixture();
    snapshot.sidebar.sections.rooms = [
      room("!z:example.invalid", "Zulu"),
      room("!a:example.invalid", "Alpha")
    ];
    snapshot.sidebar.space_rooms = [...snapshot.sidebar.sections.rooms];

    render(<Sidebar snapshot={snapshot} {...sidebarProps()} />);

    const buttons = screen.getAllByRole("button");
    expect(buttons.findIndex((button) => button.textContent?.includes("Zulu")))
      .toBeLessThan(buttons.findIndex((button) => button.textContent?.includes("Alpha")));

    fireEvent.change(screen.getByRole("searchbox", { name: /filter/i }), {
      target: { value: "alp" }
    });
    expect(screen.queryByText("Zulu")).toBeNull();
    expect(screen.getByText("Alpha")).toBeTruthy();
  });

  it("dispatches typed sidebar category and sort settings", () => {
    const snapshot = readyDesktopSnapshotFixture();
    snapshot.sidebar.sections.people = [room("!dm:example.invalid", "Alice")];
    const onUpdateSettings = vi.fn();
    render(
      <Sidebar
        snapshot={snapshot}
        {...sidebarProps()}
        onUpdateSettings={onUpdateSettings}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: /^DMs,/i }));
    expect(onUpdateSettings).toHaveBeenCalledWith({
      sidebar: {
        category: "people",
        collapsed: { favourites: false, low_priority: false, not_joined: false }
      }
    });

    fireEvent.click(screen.getByRole("button", { name: /name/i }));
    expect(onUpdateSettings).toHaveBeenCalledWith({
      room_list_sort: { kind: "normalLocale" }
    });
  });

  it("renders Home-owned account navigation and invite count", () => {
    const snapshot = readyDesktopSnapshotFixture();
    snapshot.state.ui.navigation.active_space_id = null;
    snapshot.sidebar.active_space_id = null;
    snapshot.sidebar.account_home.is_active = true;
    snapshot.sidebar.space_rail.forEach((space) => { space.is_active = false; });
    snapshot.state.domain.invites = [{
      room_id: "!invite:example.invalid",
      display_name: "Invite",
      avatar: null,
      topic: null,
      inviter_display_name: "Alice",
      inviter_user_id: "@alice:example.invalid",
      is_dm: false
    }];
    snapshot.sidebar.account_home.invite_count = 1;
    snapshot.sidebar.account_home.attention_count = 1;

    render(<Sidebar snapshot={snapshot} {...sidebarProps()} />);

    expect(screen.getByRole("button", { name: "Activity" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Explore" })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Invites/ })).toBeTruthy();
  });
});
