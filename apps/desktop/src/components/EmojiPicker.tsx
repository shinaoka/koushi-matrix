import { Search, X } from "lucide-react";
import {
  type RefObject,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import { t } from "../i18n/messages";
import {
  EMOJI_BY_CATEGORY,
  EMOJI_CATEGORIES,
  type EmojiCategory,
  type EmojiEntry,
} from "./emojiData";

const RECENT_EMOJIS_KEY = "koushi-recent-emojis";
const MAX_RECENT = 24;

function readRecentEmojis(): string[] {
  try {
    const raw = localStorage.getItem(RECENT_EMOJIS_KEY);
    if (!raw) {
      return [];
    }
    const parsed = JSON.parse(raw);
    if (
      Array.isArray(parsed) &&
      parsed.every((item) => typeof item === "string")
    ) {
      return parsed.slice(0, MAX_RECENT);
    }
  } catch {
    // ignore corrupt storage
  }
  return [];
}

function writeRecentEmojis(emojis: string[]) {
  try {
    localStorage.setItem(
      RECENT_EMOJIS_KEY,
      JSON.stringify(emojis.slice(0, MAX_RECENT)),
    );
  } catch {
    // ignore storage errors
  }
}

function pushRecentEmoji(emoji: string) {
  const current = readRecentEmojis();
  const next = [emoji, ...current.filter((item) => item !== emoji)];
  writeRecentEmojis(next);
}

interface EmojiPickerProps {
  onSelect: (emoji: string) => void;
  onClose: () => void;
  /** Element that triggered the picker; excluded from outside-click detection
   * so the trigger button can handle its own toggle without the picker
   * re-opening after the outside-click handler fires. */
  anchorRef?: RefObject<Element | null>;
}

export function EmojiPicker({ onSelect, onClose, anchorRef }: EmojiPickerProps) {
  const [query, setQuery] = useState("");
  const [activeCategory, setActiveCategory] = useState<
    EmojiCategory | "recent"
  >("smileys");
  const [recentEmojis, setRecentEmojis] = useState<string[]>(() =>
    readRecentEmojis(),
  );
  const panelRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const categoryRefs = useRef<Record<EmojiCategory, HTMLDivElement | null>>({
    smileys: null,
    people: null,
    nature: null,
    food: null,
    activities: null,
    travel: null,
    objects: null,
    symbols: null,
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
        if (entry.label.toLowerCase().includes(trimmedQuery)) {
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

  useLayoutEffect(() => {
    searchRef.current?.focus();
  }, []);

  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.stopPropagation();
        onClose();
      }
    }
    function handleClickOutside(event: MouseEvent) {
      if (
        panelRef.current &&
        !panelRef.current.contains(event.target as Node) &&
        !(anchorRef?.current?.contains(event.target as Node) ?? false)
      ) {
        onClose();
      }
    }
    document.addEventListener("keydown", handleKeyDown);
    document.addEventListener("mousedown", handleClickOutside);
    return () => {
      document.removeEventListener("keydown", handleKeyDown);
      document.removeEventListener("mousedown", handleClickOutside);
    };
  }, [onClose, anchorRef]);

  const handleSelect = useCallback(
    (emoji: string) => {
      pushRecentEmoji(emoji);
      setRecentEmojis(readRecentEmojis());
      onSelect(emoji);
      onClose();
    },
    [onSelect, onClose],
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

  return (
    <div
      ref={panelRef}
      className="emoji-picker"
      role="dialog"
      aria-label={t("composer.emoji")}
    >
      <div className="emoji-picker-header">
        <div className="emoji-picker-search">
          <Search size={14} aria-hidden="true" />
          <input
            ref={searchRef}
            type="search"
            value={query}
            placeholder={t("composer.emojiSearch")}
            aria-label={t("composer.emojiSearch")}
            onChange={(event) => setQuery(event.currentTarget.value)}
          />
        </div>
        <button
          className="icon-button"
          type="button"
          aria-label={t("mediaGallery.close")}
          onClick={onClose}
        >
          <X size={14} />
        </button>
      </div>

      {!searching && (
        <div className="emoji-picker-tabs" role="tablist">
          {recentEntries.length > 0 && (
            <button
              className={`emoji-picker-tab ${activeCategory === "recent" ? "active" : ""}`}
              type="button"
              role="tab"
              aria-selected={activeCategory === "recent"}
              onClick={() => {
                setActiveCategory("recent");
                scrollToCategory("recent");
              }}
            >
              {t("composer.emojiRecent")}
            </button>
          )}
          {EMOJI_CATEGORIES.map((category) => (
            <button
              key={category}
              className={`emoji-picker-tab ${activeCategory === category ? "active" : ""}`}
              type="button"
              role="tab"
              aria-selected={activeCategory === category}
              onClick={() => {
                setActiveCategory(category);
                scrollToCategory(category);
              }}
            >
              {t(`emoji.category.${category}` as const)}
            </button>
          ))}
        </div>
      )}

      <div className="emoji-picker-body">
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
}

function EmojiGrid({
  entries,
  onSelect,
}: {
  entries: EmojiEntry[];
  onSelect: (emoji: string) => void;
}) {
  return (
    <div className="emoji-picker-grid" role="grid">
      {entries.map((entry) => (
        <button
          key={entry.emoji}
          className="emoji-picker-item"
          type="button"
          title={entry.label}
          aria-label={entry.label}
          onClick={() => onSelect(entry.emoji)}
        >
          {entry.emoji}
        </button>
      ))}
    </div>
  );
}
