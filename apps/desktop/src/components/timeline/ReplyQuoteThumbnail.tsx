import { Image as ImageIcon, Play } from "lucide-react";

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

export function replyQuoteBody(quote: ReplyQuote): string {
  if (quote.body_preview) {
    return quote.body_preview;
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
      size={surface === "composer" ? 16 : 20}
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
  if (!onOpen) {
    return (
      <span
        className={className}
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
