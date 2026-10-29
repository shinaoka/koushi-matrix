import type { ContextMenuActionId } from "./contextMenus";
import type {
  DesktopSnapshot,
  EventNavigationSource,
  RoomSummary,
  SpaceSummary
} from "./types";

export type RightPanelMode =
  | "closed"
  | "thread"
  | "focusedContext"
  | "search"
  | "recovery"
  | "keyboardSettings"
  | "userSettings"
  | "roomInfo"
  | "spaceInfo"
  | "files"
  | "threads"
  | "scheduledMessages"
  | "pinned"
  | "people"
  | "profile";

export type RightPanelContextMenuTarget =
  | { kind: "message"; roomId: string; eventId: string }
  | { kind: "room"; roomId: string }
  | { kind: "space"; spaceId: string }
  | { kind: "spaceMember"; spaceId: string; userId: string; generation: number }
  | { kind: "account" };

export interface RightPanelIntent {
  mode?: RightPanelMode;
  selectRoomId?: string;
  selectSpaceId?: string;
  focusSearch?: boolean;
}

export type PeoplePanelScope =
  | { kind: "room"; roomId: string }
  | { kind: "space"; spaceId: string };

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
      case "openSpaceInfo":
        return { mode: "spaceInfo", selectSpaceId: target.spaceId };
      default:
        return null;
    }
  }

  if (target.kind === "account") {
    switch (actionId) {
      case "openUserSettings":
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

// The panel shown once a Rust event navigation anchors the main pane. Search and
// Pinned keep the list the jump came from; a reply quote jumps within the
// conversation, so it keeps the current panel and only drops a focused context
// that the anchored main pane replaces.
export function rightPanelModeForEventNavigation(
  source: EventNavigationSource,
  current: RightPanelMode
): RightPanelMode {
  switch (source) {
    case "activity":
    case "notification":
      return "closed";
    case "search":
      return "search";
    case "pinned":
      return "pinned";
    case "replyQuote":
      return current === "focusedContext" ? "closed" : current;
    default: {
      // A source the renderer does not know yet must fail the typecheck, and at
      // runtime it leaves the panel alone rather than inventing a mode.
      const unhandled: never = source;
      void unhandled;
      return current;
    }
  }
}

export function roomOrSpaceForPeoplePanelScope(
  scope: PeoplePanelScope | null,
  activeRoom: RoomSummary | null,
  activeSpace: SpaceSummary | null,
  rooms: RoomSummary[],
  spaces: SpaceSummary[]
): RoomSummary | SpaceSummary | null {
  if (scope?.kind === "room") {
    return rooms.find((room) => room.room_id === scope.roomId) ?? null;
  }

  if (scope?.kind === "space") {
    return spaces.find((space) => space.space_id === scope.spaceId) ?? null;
  }

  return activeRoom ?? activeSpace;
}

export function effectiveRightPanelModeForSnapshot(
  requestedMode: RightPanelMode,
  snapshot: Pick<DesktopSnapshot, "state">
): RightPanelMode {
  const sessionKind = snapshot.state.domain.session.kind;
  if (sessionKind === "needsRecovery" || sessionKind === "recovering") {
    return "recovery";
  }

  if (
    requestedMode === "focusedContext" &&
    snapshot.state.ui.focused_context.kind === "closed"
  ) {
    return "closed";
  }

  // Thread open/closed is Rust-owned product state: read it from state.thread,
  // not the legacy top-level `thread` placeholder (always null in production).
  if (requestedMode === "thread" && snapshot.state.ui.thread.kind === "closed") {
    return "closed";
  }

  if (requestedMode === "files" && snapshot.state.ui.files_view.kind === "closed") {
    return "closed";
  }

  if (requestedMode === "threads" && snapshot.state.ui.threads_list.kind === "closed") {
    return "closed";
  }

  if (
    requestedMode === "scheduledMessages" &&
    snapshot.state.ui.scheduled_sends_list.kind === "closed"
  ) {
    return "closed";
  }

  return requestedMode;
}
