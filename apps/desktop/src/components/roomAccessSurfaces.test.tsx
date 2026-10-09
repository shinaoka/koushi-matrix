// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { createElement } from "react";
import { afterEach, describe, expect, test, vi } from "vitest";

import { clearAppStoreSnapshot, setAppStoreSnapshot } from "../domain/appStore";
import { sidebarRoomAccess } from "../domain/accessCondition";
import { createTimelineStore } from "../domain/timelineStore";
import { readyDesktopSnapshotFixture } from "../test/desktopApiFixture";
import type { DesktopSnapshot, RoomListItem, RoomSettingsSnapshot } from "../domain/types";
import { setActiveLocaleProfile, t } from "../i18n/messages";
import { RoomInfoPanel } from "./RoomInfoPanel";
import { Sidebar } from "./Shell";
import { TimelinePane } from "./panes";
import { TimelineStoreContext } from "./timelineStoreContext";
import type { TimelineTransport } from "./TimelineView";

const ROOM_ID = "!conditional:example.invalid";
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

function renderRoomInfo(snapshot: DesktopSnapshot, settings: RoomSettingsSnapshot): void {
  render(
    <RoomInfoPanel
      room={{
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
        unread_count: 0
      }}
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

afterEach(() => {
  cleanup();
  clearAppStoreSnapshot();
  setActiveLocaleProfile("en", "none");
});

describe("the three access surfaces agree on the verified Space route (#1220)", () => {
  test("the room row announces the specific sentence with the Space name", () => {
    render(<Sidebar snapshot={snapshotWithRoute()} {...sidebarProps()} />);

    const row = screen.getByRole("button", { name: /^Conditional Room$/ });
    // #1249: the compact list shows the glyph; the label is its accessible name.
    expect(within(row).getByRole("img", { name: t("access.spaceMembersCanJoin") })).toBeTruthy();

    // The row's accessible description carries the Space substitution, not the
    // generic conditions sentence and never a raw id.
    const describedBy = row.getAttribute("aria-describedby");
    expect(describedBy).toBeTruthy();
    expect(document.getElementById(describedBy!)?.textContent).toBe(
      t("access.spaceMembersCanJoinDescription", { space: SPACE_NAME })
    );
  });

  test("the in-room header shows the specific badge and names the Space on keyboard focus", () => {
    const snapshot = snapshotWithRoute();
    snapshot.state.ui.timeline.room_id = ROOM_ID;
    setAppStoreSnapshot(snapshot);
    const store = createTimelineStore();
    const noop = () => undefined;

    render(
      createElement(
        TimelineStoreContext.Provider,
        { value: { store, setStore: vi.fn() } },
        createElement(TimelinePane, {
          activeRoomName: "Conditional Room",
          composerDocument: snapshot.state.ui.timeline.composer.document,
          composerMode: { kind: "plain" },
          resolveComposerKeyAction: async (): Promise<"noop"> => "noop",
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
        })
      )
    );

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
    expect(screen.queryByText(t("room.statusPrivate"))).toBeNull();
    expect(
      screen.getByRole("button", {
        name: t("room.statusShowSetting", { status: t("access.spaceMembersCanJoin") })
      })
    ).toBeTruthy();
  });

  test("the Room Info summary explains its route on hover and keyboard focus and keeps the request route", () => {
    const row: RoomListItem = {
      ...conditionalRoom(),
      access_join_rule: "knockRestricted",
      access_space_members_route: SPACE_NAME
    };
    vi.useFakeTimers();
    try {
      renderRoomInfo(snapshotWithRow(row), roomSettings("knockRestricted"));

      // Both routes of a single-Space knock-restricted rule reach Room Info, not
      // only the first label.
      const membership = screen.getByRole("button", {
        name: t("room.statusShowSetting", { status: t("access.spaceMembersCanJoin") })
      });
      const request = screen.getByRole("button", {
        name: t("room.statusShowSetting", { status: t("access.canRequest") })
      });

      // Hovering the membership route names the Space Rust resolved.
      fireEvent.mouseEnter(membership);
      act(() => {
        vi.advanceTimersByTime(300);
      });
      expect(screen.getByRole("tooltip").textContent).toBe(
        t("access.spaceMembersCanJoinDescription", { space: SPACE_NAME })
      );
      fireEvent.mouseLeave(membership);
      expect(screen.queryByRole("tooltip")).toBeNull();

      // Keyboard focus reaches the request route's own explanation.
      fireEvent.focus(request);
      expect(screen.getByRole("tooltip").textContent).toBe(t("access.requestRouteDescription"));

      // The summary badge still navigates to the property it summarizes.
      fireEvent.click(request);
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

  test("the Room Info summary keeps both generic knock-restricted routes", () => {
    const row: RoomListItem = {
      ...conditionalRoom(),
      access_join_rule: "knockRestricted",
      access_restricted_conditions: "confirmedEmpty"
    };
    delete row.access_space_members_route;
    renderRoomInfo(snapshotWithRow(row), roomSettings("knockRestricted"));

    expect(
      screen.getByRole("button", {
        name: t("room.statusShowSetting", { status: t("access.conditionsApply") })
      })
    ).toBeTruthy();
    const request = screen.getByRole("button", {
      name: t("room.statusShowSetting", { status: t("access.canRequest") })
    });
    fireEvent.focus(request);
    expect(screen.getByRole("tooltip").textContent).toBe(t("access.requestRouteDescription"));
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

    // The header uses the same checking explanation.
    setAppStoreSnapshot(snapshot);
    const store = createTimelineStore();
    const noop = () => undefined;
    const { unmount: unmountHeader } = render(
      createElement(
        TimelineStoreContext.Provider,
        { value: { store, setStore: vi.fn() } },
        createElement(TimelinePane, {
          activeRoomName: "Conditional Room",
          composerDocument: snapshot.state.ui.timeline.composer.document,
          composerMode: { kind: "plain" },
          resolveComposerKeyAction: async (): Promise<"noop"> => "noop",
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
        })
      )
    );
    const header = document.querySelector(".channel-header") as HTMLElement;
    expect(header).not.toBeNull();
    expect(within(header).getByText(t("access.checkingFull"))).toBeTruthy();
    unmountHeader();

    // Room Info agrees: checking, never the snapshot's defaulted invite. The
    // property editor keeps showing the underlying `invite` rule, so the
    // summary badge is asserted by its own role and name.
    renderRoomInfo(snapshot, roomSettings("invite"));
    const checking = screen.getByRole("button", {
      name: t("room.statusShowSetting", { status: t("access.checkingFull") })
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
