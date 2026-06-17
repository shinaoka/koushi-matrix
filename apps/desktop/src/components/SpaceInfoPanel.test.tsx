import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, test } from "vitest";

import { SpaceInfoPanel } from "./SpaceInfoPanel";

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
});
