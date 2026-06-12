import { describe, expect, test } from "vitest";

import {
  elementShortcutParity,
  keyboardShortcutGroups,
  menuAccelerators,
  shortcutConflictAudit,
  shortcutActionFromMenuPayload,
  shortcutIdForKeyboardEvent,
  shortcutById
} from "./shortcuts";

describe("shortcut registry", () => {
  test("keeps Element-like keyboard settings categories in display order", () => {
    expect(keyboardShortcutGroups.map((group) => group.category)).toEqual([
      "Composer",
      "Room",
      "Room List",
      "Navigation",
      "Autocomplete",
      "Accessibility"
    ]);
  });

  test("records Element-compatible settings and navigation shortcuts", () => {
    expect(shortcutById("showKeyboardSettings")).toMatchObject({
      label: "Keyboard settings",
      keys: ["Ctrl/Cmd", "/"],
      parity: "same",
      implemented: true
    });
    expect(shortcutById("openUserSettings")).toMatchObject({
      label: "User settings",
      keys: ["Cmd", ","],
      platforms: ["macos"],
      parity: "same"
    });
    expect(shortcutById("toggleRightPanel")).toMatchObject({
      keys: ["Ctrl/Cmd", "."],
      parity: "same"
    });
  });

  test("marks call and upload shortcuts as deferred until those features exist", () => {
    expect(shortcutById("uploadFile")).toMatchObject({
      parity: "deferred",
      implemented: false
    });
    expect(shortcutById("toggleMicrophone")).toMatchObject({
      parity: "deferred",
      implemented: false
    });
  });

  test("exposes parity rows and native menu accelerators from the same registry", () => {
    expect(elementShortcutParity().map((row) => row.id)).toContain("filterRooms");
    expect(menuAccelerators()).toContainEqual({
      id: "openUserSettings",
      accelerator: "CmdOrCtrl+,",
      nativeMenu: "app"
    });
    expect(menuAccelerators()).toContainEqual({
      id: "showKeyboardSettings",
      accelerator: "CmdOrCtrl+/",
      nativeMenu: "help"
    });
  });

  test("resolves implemented Element-compatible keyboard events", () => {
    expect(
      shortcutIdForKeyboardEvent({
        key: "/",
        ctrlKey: true,
        metaKey: false,
        shiftKey: false,
        altKey: false
      })
    ).toBe("showKeyboardSettings");
    expect(
      shortcutIdForKeyboardEvent({
        key: ",",
        ctrlKey: false,
        metaKey: true,
        shiftKey: false,
        altKey: false
      })
    ).toBe("openUserSettings");
    expect(
      shortcutIdForKeyboardEvent({
        key: "f",
        ctrlKey: true,
        metaKey: false,
        shiftKey: false,
        altKey: false
      })
    ).toBe("searchInRoom");
    expect(
      shortcutIdForKeyboardEvent({
        key: "k",
        ctrlKey: false,
        metaKey: true,
        shiftKey: false,
        altKey: false
      })
    ).toBe("filterRooms");
    expect(
      shortcutIdForKeyboardEvent({
        key: ".",
        ctrlKey: true,
        metaKey: false,
        shiftKey: false,
        altKey: false
      })
    ).toBe("toggleRightPanel");
  });

  test("accepts native menu payloads only for registered implemented actions", () => {
    expect(shortcutActionFromMenuPayload("showKeyboardSettings")).toBe(
      "showKeyboardSettings"
    );
    expect(shortcutActionFromMenuPayload("openUserSettings")).toBe("openUserSettings");
    expect(shortcutActionFromMenuPayload("toggleRightPanel")).toBe("toggleRightPanel");
    expect(shortcutActionFromMenuPayload("uploadFile")).toBeNull();
    expect(shortcutActionFromMenuPayload("unknown")).toBeNull();
  });

  test("records shortcut conflict resolution and rejects unresolved collisions", () => {
    expect(shortcutConflictAudit()).toEqual({
      adaptedWithoutReason: [],
      duplicateAccelerators: [],
      duplicateGlobalHandlers: [],
      nativeMenuWithoutAccelerator: [],
      acceleratorWithoutNativeMenu: []
    });
  });
});
