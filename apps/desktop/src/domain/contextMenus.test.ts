import { describe, expect, test } from "vitest";

import { contextMenuItems } from "./contextMenus";

describe("context menu registry", () => {
  test("message menu exposes thread actions and owner-only edit/redact", () => {
    const ownerItems = contextMenuItems({
      kind: "message",
      canManage: true,
      hasThread: true
    });
    const guestItems = contextMenuItems({
      kind: "message",
      canManage: false,
      hasThread: true
    });

    expect(ownerItems.map((item) => item.id)).toEqual([
      "openThread",
      "editMessage",
      "redactMessage"
    ]);
    expect(ownerItems.find((item) => item.id === "redactMessage")).toMatchObject({
      destructive: true
    });
    expect(guestItems.map((item) => item.id)).toEqual(["openThread"]);
  });

  test("room, space, and account menus only include implemented shell actions", () => {
    expect(
      contextMenuItems({
        kind: "room",
        tags: { favourite: null, low_priority: null }
      }).map((item) => item.id)
    ).toEqual([
      "selectRoom",
      "openRoomInfo",
      "searchInRoom",
      "setRoomFavourite",
      "setRoomLowPriority",
      "markRoomAsRead",
      "markRoomAsUnread"
    ]);
    expect(contextMenuItems({ kind: "space" }).map((item) => item.id)).toEqual([
      "selectSpace",
      "openSpaceInfo"
    ]);
    expect(contextMenuItems({ kind: "account" }).map((item) => item.id)).toEqual([
      "openUserSettings",
      "openKeyboardSettings",
      "switchAccount"
    ]);
  });

  test("room tag menu actions reflect Rust-owned tag state", () => {
    expect(
      contextMenuItems({
        kind: "room",
        tags: { favourite: { order: "0.1" }, low_priority: null }
      }).map((item) => item.id)
    ).toContain("removeRoomFavourite");
    expect(
      contextMenuItems({
        kind: "room",
        tags: { favourite: null, low_priority: { order: "0.9" } }
      }).map((item) => item.id)
    ).toContain("removeRoomLowPriority");
  });
});
