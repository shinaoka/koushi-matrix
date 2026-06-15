import type {
  NativeAttentionCapabilities,
  NativeAttentionState,
  NotificationSettings,
  RoomAttentionKind
} from "./types";

export type DesktopAttentionKind = "mention" | "dm" | "message" | "none";
export const WINDOWS_ATTENTION_OVERLAY_ICON_PATH = "src-tauri/icons/icon.png";
export const DESKTOP_ATTENTION_REQUEST_TYPE = 2;

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
  setOverlayIcon?(icon?: string): Promise<void>;
  setTrayBadgeCount?(count?: number): Promise<void>;
}

export interface DesktopAttentionTransientLike {
  playAttentionSound?(): Promise<void>;
  requestUserAttention?(requestType: typeof DESKTOP_ATTENTION_REQUEST_TYPE): Promise<void>;
}

export type DesktopAttentionTransientPolicy = Pick<NotificationSettings, "sound">;

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
  badgeCount: number,
  capabilities?: NativeAttentionCapabilities
): Promise<void> {
  const operations = [
    windowLike.setTitle(title),
    windowLike.setBadgeCount(badgeCount > 0 ? badgeCount : undefined)
  ];

  if (capabilities?.overlay_icon === "available" && windowLike.setOverlayIcon) {
    operations.push(
      windowLike.setOverlayIcon(
        badgeCount > 0 ? WINDOWS_ATTENTION_OVERLAY_ICON_PATH : undefined
      )
    );
  }

  if (capabilities?.tray === "available" && windowLike.setTrayBadgeCount) {
    operations.push(windowLike.setTrayBadgeCount(badgeCount > 0 ? badgeCount : undefined));
  }

  await Promise.allSettled(operations);
}

export async function dispatchDesktopAttentionTransientEffects(
  transport: DesktopAttentionTransientLike,
  candidate: DesktopAttentionNotificationCandidate | null,
  capabilities?: NativeAttentionCapabilities,
  policy?: DesktopAttentionTransientPolicy
): Promise<void> {
  if (!candidate) {
    return;
  }

  const operations: Promise<void>[] = [];
  const soundEnabled = policy?.sound ?? true;

  if (soundEnabled && capabilities?.sound === "available" && transport.playAttentionSound) {
    operations.push(transport.playAttentionSound());
  }

  if (capabilities?.activation === "available" && transport.requestUserAttention) {
    operations.push(transport.requestUserAttention(DESKTOP_ATTENTION_REQUEST_TYPE));
  }

  await Promise.allSettled(operations);
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
