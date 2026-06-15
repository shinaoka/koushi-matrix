import { beforeEach, describe, expect, test, vi } from "vitest";

vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: vi.fn(),
  requestPermission: vi.fn(),
  sendNotification: vi.fn()
}));

import {
  isPermissionGranted,
  requestPermission,
  sendNotification
} from "@tauri-apps/plugin-notification";

import {
  createTauriDesktopNotificationTransport,
  desktopAttentionNotificationContent,
  sendDesktopAttentionNotification
} from "./desktopNotification";

describe("desktop notification content", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  test("builds a redacted payload from allowed attention fields only", () => {
    const payload = desktopAttentionNotificationContent({
      roomDisplayName: "Announcements",
      kind: "mention",
      unreadCount: 6,
      highlightCount: 1
    });

    expect(payload).toEqual({
      title: "Mention in Announcements",
      body: "1 mention, 6 unread"
    });
    expect(Object.keys(payload)).toEqual(["title", "body"]);
    expect(JSON.stringify(payload)).not.toContain("room_id");
    expect(JSON.stringify(payload)).not.toContain("event_id");
    expect(JSON.stringify(payload)).not.toContain("transaction_id");
    expect(JSON.stringify(payload)).not.toContain("sender");
    expect(JSON.stringify(payload)).not.toContain("secret message text");
  });

  test("omits zero-count parts from the body while preserving unread fallback", () => {
    const payload = desktopAttentionNotificationContent({
      roomDisplayName: "Direct chat",
      kind: "dm",
      unreadCount: 1,
      highlightCount: 0
    });

    expect(payload.body).toBe("1 unread");
    expect(payload.body).not.toContain("0 notifications");
    expect(payload.body).not.toContain("0 unread");
  });

  test("sends the redacted payload through a mockable adapter", async () => {
    const transport = {
      notify: vi.fn().mockResolvedValue(undefined)
    };

    await sendDesktopAttentionNotification(
      {
        roomDisplayName: "Direct chat",
        kind: "dm",
        unreadCount: 3,
        highlightCount: 0
      },
      transport
    );

    expect(transport.notify).toHaveBeenCalledOnce();
    expect(transport.notify).toHaveBeenCalledWith({
      title: "Direct message in Direct chat",
      body: "3 unread"
    });
  });

  test("swallows notification transport failures", async () => {
    const transport = {
      notify: vi.fn().mockRejectedValue(new Error("notification failed"))
    };

    await expect(
      sendDesktopAttentionNotification(
        {
          roomDisplayName: "General",
          kind: "message",
          unreadCount: 1,
          highlightCount: 0
        },
        transport
      )
    ).resolves.toBeUndefined();
    expect(transport.notify).toHaveBeenCalledOnce();
  });

  test("sends through the Tauri transport when permission is already granted", async () => {
    vi.mocked(isPermissionGranted).mockResolvedValue(true);
    vi.mocked(sendNotification).mockResolvedValue(undefined);

    await createTauriDesktopNotificationTransport().notify({
      title: "Mention in Announcements",
      body: "1 mention, 6 unread"
    });

    expect(isPermissionGranted).toHaveBeenCalledOnce();
    expect(requestPermission).not.toHaveBeenCalled();
    expect(sendNotification).toHaveBeenCalledOnce();
    expect(sendNotification).toHaveBeenCalledWith({
      title: "Mention in Announcements",
      body: "1 mention, 6 unread"
    });
  });

  test("does not prompt for notification permission during passive attention dispatch", async () => {
    vi.mocked(isPermissionGranted).mockResolvedValue(false);
    vi.mocked(requestPermission).mockResolvedValue("granted");
    vi.mocked(sendNotification).mockResolvedValue(undefined);

    await createTauriDesktopNotificationTransport().notify({
      title: "Mention in Announcements",
      body: "1 mention, 6 unread"
    });

    expect(isPermissionGranted).toHaveBeenCalledOnce();
    expect(requestPermission).not.toHaveBeenCalled();
    expect(sendNotification).not.toHaveBeenCalled();
  });

  test("skips notification delivery when permission is denied", async () => {
    vi.mocked(isPermissionGranted).mockResolvedValue(false);
    vi.mocked(requestPermission).mockResolvedValue("denied");
    vi.mocked(sendNotification).mockResolvedValue(undefined);

    await createTauriDesktopNotificationTransport().notify({
      title: "Message in General",
      body: "1 unread"
    });

    expect(isPermissionGranted).toHaveBeenCalledOnce();
    expect(requestPermission).not.toHaveBeenCalled();
    expect(sendNotification).not.toHaveBeenCalled();
  });
});
