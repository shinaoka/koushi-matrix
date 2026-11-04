import type { ReplyQuote, TimelineItem, TimelineKey } from "./coreEvents";
import { timelineKeyIdentity } from "./coreEvents";
import type { TimelineStoreState } from "./timelineStore";

function quoteMedia(item: TimelineItem): ReplyQuote["media"] {
  const media = item.media;
  if (!media) {
    return null;
  }
  return {
    kind:
      media.kind === "Image"
        ? "image"
        : media.kind === "Video"
          ? "video"
          : media.kind === "File"
            ? "file"
            : "audio",
    animated: /image\/(?:gif|webp)/.test(media.mimetype ?? ""),
    width: media.width ?? null,
    height: media.height ?? null,
    has_thumbnail_source: Boolean(media.thumbnail)
  };
}

function quoteBody(item: TimelineItem, kind: ReplyQuote["media"]): string | null {
  if (item.is_redacted || item.notice_i18n) {
    return null;
  }
  if (kind?.kind === "file" || kind?.kind === "audio") {
    return item.body ?? item.media?.filename ?? null;
  }
  return item.body ?? null;
}

function eventIdOf(item: TimelineItem): string | null {
  return "Event" in item.id ? item.id.Event.event_id : null;
}

/** The composer banner reads the same Rust-projected item fields as the row. */
export function composerReplyQuote(
  store: TimelineStoreState | null | undefined,
  key: TimelineKey | null,
  eventId: string | null
): ReplyQuote | null {
  if (!store || !key || !eventId) {
    return null;
  }
  const state = store.keys.get(timelineKeyIdentity(key));
  const item = state?.items.find((candidate) => eventIdOf(candidate) === eventId);
  if (!item) {
    return null;
  }
  const media = quoteMedia(item);
  const quoteState = item.is_redacted
    ? "redacted"
    : item.unable_to_decrypt
      ? "unsupported"
      : "ready";
  return {
    event_id: eventId,
    sender: item.sender,
    sender_label: item.sender_label,
    body_preview: quoteState === "ready" ? quoteBody(item, media) : null,
    formatted: quoteState === "ready" ? item.formatted : undefined,
    mentioned_user_ids: item.mentioned_user_ids,
    media: quoteState === "ready" ? media : null,
    permalink: item.actions?.permalink ?? null,
    can_open: false,
    state: quoteState
  };
}
