import type {
  DesktopSnapshot,
  InvitePreview,
  RoomListItem,
  RoomListProjection,
  RoomListSections,
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

export function roomListSections(
  roomList: RoomListProjection,
  activeSpaceId: string | null,
  spaces: SpaceSummary[],
  rooms: RoomSummary[],
  invites: InvitePreview[]
): RoomListSections {
  if (roomList.items === null) {
    const sidebar = composeSidebar(activeSpaceId, spaces, rooms);
    return {
      favourites: sidebar.space_rooms.filter((room) => room.tags.favourite !== null),
      invites: [],
      rooms: sidebar.space_rooms.filter(
        (room) => room.tags.favourite === null && room.tags.low_priority === null
      ),
      people: sidebar.global_dms,
      lowPriority: sidebar.space_rooms.filter((room) => room.tags.low_priority !== null)
    };
  }

  const roomById = new Map(rooms.map((room) => [room.room_id, room]));
  const inviteById = new Map(invites.map((invite) => [invite.room_id, invite]));

  const favourites: RoomListItem[] = [];
  const invitesSection: RoomListItem[] = [];
  const roomsSection: RoomListItem[] = [];
  const people: RoomListItem[] = [];
  const lowPriority: RoomListItem[] = [];

  for (const item of roomList.items) {
    const room = item.kind === "room" ? roomById.get(item.room_id) : undefined;
    const invite = item.kind === "invite" ? inviteById.get(item.room_id) : undefined;
    const source = room ?? invite;
    if (!source) {
      continue;
    }
    if (!room && !invite) {
      continue;
    }
    if (invite) {
      const listItem = inviteListItem(invite);
      if (invite.is_dm) {
        people.push(listItem);
      } else {
        invitesSection.push(listItem);
      }
      continue;
    }

    if (!room) {
      continue;
    }

    const listItem = roomListItem(room);
    if (room.is_dm) {
      people.push(listItem);
    } else if (room.tags.favourite !== null) {
      favourites.push(listItem);
    } else if (room.tags.low_priority !== null) {
      lowPriority.push(listItem);
    } else {
      roomsSection.push(listItem);
    }
  }

  return {
    favourites,
    invites: invitesSection,
    rooms: roomsSection,
    people,
    lowPriority
  };
}

function inviteListItem(invite: InvitePreview): RoomListItem {
  return {
    room_id: invite.room_id,
    display_name: invite.display_name,
    avatar: invite.avatar,
    tags: { favourite: null, low_priority: null },
    unread_count: 0,
    highlight_count: 0
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
    : rooms.filter((room) => !room.is_dm).map(roomListItem);

  const globalDms = rooms.filter((room) => room.is_dm).map(roomListItem);

  return {
    active_space_id: activeSpaceId,
    account_home: {
      display_name: "Home",
      unread_count: rooms
        .filter((room) => !room.is_dm)
        .reduce((sum, room) => sum + room.unread_count, 0),
      highlight_count: rooms
        .filter((room) => !room.is_dm)
        .reduce((sum, room) => sum + (room.highlight_count ?? 0), 0),
      is_active: activeSpaceId === null
    },
    space_rail: spaces.map((space) => ({
      space_id: space.space_id,
      display_name: space.display_name,
      avatar: space.avatar,
      unread_count: space.child_room_ids
        .map((roomId) => rooms.find((room) => room.room_id === roomId))
        .filter(roomExists)
        .reduce((sum, room) => sum + room.unread_count, 0),
      highlight_count: space.child_room_ids
        .map((roomId) => rooms.find((room) => room.room_id === roomId))
        .filter(roomExists)
        .reduce((sum, room) => sum + (room.highlight_count ?? 0), 0),
      is_active: space.space_id === activeSpaceId
    })),
    space_rooms: spaceRooms,
    global_dms: globalDms,
    space_unread_count: spaceRooms.reduce((sum, room) => sum + room.unread_count, 0),
    dm_unread_count: globalDms.reduce((sum, room) => sum + room.unread_count, 0),
    space_highlight_count: spaceRooms.reduce((sum, room) => sum + room.highlight_count, 0),
    dm_highlight_count: globalDms.reduce((sum, room) => sum + room.highlight_count, 0)
  };
}

export function roomIsInScope(
  roomId: string,
  scope: SearchScopeKind,
  snapshot: DesktopSnapshot
): boolean {
  switch (scope) {
    case "currentRoom":
      return snapshot.state.ui.navigation.active_room_id === roomId;
    case "currentSpace": {
      const activeSpace = snapshot.state.domain.spaces.find(
        (space) => space.space_id === snapshot.state.ui.navigation.active_space_id
      );
      return Boolean(activeSpace?.child_room_ids.includes(roomId));
    }
    case "dms":
      return snapshot.state.domain.rooms.some((room) => room.room_id === roomId && room.is_dm);
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
    display_name: room.display_label ?? room.display_name,
    avatar: room.avatar,
    tags: room.tags,
    unread_count: room.unread_count,
    highlight_count: room.highlight_count ?? 0
  };
}

function utf16Length(value: string): number {
  return [...value].reduce((sum, char) => sum + char.length, 0);
}
