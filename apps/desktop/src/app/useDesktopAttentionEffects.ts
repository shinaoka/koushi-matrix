import { useEffect, useMemo, useRef } from "react";

// This hook is the React-owned platform-lifecycle seam for desktop attention.
import { desktopAttentionPortForAccount } from "../backend/desktopAttentionRuntime";
import type { TimelineDiagnosticLogEntry } from "../components/TimelineView";
import {
  applyDesktopAttentionToWindow,
  createDesktopCandidateSoundDispatcher,
  dispatchDesktopAttentionTransientEffects,
  desktopAttentionNotificationCandidate
} from "../domain/desktopAttention";
import {
  clearDesktopAttentionNotifications,
  sendDesktopAttentionNotification
} from "../domain/desktopNotification";
import type { DesktopNotificationActivation } from "../domain/desktopNotification";
import type { DesktopAttentionSummary } from "../domain/desktopAttention";
import type { DesktopSnapshot } from "../domain/types";

type DesktopAttentionEffectsInput = {
  accountTabId: string | null;
  snapshot: DesktopSnapshot | null;
  attentionWindowTitle: string;
  safeAttentionSummary: DesktopAttentionSummary;
  aggregateBadgeCount?: number;
  appendDiagnosticLog: (entry: TimelineDiagnosticLogEntry) => void;
  /** Present a completed notification click through the existing flows. */
  onNotificationActivated: (activation: DesktopNotificationActivation) => void;
};

const desktopCandidateSoundDispatcher = createDesktopCandidateSoundDispatcher();

export function useDesktopAttentionEffects({
  accountTabId,
  snapshot,
  attentionWindowTitle,
  safeAttentionSummary,
  aggregateBadgeCount,
  appendDiagnosticLog,
  onNotificationActivated
}: DesktopAttentionEffectsInput): void {
  const desktopAttentionPort = useMemo(
    () => desktopAttentionPortForAccount(accountTabId ?? undefined),
    [accountTabId]
  );
  // #1802: the webview dedupes on the Rust-minted alert identity alone. Contents
  // (event ids, counts, labels) move inside one alert, so they cannot be the key.
  const alertId = snapshot?.state.domain.native_attention.summary.alert_id ?? null;
  // A snapshot that carries a candidate without an identity is not "no alert": an
  // older producer keeps the pre-#1802 content key instead of going silent.
  const alertCandidate = snapshot?.state.domain.native_attention.summary.candidate;
  const dedupeKey =
    alertId ??
    (alertCandidate
      ? `content:${alertCandidate.room_display_name}|${alertCandidate.kind}|${alertCandidate.unread_count}|${alertCandidate.highlight_count}`
      : null);
  const badgeCount = aggregateBadgeCount ?? safeAttentionSummary.badgeCount;
  const notificationActivationHandler = useRef(onNotificationActivated);
  const attentionCapabilities = useMemo(
    () => snapshot?.state.domain.native_attention.summary.capabilities,
    [
      snapshot?.state.domain.native_attention.summary.capabilities.activation,
      snapshot?.state.domain.native_attention.summary.capabilities.badge,
      snapshot?.state.domain.native_attention.summary.capabilities.notifications,
      snapshot?.state.domain.native_attention.summary.capabilities.overlay_icon,
      snapshot?.state.domain.native_attention.summary.capabilities.sound,
      snapshot?.state.domain.native_attention.summary.capabilities.tray
    ]
  );

  useEffect(() => {
    document.title = attentionWindowTitle;
    if (!desktopAttentionPort) {
      return;
    }

    void applyDesktopAttentionToWindow(
      desktopAttentionPort.currentWindow(),
      attentionWindowTitle,
      badgeCount,
      attentionCapabilities,
      (token) => appendDiagnosticLog({
        timestampMs: Date.now(),
        source: "native.attention",
        message: token
      }),
      desktopAttentionPort.nativeBadge
    );

    if (!snapshot || snapshot.state.domain.session.kind !== "ready") {
      desktopCandidateSoundDispatcher.reset();
    }
  }, [
    attentionCapabilities,
    attentionWindowTitle,
    badgeCount,
    desktopAttentionPort,
    snapshot?.state.domain.session.kind
  ]);

  useEffect(() => {
    if (!snapshot || snapshot.state.domain.session.kind !== "ready") {
      return;
    }

    const candidate = desktopAttentionNotificationCandidate(
      snapshot.state.domain.native_attention
    );

    if (!candidate || !desktopAttentionPort) {
      return;
    }

    void desktopCandidateSoundDispatcher.observe(
      desktopAttentionPort.sound,
      candidate,
      snapshot.state.domain.native_attention.summary.capabilities,
      snapshot.state.domain.settings.values.notifications,
      (token) => appendDiagnosticLog({
        timestampMs: Date.now(),
        source: "native.attention",
        message: token
      })
    );

    const currentWindow = desktopAttentionPort.currentWindow();
    void dispatchDesktopAttentionTransientEffects(
      {
        requestUserAttention: (requestType) => currentWindow.requestUserAttention(requestType)
      },
      candidate,
      snapshot.state.domain.native_attention.summary.capabilities,
      { sound: false },
      (token) => appendDiagnosticLog({
        timestampMs: Date.now(),
        source: "native.attention",
        message: token
      })
    );
    void sendDesktopAttentionNotification(desktopAttentionPort.notifications, (token) =>
      appendDiagnosticLog({ timestampMs: Date.now(), source: "native.attention", message: token })
    );
  }, [
    dedupeKey,
    desktopAttentionPort
  ]);

  useEffect(() => {
    notificationActivationHandler.current = onNotificationActivated;
  }, [onNotificationActivated]);

  useEffect(() => {
    if (!desktopAttentionPort) {
      return;
    }

    return desktopAttentionPort.notifications.onActivated((activation) => {
      appendDiagnosticLog({
        timestampMs: Date.now(),
        source: "native.attention",
        message: `attention_notification_activated thread=${activation.thread_root_event_id !== null}`
      });
      notificationActivationHandler.current(activation);
    });
  }, [appendDiagnosticLog, desktopAttentionPort]);

  const accountUnreadCount = safeAttentionSummary.unreadTotal;
  const sessionReady = snapshot?.state.domain.session.kind === "ready";
  const notificationsEnabled = snapshot?.state.domain.settings.values.notifications.desktop_notifications;
  useEffect(() => {
    if (!desktopAttentionPort || (sessionReady && notificationsEnabled && accountUnreadCount !== 0)) {
      return;
    }

    void clearDesktopAttentionNotifications(desktopAttentionPort.notifications, (token) =>
      appendDiagnosticLog({ timestampMs: Date.now(), source: "native.attention", message: token })
    );
  }, [accountUnreadCount, sessionReady, notificationsEnabled, desktopAttentionPort]);
}
