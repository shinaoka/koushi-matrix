import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, test } from "vitest";

import { RoomInfoPanel } from "./RoomInfoPanel";

describe("RoomInfoPanel", () => {
  test("renders room identity, membership context, and Element-like settings entries", () => {
    const markup = renderToStaticMarkup(
      <RoomInfoPanel
        room={{
          room_id: "!room-alpha:example.invalid",
          display_name: "Alpha Room",
          avatar: null,
          is_dm: false,
          tags: { favourite: null, low_priority: null },
          parent_space_ids: ["!space-work:example.invalid"],
          unread_count: 8
        }}
        spaces={[
          {
            space_id: "!space-work:example.invalid",
            display_name: "Synthetic Workspace",
            avatar: null,
            child_room_ids: ["!room-alpha:example.invalid"]
          }
        ]}
      />
    );

    expect(markup).toContain("Alpha Room");
    expect(markup).toContain("!room-alpha:example.invalid");
    expect(markup).toContain("Room");
    expect(markup).toContain("Unread");
    expect(markup).toContain("8");
    expect(markup).toContain("Synthetic Workspace");
    expect(markup).toContain("People");
    expect(markup).toContain("Files");
    expect(markup).toContain("Notifications");
    expect(markup).toContain("Room settings");
    expect(markup).toContain("Timeline");
    expect(markup).toContain("Subscribed");
    expect(markup).toContain("Search index");
    expect(markup).toContain("Exact verified results");
    expect(markup).toContain("DM list");
    expect(markup).toContain("Room scoped");
  });

  test("labels direct messages distinctly from rooms", () => {
    const markup = renderToStaticMarkup(
      <RoomInfoPanel
        room={{
          room_id: "!dm-alice:example.invalid",
          display_name: "Alice",
          avatar: null,
          is_dm: true,
          tags: { favourite: null, low_priority: null },
          parent_space_ids: [],
          unread_count: 0
        }}
        spaces={[]}
      />
    );

    expect(markup).toContain("Direct message");
    expect(markup).toContain("No Spaces");
  });
});
