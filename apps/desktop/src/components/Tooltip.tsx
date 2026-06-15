import {
  type FocusEvent,
  type KeyboardEvent,
  type MouseEvent,
  type ReactNode,
  useEffect,
  useId,
  useRef,
  useState
} from "react";

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

export function Tooltip({ children, label, placement = "right", delayMs = 250 }: TooltipProps) {
  const tooltipId = useId();
  const [isOpen, setIsOpen] = useState(false);
  const openTimer = useRef<number | null>(null);

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

  return (
    <span className={`tooltip-host tooltip-host-${placement}`}>
      {children(triggerProps)}
      <span
        className={`tooltip-bubble ${isOpen ? "is-open" : ""}`}
        dir="auto"
        id={tooltipId}
        role="tooltip"
      >
        {label}
      </span>
    </span>
  );
}
