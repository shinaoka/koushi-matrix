import { useEffect, useMemo, useRef } from "react";

// This hook is the React-owned platform-lifecycle seam for desktop attention.
import { desktopAttentionPort } from "../backend/desktopAttentionRuntime";
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
  snapshot: DesktopSnapshot | null;
  attentionWindowTitle: string;
  safeAttentionSummary: DesktopAttentionSummary;
  appendDiagnosticLog: (entry: TimelineDiagnosticLogEntry) => void;
  /** Present a completed notification click through the existing flows. */
  onNotificationActivated: (activation: DesktopNotificationActivation) => void;
};

const desktopCandidateSoundDispatcher = createDesktopCandidateSoundDispatcher();

export function useDesktopAttentionEffects({
  snapshot,
  attentionWindowTitle,
  safeAttentionSummary,
  appendDiagnosticLog,
  onNotificationActivated
}: DesktopAttentionEffectsInput): void {
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
      safeAttentionSummary.badgeCount,
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
    safeAttentionSummary.badgeCount,
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
    snapshot?.state.domain.native_attention.dispatch.kind,
    snapshot?.state.domain.native_attention.summary.candidate?.room_display_name,
    snapshot?.state.domain.native_attention.summary.candidate?.kind,
    snapshot?.state.domain.native_attention.summary.candidate?.unread_count,
    snapshot?.state.domain.native_attention.summary.candidate?.highlight_count
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
  }, [appendDiagnosticLog]);

  useEffect(() => {
    if (!desktopAttentionPort || safeAttentionSummary.badgeCount !== 0) {
      return;
    }

    void clearDesktopAttentionNotifications(desktopAttentionPort.notifications, (token) =>
      appendDiagnosticLog({ timestampMs: Date.now(), source: "native.attention", message: token })
    );
  }, [safeAttentionSummary.badgeCount]);
}
