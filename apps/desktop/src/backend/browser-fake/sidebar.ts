import { composeSidebar } from "../../domain/desktopModel";
import type {
  RoomNotificationSettings,
  RoomSummary,
  SpaceSummary
} from "../../domain/types";
import { computeBrowserRoomListProjection } from "../roomListProjection";

export function emptySidebar() {
  return {
    active_space_id: null,
    account_home: {
      display_name: "Home",
      unread_count: 0,
      highlight_count: 0,
      invite_count: 0,
      attention_count: 0,
      is_active: true
    },
    space_rail: [],
    space_rooms: [],
    not_joined_space_rooms: [],
    global_dms: [],
    space_unread_count: 0,
    dm_unread_count: 0,
    space_highlight_count: 0,
    dm_highlight_count: 0
  };
}

export function composeBrowserFakeSidebar(
  activeSpaceId: string | null,
  sourceSpaces: SpaceSummary[],
  sourceRooms: RoomSummary[],
  roomNotificationSettings: Record<string, RoomNotificationSettings> = {},
  pendingInviteCount = 0
) {
  const sidebar = composeSidebar(
    activeSpaceId,
    sourceSpaces,
    sourceRooms,
    roomNotificationSettings,
    pendingInviteCount
  );
  const projection = computeBrowserRoomListProjection(
    { kind: "people" },
    { kind: "activity" },
    activeSpaceId,
    sourceSpaces,
    sourceRooms,
    []
  );
  const positionByRoomId = new Map(
    (projection.items ?? []).map((item, index) => [item.room_id, index])
  );
  sidebar.global_dms.sort(
    (left, right) =>
      (positionByRoomId.get(left.room_id) ?? Number.MAX_SAFE_INTEGER) -
      (positionByRoomId.get(right.room_id) ?? Number.MAX_SAFE_INTEGER)
  );
  return sidebar;
}
