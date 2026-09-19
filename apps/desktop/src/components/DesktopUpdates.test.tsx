// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { DesktopUpdateState } from "../domain/types";
import { DesktopUpdates } from "./DesktopUpdates";

const fixture = vi.hoisted(() => ({
  updateListeners: new Set<(state: DesktopUpdateState) => void>(),
  menuListeners: new Set<(action: string) => void>(),
  getState: vi.fn(),
  check: vi.fn(),
  download: vi.fn(),
  restart: vi.fn(),
}));
vi.mock("../backend/runtimeEnvironment", () => ({ isTauriRuntime: () => true }));
vi.mock("../backend/appRuntime", () => ({ api: {
  getDesktopUpdateState: fixture.getState,
  checkForDesktopUpdate: fixture.check,
  downloadDesktopUpdate: fixture.download,
  restartToInstallDesktopUpdate: fixture.restart
} }));
vi.mock("../domain/appStore", () => ({
  useAppStore: () => ({
    values: { updates: { auto_check: false, include_prereleases: false } },
    persistence: { kind: "idle" }
  }),
  getAppStoreSnapshot: () => null,
  setAppStoreSnapshot: vi.fn()
}));
vi.mock("../backend/desktopEventRuntime", () => ({ desktopEventPort: {
  listenDesktopUpdates: async (listener: (state: DesktopUpdateState) => void) => {
    fixture.updateListeners.add(listener);
    return () => fixture.updateListeners.delete(listener);
  },
  listenMenuActions: async (listener: (action: string) => void) => {
    fixture.menuListeners.add(listener);
    return () => fixture.menuListeners.delete(listener);
  }
} }));

beforeEach(() => {
  fixture.getState.mockResolvedValue({ kind: "idle" });
  fixture.check.mockResolvedValue(undefined);
  fixture.download.mockResolvedValue(undefined);
  fixture.restart.mockResolvedValue(undefined);
});
afterEach(() => { cleanup(); vi.clearAllMocks(); });

test("live updater events win over an older initial read and listeners are released", async () => {
  let resolveInitial!: (state: DesktopUpdateState) => void;
  fixture.getState.mockImplementation(() => new Promise(resolve => { resolveInitial = resolve; }));
  const view = render(<DesktopUpdates />);
  await waitFor(() => expect(fixture.getState).toHaveBeenCalledOnce());
  act(() => fixture.updateListeners.forEach(listener => listener({ kind: "available", version: "1.2.4", generation: 7 })));
  await act(async () => resolveInitial({ kind: "idle" }));
  expect(screen.getByText("Koushi 1.2.4 is available.")).toBeTruthy();
  expect(fixture.download).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: /Download/ }));
  expect(fixture.download).toHaveBeenCalledWith(7);
  view.unmount();
  expect(fixture.updateListeners.size).toBe(0);
  expect(fixture.menuListeners.size).toBe(0);
});

test("repeated menu checks present one dialog without duplicating event subscriptions", async () => {
  render(<DesktopUpdates />);
  await waitFor(() => expect(fixture.getState).toHaveBeenCalledOnce());
  await act(async () => {
    for (let i = 0; i < 3; i++) fixture.menuListeners.forEach(listener => listener("checkForUpdates"));
  });
  expect(screen.getAllByRole("dialog", { name: "Software update" })).toHaveLength(1);
  expect(fixture.updateListeners.size).toBe(1);
  expect(fixture.menuListeners.size).toBe(1);
  expect(screen.getByRole("switch", { name: "Automatically check for updates" }).getAttribute("aria-checked")).toBe("false");
  expect(fixture.check).toHaveBeenCalledTimes(3);
  expect(fixture.restart).not.toHaveBeenCalled();
});

test("a failed IPC action is visible and retry does not invent an updater result", async () => {
  render(<DesktopUpdates />);
  await waitFor(() => expect(fixture.getState).toHaveBeenCalledOnce());
  fixture.check.mockRejectedValueOnce(new Error("synthetic transport failure"));
  await act(async () => fixture.menuListeners.forEach(listener => listener("checkForUpdates")));
  expect(screen.getByRole("alert").textContent).toContain("could not be completed");
  fireEvent.click(screen.getByRole("button", { name: "Check for updates" }));
  await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
  expect(screen.queryByText(/up to date/)).toBeNull();
  act(() => fixture.updateListeners.forEach(listener => listener({ kind: "unsupported" })));
  expect(screen.getByText("In-app updates are unavailable in this build.")).toBeTruthy();
});
