import type {
  NativeAttentionCapability,
  NativeAttentionCapabilities,
  NativeAttentionState,
  NotificationSettings,
  RoomAttentionKind
} from "./types";

export type DesktopAttentionKind = "mention" | "dm" | "message" | "none";
export const WINDOWS_ATTENTION_OVERLAY_ICON_PATH = "src-tauri/icons/icon.png";
export const DESKTOP_ATTENTION_REQUEST_TYPE = 2;
export const DESKTOP_ATTENTION_SOUND_COOLDOWN_MS = 3_000;

export type DesktopAttentionDiagnosticToken = string;
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

export type NativeAttentionBadgeOutcome = "applied" | "unsupported" | "mismatch";

export interface DesktopNativeBadgeLike {
  setBadgeCount(count?: number): Promise<NativeAttentionBadgeOutcome>;
}

export interface DesktopAttentionTransientLike {
  playAttentionSound?(): Promise<NativeAttentionSoundOutcome>;
  requestUserAttention?(requestType: typeof DESKTOP_ATTENTION_REQUEST_TYPE): Promise<void>;
}
export type NativeAttentionSoundOutcome = "played" | "unsupported" | "failed" | "skipped";

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
  diagnostic?: DesktopAttentionDiagnosticSink,
  nativeBadge?: DesktopNativeBadgeLike
): Promise<void> {
  const normalizedBadgeCount = normalizeAttentionCount(badgeCount);
  diagnostic?.(
    [
      `attention_window_apply badge_count=${normalizedBadgeCount}`,
      `badge=${capabilityToken(capabilities?.badge)}`,
      `overlay=${capabilityToken(capabilities?.overlay_icon)}`,
      `tray=${capabilityToken(capabilities?.tray)}`
    ].join(" ")
  );

  const operations = [
    runNativeOperation(
      () => windowLike.setTitle(title),
      "attention_title_failed",
      diagnostic,
      undefined,
      "attention_title_set"
    )
  ];

  if (capabilities?.badge === "available") {
    operations.push(applyDesktopBadge(
      windowLike,
      nativeBadge,
      normalizedBadgeCount,
      diagnostic
    ));
  } else {
    diagnostic?.(`attention_badge_skipped capability=${capabilityToken(capabilities?.badge)}`);
  }

  if (
    capabilities?.badge === "available" &&
    capabilities.overlay_icon === "available" &&
    windowLike.setOverlayIcon
  ) {
    operations.push(
      runNativeOperation(() => windowLike.setOverlayIcon!(
        normalizedBadgeCount > 0 ? WINDOWS_ATTENTION_OVERLAY_ICON_PATH : undefined
      ), "attention_overlay_failed", diagnostic, overlayFailureToken, `attention_overlay_set count=${normalizedBadgeCount}`)
    );
  } else {
    diagnostic?.(
      `attention_overlay_skipped badge=${capabilityToken(capabilities?.badge)} overlay=${capabilityToken(capabilities?.overlay_icon)} transport=${windowLike.setOverlayIcon ? "available" : "missing"}`
    );
  }

  if (
    capabilities?.badge === "available" &&
    capabilities.tray === "available" &&
    windowLike.setTrayBadgeCount
  ) {
    operations.push(runNativeOperation(
      () => windowLike.setTrayBadgeCount!(normalizedBadgeCount > 0 ? normalizedBadgeCount : undefined),
      "attention_tray_failed",
      diagnostic,
      undefined,
      `attention_tray_set count=${normalizedBadgeCount}`
    ));
  } else {
    diagnostic?.(
      `attention_tray_skipped badge=${capabilityToken(capabilities?.badge)} tray=${capabilityToken(capabilities?.tray)} transport=${windowLike.setTrayBadgeCount ? "available" : "missing"}`
    );
  }

  await Promise.allSettled(operations);
}

async function applyDesktopBadge(
  windowLike: DesktopWindowLike,
  nativeBadge: DesktopNativeBadgeLike | undefined,
  count: number,
  diagnostic?: DesktopAttentionDiagnosticSink
): Promise<void> {
  const requestedCount = count > 0 ? count : undefined;
  if (nativeBadge) {
    try {
      const outcome = await nativeBadge.setBadgeCount(requestedCount);
      diagnostic?.(`attention_badge_native_backend outcome=${outcome} count=${count}`);
      if (outcome === "applied") {
        return;
      }
    } catch {
      diagnostic?.("attention_badge_native_backend_failed");
    }
  }

  await runNativeOperation(
    () => windowLike.setBadgeCount(requestedCount),
    "attention_badge_failed",
    diagnostic,
    undefined,
    `attention_badge_set count=${count}`
  );
}

async function runNativeOperation(
  operation: () => Promise<void>,
  failureToken: DesktopAttentionDiagnosticToken,
  diagnostic?: DesktopAttentionDiagnosticSink,
  classifyFailure?: (error: unknown) => DesktopAttentionDiagnosticToken,
  successToken?: DesktopAttentionDiagnosticToken
): Promise<void> {
  try {
    await operation();
    if (successToken) {
      diagnostic?.(successToken);
    }
  } catch (error) {
    diagnostic?.(classifyFailure?.(error) ?? failureToken);
  }
}

function overlayFailureToken(error: unknown): DesktopAttentionDiagnosticToken {
  const message = error instanceof Error ? error.message : String(error);
  const normalized = message.toLowerCase();
  return normalized.includes("overlay") && normalized.includes("not allowed")
    ? "attention_overlay_acl_denied"
    : "attention_overlay_failed";
}

export async function dispatchDesktopAttentionTransientEffects(
  transport: DesktopAttentionTransientLike,
  candidate: DesktopAttentionNotificationCandidate | null,
  capabilities?: NativeAttentionCapabilities,
  policy?: DesktopAttentionTransientPolicy,
  diagnostic?: DesktopAttentionDiagnosticSink
): Promise<void> {
  if (!candidate) {
    diagnostic?.("attention_transient_skipped reason=no_candidate");
    return;
  }

  const operations: Promise<void>[] = [];
  const soundEnabled = policy?.sound ?? true;
  diagnostic?.(
    [
      `attention_transient_candidate kind=${candidate.kind}`,
      `unread=${normalizeAttentionCount(candidate.unreadCount)}`,
      `highlight=${normalizeAttentionCount(candidate.highlightCount)}`,
      `sound=${capabilityToken(capabilities?.sound)}`,
      `activation=${capabilityToken(capabilities?.activation)}`,
      `policy_sound=${soundEnabled ? "true" : "false"}`
    ].join(" ")
  );

  if (soundEnabled && capabilities?.sound === "available" && transport.playAttentionSound) {
    operations.push(
      runNativeOperation(async () => {
        const outcome = await transport.playAttentionSound!();
        diagnostic?.(`attention_sound_outcome outcome=${outcome}`);
      }, "attention_sound_failed", diagnostic)
    );
  } else {
    diagnostic?.(
      `attention_sound_skipped policy_sound=${soundEnabled ? "true" : "false"} capability=${capabilityToken(capabilities?.sound)} transport=${transport.playAttentionSound ? "available" : "missing"}`
    );
  }

  if (capabilities?.activation === "available" && transport.requestUserAttention) {
    operations.push(runNativeOperation(
      () => transport.requestUserAttention!(DESKTOP_ATTENTION_REQUEST_TYPE),
      "attention_activation_failed",
      diagnostic,
      undefined,
      "attention_activation_requested"
    ));
  } else {
    diagnostic?.(
      `attention_activation_skipped capability=${capabilityToken(capabilities?.activation)} transport=${transport.requestUserAttention ? "available" : "missing"}`
    );
  }

  await Promise.allSettled(operations);
}

export interface DesktopBadgeSoundDispatcher {
  observe(
    transport: DesktopAttentionTransientLike,
    badgeCount: number,
    capabilities: NativeAttentionCapabilities,
    policy: DesktopAttentionTransientPolicy,
    diagnostic?: DesktopAttentionDiagnosticSink
  ): Promise<void>;
  reset(): void;
}

export function createDesktopBadgeSoundDispatcher(
  now: () => number = Date.now,
  cooldownMs = DESKTOP_ATTENTION_SOUND_COOLDOWN_MS
): DesktopBadgeSoundDispatcher {
  let previousBadgeCount: number | null = null;
  let lastSoundAt = Number.NEGATIVE_INFINITY;
  let soundInFlight = false;
  let generation = 0;
  return {
    reset() {
      generation += 1;
      previousBadgeCount = null;
      lastSoundAt = Number.NEGATIVE_INFINITY;
      soundInFlight = false;
    },
    async observe(transport, badgeCount, capabilities, policy, diagnostic) {
      const currentBadgeCount = normalizeAttentionCount(badgeCount);
      if (previousBadgeCount === null) {
        previousBadgeCount = currentBadgeCount;
        diagnostic?.(`attention_badge_sound_baseline count=${currentBadgeCount}`);
        return;
      }

      const priorBadgeCount = previousBadgeCount;
      previousBadgeCount = currentBadgeCount;
      if (currentBadgeCount <= priorBadgeCount) {
        if (currentBadgeCount < priorBadgeCount) {
          diagnostic?.(
            `attention_badge_sound_decrease previous=${priorBadgeCount} current=${currentBadgeCount}`
          );
        }
        return;
      }

      diagnostic?.(
        `attention_badge_sound_delta previous=${priorBadgeCount} current=${currentBadgeCount} delta=${currentBadgeCount - priorBadgeCount}`
      );
      const timestamp = now();
      const soundAllowed = !soundInFlight && timestamp - lastSoundAt >= cooldownMs;
      if (
        policy.sound && capabilities.sound === "available" &&
        soundAllowed && transport.playAttentionSound
      ) {
        const dispatchGeneration = generation;
        soundInFlight = true;
        try {
          const outcome = await transport.playAttentionSound();
          diagnostic?.(`attention_sound_outcome outcome=${outcome}`);
          if (outcome === "played" && generation === dispatchGeneration) {
            lastSoundAt = now();
          } else if (outcome === "failed") {
            diagnostic?.("attention_sound_failed");
          }
        } catch {
          diagnostic?.("attention_sound_failed");
        } finally {
          if (generation === dispatchGeneration) {
            soundInFlight = false;
          }
        }
      } else {
        diagnostic?.(
          `attention_sound_skipped policy_sound=${policy.sound ? "true" : "false"} capability=${capabilityToken(capabilities.sound)} transport=${transport.playAttentionSound ? "available" : "missing"} cooldown=${soundAllowed ? "false" : "true"} inflight=${soundInFlight ? "true" : "false"}`
        );
      }
    }
  };
}

function capabilityToken(capability: NativeAttentionCapability | undefined): NativeAttentionCapability | "missing" {
  return capability ?? "missing";
}

function normalizeAttentionCount(count: number): number {
  return Number.isFinite(count) ? Math.max(0, Math.trunc(count)) : 0;
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
