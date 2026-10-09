// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Sidebar } from "./Shell";
import { readyDesktopSnapshotFixture } from "../test/desktopApiFixture";
import { t } from "../i18n/messages";
import type { DesktopSnapshot } from "../domain/types";

function sidebarProps() {
  return {
    activeRoomId: null,
    activeView: "timeline" as const,
    onCreateRoom: vi.fn(),
    onNewDm: vi.fn(),
    onOpenContextMenu: vi.fn(),
    onOpenActivity: vi.fn(),
    onOpenExplore: vi.fn(),
    onOpenInvites: vi.fn(),
    onOpenThreads: vi.fn(),
    onOpenScheduledMessages: vi.fn(),
    onOpenSpaceInfo: vi.fn(),
    onSelectRoom: vi.fn()
  };
}

function homeSnapshot(): DesktopSnapshot {
  const snapshot = readyDesktopSnapshotFixture();
  snapshot.state.ui.navigation.active_space_id = null;
  snapshot.sidebar.active_space_id = null;
  snapshot.sidebar.account_home.is_active = true;
  snapshot.sidebar.space_rail.forEach((space) => {
    space.is_active = false;
  });
  return snapshot;
}

function contextActions(): HTMLElement[] {
  const group = document.querySelector<HTMLElement>('[data-toolbar-group="context"]');
  return group ? Array.from(group.querySelectorAll<HTMLElement>("[data-header-action]")) : [];
}

function endActions(): HTMLElement[] {
  const group = document.querySelector<HTMLElement>('[data-toolbar-group="end"]');
  return group ? Array.from(group.querySelectorAll<HTMLElement>("[data-header-action]")) : [];
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("unified Home/Space toolbar", () => {
  it("renders Home's account actions at the logical start and Threads/Scheduled/Info at the logical end", () => {
    render(<Sidebar snapshot={homeSnapshot()} {...sidebarProps()} />);

    expect(contextActions().map((button) => button.dataset.headerAction)).toEqual([
      "activity",
      "explore",
      "invites"
    ]);
    expect(endActions().map((button) => button.dataset.headerAction)).toEqual([
      "threads",
      "scheduled",
      "info"
    ]);
    expect(screen.queryByRole("button", { name: t("spaceMembers.navLabel") })).toBeNull();
    // The three account-global actions are no longer rendered as sidebar rows.
    expect(document.querySelector(".nav-item")).toBeNull();
  });

  it("renders Space's Members in the context group and keeps the shared end group", () => {
    const snapshot = readyDesktopSnapshotFixture();
    snapshot.state.ui.navigation.active_space_id = "!space-alpha:example.invalid";
    snapshot.sidebar.active_space_id = "!space-alpha:example.invalid";
    snapshot.sidebar.account_home.is_active = false;
    snapshot.sidebar.space_rail = snapshot.sidebar.space_rail.map((space) => ({
      ...space,
      is_active: space.space_id === "!space-alpha:example.invalid"
    }));

    render(<Sidebar snapshot={snapshot} {...sidebarProps()} />);

    expect(contextActions().map((button) => button.dataset.headerAction)).toEqual(["members"]);
    expect(endActions().map((button) => button.dataset.headerAction)).toEqual([
      "threads",
      "scheduled",
      "info"
    ]);
    expect(screen.queryByRole("button", { name: t("workspace.activity") })).toBeNull();
    expect(screen.queryByRole("button", { name: t("workspace.explore") })).toBeNull();
    expect(screen.queryByRole("button", { name: t("workspace.invites") })).toBeNull();
  });

  it("exposes localized accessible names, pressed state and tooltips", () => {
    render(<Sidebar snapshot={homeSnapshot()} {...sidebarProps()} activeView="explore" />);

    expect(screen.getByRole("button", { name: t("workspace.activity") })).toBeTruthy();
    expect(
      screen.getByRole("button", { name: t("workspace.explore") }).getAttribute("aria-pressed")
    ).toBe("true");
    expect(
      screen.getByRole("button", { name: t("workspace.activity") }).getAttribute("aria-pressed")
    ).toBe("false");
    expect(screen.getByRole("button", { name: t("workspace.scheduledMessages") })).toBeTruthy();
    expect(screen.getByRole("button", { name: t("threads.title") })).toBeTruthy();
    expect(screen.getByRole("button", { name: t("workspace.spaceInfoSettings") })).toBeTruthy();

    const scheduled = screen.getByRole("button", { name: t("workspace.scheduledMessages") });
    fireEvent.focus(scheduled);
    expect(screen.getByRole("tooltip").textContent).toBe(t("workspace.scheduledMessages"));
    fireEvent.blur(scheduled);
    expect(screen.queryByRole("tooltip")).toBeNull();
  });

  it("shows the Rust-owned invite count as a contained badge", () => {
    const snapshot = homeSnapshot();
    snapshot.state.domain.invites = [1, 2, 3].map((index) => ({
      room_id: `!invite-${index}:example.invalid`,
      display_name: `Invite ${index}`,
      avatar: null,
      topic: null,
      inviter_display_name: "Member",
      inviter_user_id: "@member:example.invalid",
      is_dm: false,
      is_space: false
    }));
    snapshot.sidebar.account_home.invite_count = 3;

    render(<Sidebar snapshot={snapshot} {...sidebarProps()} />);

    const invites = screen.getByRole("button", { name: t("workspace.invites") });
    expect(invites.getAttribute("data-count")).toBe("3");
  });

  it("keeps every toolbar action focusable and natively activatable", () => {
    const handlers = {
      activity: vi.fn(),
      explore: vi.fn(),
      invites: vi.fn(),
      threads: vi.fn(),
      scheduled: vi.fn(),
      info: vi.fn()
    };
    render(
      <Sidebar
        snapshot={homeSnapshot()}
        {...sidebarProps()}
        onOpenActivity={handlers.activity}
        onOpenExplore={handlers.explore}
        onOpenInvites={handlers.invites}
        onOpenThreads={handlers.threads}
        onOpenScheduledMessages={handlers.scheduled}
        onOpenSpaceInfo={handlers.info}
      />
    );

    const buttons = [...contextActions(), ...endActions()];
    expect(buttons).toHaveLength(6);
    for (const button of buttons) {
      button.focus();
      expect(document.activeElement).toBe(button);
      // Native buttons activate on Enter/Space; jsdom does not synthesize the
      // click, so the Playwright tier presses the key for real.
      expect(button.tagName).toBe("BUTTON");
      expect(button.getAttribute("type")).toBe("button");
    }

    for (const [action, handler] of Object.entries(handlers)) {
      const button = document.querySelector<HTMLElement>(`[data-header-action="${action}"]`)!;
      fireEvent.click(button);
      expect(handler).toHaveBeenCalledTimes(1);
    }
  });
});
