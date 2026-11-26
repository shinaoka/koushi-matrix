import type { FileTypeCategory } from "./fileTypeCategory";

/**
 * #2096: what a file attachment can show inline, decided from the category the
 * card already computes. It picks a preview only; it never decides a Matrix
 * command, and every path here reads Core's media cache rather than the user's
 * Downloads folder.
 */
export type FilePreviewKind = "text" | "pdf" | "audio" | "video" | "none";

/**
 * Largest file the card fetches to build a preview on its own, matching the
 * backend cap (`apps/desktop/src-tauri/src/media_download.rs`). Above it the
 * attachment keeps the plain download card and nothing is fetched until the
 * reader asks for the file.
 */
export const TEXT_PREVIEW_MAX_BYTES = 256 * 1024;

/**
 * Largest PDF the card reads for its thumbnail and the viewer opens, matching
 * the backend cap (`MAX_PDF_PREVIEW_BYTES`). Above it the card keeps its icon:
 * the backend refuses such a file rather than truncating it, because a
 * truncated PDF can draw as a shorter, wrong document.
 */
export const PDF_PREVIEW_MAX_BYTES = 32 * 1024 * 1024;

/**
 * Largest PDF the card fetches, parses and rasterises on its own for a
 * thumbnail. A room full of larger PDFs must not download and decode them all
 * unprompted: above this the card stays plain and fetches on click, and the
 * viewer (up to `PDF_PREVIEW_MAX_BYTES`) opens once the reader asked for it.
 */
export const PDF_AUTO_THUMBNAIL_MAX_BYTES = 8 * 1024 * 1024;

/** Most pages a PDF may have for the card to draw a thumbnail and open the viewer. */
export const PDF_PREVIEW_MAX_PAGES = 200;

/** Lines the card shows before the reader opens the full text. */
export const TEXT_PREVIEW_LINE_COUNT = 6;

export function filePreviewKind(category: FileTypeCategory): FilePreviewKind {
  switch (category) {
    case "text":
      return "text";
    case "pdf":
      return "pdf";
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
  maxBytes = kind === "pdf" ? PDF_AUTO_THUMBNAIL_MAX_BYTES : TEXT_PREVIEW_MAX_BYTES
): boolean {
  if (kind !== "text" && kind !== "pdf") {
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

/**
 * #2097: a shared Markdown document, by extension or MIME type. Only `.md` /
 * `.markdown` is rendered as a document; `.txt`, `.rst` and other text types
 * keep the plain-text preview.
 */
export function isMarkdownAttachment(filename: string, mimetype: string | null): boolean {
  if (/\.(md|markdown)$/i.test(filename.trim())) {
    return true;
  }
  return (mimetype ?? "").split(";")[0].trim().toLowerCase() === "text/markdown";
}
