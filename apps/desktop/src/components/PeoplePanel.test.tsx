// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";

import { PeoplePanel, ProfilePanel } from "./PeoplePanel";
import type {
  RoomManagementState,
  RoomMemberSummary,
  RoomSummary,
  SpaceSummary,
  UserProfile
} from "../domain/types";

const baseRoom: RoomSummary = {
  room_id: "!room-alpha:example.invalid",
  display_name: "Alpha Room",
  display_label: "Alpha Room",
  original_display_label: "Alpha Room",
  avatar: null,
  is_dm: false,
  dm_user_ids: [],
  tags: { favourite: null, low_priority: null },
  parent_space_ids: [],
  dm_space_ids: [],
  is_encrypted: false,
  unread_count: 0
};

const baseSpace: SpaceSummary = {
  space_id: "!space-work:example.invalid",
  display_name: "Synthetic Workspace",
  avatar: null,
  child_room_ids: []
};

const roomManagement = (members: RoomMemberSummary[]): RoomManagementState => ({
  selected_room_id: "!room-alpha:example.invalid",
  settings: {
    room_id: "!room-alpha:example.invalid",
    name: "Alpha Room",
    topic: null,
    avatar_url: null,
    join_rule: "invite",
    history_visibility: "shared",
    permissions: {
      can_edit_settings: true,
      can_edit_roles: true,
      can_kick: true,
      can_ban: true,
      can_unban: false
    },
    members
  },
  operation: { kind: "idle" }
});

const spaceManagement = (members: RoomMemberSummary[]): RoomManagementState => ({
  selected_room_id: "!space-work:example.invalid",
  settings: {
    room_id: "!space-work:example.invalid",
    name: "Synthetic Workspace",
    topic: null,
    avatar_url: null,
    join_rule: "invite",
    history_visibility: "shared",
    permissions: {
      can_edit_settings: false,
      can_edit_roles: false,
      can_kick: false,
      can_ban: false,
      can_unban: false
    },
    members
  },
  operation: { kind: "idle" }
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("PeoplePanel", () => {
  const members: RoomMemberSummary[] = [
    {
      user_id: "@ada:example.invalid",
      display_name: "Ada Lovelace",
      display_label: "Ada Lovelace",
      original_display_label: "Ada Lovelace",
      avatar_url: null,
      power_level: 100,
      role: "administrator"
    },
    {
      user_id: "@grace:example.invalid",
      display_name: "Grace Hopper",
      display_label: "Grace Hopper",
      original_display_label: "Grace Hopper",
      avatar_url: null,
      power_level: 50,
      role: "moderator"
    },
    {
      user_id: "@current:example.invalid",
      display_name: "Current User",
      display_label: "Current User",
      original_display_label: "Current User",
      avatar_url: null,
      power_level: 0,
      role: "user"
    }
  ];

  test("renders a sorted member list with role labels", () => {
    render(
      <PeoplePanel
        currentUserId="@current:example.invalid"
        roomOrSpace={baseRoom}
        roomManagement={roomManagement(members)}
        onOpenProfile={() => undefined}
      />
    );

    const names = screen
      .getAllByRole("button", { name: /^Open profile for/ })
      .map((button) => button.getAttribute("aria-label"));
    expect(names).toEqual([
      "Open profile for Ada Lovelace",
      "Open profile for Current User",
      "Open profile for Grace Hopper"
    ]);
    expect(screen.getByText("Administrator")).toBeTruthy();
    expect(screen.getByText("Moderator")).toBeTruthy();
    expect(screen.getByText("You")).toBeTruthy();
  });

  test("bounds rendered rows for large member lists", () => {
    const manyMembers: RoomMemberSummary[] = Array.from({ length: 300 }, (_, index) => {
      const suffix = String(index).padStart(3, "0");
      return {
        user_id: `@member-${suffix}:example.invalid`,
        display_name: `Member ${suffix}`,
        display_label: `Member ${suffix}`,
        original_display_label: `Member ${suffix}`,
        avatar_url: null,
        power_level: 0,
        role: "user"
      };
    });

    render(
      <PeoplePanel
        currentUserId="@current:example.invalid"
        roomOrSpace={baseRoom}
        roomManagement={roomManagement(manyMembers)}
        onOpenProfile={() => undefined}
      />
    );

    expect(screen.getByText("300 members")).toBeTruthy();
    expect(screen.getAllByRole("button", { name: /^Open profile for Member/ })).toHaveLength(22);
    expect(screen.queryByRole("button", { name: "Open profile for Member 299" })).toBeNull();
  });

  test("opens the profile when a member row is clicked", () => {
    const onOpenProfile = vi.fn();
    render(
      <PeoplePanel
        currentUserId="@current:example.invalid"
        roomOrSpace={baseRoom}
        roomManagement={roomManagement(members)}
        onOpenProfile={onOpenProfile}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Open profile for Ada Lovelace" }));
    expect(onOpenProfile).toHaveBeenCalledWith("@ada:example.invalid");
  });

  test("starts a direct message from the row action", () => {
    const onStartDirectMessage = vi.fn();
    render(
      <PeoplePanel
        currentUserId="@current:example.invalid"
        roomOrSpace={baseRoom}
        roomManagement={roomManagement(members)}
        onOpenProfile={() => undefined}
        onStartDirectMessage={onStartDirectMessage}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Message Ada Lovelace" }));
    expect(onStartDirectMessage).toHaveBeenCalledWith("@ada:example.invalid");
  });

  test("does not show a message action for the current user", () => {
    render(
      <PeoplePanel
        currentUserId="@current:example.invalid"
        roomOrSpace={baseRoom}
        roomManagement={roomManagement(members)}
        onOpenProfile={() => undefined}
        onStartDirectMessage={() => undefined}
      />
    );

    expect(screen.queryByRole("button", { name: "Message Current User" })).toBeNull();
  });

  test("filters members by display name or user id", () => {
    render(
      <PeoplePanel
        currentUserId="@current:example.invalid"
        roomOrSpace={baseRoom}
        roomManagement={roomManagement(members)}
        onOpenProfile={() => undefined}
      />
    );

    const search = screen.getByRole("searchbox");
    fireEvent.change(search, { target: { value: "grace" } });

    expect(screen.getByRole("button", { name: "Open profile for Grace Hopper" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Open profile for Ada Lovelace" })).toBeNull();
  });

  test("prefers room-scoped member label over global profile cache", () => {
    render(
      <PeoplePanel
        currentUserId="@current:example.invalid"
        roomOrSpace={baseRoom}
        roomManagement={roomManagement(members)}
        onOpenProfile={() => undefined}
      />
    );

    expect(screen.getByRole("button", { name: "Open profile for Grace Hopper" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Open profile for Amazing Grace" })).toBeNull();
  });

  test("searches by original display label but not by global profile labels", () => {
    const localMembers: RoomMemberSummary[] = [
      {
        user_id: "@grace:example.invalid",
        display_name: "Grace Hopper",
        display_label: "Amazing Grace",
        original_display_label: "Grace Hopper",
        avatar_url: null,
        power_level: 50,
        role: "moderator"
      }
    ];
    render(
      <PeoplePanel
        currentUserId="@current:example.invalid"
        roomOrSpace={baseRoom}
        roomManagement={roomManagement(localMembers)}
        onOpenProfile={() => undefined}
      />
    );

    const search = screen.getByRole("searchbox");
    fireEvent.change(search, { target: { value: "hopper" } });

    expect(screen.getByRole("button", { name: "Open profile for Amazing Grace" })).toBeTruthy();

    fireEvent.change(search, { target: { value: "admiral" } });

    expect(screen.queryByRole("button", { name: "Open profile for Amazing Grace" })).toBeNull();
  });

  test("shows an empty state when a search has no results", () => {
    render(
      <PeoplePanel
        currentUserId="@current:example.invalid"
        roomOrSpace={baseRoom}
        roomManagement={roomManagement(members)}
        onOpenProfile={() => undefined}
      />
    );

    const search = screen.getByRole("searchbox");
    fireEvent.change(search, { target: { value: "zzz" } });

    expect(screen.getByRole("status")).toBeTruthy();
  });

  test("invites people from the primary action", () => {
    const onInvitePeople = vi.fn();
    render(
      <PeoplePanel
        currentUserId="@current:example.invalid"
        roomOrSpace={baseRoom}
        roomManagement={roomManagement(members)}
        onOpenProfile={() => undefined}
        onInvitePeople={onInvitePeople}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Invite people" }));
    expect(onInvitePeople).toHaveBeenCalledTimes(1);
  });

  test("closes the panel from the header close button", () => {
    const onClose = vi.fn();
    render(
      <PeoplePanel
        currentUserId="@current:example.invalid"
        roomOrSpace={baseRoom}
        roomManagement={roomManagement(members)}
        onOpenProfile={() => undefined}
        onClose={onClose}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Close People" }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  test("renders space members from room-management settings", () => {
    render(
      <PeoplePanel
        currentUserId="@current:example.invalid"
        roomOrSpace={baseSpace}
        roomManagement={spaceManagement(members)}
        onOpenProfile={() => undefined}
      />
    );

    const names = screen
      .getAllByRole("button", { name: /^Open profile for/ })
      .map((button) => button.getAttribute("aria-label"));
    expect(names).toEqual([
      "Open profile for Ada Lovelace",
      "Open profile for Current User",
      "Open profile for Grace Hopper"
    ]);
  });

  test("opens a space member profile when a row is clicked", () => {
    const onOpenProfile = vi.fn();
    render(
      <PeoplePanel
        currentUserId="@current:example.invalid"
        roomOrSpace={baseSpace}
        roomManagement={spaceManagement(members)}
        onOpenProfile={onOpenProfile}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Open profile for Ada Lovelace" }));
    expect(onOpenProfile).toHaveBeenCalledWith("@ada:example.invalid");
  });

  test("starts a direct message from a space member row", () => {
    const onStartDirectMessage = vi.fn();
    render(
      <PeoplePanel
        currentUserId="@current:example.invalid"
        roomOrSpace={baseSpace}
        roomManagement={spaceManagement(members)}
        onOpenProfile={() => undefined}
        onStartDirectMessage={onStartDirectMessage}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Message Ada Lovelace" }));
    expect(onStartDirectMessage).toHaveBeenCalledWith("@ada:example.invalid");
  });

  test("shows member role in space member profile", () => {
    render(
      <ProfilePanel
        userId="@ada:example.invalid"
        currentUserId="@current:example.invalid"
        roomOrSpace={baseSpace}
        roomManagement={spaceManagement(members)}
        profileUsers={{}}
        onBack={() => undefined}
      />
    );

    expect(screen.getByText("Ada Lovelace")).toBeTruthy();
    expect(screen.getByText("Administrator")).toBeTruthy();
  });
});

describe("ProfilePanel", () => {
  const members: RoomMemberSummary[] = [
    {
      user_id: "@ada:example.invalid",
      display_name: "Ada Lovelace",
      display_label: "Ada Lovelace",
      original_display_label: "Ada Lovelace",
      avatar_url: null,
      power_level: 100,
      role: "administrator"
    }
  ];

  test("renders identity and room role", () => {
    render(
      <ProfilePanel
        userId="@ada:example.invalid"
        currentUserId="@current:example.invalid"
        roomOrSpace={baseRoom}
        roomManagement={roomManagement(members)}
        profileUsers={{}}
        onBack={() => undefined}
      />
    );

    expect(screen.getByText("Ada Lovelace")).toBeTruthy();
    expect(screen.getByText("@ada:example.invalid")).toBeTruthy();
    expect(screen.getByText("Administrator")).toBeTruthy();
  });

  test("navigates back to the people list", () => {
    const onBack = vi.fn();
    render(
      <ProfilePanel
        userId="@ada:example.invalid"
        currentUserId="@current:example.invalid"
        roomOrSpace={baseRoom}
        roomManagement={roomManagement(members)}
        profileUsers={{}}
        onBack={onBack}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(onBack).toHaveBeenCalledTimes(1);
  });

  test("closes the profile panel from the header close button", () => {
    const onClose = vi.fn();
    render(
      <ProfilePanel
        userId="@ada:example.invalid"
        currentUserId="@current:example.invalid"
        roomOrSpace={baseRoom}
        roomManagement={roomManagement(members)}
        profileUsers={{}}
        onBack={() => undefined}
        onClose={onClose}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Close Profile" }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  test("starts a direct message from the profile", () => {
    const onStartDirectMessage = vi.fn();
    render(
      <ProfilePanel
        userId="@ada:example.invalid"
        currentUserId="@current:example.invalid"
        roomOrSpace={baseRoom}
        roomManagement={roomManagement(members)}
        profileUsers={{}}
        onBack={() => undefined}
        onStartDirectMessage={onStartDirectMessage}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    expect(onStartDirectMessage).toHaveBeenCalledWith("@ada:example.invalid");
  });

  test("updates room power level from the profile", () => {
    const onUpdateMemberRole = vi.fn();
    render(
      <ProfilePanel
        userId="@ada:example.invalid"
        currentUserId="@current:example.invalid"
        roomOrSpace={baseRoom}
        roomManagement={roomManagement(members)}
        profileUsers={{}}
        onBack={() => undefined}
        onUpdateMemberRole={onUpdateMemberRole}
      />
    );

    fireEvent.change(screen.getByRole("combobox", { name: "Member role for Ada Lovelace" }), {
      target: { value: "50" }
    });

    expect(onUpdateMemberRole).toHaveBeenCalledWith(
      "!room-alpha:example.invalid",
      "@ada:example.invalid",
      50
    );
  });

  test("routes profile moderation and safety actions", () => {
    const onIgnoreUser = vi.fn();
    const onReportUser = vi.fn();
    const onModerateMember = vi.fn();
    render(
      <ProfilePanel
        userId="@ada:example.invalid"
        currentUserId="@current:example.invalid"
        ignoredUserIds={[]}
        roomOrSpace={baseRoom}
        roomManagement={roomManagement(members)}
        profileUsers={{}}
        onBack={() => undefined}
        onIgnoreUser={onIgnoreUser}
        onReportUser={onReportUser}
        onModerateMember={onModerateMember}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Ignore" }));
    fireEvent.click(screen.getByRole("button", { name: "Report user" }));
    fireEvent.click(screen.getByRole("button", { name: "Kick Ada Lovelace" }));
    fireEvent.click(screen.getByRole("button", { name: "Ban Ada Lovelace" }));

    expect(onIgnoreUser).toHaveBeenCalledWith("@ada:example.invalid");
    expect(onReportUser).toHaveBeenCalledWith("@ada:example.invalid");
    expect(onModerateMember).toHaveBeenCalledWith(
      "!room-alpha:example.invalid",
      "@ada:example.invalid",
      "kick",
      null
    );
    expect(onModerateMember).toHaveBeenCalledWith(
      "!room-alpha:example.invalid",
      "@ada:example.invalid",
      "ban",
      null
    );
  });

  test("does not expose Unban for an active room member", () => {
    const activeMemberManagement = roomManagement(members);
    activeMemberManagement.settings!.permissions.can_unban = true;

    render(
      <ProfilePanel
        userId="@ada:example.invalid"
        currentUserId="@current:example.invalid"
        ignoredUserIds={[]}
        roomOrSpace={baseRoom}
        roomManagement={activeMemberManagement}
        profileUsers={{}}
        onBack={() => undefined}
        onModerateMember={() => undefined}
      />
    );

    expect(screen.getByRole("button", { name: "Kick Ada Lovelace" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Ban Ada Lovelace" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Unban Ada Lovelace" })).toBeNull();
  });

  test("routes unignore for ignored profile users", () => {
    const onUnignoreUser = vi.fn();
    render(
      <ProfilePanel
        userId="@ada:example.invalid"
        currentUserId="@current:example.invalid"
        ignoredUserIds={["@ada:example.invalid"]}
        roomOrSpace={baseRoom}
        roomManagement={roomManagement(members)}
        profileUsers={{}}
        onBack={() => undefined}
        onUnignoreUser={onUnignoreUser}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Unignore" }));
    expect(onUnignoreUser).toHaveBeenCalledWith("@ada:example.invalid");
  });

  test("saves a local alias", () => {
    const onSetLocalUserAlias = vi.fn();
    render(
      <ProfilePanel
        userId="@ada:example.invalid"
        currentUserId="@current:example.invalid"
        roomOrSpace={baseRoom}
        roomManagement={roomManagement(members)}
        profileUsers={{}}
        onBack={() => undefined}
        onSetLocalUserAlias={onSetLocalUserAlias}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Set alias" }));
    const input = screen.getByRole("textbox");
    fireEvent.change(input, { target: { value: "Legend" } });
    fireEvent.click(screen.getByRole("button", { name: "Save alias" }));

    expect(onSetLocalUserAlias).toHaveBeenCalledWith("@ada:example.invalid", "Legend");
  });

  test("clears a local alias when the input is empty", () => {
    const onSetLocalUserAlias = vi.fn();
    render(
      <ProfilePanel
        userId="@ada:example.invalid"
        currentUserId="@current:example.invalid"
        roomOrSpace={baseRoom}
        roomManagement={roomManagement(members)}
        profileUsers={{}}
        onBack={() => undefined}
        onSetLocalUserAlias={onSetLocalUserAlias}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Set alias" }));
    const input = screen.getByRole("textbox");
    fireEvent.change(input, { target: { value: "   " } });
    fireEvent.click(screen.getByRole("button", { name: "Save alias" }));

    expect(onSetLocalUserAlias).toHaveBeenCalledWith("@ada:example.invalid", null);
  });

  test("prefers room-scoped member label over global profile cache in profile detail", () => {
    const profileUsers: Record<string, UserProfile> = {
      "@ada:example.invalid": {
        user_id: "@ada:example.invalid",
        display_name: null,
        display_label: "Countess Ada",
        original_display_label: "Ada Lovelace",
        avatar: null,
        mention_search_terms: []
      }
    };

    render(
      <ProfilePanel
        userId="@ada:example.invalid"
        currentUserId="@current:example.invalid"
        roomOrSpace={baseRoom}
        roomManagement={roomManagement(members)}
        profileUsers={profileUsers}
        onBack={() => undefined}
      />
    );

    expect(screen.getByText("Ada Lovelace")).toBeTruthy();
    expect(screen.queryByText("Countess Ada")).toBeNull();
  });

  test("renders profile data from the global profile cache when no room member", () => {
    const profileUsers: Record<string, UserProfile> = {
      "@ada:example.invalid": {
        user_id: "@ada:example.invalid",
        display_name: null,
        display_label: "Countess Ada",
        original_display_label: "Ada Lovelace",
        avatar: null,
        mention_search_terms: []
      }
    };

    render(
      <ProfilePanel
        userId="@ada:example.invalid"
        currentUserId="@current:example.invalid"
        roomOrSpace={baseRoom}
        roomManagement={roomManagement([])}
        profileUsers={profileUsers}
        onBack={() => undefined}
      />
    );

    expect(screen.getByText("Countess Ada")).toBeTruthy();
  });
});
