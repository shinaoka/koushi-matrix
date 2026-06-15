import {
  cancelAll,
  isPermissionGranted,
  removeAllActive,
  sendNotification
} from "@tauri-apps/plugin-notification";

import type { DesktopAttentionNotificationCandidate } from "./desktopAttention";

export interface DesktopNotificationContent {
  title: string;
  body: string;
}

export interface DesktopNotificationTransport {
  notify(content: DesktopNotificationContent): Promise<void>;
  clear(): Promise<void>;
}

export function desktopAttentionNotificationContent(
  candidate: DesktopAttentionNotificationCandidate
): DesktopNotificationContent {
  switch (candidate.kind) {
    case "mention":
      return {
        title: `Mention in ${candidate.roomDisplayName}`,
        body: joinAttentionCounts([
          formatCount(candidate.highlightCount, "mention"),
          formatCount(candidate.unreadCount, "unread", "unread")
        ])
      };
    case "dm":
      return {
        title: `Direct message in ${candidate.roomDisplayName}`,
        body: joinAttentionCounts([formatCount(candidate.unreadCount, "unread", "unread")])
      };
    case "message":
      return {
        title: `Message in ${candidate.roomDisplayName}`,
        body: joinAttentionCounts([formatCount(candidate.unreadCount, "unread", "unread")])
      };
  }
}

export async function sendDesktopAttentionNotification(
  candidate: DesktopAttentionNotificationCandidate,
  transport: DesktopNotificationTransport
): Promise<void> {
  try {
    await transport.notify(desktopAttentionNotificationContent(candidate));
  } catch {
    // Best-effort desktop attention: notification failures must not surface.
  }
}

export async function clearDesktopAttentionNotifications(
  transport: DesktopNotificationTransport
): Promise<void> {
  try {
    await transport.clear();
  } catch {
    // Best-effort desktop attention: native clearing failures must not surface.
  }
}

export function createTauriDesktopNotificationTransport(): DesktopNotificationTransport {
  let permissionPromise: Promise<boolean> | null = null;

  return {
    async notify(content: DesktopNotificationContent) {
      permissionPromise ??= ensureNotificationPermission();
      if (!(await permissionPromise)) {
        return;
      }
      await sendNotification(content);
    },
    async clear() {
      await Promise.allSettled([cancelAll(), removeAllActive()]);
    }
  };
}

async function ensureNotificationPermission(): Promise<boolean> {
  return isPermissionGranted();
}

function joinAttentionCounts(parts: string[]): string {
  return parts.filter((part) => part.length > 0).join(", ");
}

function formatCount(count: number, singularLabel: string, pluralLabel = `${singularLabel}s`): string {
  if (count === 0) {
    return "";
  }
  return `${count} ${count === 1 ? singularLabel : pluralLabel}`;
}
