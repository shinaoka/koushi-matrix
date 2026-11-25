import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

import { t } from "../../i18n/messages";
import { EntityAvatar } from "../Shell";
import {
  FloatingLayer,
  floatingPlacementStyle,
  useFloatingPlacement
} from "../floatingLayer";
import { Tooltip } from "../Tooltip";
import type { AvatarImage } from "../../domain/types";

/** One person in a {@link PersonListPopover}. */
export type PersonListRow = {
  user_id: string;
  display_name: string;
  initials: string;
  avatar: AvatarImage | null;
  /** Explicit avatar resource URL; `null` suppresses the unscoped fallback. */
  source_url?: string | null;
  /** Trailing muted text: a receipt timestamp, the "you" marker, nothing. */
  meta?: string | null;
  meta_class_name?: string;
};

/** Whether `target` sits inside `container`; shared by both person surfaces. */
export function containsTarget(container: Element | null, target: EventTarget | null): boolean {
  return typeof Node !== "undefined" && target instanceof Node && container?.contains(target) === true;
}

/**
 * The surface a person popover must stay inside: the thread pane is
 * overflow-clipped, so a row-local popup would be cut off at its edge.
 */
export function personListBoundaryElement(anchor: Element): Element | null {
  return anchor.closest(".thread-pane") ?? anchor.closest(".main-pane");
}

/** Scroll distance from the end that still counts as "the reader reached it". */
const NEAR_END_PX = 24;

/**
 * A fixed, one-person-per-row popover anchored to a summary element.
 *
 * #2098 built this for read receipts; #2111 shares the same part for the people
 * who reacted. The box is sized by its content (the `.receipt-tooltip` chrome),
 * rows keep their natural height, arrow/Home/End/Page move a roving focus that
 * is always visible, and Escape closes it. The caller owns the rows: one that
 * has every row passes `onActivate` to make each row open that person, and one
 * that pages its rows passes `onFocusIndex`/`onScrollNearEnd` to fetch more.
 */
export function PersonListPopover({
  anchorRef,
  open,
  label,
  rows,
  totalCount,
  startIndex = 0,
  busy = false,
  failed = false,
  loadingText = "",
  failedText = "",
  overflowCount,
  overflowText,
  dismissOnPointerLeave = false,
  focusOnOpen = false,
  resolveBoundaryElement,
  popupRef: popupRefProp,
  rowElements: rowElementsProp,
  onRequestAvatarThumbnail,
  onActivate,
  onFocusIndex,
  onScrollNearEnd,
  onClose
}: {
  anchorRef: React.RefObject<Element | null>;
  open: boolean;
  label: string;
  rows: readonly PersonListRow[];
  /** The full size the rows are a window of; also the roving order's length. */
  totalCount: number;
  startIndex?: number;
  busy?: boolean;
  failed?: boolean;
  loadingText?: string;
  failedText?: string;
  overflowCount: number;
  overflowText: string;
  /** Receipts keep the hover popup's semantics: leaving the box closes it. */
  dismissOnPointerLeave?: boolean;
  /**
   * #2111: a list opened by a right-click or a hold takes the keyboard focus it
   * would otherwise not have, so the arrow keys work immediately. A hover popup
   * leaves focus where the reader put it.
   */
  focusOnOpen?: boolean;
  resolveBoundaryElement?: (anchor: Element) => Element | null;
  popupRef?: React.RefObject<HTMLSpanElement | null>;
  rowElements?: Map<string, HTMLElement>;
  onRequestAvatarThumbnail?: (mxcUri: string) => void | Promise<void | (() => void)>;
  onActivate?: (row: PersonListRow) => void;
  onFocusIndex?: (index: number) => void;
  onScrollNearEnd?: () => void;
  onClose: () => void;
}) {
  const ownPopupRef = useRef<HTMLSpanElement>(null);
  const popupRef = popupRefProp ?? ownPopupRef;
  const ownRowElements = useRef(new Map<string, HTMLElement>());
  const rowElements = rowElementsProp ?? ownRowElements.current;
  const [focusedUserId, setFocusedUserId] = useState<string | null>(null);
  const pendingFocusIndexRef = useRef<number | null>(null);
  // A caller that pages focuses a row its next window brings in, so the pending
  // index survives until `rows`/`startIndex` change.
  const focusRow = useCallback(
    (index: number) => {
      const row = rows[index - startIndex];
      if (row) {
        pendingFocusIndexRef.current = null;
        setFocusedUserId(row.user_id);
        rowElements.get(row.user_id)?.focus();
        return;
      }
      if (!onFocusIndex) return;
      pendingFocusIndexRef.current = index;
      onFocusIndex(index);
    },
    [onFocusIndex, rowElements, rows, startIndex]
  );
  useLayoutEffect(() => {
    const pending = pendingFocusIndexRef.current;
    if (pending === null) return;
    const row = rows[pending - startIndex];
    if (!row) return;
    pendingFocusIndexRef.current = null;
    setFocusedUserId(row.user_id);
    rowElements.get(row.user_id)?.focus();
  }, [rowElements, rows, startIndex]);
  useEffect(() => {
    if (!open) {
      pendingFocusIndexRef.current = null;
      setFocusedUserId(null);
    }
  }, [open]);
  useEffect(() => {
    if (!open || !focusOnOpen) return;
    let frame: number | null = null;
    let attempts = 0;
    const focusFirst = () => {
      frame = null;
      const first = rows[0];
      // A group whose names Core did not send still has a dialog to open, and a
      // keyboard reader must be able to escape it.
      const element = (first ? rowElements.get(first.user_id) : null) ?? popupRef.current;
      if (!element) return;
      if (first) setFocusedUserId(first.user_id);
      element.focus();
      // The box is `visibility: hidden` until the floating layer has measured
      // it, and a hidden element cannot take focus, so retry for a few frames.
      if (document.activeElement !== element && attempts < 4) {
        attempts += 1;
        frame = requestAnimationFrame(focusFirst);
      }
    };
    frame = requestAnimationFrame(focusFirst);
    return () => {
      if (frame !== null) cancelAnimationFrame(frame);
    };
    // Only on open: a window that grows must not pull focus again.
  }, [focusOnOpen, open]);
  useEffect(() => {
    if (!open || dismissOnPointerLeave) return;
    // A right-click popover has no hover state to fall back on, so a press
    // anywhere outside the summary or the box closes it.
    const handlePointerDown = (event: PointerEvent) => {
      if (
        containsTarget(anchorRef.current, event.target) ||
        containsTarget(popupRef.current, event.target)
      ) {
        return;
      }
      onClose();
    };
    document.addEventListener("pointerdown", handlePointerDown, true);
    return () => document.removeEventListener("pointerdown", handlePointerDown, true);
  }, [anchorRef, dismissOnPointerLeave, onClose, open, popupRef]);
  const floatingPlacement = useFloatingPlacement({
    // #2098 tuned the placement and both surfaces want it: the box opens above
    // its summary, end-aligned, and the resolver flips it when that side cannot
    // hold it, so the popup never covers the row below the reader's own.
    align: "end",
    anchorRef,
    // Rows with a summary own this component; only an open popup measures.
    enabled: open,
    // The popup measures itself, so its box is the width its rows need (capped
    // in CSS) instead of a width chosen by the caller.
    contentRef: popupRef,
    placement: "above",
    resolveBoundaryElement
  });

  if (!open) return null;
  const interactiveRows = onActivate !== undefined;
  // A redaction or a new window can drop the remembered row; the list then keeps
  // a tab stop on its first row instead of losing every one of them.
  const focusedRowIsPresent = rows.some((row) => row.user_id === focusedUserId);
  const rowTabIndex = (index: number): number =>
    (focusedRowIsPresent ? rows[index].user_id === focusedUserId : index === 0) ? 0 : -1;
  const putRow = (userId: string) => (node: HTMLSpanElement | HTMLButtonElement | null) => {
    if (node) rowElements.set(userId, node);
    else rowElements.delete(userId);
  };
  return (
    <FloatingLayer>
      <span
        ref={popupRef}
        className="receipt-tooltip"
        role="dialog"
        // Reachable when the list has no row to focus (an unnamed group).
        tabIndex={-1}
        aria-label={label}
        aria-busy={busy}
        style={{
          // #2098: the popup is sized by its content, so the layer owns only the
          // position. Writing the measured size back would make the next
          // measurement read its own pin.
          ...floatingPlacementStyle(floatingPlacement, false),
          maxBlockSize: "min(60vh, 420px)",
          overflowY: "auto"
        }}
        onBlur={(event) => {
          const nextTarget = event.relatedTarget;
          if (
            !nextTarget ||
            (!containsTarget(anchorRef.current, nextTarget) &&
              !containsTarget(popupRef.current, nextTarget))
          ) {
            onClose();
          }
        }}
        onMouseLeave={dismissOnPointerLeave ? () => onClose() : undefined}
        onKeyDown={(event) => {
          if (event.key === "Escape" && !event.nativeEvent.isComposing && event.keyCode !== 229) {
            event.preventDefault();
            event.stopPropagation();
            onClose();
            return;
          }
          if (
            !["ArrowDown", "ArrowUp", "Home", "End", "PageDown", "PageUp"].includes(event.key) ||
            totalCount <= 0
          ) {
            return;
          }
          const focusedIndex = rows.findIndex(
            (row) => rowElements.get(row.user_id) === document.activeElement
          );
          const currentIndex = focusedIndex >= 0 ? startIndex + focusedIndex : startIndex;
          const pageSize = Math.max(rows.length, 1);
          // Only a caller that pages can reach a row it has not loaded; a
          // self-contained list ends at its own last row.
          const lastIndex = onFocusIndex ? totalCount - 1 : startIndex + rows.length - 1;
          const targetIndex =
            event.key === "ArrowDown"
              ? currentIndex + 1
              : event.key === "ArrowUp"
                ? currentIndex - 1
                : event.key === "Home"
                  ? 0
                  : event.key === "End"
                    ? lastIndex
                    : event.key === "PageDown"
                      ? currentIndex + pageSize
                      : currentIndex - pageSize;
          event.preventDefault();
          focusRow(Math.max(startIndex, Math.min(targetIndex, lastIndex)));
        }}
        onScroll={(event) => {
          if (!onScrollNearEnd) return;
          const element = event.currentTarget;
          if (element.scrollTop + element.clientHeight < element.scrollHeight - NEAR_END_PX) {
            return;
          }
          onScrollNearEnd();
        }}
      >
        <Tooltip label={t("shortcut.closeDialogOrMenu")} placement="below" align="end">
          {(tooltipProps) => (
            <button
              type="button"
              className="receipt-reader-close"
              aria-label={t("shortcut.closeDialogOrMenu")}
              onClick={onClose}
              {...tooltipProps}
            >
              ×
            </button>
          )}
        </Tooltip>
        <span role="list" aria-label={label}>
          {busy && rows.length === 0 ? (
            <span role="status">{loadingText}</span>
          ) : failed ? (
            <span role="status">{failedText}</span>
          ) : (
            <>
              {rows.map((row, index) => {
                const body = (
                  <>
                    <EntityAvatar
                      avatar={row.avatar}
                      className="receipt-reader-avatar"
                      colorSeed={row.user_id}
                      fallback={row.initials}
                      onRequestAvatarThumbnail={onRequestAvatarThumbnail}
                      sourceUrl={row.source_url}
                    />
                    <span className="receipt-reader-name" dir="auto" title={row.display_name}>
                      {row.display_name}
                    </span>
                    {row.meta ? (
                      <span className={row.meta_class_name ?? "receipt-reader-time"} dir="auto">
                        {row.meta}
                      </span>
                    ) : null}
                  </>
                );
                const rowAttributes = {
                  "aria-posinset": startIndex + index + 1,
                  "aria-setsize": totalCount,
                  className: "receipt-reader-row",
                  role: "listitem" as const
                };
                if (interactiveRows) {
                  return (
                    <span {...rowAttributes} key={row.user_id}>
                      <button
                        ref={putRow(row.user_id)}
                        type="button"
                        className="receipt-reader-row-button"
                        tabIndex={rowTabIndex(index)}
                        aria-label={row.meta ? `${row.display_name}, ${row.meta}` : row.display_name}
                        onFocus={() => setFocusedUserId(row.user_id)}
                        onClick={() => onActivate?.(row)}
                      >
                        {body}
                      </button>
                    </span>
                  );
                }
                return (
                  <span
                    {...rowAttributes}
                    key={row.user_id}
                    ref={putRow(row.user_id)}
                    tabIndex={rowTabIndex(index)}
                    onFocus={() => setFocusedUserId(row.user_id)}
                  >
                    {body}
                  </span>
                );
              })}
              {overflowCount > 0 ? (
                <span className="receipt-reader-row" dir="auto">
                  {overflowText}
                </span>
              ) : null}
            </>
          )}
        </span>
      </span>
    </FloatingLayer>
  );
}
