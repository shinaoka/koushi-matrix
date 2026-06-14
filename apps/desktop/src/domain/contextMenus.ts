import type { MessageId } from "../i18n/messages";

export type ContextMenuKind = "message" | "room" | "space" | "account";

export type ContextMenuActionId =
  | "openThread"
  | "editMessage"
  | "redactMessage"
  | "selectRoom"
  | "openRoomInfo"
  | "searchInRoom"
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
        { id: "searchInRoom", labelMessageId: "context.searchInRoom" }
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
