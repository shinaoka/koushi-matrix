import { describe, expect, test, vi } from "vitest";

import {
  applyDesktopAttentionToWindow,
  desktopAttentionNotificationCandidate,
  desktopAttentionSummary,
  desktopAttentionWindowTitle
} from "./desktopAttention";

describe("desktop attention summary", () => {
  test("computes unread totals, badge counts, title hints, and QA tokens from room attention", () => {
    const summary = desktopAttentionSummary({
      activeRoomId: null,
      rooms: [
        {
          room_id: "!dm-alerts:example.invalid",
          display_name: "Direct chat",
          is_dm: true,
          unread_count: 0,
          notification_count: 2,
          highlight_count: 0
        },
        {
          room_id: "!room-announcements:example.invalid",
          display_name: "Announcements",
          is_dm: false,
          unread_count: 3,
          highlight_count: 1
        },
        {
          room_id: "!room-updates:example.invalid",
          display_name: "Updates",
          is_dm: false,
          unread_count: 1
        }
      ]
    });

    expect(summary).toEqual({
      unreadTotal: 6,
      badgeCount: 6,
      notificationKind: "mention",
      titleHint: "6 unread",
      qaTitleToken: "unread=6 badge=6 notify=mention"
    });
    expect(summary.qaTitleToken).not.toContain("Direct chat");
    expect(summary.qaTitleToken).not.toContain("Announcements");
  });

  test("falls back to unread counts when notification metadata is zero", () => {
    const summary = desktopAttentionSummary({
      activeRoomId: null,
      rooms: [
        {
          room_id: "!room-fallback:example.invalid",
          display_name: "Fallback room",
          is_dm: false,
          unread_count: 3,
          notification_count: 0,
          highlight_count: 0
        }
      ]
    });

    expect(summary).toEqual({
      unreadTotal: 3,
      badgeCount: 3,
      notificationKind: "message",
      titleHint: "3 unread",
      qaTitleToken: "unread=3 badge=3 notify=message"
    });
  });

  test("turns a summary into a human readable window title", () => {
    const summary = desktopAttentionSummary({
      activeRoomId: null,
      rooms: [
        {
          room_id: "!room-one:example.invalid",
          display_name: "Room One",
          is_dm: false,
          unread_count: 4
        }
      ]
    });

    expect(desktopAttentionWindowTitle("matrix-desktop", summary)).toBe(
      "matrix-desktop · 4 unread"
    );
  });
});

describe("desktop notification candidate", () => {
  test("prefers mention changes over DM and message changes", () => {
    const previous = {
      activeRoomId: null,
      rooms: [
        {
          room_id: "!room-mention:example.invalid",
          display_name: "Announcements",
          is_dm: false,
          unread_count: 0,
          highlight_count: 0
        },
        {
          room_id: "!dm-room:example.invalid",
          display_name: "Direct chat",
          is_dm: true,
          unread_count: 0,
          notification_count: 0
        },
        {
          room_id: "!room-message:example.invalid",
          display_name: "General",
          is_dm: false,
          unread_count: 0
        }
      ]
    };
    const current = {
      activeRoomId: null,
      rooms: [
        {
          room_id: "!room-mention:example.invalid",
          display_name: "Announcements",
          is_dm: false,
          unread_count: 3,
          highlight_count: 1
        },
        {
          room_id: "!dm-room:example.invalid",
          display_name: "Direct chat",
          is_dm: true,
          unread_count: 2,
          notification_count: 2
        },
        {
          room_id: "!room-message:example.invalid",
          display_name: "General",
          is_dm: false,
          unread_count: 1
        }
      ]
    };

    expect(desktopAttentionNotificationCandidate(current, previous)).toEqual({
      roomDisplayName: "Announcements",
      kind: "mention",
      unreadCount: 3,
      notificationCount: 0,
      highlightCount: 1
    });
  });

  test("suppresses notification candidates for the focused room", () => {
    const previous = {
      activeRoomId: null,
      rooms: [
        {
          room_id: "!room-focused:example.invalid",
          display_name: "Focused room",
          is_dm: false,
          unread_count: 0,
          highlight_count: 0
        }
      ]
    };
    const current = {
      activeRoomId: "!room-focused:example.invalid",
      rooms: [
        {
          room_id: "!room-focused:example.invalid",
          display_name: "Focused room",
          is_dm: false,
          unread_count: 2,
          highlight_count: 1
        }
      ]
    };

    expect(desktopAttentionNotificationCandidate(current, previous)).toBeNull();
  });

  test("applies the derived title and badge count to a window-like adapter", async () => {
    const windowMock = {
      setTitle: vi.fn().mockResolvedValue(undefined),
      setBadgeCount: vi.fn().mockResolvedValue(undefined)
    };
    const summary = desktopAttentionSummary({
      activeRoomId: null,
      rooms: [
        {
          room_id: "!room-one:example.invalid",
          display_name: "Room One",
          is_dm: false,
          unread_count: 5
        }
      ]
    });

    await applyDesktopAttentionToWindow(
      windowMock,
      desktopAttentionWindowTitle("matrix-desktop", summary),
      summary.badgeCount
    );

    expect(windowMock.setTitle).toHaveBeenCalledWith("matrix-desktop · 5 unread");
    expect(windowMock.setBadgeCount).toHaveBeenCalledWith(5);
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
