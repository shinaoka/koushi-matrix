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
  ignore: vi.fn(),
}));
vi.mock("../backend/runtimeEnvironment", () => ({ isTauriRuntime: () => true }));
vi.mock("../backend/appRuntime", () => ({ api: {
  getDesktopUpdateState: fixture.getState,
  checkForDesktopUpdate: fixture.check,
  downloadDesktopUpdate: fixture.download,
  restartToInstallDesktopUpdate: fixture.restart,
  ignoreDesktopUpdate: fixture.ignore
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
  fixture.ignore.mockResolvedValue(undefined);
});
afterEach(() => { cleanup(); vi.clearAllMocks(); });

test("live updater events win over an older initial read and listeners are released", async () => {
  let resolveInitial!: (state: DesktopUpdateState) => void;
  fixture.getState.mockImplementation(() => new Promise(resolve => { resolveInitial = resolve; }));
  const view = render(<DesktopUpdates />);
  await waitFor(() => expect(fixture.getState).toHaveBeenCalledOnce());
  act(() => fixture.updateListeners.forEach(listener => listener({ kind: "available", version: "1.2.4", generation: 7, ignored: false, check_failed: false })));
  await act(async () => resolveInitial({ kind: "idle" }));
  expect(screen.getByText("Koushi 1.2.4 is available.")).toBeTruthy();
  expect(fixture.download).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: /Download/ }));
  expect(fixture.download).toHaveBeenCalledWith(7);
  view.unmount();
  expect(fixture.updateListeners.size).toBe(0);
  expect(fixture.menuListeners.size).toBe(0);
});

test("a dismissed offer is presented again when a later check re-offers it", async () => {
  render(<DesktopUpdates />);
  await waitFor(() => expect(fixture.getState).toHaveBeenCalledOnce());
  act(() => fixture.updateListeners.forEach(listener => listener({ kind: "available", version: "1.2.4", generation: 7, ignored: false, check_failed: false })));
  expect(screen.getByRole("dialog", { name: "Software update" })).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Close Software update" }));
  expect(screen.queryByRole("dialog", { name: "Software update" })).toBeNull();
  act(() => fixture.updateListeners.forEach(listener => listener({ kind: "available", version: "1.2.4", generation: 8, ignored: false, check_failed: false })));
  expect(screen.getByRole("dialog", { name: "Software update" })).toBeTruthy();
});

test("an explicitly ignored version stays dismissed while a newer one is presented", async () => {
  render(<DesktopUpdates />);
  await waitFor(() => expect(fixture.getState).toHaveBeenCalledOnce());
  act(() => fixture.updateListeners.forEach(listener => listener({ kind: "available", version: "1.2.4", generation: 7, ignored: true, check_failed: false })));
  expect(screen.queryByRole("dialog", { name: "Software update" })).toBeNull();
  act(() => fixture.updateListeners.forEach(listener => listener({ kind: "available", version: "1.3.0", generation: 8, ignored: false, check_failed: false })));
  expect(screen.getByRole("dialog", { name: "Software update" })).toBeTruthy();
});

test("an ignored offer shows reminders are off and keeps Download available", async () => {
  render(<DesktopUpdates />);
  await waitFor(() => expect(fixture.getState).toHaveBeenCalledOnce());
  // Revisit the ignored offer through the manual menu action.
  await act(async () => fixture.menuListeners.forEach(listener => listener("checkForUpdates")));
  act(() => fixture.updateListeners.forEach(listener => listener({ kind: "available", version: "1.2.4", generation: 7, ignored: true, check_failed: false })));
  expect(screen.getByText("Automatic reminders are off for this version until Koushi restarts. You can still download it.")).toBeTruthy();
  expect(screen.getByRole("button", { name: "Download update" })).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Ignore this version" })).toBeNull();
});

test("a retained offer after a failed refresh reports the failure and stays downloadable", async () => {
  render(<DesktopUpdates />);
  await waitFor(() => expect(fixture.getState).toHaveBeenCalledOnce());
  act(() => fixture.updateListeners.forEach(listener => listener({ kind: "available", version: "1.2.4", generation: 7, ignored: false, check_failed: false })));
  fireEvent.click(screen.getByRole("button", { name: "Close Software update" }));
  act(() => fixture.updateListeners.forEach(listener => listener({ kind: "available", version: "1.2.4", generation: 8, ignored: false, check_failed: true })));
  expect(screen.getByRole("dialog", { name: "Software update" })).toBeTruthy();
  expect(screen.getByRole("alert").textContent).toContain("could not check");
  fireEvent.click(screen.getByRole("button", { name: "Download update" }));
  expect(fixture.download).toHaveBeenCalledWith(8);
});

test("the ignore action dispatches the exact offered version and closes the dialog", async () => {
  render(<DesktopUpdates />);
  await waitFor(() => expect(fixture.getState).toHaveBeenCalledOnce());
  act(() => fixture.updateListeners.forEach(listener => listener({ kind: "available", version: "1.2.4", generation: 7, ignored: false, check_failed: false })));
  fireEvent.click(screen.getByRole("button", { name: "Ignore this version" }));
  expect(fixture.ignore).toHaveBeenCalledWith("1.2.4");
  await waitFor(() => expect(screen.queryByRole("dialog", { name: "Software update" })).toBeNull());
});

test("a failed ignore keeps the dialog open with the transport failure", async () => {
  render(<DesktopUpdates />);
  await waitFor(() => expect(fixture.getState).toHaveBeenCalledOnce());
  act(() => fixture.updateListeners.forEach(listener => listener({ kind: "available", version: "1.2.4", generation: 7, ignored: false, check_failed: false })));
  fixture.ignore.mockRejectedValueOnce(new Error("synthetic transport failure"));
  fireEvent.click(screen.getByRole("button", { name: "Ignore this version" }));
  await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("could not be completed"));
  expect(screen.getByRole("dialog", { name: "Software update" })).toBeTruthy();
});

test("a stale ignore completion does not dismiss a newer offer", async () => {
  let resolveIgnore!: () => void;
  fixture.ignore.mockImplementationOnce(() => new Promise<void>(resolve => { resolveIgnore = resolve; }));
  render(<DesktopUpdates />);
  await waitFor(() => expect(fixture.getState).toHaveBeenCalledOnce());
  act(() => fixture.updateListeners.forEach(listener => listener({ kind: "available", version: "1.2.4", generation: 7, ignored: false, check_failed: false })));
  fireEvent.click(screen.getByRole("button", { name: "Ignore this version" }));
  // A newer offer replaces the dialog while the ignore IPC is still pending.
  act(() => fixture.updateListeners.forEach(listener => listener({ kind: "available", version: "1.3.0", generation: 8, ignored: false, check_failed: false })));
  await act(async () => resolveIgnore());
  expect(screen.getByRole("dialog", { name: "Software update" })).toBeTruthy();
  expect(screen.getByText("Koushi 1.3.0 is available.")).toBeTruthy();
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
  act(() => fixture.updateListeners.forEach(listener => listener({ kind: "unsupported", reason: "build" })));
  expect(screen.getByText("In-app updates are unavailable in this build.")).toBeTruthy();
});

test("a package-managed installation renders the packager opt-out reason without update actions", async () => {
  render(<DesktopUpdates />);
  await waitFor(() => expect(fixture.getState).toHaveBeenCalledOnce());
  await act(async () => fixture.menuListeners.forEach(listener => listener("checkForUpdates")));
  act(() => fixture.updateListeners.forEach(listener => listener({ kind: "unsupported", reason: "package_managed" })));
  expect(screen.getByText("Updates for this installation are provided by your package manager.")).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Download update" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Check for updates" })).toBeNull();
  expect((screen.getByRole("switch", { name: "Automatically check for updates" }) as HTMLButtonElement).disabled).toBe(true);
});
