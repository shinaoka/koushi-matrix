import { describe, expect, test } from "vitest";

import {
  composerKeyEventFromDom,
  insertNewlineAtSelection,
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
