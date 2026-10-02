// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { Profiler } from "react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

import type {
  AccountTabsSnapshot,
  DesktopSnapshot,
  RoomManagementState,
  RoomMemberSummary,
  RoomSummary,
  SpaceMemberEntry,
  SpaceMembersState,
  SpaceSummary,
  StagedUploadItem,
  UserProfile
} from "../domain/types";
import { documentFromText } from "../domain/composerDocument";
import {
  exactRoomSettingsForRoom,
  spaceInviteAvailabilityReasonForSnapshot
} from "../domain/spaceInviteAvailability";
import { threadTimelineKey } from "../domain/coreEvents";
import { applyTimelineEvent, createTimelineStore } from "../domain/timelineStore";
import { t } from "../i18n/messages";
import { createDesktopApiFixture } from "../test/desktopApiFixture";
import { ContextualRightPanel, PanelHeader } from "./rightPanel";
import { TimelineStoreContext } from "./timelineStoreContext";
import { baseTransport, message } from "./timelineViewTestSupport";

// Issue #972: count renders of the memoized thread consumers without changing
// what they render.
const renderCounts = vi.hoisted(() => ({ composer: 0, timelineView: 0, rows: 0 }));

vi.mock("./timeline/TimelineItemRow", async () => {
  const actual = await vi.importActual<typeof import("./timeline/TimelineItemRow")>(
    "./timeline/TimelineItemRow"
  );
  const { createElement } = await import("react");
  // Not memoized: it renders whenever TimelineView renders it, like the real row.
  function TimelineItemRowProbe(props: Parameters<typeof actual.TimelineItemRow>[0]) {
    renderCounts.rows += 1;
    return createElement(actual.TimelineItemRow, props);
  }
  return { ...actual, TimelineItemRow: TimelineItemRowProbe };
});

vi.mock("./composer", async () => {
  const actual = await vi.importActual<typeof import("./composer")>("./composer");
  const { createElement, memo } = await import("react");
  // ThreadComposer is memoized, so this probe renders exactly when it does.
  const ThreadComposerProbe = memo(function ThreadComposerProbe(
    props: Parameters<typeof actual.ThreadComposer>[0]
  ) {
    renderCounts.composer += 1;
    return createElement(actual.ThreadComposer, props);
  });
  return { ...actual, ThreadComposer: ThreadComposerProbe };
});

vi.mock("./TimelineView", async () => {
  const actual = await vi.importActual<typeof import("./TimelineView")>("./TimelineView");
  const { createElement, memo } = await import("react");
  const TimelineViewProbe = memo(function TimelineViewProbe(
    props: Parameters<typeof actual.TimelineView>[0]
  ) {
    renderCounts.timelineView += 1;
    return createElement(actual.TimelineView, props);
  });
  return { ...actual, TimelineView: TimelineViewProbe };
});

class MockIntersectionObserver {
  static callback: IntersectionObserverCallback | null = null;

  constructor(callback: IntersectionObserverCallback) {
    MockIntersectionObserver.callback = callback;
  }

  observe(_element: Element): void {}

  unobserve(_element: Element): void {}

  disconnect(): void {}

  takeRecords(): IntersectionObserverEntry[] {
    return [];
  }

  static trigger(element: Element): void {
    MockIntersectionObserver.callback?.(
      [
        {
          isIntersecting: true,
          intersectionRatio: 1,
          target: element
        } as IntersectionObserverEntry
      ],
      {} as IntersectionObserver
    );
  }
}

const room: RoomSummary = {
  room_id: "!room-alpha:example.invalid",
  display_name: "Alpha Room",
  display_label: "Alpha Room",
  original_display_label: "Alpha Room",
  avatar: null,
  is_dm: false,
  dm_user_ids: [],
  tags: { favourite: null, low_priority: null },
  unread_count: 0,
  parent_space_ids: [],
  dm_space_ids: [],
  is_encrypted: false
};

const space: SpaceSummary = {
  space_id: "!space-work:example.invalid",
  raw_name: null,
  display_name: "Workspace",
  avatar: null,
  join_rule: null,
  child_room_ids: [room.room_id]
};

const roomMember: RoomMemberSummary = {
  user_id: "@room-member:example.invalid",
  display_name: "Room member",
  display_label: "Room member",
  original_display_label: "Room member",
  avatar_url: null,
  power_level: 0,
  role: "user",
  membership: "joined" as const,
  role_options: []
};

const roomManagement: RoomManagementState = {
  selected_room_id: room.room_id,
  settings: {
    room_id: room.room_id,
    name: room.display_name,
    topic: null,
    avatar_url: null,
    join_rule: "invite",
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
    members: [roomMember]
  },
  operation: { kind: "idle" }
};

function spaceMember(
  userId: string,
  displayLabel: string,
  membership: SpaceMemberEntry["membership"],
  overrides: Partial<SpaceMemberEntry> = {}
): SpaceMemberEntry {
  return {
    user_id: userId,
    display_name: displayLabel,
    display_label: displayLabel,
    original_display_label: displayLabel,
    avatar_url: null,
    power_level: 0,
    role: "user",
    membership,
    child_room_ids: [],
    invite_pending: false,
    role_options: [],
    ...overrides
  };
}

const spaceMembers: SpaceMembersState = {
  selected_space_id: space.space_id,
  generation: 1,
  space_joined: [
    spaceMember("@space-member:example.invalid", "Space member", "space_joined")
  ],
  space_invited: [],
  child_room_only: [
    spaceMember("@child-member:example.invalid", "Child member", "child_room_only", {
      child_room_ids: [room.room_id]
    })
  ],
  child_room_count: 1,
  complete_child_room_count: 1,
  incomplete_child_room_count: 0,
  power_levels_revision: null,
  can_edit_roles: false,
  operation: { kind: "idle" }
};

const snapshot = {
  state: {
    domain: {
      session: { user_id: "@current:example.invalid" },
      rooms: [room],
      spaces: [space],
      profile: { ignored_user_ids: [], users: {} },
      room_management: roomManagement,
      space_members: spaceMembers
    },
    ui: { timeline: { media_downloads: {} } }
  }
} as unknown as DesktopSnapshot;

function stagedThreadImage(caption: string): StagedUploadItem {
  return {
    staged_id: "staged-thread-image",
    room_id: room.room_id,
    position: 0,
    filename: "thread-image.png",
    mime_type: "image/png",
    byte_count: 128,
    kind: { kind: "image", width: 16, height: 16 },
    caption: caption ? documentFromText(caption) : null,
    compression_choice: { kind: "original" },
    preparation: {
      kind: "ready",
      variants: [
        {
          variant_id: "original-keep",
          resize: "original",
          format_choice: "keep",
          filename: "thread-image.png",
          mime_type: "image/png",
          byte_count: 128,
          width: 16,
          height: 16,
          format: "original",
          savings_percent: 0,
          metadata_stripped: false,
          thumbnail_refreshed: false
        }
      ],
      selected: { resize: "original", format: "keep" },
      pending: null,
      generation: 1
    }
  };
}

function threadSnapshot(caption: string): DesktopSnapshot {
  return {
    ...snapshot,
    state: {
      ...snapshot.state,
      domain: {
        ...snapshot.state.domain,
        live_signals: { presence: {} },
        mention_candidates: { targets: [] },
        settings: {
          values: {
            timeline: { auto_load_older_messages: false },
            display: { code_block_wrap: false }
          }
        },
        profile: { ignored_user_ids: [], users: {} },
        room_interactions: {},
        session: { user_id: "@current:example.invalid" }
      },
      ui: {
        ...snapshot.state.ui,
        thread: {
          kind: "open",
          room_id: room.room_id,
          root_event_id: "$root:example.invalid",
          intent: "existingThread",
          is_subscribed: true,
          composer: {
            accepted_submission_ids: [],
            pending_transaction_id: null,
            draft_revision: "0",
            last_accepted_clear_revision: "0",
            draft: "",
            document: { version: 2, inlines: [] },
            mode: "Plain"
          },
          staged_uploads: [stagedThreadImage(caption)]
        }
      }
    }
  } as unknown as DesktopSnapshot;
}

type RightPanelProps = Parameters<typeof ContextualRightPanel>[0];

const defaultProps = {
  activeRoom: room,
  activeSpace: space,
  activeSpaceName: space.display_name,
  isRecoveryBusy: false,
  mode: "people" as const,
  peoplePanelScope: { kind: "room" as const, roomId: room.room_id },
  recoverySecretFilled: false,
  snapshot,
  searchQuery: "",
  searchResults: [],
  savedSessions: [],
  onClosePanel: vi.fn(),
  onOpenThread: vi.fn(),
  onOpenFiles: vi.fn(),
  onRefreshFilesView: vi.fn(),
  onPaginateThreadsList: vi.fn(),
  onOpenRecovery: vi.fn(),
  onProbeLocalEncryption: vi.fn(),
  onResetLocalData: vi.fn(),
  onRecoverySecretPresenceChange: vi.fn(),
  onReply: vi.fn(),
  onResultSelect: vi.fn(),
  onSubmitRecovery: vi.fn(),
  onSwitchAccount: vi.fn(),
  onAcceptVerification: vi.fn(),
  onBootstrapCrossSigning: vi.fn(),
  onCancelVerification: vi.fn(),
  onConfirmSasVerification: vi.fn(),
  onExportRoomKeys: vi.fn(),
  onImportRoomKeys: vi.fn(),
  onBootstrapSecureBackup: vi.fn(),
  onChangeSecureBackupPassphrase: vi.fn(),
  onEnableKeyBackup: vi.fn(),
  onResetIdentity: vi.fn(),
  onCancelIdentityReset: vi.fn(),
  onSubmitIdentityResetOAuth: vi.fn(),
  onSubmitIdentityResetPassword: vi.fn(),
  onThreadComposerDraftChange: vi.fn(),
  onOpenProfile: vi.fn(),
  onInviteUserToSpace: vi.fn(),
  canInviteToSpace: true
} as unknown as RightPanelProps;

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

beforeEach(() => {
  MockIntersectionObserver.callback = null;
  vi.stubGlobal("IntersectionObserver", MockIntersectionObserver);
});

function renderPanel(overrides: Partial<RightPanelProps> = {}) {
  return render(<ContextualRightPanel {...defaultProps} {...overrides} />);
}

describe("PanelHeader", () => {
  test("exposes only its title and requested Close action", () => {
    const onClose = vi.fn();
    const title = t("panel.userSettings");
    render(<PanelHeader title={title} onClose={onClose} />);

    expect(screen.getByText(title)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "More" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: t("action.close", { title }) }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});

describe("ContextualRightPanel settings modal", () => {
  test("keeps the account tab strip inside account settings", async () => {
    const onSelectAccountTab = vi.fn();
    const settingsSnapshot = await createDesktopApiFixture().getSnapshot();
    const accountTabs = {
      selectedTabId: "work",
      tabs: [{
        id: "work",
        accountKey: "@work:matrix.org",
        homeserver: "https://matrix.org",
        displayName: "Work",
        avatarSourceRef: null,
        status: "ready",
        unreadCount: 0
      }],
      badgeCount: 0
    } satisfies AccountTabsSnapshot;

    renderPanel({
      mode: "userSettings",
      snapshot: settingsSnapshot,
      accountTabs,
      selectedAccountTabId: "work",
      onSelectAccountTab
    });

    expect(screen.getByRole("dialog", { name: t("settings.accountSettings") })).toBeTruthy();
    expect(screen.getByRole("navigation", { name: t("settings.accountSettings") })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", {
      name: t("accountTabs.select", {
        account: "Work",
        status: t("accountTabs.ready")
      })
    }));
    expect(onSelectAccountTab).toHaveBeenCalledWith("work");
  });
});

describe("ContextualRightPanel people composition", () => {
  test("forwards Space presentation data and the close action", () => {
    const onClosePanel = vi.fn();
    const administratorId = "@space-administrator:example.invalid";
    const presentationSnapshot = {
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          profile: {
            ignored_user_ids: [],
            users: {
              [administratorId]: {
                user_id: administratorId,
                display_name: "Space member",
                display_label: "Space member",
                original_display_label: "Space member",
                mention_search_terms: ["space", "member"],
                avatar: {
                  mxc_uri: "mxc://example.invalid/space-member-avatar",
                  thumbnail: {
                    kind: "ready",
                    source_ref: "asset://space-member-avatar",
                    width: null,
                    height: null,
                    mime_type: null
                  }
                }
              } satisfies UserProfile
            }
          },
          space_members: {
            ...spaceMembers,
            space_joined: [
              spaceMember(administratorId, "Space member", "space_joined", {
                role: "administrator",
      role_options: []
              }),
              spaceMember("@space-creator:example.invalid", "Space creator", "space_joined", {
                role: "creator",
      role_options: []
              })
            ],
            child_room_only: []
          }
        }
      }
    } as unknown as DesktopSnapshot;

    renderPanel({
      snapshot: presentationSnapshot,
      peoplePanelScope: { kind: "space", spaceId: space.space_id },
      onClosePanel
    });

    fireEvent.click(screen.getByRole("button", { name: "Close Space members" }));

    expect(onClosePanel).toHaveBeenCalledTimes(1);
    expect(screen.getByText("Administrator")).toBeTruthy();
    expect(screen.getByText("Creator")).toBeTruthy();
    expect(screen.getByRole("img", { name: "" }).querySelector("img")?.getAttribute("src")).toBe(
      "asset://space-member-avatar"
    );
  });

  test("forwards visibility-triggered Space avatar thumbnail requests", () => {
    const onRequestMemberAvatarThumbnail = vi.fn();
    const administratorId = "@space-administrator:example.invalid";
    const requestSnapshot = {
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          profile: {
            ignored_user_ids: [],
            users: {
              [administratorId]: {
                user_id: administratorId,
                display_name: "Space member",
                display_label: "Space member",
                original_display_label: "Space member",
                mention_search_terms: ["space", "member"],
                avatar: {
                  mxc_uri: "mxc://example.invalid/space-member-avatar",
                  thumbnail: { kind: "notRequested" }
                }
              } satisfies UserProfile
            }
          },
          space_members: {
            ...spaceMembers,
            space_joined: [
              spaceMember(administratorId, "Space member", "space_joined", {
                role: "administrator",
      role_options: []
              })
            ],
            child_room_only: []
          }
        }
      }
    } as unknown as DesktopSnapshot;

    renderPanel({
      snapshot: requestSnapshot,
      peoplePanelScope: { kind: "space", spaceId: space.space_id },
      onRequestMemberAvatarThumbnail
    });

    expect(onRequestMemberAvatarThumbnail).not.toHaveBeenCalled();
    const row = screen.getByText("Space member").closest("li");
    expect(row).not.toBeNull();
    MockIntersectionObserver.trigger(row!);

    expect(onRequestMemberAvatarThumbnail).toHaveBeenCalledTimes(1);
    expect(onRequestMemberAvatarThumbnail).toHaveBeenCalledWith(
      "mxc://example.invalid/space-member-avatar"
    );
  });

  test("keeps the Space header invite control mounted across a synthetic settings-slot transition (#1033)", () => {
    const spaceSettings = structuredClone(roomManagement.settings!);
    spaceSettings.room_id = space.space_id;
    const spaceSnapshot = (management: RoomManagementState): DesktopSnapshot =>
      ({
        ...snapshot,
        state: {
          ...snapshot.state,
          domain: { ...snapshot.state.domain, room_management: management },
          ui: { ...snapshot.state.ui, navigation: { active_space_id: space.space_id } }
        }
      }) as unknown as DesktopSnapshot;
    // Mirrors App's derivation: both inputs come from Rust-owned snapshot state.
    const propsFor = (next: DesktopSnapshot): Partial<RightPanelProps> => ({
      snapshot: next,
      peoplePanelScope: { kind: "space", spaceId: space.space_id },
      canInviteToSpace: Boolean(
        exactRoomSettingsForRoom(next, space.space_id)?.permissions.can_invite
      ),
      spaceInviteAvailabilityReason: spaceInviteAvailabilityReasonForSnapshot(next, space.space_id)
    });
    const authorized = spaceSnapshot({
      selected_room_id: space.space_id,
      settings: spaceSettings,
      operation: { kind: "idle" }
    });
    // A room-scoped settings load replaces the single room-management slot.
    const slotReplaced = spaceSnapshot(roomManagement);

    const { rerender } = renderPanel(propsFor(authorized));
    const trigger = screen.getByRole("button", { name: "Invite people" });
    expect(trigger).toHaveProperty("disabled", false);

    rerender(<ContextualRightPanel {...defaultProps} {...propsFor(slotReplaced)} />);
    expect(screen.getByRole("button", { name: "Invite people" })).toBe(trigger);
    expect(trigger).toHaveProperty("disabled", true);
    expect(trigger.getAttribute("data-invite-availability")).toBe("settings_unavailable");

    rerender(<ContextualRightPanel {...defaultProps} {...propsFor(authorized)} />);
    expect(screen.getByRole("button", { name: "Invite people" })).toBe(trigger);
    expect(trigger).toHaveProperty("disabled", false);
  });

  test("renders SpaceMembersPanel for a Space scope and forwards Space callbacks", () => {
    const onInviteUserToSpace = vi.fn();
    const onOpenProfile = vi.fn();
    const onOpenContextMenu = vi.fn();

    renderPanel({
      peoplePanelScope: { kind: "space", spaceId: space.space_id },
      onInviteUserToSpace,
      onOpenProfile,
      onOpenContextMenu,
      canInviteToSpace: true
    });

    expect(screen.getByRole("heading", { name: "Space members", level: 2 })).toBeTruthy();
    expect(screen.queryByRole("heading", { name: "People", level: 2 })).toBeNull();
    expect(screen.getByText("Space member")).toBeTruthy();
    expect(screen.getByText("Child member")).toBeTruthy();
    expect(screen.getByText("In child rooms: Alpha Room")).toBeTruthy();
    expect(screen.queryByText(room.room_id)).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Open profile for Space member" }));
    fireEvent.click(screen.getByRole("button", { name: "Invite to Space" }));
    fireEvent.contextMenu(screen.getByText("Child member").closest("li")!);

    expect(onOpenProfile).toHaveBeenCalledWith("@space-member:example.invalid");
    expect(onInviteUserToSpace).toHaveBeenCalledWith("@child-member:example.invalid");
    expect(onOpenContextMenu).toHaveBeenCalledWith(
      expect.anything(),
      {
        kind: "spaceMember",
        spaceId: space.space_id,
        userId: "@child-member:example.invalid",
        generation: 1
      },
      expect.arrayContaining([expect.objectContaining({ id: "inviteUserToSpace" })])
    );
  });

  test("forwards the inline Space invite cancellation callback and gate", () => {
    const invitedUserId = "@invited-member:example.invalid";
    const onCancelInvite = vi.fn();
    const cancellationSnapshot = structuredClone(snapshot);
    cancellationSnapshot.state.domain.space_members = {
      ...spaceMembers,
      space_invited: [spaceMember(invitedUserId, "Invited member", "space_invited")]
    };

    renderPanel({
      snapshot: cancellationSnapshot,
      peoplePanelScope: { kind: "space", spaceId: space.space_id },
      onCancelInvite,
      canCancelInvite: true,
      cancelAvailabilityReason: "available"
    });

    fireEvent.click(screen.getByRole("button", { name: "Cancel invitation" }));

    expect(onCancelInvite).toHaveBeenCalledWith(invitedUserId);
  });

  test("keeps a Room scope on PeoplePanel and does not classify from Space state", () => {
    renderPanel({
      peoplePanelScope: { kind: "room", roomId: room.room_id }
    });

    expect(screen.getByRole("heading", { name: "People", level: 2 })).toBeTruthy();
    expect(screen.getByText("Room member")).toBeTruthy();
    expect(screen.queryByText("Space member")).toBeNull();
    expect(screen.queryByText("Child member")).toBeNull();
    expect(screen.getByRole("searchbox", { name: "Search room members" })).toBeTruthy();
  });

  test("uses a child-room count when the only room label is its identifier fallback", () => {
    const identifierRoomId = "!identifier-only:example.invalid";
    const identifierRoom: RoomSummary = {
      ...room,
      room_id: identifierRoomId,
      display_name: identifierRoomId,
      display_label: identifierRoomId,
      original_display_label: identifierRoomId
    };
    const identifierSpace: SpaceSummary = {
      ...space,
      join_rule: null,
      child_room_ids: [identifierRoomId]
    };
    const identifierSnapshot = {
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          rooms: [identifierRoom],
          spaces: [identifierSpace],
          space_members: {
            ...spaceMembers,
            child_room_only: [
              spaceMember("@identifier-child:example.invalid", "Child member", "child_room_only", {
                child_room_ids: [identifierRoomId]
              })
            ],
            child_room_count: 1
          }
        }
      }
    } as unknown as DesktopSnapshot;

    renderPanel({
      snapshot: identifierSnapshot,
      activeRoom: identifierRoom,
      activeSpace: identifierSpace,
      peoplePanelScope: { kind: "space", spaceId: identifierSpace.space_id }
    });

    expect(screen.getByText("In 1 child room")).toBeTruthy();
    expect(screen.queryByText(identifierRoomId)).toBeNull();
  });
});

describe("ContextualRightPanel thread upload previews", () => {
  test("does not reload an unchanged preview for caption-only snapshots", async () => {
    const loadPreview = vi.fn(async () => [1, 2, 3]);
    const createObjectURL = vi
      .spyOn(URL, "createObjectURL")
      .mockReturnValue("blob:thread-preview");
    const revokeObjectURL = vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => undefined);
    const { rerender } = renderPanel({
      mode: "thread",
      snapshot: threadSnapshot("before"),
      onThreadLoadStagedUploadPreview: loadPreview
    });

    await waitFor(() =>
      expect(screen.getByRole("img", { name: "Prepared attachment preview" })).toBeTruthy()
    );
    expect(loadPreview).toHaveBeenCalledTimes(1);
    expect(createObjectURL).toHaveBeenCalledTimes(1);

    rerender(
      <ContextualRightPanel
        {...defaultProps}
        mode="thread"
        snapshot={threadSnapshot("after")}
        onThreadLoadStagedUploadPreview={loadPreview}
      />
    );

    expect(loadPreview).toHaveBeenCalledTimes(1);
    expect(
      screen.getByRole("img", { name: "Prepared attachment preview" }).getAttribute("src")
    ).toBe(
      "blob:thread-preview"
    );
    expect(revokeObjectURL).not.toHaveBeenCalled();
  });
});

describe("ContextualRightPanel secure-backup degradation", () => {
  test("disables the thread composer while encrypted sending is blocked", () => {
    renderPanel({
      mode: "thread",
      snapshot: threadSnapshot(""),
      encryptedComposerBlocked: true
    });

    expect(
      screen
        .getByRole("textbox", { name: "Thread composer" })
        .getAttribute("contenteditable")
    ).toBe("false");
    expect(
      (screen.getByRole("button", { name: "Send" }) as HTMLButtonElement).disabled
    ).toBe(true);
  });
});

// Issue #959: the thread pane is the same timeline surface as the room pane, so
// the sender profile handler must reach its rows through the shared route
// rightPanel -> TimelineView -> TimelineItemRow rather than a thread-local
// implementation.
describe("ContextualRightPanel thread sender profiles", () => {
  test("opens the sender profile from a thread reply", () => {
    const onOpenSenderProfile = vi.fn();
    const currentUserId = "@current:example.invalid";
    const rootEventId = "$root:example.invalid";
    const key = threadTimelineKey(currentUserId, room.room_id, rootEventId);
    const base = threadSnapshot("");
    const threadTimelineSnapshot = {
      ...base,
      state: {
        ...base.state,
        domain: {
          ...base.state.domain,
          live_signals: { presence: {}, rooms: {} },
          profile: { ...base.state.domain.profile, own: { avatar: null } },
          settings: {
            ...base.state.domain.settings,
            values: {
              ...base.state.domain.settings.values,
              appearance: { density: "default" }
            }
          }
        }
      }
    } as unknown as DesktopSnapshot;
    const store = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key,
        generation: 1,
        items: [
          {
            ...message("$reply:example.invalid", "Thread reply"),
            sender: "@other:example.invalid",
            sender_label: "Other Person"
          }
        ]
      }
    });

    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <ContextualRightPanel
          {...defaultProps}
          mode="thread"
          snapshot={threadTimelineSnapshot}
          timelineTransport={baseTransport({})}
          onOpenSenderProfile={onOpenSenderProfile}
        />
      </TimelineStoreContext.Provider>
    );

    fireEvent.click(screen.getByRole("button", { name: "Open profile for Other Person" }));
    expect(onOpenSenderProfile).toHaveBeenCalledWith(room.room_id, "@other:example.invalid");
  });
});

describe("ContextualRightPanel thread render isolation", () => {
  // App hands this panel freshly created closures on every render.
  function handlersLikeApp(): Partial<RightPanelProps> {
    return Object.fromEntries(
      Object.entries(defaultProps)
        .filter(([, value]) => typeof value === "function")
        .map(([name, value]) => [
          name,
          (...args: unknown[]) => (value as (...inner: unknown[]) => unknown)(...args)
        ])
    ) as Partial<RightPanelProps>;
  }

  test("an App render that changes no thread data leaves the thread timeline and composer alone", () => {
    const currentUserId = "@current:example.invalid";
    const rootEventId = "$root:example.invalid";
    const key = threadTimelineKey(currentUserId, room.room_id, rootEventId);
    const base = threadSnapshot("");
    const threadTimelineSnapshot = {
      ...base,
      state: {
        ...base.state,
        domain: {
          ...base.state.domain,
          live_signals: { presence: {}, rooms: {} },
          profile: { ...base.state.domain.profile, own: { avatar: null } },
          settings: {
            ...base.state.domain.settings,
            values: {
              ...base.state.domain.settings.values,
              appearance: { density: "default" }
            }
          }
        }
      }
    } as unknown as DesktopSnapshot;
    const storeContext = {
      store: applyTimelineEvent(createTimelineStore(), {
        InitialItems: {
          request_id: null,
          key,
          generation: 1,
          items: [message("$reply:example.invalid", "Thread reply")]
        }
      }),
      setStore: vi.fn()
    };
    const timelineTransport = baseTransport({});
    const panel = () => (
      <TimelineStoreContext.Provider value={storeContext}>
        <ContextualRightPanel
          {...defaultProps}
          {...handlersLikeApp()}
          mode="thread"
          snapshot={threadTimelineSnapshot}
          timelineTransport={timelineTransport}
          onOpenSenderProfile={() => undefined}
          onStartDirectMessage={() => undefined}
          onOpenMatrixTarget={() => undefined}
        />
      </TimelineStoreContext.Provider>
    );

    const { rerender } = render(panel());
    expect(screen.getByText("Thread reply")).toBeTruthy();
    const mounted = { ...renderCounts };

    rerender(panel());
    rerender(panel());

    expect(renderCounts.timelineView).toBe(mounted.timelineView);
    expect(renderCounts.composer).toBe(mounted.composer);
  });
  // The cost that #972 measured is the rows: an unrelated App render used to
  // re-render every row of the open thread. Row renders are the deterministic
  // stand-in for that time; the commit duration is logged, not asserted,
  // because wall-clock thresholds flake in CI.
  test("an unrelated App render re-renders no thread rows", () => {
    const currentUserId = "@current:example.invalid";
    const rootEventId = "$root:example.invalid";
    const key = threadTimelineKey(currentUserId, room.room_id, rootEventId);
    const base = threadSnapshot("");
    const threadTimelineSnapshot = {
      ...base,
      state: {
        ...base.state,
        domain: {
          ...base.state.domain,
          live_signals: { presence: {}, rooms: {} },
          profile: { ...base.state.domain.profile, own: { avatar: null } },
          settings: {
            ...base.state.domain.settings,
            values: {
              ...base.state.domain.settings.values,
              appearance: { density: "default" }
            }
          }
        }
      }
    } as unknown as DesktopSnapshot;
    const items = Array.from({ length: 13 }, (_, index) =>
      message(`$reply-${index}:example.invalid`, `Thread reply ${index}`)
    );
    const storeContext = {
      store: applyTimelineEvent(createTimelineStore(), {
        InitialItems: { request_id: null, key, generation: 1, items }
      }),
      setStore: vi.fn()
    };
    const timelineTransport = baseTransport({});
    const updateCommitsMs: number[] = [];
    const panel = () => (
      <Profiler
        id="thread-pane"
        onRender={(_id, phase, actualDuration) => {
          if (phase === "update") updateCommitsMs.push(actualDuration);
        }}
      >
        <TimelineStoreContext.Provider value={storeContext}>
          <ContextualRightPanel
            {...defaultProps}
            {...handlersLikeApp()}
            mode="thread"
            snapshot={threadTimelineSnapshot}
            timelineTransport={timelineTransport}
          />
        </TimelineStoreContext.Provider>
      </Profiler>
    );

    const { rerender } = render(panel());
    expect(renderCounts.rows).toBeGreaterThan(0);
    const rowsAfterMount = renderCounts.rows;
    updateCommitsMs.length = 0;

    const appRenders = 10;
    for (let index = 0; index < appRenders; index += 1) rerender(panel());

    console.info(
      `thread pane: ${renderCounts.rows - rowsAfterMount} row renders and ` +
        `${updateCommitsMs.reduce((total, ms) => total + ms, 0).toFixed(1)} ms of React render time ` +
        `over ${appRenders} unrelated App renders`
    );
    expect(renderCounts.rows).toBe(rowsAfterMount);
  });
});
