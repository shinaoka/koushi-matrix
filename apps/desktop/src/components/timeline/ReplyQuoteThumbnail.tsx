import { Image as ImageIcon, Play } from "lucide-react";
import type { CSSProperties } from "react";

import { mediaSourceUrl } from "../../backend/linkMediaRuntime";
import type { ReplyQuote, ReplyQuoteMedia } from "../../domain/coreEvents";
import type { TimelineMediaDownloadState } from "../../domain/types";
import { t } from "../../i18n/messages";

export function replyQuoteHasThumbnail(quote: ReplyQuote): boolean {
  const kind = quote.media?.kind;
  return Boolean(
    quote.state === "ready" &&
      quote.media &&
      (kind === "image" || kind === "sticker" || kind === "video")
  );
}

export function replyQuoteMediaLabel(media: ReplyQuoteMedia | null | undefined): string | null {
  switch (media?.kind) {
    case "image":
      return t("timeline.replyQuoteKindImage");
    case "sticker":
      return t("timeline.replyQuoteKindSticker");
    case "video":
      return t("timeline.replyQuoteKindVideo");
    case "file":
      return t("timeline.replyQuoteKindFile");
    case "audio":
      return t("timeline.replyQuoteKindAudio");
    default:
      return null;
  }
}

/**
 * The quote's text line. #1874: a visual thumbnail (image, sticker, video)
 * already shows what the original is, so a captionless one has no text line
 * instead of a redundant kind label.
 */
export function replyQuoteBody(quote: ReplyQuote): string | null {
  if (quote.body_preview) {
    return quote.body_preview;
  }
  if (replyQuoteHasThumbnail(quote)) {
    return null;
  }
  const mediaLabel = replyQuoteMediaLabel(quote.media);
  if (mediaLabel) {
    return mediaLabel;
  }
  if (quote.state === "loading") {
    return t("timeline.replyQuoteLoading");
  }
  if (quote.state === "failed") {
    return t("timeline.replyQuoteFailed");
  }
  if (quote.state === "redacted") {
    return t("timeline.redactedMessage");
  }
  if (quote.state === "missing") {
    return t("timeline.replyQuoteMissing");
  }
  if (quote.state === "unsupported") {
    return t("timeline.replyQuoteUnsupported");
  }
  return t("timeline.replyQuoteUnavailable");
}

export function replyQuoteThumbnailLabel(quote: ReplyQuote): string {
  const sender = quote.sender_label?.trim() || t("timeline.replyQuoteUnknownSender");
  return t("timeline.replyQuoteThumbnailOpen", { sender });
}

// #1874: the timeline preview keeps the original's shape inside a
// 240 x 160 px box. An extreme panorama or strip gets a box clamped to these
// ratios, and the stylesheet scales the image down to fit inside it, centered.
const PREVIEW_MAX_INLINE_PX = 240;
const PREVIEW_MAX_BLOCK_PX = 160;
const PREVIEW_MIN_RATIO = 1 / 2;
const PREVIEW_MAX_RATIO = 3;
const PREVIEW_FALLBACK_RATIO = 4 / 3;

/**
 * Sizes the timeline preview from the media hints, so the box has its final
 * shape before the thumbnail arrives and the quote never changes height.
 */
export function replyQuotePreviewStyle(media: ReplyQuoteMedia): CSSProperties {
  const { width, height } = media;
  const hinted = width && height && width > 0 && height > 0 ? width / height : null;
  const ratio = Math.min(
    PREVIEW_MAX_RATIO,
    Math.max(PREVIEW_MIN_RATIO, hinted ?? PREVIEW_FALLBACK_RATIO)
  );
  const inlineSize = Math.min(PREVIEW_MAX_INLINE_PX, PREVIEW_MAX_BLOCK_PX * ratio);
  return {
    aspectRatio: `${ratio}`,
    inlineSize: `${Math.round(inlineSize)}px`
  };
}

export function ReplyQuoteThumbnail({
  quote,
  state,
  surface = "timeline",
  onOpen
}: {
  quote: ReplyQuote;
  state?: TimelineMediaDownloadState;
  surface?: "timeline" | "composer";
  onOpen?: (quote: ReplyQuote) => void;
}) {
  if (!replyQuoteHasThumbnail(quote)) {
    return null;
  }
  const media = quote.media!;
  const sourceUrl =
    state?.kind === "ready"
      ? mediaSourceUrl(state.source_ref, state.mime_type)
      : "";
  const kindClass = media.kind === "sticker" ? "is-sticker" : media.kind === "video" ? "is-video" : "is-image";
  const thumbnailState = state?.kind ?? "notRequested";
  const content = sourceUrl ? (
    <img
      src={sourceUrl}
      alt=""
      className="reply-quote-thumbnail-image"
      draggable={false}
      aria-hidden="true"
    />
  ) : (
    <ImageIcon
      size={surface === "composer" ? 16 : 28}
      aria-hidden="true"
      className="reply-quote-thumbnail-placeholder-icon"
    />
  );
  const body = (
    <>
      {content}
      {media.kind === "video" ? (
        <span className="reply-quote-thumbnail-play" aria-hidden="true">
          <Play size={12} fill="currentColor" />
        </span>
      ) : null}
    </>
  );
  const className = [
    "reply-quote-thumbnail",
    surface === "composer" ? "is-composer" : "is-timeline",
    kindClass
  ].join(" ");
  const style = surface === "timeline" ? replyQuotePreviewStyle(media) : undefined;
  if (!onOpen) {
    return (
      <span
        className={className}
        style={style}
        data-thumbnail-state={thumbnailState}
        aria-hidden="true"
      >
        {body}
      </span>
    );
  }
  return (
    <button
      type="button"
      className={className}
      style={style}
      data-thumbnail-state={thumbnailState}
      aria-label={replyQuoteThumbnailLabel(quote)}
      onClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
        onOpen(quote);
      }}
    >
      {body}
    </button>
  );
}
