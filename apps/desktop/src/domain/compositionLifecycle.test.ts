import { describe, expect, it, vi } from "vitest";

import {
  canApplyResolvedComposerAction,
  createCompositionOwnedValueState,
  createCompositionLifecycle,
  isComposerImeEnter,
  type ComposerKeyIntentSnapshot
} from "./compositionLifecycle";

describe("composition lifecycle", () => {
  it("keeps a dirty DOM value until the external owner acknowledges it", () => {
    const state = createCompositionOwnedValueState("before", "field-a");

    state.recordLocalValue("local");

    expect(state.observeExternal("before", "field-a", false)).toEqual({
      kind: "ignoreStale"
    });
    expect(state.observeExternal("local", "field-a", false)).toEqual({
      kind: "acknowledged"
    });
    expect(state.observeExternal("server", "field-a", false)).toEqual({
      kind: "write",
      value: "server"
    });
  });

  it("keeps the native DOM authoritative while composition is active", () => {
    const state = createCompositionOwnedValueState("before", "field-a");

    expect(state.observeExternal("stale", "field-a", true)).toEqual({
      kind: "ignoreComposition"
    });
  });

  it("forces a new semantic field value across a dirty key change", () => {
    const state = createCompositionOwnedValueState("before", "field-a");
    state.recordLocalValue("local");

    expect(state.observeExternal("next", "field-b", true)).toEqual({
      kind: "write",
      value: "next"
    });
    expect(state.currentKey()).toBe("field-b");
  });

  it("does not synchronize DOM-only controls from an absent external value", () => {
    const state = createCompositionOwnedValueState(undefined, "secret");
    state.recordLocalValue("private");

    expect(state.observeExternal(undefined, "secret", false)).toEqual({
      kind: "uncontrolled"
    });
  });

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
