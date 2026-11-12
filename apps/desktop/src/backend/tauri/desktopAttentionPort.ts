import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import type { DesktopAttentionPort } from "../desktopAttentionPort";
import { isRendererSelectedAccountTabId } from "../client";
import type {
  DesktopNotificationActivation,
  NotificationAuthorization
} from "../../domain/desktopNotification";

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
      requestPermission: () => invoke<boolean>("request_native_attention_notification_permission"),
      authorization: () =>
        invoke<NotificationAuthorization>("native_attention_notification_authorization"),
      openSystemSettings: () =>
        invoke<void>("open_native_attention_notification_settings"),
      async clear() {
        await invokeAccount("clear_native_attention_notifications");
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
