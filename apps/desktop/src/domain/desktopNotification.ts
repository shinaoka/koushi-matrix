import type { DesktopAttentionDiagnosticSink } from "./desktopAttention";

/**
 * Navigation target of one completed desktop notification click.
 *
 * Rust owns the notification text and target; the webview only receives the
 * identifiers it needs to present the target after a click.
 */
export interface DesktopNotificationActivation {
  room_id: string;
  event_id: string | null;
  thread_root_event_id: string | null;
}

export interface DesktopNotificationTransport {
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
