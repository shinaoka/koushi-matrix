import type { ContextMenuActionId, ContextMenuItem } from "../domain/contextMenus";

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
        role="menu"
        style={{ left: position.left, top: position.top }}
        onClick={(event) => event.stopPropagation()}
      >
        {items.map((item) => (
          <button
            className={`context-menu-item ${item.destructive ? "destructive" : ""}`.trim()}
            key={item.id}
            role="menuitem"
            type="button"
            onClick={() => onAction(item.id)}
          >
            {item.label}
          </button>
        ))}
      </div>
    </div>
  );
}
