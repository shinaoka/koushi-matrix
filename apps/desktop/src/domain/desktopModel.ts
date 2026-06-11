import type {
  DesktopSnapshot,
  RoomListItem,
  RoomSummary,
  SearchScopeKind,
  SpaceSummary,
  VisibleRooms
} from "./types";

export function visibleRooms(snapshot: DesktopSnapshot): VisibleRooms {
  return {
    spaceRooms: snapshot.sidebar.space_rooms,
    globalDms: snapshot.sidebar.global_dms
  };
}

export function composeSidebar(
  activeSpaceId: string | null,
  spaces: SpaceSummary[],
  rooms: RoomSummary[]
): DesktopSnapshot["sidebar"] {
  const roomExists = (room: RoomSummary | undefined): room is RoomSummary =>
    room !== undefined && !room.is_dm;

  const spaceRooms = activeSpaceId
    ? spaces
        .find((space) => space.space_id === activeSpaceId)
        ?.child_room_ids.map((roomId) => rooms.find((room) => room.room_id === roomId))
        .filter(roomExists)
        .map(roomListItem) ?? []
    : rooms
        .filter((room) => !room.is_dm && room.parent_space_ids.length === 0)
        .map(roomListItem);

  const globalDms = rooms.filter((room) => room.is_dm).map(roomListItem);

  return {
    active_space_id: activeSpaceId,
    space_rail: spaces.map((space) => ({
      space_id: space.space_id,
      display_name: space.display_name,
      unread_count: space.child_room_ids
        .map((roomId) => rooms.find((room) => room.room_id === roomId))
        .filter(roomExists)
        .reduce((sum, room) => sum + room.unread_count, 0),
      is_active: space.space_id === activeSpaceId
    })),
    space_rooms: spaceRooms,
    global_dms: globalDms,
    space_unread_count: spaceRooms.reduce((sum, room) => sum + room.unread_count, 0),
    dm_unread_count: globalDms.reduce((sum, room) => sum + room.unread_count, 0)
  };
}

export function roomIsInScope(
  roomId: string,
  scope: SearchScopeKind,
  snapshot: DesktopSnapshot
): boolean {
  switch (scope) {
    case "currentRoom":
      return snapshot.state.navigation.active_room_id === roomId;
    case "currentSpace": {
      const activeSpace = snapshot.state.spaces.find(
        (space) => space.space_id === snapshot.state.navigation.active_space_id
      );
      return Boolean(activeSpace?.child_room_ids.includes(roomId));
    }
    case "dms":
      return snapshot.state.rooms.some((room) => room.room_id === roomId && room.is_dm);
    case "allRooms":
      return true;
  }
}

export function textRangeUtf16(haystack: string, needle: string) {
  const start = haystack.indexOf(needle);
  if (start < 0 || needle.length === 0) {
    return null;
  }
  const end = start + needle.length;
  return {
    start_utf16: utf16Length(haystack.slice(0, start)),
    end_utf16: utf16Length(haystack.slice(0, end))
  };
}

function roomListItem(room: RoomSummary): RoomListItem {
  return {
    room_id: room.room_id,
    display_name: room.display_name,
    unread_count: room.unread_count
  };
}

function utf16Length(value: string): number {
  return [...value].reduce((sum, char) => sum + char.length, 0);
}
