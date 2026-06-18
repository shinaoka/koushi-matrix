// @vitest-environment jsdom
import { renderToStaticMarkup } from "react-dom/server";
import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";

import { RoomInfoPanel } from "./RoomInfoPanel";

afterEach(cleanup);
import type {
  LinkPreviewSettingsState,
  RoomNotificationSettings,
  RoomSummary,
  SettingsState
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
  parent_space_ids: ["!space-work:example.invalid"],
  is_encrypted: false,
  unread_count: 8
};

const idleSettings: RoomNotificationSettings = {
  mode: { kind: "all" },
  operation: { kind: "idle" }
};

const pendingSettings: RoomNotificationSettings = {
  mode: { kind: "mute" },
  operation: { kind: "pending", request_id: 1 }
};

const baseAppSettings: SettingsState = {
  values: {
    locale: { language_tag: null, text_direction: "auto" },
    appearance: { theme: "dark" },
    typography: { font: "system", emoji: "system" },
    keyboard: { composer_send_shortcut: "enter" },
    notifications: {
      desktop_notifications: true,
      sound: true,
      badges: true,
      send_read_receipts: true,
      send_typing_notifications: true
    },
    display: {
      code_block_wrap: true,
      hide_redacted: false,
      url_previews_enabled: true,
      encrypted_url_previews_enabled: false
    },
    media: {
      image_upload_compression: "never",
      image_upload_compression_policy: {
        threshold_bytes: 1048576,
        threshold_long_edge: 2560,
        target_long_edge: 2048,
        quality_percent: 82
      }
    },
    timeline: {
      auto_load_older_messages: false
    },
    search_crawler: {
      speed: "standard" as const,
      include_media_captions: true,
      include_filenames: true
    }
  },
  persistence: { kind: "idle" }
};

const baseLinkPreviewSettings: LinkPreviewSettingsState = {
  room_overrides: {}
};

describe("RoomInfoPanel", () => {
  test("renders room identity, membership context, and Element-like settings entries", () => {
    const markup = renderToStaticMarkup(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
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
          ...baseRoom,
          room_id: "!dm-alice:example.invalid",
          display_name: "Alice",
          display_label: "Alice",
          original_display_label: "Alice",
          is_dm: true,
          dm_user_ids: ["@alice:example.invalid"],
          parent_space_ids: [],
          unread_count: 0
        }}
        roomNotificationSettings={idleSettings}
        spaces={[]}
      />
    );

    expect(markup).toContain("Direct message");
    expect(markup).toContain("No Spaces");
  });

  test("renders room titles from the Rust-projected display label", () => {
    const markup = renderToStaticMarkup(
      <RoomInfoPanel
        room={{
          ...baseRoom,
          room_id: "!dm-alice:example.invalid",
          display_name: "Alice Upstream",
          display_label: "Alice Local",
          original_display_label: "Alice Upstream",
          is_dm: true,
          dm_user_ids: ["@alice:example.invalid"],
          parent_space_ids: [],
          unread_count: 0
        }}
        roomNotificationSettings={idleSettings}
        spaces={[]}
      />
    );

    expect(markup).toContain("Alice Local");
    expect(markup).not.toContain("Alice Upstream");
  });

  test("renders room member labels from the Rust-projected display label", () => {
    const markup = renderToStaticMarkup(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        roomManagement={{
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

    expect(markup).toContain("Local Remark");
    expect(markup).toContain("Kick Local Remark");
  });

  test("renders alias edit controls with Rust-projected original member context", () => {
    const markup = renderToStaticMarkup(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        onSetLocalUserAlias={() => undefined}
        roomManagement={{
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

    expect(markup).toContain("Local Remark");
    expect(markup).toContain("Original: Upstream Member");
    expect(markup).toContain("Edit alias for Local Remark");
    expect(markup).toContain("Clear alias for Local Remark");
  });

  test("does not synthesize room member labels when the projected label is empty", () => {
    const markup = renderToStaticMarkup(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        roomManagement={{
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
            members: [
              {
                user_id: "@member:example.invalid",
                display_name: "Upstream Member",
                display_label: "",
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

    expect(markup).not.toContain("Upstream Member");
    expect(markup).not.toContain("Kick @member:example.invalid");
  });

  test("renders notification mode options", () => {
    const markup = renderToStaticMarkup(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        onSetRoomNotificationMode={() => undefined}
      />
    );

    expect(markup).toContain("All messages");
    expect(markup).toContain("Mentions only");
    expect(markup).toContain("Mute");
  });

  test("selects the current notification mode", () => {
    const markup = renderToStaticMarkup(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={{
          mode: { kind: "mentions" },
          operation: { kind: "idle" }
        }}
        spaces={[]}
        onSetRoomNotificationMode={() => undefined}
      />
    );

    expect(markup).toContain('value="mentions"');
  });

  test("disables the notification select while a mode change is pending", () => {
    const markup = renderToStaticMarkup(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={pendingSettings}
        spaces={[]}
        onSetRoomNotificationMode={() => undefined}
      />
    );

    expect(markup).toContain('disabled=""');
    expect(markup).toContain('value="mute"');
  });

  test("disables the notification select when no handler is provided", () => {
    const markup = renderToStaticMarkup(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
      />
    );

    expect(markup).toContain('disabled=""');
  });
});

describe("RoomInfoPanel URL previews", () => {
  test("renders the URL-preview section when settings and handler are supplied", () => {
    const { getByRole } = render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        appSettings={baseAppSettings}
        linkPreviewSettings={baseLinkPreviewSettings}
        onSetRoomUrlPreviewOverride={() => undefined}
      />
    );

    expect(
      getByRole("switch", { name: "Enable link previews for this room" })
    ).toBeDefined();
  });

  test("checks the toggle for an unencrypted room that falls back to the global setting", () => {
    const { getByRole } = render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        appSettings={baseAppSettings}
        linkPreviewSettings={baseLinkPreviewSettings}
        onSetRoomUrlPreviewOverride={() => undefined}
      />
    );

    const toggle = getByRole("switch", {
      name: "Enable link previews for this room"
    });
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    expect(toggle.hasAttribute("disabled")).toBe(false);
  });

  test("unchecks but keeps the toggle enabled for encrypted rooms by default", () => {
    const { getByRole, getByText } = render(
      <RoomInfoPanel
        room={{ ...baseRoom, is_encrypted: true }}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        appSettings={baseAppSettings}
        linkPreviewSettings={baseLinkPreviewSettings}
        onSetRoomUrlPreviewOverride={() => undefined}
      />
    );

    const toggle = getByRole("switch", {
      name: "Enable link previews for this room"
    });
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    expect(toggle.hasAttribute("disabled")).toBe(false);
    expect(
      getByText("Encrypted-room previews can reveal URLs to the homeserver and destination site.")
    ).toBeDefined();
  });

  test("checks the toggle for encrypted rooms when the encrypted global default is on", () => {
    const { getByRole } = render(
      <RoomInfoPanel
        room={{ ...baseRoom, is_encrypted: true }}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        appSettings={{
          ...baseAppSettings,
          values: {
            ...baseAppSettings.values,
            display: {
              ...baseAppSettings.values.display,
              encrypted_url_previews_enabled: true
            }
          }
        }}
        linkPreviewSettings={baseLinkPreviewSettings}
        onSetRoomUrlPreviewOverride={() => undefined}
      />
    );

    const toggle = getByRole("switch", {
      name: "Enable link previews for this room"
    });
    expect(toggle.getAttribute("aria-checked")).toBe("true");
  });

  test("dispatches a per-room override when the toggle is clicked", () => {
    const onSetRoomUrlPreviewOverride = vi.fn();
    const { getByRole } = render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        appSettings={baseAppSettings}
        linkPreviewSettings={baseLinkPreviewSettings}
        onSetRoomUrlPreviewOverride={onSetRoomUrlPreviewOverride}
      />
    );

    const toggle = getByRole("switch", {
      name: "Enable link previews for this room"
    });
    fireEvent.click(toggle);

    expect(onSetRoomUrlPreviewOverride).toHaveBeenCalledTimes(1);
    expect(onSetRoomUrlPreviewOverride).toHaveBeenCalledWith(
      "!room-alpha:example.invalid",
      false
    );
  });
});
