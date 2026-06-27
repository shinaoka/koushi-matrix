import { describe, expect, test } from "vitest";

import {
  applyMacEmacsAction,
  composerKeyEventFromDom,
  insertNewlineAtSelection,
  macEmacsActionFromEvent,
  shouldLetNativeImeHandleComposerKeyEvent,
  shouldResolveComposerKeyEvent
} from "./composerKeyEvents";

describe("composer key event normalization", () => {
  test("normalizes DOM keyboard facts without platform-specific branching", () => {
    expect(
      composerKeyEventFromDom({
        key: "Enter",
        ctrlKey: true,
        metaKey: false,
        shiftKey: false,
        altKey: false,
        nativeEvent: { isComposing: true }
      }, { start: 2, end: 4 })
    ).toEqual({
      key: "enter",
      modifiers: { ctrl: true, meta: false, shift: false, alt: false },
      is_composing: true,
      selection: { start: 2, end: 4 }
    });
  });

  test("only Enter and Escape require the async Rust resolver", () => {
    expect(shouldResolveComposerKeyEvent({ key: "Enter", ctrlKey: false, metaKey: false, shiftKey: false, altKey: false })).toBe(true);
    expect(shouldResolveComposerKeyEvent({ key: "Escape", ctrlKey: false, metaKey: false, shiftKey: false, altKey: false })).toBe(true);
    expect(shouldResolveComposerKeyEvent({ key: "a", ctrlKey: false, metaKey: false, shiftKey: false, altKey: false })).toBe(false);
  });

  test("lets native IME composition commits keep their browser default", () => {
    const composingEnter = composerKeyEventFromDom({
      key: "Enter",
      ctrlKey: false,
      metaKey: false,
      shiftKey: false,
      altKey: false,
      nativeEvent: { isComposing: true }
    });
    const plainEnter = composerKeyEventFromDom({
      key: "Enter",
      ctrlKey: false,
      metaKey: false,
      shiftKey: false,
      altKey: false,
      nativeEvent: { isComposing: false }
    });

    expect(shouldLetNativeImeHandleComposerKeyEvent(composingEnter)).toBe(true);
    expect(shouldLetNativeImeHandleComposerKeyEvent(plainEnter)).toBe(false);
  });

  test("inserts resolver-approved newlines at the captured textarea selection", () => {
    expect(insertNewlineAtSelection("hello world", 5, 6)).toEqual({
      value: "hello\nworld",
      cursor: 6
    });
  });
});

describe("macOS Emacs text-editing bindings", () => {
  const base = { metaKey: false, shiftKey: false, altKey: false };

  test("macEmacsActionFromEvent maps Ctrl+F/B/P/N/K/Y on macOS", () => {
    expect(macEmacsActionFromEvent({ ...base, key: "f", ctrlKey: true })).toBe("moveForward");
    expect(macEmacsActionFromEvent({ ...base, key: "b", ctrlKey: true })).toBe("moveBackward");
    expect(macEmacsActionFromEvent({ ...base, key: "p", ctrlKey: true })).toBe("moveUp");
    expect(macEmacsActionFromEvent({ ...base, key: "n", ctrlKey: true })).toBe("moveDown");
    expect(macEmacsActionFromEvent({ ...base, key: "k", ctrlKey: true })).toBe("killToEol");
    expect(macEmacsActionFromEvent({ ...base, key: "y", ctrlKey: true })).toBe("yank");
  });

  test("macEmacsActionFromEvent returns null when Ctrl is absent or a second modifier is held", () => {
    expect(macEmacsActionFromEvent({ ...base, key: "f", ctrlKey: false })).toBeNull();
    expect(macEmacsActionFromEvent({ key: "f", ctrlKey: true, metaKey: true, shiftKey: false, altKey: false })).toBeNull();
    expect(macEmacsActionFromEvent({ key: "f", ctrlKey: true, metaKey: false, shiftKey: true, altKey: false })).toBeNull();
    expect(macEmacsActionFromEvent({ ...base, key: "z", ctrlKey: true })).toBeNull();
  });

  test("applyMacEmacsAction moveForward advances cursor by one character", () => {
    expect(applyMacEmacsAction("moveForward", "hello", 2, 2, "")).toEqual({
      newSelectionPos: 3
    });
    // Clamps at end of string
    expect(applyMacEmacsAction("moveForward", "hello", 5, 5, "")).toEqual({
      newSelectionPos: 5
    });
  });

  test("applyMacEmacsAction moveBackward retreats cursor by one character", () => {
    expect(applyMacEmacsAction("moveBackward", "hello", 3, 3, "")).toEqual({
      newSelectionPos: 2
    });
    // Clamps at start
    expect(applyMacEmacsAction("moveBackward", "hello", 0, 0, "")).toEqual({
      newSelectionPos: 0
    });
  });

  test("applyMacEmacsAction moveUp moves to same column on previous line", () => {
    const value = "abc\ndef\nghi";
    // Cursor at 'h' (index 8, col 0 of 3rd line) → 'd' (index 4, col 0 of 2nd line)
    expect(applyMacEmacsAction("moveUp", value, 8, 8, "")).toEqual({ newSelectionPos: 4 });
    // Already on first line — clamps to 0
    expect(applyMacEmacsAction("moveUp", value, 2, 2, "")).toEqual({ newSelectionPos: 0 });
  });

  test("applyMacEmacsAction moveDown moves to same column on next line", () => {
    const value = "abc\ndef\nghi";
    // Cursor at 'b' (index 1, col 1 of 1st line) → 'e' (index 5, col 1 of 2nd line)
    expect(applyMacEmacsAction("moveDown", value, 1, 1, "")).toEqual({ newSelectionPos: 5 });
    // Already on last line — clamps to end
    expect(applyMacEmacsAction("moveDown", value, 9, 9, "")).toEqual({ newSelectionPos: 11 });
  });

  test("applyMacEmacsAction killToEol deletes from cursor to end of line and saves kill ring", () => {
    // Mid-line: delete 'llo'
    expect(applyMacEmacsAction("killToEol", "hello\nworld", 2, 2, "")).toEqual({
      newValue: "he\nworld",
      newSelectionPos: 2,
      newKillRing: "llo"
    });
    // At EOL: kill the newline itself
    expect(applyMacEmacsAction("killToEol", "hello\nworld", 5, 5, "")).toEqual({
      newValue: "helloworld",
      newSelectionPos: 5,
      newKillRing: "\n"
    });
    // At very end of string: nothing to kill
    expect(applyMacEmacsAction("killToEol", "hello", 5, 5, "")).toBeNull();
  });

  test("applyMacEmacsAction yank inserts kill-ring content at cursor", () => {
    expect(applyMacEmacsAction("yank", "he\nworld", 2, 2, "llo")).toEqual({
      newValue: "hello\nworld",
      newSelectionPos: 5
    });
    // Empty kill ring: no-op
    expect(applyMacEmacsAction("yank", "hello", 3, 3, "")).toBeNull();
  });
});
