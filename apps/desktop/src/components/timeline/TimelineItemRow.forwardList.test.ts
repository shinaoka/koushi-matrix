// @vitest-environment jsdom
import { describe, expect, test } from "vitest";

import type { FloatingPlacementResult } from "../floatingLayer";
import { forwardListStyle } from "./TimelineItemRow";

// #2078 review, BLOCKER 1: the Forward destination list is content-sized, so a
// box placed `above` must anchor its block-end edge. `resolveFloatingPlacement`
// returns a `top` that assumes the box is exactly `blockSize` tall, which is
// only true when the rows fill the cap — a five-room list drawn from that `top`
// sits hundreds of pixels away from the item that opened it.

function placement(overrides: Partial<FloatingPlacementResult>): FloatingPlacementResult {
  return {
    align: "end",
    blockSize: 400,
    bounds: { bottom: window.innerHeight - 16, left: 16, right: 1264, top: 16 },
    inlineSize: 240,
    left: 998,
    placement: "below",
    top: 100,
    ...overrides
  };
}

describe("the Forward destination list's box (#2078)", () => {
  test("an above placement pins the block-end edge and leaves the top free", () => {
    const style = forwardListStyle(placement({ placement: "above", top: 100 }));
    // bottom = viewport - (top + blockSize): the box's bottom edge lands where
    // the resolver put the cap's bottom edge, whatever height the rows need.
    expect(style?.bottom).toBe(`${window.innerHeight - 500}px`);
    expect(style).not.toHaveProperty("top");
    expect(style?.maxBlockSize).toBe("400px");
    expect(style?.left).toBe("998px");
    expect(style?.inlineSize).toBe("240px");
  });

  test("a below placement pins the top edge", () => {
    const style = forwardListStyle(placement({ placement: "below", top: 100 }));
    expect(style?.top).toBe("100px");
    expect(style).not.toHaveProperty("bottom");
  });

  test("a list that has not been measured yet keeps the browser's own box", () => {
    // Not `visibility: hidden`: the layer resolves the placement before the
    // next paint, and a hidden panel cannot take the focus opening it asks for.
    expect(forwardListStyle(null)).toBeUndefined();
  });
});
