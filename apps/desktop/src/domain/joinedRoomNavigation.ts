interface JoinedRoom {
  room_id: string;
}

export async function selectJoinedRoomIfPresent(
  rooms: readonly JoinedRoom[],
  roomId: string,
  selectRoom: (roomId: string) => Promise<boolean>
): Promise<boolean> {
  if (!rooms.some((room) => room.room_id === roomId)) return false;
  return selectRoom(roomId);
}
