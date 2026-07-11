import { describe, expect, it, vi } from "vitest";

import {
  canApplyResolvedComposerAction,
  createCompositionLifecycle,
  isComposerImeEnter,
  type ComposerKeyIntentSnapshot
} from "./compositionLifecycle";

describe("composition lifecycle", () => {
  it("keeps composition B active when the deferred end from A fires", () => {
    vi.useFakeTimers();
    const lifecycle = createCompositionLifecycle();
    const epochA = lifecycle.start();
    lifecycle.end(epochA);
    const epochB = lifecycle.start();

    vi.runAllTimers();

    expect(epochB).toBeGreaterThan(epochA);
    expect(lifecycle.active()).toBe(true);
    vi.useRealTimers();
  });

  it("invalidates the lifecycle generation when disposed", () => {
    const lifecycle = createCompositionLifecycle();
    const generation = lifecycle.generation();
    lifecycle.dispose();
    expect(lifecycle.generation()).toBeGreaterThan(generation);
  });

  it.each([
    ["send", true, false, true],
    ["cancel", true, false, true],
    ["closeAutocomplete", true, false, true],
    ["insertNewline", true, false, false],
    ["acceptAutocomplete", true, false, false],
    ["send", false, true, false]
  ] as const)(
    "applies %s according to result and mutation validity",
    (action, resultValid, mutationValid, expected) => {
      const intent: ComposerKeyIntentSnapshot = {
        value: "draft",
        selectionStart: 0,
        selectionEnd: 0,
        intentGeneration: 1,
        releaseResolution: () => undefined,
        isValidForResult: () => resultValid,
        isCurrentForMutation: () => mutationValid
      };
      expect(canApplyResolvedComposerAction(intent, action)).toBe(expected);
    }
  );

  it.each([
    [{ epochActive: true, nativeIsComposing: false, keyCode: 13 }, true],
    [{ epochActive: false, nativeIsComposing: true, keyCode: 13 }, true],
    [{ epochActive: false, nativeIsComposing: false, keyCode: 229 }, true],
    [{ epochActive: false, nativeIsComposing: false, keyCode: 13 }, false]
  ])("unifies epoch, native, and keyCode composing signals", (signals, expected) => {
    expect(isComposerImeEnter("Enter", signals)).toBe(expected);
  });
});
