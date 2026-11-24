import { useCallback, useEffect, useMemo, useRef, useState } from "react";

// Platform-lifecycle seam for the OS notification grant (#1868). The persisted
// Desktop notifications switch is Rust-owned; this hook only reports and
// requests what the operating system will actually deliver.
import { desktopAttentionPortForAccount } from "../backend/desktopAttentionRuntime";
import type {
  NotificationAuthorization,
  NotificationPermission
} from "../domain/desktopNotification";

/**
 * One permission request per launch, never per render, mount, or account switch.
 *
 * macOS prompts at most once per install, so asking again cannot help the user
 * and can double-prompt when the startup path and the switch path race. The one
 * exception is an install that is already granted: it is asked once per launch
 * so a newly requested option (the Dock badge, #2095) can be added to the grant.
 */
let launchRequestSpent = false;

/** Spend this launch's permission request; false when it was already spent. */
export function claimLaunchNotificationRequest(): boolean {
  if (launchRequestSpent) {
    return false;
  }
  launchRequestSpent = true;
  return true;
}

export function resetNotificationPermissionStartupForTests(): void {
  launchRequestSpent = false;
}

type NotificationPermissionInput = {
  accountTabId: string | null;
  /** Whether this device's Desktop notifications switch is on. */
  desktopNotificationsEnabled: boolean;
  /** Whether the Settings dialog is showing, which re-reads the OS state. */
  settingsOpen: boolean;
};

export function useNotificationPermission({
  accountTabId,
  desktopNotificationsEnabled,
  settingsOpen
}: NotificationPermissionInput): NotificationPermission {
  const port = useMemo(
    () => desktopAttentionPortForAccount(accountTabId ?? undefined),
    [accountTabId]
  );
  const notifications = port?.notifications;
  const canAsk = notifications?.requestPermission !== undefined;
  const [authorization, setAuthorization] = useState<NotificationAuthorization | null>(null);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  // Latest read wins: an answer overtaken by a newer request or refresh is dropped
  // instead of overwriting fresher OS state.
  const readSequence = useRef(0);
  const read = useCallback(async () => {
    const sequence = ++readSequence.current;
    let state: NotificationAuthorization | null = null;
    if (notifications?.authorization) {
      try {
        state = await notifications.authorization();
      } catch {
        state = null;
      }
    }
    return sequence === readSequence.current ? state : undefined;
  }, [notifications]);

  const refresh = useCallback(() => {
    void read().then((state) => {
      if (state !== undefined && mounted.current) {
        setAuthorization(state);
      }
    });
  }, [read]);

  const request = useCallback(() => {
    launchRequestSpent = true;
    void (async () => {
      try {
        await notifications?.requestPermission?.();
      } finally {
        const state = await read();
        if (state !== undefined && mounted.current) {
          setAuthorization(state);
        }
      }
    })();
  }, [notifications, read]);

  const openSystemSettings = useCallback(() => {
    void notifications?.openSystemSettings?.().catch(() => undefined);
  }, [notifications]);

  // The migrated install: the switch is already ON, so the off→on request path
  // never runs. Ask once instead, and only where the OS can still answer.
  useEffect(() => {
    if (!desktopNotificationsEnabled || !canAsk || !claimLaunchNotificationRequest()) {
      return;
    }
    void (async () => {
      const state = await read();
      // An install that granted before the request included the badge option has
      // no *Badge application icon* switch, so the Dock badge never shows (#2095).
      // Asking again adds the missing option; macOS does not decide a grant twice
      // and a denial is never retried here.
      if (state === "not_determined" || state === "authorized") {
        request();
        return;
      }
      if (state !== undefined && mounted.current) {
        setAuthorization(state);
      }
    })();
  }, [canAsk, desktopNotificationsEnabled, read, request]);

  // Re-read whenever Settings opens: the user may have changed the grant in the
  // OS while Koushi kept running.
  useEffect(() => {
    if (settingsOpen) {
      refresh();
    }
  }, [refresh, settingsOpen]);

  return { authorization, request, openSystemSettings, refresh };
}
