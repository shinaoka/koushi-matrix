import { describe, expect, test } from "vitest";

import {
  effectiveRightPanelModeForSnapshot,
  rightPanelIntentForContextMenuAction,
  rightPanelModeForSearchQuery
} from "./rightPanel";
import type { DesktopSnapshot } from "./types";

describe("right panel context menu routing", () => {
  test("routes room and Space menu actions through selection plus panel mode", () => {
    expect(
      rightPanelIntentForContextMenuAction(
        { kind: "room", roomId: "!room-a:example.invalid" },
        "openRoomInfo"
      )
    ).toEqual({ mode: "roomInfo", selectRoomId: "!room-a:example.invalid" });

    expect(
      rightPanelIntentForContextMenuAction(
        { kind: "space", spaceId: "!space-a:example.invalid" },
        "openSpaceInfo"
      )
    ).toEqual({ mode: "spaceInfo", selectSpaceId: "!space-a:example.invalid" });
  });

  test("routes account menu actions to user and keyboard settings panels", () => {
    expect(
      rightPanelIntentForContextMenuAction({ kind: "account" }, "openUserSettings")
    ).toEqual({ mode: "userSettings" });
    expect(
      rightPanelIntentForContextMenuAction({ kind: "account" }, "openKeyboardSettings")
    ).toEqual({ mode: "keyboardSettings" });
    expect(
      rightPanelIntentForContextMenuAction({ kind: "account" }, "switchAccount")
    ).toEqual({ mode: "userSettings" });
  });

  test("does not invent panel switches for open and search-only actions", () => {
    expect(
      rightPanelIntentForContextMenuAction(
        { kind: "space", spaceId: "!space-a:example.invalid" },
        "selectSpace"
      )
    ).toEqual({ selectSpaceId: "!space-a:example.invalid" });
    expect(
      rightPanelIntentForContextMenuAction(
        { kind: "room", roomId: "!room-a:example.invalid" },
        "searchInRoom"
      )
    ).toEqual({ selectRoomId: "!room-a:example.invalid", focusSearch: true });
    expect(
      rightPanelIntentForContextMenuAction(
        { kind: "message", roomId: "!room-a:example.invalid", eventId: "$event-a" },
        "openRoomInfo"
      )
    ).toBeNull();
  });

  test("opens a search context panel only for non-empty searches", () => {
    expect(rightPanelModeForSearchQuery(" Alpha ")).toBe("search");
    expect(rightPanelModeForSearchQuery("   ")).toBeNull();
  });

  test("forces recovery mode only while recovery is required", () => {
    const snapshot = snapshotForPanelMode("needsRecovery", false);

    expect(effectiveRightPanelModeForSnapshot("roomInfo", snapshot)).toBe("recovery");
    expect(
      effectiveRightPanelModeForSnapshot(
        "search",
        snapshotForPanelMode("recovering", true)
      )
    ).toBe("recovery");
    expect(effectiveRightPanelModeForSnapshot("roomInfo", snapshotForPanelMode("ready", false))).toBe(
      "roomInfo"
    );
  });

  test("closes missing thread mode without affecting other ready panels", () => {
    expect(effectiveRightPanelModeForSnapshot("thread", snapshotForPanelMode("ready", false))).toBe(
      "closed"
    );
    expect(effectiveRightPanelModeForSnapshot("thread", snapshotForPanelMode("ready", true))).toBe(
      "thread"
    );
    expect(effectiveRightPanelModeForSnapshot("search", snapshotForPanelMode("ready", false))).toBe(
      "search"
    );
  });
});

function snapshotForPanelMode(
  sessionKind: DesktopSnapshot["state"]["session"]["kind"],
  hasThread: boolean
): Pick<DesktopSnapshot, "state" | "thread"> {
  return {
    state: {
      session: { kind: sessionKind },
      auth: { kind: "unknown" },
      settings: {
        values: {
          locale: { language_tag: null, text_direction: "auto" },
          appearance: { theme: "system" },
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
          }
        },
        persistence: { kind: "idle" }
      },
      link_preview_settings: { room_overrides: {} },
      locale_profile: {
        lang: "en",
        dir: "ltr",
        catalog_locale: "en",
        pseudo_locale: "none",
        platform: "linux",
        modifier_labels: { primary: "Ctrl" }
      },
      typography_profile: {
        font: "system",
        emoji: "system",
        platform: "linux",
        font_asset: "systemFallback",
        emoji_asset: "systemFallback"
      },
      profile: {
        own: { display_name: null, avatar: null },
        users: {},
        local_aliases: {},
        local_alias_update: { kind: "idle" },
        ignored_user_ids: [],
        ignored_user_update: { kind: "idle" },
        update: { kind: "idle" }
      },
      sync: "stopped",
      sync_mode: { kind: "unsupported" },
      navigation: { active_space_id: null, active_room_id: null },
      spaces: [],
      rooms: [],
      invites: [],
      room_list: { active_filter: { kind: "rooms" }, sort: { kind: "activity" }, items: [] },
      room_interactions: {},
      room_notification_settings: {},
      device_sessions: { kind: "idle" },
      account_management: { kind: "idle" },
      account_management_capabilities: { change_password: { kind: "unknown" } },
      soft_logout_reauth: { kind: "idle" },
      qr_login: { kind: "idle" },
      directory: { query: { kind: "closed" }, join: { kind: "idle" } },
      room_management: { selected_room_id: null, settings: null, operation: { kind: "idle" } },
      activity: { kind: "closed" },
      timeline: {
        room_id: null,
        is_subscribed: false,
        is_paginating_backwards: false,
        composer: { pending_transaction_id: null, draft: "", mode: "Plain" },
        scheduled_send_capability: "unknown",
        scheduled_sends: [],
        staged_uploads: [],
        media_gallery: []
      },
      thread: hasThread
        ? { kind: "open", room_id: "!room:example", root_event_id: "$event" }
        : { kind: "closed" },
      thread_attention: hasThread
        ? {
            kind: "tracking",
            room_id: "!room:example",
            root_event_id: "$event",
            notification_count: 0,
            highlight_count: 0,
            live_event_marker_count: 0
          }
        : { kind: "closed" },
      focused_context: { kind: "closed" },
      search: { kind: "closed" },
      files_view: { kind: "closed" },
      threads_list: { kind: "closed" },
      errors: [],
      basic_operation: { kind: "idle" },
      live_signals: { rooms: {}, presence: {} },
      e2ee_trust: {
        verification: { kind: "idle" },
        cross_signing: { kind: "unknown" },
        key_backup: { kind: "unknown" },
        identity_reset: { kind: "idle" },
        key_management: {
          room_key_export: { kind: "idle" },
          room_key_import: { kind: "idle" },
          secure_backup_setup: { kind: "idle" },
          passphrase_change: { kind: "idle" }
        },
        devices: []
      },
      local_encryption: { kind: "unknown" },
      native_attention: {
        summary: {
          unread_count: 0,
          highlight_count: 0,
          badge_count: 0,
          candidate: null,
          capabilities: {
            notifications: "unknown",
            badge: "unknown",
            overlay_icon: "unknown",
            sound: "unknown",
            tray: "unknown",
            activation: "unknown"
          }
        },
        dispatch: { kind: "idle" }
      },
      cjk_text_policy: {
        japanese_catalog: {
          catalog_locale: "en",
          complete: true,
          missing_message_ids: []
        },
        normalization: {
          form: "nfkc",
          width_fold: true,
          kana_fold: true
        },
        collation: {
          locale: "ja",
          numeric: true,
          case_first: null
        }
      }
    },
    // Production always sends the legacy top-level thread as null; the open/closed
    // decision must come from state.thread, never this placeholder.
    thread: null
  };
}
