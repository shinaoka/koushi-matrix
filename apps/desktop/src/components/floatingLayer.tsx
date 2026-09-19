//! Shared viewport-floating layer for contextual timeline and composer UI.
//!
//! Panes in this shell are overflow-clipped, so a popup anchored inside a row
//! or pane gets cut off at the pane boundary. Anything contextual therefore
//! renders in a body-level layer positioned from measured viewport
//! coordinates: it prefers the caller's side, flips when that side cannot fit,
//! and is clamped inside the boundary so it can never be cut off.
//!
//! This is the generalized form of the emoji picker placement introduced for
//! issue #302; issue #314 reuses it for the read-receipt reader popup.

import {
  type CSSProperties,
  type ReactNode,
  type RefObject,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { ModalPortalContext } from "./ModalDialog";

/** Minimum distance kept between the panel and the boundary edges. */
export const FLOATING_LAYER_VIEWPORT_MARGIN_PX = 16;
/** Distance between the anchor and the panel. */
export const FLOATING_LAYER_ANCHOR_GAP_PX = 6;

export interface FloatingRect {
  top: number;
  right: number;
  bottom: number;
  left: number;
}

export interface FloatingPlacementInput {
  /** Trigger rectangle in viewport coordinates, or null when unanchored. */
  anchor: FloatingRect | null;
  /** Container the panel must stay inside, in addition to the viewport. */
  boundary?: FloatingRect | null;
  viewport: { width: number; height: number };
  /** Preferred block-axis side; flipped when that side cannot fit the panel. */
  placement: "above" | "below";
  /** Preferred inline-axis alignment; flipped when it would overflow. */
  align: "start" | "end";
  direction: "ltr" | "rtl";
  /** Preferred panel size; narrowed to the available space when smaller. */
  inlineSize: number;
  blockSize: number;
  /**
   * Space that makes a side comfortable enough to keep the preferred
   * placement. Defaults to the full preferred block size, which is what a
   * small popup wants: flip as soon as it would not fit whole.
   */
  comfortableBlockSize?: number;
  margin?: number;
  gap?: number;
  safeTop?: number;
}

export interface FloatingPlacementResult {
  placement: "above" | "below";
  align: "start" | "end";
  /** Physical viewport coordinates: the panel is positioned `fixed`. */
  left: number;
  top: number;
  inlineSize: number;
  blockSize: number;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}

function isUsableRect(rect: FloatingRect): boolean {
  return rect.right > rect.left && rect.bottom > rect.top;
}

/**
 * Resolve the panel rectangle for a viewport-fixed floating layer.
 *
 * The panel prefers the caller's placement/alignment, flips to the opposite
 * side when the preferred one cannot fit, and is finally clamped inside the
 * boundary so an overflow-clipped pane or a small window can never cut it off.
 */
export function resolveFloatingPlacement(
  input: FloatingPlacementInput,
): FloatingPlacementResult {
  const margin = input.margin ?? FLOATING_LAYER_VIEWPORT_MARGIN_PX;
  const gap = input.gap ?? FLOATING_LAYER_ANCHOR_GAP_PX;
  const comfortable = input.comfortableBlockSize ?? input.blockSize;
  const viewportBounds: FloatingRect = {
    left: margin,
    top: Math.min(Math.max(margin, input.viewport.height - margin), margin + (input.safeTop ?? 0)),
    right: Math.max(margin, input.viewport.width - margin),
    bottom: Math.max(margin, input.viewport.height - margin),
  };
  const requested = input.boundary;
  const intersected: FloatingRect | null =
    requested && isUsableRect(requested)
      ? {
          left: Math.max(viewportBounds.left, requested.left),
          top: Math.max(viewportBounds.top, requested.top),
          right: Math.min(viewportBounds.right, requested.right),
          bottom: Math.min(viewportBounds.bottom, requested.bottom),
        }
      : null;
  // A missing, collapsed, or fully off-screen boundary cannot host the panel;
  // the viewport is then the only meaningful constraint.
  const bounds =
    intersected && isUsableRect(intersected) ? intersected : viewportBounds;

  const inlineSize = Math.min(input.inlineSize, bounds.right - bounds.left);
  const anchor: FloatingRect = input.anchor ?? {
    left: bounds.left,
    right: bounds.left,
    top: bounds.top,
    bottom: bounds.top,
  };

  const availableAbove = Math.max(0, anchor.top - bounds.top - gap);
  const availableBelow = Math.max(0, bounds.bottom - anchor.bottom - gap);
  const preferredAvailable =
    input.placement === "above" ? availableAbove : availableBelow;
  const oppositeAvailable =
    input.placement === "above" ? availableBelow : availableAbove;
  const opposite = input.placement === "above" ? "below" : "above";
  let placement = input.placement;
  if (preferredAvailable < comfortable) {
    if (oppositeAvailable >= comfortable) {
      placement = opposite;
    } else {
      placement = availableAbove >= availableBelow ? "above" : "below";
    }
  }
  const availableBlock = placement === "above" ? availableAbove : availableBelow;
  const blockSize = Math.min(input.blockSize, availableBlock, bounds.bottom - bounds.top);
  const top =
    placement === "above" ? anchor.top - gap - blockSize : anchor.bottom + gap;

  // `start`/`end` are logical: in RTL the inline-start edge is the right one.
  const startLeft =
    input.direction === "rtl" ? anchor.right - inlineSize : anchor.left;
  const endLeft =
    input.direction === "rtl" ? anchor.left : anchor.right - inlineSize;
  let align = input.align;
  let left = align === "start" ? startLeft : endLeft;
  if (left < bounds.left || left + inlineSize > bounds.right) {
    const flipped = align === "start" ? endLeft : startLeft;
    if (flipped >= bounds.left && flipped + inlineSize <= bounds.right) {
      align = align === "start" ? "end" : "start";
      left = flipped;
    }
  }

  return {
    align,
    blockSize,
    inlineSize,
    left: clamp(left, bounds.left, Math.max(bounds.left, bounds.right - inlineSize)),
    placement,
    top: clamp(top, bounds.top, Math.max(bounds.top, bounds.bottom - blockSize)),
  };
}

function placementsEqual(
  current: FloatingPlacementResult | null,
  next: FloatingPlacementResult,
): boolean {
  return (
    current != null &&
    current.align === next.align &&
    current.blockSize === next.blockSize &&
    current.inlineSize === next.inlineSize &&
    current.left === next.left &&
    current.placement === next.placement &&
    current.top === next.top
  );
}

function viewportRectOf(element: Element | null): FloatingRect | null {
  if (!element) {
    return null;
  }
  const { top, right, bottom, left } = element.getBoundingClientRect();
  return { bottom, left, right, top };
}

export interface FloatingPlacementOptions {
  anchorRef?: RefObject<Element | null>;
  /**
   * Resolves an extra container the panel must stay inside. Layout knowledge
   * stays with the caller; the panel re-resolves it on every measurement so a
   * resize or scroll cannot leave a stale boundary behind.
   */
  resolveBoundaryElement?: (anchor: Element) => Element | null;
  placement: "above" | "below";
  align: "start" | "end";
  inlineSize: number;
  blockSize: number;
  comfortableBlockSize?: number;
}

/**
 * Measure and keep a floating panel's viewport rectangle current.
 *
 * Remeasured on resize, on capture-phase scroll, and after every commit: an
 * ancestor re-render can move the anchor without firing either event (a pane
 * resize drag, a pane opening). Identical geometry keeps the previous state
 * object, so this cannot loop.
 */
export function useFloatingPlacement(
  options: FloatingPlacementOptions,
): FloatingPlacementResult | null {
  const {
    anchorRef,
    resolveBoundaryElement,
    placement,
    align,
    inlineSize,
    blockSize,
    comfortableBlockSize,
  } = options;
  const [resolved, setResolved] = useState<FloatingPlacementResult | null>(null);

  const measure = useCallback(() => {
    const anchorElement = anchorRef?.current ?? null;
    const boundaryElement =
      anchorElement && resolveBoundaryElement
        ? resolveBoundaryElement(anchorElement)
        : null;
    const next = resolveFloatingPlacement({
      safeTop: nativeTitlebarSafeTop(),
      align,
      anchor: viewportRectOf(anchorElement),
      blockSize,
      boundary: viewportRectOf(boundaryElement),
      comfortableBlockSize,
      // Root `dir` is Rust-owned locale profile output; the panel only reads it.
      direction: document.documentElement.dir === "rtl" ? "rtl" : "ltr",
      inlineSize,
      placement,
      viewport: { height: window.innerHeight, width: window.innerWidth },
    });
    setResolved((current) => (placementsEqual(current, next) ? current : next));
  }, [
    align,
    anchorRef,
    blockSize,
    comfortableBlockSize,
    inlineSize,
    placement,
    resolveBoundaryElement,
  ]);

  useLayoutEffect(() => {
    measure();
  });

  useEffect(() => {
    const observer = new MutationObserver(measure);
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["style", "data-platform", "dir"] });
    window.addEventListener("resize", measure);
    document.addEventListener("scroll", measure, true);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", measure);
      document.removeEventListener("scroll", measure, true);
    };
  }, [measure]);

  return resolved;
}

/** Inline style that positions a panel at a resolved placement. */
export function floatingPlacementStyle(
  resolved: FloatingPlacementResult | null,
): CSSProperties {
  return resolved
    ? {
        blockSize: `${resolved.blockSize}px`,
        inlineSize: `${resolved.inlineSize}px`,
        left: `${resolved.left}px`,
        top: `${resolved.top}px`,
      }
    : { visibility: "hidden" };
}

/** Read the shared CSS length in CSS pixels (custom properties may be calc()). */
function nativeTitlebarSafeTop(): number {
  const probe = document.createElement("span");
  probe.style.cssText = "position:fixed;visibility:hidden;pointer-events:none;height:var(--native-titlebar-safe-block, 0px)";
  document.body.append(probe);
  const height = probe.getBoundingClientRect().height;
  probe.remove();
  return height;
}

/** Keep modal descendants in their top layer, where they are not inert. */
export function FloatingLayer({ children }: { children: ReactNode }) {
  const modal = useContext(ModalPortalContext);
  const marker = useRef<HTMLSpanElement>(null);
  // Conditional popups mount directly into the established host, preserving
  // their caller's mount-time autofocus instead of delaying the portal.
  const [host, setHost] = useState<Element | null>(() => modal ? modal.current : typeof document === "undefined" ? null : document.body);
  useLayoutEffect(() => {
    setHost(marker.current?.closest("dialog") ?? document.body);
  }, []);
  if (typeof document === "undefined") return <>{children}</>;
  return <><span ref={marker} hidden />{host ? createPortal(children, host) : null}</>;
}

/**
 * Open state for a hover/focus popup.
 *
 * Hover and focus open the same popup, and pointer-leave, blur, or Escape
 * close it, so keyboard users reach exactly what pointer users see.
 */
export function useHoverFocusPopup(): {
  open: boolean;
  triggerProps: {
    onBlur: () => void;
    onFocus: () => void;
    onMouseEnter: () => void;
    onMouseLeave: () => void;
  };
} {
  const [open, setOpen] = useState(false);
  const openRef = useRef(open);
  openRef.current = open;

  useEffect(() => {
    if (!open) {
      return;
    }
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape" && !event.defaultPrevented && !event.isComposing && event.keyCode !== 229) {
        setOpen(false);
      }
    }
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [open]);

  return {
    open,
    triggerProps: {
      onBlur: () => setOpen(false),
      onFocus: () => setOpen(true),
      onMouseEnter: () => setOpen(true),
      onMouseLeave: () => setOpen(false),
    },
  };
}
