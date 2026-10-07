import {
  type CSSProperties,
  type FocusEvent,
  type KeyboardEvent,
  type MouseEvent,
  type ReactNode,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState
} from "react";

import { FloatingLayer } from "./floatingLayer";

type TooltipTriggerProps = {
  "aria-describedby"?: string;
  onBlur: (event: FocusEvent<HTMLElement>) => void;
  onFocus: (event: FocusEvent<HTMLElement>) => void;
  onKeyDown: (event: KeyboardEvent<HTMLElement>) => void;
  onMouseEnter: (event: MouseEvent<HTMLElement>) => void;
  onMouseLeave: (event: MouseEvent<HTMLElement>) => void;
};

type TooltipProps = {
  children: (props: TooltipTriggerProps) => ReactNode;
  label: string;
  placement?: "right";
  delayMs?: number;
};

/** Keeps the bubble away from the viewport edges. */
const TOOLTIP_VIEWPORT_MARGIN_PX = 12;
/** Distance between the trigger and the bubble. */
const TOOLTIP_ANCHOR_GAP_PX = 8;

export function Tooltip({ children, label, placement = "right", delayMs = 250 }: TooltipProps) {
  const tooltipId = useId();
  const [isOpen, setIsOpen] = useState(false);
  const openTimer = useRef<number | null>(null);
  const hostRef = useRef<HTMLSpanElement>(null);
  const bubbleRef = useRef<HTMLSpanElement>(null);
  const [style, setStyle] = useState<CSSProperties>({ visibility: "hidden" });

  function clearOpenTimer() {
    if (openTimer.current !== null) {
      window.clearTimeout(openTimer.current);
      openTimer.current = null;
    }
  }

  function openAfterDelay() {
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
    clearOpenTimer();
    setIsOpen(true);
  }

  function close() {
    clearOpenTimer();
    setIsOpen(false);
  }

  useEffect(() => {
    return () => clearOpenTimer();
  }, []);

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
  useLayoutEffect(() => {
    if (!isOpen) {
      setStyle({ visibility: "hidden" });
      return;
    }
    const host = hostRef.current;
    const bubble = bubbleRef.current;
    if (!host) {
      return;
    }
    const anchor = host.getBoundingClientRect();
    const size = bubble?.getBoundingClientRect();
    const width = size?.width ?? 0;
    const height = size?.height ?? 0;
    const viewportWidth = window.innerWidth;
    const viewportHeight = window.innerHeight;
    const wantsRight = placement === "right";
    let left = wantsRight
      ? anchor.right + TOOLTIP_ANCHOR_GAP_PX
      : anchor.left - TOOLTIP_ANCHOR_GAP_PX - width;
    if (left + width > viewportWidth - TOOLTIP_VIEWPORT_MARGIN_PX) {
      const flipped = anchor.left - TOOLTIP_ANCHOR_GAP_PX - width;
      left = flipped >= TOOLTIP_VIEWPORT_MARGIN_PX
        ? flipped
        : Math.max(TOOLTIP_VIEWPORT_MARGIN_PX, viewportWidth - TOOLTIP_VIEWPORT_MARGIN_PX - width);
    }
    if (left < TOOLTIP_VIEWPORT_MARGIN_PX) {
      left = TOOLTIP_VIEWPORT_MARGIN_PX;
    }
    const centered = anchor.top + anchor.height / 2 - height / 2;
    const top = Math.min(
      Math.max(centered, TOOLTIP_VIEWPORT_MARGIN_PX),
      Math.max(
        TOOLTIP_VIEWPORT_MARGIN_PX,
        viewportHeight - TOOLTIP_VIEWPORT_MARGIN_PX - height
      )
    );
    setStyle({ position: "fixed", left: `${left}px`, top: `${top}px`, visibility: "visible" });
  }, [isOpen, label, placement]);

  const triggerProps: TooltipTriggerProps = {
    "aria-describedby": isOpen ? tooltipId : undefined,
    onBlur: close,
    onFocus: openNow,
    onKeyDown: (event) => {
      if (event.key === "Escape") {
        close();
      }
    },
    onMouseEnter: openAfterDelay,
    onMouseLeave: close
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

  return (
    <span className="tooltip-host tooltip-host-floating" ref={hostRef}>
      {children(triggerProps)}
      {isOpen ? <FloatingLayer>{bubble}</FloatingLayer> : null}
    </span>
  );
}
