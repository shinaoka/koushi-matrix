/* @vitest-environment jsdom */

import { readFileSync } from "node:fs";
import { URL as NodeURL } from "node:url";
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { afterEach, expect, test } from "vitest";

import { shortcutIdForKeyboardEvent } from "../../domain/shortcuts";
import { listenForAppShortcuts } from "../../app/keyboardShortcuts";
import { createTauriWindowDialogPort } from "./windowDialogPort";

const capability = JSON.parse(readFileSync(
  new NodeURL("../../../src-tauri/capabilities/default.json", import.meta.url), "utf8"
)) as { permissions: string[] };

afterEach(() => {
  clearMocks();
  document.documentElement.style.removeProperty("--webview-zoom");
});

test("Cmd+Ctrl+F can enter and leave fullscreen through the permitted window IPC", async () => {
  mockWindows("main");
  let fullscreen = false;
  const calls: string[] = [];
  mockIPC((command, args) => {
    calls.push(command);
    if (command === "plugin:window|is_fullscreen") {
      // is_fullscreen belongs to core:default; the setter requires an opt-in.
      expect(capability.permissions).toContain("core:default");
      return fullscreen;
    }
    if (command === "plugin:window|set_fullscreen") {
      expect(capability.permissions).toContain("core:window:allow-set-fullscreen");
      expect(args).toMatchObject({ label: "main", value: !fullscreen });
      fullscreen = (args as { value: boolean }).value;
      return;
    }
    throw new Error(`Unexpected window command: ${command}`);
  });

  const key = new KeyboardEvent("keydown", { key: "f", metaKey: true, ctrlKey: true });
  expect(shortcutIdForKeyboardEvent(key, "macos")).toBe("toggleFullscreen");
  const port = createTauriWindowDialogPort();
  await port.toggleFullscreen();
  expect(fullscreen).toBe(true);
  await port.toggleFullscreen();
  expect(fullscreen).toBe(false);
  expect(calls).toEqual([
    "plugin:window|is_fullscreen", "plugin:window|set_fullscreen",
    "plugin:window|is_fullscreen", "plugin:window|set_fullscreen"
  ]);
});

test("captured zoom keys reach the permitted native webview command, including rapid input", async () => {
  mockWindows("main");
  const values: number[] = [];
  mockIPC((command, args) => {
    expect(command).toBe("plugin:webview|set_webview_zoom");
    expect(capability.permissions).toContain("core:webview:allow-set-webview-zoom");
    expect(args).toMatchObject({ label: "main" });
    values.push((args as { value: number }).value);
  });
  const port = createTauriWindowDialogPort();
  const changes: Promise<void>[] = [];
  const dispose = listenForAppShortcuts((action) => {
    switch (action) {
      case "zoomIn": changes.push(port.changeZoom("in")); return true;
      case "zoomOut": changes.push(port.changeZoom("out")); return true;
      case "resetZoom": changes.push(port.changeZoom("reset")); return true;
      default: return false;
    }
  });
  const dialog = document.createElement("dialog");
  dialog.open = true;
  dialog.addEventListener("keydown", (event) => event.stopPropagation());
  document.body.append(dialog);
  try {
    for (const key of ["-", "=", "+", "+", "0"]) {
      const event = new KeyboardEvent("keydown", {
        key, metaKey: true, shiftKey: key === "+", bubbles: true, cancelable: true
      });
      dialog.dispatchEvent(event);
      expect(event.defaultPrevented).toBe(true);
    }
    await Promise.all(changes);
    expect(values).toEqual([0.8, 1, 1.2, 1.4, 1]);
  } finally {
    dispose();
    dialog.remove();
  }
});

test("a failed native zoom propagates the error without advancing the scale or blocking later input", async () => {
  mockWindows("main");
  const values: number[] = [];
  mockIPC((_command, args) => {
    values.push((args as { value: number }).value);
    if (values.length === 1) throw new Error("zoom failed");
  });
  const port = createTauriWindowDialogPort();
  await expect(port.changeZoom("in")).rejects.toThrow("zoom failed");
  expect(document.documentElement.style.getPropertyValue("--webview-zoom")).toBe("");
  await port.changeZoom("in");
  expect(document.documentElement.style.getPropertyValue("--webview-zoom")).toBe("1.2");
  await port.changeZoom("out");
  expect(document.documentElement.style.getPropertyValue("--webview-zoom")).toBe("1");
  expect(values).toEqual([1.2, 1.2, 1]);
});

test("zoom stays within the native hotkey limits and reset restores 100%", async () => {
  mockWindows("main");
  let value = 1;
  mockIPC((_command, args) => {
    value = (args as { value: number }).value;
    expect(value).toBeGreaterThanOrEqual(0.2);
    expect(value).toBeLessThanOrEqual(10);
  });
  const port = createTauriWindowDialogPort();
  await Promise.all(Array.from({ length: 60 }, () => port.changeZoom("out")));
  expect(value).toBe(0.2);
  await Promise.all(Array.from({ length: 60 }, () => port.changeZoom("in")));
  expect(value).toBe(10);
  await port.changeZoom("reset");
  expect(value).toBe(1);
});
