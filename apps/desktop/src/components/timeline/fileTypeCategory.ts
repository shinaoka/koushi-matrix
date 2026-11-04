import type { TimelineMediaKind } from "../../domain/coreEvents";

// #1784: presentation-only grouping for the file card's type tile. It picks an
// icon and tint; it never decides a Matrix command or download behavior.
export type FileTypeCategory =
  | "spreadsheet"
  | "document"
  | "slides"
  | "pdf"
  | "archive"
  | "image"
  | "audio"
  | "video"
  | "text"
  | "other";

const EXTENSION_CATEGORIES: Record<string, FileTypeCategory> = {
  xls: "spreadsheet",
  xlsx: "spreadsheet",
  xlsm: "spreadsheet",
  csv: "spreadsheet",
  tsv: "spreadsheet",
  ods: "spreadsheet",
  numbers: "spreadsheet",
  doc: "document",
  docx: "document",
  odt: "document",
  rtf: "document",
  pages: "document",
  ppt: "slides",
  pptx: "slides",
  odp: "slides",
  key: "slides",
  pdf: "pdf",
  zip: "archive",
  rar: "archive",
  "7z": "archive",
  tar: "archive",
  gz: "archive",
  tgz: "archive",
  bz2: "archive",
  xz: "archive",
  zst: "archive",
  png: "image",
  jpg: "image",
  jpeg: "image",
  gif: "image",
  webp: "image",
  heic: "image",
  svg: "image",
  bmp: "image",
  mp3: "audio",
  m4a: "audio",
  aac: "audio",
  wav: "audio",
  flac: "audio",
  ogg: "audio",
  opus: "audio",
  mp4: "video",
  m4v: "video",
  mov: "video",
  mkv: "video",
  webm: "video",
  avi: "video",
  txt: "text",
  md: "text",
  log: "text",
  json: "text",
  yaml: "text",
  yml: "text",
  toml: "text",
  xml: "text",
  html: "text",
  css: "text",
  js: "text",
  ts: "text",
  tsx: "text",
  py: "text",
  rs: "text",
  go: "text",
  c: "text",
  h: "text",
  cpp: "text",
  java: "text",
  sh: "text",
  tex: "text",
  typ: "text"
};

function extensionCategory(filename: string): FileTypeCategory | null {
  const dot = filename.lastIndexOf(".");
  // A leading dot (".hidden") names the file, not its type.
  if (dot <= 0 || dot === filename.length - 1) {
    return null;
  }
  return EXTENSION_CATEGORIES[filename.slice(dot + 1).toLowerCase()] ?? null;
}

function mimeCategory(mimetype: string | null): FileTypeCategory | null {
  if (!mimetype) {
    return null;
  }
  const mime = mimetype.toLowerCase().split(";")[0]!.trim();
  if (mime === "application/pdf") return "pdf";
  if (mime === "text/csv" || mime.includes("spreadsheet") || mime.includes("ms-excel")) {
    return "spreadsheet";
  }
  if (mime.includes("presentation") || mime.includes("ms-powerpoint")) return "slides";
  if (
    mime.includes("wordprocessing") ||
    mime === "application/msword" ||
    mime === "application/rtf" ||
    mime === "application/vnd.oasis.opendocument.text"
  ) {
    return "document";
  }
  if (
    mime === "application/zip" ||
    mime === "application/gzip" ||
    mime === "application/x-tar" ||
    mime === "application/x-7z-compressed" ||
    mime === "application/vnd.rar" ||
    mime === "application/x-rar-compressed"
  ) {
    return "archive";
  }
  if (mime.startsWith("image/")) return "image";
  if (mime.startsWith("audio/")) return "audio";
  if (mime.startsWith("video/")) return "video";
  if (mime.startsWith("text/") || mime === "application/json") return "text";
  return null;
}

export function fileTypeCategory(
  filename: string,
  mimetype: string | null,
  kind: TimelineMediaKind
): FileTypeCategory {
  return (
    extensionCategory(filename) ??
    mimeCategory(mimetype) ??
    (kind === "Audio" ? "audio" : kind === "Video" ? "video" : "other")
  );
}
