import { describe, expect, it, vi } from "vitest";

import { createCompositionLifecycle, isComposerImeEnter } from "./compositionLifecycle";

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
    [{ epochActive: true, nativeIsComposing: false, keyCode: 13 }, true],
    [{ epochActive: false, nativeIsComposing: true, keyCode: 13 }, true],
    [{ epochActive: false, nativeIsComposing: false, keyCode: 229 }, true],
    [{ epochActive: false, nativeIsComposing: false, keyCode: 13 }, false]
  ])("unifies epoch, native, and keyCode composing signals", (signals, expected) => {
    expect(isComposerImeEnter("Enter", signals)).toBe(expected);
  });
});
