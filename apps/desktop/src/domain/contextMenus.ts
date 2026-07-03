import type { MessageId } from "../i18n/messages";
import type { RoomTags } from "./types";

export type ContextMenuKind = "message" | "room" | "space" | "account";

export type ContextMenuActionId =
  | "openThread"
  | "editMessage"
  | "redactMessage"
  | "ignoreUser"
  | "unignoreUser"
  | "reportUser"
  | "reportContent"
  | "selectRoom"
  | "openUserInfo"
  | "openRoomInfo"
  | "searchInRoom"
  | "reportRoom"
  | "setRoomFavourite"
  | "removeRoomFavourite"
  | "setRoomLowPriority"
  | "removeRoomLowPriority"
  | "markRoomAsRead"
  | "markRoomAsUnread"
  | "selectSpace"
  | "openSpaceInfo"
  | "leaveSpace"
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
      senderUserId: string;
      currentUserId: string;
      roomId: string;
      eventId: string;
      isIgnored: boolean;
    }
  | {
      kind: "room";
      roomId: string;
      tags?: RoomTags;
      dmUserIds?: string[];
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
      if (request.senderUserId !== request.currentUserId) {
        if (request.isIgnored) {
          items.push({ id: "unignoreUser", labelMessageId: "context.unignoreUser" });
        } else {
          items.push({ id: "ignoreUser", labelMessageId: "context.ignoreUser" });
        }
        items.push({
          id: "reportUser",
          labelMessageId: "context.reportUser",
          destructive: true
        });
        items.push({
          id: "reportContent",
          labelMessageId: "context.reportContent",
          destructive: true
        });
      }
      return items;
    }
    case "room": {
      const userInfoItem =
        request.dmUserIds?.length === 1
          ? [{ id: "openUserInfo" as const, labelMessageId: "context.openUserInfo" as const }]
          : [];
      return [
        { id: "selectRoom", labelMessageId: "context.selectRoom" },
        ...userInfoItem,
        { id: "openRoomInfo", labelMessageId: "context.openRoomInfo" },
        { id: "searchInRoom", labelMessageId: "context.searchInRoom" },
        {
          id: "reportRoom",
          labelMessageId: "context.reportRoom",
          destructive: true
        },
        request.tags?.favourite
          ? { id: "removeRoomFavourite", labelMessageId: "context.removeFromFavourites" }
          : { id: "setRoomFavourite", labelMessageId: "context.addToFavourites" },
        request.tags?.low_priority
          ? { id: "removeRoomLowPriority", labelMessageId: "context.removeFromLowPriority" }
          : { id: "setRoomLowPriority", labelMessageId: "context.addToLowPriority" },
        { id: "markRoomAsRead", labelMessageId: "room.markAsRead" },
        { id: "markRoomAsUnread", labelMessageId: "room.markAsUnread" }
      ];
    }
    case "space":
      return [
        { id: "selectSpace", labelMessageId: "context.selectSpace" },
        { id: "openSpaceInfo", labelMessageId: "context.openSpaceInfo" },
        { id: "leaveSpace", labelMessageId: "context.leaveSpace", destructive: true }
      ];
    case "account":
      return [
        { id: "openUserSettings", labelMessageId: "context.openUserSettings" },
        { id: "openKeyboardSettings", labelMessageId: "context.openKeyboardSettings" },
        { id: "switchAccount", labelMessageId: "context.switchAccount" }
      ];
  }
}
