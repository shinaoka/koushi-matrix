/* @vitest-environment jsdom */

import { afterEach, expect, test, vi } from "vitest";

test("constructs one event adapter without subscribing eagerly", async () => {
  vi.resetModules();
  const port = {
    listenCoreEvents: vi.fn(),
    listenDesktopUpdates: vi.fn(),
    listenMenuActions: vi.fn(),
    listenStateUpdates: vi.fn()
  };
  const createTauriDesktopEventPort = vi.fn(() => port);
  vi.doMock("./tauri/desktopEventPort", () => ({ createTauriDesktopEventPort }));

  await import("./desktopEventRuntime");

  expect(createTauriDesktopEventPort).toHaveBeenCalledOnce();
  expect(port.listenCoreEvents).not.toHaveBeenCalled();
  expect(port.listenDesktopUpdates).not.toHaveBeenCalled();
  expect(port.listenMenuActions).not.toHaveBeenCalled();
  expect(port.listenStateUpdates).not.toHaveBeenCalled();
});

afterEach(() => {
  vi.clearAllMocks();
});
