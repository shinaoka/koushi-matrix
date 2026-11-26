import { describe, expect, it } from "vitest";

import {
  TEXT_PREVIEW_LINE_COUNT,
  TEXT_PREVIEW_MAX_BYTES,
  canAutoFetchPreview,
  isMarkdownAttachment,
  filePreviewKind,
  textPreviewLines
} from "./filePreview";

describe("file preview kind (#2096)", () => {
  it("previews exactly the categories a card can render", () => {
    expect(filePreviewKind("text")).toBe("text");
    expect(filePreviewKind("audio")).toBe("audio");
    expect(filePreviewKind("video")).toBe("video");
    for (const category of ["pdf", "document", "spreadsheet", "slides", "archive", "image", "other"] as const) {
      expect(filePreviewKind(category)).toBe("none");
    }
  });

  it("only fetches a small text file on its own", () => {
    expect(canAutoFetchPreview("text", 0)).toBe(true);
    expect(canAutoFetchPreview("text", TEXT_PREVIEW_MAX_BYTES)).toBe(true);
    expect(canAutoFetchPreview("text", TEXT_PREVIEW_MAX_BYTES + 1)).toBe(false);
    // Unknown size, an unknown category, and a player type stay unfetched: the
    // reader asks for those files instead of the timeline fetching them.
    expect(canAutoFetchPreview("text", null)).toBe(false);
    expect(canAutoFetchPreview("none", 1024)).toBe(false);
    expect(canAutoFetchPreview("audio", 1024)).toBe(false);
    expect(canAutoFetchPreview("video", 1024)).toBe(false);
  });

  it("keeps the cap the backend enforces", () => {
    // Mirrors `MAX_TEXT_PREVIEW_BYTES` in apps/desktop/src-tauri/src/media_download.rs.
    expect(TEXT_PREVIEW_MAX_BYTES).toBe(256 * 1024);
    expect(TEXT_PREVIEW_LINE_COUNT).toBe(6);
  });
});

describe("markdown attachments (#2097)", () => {
  it("recognises .md and .markdown, and the text/markdown type", () => {
    expect(isMarkdownAttachment("plan.md", null)).toBe(true);
    expect(isMarkdownAttachment("PLAN.MARKDOWN", "application/octet-stream")).toBe(true);
    expect(isMarkdownAttachment("plan", "text/markdown; charset=utf-8")).toBe(true);
  });

  it("leaves other text types to the plain-text preview", () => {
    expect(isMarkdownAttachment("notes.txt", "text/plain")).toBe(false);
    expect(isMarkdownAttachment("spec.rst", "text/x-rst")).toBe(false);
    expect(isMarkdownAttachment("readme.md.bak", null)).toBe(false);
  });
});

describe("text preview lines (#2096)", () => {
  it("shows the first lines and drops the last empty one", () => {
    expect(textPreviewLines("one\ntwo\nthree\n")).toEqual(["one", "two", "three"]);
    expect(textPreviewLines("one\ntwo")).toEqual(["one", "two"]);
  });

  it("stops at the line count instead of the whole file", () => {
    const source = Array.from({ length: 40 }, (_, index) => `line ${index}`).join("\n");
    const lines = textPreviewLines(source);
    expect(lines).toHaveLength(TEXT_PREVIEW_LINE_COUNT);
    expect(lines[0]).toBe("line 0");
    expect(lines[TEXT_PREVIEW_LINE_COUNT - 1]).toBe(`line ${TEXT_PREVIEW_LINE_COUNT - 1}`);
  });

  it("reads CRLF and CR files without leaving a stray carriage return", () => {
    expect(textPreviewLines("one\r\ntwo\r\n")).toEqual(["one", "two"]);
    expect(textPreviewLines("one\rtwo\r")).toEqual(["one", "two"]);
  });

  it("keeps an empty line the file really has, and indentation", () => {
    expect(textPreviewLines("one\n\ntwo\n")).toEqual(["one", "", "two"]);
    expect(textPreviewLines("  indented\n\t")).toEqual(["  indented", "\t"]);
    expect(textPreviewLines("")).toEqual([""]);
  });
});
