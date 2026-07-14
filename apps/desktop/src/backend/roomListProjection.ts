import type {
  InvitePreview,
  RoomListFilter,
  RoomListProjection,
  RoomListSort,
  RoomSummary,
  SpaceSummary
} from "../domain/types";

export function computeBrowserRoomListProjection(
  activeFilter: RoomListFilter,
  sort: RoomListSort,
  activeSpaceId: string | null,
  spaces: SpaceSummary[],
  rooms: RoomSummary[],
  invites: InvitePreview[]
): RoomListProjection {
  const activeSpaceChildRoomIds =
    activeSpaceId === null
      ? null
      : spaces.find((space) => space.space_id === activeSpaceId)?.child_room_ids ?? null;
  const items =
    activeFilter.kind === "invites"
      ? invites.map((invite) => ({
          room_id: invite.room_id,
          kind: "invite" as const
        }))
      : rooms
          .filter((room) => roomVisibleInActiveSpace(room, activeSpaceId, activeSpaceChildRoomIds))
          .filter((room) => roomMatchesFilter(room, activeFilter))
          .map((room) => ({
            room_id: room.room_id,
            kind: "room" as const
          }));

  const roomById = new Map(rooms.map((room) => [room.room_id, room]));
  items.sort((left, right) => {
    switch (sort.kind) {
      case "activity":
      case "recentFirst": {
        return compareConversationActivity(
          roomById.get(left.room_id),
          roomById.get(right.room_id)
        );
      }
      case "normalLocale": {
        const leftRoom = rooms.find((room) => room.room_id === left.room_id);
        const rightRoom = rooms.find((room) => room.room_id === right.room_id);
        const leftLabel = (leftRoom?.display_label ?? left.room_id).toLowerCase();
        const rightLabel = (rightRoom?.display_label ?? right.room_id).toLowerCase();
        return leftLabel.localeCompare(rightLabel) || left.room_id.localeCompare(right.room_id);
      }
    }
  });

  return {
    active_filter: activeFilter,
    sort,
    items
  };
}

function compareConversationActivity(
  left: RoomSummary | undefined,
  right: RoomSummary | undefined
): number {
  const leftActivity = left?.conversation_activity ?? null;
  const rightActivity = right?.conversation_activity ?? null;
  if (leftActivity === null && rightActivity !== null) return 1;
  if (leftActivity !== null && rightActivity === null) return -1;
  if (leftActivity !== null && rightActivity !== null) {
    const timestampOrder = rightActivity.timestamp_ms - leftActivity.timestamp_ms;
    if (timestampOrder !== 0) return timestampOrder;
  }
  const leftLabel = (left?.display_label ?? "").toLowerCase();
  const rightLabel = (right?.display_label ?? "").toLowerCase();
  if (leftLabel < rightLabel) return -1;
  if (leftLabel > rightLabel) return 1;
  const leftId = left?.room_id ?? "";
  const rightId = right?.room_id ?? "";
  return leftId < rightId ? -1 : leftId > rightId ? 1 : 0;
}

function roomVisibleInActiveSpace(
  room: RoomSummary,
  activeSpaceId: string | null,
  activeSpaceChildRoomIds: string[] | null
): boolean {
  if (activeSpaceId === null) {
    return room.is_dm || room.parent_space_ids.length === 0;
  }
  if (room.is_dm) {
    return room.dm_space_ids.includes(activeSpaceId);
  }
  return (
    room.parent_space_ids.includes(activeSpaceId) ||
    (activeSpaceChildRoomIds?.includes(room.room_id) ?? false)
  );
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
      return !room.is_dm && room.tags.favourite === null && room.tags.low_priority === null;
    case "favourites":
      return room.tags.favourite !== null;
    case "invites":
      return false;
  }
}
