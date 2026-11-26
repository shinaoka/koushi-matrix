import type { TimelineItem } from "../domain/coreEvents";

/**
 * Facts the save-name policy needs beyond the attachment's own filename (#1135).
 * The renderer resolves them where the platform facts live: the media kind from
 * the item, and the event's local offset from `Date`.
 */
export type MediaSaveNameFacts = {
  kind: "image" | "file";
  timestampMs: number | null;
};

const DOWNLOAD_SOURCE_REF =
  /^download\/(\d{1,16})\/(\d{1,16})(?:\/(\d{1,16}x\d{1,16}))?$/;

/** The protocol maps a fixed token to a response Content-Type, never a path. */
const MEDIA_DOWNLOAD_MIME_TOKENS: Readonly<Record<string, string>> = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/jpg": "jpg",
  "image/gif": "gif",
  "image/webp": "webp",
  "image/avif": "avif",
  "video/mp4": "mp4",
  "video/webm": "webm",
  "video/x-m4v": "m4v",
  // #2096: the inline player's audio types. The list mirrors the protocol's
  // `MIME_TOKENS`; a type without a token keeps the plain download card.
  "audio/mpeg": "mp3",
  "audio/mp4": "m4a",
  "audio/x-m4a": "m4a",
  "audio/aac": "aac",
  "audio/wav": "wav",
  "audio/x-wav": "wav",
  "audio/flac": "flac",
  "audio/ogg": "ogg",
  "audio/opus": "opus"
};

/**
 * Build a `koushi-media` URL for a downloaded image (#1619).
 *
 * Core names the cache entry with an opaque `download/<room-hash>/<event-hash>`
 * ref; only this adapter turns it into a URL the webview can load. The cache
 * path itself never leaves Rust.
 */
export function renderableMediaDownloadSourceUrl(
  sourceRef: string,
  mimeType: string | null | undefined,
  protocolBase: string
): string | null {
  const match = DOWNLOAD_SOURCE_REF.exec(sourceRef);
  if (!match) {
    return null;
  }
  const mime = mimeType?.split(";")[0]?.trim().toLowerCase() ?? "";
  const token = MEDIA_DOWNLOAD_MIME_TOKENS[mime];
  if (!token) {
    return null;
  }
  const size = match[3] ? `/${match[3]}` : "";
  return `${protocolBase}download/${token}/${match[1]}/${match[2]}${size}`;
}

export interface LinkMediaPort {
  openHttpUrl(url: string): Promise<void>;
  mediaSourceUrl(sourceRef: string, mimeType?: string | null): string;
  renderableThumbnailSourceUrl(sourceRef: string): string | null;
  renderableMediaDownloadSourceUrl(
    sourceRef: string,
    mimeType: string | null | undefined
  ): string | null;
  saveMediaFile(
    sourceRef: string,
    filename: string,
    saveName: MediaSaveNameFacts | null,
    accountTabId?: string
  ): Promise<void>;
  /**
   * #2096: the text of an already downloaded attachment, for an inline preview.
   * It reads Core's media cache and never touches the user's Downloads folder;
   * the backend refuses anything above its own preview limit.
   */
  readDownloadedText(sourceRef: string, accountTabId?: string): Promise<string>;
  /**
   * #2097: a downloaded Markdown attachment rendered by Core to the sanitized
   * formatted body a message carries. React never parses Markdown; the backend
   * enforces the same preview limit as `readDownloadedText`.
   */
  readDownloadedMarkdown(
    sourceRef: string,
    accountTabId?: string
  ): Promise<TimelineFormattedBodyDto>;
}

/** The formatted body Core renders for a Markdown document (#2097). */
export type TimelineFormattedBodyDto = NonNullable<TimelineItem["formatted"]>;
