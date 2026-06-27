import { describe, expect, test } from "vitest";

import {
  elementShortcutParity,
  keyboardShortcutGroups,
  menuAccelerators,
  shortcutConflictAudit,
  shortcutActionFromMenuPayload,
  shortcutIdForKeyboardEvent,
  shortcutById,
  formatModShortcut
} from "./shortcuts";

describe("shortcut registry", () => {
  test("keeps Element-like keyboard settings categories in display order", () => {
    expect(keyboardShortcutGroups.map((group) => group.category)).toEqual([
      "composer",
      "room",
      "roomList",
      "navigation",
      "autocomplete",
      "accessibility"
    ]);
    expect(keyboardShortcutGroups.map((group) => group.categoryMessageId)).toEqual([
      "shortcut.categoryComposer",
      "shortcut.categoryRoom",
      "shortcut.categoryRoomList",
      "shortcut.categoryNavigation",
      "shortcut.categoryAutocomplete",
      "shortcut.categoryAccessibility"
    ]);
  });

  test("records Element-compatible settings and navigation shortcuts", () => {
    expect(shortcutById("showKeyboardSettings")).toMatchObject({
      labelMessageId: "shortcut.showKeyboardSettings",
      keys: ["Ctrl/Cmd", "/"],
      parity: "same",
      implemented: true
    });
    expect(shortcutById("openUserSettings")).toMatchObject({
      labelMessageId: "shortcut.openUserSettings",
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
    // Windows/Linux: Ctrl+F → searchInRoom
    expect(
      shortcutIdForKeyboardEvent(
        { key: "f", ctrlKey: true, metaKey: false, shiftKey: false, altKey: false },
        "linux"
      )
    ).toBe("searchInRoom");
    // Windows/Linux: Ctrl+K → filterRooms
    expect(
      shortcutIdForKeyboardEvent(
        { key: "k", ctrlKey: true, metaKey: false, shiftKey: false, altKey: false },
        "windows"
      )
    ).toBe("filterRooms");
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

  test("does not intercept Ctrl+F or Ctrl+K on macOS (reserved for native Emacs text bindings)", () => {
    // On macOS, Ctrl+F/K must pass through to the AppKit text system.
    expect(
      shortcutIdForKeyboardEvent(
        { key: "f", ctrlKey: true, metaKey: false, shiftKey: false, altKey: false },
        "macos"
      )
    ).toBeNull();
    expect(
      shortcutIdForKeyboardEvent(
        { key: "k", ctrlKey: true, metaKey: false, shiftKey: false, altKey: false },
        "macos"
      )
    ).toBeNull();
    // Cmd+F / Cmd+K still work as app shortcuts on macOS.
    expect(
      shortcutIdForKeyboardEvent(
        { key: "f", ctrlKey: false, metaKey: true, shiftKey: false, altKey: false },
        "macos"
      )
    ).toBe("searchInRoom");
    expect(
      shortcutIdForKeyboardEvent(
        { key: "k", ctrlKey: false, metaKey: true, shiftKey: false, altKey: false },
        "macos"
      )
    ).toBe("filterRooms");
  });

  test("formats modifier labels through explicit platform profiles", () => {
    expect(formatModShortcut("Enter", { platform: "macos", modLabel: "Cmd" })).toBe(
      "Cmd+Enter"
    );
    expect(
      formatModShortcut("Enter", { platform: "windows", modLabel: "Ctrl" })
    ).toBe("Ctrl+Enter");
    expect(formatModShortcut("Enter", { platform: "linux", modLabel: "Ctrl" })).toBe(
      "Ctrl+Enter"
    );
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
