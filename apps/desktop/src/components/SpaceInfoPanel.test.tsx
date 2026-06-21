// @vitest-environment jsdom
import { renderToStaticMarkup } from "react-dom/server";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test } from "vitest";

import { SpaceInfoPanel } from "./SpaceInfoPanel";

afterEach(cleanup);

describe("SpaceInfoPanel", () => {
  test("renders space identity, child rooms, unread total, and Element-like entries", () => {
    const markup = renderToStaticMarkup(
      <SpaceInfoPanel
        fallbackName="Synthetic Workspace"
        rooms={[
          {
            room_id: "!room-alpha:example.invalid",
            display_name: "Alpha Upstream",
            display_label: "Alpha Local",
            original_display_label: "Alpha Upstream",
            avatar: null,
            is_dm: false,
            dm_user_ids: [],
            tags: { favourite: null, low_priority: null },
            parent_space_ids: ["!space-work:example.invalid"],
            dm_space_ids: [],
            is_encrypted: false,
            unread_count: 8
          },
          {
            room_id: "!room-beta:example.invalid",
            display_name: "Beta Room",
            display_label: "Beta Room",
            original_display_label: "Beta Room",
            avatar: null,
            is_dm: false,
            dm_user_ids: [],
            tags: { favourite: null, low_priority: null },
            parent_space_ids: ["!space-work:example.invalid"],
            dm_space_ids: [],
            is_encrypted: false,
            unread_count: 2
          },
          {
            room_id: "!dm-alice:example.invalid",
            display_name: "Alice",
            display_label: "Alice",
            original_display_label: "Alice",
            avatar: null,
            is_dm: true,
            dm_user_ids: ["@alice:example.invalid"],
            tags: { favourite: null, low_priority: null },
            parent_space_ids: ["!space-work:example.invalid"],
            dm_space_ids: [],
            is_encrypted: false,
            unread_count: 4
          }
        ]}
        space={{
          space_id: "!space-work:example.invalid",
          display_name: "Synthetic Workspace",
          avatar: null,
          child_room_ids: ["!room-alpha:example.invalid", "!room-beta:example.invalid"]
        }}
      />
    );

    expect(markup).toContain("Synthetic Workspace");
    expect(markup).toContain("!space-work:example.invalid");
    expect(markup).toContain("Rooms");
    expect(markup).toContain("2");
    expect(markup).toContain("Unread");
    expect(markup).toContain("10");
    expect(markup).toContain("Alpha Local");
    expect(markup).not.toContain("Alpha Upstream");
    expect(markup).toContain("Beta Room");
    expect(markup).not.toContain("Alice");
    expect(markup).toContain("Home");
    expect(markup).toContain("Preferences");
    expect(markup).toContain("Space settings");
    expect(markup).toContain("Invite");
    expect(markup).toContain("Space preferences");
    expect(markup).toContain("Room membership");
    expect(markup).toContain("Child rooms");
    expect(markup).toContain("Direct messages");
    expect(markup).toContain("Global DM list");
  });

  test("renders account home summary when no Space is selected", () => {
    const markup = renderToStaticMarkup(
      <SpaceInfoPanel
        fallbackName="Home"
        rooms={[
          {
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
            unread_count: 8
          },
          {
            room_id: "!dm-alice:example.invalid",
            display_name: "Alice",
            display_label: "Alice",
            original_display_label: "Alice",
            avatar: null,
            is_dm: true,
            dm_user_ids: ["@alice:example.invalid"],
            tags: { favourite: null, low_priority: null },
            parent_space_ids: [],
            dm_space_ids: [],
            is_encrypted: false,
            unread_count: 4
          }
        ]}
        space={null}
      />
    );

    expect(markup).toContain("Home");
    expect(markup).toContain("All rooms");
    expect(markup).toContain("Alpha Room");
    expect(markup).not.toContain("Alice");
  });

  test("starts a direct message from a loaded space member row", () => {
    const startedUserIds: string[] = [];
    render(
      <SpaceInfoPanel
        fallbackName="Synthetic Workspace"
        rooms={[]}
        space={{
          space_id: "!space-work:example.invalid",
          display_name: "Synthetic Workspace",
          avatar: null,
          child_room_ids: []
        }}
        onStartDirectMessage={(userId) => {
          startedUserIds.push(userId);
        }}
        roomManagement={{
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
            members: [
              {
                user_id: "@member:example.invalid",
                display_name: "Upstream Member",
                display_label: "Local Remark",
                original_display_label: "Upstream Member",
                avatar_url: null,
                power_level: 0,
                role: "user"
              }
            ]
          },
          operation: { kind: "idle" }
        }}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Message Local Remark" }));

    expect(startedUserIds).toEqual(["@member:example.invalid"]);
  });

  test("renders every loaded space member with a direct-message entry point", () => {
    render(
      <SpaceInfoPanel
        fallbackName="Synthetic Workspace"
        rooms={[]}
        space={{
          space_id: "!space-work:example.invalid",
          display_name: "Synthetic Workspace",
          avatar: null,
          child_room_ids: []
        }}
        onStartDirectMessage={() => undefined}
        roomManagement={{
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
            members: [
              {
                user_id: "@ada:example.invalid",
                display_name: "Ada",
                display_label: "Ada",
                original_display_label: "Ada",
                avatar_url: null,
                power_level: 0,
                role: "user"
              },
              {
                user_id: "@grace:example.invalid",
                display_name: "Grace",
                display_label: "Grace",
                original_display_label: "Grace",
                avatar_url: null,
                power_level: 0,
                role: "user"
              },
              {
                user_id: "@linus:example.invalid",
                display_name: "Linus",
                display_label: "Linus",
                original_display_label: "Linus",
                avatar_url: null,
                power_level: 0,
                role: "user"
              }
            ]
          },
          operation: { kind: "idle" }
        }}
      />
    );

    for (const member of ["Ada", "Grace", "Linus"]) {
      expect(screen.getByText(member)).toBeTruthy();
      expect(screen.getByRole("button", { name: `Message ${member}` })).toBeTruthy();
    }
    for (const userId of [
      "@ada:example.invalid",
      "@grace:example.invalid",
      "@linus:example.invalid"
    ]) {
      expect(screen.getByText(userId)).toBeTruthy();
    }
  });
});
