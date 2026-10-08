// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { EntityAvatar, Sidebar, WorkspaceRail, composerSendingAccount } from "./Shell";
import { readyDesktopSnapshotFixture } from "../test/desktopApiFixture";
import type { AccountTabSummary, DesktopSnapshot, RoomListItem } from "../domain/types";
import { setActiveLocaleProfile, t } from "../i18n/messages";

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

function railProps(overrides: Partial<Parameters<typeof WorkspaceRail>[0]> = {}) {
  return {
    onCreateSpace: vi.fn(),
    onOpenContextMenu: vi.fn(),
    onOpenUserSettings: vi.fn(),
    onReorderSpaces: vi.fn(),
    onSelectSpace: vi.fn(),
    ...overrides
  };
}

function railSpace(
  space_id: string,
  display_name: string,
  is_active: boolean
): DesktopSnapshot["sidebar"]["space_rail"][number] {
  return {
    space_id,
    display_name,
    local_icon: null,
    avatar: null,
    unread_count: 0,
    highlight_count: 0,
    is_active,
    leave_candidates: []
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
      is_active: true,
      leave_candidates: []
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

  // #1217: an explicit local name is user text and renders in full.
  it("renders multiple graphemes of a local Space name without an icon or image", () => {
    const snapshot = readyDesktopSnapshotFixture();
    snapshot.sidebar.space_rail = [railSpace("!lab:example.invalid", "研究室", false)];
    snapshot.state.ui.navigation.space_local_presentations = {
      "!lab:example.invalid": { name: "研究室" }
    };

    render(<WorkspaceRail snapshot={snapshot} {...railProps()} />);

    const button = screen.getByRole("button", { name: "研究室" });
    const fallback = within(button).getByText("研究室");
    expect(fallback.classList.contains("compact-label")).toBe(true);
    expect(fallback.classList.contains("element-space")).toBe(false);
  });

  // #1217: a Matrix-only name keeps the #414 Element single-grapheme fallback.
  it("keeps the single-grapheme Element fallback for a Space without a local name (#414)", () => {
    const snapshot = readyDesktopSnapshotFixture();

    render(<WorkspaceRail snapshot={snapshot} {...railProps()} />);

    const button = screen.getByRole("button", { name: "Synthetic Workspace" });
    const fallback = within(button).getByText("S");
    expect(fallback.classList.contains("element-space")).toBe(true);
    expect(within(button).queryByText("Synthetic Workspace")).toBeNull();
  });

  // #1217: a long local name scales the whole name instead of one grapheme.
  it("renders a longer local Space name within the tile instead of only its initial", () => {
    const snapshot = readyDesktopSnapshotFixture();
    const name = "Research Laboratory Annex";
    snapshot.sidebar.space_rail = [railSpace("!lab:example.invalid", name, false)];
    snapshot.state.ui.navigation.space_local_presentations = {
      "!lab:example.invalid": { name }
    };

    render(<WorkspaceRail snapshot={snapshot} {...railProps()} />);

    const fallback = within(screen.getByRole("button", { name })).getByText(name);
    expect(fallback.classList.contains("compact-label")).toBe(true);
  });

  // #1217: the label length counts graphemes, so a decomposed name is not sized
  // as twice as long; the stored name is never normalised.
  it("sizes precomposed and decomposed local names to the same tile label length", () => {
    const precomposed = render(
      <EntityAvatar
        avatar={null}
        className="workspace-button-avatar is-space"
        fallback={"é"}
        fallbackMode="compactLabel"
      />
    );
    const precomposedTile = precomposed.container.querySelector<HTMLElement>(".avatar-fallback");
    precomposed.unmount();

    const decomposed = render(
      <EntityAvatar
        avatar={null}
        className="workspace-button-avatar is-space"
        fallback={"e\u0301"}
        fallbackMode="compactLabel"
      />
    );
    const decomposedTile = decomposed.container.querySelector<HTMLElement>(".avatar-fallback");

    expect(precomposedTile?.style.getPropertyValue("--avatar-label-length")).toBe("1");
    expect(decomposedTile?.style.getPropertyValue("--avatar-label-length")).toBe("1");
    expect(decomposedTile?.textContent).toBe("e\u0301");
  });

  // #1218: selection is the button itself, so the menu keeps only its own actions.
  it("offers only Space info and Leave Space in the Space context menu (#1218)", () => {
    const snapshot = readyDesktopSnapshotFixture();
    const onOpenContextMenu = vi.fn();

    render(<WorkspaceRail snapshot={snapshot} {...railProps({ onOpenContextMenu })} />);
    fireEvent.contextMenu(screen.getByRole("button", { name: "Synthetic Workspace" }));

    expect(onOpenContextMenu).toHaveBeenCalledTimes(1);
    const items = onOpenContextMenu.mock.calls[0]?.[2] as Array<{ id: string }>;
    expect(items.map((item) => item.id)).toEqual(["openSpaceInfo", "leaveSpace"]);
  });

  // #1206: selection is exposed as `aria-current`; the browser tier in e2e/
  // asserts the rendered light/dark indicator.
  it("exposes the active Space, and Home, as the current rail item (#1206)", () => {
    const snapshot = readyDesktopSnapshotFixture();
    snapshot.sidebar.account_home.is_active = false;
    snapshot.sidebar.space_rail = [
      railSpace("!alpha:example.invalid", "Alpha Space", true),
      railSpace("!beta:example.invalid", "Beta Space", false)
    ];
    const { unmount } = render(<WorkspaceRail snapshot={snapshot} {...railProps()} />);

    const active = screen.getByRole("button", { name: "Alpha Space" });
    const inactive = screen.getByRole("button", { name: "Beta Space" });
    expect(active.classList.contains("is-active")).toBe(true);
    expect(active.getAttribute("aria-current")).toBe("page");
    expect(inactive.classList.contains("is-active")).toBe(false);
    expect(inactive.getAttribute("aria-current")).toBeNull();
    unmount();

    snapshot.sidebar.account_home.is_active = true;
    snapshot.sidebar.space_rail = snapshot.sidebar.space_rail.map((space) => ({
      ...space,
      is_active: false
    }));
    render(<WorkspaceRail snapshot={snapshot} {...railProps()} />);

    const home = screen.getByRole("button", { name: "Home" });
    expect(home.classList.contains("is-active")).toBe(true);
    expect(home.getAttribute("aria-current")).toBe("page");
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

  it("dispatches typed section collapse and sort settings", () => {
    const snapshot = readyDesktopSnapshotFixture();
    snapshot.state.ui.navigation.active_space_id = null;
    snapshot.sidebar.active_space_id = null;
    snapshot.sidebar.account_home.is_active = true;
    snapshot.sidebar.space_rail.forEach((space) => { space.is_active = false; });
    const onUpdateSettings = vi.fn();
    render(
      <Sidebar
        snapshot={snapshot}
        {...sidebarProps()}
        onUpdateSettings={onUpdateSettings}
      />
    );

    fireEvent.click(within(screen.getByRole("region", { name: "DMs" })).getByRole("button", {
      name: /options for dms/i
    }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: "Name" }));
    expect(onUpdateSettings).toHaveBeenCalledWith({
      sidebar_section: {
        scope: "__home__",
        section: "dms",
        sort: { kind: "normalLocale" }
      }
    });

    fireEvent.click(within(screen.getByRole("region", { name: "Rooms" })).getByRole("button", {
      name: "Rooms"
    }));
    expect(onUpdateSettings).toHaveBeenCalledWith({
      sidebar_section: {
        scope: "__home__",
        section: "rooms",
        collapsed: true
      }
    });
  });

  it("renders Low priority below DMs from the Rust section split", () => {
    const snapshot = readyDesktopSnapshotFixture();
    const low = room("!low:example.invalid", "Quiet Room");
    low.tags = { favourite: null, low_priority: { order: null } };
    low.unread_count = 9;
    low.has_unread_content = true;
    snapshot.sidebar.sections.low_priority = [low];

    render(<Sidebar snapshot={snapshot} {...sidebarProps()} />);

    const dms = screen.getByRole("region", { name: "DMs" });
    const lowPriority = screen.getByRole("region", { name: "Low priority" });
    expect(dms.compareDocumentPosition(lowPriority) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    // The row keeps its own raw unread count even though the section is quiet.
    expect(within(lowPriority).getByRole("button", { name: /Quiet Room/ })).toBeTruthy();
    // No create action and no aggregate unread badge on this heading.
    expect(within(lowPriority).queryByRole("button", { name: /create room/i })).toBeNull();
    expect(lowPriority.querySelector(".section-unread-count")).toBeNull();
  });

  it("renders the Rust-marked empty-room placeholder in the active locale", () => {
    // #1050: the SDK's calculated "Empty Room" is English-only caller data.
    setActiveLocaleProfile("ja", "none");
    try {
      const snapshot = readyDesktopSnapshotFixture();
      snapshot.sidebar.sections.rooms = [
        {
          ...room("!empty:example.invalid", "Empty Room"),
          display_name_placeholder: { kind: "empty" }
        },
        room("!named:example.invalid", "Named")
      ];
      snapshot.sidebar.space_rooms = [...snapshot.sidebar.sections.rooms];

      render(<Sidebar snapshot={snapshot} {...sidebarProps()} />);

      expect(screen.getByRole("button", { name: /空のルーム/ })).toBeTruthy();
      expect(screen.queryByText("Empty Room")).toBeNull();

      fireEvent.change(screen.getByRole("searchbox"), { target: { value: "空の" } });
      expect(screen.getByText("空のルーム")).toBeTruthy();
      expect(screen.queryByText("Named")).toBeNull();
    } finally {
      setActiveLocaleProfile("en", "none");
    }
  });

  it("hides Low priority when the Rust section is empty", () => {
    const snapshot = readyDesktopSnapshotFixture();
    snapshot.sidebar.sections.low_priority = [];

    render(<Sidebar snapshot={snapshot} {...sidebarProps()} />);

    expect(screen.queryByRole("region", { name: "Low priority" })).toBeNull();
  });

  it("dispatches a typed Low priority collapse patch", () => {
    const snapshot = readyDesktopSnapshotFixture();
    snapshot.state.ui.navigation.active_space_id = null;
    snapshot.sidebar.active_space_id = null;
    snapshot.sidebar.account_home.is_active = true;
    snapshot.sidebar.space_rail.forEach((space) => { space.is_active = false; });
    snapshot.sidebar.sections.low_priority = [room("!low:example.invalid", "Quiet Room")];
    snapshot.sidebar.low_priority_collapsed = false;
    const onUpdateSettings = vi.fn();

    render(
      <Sidebar snapshot={snapshot} {...sidebarProps()} onUpdateSettings={onUpdateSettings} />
    );

    fireEvent.click(
      within(screen.getByRole("region", { name: "Low priority" })).getByRole("button", {
        name: "Low priority"
      })
    );
    expect(onUpdateSettings).toHaveBeenCalledWith({
      sidebar_section: { scope: "__home__", section: "lowPriority", collapsed: true }
    });
  });

  it("renders Rust unread totals as heading badges independent of visible rows", () => {
    const snapshot = readyDesktopSnapshotFixture();
    snapshot.sidebar.sections.rooms = [
      room("!r1:example.invalid", "Alpha"),
      room("!r2:example.invalid", "Beta")
    ];
    snapshot.sidebar.sections.people = [room("!d1:example.invalid", "Person")];
    // Deliberately unequal to the visible row counts: the badge is the Rust
    // aggregate, not a sum over rendered rows.
    snapshot.sidebar.space_unread_count = 5;
    snapshot.sidebar.dm_unread_count = 3;

    render(<Sidebar snapshot={snapshot} {...sidebarProps()} />);

    const rooms = screen.getByRole("region", { name: "Rooms" });
    const dms = screen.getByRole("region", { name: "DMs" });
    expect(rooms.querySelector(".section-unread-count")?.textContent).toBe("5");
    expect(dms.querySelector(".section-unread-count")?.textContent).toBe("3");
    expect(within(rooms).getByLabelText("Rooms unread: 5")).toBeTruthy();
    expect(within(dms).getByLabelText("DMs unread: 3")).toBeTruthy();
    // The conversation-count meta is replaced, not recoloured.
    expect(rooms.querySelector(".section-count")).toBeNull();

    // Filtering hides rows but must not change the Rust-owned totals.
    fireEvent.change(screen.getByRole("searchbox", { name: /filter/i }), {
      target: { value: "zzz" }
    });
    expect(rooms.querySelector(".section-unread-count")?.textContent).toBe("5");
    expect(dms.querySelector(".section-unread-count")?.textContent).toBe("3");
  });

  it("hides a zero unread badge and clamps large totals to 99+", () => {
    const snapshot = readyDesktopSnapshotFixture();
    snapshot.sidebar.space_unread_count = 100;
    snapshot.sidebar.dm_unread_count = 0;

    render(<Sidebar snapshot={snapshot} {...sidebarProps()} />);

    const rooms = screen.getByRole("region", { name: "Rooms" });
    const dms = screen.getByRole("region", { name: "DMs" });
    expect(rooms.querySelector(".section-unread-count")?.textContent).toBe("99+");
    expect(within(rooms).getByLabelText("Rooms unread: 100")).toBeTruthy();
    expect(dms.querySelector(".section-unread-count")).toBeNull();
  });

  it("renders Rooms above DMs as independent sections", () => {
    const snapshot = readyDesktopSnapshotFixture();

    render(<Sidebar snapshot={snapshot} {...sidebarProps()} />);

    const rooms = screen.getByRole("region", { name: "Rooms" });
    const dms = screen.getByRole("region", { name: "DMs" });
    expect(rooms.compareDocumentPosition(dms) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(within(rooms).getByRole("button", { name: /create room/i })).toBeTruthy();
    expect(within(dms).getByRole("button", { name: /new dm/i })).toBeTruthy();
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
      is_dm: false,
      is_space: false
    }];
    snapshot.sidebar.account_home.invite_count = 1;
    snapshot.sidebar.account_home.attention_count = 1;

    render(<Sidebar snapshot={snapshot} {...sidebarProps()} />);

    expect(screen.getByRole("button", { name: "Activity" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Explore" })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Invites/ })).toBeTruthy();
  });

  // Issue #961: rooms the account is not in live in their own lane below the
  // joined ones, each saying which relationship it is in, and clicking one
  // joins it instead of trying to open a room the account cannot read.
  it("renders the not-joined lane last, with a membership label and a join action", () => {
    const snapshot = readyDesktopSnapshotFixture();
    const onJoinRoom = vi.fn();
    snapshot.sidebar.sections.rooms = [room("!joined:example.invalid", "Joined Room")];
    snapshot.sidebar.space_rooms = [...snapshot.sidebar.sections.rooms];
    snapshot.sidebar.sections.low_priority = [room("!low:example.invalid", "Low Room")];
    snapshot.sidebar.sections.not_joined = [
      { ...room("!open:example.invalid", "Open Room"), membership: "not_joined", can_join: true },
      {
        ...room("!invited:example.invalid", "Invited Room"),
        membership: "invited",
        can_join: true
      }
    ];

    render(<Sidebar snapshot={snapshot} {...sidebarProps()} onJoinRoom={onJoinRoom} />);

    const open = screen.getByText("Open Room").closest("button") as HTMLButtonElement;
    const invited = screen.getByText("Invited Room").closest("button") as HTMLButtonElement;
    expect(open.textContent).toContain("Not joined");
    expect(invited.textContent).toContain("Invited");

    const buttons = screen.getAllByRole("button");
    expect(buttons.indexOf(open)).toBeGreaterThan(
      buttons.findIndex((button) => button.textContent?.includes("Low Room"))
    );

    fireEvent.click(open);
    expect(onJoinRoom).toHaveBeenCalledWith("!open:example.invalid");
    expect(sidebarProps().onSelectRoom).not.toHaveBeenCalled();
  });

  // Issue #961: a click must never fire a request the server would reject, and
  // an invitation is answered where accept and decline live, not by a single
  // click that silently joins.
  it("joins only joinable rows and sends an invited row to the invites view", () => {
    const snapshot = readyDesktopSnapshotFixture();
    const onJoinRoom = vi.fn();
    const onOpenInvites = vi.fn();
    snapshot.sidebar.sections.not_joined = [
      { ...room("!open:example.invalid", "Open Room"), membership: "not_joined", can_join: true },
      {
        ...room("!closed:example.invalid", "Invite Only"),
        membership: "not_joined",
        can_join: false
      },
      {
        ...room("!invited:example.invalid", "Invited Room"),
        membership: "invited",
        can_join: true
      }
    ];

    render(
      <Sidebar
        snapshot={snapshot}
        {...sidebarProps()}
        onJoinRoom={onJoinRoom}
        onOpenInvites={onOpenInvites}
      />
    );

    fireEvent.click(screen.getByText("Invite Only").closest("button") as HTMLButtonElement);
    expect(onJoinRoom).not.toHaveBeenCalled();
    expect(onOpenInvites).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText("Invited Room").closest("button") as HTMLButtonElement);
    expect(onOpenInvites).toHaveBeenCalledTimes(1);
    expect(onJoinRoom).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText("Open Room").closest("button") as HTMLButtonElement);
    expect(onJoinRoom).toHaveBeenCalledWith("!open:example.invalid");
  });

  // #1007: the selected Space's Rooms actions offer Add existing room.
  it("offers Add existing room from the Space's Rooms options, not at Home", () => {
    const snapshot = readyDesktopSnapshotFixture();
    const onAddExistingRoom = vi.fn();
    snapshot.sidebar.space_rail = snapshot.sidebar.space_rail.map((space) => ({ ...space, is_active: false }));
    const { unmount } = render(
      <Sidebar snapshot={snapshot} {...sidebarProps()} onAddExistingRoom={onAddExistingRoom} />
    );
    fireEvent.click(screen.getByRole("button", {
      name: t("roomList.sectionOptions", { section: t("roomList.categoryRooms") })
    }));
    expect(screen.queryByRole("menuitem", { name: t("spaceAddRooms.action") })).toBeNull();
    unmount();

    snapshot.sidebar.space_rail = [{
      space_id: "!space:example.invalid",
      display_name: "Synthetic Workspace",
      avatar: null,
      unread_count: 0,
      highlight_count: 0,
      is_active: true,
      leave_candidates: []
    }];
    render(<Sidebar snapshot={snapshot} {...sidebarProps()} onAddExistingRoom={onAddExistingRoom} />);
    fireEvent.click(screen.getByRole("button", {
      name: t("roomList.sectionOptions", { section: t("roomList.categoryRooms") })
    }));
    fireEvent.click(screen.getByRole("menuitem", { name: t("spaceAddRooms.action") }));
    expect(onAddExistingRoom).toHaveBeenCalledWith("!space:example.invalid");
    expect(screen.queryByRole("menu")).toBeNull();
  });
});

describe("composerSendingAccount", () => {
  const tab = (
    id: string,
    accountKey: string | null,
    status: AccountTabSummary["status"] = "ready"
  ): AccountTabSummary => ({
    id,
    accountKey,
    homeserver: null,
    displayName: "Same Name",
    avatarSourceRef: null,
    status,
    unreadCount: 0
  });

  it("hides the sender when only one account is signed in", () => {
    expect(composerSendingAccount([tab("a", "@a:one.invalid")], "a")).toBeNull();
    expect(
      composerSendingAccount(
        [tab("a", "@a:one.invalid"), tab("b", "@b:two.invalid", "signedOut"), tab("c", null, "addAccount")],
        "a"
      )
    ).toBeNull();
  });

  it("names the sender by Matrix ID when several accounts are signed in", () => {
    expect(
      composerSendingAccount([tab("a", "@a:one.invalid"), tab("b", "@a:two.invalid")], "b")
    ).toMatchObject({ name: "Same Name", userId: "@a:two.invalid" });
  });
});
