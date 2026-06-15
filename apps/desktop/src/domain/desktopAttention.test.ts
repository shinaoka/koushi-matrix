import { describe, expect, test, vi } from "vitest";

import {
  applyDesktopAttentionToWindow,
  desktopAttentionNotificationCandidate,
  desktopAttentionSummary,
  desktopAttentionWindowTitle,
  WINDOWS_ATTENTION_OVERLAY_ICON_PATH
} from "./desktopAttention";
import type { NativeAttentionState } from "./types";

function nativeAttentionState(
  partial: Partial<NativeAttentionState["summary"]> = {},
  dispatch: NativeAttentionState["dispatch"] = { kind: "idle" }
): NativeAttentionState {
  return {
    summary: {
      unread_count: 0,
      highlight_count: 0,
      badge_count: 0,
      candidate: null,
      capabilities: {
        notifications: "unknown",
        badge: "unknown",
        overlay_icon: "unknown",
        sound: "unknown",
        tray: "unknown",
        activation: "unknown"
      },
      ...partial
    },
    dispatch
  };
}

describe("desktop attention summary", () => {
  test("projects window attention from Rust-owned native attention state", () => {
    const summary = desktopAttentionSummary(
      nativeAttentionState({
        unread_count: 6,
        highlight_count: 1,
        badge_count: 4,
        candidate: {
          room_display_name: "Announcements",
          kind: "mention",
          unread_count: 3,
          highlight_count: 1
        }
      })
    );

    expect(summary).toEqual({
      unreadTotal: 6,
      badgeCount: 4,
      notificationKind: "mention",
      titleHint: "6 unread",
      qaTitleToken: "unread=6 badge=4 notify=mention"
    });
    expect(summary.qaTitleToken).not.toContain("Announcements");
  });

  test("renders no notification intent when Rust suppresses or clears the candidate", () => {
    const summary = desktopAttentionSummary(
      nativeAttentionState(
        {
          unread_count: 3,
          highlight_count: 1,
          badge_count: 3,
          candidate: null
        },
        { kind: "suppressed", reason: "windowFocused" }
      )
    );

    expect(summary).toEqual({
      unreadTotal: 3,
      badgeCount: 3,
      notificationKind: "none",
      titleHint: "3 unread",
      qaTitleToken: "unread=3 badge=3 notify=none"
    });
  });

  test("turns a summary into a human readable window title", () => {
    const summary = desktopAttentionSummary(
      nativeAttentionState({
        unread_count: 4,
        badge_count: 4,
        candidate: {
          room_display_name: "Room One",
          kind: "message",
          unread_count: 4,
          highlight_count: 0
        }
      })
    );

    expect(desktopAttentionWindowTitle("matrix-desktop", summary)).toBe(
      "matrix-desktop · 4 unread"
    );
  });
});

describe("desktop notification candidate", () => {
  test("uses the Rust-owned native attention candidate without React room diffing", () => {
    expect(
      desktopAttentionNotificationCandidate(
        nativeAttentionState({
          unread_count: 6,
          highlight_count: 1,
          badge_count: 6,
          candidate: {
            room_display_name: "Announcements",
            kind: "mention",
            unread_count: 3,
            highlight_count: 1
          }
        })
      )
    ).toEqual({
      roomDisplayName: "Announcements",
      kind: "mention",
      unreadCount: 3,
      highlightCount: 1
    });
  });

  test("does not dispatch when Rust state marks the candidate non-idle", () => {
    expect(
      desktopAttentionNotificationCandidate(
        nativeAttentionState(
          {
            unread_count: 2,
            highlight_count: 1,
            badge_count: 2,
            candidate: {
              room_display_name: "Focused room",
              kind: "mention",
              unread_count: 2,
              highlight_count: 1
            }
          },
          { kind: "suppressed", reason: "windowFocused" }
        )
      )
    ).toBeNull();
  });

  test("applies the derived title and badge count to a window-like adapter", async () => {
    const windowMock = {
      setTitle: vi.fn().mockResolvedValue(undefined),
      setBadgeCount: vi.fn().mockResolvedValue(undefined)
    };
    const summary = desktopAttentionSummary(
      nativeAttentionState({
        unread_count: 5,
        badge_count: 5,
        candidate: {
          room_display_name: "Room One",
          kind: "message",
          unread_count: 5,
          highlight_count: 0
        }
      })
    );

    await applyDesktopAttentionToWindow(
      windowMock,
      desktopAttentionWindowTitle("matrix-desktop", summary),
      summary.badgeCount
    );

    expect(windowMock.setTitle).toHaveBeenCalledWith("matrix-desktop · 5 unread");
    expect(windowMock.setBadgeCount).toHaveBeenCalledWith(5);
  });

  test("routes Windows overlay icon through the native attention capability DTO", async () => {
    const windowMock = {
      setTitle: vi.fn().mockResolvedValue(undefined),
      setBadgeCount: vi.fn().mockResolvedValue(undefined),
      setOverlayIcon: vi.fn().mockResolvedValue(undefined)
    };

    await applyDesktopAttentionToWindow(windowMock, "matrix-desktop · 3 unread", 3, {
      notifications: "available",
      badge: "unknown",
      overlay_icon: "available",
      sound: "available",
      tray: "unknown",
      activation: "unknown"
    });

    expect(windowMock.setOverlayIcon).toHaveBeenCalledWith(WINDOWS_ATTENTION_OVERLAY_ICON_PATH);
  });

  test("clears Windows overlay icon through the native attention capability DTO", async () => {
    const windowMock = {
      setTitle: vi.fn().mockResolvedValue(undefined),
      setBadgeCount: vi.fn().mockResolvedValue(undefined),
      setOverlayIcon: vi.fn().mockResolvedValue(undefined)
    };

    await applyDesktopAttentionToWindow(windowMock, "matrix-desktop", 0, {
      notifications: "available",
      badge: "unknown",
      overlay_icon: "available",
      sound: "available",
      tray: "unknown",
      activation: "unknown"
    });

    expect(windowMock.setOverlayIcon).toHaveBeenCalledWith(undefined);
  });

  test("swallows native title and badge failures without rejecting", async () => {
    const windowMock = {
      setTitle: vi.fn().mockRejectedValue(new Error("title failed")),
      setBadgeCount: vi.fn().mockRejectedValue(new Error("badge failed"))
    };

    await expect(
      applyDesktopAttentionToWindow(windowMock, "matrix-desktop", 2)
    ).resolves.toBeUndefined();

    expect(windowMock.setTitle).toHaveBeenCalledWith("matrix-desktop");
    expect(windowMock.setBadgeCount).toHaveBeenCalledWith(2);
  });
});
