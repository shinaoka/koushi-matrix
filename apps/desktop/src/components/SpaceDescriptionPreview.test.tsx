// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, describe, expect, test, vi } from "vitest";
import type { DesktopSnapshot, InvitePreview, SpaceChildSummary, SpaceSummary } from "../domain/types";
import { readyDesktopSnapshotFixture } from "../test/desktopApiFixture";
import { InvitesPane } from "./panes";
import { Sidebar } from "./Shell";
import { SpaceDescriptionPreview } from "./SpaceDescriptionPreview";
import { SpaceInfoPanel } from "./SpaceInfoPanel";

/**
 * #1661: the confirmed Space description, read-only, outside Space info.
 *
 * jsdom has no layout, so the clamp measurement is stubbed: text longer than
 * LONG_TEXT characters overflows the collapsed box, and only the collapsed box
 * is shorter than its content. Playwright checks the real measurement.
 */
const LONG_TEXT = 80;
const LONG = "Welcome to the workspace.\nPlease join the general room first.\nRead the guide before posting.\nAsk in help.";
const SHORT = "Short welcome";

const descriptors = {
  scrollHeight: Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollHeight"),
  clientHeight: Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight")
};
beforeAll(() => {
  const contentHeight = (element: HTMLElement) => ((element.textContent ?? "").length > LONG_TEXT ? 120 : 40);
  Object.defineProperty(HTMLElement.prototype, "scrollHeight", {
    configurable: true,
    get(this: HTMLElement) { return contentHeight(this); }
  });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", {
    configurable: true,
    get(this: HTMLElement) {
      return this.classList.contains("is-collapsed") ? Math.min(contentHeight(this), 60) : contentHeight(this);
    }
  });
});
afterAll(() => {
  for (const [key, descriptor] of Object.entries(descriptors)) {
    if (descriptor) Object.defineProperty(HTMLElement.prototype, key, descriptor);
    else delete (HTMLElement.prototype as unknown as Record<string, unknown>)[key];
  }
});
afterEach(cleanup);

function descriptionText(container: HTMLElement = document.body): HTMLElement | null {
  return container.querySelector<HTMLElement>(".space-description-text");
}

describe("SpaceDescriptionPreview", () => {
  test.each([null, "", "  \n "])("renders nothing for an empty description (%j)", (text) => {
    const { container } = render(<SpaceDescriptionPreview text={text} />);
    expect(container.innerHTML).toBe("");
  });

  test("short text keeps its line breaks, resolves its own direction and has no toggle", () => {
    render(<SpaceDescriptionPreview text={"שלום 42!\nSecond line"} />);
    const text = descriptionText();
    expect(text?.textContent).toBe("שלום 42!\nSecond line");
    expect(text?.getAttribute("dir")).toBe("auto");
    expect(screen.queryByRole("button")).toBeNull();
  });

  test("long text is collapsed and a real button expands and collapses it again", () => {
    render(<SpaceDescriptionPreview text={LONG} />);
    const text = descriptionText();
    expect(text?.classList.contains("is-collapsed")).toBe(true);
    const toggle = screen.getByRole("button", { name: "Show more" });
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(toggle.getAttribute("aria-controls")).toBe(text?.id);

    fireEvent.click(toggle);
    expect(text?.classList.contains("is-collapsed")).toBe(false);
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    expect(toggle.textContent).toBe("Show less");

    fireEvent.click(screen.getByRole("button", { name: "Show less" }));
    expect(text?.classList.contains("is-collapsed")).toBe(true);
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
  });

  test("a changed description starts collapsed and drops a toggle it no longer needs", () => {
    const view = render(<SpaceDescriptionPreview text={LONG} />);
    fireEvent.click(screen.getByRole("button", { name: "Show more" }));
    view.rerender(<SpaceDescriptionPreview text={SHORT} />);
    expect(screen.queryByRole("button")).toBeNull();
    expect(descriptionText()?.classList.contains("is-collapsed")).toBe(true);

    view.rerender(<SpaceDescriptionPreview text={LONG} />);
    fireEvent.click(screen.getByRole("button", { name: "Show more" }));
    view.rerender(<SpaceDescriptionPreview text={`${LONG}\nOne more line.`} />);
    expect(screen.getByRole("button", { name: "Show more" }).getAttribute("aria-expanded")).toBe("false");
  });
});

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
    onSelectRoom: vi.fn(),
    onUpdateSettings: vi.fn()
  };
}

function spaceSnapshot(topic: string | null): DesktopSnapshot {
  const snapshot = readyDesktopSnapshotFixture();
  snapshot.state.domain.spaces = snapshot.state.domain.spaces.map((space) => ({ ...space, topic }));
  return snapshot;
}

describe("selected Space description in the sidebar", () => {
  test("shows the active Space's synced description at the top of its room list", () => {
    render(<Sidebar snapshot={spaceSnapshot(LONG)} {...sidebarProps()} />);
    const region = screen.getByRole("region", { name: "Description" });
    expect(descriptionText(region)?.textContent).toBe(LONG);
    expect(within(region).getByRole("button", { name: "Show more" })).toBeTruthy();
    // It leads the Space-scoped list, ahead of the filter and the sections.
    const scroll = document.querySelector(".sidebar-scroll");
    expect(scroll?.firstElementChild).toBe(region);
  });

  test("shows nothing when the Space has no description or no Space is selected", () => {
    const view = render(<Sidebar snapshot={spaceSnapshot(null)} {...sidebarProps()} />);
    expect(screen.queryByRole("region", { name: "Description" })).toBeNull();

    const home = spaceSnapshot(SHORT);
    home.state.ui.navigation.active_space_id = null;
    home.sidebar.active_space_id = null;
    home.sidebar.account_home.is_active = true;
    home.sidebar.space_rail = home.sidebar.space_rail.map((space) => ({ ...space, is_active: false }));
    view.rerender(<Sidebar snapshot={home} {...sidebarProps()} />);
    expect(screen.queryByRole("region", { name: "Description" })).toBeNull();
  });

  test("a new Space starts collapsed even after the previous one was expanded", () => {
    const view = render(<Sidebar snapshot={spaceSnapshot(LONG)} {...sidebarProps()} />);
    fireEvent.click(screen.getByRole("button", { name: "Show more" }));
    expect(screen.getByRole("button", { name: "Show less" })).toBeTruthy();

    const other = spaceSnapshot(LONG);
    const otherId = "!space-beta:example.invalid";
    other.state.domain.spaces = other.state.domain.spaces.map((space) => ({ ...space, space_id: otherId }));
    other.state.ui.navigation.active_space_id = otherId;
    other.sidebar.active_space_id = otherId;
    other.sidebar.space_rail = other.sidebar.space_rail.map((space) => ({ ...space, space_id: otherId }));
    view.rerender(<Sidebar snapshot={other} {...sidebarProps()} />);
    expect(screen.getByRole("button", { name: "Show more" }).getAttribute("aria-expanded")).toBe("false");
  });
});

function invite(overrides: Partial<InvitePreview>): InvitePreview {
  return {
    room_id: "!invite:example.invalid",
    display_name: "Synthetic Workspace",
    avatar: null,
    topic: null,
    inviter_display_name: "Member 1",
    inviter_user_id: "@member1:example.invalid",
    is_dm: false,
    is_space: false,
    ...overrides
  };
}

function invitesPane(preview: InvitePreview) {
  const snapshot = readyDesktopSnapshotFixture();
  snapshot.state.domain.invites = [preview];
  return (
    <InvitesPane
      isBusy={false}
      inviteActionError={null}
      snapshot={snapshot}
      onAcceptInvite={vi.fn()}
      onDeclineInvite={vi.fn()}
      onNewDm={vi.fn()}
    />
  );
}

describe("Space invite preview", () => {
  test("presents a Space's topic as its description with line breaks", () => {
    render(invitesPane(invite({ is_space: true, topic: "Welcome\nPlease join the general room" })));
    const preview = screen.getByRole("region", { name: "Invite preview" });
    const field = within(preview).getByText("Description").closest(".settings-summary-tile");
    const value = field?.querySelector("strong");
    expect(value?.textContent).toBe("Welcome\nPlease join the general room");
    expect(value?.getAttribute("dir")).toBe("auto");
    expect(value?.classList.contains("settings-summary-tile-multiline")).toBe(true);
    expect(within(preview).queryByText("Topic")).toBeNull();
  });

  test("a Space invite without a topic says it has no description", () => {
    render(invitesPane(invite({ is_space: true, topic: null })));
    const preview = screen.getByRole("region", { name: "Invite preview" });
    expect(within(preview).getByText("No description")).toBeTruthy();
  });

  test("a room invite keeps the topic label", () => {
    render(invitesPane(invite({ topic: "Room topic" })));
    const preview = screen.getByRole("region", { name: "Invite preview" });
    expect(within(preview).getByText("Topic")).toBeTruthy();
    expect(within(preview).getByText("Room topic").getAttribute("dir")).toBe("auto");
  });
});

describe("not-joined sub-Space rows in Space info", () => {
  const parent: SpaceSummary = {
    space_id: "!parent:example.invalid",
    raw_name: "Parent",
    display_name: "Parent",
    avatar: null,
    join_rule: null,
    topic: null,
    child_room_ids: [],
    parent_side_child_room_ids: []
  };
  function child(room_id: string, is_space: boolean, topic: string | null): SpaceChildSummary {
    return {
      room_id,
      display_name: is_space ? "Sub-Space" : "Plain room",
      avatar: null,
      topic,
      membership: "not_joined",
      can_join: true,
      is_space,
      joined_members: 2
    };
  }

  test("shows a sub-Space's hierarchy description on its row", () => {
    render(
      <SpaceInfoPanel
        fallbackName="Parent"
        rooms={[]}
        space={parent}
        spaceChildren={[
          child("!sub:example.invalid", true, "Sub-Space guide\nSecond line"),
          child("!plain:example.invalid", false, "Room topic stays out")
        ]}
      />
    );
    const subRow = screen.getByText("Sub-Space").closest(".settings-detail-row") as HTMLElement;
    expect(descriptionText(subRow)?.textContent).toBe("Sub-Space guide\nSecond line");
    const plainRow = screen.getByText("Plain room").closest(".settings-detail-row") as HTMLElement;
    expect(descriptionText(plainRow)).toBeNull();
    // A row's description is part of that row, not another landmark.
    expect(screen.queryByRole("region", { name: "Description" })).toBeNull();
  });
});
