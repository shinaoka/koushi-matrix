// @vitest-environment jsdom
import { renderHook } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";

import type {
  DesktopSnapshot,
  NativeAttentionCandidate,
  NativeAttentionState,
  NotificationSettings
} from "../domain/types";
import { useDesktopAttentionEffects } from "./useDesktopAttentionEffects";

const harness = vi.hoisted(() => ({
  show: vi.fn(async () => undefined),
  clear: vi.fn(async () => undefined),
  playAttentionSound: vi.fn(async () => "played" as const),
  setTitle: vi.fn(async () => undefined),
  requestUserAttention: vi.fn(async () => undefined)
}));

vi.mock("../backend/desktopAttentionRuntime", () => ({
  desktopAttentionPortForAccount: () => ({
    currentWindow: () => ({
      setTitle: harness.setTitle,
      setBadgeCount: harness.setTitle,
      requestUserAttention: harness.requestUserAttention
    }),
    notifications: {
      show: harness.show,
      clear: harness.clear,
      onActivated: () => () => undefined
    },
    sound: { playAttentionSound: harness.playAttentionSound },
    nativeBadge: { setBadgeCount: async () => "applied" as const }
  })
}));

const settings: NotificationSettings = {
  badges: true,
  desktop_notifications: true,
  message_previews: false,
  send_read_receipts: false,
  send_typing_notifications: true,
  sound: false
};

function candidate(roomDisplayName: string, unreadCount: number): NativeAttentionCandidate {
  return {
    room_display_name: roomDisplayName,
    kind: "message",
    unread_count: unreadCount,
    highlight_count: 0
  };
}

function attention(
  alertId: number | null,
  raisedCandidate: NativeAttentionCandidate | null
): NativeAttentionState {
  return {
    summary: {
      unread_count: 1,
      highlight_count: 0,
      badge_count: 1,
      alert_id: alertId,
      candidate: raisedCandidate,
      capabilities: {
        notifications: "available",
        badge: "available",
        overlay_icon: "unavailable",
        sound: "available",
        tray: "unknown",
        activation: "unavailable"
      }
    },
    dispatch: { kind: "idle" }
  };
}

function desktopSnapshot(nativeAttention: NativeAttentionState): DesktopSnapshot {
  return {
    state: {
      domain: {
        session: { kind: "ready" },
        native_attention: nativeAttention,
        settings: { values: { notifications: settings } }
      }
    }
  } as unknown as DesktopSnapshot;
}

function summarize(nativeAttention: NativeAttentionState) {
  return {
    unreadTotal: nativeAttention.summary.unread_count,
    badgeCount: nativeAttention.summary.badge_count,
    notificationKind: nativeAttention.summary.candidate?.kind ?? ("none" as const),
    titleHint: "1 unread",
    qaTitleToken: "unread=1 badge=1 notify=message"
  };
}

function renderAttentionHook(initialSnapshot: DesktopSnapshot) {
  return renderHook(
    ({ snapshot }: { snapshot: DesktopSnapshot }) =>
      useDesktopAttentionEffects({
        accountTabId: null,
        snapshot,
        attentionWindowTitle: "Koushi",
        safeAttentionSummary: summarize(snapshot.state.domain.native_attention),
        appendDiagnosticLog: () => undefined,
        onNotificationActivated: () => undefined
      }),
    { initialProps: { snapshot: initialSnapshot } }
  );
}

beforeEach(() => {
  harness.show.mockClear();
  harness.playAttentionSound.mockClear();
});

test("one alert shows one banner across a relabel, and a new raise shows another (#1802)", () => {
  const { rerender } = renderAttentionHook(
    desktopSnapshot(attention(7, candidate("Announcements", 1)))
  );
  expect(harness.show).toHaveBeenCalledTimes(1);

  // The same alert, relabelled: the room's display label and counts move but no
  // newer unread raise produced it. Keying on contents showed a second banner here.
  rerender({ snapshot: desktopSnapshot(attention(7, candidate("Renamed announcements", 2))) });
  expect(harness.show).toHaveBeenCalledTimes(1);

  // A genuinely new raise: a new identity, so the banner is shown again.
  rerender({ snapshot: desktopSnapshot(attention(8, candidate("Renamed announcements", 3))) });
  expect(harness.show).toHaveBeenCalledTimes(2);

  // Cleared and raised again is another new identity.
  rerender({ snapshot: desktopSnapshot(attention(null, null)) });
  rerender({ snapshot: desktopSnapshot(attention(9, candidate("Renamed announcements", 1))) });
  expect(harness.show).toHaveBeenCalledTimes(3);
});

test("a cleared alert does not leave the last alert id behind", () => {
  const { rerender } = renderAttentionHook(
    desktopSnapshot(attention(7, candidate("Announcements", 1)))
  );
  expect(harness.show).toHaveBeenCalledTimes(1);

  // Clearing and re-raising with the same identity can only happen if the identity
  // were reused; a clear must therefore drop the remembered one, so the raise that
  // follows is shown again even when Rust hands out the same number.
  rerender({ snapshot: desktopSnapshot(attention(null, null)) });
  rerender({ snapshot: desktopSnapshot(attention(7, candidate("Announcements", 1))) });
  expect(harness.show).toHaveBeenCalledTimes(2);
});
