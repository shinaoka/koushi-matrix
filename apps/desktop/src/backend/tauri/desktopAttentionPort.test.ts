/* @vitest-environment jsdom */

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { cancelAll, removeAllActive } from "@tauri-apps/plugin-notification";
import { beforeEach, describe, expect, test, vi } from "vitest";

import type { DesktopNotificationActivation } from "../../domain/desktopNotification";
import { setRendererSelectedAccountTabId } from "../client";
import { createTauriDesktopAttentionPort } from "./desktopAttentionPort";

const currentWindow = vi.hoisted(() => ({
  setTitle: vi.fn(async () => undefined),
  setBadgeCount: vi.fn(async () => undefined),
  requestUserAttention: vi.fn(async () => undefined)
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: vi.fn(() => currentWindow) }));
vi.mock("@tauri-apps/plugin-notification", () => ({
  cancelAll: vi.fn(),
  removeAllActive: vi.fn()
}));

const unlisten = vi.fn();

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(listen).mockResolvedValue(unlisten);
  vi.mocked(cancelAll).mockResolvedValue(undefined);
  vi.mocked(removeAllActive).mockResolvedValue(undefined);
  vi.mocked(invoke).mockResolvedValue("delivered");
});

describe("Tauri desktop attention port", () => {
  test("acquires the current window and preserves native command contracts", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce("applied")
      .mockResolvedValueOnce("played");
    const port = createTauriDesktopAttentionPort();

    expect(port.currentWindow()).toBe(currentWindow);
    expect(getCurrentWindow).toHaveBeenCalledOnce();
    await expect(port.nativeBadge.setBadgeCount(3)).resolves.toBe("applied");
    expect(invoke).toHaveBeenNthCalledWith(1, "set_native_attention_badge", { count: 3 });
    await expect(port.sound.playAttentionSound?.()).resolves.toBe("played");
    expect(invoke).toHaveBeenNthCalledWith(2, "play_native_attention_sound");
  });

  test("binds notification and sound commands to their account tab", async () => {
    setRendererSelectedAccountTabId("account:alice");
    const port = createTauriDesktopAttentionPort("account:alice");

    await port.notifications.show();
    await port.sound.playAttentionSound?.();
    expect(invoke).toHaveBeenNthCalledWith(1, "show_native_attention_notification", {
      accountTabId: "account:alice"
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "play_native_attention_sound", {
      accountTabId: "account:alice"
    });

    setRendererSelectedAccountTabId("account:bob");
    await expect(port.notifications.show()).rejects.toThrow("account tab is no longer selected");
    await expect(port.sound.playAttentionSound?.()).rejects.toThrow(
      "account tab is no longer selected"
    );
    expect(invoke).toHaveBeenCalledTimes(2);
  });

  // Rust composes the banner, so the webview command carries no payload at all.
  test("asks Rust to show the notification it owns", async () => {
    const port = createTauriDesktopAttentionPort();

    await port.notifications.show();

    expect(invoke).toHaveBeenCalledOnce();
    expect(invoke).toHaveBeenCalledWith("show_native_attention_notification");
  });

  test("delivers activation events and stops listening after unsubscribe", async () => {
    const port = createTauriDesktopAttentionPort();
    const handler = vi.fn<(activation: DesktopNotificationActivation) => void>();
    const stop = port.notifications.onActivated(handler);
    await vi.waitFor(() => expect(listen).toHaveBeenCalledOnce());

    const [eventName, listener] = vi.mocked(listen).mock.calls[0];
    expect(eventName).toBe("koushi-desktop://notification-activated");
    const activation: DesktopNotificationActivation = {
      account_tab_id: "account:@alice:example.invalid",
      room_id: "!room:example.invalid",
      event_id: "$event:example.invalid",
      thread_root_event_id: null
    };
    listener({ event: eventName, id: 1, payload: activation });
    expect(handler).toHaveBeenCalledWith(activation);

    stop();
    expect(unlisten).toHaveBeenCalledOnce();
    listener({ event: eventName, id: 2, payload: activation });
    expect(handler).toHaveBeenCalledOnce();
  });

  test.each([
    ["pending cancellation", () => vi.mocked(cancelAll).mockRejectedValue(new Error("failed"))],
    ["active removal", () => vi.mocked(removeAllActive).mockRejectedValue(new Error("failed"))]
  ])("settles both clear operations when %s fails", async (_operation, rejectOperation) => {
    rejectOperation();
    const port = createTauriDesktopAttentionPort();

    await expect(port.notifications.clear()).rejects.toThrow("native_notification_clear_failed");
    expect(cancelAll).toHaveBeenCalledOnce();
    expect(removeAllActive).toHaveBeenCalledOnce();
  });
});
