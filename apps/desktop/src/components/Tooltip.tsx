import {
  type CSSProperties,
  type KeyboardEvent,
  type FocusEvent,
  type MouseEvent,
  type PointerEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState
} from "react";

import { FloatingLayer, resolveFloatingPlacement } from "./floatingLayer";
import {
  getHoverIntentSnapshot,
  hoverIntentAllowsPointerHover,
  subscribeHoverIntent
} from "./hoverIntent";

export type TooltipPlacement = "right" | "above" | "below";

export type TooltipTriggerProps = {
  "aria-describedby"?: string;
  onFocus?: (event: FocusEvent<HTMLElement>) => void;
  onBlur?: (event: FocusEvent<HTMLElement>) => void;
  onKeyDown: (event: KeyboardEvent<HTMLElement>) => void;
  onMouseEnter: (event: MouseEvent<HTMLElement>) => void;
  onMouseLeave: (event: MouseEvent<HTMLElement>) => void;
  onPointerMove: (event: PointerEvent<HTMLElement>) => void;
  onPointerDown: (event: PointerEvent<HTMLElement>) => void;
};

type TooltipProps = {
  children: (props: TooltipTriggerProps) => ReactNode;
  label: string;
  /** Non-action metadata may disclose its full value to keyboard users. */
  openOnFocus?: boolean;
  /**
   * `right` keeps the bubble beside the trigger. `above`/`below` anchor on the
   * trigger itself, which is what a toolbar or a header row wants: the control
   * keeps its own place in the row and a bubble near a pane or window edge
   * cannot be mistaken for the neighbouring control.
   */
  placement?: TooltipPlacement;
  /** Preferred inline alignment for above/below; flips when it would overflow. */
  align?: "start" | "end";
  delayMs?: number;
  /**
   * #1327: extra host class. A row-level trigger needs a block-level host so the
   * wrapped button keeps the row's own box.
   */
  className?: string;
  /**
   * #1327: identity of the fact this popup explains — the room whose status the
   * trigger belongs to. A change closes an open bubble, so a room switch never
   * leaves the previous room's explanation on screen. A label change for the
   * *same* room keeps the bubble open and shows the new text.
   */
  resetKey?: string | number | null;
};

/** Keeps the bubble away from the viewport edges. */
const TOOLTIP_VIEWPORT_MARGIN_PX = 12;
/** Distance between the trigger and the bubble. */
const TOOLTIP_ANCHOR_GAP_PX = 8;

/**
 * The shared explanation bubble for an icon-only control.
 *
 * #1388: the bubble is pointer-owned and opens on hover only. Keyboard and
 * programmatic focus must not open it by default — returning focus to a menu trigger after a
 * selection used to leave the explanation floating over the sidebar. The wrapped
 * control keeps its own `aria-label`, which is what assistive technology reads.
 */
export function Tooltip({
  children,
  label,
  placement = "right",
  align = "start",
  delayMs = 250,
  className,
  resetKey,
  openOnFocus = false
}: TooltipProps) {
  const tooltipId = useId();
  const [isOpen, setIsOpen] = useState(false);
  const openTimer = useRef<number | null>(null);
  const hostRef = useRef<HTMLSpanElement>(null);
  const bubbleRef = useRef<HTMLSpanElement>(null);
  const [style, setStyle] = useState<CSSProperties>({ visibility: "hidden" });
  // A press answers the question the tooltip was asking, and the pointer stays
  // parked on the control afterwards, so without this the parked hover re-opened
  // the bubble and it stayed on screen — duplicating the label the action had
  // just revealed and shadowing that text for anything matching it. Leaving the
  // control re-arms the explanation, so the next hover still describes the
  // button.
  const pressDismissed = useRef(false);

  function clearOpenTimer() {
    if (openTimer.current !== null) {
      window.clearTimeout(openTimer.current);
      openTimer.current = null;
    }
  }

  function openAfterDelay() {
    if (pressDismissed.current || !hoverIntentAllowsPointerHover()) {
      return;
    }
    clearOpenTimer();
    if (
      delayMs <= 0 ||
      window.matchMedia?.("(prefers-reduced-motion: reduce)").matches
    ) {
      openNow();
      return;
    }
    openTimer.current = window.setTimeout(() => {
      openTimer.current = null;
      setIsOpen(true);
    }, delayMs);
  }

  function openNow() {
    if (pressDismissed.current) {
      return;
    }
    clearOpenTimer();
    setIsOpen(true);
  }

  function close() {
    clearOpenTimer();
    setIsOpen(false);
  }

  function press(event: PointerEvent<HTMLElement>) {
    // Only the primary button acts on the control, so a secondary press must not
    // swallow the explanation. It still dismisses it: the bubble renders above
    // the context menu layer, and a bubble left over a freshly opened menu would
    // paint on top of that menu. Dismissing without arming keeps a later hover
    // free to explain the control again.
    if (event.button !== 0) {
      close();
      return;
    }
    pressDismissed.current = true;
    close();
  }

  /** Leaving the control re-arms the bubble for the next hover. */
  function release() {
    pressDismissed.current = false;
    close();
  }

  useEffect(() => {
    return () => clearOpenTimer();
  }, []);

  // Content moving under a parked pointer is not a hover. Dismiss an open
  // bubble when a scroll gesture starts instead of letting it chase the row.
  // Subscribing without render state keeps a parked hover from re-rendering
  // every tooltip in a scrolling timeline.
  useEffect(() => {
    let generation = getHoverIntentSnapshot().scrollGeneration;
    return subscribeHoverIntent(() => {
      const next = getHoverIntentSnapshot();
      if (next.scrollGeneration === generation) {
        return;
      }
      generation = next.scrollGeneration;
      clearOpenTimer();
      setIsOpen(false);
    });
  }, []);

  // Close — do not re-anchor — when the trigger starts explaining a different
  // fact, so a room switch dismisses the popup instead of rewriting it.
  const previousResetKey = useRef(resetKey);
  useEffect(() => {
    if (previousResetKey.current === resetKey) {
      return;
    }
    previousResetKey.current = resetKey;
    close();
  }, [resetKey]);

  useEffect(() => {
    if (!isOpen) {
      return undefined;
    }
    function onDocumentKeyDown(event: globalThis.KeyboardEvent) {
      if (event.key === "Escape") {
        close();
      }
    }
    document.addEventListener("keydown", onDocumentKeyDown);
    return () => document.removeEventListener("keydown", onDocumentKeyDown);
  }, [isOpen]);

  // #1166: the bubble renders in the body-level floating layer, because sidebar
  // and pane scrollports clip an in-row bubble (the same reason the read-receipt
  // popup does). It is measured after mount, prefers the requested side, flips
  // when that side cannot fit, and is clamped inside the viewport.
  const positionBubble = useCallback(() => {
    const host = hostRef.current;
    const bubble = bubbleRef.current;
    if (!host) {
      return;
    }
    // The trigger is its own anchor. The measured placements keep the host out
    // of layout (`display: contents`), so it has no box of its own and the first
    // element child is the control the bubble explains.
    const firstChild = host.firstElementChild;
    const anchorElement = firstChild instanceof HTMLElement ? firstChild : host;
    const anchor = anchorElement.getBoundingClientRect();
    const size = bubble?.getBoundingClientRect();
    const width = size?.width ?? 0;
    const height = size?.height ?? 0;
    const viewportWidth = window.innerWidth;
    const viewportHeight = window.innerHeight;
    const maxLeft = Math.max(
      TOOLTIP_VIEWPORT_MARGIN_PX,
      viewportWidth - TOOLTIP_VIEWPORT_MARGIN_PX - width
    );
    let left: number;
    let top: number;
    if (placement === "right") {
      left = anchor.right + TOOLTIP_ANCHOR_GAP_PX;
      if (left + width > viewportWidth - TOOLTIP_VIEWPORT_MARGIN_PX) {
        const flipped = anchor.left - TOOLTIP_ANCHOR_GAP_PX - width;
        left = flipped >= TOOLTIP_VIEWPORT_MARGIN_PX ? flipped : maxLeft;
      }
      if (left < TOOLTIP_VIEWPORT_MARGIN_PX) {
        left = TOOLTIP_VIEWPORT_MARGIN_PX;
      }
      const centered = anchor.top + anchor.height / 2 - height / 2;
      top = Math.min(
        Math.max(centered, TOOLTIP_VIEWPORT_MARGIN_PX),
        Math.max(
          TOOLTIP_VIEWPORT_MARGIN_PX,
          viewportHeight - TOOLTIP_VIEWPORT_MARGIN_PX - height
        )
      );
    } else {
      // The shared resolver owns above/below geometry: it keeps `start`/`end`
      // logical for an RTL root, flips the side that cannot fit, and clamps the
      // result, so a bubble cannot drift from the other floating panels. The
      // tooltip keeps its own tighter margins because a bubble sits next to
      // text rather than over a pane.
      const resolved = resolveFloatingPlacement({
        align,
        anchor: {
          bottom: anchor.bottom,
          left: anchor.left,
          right: anchor.right,
          top: anchor.top
        },
        blockSize: height,
        // Root `dir` is Rust-owned locale profile output; the bubble only reads it.
        direction: document.documentElement.dir === "rtl" ? "rtl" : "ltr",
        gap: TOOLTIP_ANCHOR_GAP_PX,
        inlineSize: width,
        margin: TOOLTIP_VIEWPORT_MARGIN_PX,
        placement,
        viewport: { height: viewportHeight, width: viewportWidth }
      });
      left = resolved.left;
      top = resolved.top;
    }
    const next: CSSProperties = {
      position: "fixed",
      left: `${left}px`,
      top: `${top}px`,
      visibility: "visible"
    };
    // Identical geometry keeps the previous object, so a scroll frame that does
    // not move the anchor cannot re-render in a loop.
    setStyle((previous) =>
      previous.left === next.left && previous.top === next.top ? previous : next
    );
  }, [placement, align]);

  useLayoutEffect(() => {
    if (!isOpen) {
      setStyle({ visibility: "hidden" });
      return;
    }
    positionBubble();
  }, [isOpen, label, placement, align, positionBubble]);

  // A hovered trigger can stay open while its pane scrolls or the window
  // resizes, so the bubble follows the anchor instead of staying behind.
  useEffect(() => {
    if (!isOpen) {
      return undefined;
    }
    const onMove = () => positionBubble();
    window.addEventListener("scroll", onMove, true);
    window.addEventListener("resize", onMove);
    return () => {
      window.removeEventListener("scroll", onMove, true);
      window.removeEventListener("resize", onMove);
    };
  }, [isOpen, positionBubble]);

  const triggerProps: TooltipTriggerProps = {
    "aria-describedby": isOpen ? tooltipId : undefined,
    onKeyDown: (event) => {
      if (event.key === "Escape") {
        close();
      }
    },
    onFocus: openOnFocus ? openNow : undefined,
    onBlur: openOnFocus ? release : undefined,
    onMouseEnter: openAfterDelay,
    onMouseLeave: release,
    // A row that scrolled under a parked pointer got mouseenter while the gate
    // was closed, so the reader's first real pointer movement must re-arm it.
    onPointerMove: () => {
      if (isOpen || openTimer.current !== null) {
        return;
      }
      openAfterDelay();
    },
    onPointerDown: press
  };

  const bubble = (
    <span
      ref={bubbleRef}
      className={`tooltip-bubble is-floating ${isOpen ? "is-open" : ""}`}
      dir="auto"
      id={tooltipId}
      role="tooltip"
      style={style}
    >
      {label}
    </span>
  );

  const hostClassName = [
    "tooltip-host",
    placement === "right" ? null : "tooltip-host-anchored",
    className
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <span className={hostClassName} ref={hostRef}>
      {children(triggerProps)}
      {isOpen ? <FloatingLayer>{bubble}</FloatingLayer> : null}
    </span>
  );
}
