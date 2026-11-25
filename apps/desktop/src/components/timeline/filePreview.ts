import type { FileTypeCategory } from "./fileTypeCategory";

/**
 * #2096: what a file attachment can show inline, decided from the category the
 * card already computes. It picks a preview only; it never decides a Matrix
 * command, and every path here reads Core's media cache rather than the user's
 * Downloads folder.
 */
export type FilePreviewKind = "text" | "audio" | "video" | "none";

/**
 * Largest file the card fetches to build a preview on its own, matching the
 * backend cap (`apps/desktop/src-tauri/src/media_download.rs`). Above it the
 * attachment keeps the plain download card and nothing is fetched until the
 * reader asks for the file.
 */
export const TEXT_PREVIEW_MAX_BYTES = 256 * 1024;

/** Lines the card shows before the reader opens the full text. */
export const TEXT_PREVIEW_LINE_COUNT = 6;

export function filePreviewKind(category: FileTypeCategory): FilePreviewKind {
  switch (category) {
    case "text":
      return "text";
    case "audio":
      return "audio";
    case "video":
      return "video";
    default:
      return "none";
  }
}

/**
 * Whether the card may fetch this file on its own to show a preview. An unknown
 * size stays unfetched: the DTO always carries one for a real attachment, and
 * guessing would fetch a file the reader never asked for.
 */
export function canAutoFetchPreview(
  kind: FilePreviewKind,
  size: number | null,
  maxBytes = TEXT_PREVIEW_MAX_BYTES
): boolean {
  if (kind !== "text") {
    return false;
  }
  return size !== null && Number.isFinite(size) && size >= 0 && size <= maxBytes;
}

/**
 * The first lines of a text attachment, as the card shows them. A line is kept
 * as it is (no wrapping decision here) and a trailing newline never becomes an
 * empty last line.
 */
export function textPreviewLines(
  text: string,
  maxLines = TEXT_PREVIEW_LINE_COUNT
): string[] {
  if (maxLines <= 0) {
    return [];
  }
  const normalized = text.replace(/\r\n?/g, "\n");
  const lines = normalized.split("\n");
  if (lines.length > 1 && lines[lines.length - 1] === "") {
    lines.pop();
  }
  return lines.slice(0, maxLines);
}
