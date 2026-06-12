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
    expect(contextMenuItems({ kind: "room" }).map((item) => item.id)).toEqual([
      "selectRoom",
      "openRoomInfo",
      "searchInRoom"
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
});
