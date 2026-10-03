import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { cancelAll, removeAllActive } from "@tauri-apps/plugin-notification";

import type { DesktopAttentionPort } from "../desktopAttentionPort";
import { isRendererSelectedAccountTabId } from "../client";
import type { DesktopNotificationActivation } from "../../domain/desktopNotification";

/** Tauri event emitted by the Rust adapter after a notification click. */
const NOTIFICATION_ACTIVATED_EVENT_NAME = "koushi-desktop://notification-activated";

export function createTauriDesktopAttentionPort(accountTabId?: string): DesktopAttentionPort {
  const invokeAccount = <T>(command: string): Promise<T> => {
    if (accountTabId !== undefined && !isRendererSelectedAccountTabId(accountTabId)) {
      return Promise.reject(new Error("account tab is no longer selected"));
    }
    return accountTabId === undefined
      ? invoke<T>(command)
      : invoke<T>(command, { accountTabId });
  };
  return {
    currentWindow: getCurrentWindow,
    notifications: {
      async show() {
        await invokeAccount("show_native_attention_notification");
      },
      async clear() {
        // The plugin's desktop backend registers only notify/request_permission;
        // its cancel APIs are mobile-only, so this resolves as a rejection on
        // desktop and is reported as such. Replacing it needs a native
        // clear path (see the notification compatibility work in #67).
        const outcomes = await Promise.allSettled([cancelAll(), removeAllActive()]);
        if (outcomes.some((outcome) => outcome.status === "rejected")) {
          throw new Error("native_notification_clear_failed");
        }
      },
      onActivated(handler) {
        let disposed = false;
        let unlisten: (() => void) | null = null;
        void listen<DesktopNotificationActivation>(NOTIFICATION_ACTIVATED_EVENT_NAME, (event) => {
          if (!disposed) {
            handler(event.payload);
          }
        })
          .then((stop) => {
            if (disposed) {
              stop();
            } else {
              unlisten = stop;
            }
          })
          .catch(() => undefined);
        return () => {
          disposed = true;
          unlisten?.();
          unlisten = null;
        };
      }
    },
    sound: {
      playAttentionSound: () =>
        invokeAccount<"played" | "unsupported" | "failed" | "skipped">(
          "play_native_attention_sound"
        )
    },
    nativeBadge: {
      setBadgeCount: (count?: number) =>
        invoke<"applied" | "unsupported" | "mismatch">("set_native_attention_badge", {
          count
        })
    }
  };
}
