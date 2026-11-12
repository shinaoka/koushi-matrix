import type { DesktopAttentionDiagnosticSink } from "./desktopAttention";

/**
 * Navigation target of one completed desktop notification click.
 *
 * Rust owns the notification text and target; the webview only receives the
 * identifiers it needs to present the target after a click.
 */
export interface DesktopNotificationActivation {
  account_tab_id: string;
  room_id: string;
  event_id: string | null;
  thread_root_event_id: string | null;
}

export interface DesktopNotificationTransport {
  /**
   * What the OS currently authorizes for this app's banners.
   *
   * Absent where the port cannot answer; the Settings panel then shows no OS
   * state at all rather than claiming one.
   */
  authorization?(): Promise<NotificationAuthorization>;
  /**
   * Ask the OS for permission on an explicit action only; passive notification
   * dispatch never prompts. The prompt policy lives in overview "Desktop
   * Attention Surfaces". macOS prompts at most once per install, so a denied app
   * returns false without prompting and callers offer `openSystemSettings`.
   */
  requestPermission?(): Promise<boolean>;
  /** Open the OS page where the user can allow or deny this app's banners. */
  openSystemSettings?(): Promise<void>;
  /**
   * Ask the Rust adapter to show the notification it owns for the current
   * attention candidate.
   *
   * The banner text, the message-preview gate, and the navigation target are
   * Rust-owned: the webview never composes or reads them.
   */
  show(): Promise<void>;
  clear(): Promise<void>;
  /** Subscribe to completed notification clicks. */
  onActivated(handler: (activation: DesktopNotificationActivation) => void): () => void;
}

/**
 * OS authorization for this app's banners, as the Rust adapter reports it.
 *
 * The persisted Desktop notifications switch is the user's preference; this is
 * what the platform will actually deliver. `unsupported` means the platform has
 * no OS-level grant to report, `unavailable` means it could not answer (for
 * example an unbundled dev build).
 */
export type NotificationAuthorization =
  | "authorized"
  | "provisional"
  | "ephemeral"
  | "denied"
  | "not_determined"
  | "unsupported"
  | "unavailable";

/** The OS action Settings must offer, or `null` when banners will appear. */
export type NotificationPermissionPrompt = "request" | "open_settings";

/**
 * Resolve what the Settings panel owes the user about the OS grant.
 *
 * An ON switch is only the stored preference: while the OS never decided, the
 * app must still ask; once denied, macOS never prompts again, so the only
 * recovery is System Settings.
 */
export function notificationPermissionPrompt(
  authorization: NotificationAuthorization | null,
  desktopNotificationsEnabled: boolean
): NotificationPermissionPrompt | null {
  if (!desktopNotificationsEnabled) {
    return null;
  }
  switch (authorization) {
    case "not_determined":
      return "request";
    case "denied":
      return "open_settings";
    default:
      return null;
  }
}

/** What the Settings panel reads and dispatches about the OS grant. */
export interface NotificationPermission {
  authorization: NotificationAuthorization | null;
  /** Ask the OS, then re-read what it answered. */
  request(): void;
  openSystemSettings(): void;
  /** Re-read the OS state, for example after a settings change. */
  refresh(): void;
}

/**
 * Presentation plan for one notification click.
 *
 * A thread reply opens the thread panel with the reply pinned; every other
 * target navigates the main timeline. A target without an event still opens the
 * room, so a stale notification never becomes a no-op.
 */
export type DesktopNotificationTargetPlan =
  | { kind: "thread"; roomId: string; rootEventId: string; eventId: string }
  | { kind: "event"; roomId: string; eventId: string }
  | { kind: "room"; roomId: string };

export interface DesktopNotificationActivationPlan {
  accountTabId: string;
  target: DesktopNotificationTargetPlan;
}

export function desktopNotificationActivationPlan(
  activation: DesktopNotificationActivation
): DesktopNotificationActivationPlan {
  return {
    accountTabId: activation.account_tab_id,
    target: desktopNotificationTargetPlan(activation)
  };
}

export function desktopNotificationTargetPlan(
  activation: DesktopNotificationActivation
): DesktopNotificationTargetPlan {
  if (activation.thread_root_event_id !== null && activation.event_id !== null) {
    return {
      kind: "thread",
      roomId: activation.room_id,
      rootEventId: activation.thread_root_event_id,
      eventId: activation.event_id
    };
  }
  if (activation.event_id !== null) {
    return { kind: "event", roomId: activation.room_id, eventId: activation.event_id };
  }
  return { kind: "room", roomId: activation.room_id };
}

export async function sendDesktopAttentionNotification(
  transport: DesktopNotificationTransport,
  diagnostic?: DesktopAttentionDiagnosticSink
): Promise<void> {
  try {
    await transport.show();
  } catch {
    diagnostic?.("attention_notification_failed");
  }
}

export async function clearDesktopAttentionNotifications(
  transport: DesktopNotificationTransport,
  diagnostic?: DesktopAttentionDiagnosticSink
): Promise<void> {
  try {
    await transport.clear();
  } catch {
    diagnostic?.("attention_notification_clear_failed");
  }
}
