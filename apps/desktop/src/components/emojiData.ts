import {
  DATA_BY_CATEGORY,
  type Emoji as EmojibaseEmoji
} from "@matrix-org/emojibase-bindings";

export interface EmojiEntry {
  emoji: string;
  label: string;
  search: string;
}

export const EMOJI_CATEGORIES = [
  "people",
  "nature",
  "foods",
  "activity",
  "places",
  "objects",
  "symbols",
  "flags"
] as const;

export type EmojiCategory = (typeof EMOJI_CATEGORIES)[number];

function normalizeEmojiEntry(entry: EmojibaseEmoji): EmojiEntry {
  const label = entry.label;
  const search = [
    label,
    ...entry.shortcodes,
    ...(entry.tags ?? [])
  ].join(" ").toLowerCase();
  return {
    emoji: entry.unicode,
    label,
    search
  };
}

export const EMOJI_BY_CATEGORY: Record<EmojiCategory, EmojiEntry[]> =
  Object.fromEntries(
    EMOJI_CATEGORIES.map((category) => [
      category,
      (DATA_BY_CATEGORY[category] ?? []).map(normalizeEmojiEntry)
    ])
  ) as Record<EmojiCategory, EmojiEntry[]>;
