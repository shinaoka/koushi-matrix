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
      sync: "stopped",
      navigation: { active_space_id: null, active_room_id: null },
      spaces: [],
      rooms: [],
      timeline: {
        room_id: null,
        is_subscribed: false,
        is_paginating_backwards: false,
        composer: { pending_transaction_id: null, draft: "", mode: "Plain" }
      },
      thread: hasThread
        ? { kind: "open", room_id: "!room:example", root_event_id: "$event" }
        : { kind: "closed" },
      search: { kind: "closed" },
      errors: [],
      basic_operation: { kind: "idle" }
    },
    // Production always sends the legacy top-level thread as null; the open/closed
    // decision must come from state.thread, never this placeholder.
    thread: null
  };
}
