import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { ChevronDown, ChevronUp } from "lucide-react";
import { t } from "../i18n/messages";
import type { SpaceRailItem } from "../domain/types";
import { ICON_SIZE } from "../app/uiShared";
import { Tooltip } from "./Tooltip";
import {
  EMPTY_RAIL_OVERFLOW,
  nearestRailScrollTop,
  sameRailOverflow,
  summarizeRailOverflow,
  type RailEdgeAttention,
  type RailItemMeasure,
  type RailOverflow,
  type RailViewport
} from "./workspaceRailOverflow";

/** Reads the rail's edge inset (the fade height) from its CSS token. */
function workspaceRailEdgeInset(list: HTMLElement): number {
  const value = Number.parseFloat(
    getComputedStyle(list).getPropertyValue("--workspace-rail-edge-inset")
  );
  return Number.isFinite(value) ? value : 0;
}

function measureWorkspaceRail(
  list: HTMLElement,
  rail: readonly SpaceRailItem[]
): { items: RailItemMeasure[]; viewport: RailViewport } {
  const listTop = list.getBoundingClientRect().top;
  const buttons = new Map<string, HTMLElement>();
  for (const button of list.querySelectorAll<HTMLElement>("[data-space-id]")) {
    buttons.set(button.dataset.spaceId ?? "", button);
  }
  const items: RailItemMeasure[] = [];
  for (const space of rail) {
    const button = buttons.get(space.space_id);
    if (!button) continue;
    const box = button.getBoundingClientRect();
    const top = box.top - listTop + list.scrollTop;
    items.push({
      spaceId: space.space_id,
      top,
      bottom: top + box.height,
      unreadCount: space.unread_count,
      highlightCount: space.highlight_count
    });
  }
  return {
    items,
    viewport: {
      scrollTop: list.scrollTop,
      clientHeight: list.clientHeight,
      scrollHeight: list.scrollHeight
    }
  };
}

/**
 * #1509: keeps the active Space inside the scrolling rail and summarises
 * attention on Spaces scrolled out of view. Only viewport measurements and the
 * scroll position live here (presentation state); every count is the
 * Rust-projected `SpaceRailItem` value. Rail buttons carry `data-space-id`.
 */
export function useWorkspaceRailOverflow(rail: readonly SpaceRailItem[]) {
  const listRef = useRef<HTMLDivElement | null>(null);
  const railRef = useRef(rail);
  railRef.current = rail;
  const frameRef = useRef<number | null>(null);
  const [overflow, setOverflow] = useState<RailOverflow>(EMPTY_RAIL_OVERFLOW);
  const activeSpaceId = rail.find((space) => space.is_active)?.space_id ?? null;
  const railSignature = rail
    .map(
      (space) =>
        `${space.space_id}\u0000${space.display_name}\u0000${space.unread_count}\u0000${space.highlight_count}\u0000${space.is_active ? 1 : 0}`
    )
    .join("\u0001");
  const previousActiveRef = useRef<string | null | undefined>(undefined);

  const measure = useCallback(() => {
    const list = listRef.current;
    if (!list) return;
    const { items, viewport } = measureWorkspaceRail(list, railRef.current);
    const next = summarizeRailOverflow(items, viewport, workspaceRailEdgeInset(list));
    setOverflow((current) => (sameRailOverflow(current, next) ? current : next));
  }, []);

  const scrollSpaceIntoView = useCallback((spaceId: string): HTMLElement | null => {
    const list = listRef.current;
    if (!list) return null;
    const { items, viewport } = measureWorkspaceRail(list, railRef.current);
    const item = items.find((candidate) => candidate.spaceId === spaceId);
    if (!item) return null;
    const target = nearestRailScrollTop(item, viewport, workspaceRailEdgeInset(list));
    if (target !== null) list.scrollTop = target;
    return (
      Array.from(list.querySelectorAll<HTMLElement>("[data-space-id]")).find(
        (button) => button.dataset.spaceId === spaceId
      ) ?? null
    );
  }, []);

  useLayoutEffect(() => {
    // Selecting a Space by any path (Activity, a notification, a room link,
    // restored navigation) only moves `is_active`; keep that item in view.
    // Gated on the active id alone, so count updates never undo a manual
    // rail scroll.
    if (previousActiveRef.current !== activeSpaceId) {
      previousActiveRef.current = activeSpaceId;
      if (activeSpaceId !== null) scrollSpaceIntoView(activeSpaceId);
    }
    // Rail membership, order and counts all change the summary; the value
    // comparison keeps an unchanged result from re-rendering.
    measure();
    // #1889: gated on what the summary actually reads. Without deps this ran
    // after every render of the shell, re-measuring the rail once per pass of a
    // room switch even though nothing about the rail had changed.
  }, [activeSpaceId, railSignature, measure, scrollSpaceIntoView]);

  useEffect(() => {
    const list = listRef.current;
    if (!list || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => measure());
    observer.observe(list);
    // #1889: button geometry can change without the rail data changing (a
    // rewrap, a font or density change), and that no longer re-runs the layout
    // effect. The buttons are re-observed when the rail changes because the
    // observed element has to exist when the observer is created.
    for (const button of list.querySelectorAll<HTMLElement>("[data-space-id]")) {
      observer.observe(button);
    }
    return () => {
      observer.disconnect();
      if (frameRef.current !== null) cancelAnimationFrame(frameRef.current);
      frameRef.current = null;
    };
    // `measure` reads only refs, so the observer is recreated only with the rail.
  }, [measure, railSignature]);

  function onListScroll() {
    if (frameRef.current !== null) return;
    frameRef.current = requestAnimationFrame(() => {
      frameRef.current = null;
      measure();
    });
  }

  function revealSpace(spaceId: string) {
    const button = scrollSpaceIntoView(spaceId);
    // The indicator may disappear once the Space is visible; move focus to the
    // Space it revealed so keyboard users are not dropped on the document.
    button?.focus({ preventScroll: true });
    measure();
  }

  return { listRef, overflow, onListScroll, revealSpace };
}

export function WorkspaceRailEdgeIndicator({
  edge,
  attention,
  onReveal
}: {
  edge: "above" | "below";
  attention: RailEdgeAttention;
  onReveal: (spaceId: string) => void;
}) {
  const values = { count: String(attention.count) };
  const label =
    edge === "above"
      ? t(attention.hasHighlight ? "workspace.railMentionsAbove" : "workspace.railActivityAbove", values)
      : t(attention.hasHighlight ? "workspace.railMentionsBelow" : "workspace.railActivityBelow", values);
  const Icon = edge === "above" ? ChevronUp : ChevronDown;
  return (
    <Tooltip label={label}>
      {(tooltipProps) => (
        <button
          className={`workspace-rail-edge-indicator is-${edge}`}
          data-highlight={attention.hasHighlight || undefined}
          type="button"
          aria-label={label}
          onClick={() => onReveal(attention.nearestSpaceId)}
          {...tooltipProps}
        >
          <Icon size={ICON_SIZE.micro} aria-hidden="true" />
        </button>
      )}
    </Tooltip>
  );
}
