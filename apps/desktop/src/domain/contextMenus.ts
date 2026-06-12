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
  label: string;
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
        items.push({ id: "openThread", label: "Reply in thread" });
      }
      if (request.canManage) {
        items.push({ id: "editMessage", label: "Edit" });
        items.push({ id: "redactMessage", label: "Redact", destructive: true });
      }
      return items;
    }
    case "room":
      return [
        { id: "selectRoom", label: "Open" },
        { id: "openRoomInfo", label: "Room info" },
        { id: "searchInRoom", label: "Search in room" }
      ];
    case "space":
      return [
        { id: "selectSpace", label: "Open Space" },
        { id: "openSpaceInfo", label: "Space info" }
      ];
    case "account":
      return [
        { id: "openUserSettings", label: "User settings" },
        { id: "openKeyboardSettings", label: "Keyboard shortcuts" },
        { id: "switchAccount", label: "Switch account" }
      ];
  }
}
