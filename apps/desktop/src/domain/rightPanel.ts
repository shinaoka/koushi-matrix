import type { ContextMenuActionId } from "./contextMenus";
import type { DesktopSnapshot } from "./types";

export type RightPanelMode =
  | "closed"
  | "thread"
  | "search"
  | "recovery"
  | "keyboardSettings"
  | "userSettings"
  | "roomInfo"
  | "spaceInfo";

export type RightPanelContextMenuTarget =
  | { kind: "message"; roomId: string; eventId: string }
  | { kind: "room"; roomId: string }
  | { kind: "space"; spaceId: string }
  | { kind: "account" };

export interface RightPanelIntent {
  mode?: RightPanelMode;
  selectRoomId?: string;
  selectSpaceId?: string;
  focusSearch?: boolean;
}

export function rightPanelIntentForContextMenuAction(
  target: RightPanelContextMenuTarget,
  actionId: ContextMenuActionId
): RightPanelIntent | null {
  if (target.kind === "room") {
    switch (actionId) {
      case "selectRoom":
        return { selectRoomId: target.roomId };
      case "openRoomInfo":
        return { mode: "roomInfo", selectRoomId: target.roomId };
      case "searchInRoom":
        return { selectRoomId: target.roomId, focusSearch: true };
      default:
        return null;
    }
  }

  if (target.kind === "space") {
    switch (actionId) {
      case "selectSpace":
        return { selectSpaceId: target.spaceId };
      case "openSpaceInfo":
        return { mode: "spaceInfo", selectSpaceId: target.spaceId };
      default:
        return null;
    }
  }

  if (target.kind === "account") {
    switch (actionId) {
      case "openUserSettings":
      case "switchAccount":
        return { mode: "userSettings" };
      case "openKeyboardSettings":
        return { mode: "keyboardSettings" };
      default:
        return null;
    }
  }

  return null;
}

export function rightPanelModeForSearchQuery(query: string): RightPanelMode | null {
  return query.trim() ? "search" : null;
}

export function effectiveRightPanelModeForSnapshot(
  requestedMode: RightPanelMode,
  snapshot: Pick<DesktopSnapshot, "state" | "thread">
): RightPanelMode {
  const sessionKind = snapshot.state.session.kind;
  if (sessionKind === "needsRecovery" || sessionKind === "recovering") {
    return "recovery";
  }

  if (requestedMode === "thread" && !snapshot.thread) {
    return "closed";
  }

  return requestedMode;
}
