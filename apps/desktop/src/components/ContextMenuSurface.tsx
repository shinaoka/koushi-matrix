import type { ContextMenuActionId, ContextMenuItem } from "../domain/contextMenus";
import { t } from "../i18n/messages";
import { useEffect, useRef } from "react";

const MENU_WIDTH = 184;
const MENU_ITEM_HEIGHT = 34;
const MENU_BORDER_HEIGHT = 2;
const VIEWPORT_PADDING = 8;

export function contextMenuPosition({
  itemCount,
  viewport,
  x,
  y
}: {
  itemCount: number;
  viewport: { height: number; width: number } | null;
  x: number;
  y: number;
}) {
  if (!viewport) {
    return { left: x, top: y };
  }

  const menuHeight = itemCount * MENU_ITEM_HEIGHT + MENU_BORDER_HEIGHT;
  const maxLeft = Math.max(VIEWPORT_PADDING, viewport.width - MENU_WIDTH - VIEWPORT_PADDING);
  const maxTop = Math.max(VIEWPORT_PADDING, viewport.height - menuHeight - VIEWPORT_PADDING);

  return {
    left: clamp(x, VIEWPORT_PADDING, maxLeft),
    top: clamp(y, VIEWPORT_PADDING, maxTop)
  };
}

function clamp(value: number, min: number, max: number) {
  return Math.min(Math.max(value, min), max);
}

/**
 * Roving focus over a menu's `[role="menuitem"]` children (#480 audit).
 * Arrow Up/Down move and wrap, Home/End jump to the ends. The browser keeps
 * the focused item visible, so no manual scroll management is needed.
 *
 * Roving stays within the focused item's own menu level: when focus is inside
 * a nested `[role="menu"]` (e.g. a forwarding submenu), navigation applies to
 * that submenu's items only, so a submenu keeps its own focus ownership.
 */
export function moveRovingMenuFocus(
  container: HTMLElement,
  target: "next" | "previous" | "first" | "last"
): void {
  const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const activeMenu = active?.closest<HTMLElement>('[role="menu"]') ?? null;
  const scope =
    activeMenu && container.contains(activeMenu) && activeMenu !== container
      ? activeMenu
      : container;
  // `[role^="menuitem"]` covers menuitem, menuitemradio, and menuitemcheckbox,
  // so a radio-style menu (a section's sort options) roves the same way.
  const items = Array.from(
    scope.querySelectorAll<HTMLElement>('[role^="menuitem"]:not([hidden])')
  ).filter((item) => item.closest('[role="menu"]') === scope);
  if (items.length === 0) {
    return;
  }
  const currentIndex = active ? items.indexOf(active) : -1;
  let nextIndex: number;
  switch (target) {
    case "first":
      nextIndex = 0;
      break;
    case "last":
      nextIndex = items.length - 1;
      break;
    case "next":
      nextIndex = currentIndex < 0 ? 0 : (currentIndex + 1) % items.length;
      break;
    case "previous":
      nextIndex =
        currentIndex < 0 ? items.length - 1 : (currentIndex + items.length - 1) % items.length;
      break;
  }
  items[nextIndex].focus();
}

/**
 * The row a Tab press moves to inside a menu, or null at either end (#2078).
 *
 * A menu that renders in the body-level floating layer is not in the browser's
 * own Tab order — WebKit leaves such a panel at once — so walking the rows has
 * to be asked for. The caller owns what happens at either end: this only says
 * which row is next.
 */
export function menuTabTarget(
  container: HTMLElement,
  backwards: boolean
): HTMLElement | null {
  const rows = Array.from(
    container.querySelectorAll<HTMLElement>('[role^="menuitem"]:not([hidden])')
  ).filter((item) => item.closest('[role="menu"]') === container);
  const index = rows.indexOf(document.activeElement as HTMLElement);
  if (index < 0) {
    return null;
  }
  return rows[index + (backwards ? -1 : 1)] ?? null;
}

/** Keydown handler for a menu container: arrows rove, Escape bubbles to the caller. */
export function onMenuKeyDown(
  event: React.KeyboardEvent<HTMLElement>,
  container: HTMLElement | null
): void {
  if (event.key === "ArrowDown") {
    event.preventDefault();
    if (container) {
      moveRovingMenuFocus(container, "next");
    }
  } else if (event.key === "ArrowUp") {
    event.preventDefault();
    if (container) {
      moveRovingMenuFocus(container, "previous");
    }
  } else if (event.key === "Home") {
    event.preventDefault();
    if (container) {
      moveRovingMenuFocus(container, "first");
    }
  } else if (event.key === "End") {
    event.preventDefault();
    if (container) {
      moveRovingMenuFocus(container, "last");
    }
  }
}

export function ContextMenuSurface({
  items,
  x,
  y,
  onAction,
  onClose
}: {
  items: ContextMenuItem[];
  x: number;
  y: number;
  onAction: (actionId: ContextMenuActionId) => void;
  onClose: () => void;
}) {
  const firstItemRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  // Opener captured during the first render, before the menu's own focus
  // moves, so focus can be returned to the invoking element on close.
  const openerRef = useRef<HTMLElement | null>(null);
  if (openerRef.current === null) {
    openerRef.current = document.activeElement as HTMLElement | null;
  }

  useEffect(() => {
    firstItemRef.current?.focus();
    return () => {
      // Restore focus to the opener only when the menu really unmounted:
      // React StrictMode rehearses effects with the menu still connected, and
      // the opener may have been removed (e.g. its panel closed), in which
      // case forcing focus would drop it onto the body.
      const menu = menuRef.current;
      if (menu && !menu.isConnected && openerRef.current?.isConnected) {
        openerRef.current.focus();
      }
    };
  }, []);

  if (!items.length) {
    return null;
  }

  const position = contextMenuPosition({
    itemCount: items.length,
    viewport:
      typeof window === "undefined" ? null : { height: window.innerHeight, width: window.innerWidth },
    x,
    y
  });

  return (
    <div className="context-menu-backdrop" onClick={onClose}>
      <div
        className="context-menu"
        ref={menuRef}
        role="menu"
        style={{ left: position.left, top: position.top }}
        onClick={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            onClose();
            return;
          }
          onMenuKeyDown(event, event.currentTarget);
        }}
      >
        {items.map((item, index) => (
          <button
            className={`context-menu-item ${item.destructive ? "destructive" : ""}`.trim()}
            key={item.id}
            ref={index === 0 ? firstItemRef : undefined}
            role="menuitem"
            type="button"
            onClick={() => onAction(item.id)}
          >
            {t(item.labelMessageId)}
          </button>
        ))}
      </div>
    </div>
  );
}
