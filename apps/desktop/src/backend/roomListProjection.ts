import type {
  InvitePreview,
  RoomListFilter,
  RoomListProjection,
  RoomListSort,
  RoomSummary
} from "../domain/types";

export function computeBrowserRoomListProjection(
  activeFilter: RoomListFilter,
  sort: RoomListSort,
  rooms: RoomSummary[],
  invites: InvitePreview[]
): RoomListProjection {
  const items =
    activeFilter.kind === "invites"
      ? invites.map((invite) => ({
          room_id: invite.room_id,
          kind: "invite" as const
        }))
      : rooms
          .filter((room) => roomMatchesFilter(room, activeFilter))
          .map((room) => ({
            room_id: room.room_id,
            kind: "room" as const
          }));

  if (sort.kind === "activity") {
    const activityByRoomId = new Map(
      rooms.map((room) => [room.room_id, room.last_activity_ms ?? 0])
    );
    items.sort((left, right) => {
      const activityDelta =
        (activityByRoomId.get(right.room_id) ?? 0) -
        (activityByRoomId.get(left.room_id) ?? 0);
      return activityDelta || left.room_id.localeCompare(right.room_id);
    });
  }

  return {
    active_filter: activeFilter,
    sort,
    items
  };
}

function roomMatchesFilter(room: RoomSummary, filter: RoomListFilter): boolean {
  switch (filter.kind) {
    case "unread":
      return (
        room.unread_count > 0 ||
        (room.notification_count ?? 0) > 0 ||
        (room.highlight_count ?? 0) > 0 ||
        Boolean(room.marked_unread)
      );
    case "people":
      return room.is_dm;
    case "rooms":
      return !room.is_dm;
    case "favourites":
      return room.tags.favourite !== null;
    case "invites":
      return false;
  }
}
