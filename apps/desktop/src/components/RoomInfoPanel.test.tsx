// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";

import { RoomInfoPanel } from "./RoomInfoPanel";
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
  dm_space_ids: [],
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
      auto_load_older_messages: true,
      thread_root_order: { kind: "latestReply" }
    },
    search_crawler: {
      speed: "standard" as const,
      include_media_captions: true,
      include_filenames: true
    },
    thread_list_order: { kind: "latestReply" },
    room_list_sort: { kind: "activity" }
  },
  persistence: { kind: "idle" }
};

const baseLinkPreviewSettings: LinkPreviewSettingsState = {
  room_overrides: {}
};

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("RoomInfoPanel", () => {
  test("renders room identity and simplified info entries", () => {
    render(
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

    expect(screen.getByText("Alpha Room")).toBeTruthy();
    expect(screen.getByText("!room-alpha:example.invalid")).toBeTruthy();
    expect(screen.getByRole("button", { name: "People" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Files" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Notifications" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Room settings" })).toBeTruthy();
    expect(screen.getByText("Synthetic Workspace")).toBeTruthy();
  });

  test("labels direct messages distinctly from rooms", () => {
    render(
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
          dm_space_ids: [],
          unread_count: 0
        }}
        roomNotificationSettings={idleSettings}
        spaces={[]}
      />
    );

    expect(screen.getByText("Direct message")).toBeTruthy();
    expect(screen.queryAllByText("No Spaces").length).toBeGreaterThanOrEqual(1);
  });

  test("renders room titles from the Rust-projected display label", () => {
    render(
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
          dm_space_ids: [],
          unread_count: 0
        }}
        roomNotificationSettings={idleSettings}
        spaces={[]}
      />
    );

    expect(screen.getByText("Alice Local")).toBeTruthy();
    expect(screen.queryByText("Alice Upstream")).toBeNull();
  });

  test("opens the People panel when the People entry is clicked", () => {
    const onOpenPeople = vi.fn();
    render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        onOpenPeople={onOpenPeople}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "People" }));
    expect(onOpenPeople).toHaveBeenCalledTimes(1);
  });

  test("does not render a dense member list in the room info panel", () => {
    render(
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

    expect(screen.queryByText("Local Remark")).toBeNull();
    expect(screen.queryByRole("button", { name: /Message/ })).toBeNull();
  });

  test("renders notification mode options", () => {
    render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        onSetRoomNotificationMode={() => undefined}
      />
    );

    expect(screen.getByText("All messages")).toBeTruthy();
    expect(screen.getByText("Mentions only")).toBeTruthy();
    expect(screen.getByText("Mute")).toBeTruthy();
  });

  test("selects the current notification mode", () => {
    const { container } = render(
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

    const select = container.querySelector("select");
    expect(select).toBeTruthy();
    expect(select?.value).toBe("mentions");
  });

  test("disables the notification select while a mode change is pending", () => {
    const { container } = render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={pendingSettings}
        spaces={[]}
        onSetRoomNotificationMode={() => undefined}
      />
    );

    const select = container.querySelector("select");
    expect(select).toBeTruthy();
    expect(select?.hasAttribute("disabled")).toBe(true);
    expect(select?.value).toBe("mute");
  });

  test("disables the notification select when no handler is provided", () => {
    const { container } = render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
      />
    );

    const select = container.querySelector("select");
    expect(select).toBeTruthy();
    expect(select?.hasAttribute("disabled")).toBe(true);
  });
});

describe("RoomInfoPanel URL previews", () => {
  test("renders the URL-preview section when settings and handler are supplied", () => {
    render(
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
      screen.getByRole("switch", { name: "Enable link previews for this room" })
    ).toBeTruthy();
  });

  test("checks the toggle for an unencrypted room that falls back to the global setting", () => {
    render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        appSettings={baseAppSettings}
        linkPreviewSettings={baseLinkPreviewSettings}
        onSetRoomUrlPreviewOverride={() => undefined}
      />
    );

    const toggle = screen.getByRole("switch", {
      name: "Enable link previews for this room"
    });
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    expect(toggle.hasAttribute("disabled")).toBe(false);
  });

  test("unchecks but keeps the toggle enabled for encrypted rooms by default", () => {
    render(
      <RoomInfoPanel
        room={{ ...baseRoom, is_encrypted: true }}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        appSettings={baseAppSettings}
        linkPreviewSettings={baseLinkPreviewSettings}
        onSetRoomUrlPreviewOverride={() => undefined}
      />
    );

    const toggle = screen.getByRole("switch", {
      name: "Enable link previews for this room"
    });
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    expect(toggle.hasAttribute("disabled")).toBe(false);
    expect(
      screen.getByText(
        "Encrypted-room previews can reveal URLs to the homeserver and destination site."
      )
    ).toBeTruthy();
  });

  test("checks the toggle for encrypted rooms when the encrypted global default is on", () => {
    render(
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

    const toggle = screen.getByRole("switch", {
      name: "Enable link previews for this room"
    });
    expect(toggle.getAttribute("aria-checked")).toBe("true");
  });

  test("dispatches a per-room override when the toggle is clicked", () => {
    const onSetRoomUrlPreviewOverride = vi.fn();
    render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        appSettings={baseAppSettings}
        linkPreviewSettings={baseLinkPreviewSettings}
        onSetRoomUrlPreviewOverride={onSetRoomUrlPreviewOverride}
      />
    );

    const toggle = screen.getByRole("switch", {
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
