import type { MessageId } from "../i18n/messages";
import type { RoomTags } from "./types";

export type ContextMenuKind = "message" | "room" | "space" | "account";

export type ContextMenuActionId =
  | "openThread"
  | "editMessage"
  | "redactMessage"
  | "selectRoom"
  | "openRoomInfo"
  | "searchInRoom"
  | "setRoomFavourite"
  | "removeRoomFavourite"
  | "setRoomLowPriority"
  | "removeRoomLowPriority"
  | "markRoomAsRead"
  | "markRoomAsUnread"
  | "selectSpace"
  | "openSpaceInfo"
  | "openUserSettings"
  | "openKeyboardSettings"
  | "switchAccount";

export interface ContextMenuItem {
  id: ContextMenuActionId;
  labelMessageId: MessageId;
  destructive?: boolean;
}

export type ContextMenuRequest =
  | {
      kind: "message";
      canManage: boolean;
      hasThread: boolean;
    }
  | {
      kind: "room";
      tags?: RoomTags;
    }
  | {
      kind: "space";
    }
  | {
      kind: "account";
    };

export function contextMenuItems(request: ContextMenuRequest): ContextMenuItem[] {
  switch (request.kind) {
    case "message": {
      const items: ContextMenuItem[] = [];
      if (request.hasThread) {
        items.push({ id: "openThread", labelMessageId: "context.openThread" });
      }
      if (request.canManage) {
        items.push({ id: "editMessage", labelMessageId: "context.editMessage" });
        items.push({
          id: "redactMessage",
          labelMessageId: "context.redactMessage",
          destructive: true
        });
      }
      return items;
    }
    case "room":
      return [
        { id: "selectRoom", labelMessageId: "context.selectRoom" },
        { id: "openRoomInfo", labelMessageId: "context.openRoomInfo" },
        { id: "searchInRoom", labelMessageId: "context.searchInRoom" },
        request.tags?.favourite
          ? { id: "removeRoomFavourite", labelMessageId: "context.removeFromFavourites" }
          : { id: "setRoomFavourite", labelMessageId: "context.addToFavourites" },
        request.tags?.low_priority
          ? { id: "removeRoomLowPriority", labelMessageId: "context.removeFromLowPriority" }
          : { id: "setRoomLowPriority", labelMessageId: "context.addToLowPriority" },
        { id: "markRoomAsRead", labelMessageId: "room.markAsRead" },
        { id: "markRoomAsUnread", labelMessageId: "room.markAsUnread" }
      ];
    case "space":
      return [
        { id: "selectSpace", labelMessageId: "context.selectSpace" },
        { id: "openSpaceInfo", labelMessageId: "context.openSpaceInfo" }
      ];
    case "account":
      return [
        { id: "openUserSettings", labelMessageId: "context.openUserSettings" },
        { id: "openKeyboardSettings", labelMessageId: "context.openKeyboardSettings" },
        { id: "switchAccount", labelMessageId: "context.switchAccount" }
      ];
  }
}
