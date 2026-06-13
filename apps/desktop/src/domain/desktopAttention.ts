export type DesktopAttentionKind = "mention" | "dm" | "message" | "none";

export interface DesktopAttentionRoomSummary {
  room_id: string;
  display_name: string;
  is_dm: boolean;
  unread_count: number;
  notification_count?: number;
  highlight_count?: number;
}

export interface DesktopAttentionInput {
  activeRoomId: string | null;
  rooms: readonly DesktopAttentionRoomSummary[];
}

export interface DesktopAttentionSummary {
  unreadTotal: number;
  badgeCount: number;
  notificationKind: DesktopAttentionKind;
  titleHint: string | null;
  qaTitleToken: string;
}

export interface DesktopAttentionNotificationCandidate {
  roomDisplayName: string;
  kind: Exclude<DesktopAttentionKind, "none">;
  unreadCount: number;
  notificationCount: number;
  highlightCount: number;
}

export interface DesktopWindowLike {
  setTitle(title: string): Promise<void>;
  setBadgeCount(count?: number): Promise<void>;
}

export function desktopAttentionSummary(input: DesktopAttentionInput): DesktopAttentionSummary {
  const unreadTotal = input.rooms.reduce((sum, room) => sum + attentionCount(room), 0);
  const notificationKind = notificationKindForRooms(input.rooms);

  return {
    unreadTotal,
    badgeCount: unreadTotal,
    notificationKind,
    titleHint: unreadTotal > 0 ? `${unreadTotal} unread` : null,
    qaTitleToken: `unread=${unreadTotal} badge=${unreadTotal} notify=${notificationKind}`
  };
}

export function desktopAttentionWindowTitle(
  baseTitle: string,
  summary: DesktopAttentionSummary
): string {
  return summary.titleHint ? `${baseTitle} · ${summary.titleHint}` : baseTitle;
}

export async function applyDesktopAttentionToWindow(
  windowLike: DesktopWindowLike,
  title: string,
  badgeCount: number
): Promise<void> {
  await Promise.allSettled([
    windowLike.setTitle(title),
    windowLike.setBadgeCount(badgeCount > 0 ? badgeCount : undefined)
  ]);
}

export function desktopAttentionNotificationCandidate(
  current: DesktopAttentionInput,
  previous: DesktopAttentionInput | null
): DesktopAttentionNotificationCandidate | null {
  if (!previous) {
    return null;
  }

  const previousByRoom = new Map(previous.rooms.map((room) => [room.room_id, room] as const));
  const candidates: DesktopAttentionCandidateEntry[] = [];

  for (const room of current.rooms) {
    if (room.room_id === current.activeRoomId) {
      continue;
    }

    const previousRoom = previousByRoom.get(room.room_id);
    if (!previousRoom) {
      continue;
    }

    const currentHighlight = highlightCount(room);
    const previousHighlight = highlightCount(previousRoom);
    const currentAttention = attentionCount(room);
    const previousAttention = attentionCount(previousRoom);

    if (currentHighlight > previousHighlight) {
      candidates.push({
        roomDisplayName: room.display_name,
        kind: "mention",
        unreadCount: room.unread_count,
        notificationCount: room.notification_count ?? 0,
        highlightCount: currentHighlight,
        priority: 0,
        delta: currentHighlight - previousHighlight
      });
      continue;
    }

    if (room.is_dm && currentAttention > previousAttention) {
      candidates.push({
        roomDisplayName: room.display_name,
        kind: "dm",
        unreadCount: room.unread_count,
        notificationCount: room.notification_count ?? 0,
        highlightCount: currentHighlight,
        priority: 1,
        delta: currentAttention - previousAttention
      });
      continue;
    }

    if (!room.is_dm && currentAttention > previousAttention) {
      candidates.push({
        roomDisplayName: room.display_name,
        kind: "message",
        unreadCount: room.unread_count,
        notificationCount: room.notification_count ?? 0,
        highlightCount: currentHighlight,
        priority: 2,
        delta: currentAttention - previousAttention
      });
    }
  }

  if (!candidates.length) {
    return null;
  }

  candidates.sort((left, right) => {
    if (left.priority !== right.priority) {
      return left.priority - right.priority;
    }
    if (left.delta !== right.delta) {
      return right.delta - left.delta;
    }
    return left.roomDisplayName.localeCompare(right.roomDisplayName);
  });

  const candidate = candidates[0];
  return {
    roomDisplayName: candidate.roomDisplayName,
    kind: candidate.kind,
    unreadCount: candidate.unreadCount,
    notificationCount: candidate.notificationCount,
    highlightCount: candidate.highlightCount
  };
}

type DesktopAttentionCandidateEntry = DesktopAttentionNotificationCandidate & {
  priority: number;
  delta: number;
};

function attentionCount(room: Pick<DesktopAttentionRoomSummary, "notification_count" | "unread_count"> | undefined): number {
  if (!room) {
    return 0;
  }
  return Math.max(room.notification_count ?? 0, room.unread_count);
}

function highlightCount(room: Pick<DesktopAttentionRoomSummary, "highlight_count"> | undefined): number {
  return room?.highlight_count ?? 0;
}

function notificationKindForRooms(rooms: readonly DesktopAttentionRoomSummary[]): DesktopAttentionKind {
  if (rooms.some((room) => highlightCount(room) > 0)) {
    return "mention";
  }
  if (rooms.some((room) => room.is_dm && attentionCount(room) > 0)) {
    return "dm";
  }
  if (rooms.some((room) => attentionCount(room) > 0)) {
    return "message";
  }
  return "none";
}
