import { Search, X } from "lucide-react";
import {
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type RefObject,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import {
  FloatingLayer,
  type FloatingRect,
  floatingPlacementStyle,
  pinnedPlacementStyle,
  useFloatingPlacement,
} from "./floatingLayer";
import { t } from "../i18n/messages";
import { ImeTextField } from "./ImeTextControl";
import { Tooltip } from "./Tooltip";
import { useEmojiPickerSize } from "./useEmojiPickerSize";
import {
  EMOJI_BY_CATEGORY,
  EMOJI_CATEGORIES,
  type EmojiCategory,
  type EmojiEntry,
} from "./emojiData";

const MAX_RECENT = 24;
const EMOJI_CATEGORY_ICONS: Record<EmojiCategory, string> = {
  people: "😀",
  nature: "🐕",
  foods: "🍎",
  activity: "⚽️",
  places: "🚗",
  objects: "💡",
  symbols: "⁉️",
  flags: "🏁",
};

/**
 * Number of emoji columns in the grid. Single source of truth: it drives both
 * the rendered CSS grid (through `--emoji-picker-columns`) and the ArrowUp /
 * ArrowDown keyboard step, so the two can never disagree.
 */
export const EMOJI_PICKER_GRID_COLUMNS = 10;
/** Preferred panel inline size; narrowed to the available space when smaller. */
export const EMOJI_PICKER_INLINE_SIZE_PX = 380;
/** Preferred panel block size; narrowed to the available space when smaller. */
export const EMOJI_PICKER_BLOCK_SIZE_PX = 520;

/**
 * Placement inputs kept as named constants: the picker prefers opening above
 * its trigger, and only flips when a comfortable panel would not fit.
 */
const EMOJI_PICKER_COMFORTABLE_BLOCK_SIZE_PX = 360;

const MIN_PICKER_WIDTH = 300;
const MIN_PICKER_HEIGHT = 200;
const MAX_PICKER_SIZE = 1200;
const RESIZE_KEY_STEP = 20;

function constrainPickerSize(width: number, height: number): [number, number] {
  return [
    Math.round(Math.max(MIN_PICKER_WIDTH, Math.min(MAX_PICKER_SIZE, width))),
    Math.round(Math.max(MIN_PICKER_HEIGHT, Math.min(MAX_PICKER_SIZE, height))),
  ];
}

interface PickerResizeDrag {
  pointer: { x: number; y: number };
  /** Panel rectangle when the drag began: the grabbed edges move from here. */
  rect: FloatingRect;
  bounds: FloatingRect;
  /** Physical edges the grabbed handle moves: -1 is the left/top one. */
  horizontal: -1 | 0 | 1;
  vertical: -1 | 0 | 1;
  /** Preferred size, the value committed on release. */
  size: [number, number];
  /** Marked corner, pinned for the drag so an alignment flip cannot move it. */
  marked: FrameZoneName;
  moved: boolean;
}

type FrameZoneName =
  | "top"
  | "bottom"
  | "left"
  | "right"
  | "top-left"
  | "top-right"
  | "bottom-left"
  | "bottom-right";

interface FrameZone {
  name: FrameZoneName;
  horizontal: -1 | 0 | 1;
  vertical: -1 | 0 | 1;
}

/** The whole frame resizes: four edges, four corners (#1583). */
const FRAME_ZONES: FrameZone[] = [
  { horizontal: 0, name: "top", vertical: -1 },
  { horizontal: 0, name: "bottom", vertical: 1 },
  { horizontal: -1, name: "left", vertical: 0 },
  { horizontal: 1, name: "right", vertical: 0 },
  { horizontal: -1, name: "top-left", vertical: -1 },
  { horizontal: 1, name: "top-right", vertical: -1 },
  { horizontal: -1, name: "bottom-left", vertical: 1 },
  { horizontal: 1, name: "bottom-right", vertical: 1 },
];

/**
 * Logical insets place a frame zone; the cursor names the physical edge pair
 * because pointer coordinates are physical. A resize strip on an inline-side
 * in RTL is the physical left one, so the inline side is mapped through the
 * root direction.
 */
function frameZoneClass(zone: FrameZone, rtl: boolean, marked: boolean): string {
  const position: string[] = [];
  if (zone.vertical < 0) position.push("at-block-start");
  if (zone.vertical > 0) position.push("at-block-end");
  if (zone.horizontal !== 0) {
    position.push((zone.horizontal < 0) !== rtl ? "at-inline-start" : "at-inline-end");
  }
  const shape =
    zone.horizontal !== 0 && zone.vertical !== 0
      ? "is-corner"
      : zone.horizontal !== 0
        ? "is-edge-inline"
        : "is-edge-block";
  const cursor =
    zone.horizontal !== 0 && zone.vertical !== 0
      ? zone.horizontal === zone.vertical
        ? "is-nwse"
        : "is-nesw"
      : zone.horizontal !== 0
        ? "is-ew"
        : "is-ns";
  return ["emoji-picker-resize", shape, cursor, ...position, marked ? "is-marked" : ""]
    .filter(Boolean)
    .join(" ");
}

interface EmojiPickerProps {
  onSelect: (emoji: string) => void;
  onClose: () => void;
  size?: [number, number] | null;
  onSizeChange?: (size: [number, number]) => void;
  recentEmojis?: string[];
  onRecentEmojisChange?: (emojis: string[]) => void;
  /** Element that triggered the picker; excluded from outside-click detection
   * so the trigger button can handle its own toggle without the picker
   * re-opening after the outside-click handler fires. It is also the anchor
   * rectangle the floating layer is positioned against. */
  anchorRef?: RefObject<Element | null>;
  /**
   * Resolves an extra container the panel must stay inside. Layout knowledge
   * stays with the caller; the panel re-resolves it on every measurement so a
   * resize or scroll cannot leave a stale boundary behind.
   */
  resolveBoundaryElement?: (anchor: Element) => Element | null;
  placement?: "above" | "below";
  align?: "start" | "end";
  className?: string;
}

export function EmojiPicker({
  onSelect,
  onClose,
  size,
  onSizeChange,
  recentEmojis = [],
  onRecentEmojisChange,
  anchorRef,
  resolveBoundaryElement,
  placement = "above",
  align = "start",
  className,
}: EmojiPickerProps) {
  const savedSize = useEmojiPickerSize();
  size ??= savedSize.size;
  onSizeChange ??= savedSize.save;
  const [draftSize, setDraftSize] = useState<[number, number]>(() =>
    size ? constrainPickerSize(...size) : [EMOJI_PICKER_INLINE_SIZE_PX, EMOJI_PICKER_BLOCK_SIZE_PX]
  );
  const drag = useRef<PickerResizeDrag | null>(null);
  /** Rectangle a pointer resize pinned; until it is set the panel is anchored. */
  const [pinnedGeometry, setPinnedGeometry] = useState<{
    left: number;
    marked: FrameZoneName;
    top: number;
  } | null>(null);
  const [query, setQuery] = useState("");
  const [activeCategory, setActiveCategory] = useState<
    EmojiCategory | "recent"
  >("people");
  const panelRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const categoryRefs = useRef<Record<EmojiCategory, HTMLDivElement | null>>({
    people: null,
    nature: null,
    foods: null,
    activity: null,
    places: null,
    objects: null,
    symbols: null,
    flags: null,
  });
  const recentRef = useRef<HTMLDivElement | null>(null);

  const trimmedQuery = query.trim().toLowerCase();
  const searching = trimmedQuery.length > 0;

  const filtered = useMemo(() => {
    if (!searching) {
      return null;
    }
    const results: EmojiEntry[] = [];
    for (const category of EMOJI_CATEGORIES) {
      for (const entry of EMOJI_BY_CATEGORY[category]) {
        if (entry.search.includes(trimmedQuery)) {
          results.push(entry);
        }
      }
    }
    return results;
  }, [searching, trimmedQuery]);

  const recentEntries = useMemo(() => {
    const all = new Map<string, EmojiEntry>();
    for (const category of EMOJI_CATEGORIES) {
      for (const entry of EMOJI_BY_CATEGORY[category]) {
        all.set(entry.emoji, entry);
      }
    }
    return recentEmojis
      .map((emoji) => all.get(emoji))
      .filter((entry): entry is EmojiEntry => entry != null);
  }, [recentEmojis]);

  const resolvedPlacement = useFloatingPlacement({
    align,
    anchorRef,
    blockSize: draftSize[1],
    comfortableBlockSize: EMOJI_PICKER_COMFORTABLE_BLOCK_SIZE_PX,
    inlineSize: draftSize[0],
    placement,
    resolveBoundaryElement
  });

  const resizeAbove = resolvedPlacement?.placement === "above";
  const resizeLeft = (resolvedPlacement?.align === "end") !== (document.documentElement.dir === "rtl");
  /** The marked corner is the one furthest from the anchor; a pinned panel keeps
   * the corner it was resized from, whatever the resolver does afterwards. */
  const markedZone: FrameZoneName =
    pinnedGeometry?.marked ??
    (`${resizeAbove ? "top" : "bottom"}-${resizeLeft ? "left" : "right"}` as FrameZoneName);

  const initiallyFocused = useRef(false);
  useLayoutEffect(() => {
    if (resolvedPlacement && searchRef.current && !initiallyFocused.current) {
      searchRef.current.focus();
      initiallyFocused.current = true;
    }
  }, [resolvedPlacement]);

  useEffect(() => {
    function handleClickOutside(event: MouseEvent) {
      if (
        panelRef.current &&
        !panelRef.current.contains(event.target as Node) &&
        !(anchorRef?.current?.contains(event.target as Node) ?? false)
      ) {
        onClose();
      }
    }
    document.addEventListener("mousedown", handleClickOutside);
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
    };
  }, [onClose, anchorRef]);

  const handleSelect = useCallback(
    (emoji: string) => {
      onRecentEmojisChange?.(
        [emoji, ...recentEmojis.filter((item) => item !== emoji)].slice(0, MAX_RECENT)
      );
      onSelect(emoji);
      onClose();
    },
    [onSelect, onClose, onRecentEmojisChange, recentEmojis],
  );

  const scrollToCategory = useCallback((category: EmojiCategory | "recent") => {
    const node =
      category === "recent"
        ? recentRef.current
        : categoryRefs.current[category];
    if (node) {
      node.scrollIntoView({ block: "start" });
    }
  }, []);

  // Every edge and corner resizes (#1583). The grabbed edges follow the
  // pointer and the opposite ones stay where the drag started, so the anchored
  // placement and its flip heuristic cannot move the panel under the pointer.
  // The pinned rectangle outlives the drag: recomputing it from the anchor on
  // release would snap an anchor-side edge back and fight the gesture.
  const beginResize = (zone: FrameZone) => (event: ReactPointerEvent<HTMLElement>) => {
    if (event.button !== 0 || !panelRef.current || !resolvedPlacement) return;
    event.preventDefault();
    const rect = panelRef.current.getBoundingClientRect();
    drag.current = {
      bounds: resolvedPlacement.bounds,
      horizontal: zone.horizontal,
      marked: markedZone,
      moved: false,
      pointer: { x: event.clientX, y: event.clientY },
      rect: { bottom: rect.bottom, left: rect.left, right: rect.right, top: rect.top },
      size: draftSize,
      vertical: zone.vertical,
    };
    event.currentTarget.setPointerCapture(event.pointerId);
  };

  const moveResize = (event: ReactPointerEvent<HTMLElement>) => {
    const start = drag.current;
    if (!start) return;
    // An axis this handle does not own keeps the preferred size, so resizing
    // one dimension cannot overwrite a window-clamped value on the other.
    start.size = constrainPickerSize(
      start.horizontal === 0
        ? start.size[0]
        : start.rect.right - start.rect.left + start.horizontal * (event.clientX - start.pointer.x),
      start.vertical === 0
        ? start.size[1]
        : start.rect.bottom - start.rect.top + start.vertical * (event.clientY - start.pointer.y),
    );
    start.moved = true;
    const shownWidth = Math.min(start.size[0], start.bounds.right - start.bounds.left);
    const shownHeight = Math.min(start.size[1], start.bounds.bottom - start.bounds.top);
    setDraftSize(start.size);
    setPinnedGeometry({
      left: start.horizontal < 0 ? start.rect.right - shownWidth : start.rect.left,
      marked: start.marked,
      top: start.vertical < 0 ? start.rect.bottom - shownHeight : start.rect.top,
    });
  };

  const endResize = (event: ReactPointerEvent<HTMLElement>) => {
    const start = drag.current;
    if (!start) return;
    drag.current = null;
    event.currentTarget.releasePointerCapture(event.pointerId);
    if (start.moved) onSizeChange?.(start.size);
  };

  const cancelResize = () => {
    drag.current = null;
    setPinnedGeometry(null);
    setDraftSize(size ?? [EMOJI_PICKER_INLINE_SIZE_PX, EMOJI_PICKER_BLOCK_SIZE_PX]);
  };

  // The marked handle is the only one in the tab order, and the arrow keys move
  // its two edges just like a pointer would.
  const keyboardResize = (event: ReactKeyboardEvent<HTMLElement>, zone: FrameZone) => {
    const horizontal = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
    const vertical = event.key === "ArrowDown" ? 1 : event.key === "ArrowUp" ? -1 : 0;
    if (horizontal === 0 && vertical === 0) return;
    event.preventDefault();
    const next = constrainPickerSize(
      draftSize[0] + horizontal * zone.horizontal * RESIZE_KEY_STEP,
      draftSize[1] + vertical * zone.vertical * RESIZE_KEY_STEP,
    );
    setDraftSize(next);
    onSizeChange?.(next);
  };

  // Rendered in a body-level floating layer: the composer lives inside
  // overflow-clipped panes (the thread pane), so an anchored popup would be cut
  // off at the pane boundary. Coordinates are physical because they come from
  // viewport rect measurements; the logical side is carried by `align-*`.
  const panel = (
    <div
      ref={panelRef}
      className={[
        "emoji-picker",
        `is-${resolvedPlacement?.placement ?? placement}`,
        `align-${resolvedPlacement?.align ?? align}`,
        className,
      ]
        .filter(Boolean)
        .join(" ")}
      role="dialog"
      aria-label={t("composer.emoji")}
      style={
        pinnedGeometry && resolvedPlacement
          ? pinnedPlacementStyle(resolvedPlacement, pinnedGeometry, draftSize[0], draftSize[1])
          : floatingPlacementStyle(resolvedPlacement)
      }
      onKeyDown={(event) => {
        if (event.key === "Escape" && !event.defaultPrevented && !event.nativeEvent.isComposing && event.keyCode !== 229) {
          event.preventDefault();
          event.stopPropagation();
          onClose();
          if (anchorRef?.current instanceof HTMLElement) anchorRef.current.focus();
        }
      }}
    >
      {FRAME_ZONES.map((zone) => {
        const className = frameZoneClass(
          zone,
          document.documentElement.dir === "rtl",
          zone.name === markedZone,
        );
        const pointerHandlers = {
          onLostPointerCapture: () => {
            drag.current = null;
          },
          onPointerCancel: cancelResize,
          onPointerDown: beginResize(zone),
          onPointerMove: moveResize,
          onPointerUp: endResize,
        };
        return zone.name === markedZone ? (
          <button
            key={zone.name}
            type="button"
            className={className}
            data-emoji-picker-resize={zone.name}
            aria-label={t("composer.emojiResize")}
            onKeyDown={(event) => keyboardResize(event, zone)}
            {...pointerHandlers}
          >
            <span aria-hidden="true">⋰</span>
          </button>
        ) : (
          <div
            key={zone.name}
            className={className}
            data-emoji-picker-resize={zone.name}
            aria-hidden="true"
            {...pointerHandlers}
          />
        );
      })}
      <div className="emoji-picker-header">
        <div className="emoji-picker-search">
          <Search size={14} aria-hidden="true" />
          <ImeTextField
            ref={searchRef}
            type="search"
            value={query}
            syncKey="emoji-search"
            placeholder={t("composer.emojiSearch")}
            aria-label={t("composer.emojiSearch")}
            onChange={(event) => setQuery(event.currentTarget.value)}
          />
        </div>
        <Tooltip label={t("mediaGallery.close")} placement="below" align="end">
          {(tooltipProps) => (
            <button
              className="icon-button"
              type="button"
              aria-label={t("mediaGallery.close")}
              onClick={onClose}
              {...tooltipProps}
            >
              <X size={14} />
            </button>
          )}
        </Tooltip>
      </div>

      {!searching && (
        <div className="emoji-picker-tabs" role="tablist">
          {recentEntries.length > 0 && (
            <Tooltip label={t("composer.emojiRecent")} placement="below">
              {(tooltipProps) => (
                <button
                  className={`emoji-picker-tab ${activeCategory === "recent" ? "active" : ""}`}
                  type="button"
                  role="tab"
                  aria-selected={activeCategory === "recent"}
                  aria-label={t("composer.emojiRecent")}
                  onClick={() => {
                    setActiveCategory("recent");
                    scrollToCategory("recent");
                  }}
                  {...tooltipProps}
                >
                  <span aria-hidden="true">🕒</span>
                </button>
              )}
            </Tooltip>
          )}
          {EMOJI_CATEGORIES.map((category) => (
            <Tooltip
              key={category}
              label={t(`emoji.category.${category}` as const)}
              placement="below"
            >
              {(tooltipProps) => (
                <button
                  className={`emoji-picker-tab ${activeCategory === category ? "active" : ""}`}
                  type="button"
                  role="tab"
                  aria-selected={activeCategory === category}
                  aria-label={t(`emoji.category.${category}` as const)}
                  onClick={() => {
                    setActiveCategory(category);
                    scrollToCategory(category);
                  }}
                  {...tooltipProps}
                >
                  <span aria-hidden="true">{EMOJI_CATEGORY_ICONS[category]}</span>
                </button>
              )}
            </Tooltip>
          ))}
        </div>
      )}

      <div className="emoji-picker-body">
        {savedSize.failed && <p role="alert">{t("composer.emojiSizeSaveFailed")}</p>}
        {searching ? (
          filtered && filtered.length > 0 ? (
            <EmojiGrid entries={filtered} onSelect={handleSelect} />
          ) : (
            <div className="emoji-picker-empty">{t("emoji.noResults")}</div>
          )
        ) : (
          <>
            {recentEntries.length > 0 && (
              <div
                ref={(node) => {
                  recentRef.current = node;
                }}
                className="emoji-picker-section"
              >
                <h3>{t("composer.emojiRecent")}</h3>
                <EmojiGrid entries={recentEntries} onSelect={handleSelect} />
              </div>
            )}
            {EMOJI_CATEGORIES.map((category) => (
              <div
                key={category}
                ref={(node) => {
                  categoryRefs.current[category] = node;
                }}
                className="emoji-picker-section"
              >
                <h3>{t(`emoji.category.${category}` as const)}</h3>
                <EmojiGrid
                  entries={EMOJI_BY_CATEGORY[category]}
                  onSelect={handleSelect}
                />
              </div>
            ))}
          </>
        )}
      </div>
    </div>
  );

  return <FloatingLayer>{panel}</FloatingLayer>;
}

function EmojiGrid({
  entries,
  onSelect,
}: {
  entries: EmojiEntry[];
  onSelect: (emoji: string) => void;
}) {
  // focusedIndex drives tabIndex so the roving-tabindex pattern is consistent;
  // actual DOM focus is moved synchronously in handleKeyDown so no async
  // React-cycle races arise during Playwright keyboard events.
  const [focusedIndex, setFocusedIndex] = useState<number | null>(null);
  const itemRefs = useRef<(HTMLButtonElement | null)[]>([]);

  // Sync itemRefs array length with entries
  itemRefs.current = itemRefs.current.slice(0, entries.length);

  function handleKeyDown(event: ReactKeyboardEvent<HTMLButtonElement>, index: number) {
    let next: number | null = null;
    if (event.key === "ArrowRight") {
      next = index + 1 < entries.length ? index + 1 : index;
    } else if (event.key === "ArrowLeft") {
      next = index - 1 >= 0 ? index - 1 : index;
    } else if (event.key === "ArrowDown") {
      const candidate = index + EMOJI_PICKER_GRID_COLUMNS;
      next = candidate < entries.length ? candidate : index;
    } else if (event.key === "ArrowUp") {
      const candidate = index - EMOJI_PICKER_GRID_COLUMNS;
      next = candidate >= 0 ? candidate : index;
    } else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      onSelect(entries[index].emoji);
      return;
    }
    if (next !== null && next !== index) {
      event.preventDefault();
      // Update tabIndex state and focus synchronously so the DOM reflects the
      // change before any Playwright assertion runs.
      setFocusedIndex(next);
      itemRefs.current[next]?.focus();
    }
  }

  return (
    <div
      className="emoji-picker-grid"
      role="grid"
      // The rendered column count and the keyboard step share this constant.
      style={
        { "--emoji-picker-columns": EMOJI_PICKER_GRID_COLUMNS } as CSSProperties
      }
    >
      {entries.map((entry, index) => (
        <button
          key={entry.emoji}
          ref={(node) => {
            itemRefs.current[index] = node;
          }}
          className="emoji-picker-item"
          type="button"
          title={entry.label}
          aria-label={entry.label}
          // Roving tabindex: only the active cell participates in the tab
          // sequence; all others are skipped by Tab.
          tabIndex={index === (focusedIndex ?? 0) ? 0 : -1}
          onFocus={() => setFocusedIndex(index)}
          onKeyDown={(e) => handleKeyDown(e, index)}
          onClick={() => onSelect(entry.emoji)}
        >
          {entry.emoji}
        </button>
      ))}
    </div>
  );
}
