// @vitest-environment jsdom

import { describe, expect, it } from "vitest";

import {
  FLOATING_LAYER_VIEWPORT_MARGIN_PX as MARGIN,
  type FloatingPlacementInput,
  resolveFloatingPlacement
} from "./floatingLayer";

function input(overrides: Partial<FloatingPlacementInput> = {}): FloatingPlacementInput {
  return {
    align: "start",
    anchor: { top: 400, right: 340, bottom: 420, left: 300 },
    blockSize: 120,
    direction: "ltr",
    inlineSize: 240,
    placement: "above",
    viewport: { height: 800, width: 1280 },
    ...overrides
  };
}

describe("resolveFloatingPlacement", () => {
  it("intersects the native safe area with caller bounds even for an offscreen anchor", () => {
    const placement = resolveFloatingPlacement(input({
      safeTop: 88,
      viewport: { width: 700, height: 320 },
      boundary: { left: 100, right: 650, top: 30, bottom: 300 },
      anchor: { left: 200, right: 240, top: 900, bottom: 920 },
      blockSize: 700,
    }));
    expect(placement.top).toBeGreaterThanOrEqual(88 + MARGIN);
    expect(placement.top + placement.blockSize).toBeLessThanOrEqual(300);
    expect(placement.left).toBeGreaterThanOrEqual(100);
  });
  it("keeps the preferred side and alignment when the anchor has room", () => {
    const placement = resolveFloatingPlacement(input());

    expect(placement.placement).toBe("above");
    expect(placement.align).toBe("start");
    expect(placement.left).toBe(300);
    expect(placement.top).toBe(400 - 6 - 120);
    expect(placement.inlineSize).toBe(240);
    expect(placement.blockSize).toBe(120);
  });

  it("flips the inline alignment rather than overflowing the viewport", () => {
    // A thread-pane receipt stack sits near the right edge: start alignment
    // would push the popup past the viewport.
    const placement = resolveFloatingPlacement(
      input({ anchor: { top: 400, right: 1264, bottom: 420, left: 1200 } })
    );

    expect(placement.align).toBe("end");
    expect(placement.left + placement.inlineSize).toBeLessThanOrEqual(1280 - MARGIN);
  });

  it("flips below when the anchor sits under the top margin", () => {
    const placement = resolveFloatingPlacement(
      input({ anchor: { top: 24, right: 340, bottom: 44, left: 300 } })
    );

    expect(placement.placement).toBe("below");
    expect(placement.top).toBe(44 + 6);
  });

  it("stays inside a clipped pane boundary", () => {
    // The thread pane is the boundary: the popup may not escape it sideways.
    const placement = resolveFloatingPlacement(
      input({
        anchor: { top: 400, right: 1270, bottom: 420, left: 1240 },
        boundary: { top: 60, right: 1280, bottom: 760, left: 900 }
      })
    );

    expect(placement.left).toBeGreaterThanOrEqual(900);
    expect(placement.left + placement.inlineSize).toBeLessThanOrEqual(1280 - MARGIN);
  });

  it("shrinks to the space that is actually available", () => {
    // A tall panel with a short gap above: it must shrink, not overflow.
    const placement = resolveFloatingPlacement(
      input({ anchor: { top: 120, right: 340, bottom: 140, left: 300 }, blockSize: 400 })
    );

    expect(placement.placement).toBe("below");
    expect(placement.top + placement.blockSize).toBeLessThanOrEqual(800 - MARGIN);
  });

  it("ignores a collapsed boundary and falls back to the viewport", () => {
    const placement = resolveFloatingPlacement(
      input({ boundary: { top: 0, right: 0, bottom: 0, left: 0 } })
    );

    expect(placement.inlineSize).toBe(240);
    expect(placement.blockSize).toBe(120);
    expect(placement.left).toBe(300);
  });

  it("never returns a rectangle outside the viewport margin", () => {
    for (const anchor of [
      { top: 0, right: 8, bottom: 4, left: 0 },
      { top: 796, right: 1280, bottom: 800, left: 1272 },
      { top: 400, right: 700, bottom: 420, left: 660 }
    ]) {
      for (const align of ["start", "end"] as const) {
        for (const preferred of ["above", "below"] as const) {
          const placement = resolveFloatingPlacement(
            input({ align, anchor, placement: preferred })
          );
          expect(placement.left).toBeGreaterThanOrEqual(MARGIN);
          expect(placement.top).toBeGreaterThanOrEqual(MARGIN);
          expect(placement.left + placement.inlineSize).toBeLessThanOrEqual(1280 - MARGIN);
          expect(placement.top + placement.blockSize).toBeLessThanOrEqual(800 - MARGIN);
        }
      }
    }
  });

  it("mirrors the inline alignment under right-to-left direction", () => {
    const anchor = { top: 400, right: 640, bottom: 420, left: 600 };
    const ltr = resolveFloatingPlacement(input({ anchor }));
    const rtl = resolveFloatingPlacement(input({ anchor, direction: "rtl" }));

    expect(ltr.left).toBe(600);
    expect(rtl.left).toBe(640 - 240);
  });
});
