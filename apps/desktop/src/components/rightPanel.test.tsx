// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

import type {
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
import { t } from "../i18n/messages";
import { ContextualRightPanel, PanelHeader } from "./rightPanel";

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
  display_name: "Workspace",
  avatar: null,
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
  onOpenKeyboardSettings: vi.fn(),
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
