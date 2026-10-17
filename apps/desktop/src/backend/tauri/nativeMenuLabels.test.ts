/* @vitest-environment jsdom */

import { invoke } from "@tauri-apps/api/core";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, expect, test, vi } from "vitest";

import { catalogs } from "../../i18n/messages";
import { syncNativeMenuLabels } from "./nativeMenuLabels";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (command: string) =>
    command === "native_menu_label_keys" ? ["menu.view", "menu.toggleRightPanel"] : undefined
  )
}));

afterEach(() => {
  vi.clearAllMocks();
  Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
});

test("Rust menu label ids exist in both frontend catalogs", () => {
  const source = readFileSync(resolve(process.cwd(), "src-tauri/src/desktop_menu.rs"), "utf8");
  const rustKeys = new Set([...source.matchAll(/"(menu\.[A-Za-z]+)"/g)].map((match) => match[1]));
  expect(rustKeys).toEqual(new Set(Object.keys(catalogs.en).filter((key) => key.startsWith("menu."))));
  expect(rustKeys).toEqual(new Set(Object.keys(catalogs.ja).filter((key) => key.startsWith("menu."))));
});

test("pushes the locale-resolved labels for every key the menu owns", async () => {
  Reflect.set(window, "__TAURI_INTERNALS__", {});
  await syncNativeMenuLabels("ja");
  expect(invoke).toHaveBeenLastCalledWith("set_native_menu_labels", {
    labels: { "menu.view": "表示", "menu.toggleRightPanel": "右パネルを切り替え" }
  });
});

test("renders the Rust-resolved bidi pseudo locale rather than accented labels", async () => {
  Reflect.set(window, "__TAURI_INTERNALS__", {});
  await syncNativeMenuLabels("pseudo", "bidi");
  expect(invoke).toHaveBeenLastCalledWith("set_native_menu_labels", {
    labels: {
      "menu.view": expect.stringContaining("\u202e"),
      "menu.toggleRightPanel": expect.stringContaining("\u202e")
    }
  });
});

test("a slower old locale cannot replace the latest menu", async () => {
  Reflect.set(window, "__TAURI_INTERNALS__", {});
  let releaseFirst!: (keys: string[]) => void;
  let keyRequests = 0;
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command !== "native_menu_label_keys") return;
    if (++keyRequests === 1) {
      return new Promise<string[]>((resolve) => { releaseFirst = resolve; });
    }
    return ["menu.view"];
  });
  const oldUpdate = syncNativeMenuLabels("en");
  const newUpdate = syncNativeMenuLabels("ja");
  await Promise.resolve();
  await Promise.resolve();
  releaseFirst(["menu.view"]);
  await Promise.all([oldUpdate, newUpdate]);
  const updates = vi.mocked(invoke).mock.calls.filter(([command]) => command === "set_native_menu_labels");
  expect(updates.at(-1)?.[1]).toEqual({ labels: { "menu.view": "表示" } });
});

test("does nothing outside the desktop runtime", async () => {
  await syncNativeMenuLabels("ja");
  expect(invoke).not.toHaveBeenCalled();
});

test("names the selected account on account-scoped menu items (#1504)", async () => {
  Reflect.set(window, "__TAURI_INTERNALS__", {});
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === "native_menu_label_keys"
      ? ["menu.signOut", "menu.accountSettings", "menu.view"]
      : undefined
  );
  await syncNativeMenuLabels("en", "none", "Alice");
  expect(invoke).toHaveBeenLastCalledWith("set_native_menu_labels", {
    labels: {
      "menu.signOut": "Sign Out of Alice",
      "menu.accountSettings": "Account Settings for Alice…",
      "menu.view": "View"
    }
  });
});
