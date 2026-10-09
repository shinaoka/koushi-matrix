// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { createElement } from "react";
import { afterEach, describe, expect, test, vi } from "vitest";

import { clearAppStoreSnapshot, setAppStoreSnapshot } from "../domain/appStore";
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

function snapshotWithRoute(): DesktopSnapshot {
  const snapshot = readyDesktopSnapshotFixture();
  const row = conditionalRoom();
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
    expect(within(row).getByText(t("access.spaceMembersCanJoin"))).toBeTruthy();

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
    const settings: RoomSettingsSnapshot = {
      room_id: ROOM_ID,
      name: "Conditional Room",
      topic: null,
      avatar_url: null,
      join_rule: "restricted",
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
      members: []
    };
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
        access={{
          joinRule: "restricted",
          restricted: "membershipOnly",
          spaceMembersRoute: SPACE_NAME,
          allowedRoomNames: [SPACE_NAME]
        }}
        onUpdateRoomSetting={vi.fn()}
      />
    );

    expect(screen.getByText(t("access.spaceMembersCanJoin"))).toBeTruthy();
    expect(screen.queryByText(t("room.statusPrivate"))).toBeNull();
    expect(
      screen.getByRole("button", {
        name: t("room.statusShowSetting", { status: t("access.spaceMembersCanJoin") })
      })
    ).toBeTruthy();
  });
});
