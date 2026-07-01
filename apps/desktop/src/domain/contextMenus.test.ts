import { describe, expect, test } from "vitest";

import { contextMenuItems } from "./contextMenus";

describe("context menu registry", () => {
  test("message menu exposes thread actions and owner-only edit/redact", () => {
    const ownerItems = contextMenuItems({
      kind: "message",
      canManage: true,
      canReply: true,
      hasThread: true,
      senderUserId: "@owner:example.invalid",
      currentUserId: "@owner:example.invalid",
      roomId: "!room:example.invalid",
      eventId: "$event:example.invalid",
      isIgnored: false
    });
    const guestItems = contextMenuItems({
      kind: "message",
      canManage: false,
      canReply: true,
      hasThread: true,
      senderUserId: "@other:example.invalid",
      currentUserId: "@me:example.invalid",
      roomId: "!room:example.invalid",
      eventId: "$event:example.invalid",
      isIgnored: false
    });

    expect(ownerItems.map((item) => item.id)).toEqual([
      "replyToMessage",
      "openThread",
      "editMessage",
      "redactMessage"
    ]);
    expect(ownerItems.find((item) => item.id === "redactMessage")).toMatchObject({
      destructive: true
    });
    expect(guestItems.map((item) => item.id)).toEqual([
      "replyToMessage",
      "openThread",
      "ignoreUser",
      "reportUser",
      "reportContent"
    ]);
  });

  test("message menu omits normal reply when the row cannot be replied to", () => {
    const items = contextMenuItems({
      kind: "message",
      canManage: false,
      canReply: false,
      hasThread: true,
      senderUserId: "@other:example.invalid",
      currentUserId: "@me:example.invalid",
      roomId: "!room:example.invalid",
      eventId: "$event:example.invalid",
      isIgnored: false
    }).map((item) => item.id);

    expect(items).not.toContain("replyToMessage");
    expect(items).toContain("openThread");
  });

  test("message menu shows unignore when sender is already ignored", () => {
    const items = contextMenuItems({
      kind: "message",
      canManage: false,
      canReply: true,
      hasThread: false,
      senderUserId: "@other:example.invalid",
      currentUserId: "@me:example.invalid",
      roomId: "!room:example.invalid",
      eventId: "$event:example.invalid",
      isIgnored: true
    }).map((item) => item.id);

    expect(items).toContain("unignoreUser");
    expect(items).not.toContain("ignoreUser");
  });

  test("room, space, and account menus only include implemented shell actions", () => {
    expect(
      contextMenuItems({
        kind: "room",
        roomId: "!room:example.invalid",
        tags: { favourite: null, low_priority: null }
      }).map((item) => item.id)
    ).toEqual([
      "selectRoom",
      "openRoomInfo",
      "searchInRoom",
      "reportRoom",
      "setRoomFavourite",
      "setRoomLowPriority",
      "markRoomAsRead",
      "markRoomAsUnread"
    ]);
    expect(contextMenuItems({ kind: "space" }).map((item) => item.id)).toEqual([
      "selectSpace",
      "openSpaceInfo",
      "leaveSpace"
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
        roomId: "!room:example.invalid",
        tags: { favourite: { order: "0.1" }, low_priority: null }
      }).map((item) => item.id)
    ).toContain("removeRoomFavourite");
    expect(
      contextMenuItems({
        kind: "room",
        roomId: "!room:example.invalid",
        tags: { favourite: null, low_priority: { order: "0.9" } }
      }).map((item) => item.id)
    ).toContain("removeRoomLowPriority");
  });
});
