// @vitest-environment jsdom
import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";

import type { NotificationAuthorization } from "../domain/desktopNotification";
import {
  claimLaunchNotificationRequest,
  resetNotificationPermissionStartupForTests,
  useNotificationPermission
} from "./useNotificationPermission";

const harness = vi.hoisted(() => ({
  authorization: vi.fn(async () => "not_determined" as NotificationAuthorization),
  requestPermission: vi.fn(async () => true),
  openSystemSettings: vi.fn(async () => undefined)
}));

vi.mock("../backend/desktopAttentionRuntime", () => ({
  desktopAttentionPortForAccount: () => ({
    currentWindow: () => ({
      setTitle: async () => undefined,
      setBadgeCount: async () => undefined,
      requestUserAttention: async () => undefined
    }),
    notifications: {
      show: async () => undefined,
      clear: async () => undefined,
      onActivated: () => () => undefined,
      authorization: harness.authorization,
      requestPermission: harness.requestPermission,
      openSystemSettings: harness.openSystemSettings
    },
    sound: { playAttentionSound: async () => "played" as const },
    nativeBadge: { setBadgeCount: async () => "applied" as const }
  })
}));

function renderPermission(options?: { enabled?: boolean; settingsOpen?: boolean }) {
  return renderHook(
    ({ enabled, settingsOpen }: { enabled: boolean; settingsOpen: boolean }) =>
      useNotificationPermission({
        accountTabId: "account:@demo-user:example.invalid",
        desktopNotificationsEnabled: enabled,
        settingsOpen
      }),
    {
      initialProps: {
        enabled: options?.enabled ?? true,
        settingsOpen: options?.settingsOpen ?? false
      }
    }
  );
}

beforeEach(() => {
  resetNotificationPermissionStartupForTests();
  harness.authorization.mockReset();
  harness.requestPermission.mockReset();
  harness.openSystemSettings.mockReset();
  harness.authorization.mockResolvedValue("not_determined");
  harness.requestPermission.mockResolvedValue(true);
  harness.openSystemSettings.mockResolvedValue(undefined);
});

test("asks the OS once at startup when the switch is on and macOS never decided", async () => {
  // The migration case: the persisted switch is already ON, so the off→on
  // request never runs; startup is the only place that can still ask.
  harness.authorization
    .mockResolvedValueOnce("not_determined")
    .mockResolvedValue("authorized");
  const { result, rerender } = renderPermission();

  await waitFor(() => expect(result.current.authorization).toBe("authorized"));
  expect(harness.requestPermission).toHaveBeenCalledTimes(1);

  // Re-renders and repeated snapshots must not ask again.
  rerender({ enabled: true, settingsOpen: false });
  rerender({ enabled: true, settingsOpen: false });
  expect(result.current.authorization).toBe("authorized");
  expect(harness.requestPermission).toHaveBeenCalledTimes(1);
});

test("a second mount in the same launch does not ask again", async () => {
  harness.authorization.mockResolvedValue("authorized");
  renderPermission();
  await waitFor(() => expect(harness.authorization).toHaveBeenCalledTimes(1));

  // A remount (account switch) is not a new launch.
  renderPermission();
  await waitFor(() => expect(harness.authorization).toHaveBeenCalledTimes(1));
  expect(harness.requestPermission).not.toHaveBeenCalled();
});

test("does not ask when macOS already denied; the UI offers System Settings instead", async () => {
  harness.authorization.mockResolvedValue("denied");
  const { result } = renderPermission();

  await waitFor(() => expect(result.current.authorization).toBe("denied"));
  expect(harness.requestPermission).not.toHaveBeenCalled();
});

test.each(["unsupported", "unavailable"] as const)(
  "never asks where %s cannot authorize",
  async (state) => {
    harness.authorization.mockResolvedValue(state);
    const { result } = renderPermission();

    await waitFor(() => expect(result.current.authorization).toBe(state));
    expect(harness.requestPermission).not.toHaveBeenCalled();
  }
);

test("does not query or ask while Desktop notifications is off", async () => {
  const { result } = renderPermission({ enabled: false });

  await waitFor(() => expect(result.current.authorization).toBeNull());
  expect(harness.authorization).not.toHaveBeenCalled();
  expect(harness.requestPermission).not.toHaveBeenCalled();
});

test("the explicit switch ON spends the launch-time request instead of prompting twice", async () => {
  const { rerender } = renderPermission({ enabled: false });
  await waitFor(() => expect(harness.authorization).not.toHaveBeenCalled());

  // App.tsx asks on the switch itself; the startup path must stay out of the way.
  expect(claimLaunchNotificationRequest()).toBe(true);
  rerender({ enabled: true, settingsOpen: false });

  await waitFor(() => expect(claimLaunchNotificationRequest()).toBe(false));
  expect(harness.requestPermission).not.toHaveBeenCalled();
});

test("re-queries when the settings dialog opens", async () => {
  harness.authorization.mockResolvedValue("authorized");
  const { result, rerender } = renderPermission();
  await waitFor(() => expect(harness.authorization).toHaveBeenCalledTimes(1));

  rerender({ enabled: true, settingsOpen: true });
  await waitFor(() => expect(harness.authorization).toHaveBeenCalledTimes(2));
  expect(result.current.authorization).toBe("authorized");
  expect(harness.requestPermission).not.toHaveBeenCalled();
});

test("the explicit request asks the OS and re-queries the state", async () => {
  const { result } = renderPermission({ enabled: false });
  await waitFor(() => expect(result.current.authorization).toBeNull());

  harness.authorization.mockResolvedValue("denied");
  result.current.request();

  await waitFor(() => expect(harness.requestPermission).toHaveBeenCalledTimes(1));
  await waitFor(() => expect(result.current.authorization).toBe("denied"));
});

test("opens the OS notification settings", async () => {
  harness.authorization.mockResolvedValue("denied");
  const { result } = renderPermission();
  await waitFor(() => expect(result.current.authorization).toBe("denied"));

  result.current.openSystemSettings();
  await waitFor(() => expect(harness.openSystemSettings).toHaveBeenCalledTimes(1));
});

test("refresh re-reads the state without asking", async () => {
  harness.authorization.mockResolvedValue("denied");
  const { result } = renderPermission();
  await waitFor(() => expect(result.current.authorization).toBe("denied"));

  harness.authorization.mockResolvedValue("authorized");
  result.current.refresh();

  await waitFor(() => expect(result.current.authorization).toBe("authorized"));
  expect(harness.requestPermission).not.toHaveBeenCalled();
});
