// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { createElement } from "react";
import { afterEach, describe, expect, test, vi } from "vitest";

import { clearAppStoreSnapshot, setAppStoreSnapshot } from "../domain/appStore";
import { sidebarRoomAccess } from "../domain/accessCondition";
import { createTimelineStore } from "../domain/timelineStore";
import { readyDesktopSnapshotFixture } from "../test/desktopApiFixture";
import type {
  DesktopSnapshot,
  RoomListItem,
  RoomSettingsSnapshot,
  RoomSummary
} from "../domain/types";
import { setActiveLocaleProfile, t } from "../i18n/messages";
import { RoomInfoPanel } from "./RoomInfoPanel";
import { Sidebar } from "./Shell";
import { TimelinePane } from "./panes";
import { TimelineStoreContext } from "./timelineStoreContext";
import type { TimelineTransport } from "./TimelineView";

const ROOM_ID = "!conditional:example.invalid";
const DM_ID = "!dm:example.invalid";
const SPACE_NAME = "Synthetic Space";

/** A restricted room with exactly one verified Space route, as Rust projects it. */
function conditionalRoom(): RoomListItem {
  return {
    room_id: ROOM_ID,
    display_name: "Conditional Room",
    avatar: null,
    tags: { favourite: null, low_priority: null },
    unread_count: 0,
    highlight_count: 0,
    notification_count: 0,
    display_count: 0,
    has_unread_content: false,
    is_attention_highlighted: false,
    has_unread_mention: false,
    is_muted: false,
    access_join_rule: "restricted",
    access_restricted_conditions: "membershipOnly",
    access_allowed_room_names: [SPACE_NAME],
    access_space_members_route: SPACE_NAME
  };
}

function snapshotWithRow(row: RoomListItem): DesktopSnapshot {
  const snapshot = readyDesktopSnapshotFixture();
  snapshot.sidebar.active_space_id = null;
  snapshot.sidebar.account_home.is_active = true;
  snapshot.sidebar.space_rail = [];
  snapshot.sidebar.space_rooms = [row];
  snapshot.sidebar.global_dms = [];
  snapshot.sidebar.sections = {
    favourites: [],
    rooms: [row],
    people: [],
    low_priority: [],
    not_joined: []
  };
  return snapshot;
}

function snapshotWithRoute(): DesktopSnapshot {
  return snapshotWithRow(conditionalRoom());
}

function roomSettings(
  joinRule: RoomSettingsSnapshot["join_rule"],
  overrides: Partial<RoomSettingsSnapshot> = {}
): RoomSettingsSnapshot {
  return {
    room_id: ROOM_ID,
    name: "Conditional Room",
    topic: null,
    avatar_url: null,
    join_rule: joinRule,
    history_visibility: "shared",
    permissions: {
      can_edit_settings: true,
      can_change_join_rule: true,
      can_edit_roles: true,
      can_invite: true,
      can_kick: true,
      can_ban: true,
      can_unban: true
    },
    members: [],
    ...overrides
  };
}

function roomSummary(overrides: Partial<RoomSummary> = {}): RoomSummary {
  return {
    room_id: ROOM_ID,
    display_name: "Conditional Room",
    display_label: "Conditional Room",
    original_display_label: "Conditional Room",
    avatar: null,
    is_dm: false,
    dm_user_ids: [],
    tags: { favourite: null, low_priority: null },
    parent_space_ids: [],
    dm_space_ids: [],
    is_encrypted: false,
    unread_count: 0,
    ...overrides
  };
}

function renderRoomInfo(
  snapshot: DesktopSnapshot,
  settings: RoomSettingsSnapshot,
  room: RoomSummary = roomSummary()
): void {
  render(
    <RoomInfoPanel
      room={room}
      roomManagement={{ selected_room_id: ROOM_ID, settings, operation: { kind: "idle" } }}
      roomNotificationSettings={undefined}
      spaces={[]}
      // The production lookup path, not a hand-built projection.
      access={sidebarRoomAccess(snapshot.sidebar, ROOM_ID)}
      onUpdateRoomSetting={vi.fn()}
    />
  );
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

function noopTimelineTransport(): TimelineTransport {
  return {
    listenCoreEvents: () => () => undefined,
    paginateBackwards: async () => undefined,
    sendReaction: async () => undefined,
    retrySend: async () => undefined,
    cancelSend: async () => undefined,
    redactReaction: async () => undefined,
    sendReadReceipt: async () => undefined,
    setFullyRead: async () => undefined,
    setTyping: async () => undefined,
    editMessage: async () => undefined,
    redactMessage: async () => undefined,
    pinEvent: async () => undefined,
    unpinEvent: async () => undefined,
    downloadMedia: async () => undefined,
    downloadAvatarThumbnail: async () => undefined,
    loadMessageSource: async () => undefined,
    requestRoomKey: async () => undefined,
    forwardMessage: async () => undefined,
    loadLinkPreviews: async () => undefined,
    hideLinkPreview: async () => undefined,
    observeViewport: async () => undefined,
    openAtTimestamp: async () => undefined
  };
}

function timelinePaneProps(snapshot: DesktopSnapshot) {
  const noop = () => undefined;
  return {
    activeRoomName: "Conditional Room",
    composerDocument: snapshot.state.ui.timeline.composer.document,
    composerMode: { kind: "plain" as const },
    resolveComposerKeyAction: async (): Promise<"noop"> => "noop" as const,
    searchQuery: "",
    searchResults: [],
    showSearchResults: false,
    snapshot,
    timelineTransport: noopTimelineTransport(),
    onCancelReply: noop,
    onCancelScheduledSend: noop,
    onAttachFiles: noop,
    onClearUploadStaging: noop,
    onComposerMathModeChange: noop,
    onUpdateStagedUploadCaption: noop,
    onSelectStagedUploadOutput: noop,
    onSendStagedAttachments: noop,
    onLoadStagedUploadPreview: async () => [],
    onComposerDocumentChange: noop,
    onOpenContextMenu: noop,
    onOpenThread: noop,
    onRedactMessage: noop,
    onReply: noop,
    onRescheduleScheduledSend: noop,
    onResultSelect: noop,
    onScheduleSend: noop,
    onSendText: noop,
    onSetLocalUserAlias: noop,
    onUnpinPinnedEvent: noop,
    onOpenPeople: noop,
    onOpenThreads: noop,
    onToggleRoomInfo: noop
  };
}

/** Render the one-line conversation header for the given snapshot. */
function renderHeader(snapshot: DesktopSnapshot, onOpenRoomInfoSetting?: () => void): void {
  setAppStoreSnapshot(snapshot);
  const store = createTimelineStore();
  render(
    createElement(
      TimelineStoreContext.Provider,
      { value: { store, setStore: vi.fn() } },
      createElement(TimelinePane, {
        ...timelinePaneProps(snapshot),
        ...(onOpenRoomInfoSetting ? { onOpenRoomInfoSetting } : {})
      })
    )
  );
}

afterEach(() => {
  cleanup();
  clearAppStoreSnapshot();
  setActiveLocaleProfile("en", "none");
});

describe("the three access surfaces agree on the verified Space route (#1220, #1327)", () => {
  test("the room row announces the specific sentence with the Space name", () => {
    render(<Sidebar snapshot={snapshotWithRoute()} {...sidebarProps()} />);

    const row = screen.getByRole("button", { name: /^Conditional Room$/ });
    // #1327: the compact list shows one access glyph; the label is its
    // accessible name and the explanation is the row's description.
    expect(within(row).getByRole("img", { name: t("access.spaceMembersCanJoin") })).toBeTruthy();

    // The row's accessible description carries the Space substitution, not the
    // generic conditions sentence and never a raw id.
    const describedBy = row.getAttribute("aria-describedby");
    expect(describedBy).toBeTruthy();
    expect(document.getElementById(describedBy!)?.textContent).toBe(
      t("access.spaceMembersCanJoinDescription", { space: SPACE_NAME })
    );
  });

  test("the in-room header shows the access pill and names the Space on keyboard focus", () => {
    const snapshot = snapshotWithRoute();
    snapshot.state.ui.timeline.room_id = ROOM_ID;
    snapshot.state.domain.rooms = [roomSummary()];
    renderHeader(snapshot);

    const header = document.querySelector(".channel-header");
    expect(header).not.toBeNull();
    const badge = within(header as HTMLElement).getByText(t("access.spaceMembersCanJoin"));
    fireEvent.focus(badge);
    expect(screen.getByRole("tooltip").textContent).toBe(
      t("access.spaceMembersCanJoinDescription", { space: SPACE_NAME })
    );
  });

  test("the Room Info summary shows the specific sentence instead of Private", () => {
    renderRoomInfo(snapshotWithRoute(), roomSettings("restricted"));

    expect(screen.getByText(t("access.spaceMembersCanJoin"))).toBeTruthy();
    expect(
      screen.getByRole("button", {
        name: t("room.statusShowSetting", { status: t("access.spaceMembersCanJoin") })
      })
    ).toBeTruthy();
  });

  test("the Room Info summary explains its route on hover and keyboard focus", () => {
    vi.useFakeTimers();
    try {
      renderRoomInfo(snapshotWithRoute(), roomSettings("restricted"));

      const membership = screen.getByRole("button", {
        name: t("room.statusShowSetting", { status: t("access.spaceMembersCanJoin") })
      });

      // Hovering the access summary names the Space Rust resolved.
      fireEvent.mouseEnter(membership);
      act(() => {
        vi.advanceTimersByTime(300);
      });
      expect(screen.getByRole("tooltip").textContent).toBe(
        t("access.spaceMembersCanJoinDescription", { space: SPACE_NAME })
      );
      fireEvent.mouseLeave(membership);
      expect(screen.queryByRole("tooltip")).toBeNull();

      // Keyboard focus reveals the same explanation.
      fireEvent.focus(membership);
      expect(screen.getByRole("tooltip").textContent).toBe(
        t("access.spaceMembersCanJoinDescription", { space: SPACE_NAME })
      );

      // And the summary still navigates to the property it summarizes.
      fireEvent.click(membership);
      expect(document.activeElement).toBe(
        within(screen.getByRole("region", { name: t("room.accessAndHistory") })).getByRole(
          "heading",
          { name: t("room.joinRule") }
        )
      );
    } finally {
      vi.useRealTimers();
    }
  });

  test("the Room Info summary keeps both generic knock-restricted routes in one pill", () => {
    const row: RoomListItem = {
      ...conditionalRoom(),
      access_join_rule: "knockRestricted",
      access_restricted_conditions: "confirmedEmpty"
    };
    delete row.access_space_members_route;
    renderRoomInfo(snapshotWithRow(row), roomSettings("knockRestricted"));

    const summary = screen.getByRole("button", {
      name: t("room.statusShowSetting", { status: t("access.knockRestrictedLabel") })
    });
    fireEvent.focus(summary);
    expect(screen.getByRole("tooltip").textContent).toBe(
      t("access.restrictedNoUsableConditionsCanRequestDescription")
    );
  });

  test("an explicitly unavailable projection reads as checking on all three surfaces", () => {
    // Nothing was projected, so `access_join_rule` is absent. The settings
    // snapshot still carries the SDK's defaulted `invite`, which must not turn
    // the third surface into a confirmed invite-only label.
    const row: RoomListItem = { ...conditionalRoom() };
    delete row.access_join_rule;
    delete row.access_restricted_conditions;
    delete row.access_space_members_route;
    delete row.access_allowed_room_names;
    const snapshot = snapshotWithRow(row);
    snapshot.state.ui.timeline.room_id = ROOM_ID;
    snapshot.state.domain.rooms = [roomSummary()];

    // The room row announces the shared checking explanation.
    const { unmount: unmountSidebar } = render(<Sidebar snapshot={snapshot} {...sidebarProps()} />);
    const rowButton = screen.getByRole("button", { name: /^Conditional Room$/ });
    expect(within(rowButton).getByRole("img", { name: t("access.checking") })).toBeTruthy();
    const rowDescription = rowButton.getAttribute("aria-describedby");
    expect(rowDescription).toBeTruthy();
    expect(document.getElementById(rowDescription!)?.textContent).toBe(
      t("access.checkingDescription")
    );
    unmountSidebar();

    // The header uses the same checking explanation, never the snapshot's invite.
    const { unmount: unmountHeader } = render(
      createElement(
        TimelineStoreContext.Provider,
        { value: { store: createTimelineStore(), setStore: vi.fn() } },
        createElement(TimelinePane, timelinePaneProps(snapshot))
      )
    );
    const header = document.querySelector(".channel-header") as HTMLElement;
    expect(header).not.toBeNull();
    expect(within(header).getByText(t("access.checking"))).toBeTruthy();
    expect(within(header).queryByText(t("access.inviteOnly"))).toBeNull();
    unmountHeader();

    // Room Info agrees: checking, never the snapshot's defaulted invite. The
    // property editor keeps showing the underlying `invite` rule, so the
    // summary badge is asserted by its own role and name.
    renderRoomInfo(snapshot, roomSettings("invite"));
    const checking = screen.getByRole("button", {
      name: t("room.statusShowSetting", { status: t("access.checking") })
    });
    expect(
      screen.queryByRole("button", {
        name: t("room.statusShowSetting", { status: t("access.inviteOnly") })
      })
    ).toBeNull();
    fireEvent.focus(checking);
    expect(screen.getByRole("tooltip").textContent).toBe(t("access.checkingDescription"));
  });
});

describe("the room list shows one access icon and no DM status (#1327)", () => {
  test("a DM row shows neither an encryption nor an access indicator", () => {
    const dm: RoomListItem = {
      ...conditionalRoom(),
      room_id: DM_ID,
      display_name: "Synthetic Person",
      access_join_rule: "invite"
    };
    const snapshot = readyDesktopSnapshotFixture();
    snapshot.sidebar.active_space_id = null;
    snapshot.sidebar.account_home.is_active = true;
    snapshot.sidebar.space_rail = [];
    snapshot.sidebar.space_rooms = [];
    snapshot.sidebar.global_dms = [dm];
    snapshot.sidebar.sections = {
      favourites: [],
      rooms: [],
      people: [dm],
      low_priority: [],
      not_joined: []
    };
    render(<Sidebar snapshot={snapshot} {...sidebarProps()} />);

    const row = screen.getByRole("button", { name: /^Synthetic Person$/ });
    expect(row.querySelector("[data-room-access]")).toBeNull();
    expect(row.querySelector("[data-room-encryption]")).toBeNull();
    expect(row.querySelector(".room-access-icon")).toBeNull();
    expect(row.getAttribute("aria-describedby")).toBeNull();
  });

  test.each([
    ["public", "access.public", "globe"],
    ["invite", "access.inviteOnly", "userRoundPlus"],
    ["restricted", "access.conditionsApply", "usersRound"],
    ["knock", "access.canRequest", "hand"],
    ["knockRestricted", "access.knockRestrictedLabel", "hand"],
    ["private", "access.privateReserved", "circleHelp"],
    ["unknown", "access.unknown", "circleHelp"]
  ] as const)(
    "a %s room row shows exactly one %s icon and never a padlock",
    (rule, labelId, glyph) => {
      // No restricted allow-condition facts: each rule keeps its own generic
      // label, so the mapping is asserted on the rule alone.
      const row: RoomListItem = { ...conditionalRoom(), access_join_rule: rule };
      delete row.access_restricted_conditions;
      delete row.access_space_members_route;
      delete row.access_allowed_room_names;
      render(<Sidebar snapshot={snapshotWithRow(row)} {...sidebarProps()} />);

      const button = screen.getByRole("button", { name: /^Conditional Room$/ });
      const icons = button.querySelectorAll("[data-room-access]");
      expect(icons).toHaveLength(1);
      expect(icons[0].getAttribute("data-room-access")).toBe(glyph);
      expect(within(button).getByRole("img", { name: t(labelId) })).toBeTruthy();
      // The icon is decorative markup inside the row's single tab stop: no
      // nested button and no extra tab stop.
      expect(button.querySelectorAll("button")).toHaveLength(0);
      expect(icons[0].getAttribute("tabindex")).toBeNull();
    }
  );
});

describe("the conversation header status pills (#1327)", () => {
  test("shows independent encryption and participation pills after the name", () => {
    const snapshot = snapshotWithRow(conditionalRoom());
    snapshot.state.ui.timeline.room_id = ROOM_ID;
    snapshot.state.domain.rooms = [roomSummary({ is_encrypted: true })];
    renderHeader(snapshot);

    const header = document.querySelector(".channel-header") as HTMLElement;
    const name = within(header).getByText("Conditional Room");
    const encryption = header.querySelector("[data-room-encryption]") as HTMLElement;
    const access = header.querySelector("[data-room-access]") as HTMLElement;

    // Order: name, then encryption, then the join condition.
    expect(name.compareDocumentPosition(encryption) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(encryption.compareDocumentPosition(access) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(encryption.getAttribute("data-room-encryption")).toBe("encrypted");
    expect(encryption.textContent).toContain(t("room.statusEncrypted"));
    expect(access.getAttribute("data-room-access")).toBe("usersRound");
    expect(access.tagName).toBe("BUTTON");

    // Both pills are on the one existing header row.
    expect(header.querySelectorAll(".channel-title")).toHaveLength(1);
    expect(encryption.closest(".channel-actions")).toBeNull();
    expect(access.closest(".channel-actions")).toBeNull();
  });

  test("an unencrypted conversation says so instead of hiding the fact", () => {
    const snapshot = snapshotWithRow(conditionalRoom());
    snapshot.state.ui.timeline.room_id = ROOM_ID;
    snapshot.state.domain.rooms = [roomSummary({ is_encrypted: false })];
    renderHeader(snapshot);

    const encryption = document.querySelector("[data-room-encryption]") as HTMLElement;
    expect(encryption.getAttribute("data-room-encryption")).toBe("plaintext");
    expect(encryption.textContent).toContain(t("room.statusNotEncrypted"));
    fireEvent.focus(encryption);
    expect(screen.getByRole("tooltip").textContent).toBe(t("room.notEncryptedDescription"));
  });

  test("each pill explains itself on hover and focus and dismisses on Escape and blur", () => {
    vi.useFakeTimers();
    try {
      const snapshot = snapshotWithRow(conditionalRoom());
      snapshot.state.ui.timeline.room_id = ROOM_ID;
      snapshot.state.domain.rooms = [roomSummary({ is_encrypted: true })];
      renderHeader(snapshot);

      const header = document.querySelector(".channel-header") as HTMLElement;
      const encryption = header.querySelector("[data-room-encryption]") as HTMLElement;
      const access = header.querySelector("[data-room-access]") as HTMLElement;

      fireEvent.mouseEnter(encryption);
      act(() => {
        vi.advanceTimersByTime(300);
      });
      expect(screen.getByRole("tooltip").textContent).toBe(t("room.encryptedDescription"));
      fireEvent.mouseLeave(encryption);
      expect(screen.queryByRole("tooltip")).toBeNull();

      fireEvent.focus(access);
      expect(screen.getByRole("tooltip").textContent).toBe(
        t("access.spaceMembersCanJoinDescription", { space: SPACE_NAME })
      );
      fireEvent.keyDown(access, { key: "Escape" });
      expect(screen.queryByRole("tooltip")).toBeNull();

      fireEvent.focus(access);
      expect(screen.getByRole("tooltip")).toBeTruthy();
      fireEvent.blur(access);
      expect(screen.queryByRole("tooltip")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  test("the access pill stays a button and opens Room Info at the join condition", () => {
    const snapshot = snapshotWithRow(conditionalRoom());
    snapshot.state.ui.timeline.room_id = ROOM_ID;
    snapshot.state.domain.rooms = [roomSummary({ is_encrypted: true })];
    const onOpenRoomInfoSetting = vi.fn();
    renderHeader(snapshot, onOpenRoomInfoSetting);

    const access = document.querySelector("[data-room-access]") as HTMLElement;
    // One tab stop per pill, no nested controls, and an accessible name that
    // says what activation does.
    expect(access.tagName).toBe("BUTTON");
    expect(access.querySelector("button")).toBeNull();
    expect(access.getAttribute("aria-label")).toBe(
      t("room.statusShowSetting", { status: t("access.spaceMembersCanJoin") })
    );
    fireEvent.click(access);
    expect(onOpenRoomInfoSetting).toHaveBeenCalledTimes(1);
  });

  test("a room switch dismisses an open popup instead of rewriting it", () => {
    const first = snapshotWithRow(conditionalRoom());
    first.state.ui.timeline.room_id = ROOM_ID;
    first.state.domain.rooms = [roomSummary({ is_encrypted: true })];
    const store = createTimelineStore();
    const tree = (snapshot: DesktopSnapshot) =>
      createElement(
        TimelineStoreContext.Provider,
        { value: { store, setStore: vi.fn() } },
        createElement(TimelinePane, { ...timelinePaneProps(snapshot), snapshot })
      );

    const { rerender } = render(tree(first));
    fireEvent.focus(document.querySelector("[data-room-access]") as HTMLElement);
    expect(screen.getByRole("tooltip")).toBeTruthy();

    // The next room's own projection, so the pill keeps its place in the header.
    const secondRow: RoomListItem = {
      ...conditionalRoom(),
      room_id: "!second:example.invalid",
      display_name: "Second Room",
      access_join_rule: "public"
    };
    delete secondRow.access_restricted_conditions;
    delete secondRow.access_space_members_route;
    delete secondRow.access_allowed_room_names;
    const second = snapshotWithRow(secondRow);
    second.state.ui.timeline.room_id = secondRow.room_id;
    second.state.domain.rooms = [
      roomSummary({
        room_id: secondRow.room_id,
        display_name: "Second Room",
        display_label: "Second Room",
        original_display_label: "Second Room",
        is_encrypted: false
      })
    ];
    rerender(tree(second));

    // The previous room's explanation is gone rather than re-anchored.
    expect(document.querySelector("[data-room-access]")?.getAttribute("data-room-access")).toBe(
      "globe"
    );
    expect(screen.queryByRole("tooltip")).toBeNull();

    // Hovering again explains the new room.
    fireEvent.focus(document.querySelector("[data-room-access]") as HTMLElement);
    expect(screen.getByRole("tooltip").textContent).toBe(t("access.publicDescription"));
  });

  test("a DM header shows both facts and uses the shortened invitation copy", () => {
    const dmRow: RoomListItem = {
      ...conditionalRoom(),
      room_id: DM_ID,
      display_name: "Synthetic Person",
      access_join_rule: "invite"
    };
    const snapshot = snapshotWithRow(dmRow);
    snapshot.state.ui.timeline.room_id = DM_ID;
    snapshot.state.domain.rooms = [
      roomSummary({
        room_id: DM_ID,
        display_name: "Synthetic Person",
        display_label: "Synthetic Person",
        original_display_label: "Synthetic Person",
        is_dm: true,
        dm_user_ids: ["@synthetic:example.invalid"],
        is_encrypted: true
      })
    ];
    renderHeader(snapshot);

    const header = document.querySelector(".channel-header") as HTMLElement;
    expect(header.querySelector("[data-room-encryption]")).not.toBeNull();
    const access = header.querySelector("[data-room-access]") as HTMLElement;
    expect(access.getAttribute("data-room-access")).toBe("userRoundPlus");
    fireEvent.focus(access);
    expect(screen.getByRole("tooltip").textContent).toBe(t("access.inviteOnlyDmDescription"));
  });
});
