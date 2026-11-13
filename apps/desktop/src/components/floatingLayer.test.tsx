// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useRef } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  FLOATING_LAYER_ANCHOR_GAP_PX as GAP,
  FLOATING_LAYER_VIEWPORT_MARGIN_PX as MARGIN,
  type FloatingPlacementInput,
  floatingPlacementStyle,
  pinnedPanelSize,
  resolveFloatingPlacement,
  useFloatingPlacement
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

  it("flips a fixed-size panel when the opposite side can hold the box it draws", () => {
    // The anchor leaves 78px above and 600px below the layer gap, and the panel
    // draws its own 600px. `comfortableBlockSize` is irrelevant here: 20px above
    // is "comfortable" for the small-popup heuristic, yet flipping is the only
    // way to draw 600px without covering the trigger (#1839 review).
    const placement = resolveFloatingPlacement(
      input({
        anchor: { top: 100, right: 340, bottom: 178, left: 300 },
        blockSize: 600,
        comfortableBlockSize: 20,
        shrinkToAnchor: false
      })
    );

    expect(placement.placement).toBe("below");
    expect(placement.blockSize).toBe(600);
    expect(placement.top).toBe(178 + GAP);
    expect(placement.top + placement.blockSize).toBe(800 - MARGIN);
  });

  it("takes the roomier side when neither side holds a fixed-size panel", () => {
    // 318px above against 398px below, and the panel draws 600px: the flip does
    // not invent room, so the roomier side wins and the clamp keeps the box
    // inside the bounds.
    const placement = resolveFloatingPlacement(
      input({
        anchor: { top: 340, right: 340, bottom: 380, left: 300 },
        blockSize: 600,
        shrinkToAnchor: false
      })
    );

    expect(placement.placement).toBe("below");
    expect(placement.blockSize).toBe(600);
    expect(placement.top).toBe(800 - MARGIN - 600);
  });

  it("keeps the comfortable-space flip for a panel that shrinks to its anchor", () => {
    // The same geometry as the fixed-size flip above, without the new option: the
    // small-popup heuristic must keep behaving exactly as before.
    const placement = resolveFloatingPlacement(
      input({
        anchor: { top: 100, right: 340, bottom: 178, left: 300 },
        blockSize: 600,
        comfortableBlockSize: 20
      })
    );

    expect(placement.placement).toBe("above");
    expect(placement.blockSize).toBe(100 - MARGIN - GAP);
    expect(placement.top).toBe(MARGIN);
  });

  it("keeps a panel's own block size and shifts it inside the bounds instead", () => {
    // A panel that scrolls its content (#1839) is narrowed by the bounds only;
    // the final clamp moves it off its anchor room rather than cutting it down.
    const placement = resolveFloatingPlacement(
      input({
        anchor: { top: 500, right: 340, bottom: 520, left: 300 },
        blockSize: 600,
        shrinkToAnchor: false
      })
    );

    expect(placement.blockSize).toBe(600);
    expect(placement.top).toBeGreaterThanOrEqual(MARGIN);
    expect(placement.top + placement.blockSize).toBeLessThanOrEqual(800 - MARGIN);
  });

  it("narrows a panel that keeps its own size to the bounds", () => {
    const placement = resolveFloatingPlacement(
      input({ blockSize: 900, shrinkToAnchor: false })
    );

    expect(placement.blockSize).toBe(800 - 2 * MARGIN);
    expect(placement.top).toBe(MARGIN);
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

describe("useFloatingPlacement with a content-measured panel", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  function MeasuredPanel() {
    const anchorRef = useRef<HTMLButtonElement>(null);
    const panelRef = useRef<HTMLDivElement>(null);
    const placement = useFloatingPlacement({
      align: "end",
      anchorRef,
      contentRef: panelRef,
      placement: "above"
    });
    return (
      <>
        <button ref={anchorRef} type="button" />
        {/* The layer positions this panel; its own CSS keeps the box. */}
        <div data-testid="panel" ref={panelRef} style={floatingPlacementStyle(placement, false)} />
      </>
    );
  }

  function panelStyle() {
    return (screen.getByTestId("panel") as HTMLElement).style;
  }

  it("places the panel from its own measured box and leaves the box to CSS", () => {
    // The trigger sits at (300, 200) and the panel measures 180x106.
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(
      function (this: HTMLElement) {
        return this.dataset.testid === "panel"
          ? new DOMRect(220, 180, 180, 106)
          : new DOMRect(300, 200, 24, 24);
      }
    );

    render(<MeasuredPanel />);

    // The measured 106px is what the placement math used: the panel sits above
    // its trigger with one gap and its own height. Nothing writes that size
    // back, because the next measurement would then read the clamped box
    // instead of the content (#1369 review).
    expect(panelStyle().top).toBe("88px");
    expect(panelStyle().blockSize).toBe("");
    expect(panelStyle().inlineSize).toBe("");
    expect(panelStyle().visibility).toBe("");
  });

  it("still places a panel that has no rectangle of its own", () => {
    // jsdom lays nothing out, so every rectangle is empty. The panel must be
    // positioned rather than left hidden forever.
    render(<MeasuredPanel />);

    expect(panelStyle().visibility).toBe("");
    expect(panelStyle().top).not.toBe("");
  });
});

describe("useFloatingPlacement for a closed popup", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  // A fixed-size popup like the receipt readers: no content ref, so only
  // `enabled` can tell the layer that nothing is open.
  function Popup({ enabled }: { enabled: boolean }) {
    const anchorRef = useRef<HTMLButtonElement>(null);
    const placement = useFloatingPlacement({
      align: "end",
      anchorRef,
      blockSize: 100,
      enabled,
      inlineSize: 200,
      placement: "above"
    });
    return (
      <>
        <button ref={anchorRef} type="button" />
        <div data-testid="panel" style={floatingPlacementStyle(placement)} />
      </>
    );
  }

  function panelStyle() {
    return (screen.getByTestId("panel") as HTMLElement).style;
  }

  function scrollListeners(listen: { mock: { calls: unknown[][] } }) {
    return listen.mock.calls.filter(([type]) => type === "scroll").length;
  }

  it("reads no layout and listens to nothing while closed (#1892)", () => {
    // Every timeline row with read receipts owns one of these. While closed it
    // must not force a layout on render or on any scroll in the page.
    const rects = vi.spyOn(Element.prototype, "getBoundingClientRect");
    const listen = vi.spyOn(document, "addEventListener");
    const observe = vi.spyOn(MutationObserver.prototype, "observe");
    const append = vi.spyOn(document.body, "append");

    const { rerender } = render(<Popup enabled={false} />);
    rerender(<Popup enabled={false} />);
    act(() => {
      fireEvent.scroll(document);
    });

    expect(rects).not.toHaveBeenCalled();
    expect(append).not.toHaveBeenCalled();
    expect(scrollListeners(listen)).toBe(0);
    expect(observe).not.toHaveBeenCalled();
    expect(panelStyle().visibility).toBe("hidden");

    rerender(<Popup enabled />);

    expect(rects).toHaveBeenCalled();
    expect(scrollListeners(listen)).toBe(1);
    expect(panelStyle().visibility).toBe("");
    expect(panelStyle().top).not.toBe("");
  });

  it("forgets its placement when it closes, so a reopen never draws a stale position", () => {
    const { rerender } = render(<Popup enabled />);
    expect(panelStyle().visibility).toBe("");

    rerender(<Popup enabled={false} />);

    expect(panelStyle().visibility).toBe("hidden");
  });

  it("reads the native titlebar inset once, not on every scroll (#1892)", () => {
    // The inset probe is a DOM write followed by a read. Repeating it per
    // measurement turns every later rectangle read into a forced layout.
    const append = vi.spyOn(document.body, "append");
    const probes = () => append.mock.calls.filter(([node]) => node instanceof HTMLSpanElement).length;

    render(<Popup enabled />);
    expect(probes()).toBe(1);

    act(() => {
      fireEvent.scroll(document);
      fireEvent.scroll(document);
    });
    expect(probes()).toBe(1);

    // A resize or a root platform/zoom change may move the inset.
    act(() => {
      fireEvent(window, new Event("resize"));
    });
    expect(probes()).toBe(2);
    act(() => {
      document.documentElement.setAttribute("data-platform", "macos");
    });
    return Promise.resolve().then(() => {
      expect(probes()).toBe(3);
      document.documentElement.removeAttribute("data-platform");
    });
  });
});

describe("pinnedPanelSize", () => {
  // #1583 keeps the chosen size, so a pinned (dragged) box can be narrower than
  // it on either axis: the layer bounds and the edges the drag holds still both
  // narrow it. The picker reads this back to say when it draws less than asked.
  const resolved = resolveFloatingPlacement(input({ blockSize: 900, inlineSize: 700 }));
  const unpinned = { bottom: null, left: 100, right: null, top: null };

  it("keeps the chosen size when the pinned edges leave room for it", () => {
    expect(pinnedPanelSize(resolved, unpinned, [700, 900])).toEqual([700, resolved.bounds.bottom - resolved.bounds.top]);
  });

  it("narrows from the pinned edge, never past the layer bounds", () => {
    expect(pinnedPanelSize(resolved, { ...unpinned, top: 600 }, [700, 900])[1]).toBe(
      resolved.bounds.bottom - 600
    );
    expect(
      pinnedPanelSize(resolved, { ...unpinned, right: resolved.bounds.left + 300 }, [700, 900])[0]
    ).toBe(300);
  });
});
