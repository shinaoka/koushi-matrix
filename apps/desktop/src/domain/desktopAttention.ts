import type { NativeAttentionState, RoomAttentionKind } from "./types";

export type DesktopAttentionKind = "mention" | "dm" | "message" | "none";

export interface DesktopAttentionSummary {
  unreadTotal: number;
  badgeCount: number;
  notificationKind: DesktopAttentionKind;
  titleHint: string | null;
  qaTitleToken: string;
}

export interface DesktopAttentionNotificationCandidate {
  roomDisplayName: string;
  kind: RoomAttentionKind;
  unreadCount: number;
  highlightCount: number;
}

export interface DesktopWindowLike {
  setTitle(title: string): Promise<void>;
  setBadgeCount(count?: number): Promise<void>;
}

export function desktopAttentionSummary(attention: NativeAttentionState): DesktopAttentionSummary {
  const unreadTotal = attention.summary.unread_count;
  const badgeCount = attention.summary.badge_count;
  const notificationKind = attention.summary.candidate?.kind ?? "none";

  return {
    unreadTotal,
    badgeCount,
    notificationKind,
    titleHint: unreadTotal > 0 ? `${unreadTotal} unread` : null,
    qaTitleToken: `unread=${unreadTotal} badge=${badgeCount} notify=${notificationKind}`
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
  attention: NativeAttentionState
): DesktopAttentionNotificationCandidate | null {
  if (attention.dispatch.kind !== "idle" || !attention.summary.candidate) {
    return null;
  }

  const candidate = attention.summary.candidate;
  return {
    roomDisplayName: candidate.room_display_name,
    kind: candidate.kind,
    unreadCount: candidate.unread_count,
    highlightCount: candidate.highlight_count
  };
}
