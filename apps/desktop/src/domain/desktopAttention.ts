import type {
  NativeAttentionCapabilities,
  NativeAttentionState,
  NotificationSettings,
  RoomAttentionKind
} from "./types";

export type DesktopAttentionKind = "mention" | "dm" | "message" | "none";
export const WINDOWS_ATTENTION_OVERLAY_ICON_PATH = "src-tauri/icons/icon.png";
export const DESKTOP_ATTENTION_REQUEST_TYPE = 2;
export const DESKTOP_ATTENTION_SOUND_COOLDOWN_MS = 3_000;

export type DesktopAttentionDiagnosticToken =
  | "attention_title_failed"
  | "attention_badge_failed"
  | "attention_overlay_failed"
  | "attention_tray_failed"
  | "attention_sound_failed"
  | "attention_activation_failed"
  | "attention_notification_failed"
  | "attention_notification_clear_failed";
export type DesktopAttentionDiagnosticSink = (token: DesktopAttentionDiagnosticToken) => void;

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
  playAttentionSound?(): Promise<NativeAttentionSoundOutcome>;
  requestUserAttention?(requestType: typeof DESKTOP_ATTENTION_REQUEST_TYPE): Promise<void>;
}
export type NativeAttentionSoundOutcome = "played" | "unsupported" | "failed";

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
  capabilities?: NativeAttentionCapabilities,
  diagnostic?: DesktopAttentionDiagnosticSink
): Promise<void> {
  const operations = [runNativeOperation(() => windowLike.setTitle(title), "attention_title_failed", diagnostic)];

  if (capabilities?.badge === "available") {
    operations.push(runNativeOperation(
      () => windowLike.setBadgeCount(badgeCount > 0 ? badgeCount : undefined),
      "attention_badge_failed",
      diagnostic
    ));
  }

  if (
    capabilities?.badge === "available" &&
    capabilities.overlay_icon === "available" &&
    windowLike.setOverlayIcon
  ) {
    operations.push(
      runNativeOperation(() => windowLike.setOverlayIcon!(
        badgeCount > 0 ? WINDOWS_ATTENTION_OVERLAY_ICON_PATH : undefined
      ), "attention_overlay_failed", diagnostic)
    );
  }

  if (
    capabilities?.badge === "available" &&
    capabilities.tray === "available" &&
    windowLike.setTrayBadgeCount
  ) {
    operations.push(runNativeOperation(
      () => windowLike.setTrayBadgeCount!(badgeCount > 0 ? badgeCount : undefined),
      "attention_tray_failed",
      diagnostic
    ));
  }

  await Promise.allSettled(operations);
}

async function runNativeOperation(
  operation: () => Promise<void>,
  failureToken: DesktopAttentionDiagnosticToken,
  diagnostic?: DesktopAttentionDiagnosticSink
): Promise<void> {
  try {
    await operation();
  } catch {
    diagnostic?.(failureToken);
  }
}

export async function dispatchDesktopAttentionTransientEffects(
  transport: DesktopAttentionTransientLike,
  candidate: DesktopAttentionNotificationCandidate | null,
  capabilities?: NativeAttentionCapabilities,
  policy?: DesktopAttentionTransientPolicy,
  diagnostic?: DesktopAttentionDiagnosticSink
): Promise<void> {
  if (!candidate) {
    return;
  }

  const operations: Promise<void>[] = [];
  const soundEnabled = policy?.sound ?? true;

  if (soundEnabled && capabilities?.sound === "available" && transport.playAttentionSound) {
    operations.push(runNativeOperation(
      async () => { await transport.playAttentionSound!(); }, "attention_sound_failed", diagnostic
    ));
  }

  if (capabilities?.activation === "available" && transport.requestUserAttention) {
    operations.push(runNativeOperation(
      () => transport.requestUserAttention!(DESKTOP_ATTENTION_REQUEST_TYPE),
      "attention_activation_failed",
      diagnostic
    ));
  }

  await Promise.allSettled(operations);
}

export interface DesktopAttentionTransientDispatcher {
  dispatch(
    transport: DesktopAttentionTransientLike,
    candidate: DesktopAttentionNotificationCandidate | null,
    capabilities: NativeAttentionCapabilities,
    policy: DesktopAttentionTransientPolicy,
    diagnostic?: DesktopAttentionDiagnosticSink
  ): Promise<void>;
}

export function createDesktopAttentionTransientDispatcher(
  now: () => number = Date.now,
  cooldownMs = DESKTOP_ATTENTION_SOUND_COOLDOWN_MS
): DesktopAttentionTransientDispatcher {
  let lastSoundAt = Number.NEGATIVE_INFINITY;
  return {
    async dispatch(transport, candidate, capabilities, policy, diagnostic) {
      const timestamp = now();
      const soundAllowed = timestamp - lastSoundAt >= cooldownMs;
      if (
        candidate && policy.sound && capabilities.sound === "available" &&
        soundAllowed && transport.playAttentionSound
      ) {
        try {
          const outcome = await transport.playAttentionSound();
          if (outcome === "played") {
            lastSoundAt = timestamp;
          } else if (outcome === "failed") {
            diagnostic?.("attention_sound_failed");
          }
        } catch {
          diagnostic?.("attention_sound_failed");
        }
      }
      await dispatchDesktopAttentionTransientEffects(
        { ...transport, playAttentionSound: undefined },
        candidate,
        capabilities,
        { sound: false },
        diagnostic
      );
    }
  };
}

export function createTauriDesktopAttentionTransientTransport(
  invokeNative: () => Promise<NativeAttentionSoundOutcome>
): DesktopAttentionTransientLike {
  return {
    async playAttentionSound() {
      return invokeNative();
    }
  };
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
