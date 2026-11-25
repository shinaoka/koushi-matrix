// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

const readDownloadedText = vi.fn<(sourceRef: string) => Promise<string>>();
vi.mock("../../backend/linkMediaRuntime", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../backend/linkMediaRuntime")>()),
  readDownloadedText: (sourceRef: string) => readDownloadedText(sourceRef)
}));

import { TimelineMediaAttachment } from "./TimelineMedia";
import { fileTypeCategory } from "./fileTypeCategory";
import type { TimelineItem } from "../../domain/coreEvents";
import type { MediaTransferProgress } from "../../domain/coreEvents";
import type { TimelineMediaDownloadState } from "../../domain/types";
import { t } from "../../i18n/messages";

afterEach(() => {
  cleanup();
  readDownloadedText.mockReset();
});

const XLSX_MIME = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";

const fileMedia: NonNullable<TimelineItem["media"]> = {
  kind: "File",
  filename: "synthetic_report.xlsx",
  source: { mxc_uri: "mxc://example.invalid/synthetic-file", encrypted: true, encryption_version: null },
  mimetype: XLSX_MIME,
  size: 9 * 1024,
  width: null,
  height: null,
  thumbnail: null
};

const TEXT_MIME = "text/plain";

function textMedia(size: number, filename = "notes.txt"): NonNullable<TimelineItem["media"]> {
  return {
    kind: "File",
    filename,
    source: { mxc_uri: "mxc://example.invalid/synthetic-text", encrypted: false, encryption_version: null },
    mimetype: TEXT_MIME,
    size,
    width: null,
    height: null,
    thumbnail: null
  };
}

function audioMedia(filename = "clip.mp3"): NonNullable<TimelineItem["media"]> {
  return { ...textMedia(4096, filename), mimetype: "audio/mpeg" };
}

function videoMedia(filename = "clip.mp4"): NonNullable<TimelineItem["media"]> {
  return { ...textMedia(4096, filename), mimetype: "video/mp4" };
}

function readyDownload(
  mimeType: string
): Extract<TimelineMediaDownloadState, { kind: "ready" }> {
  return {
    kind: "ready",
    source_ref: "download/1234567890123456/6543210987654321",
    width: null,
    height: null,
    mime_type: mimeType
  };
}

const READY_TEXT_DOWNLOAD = readyDownload(TEXT_MIME);

const viewerEntry = {
  eventId: "$synthetic-media:example.invalid",
  filename: "synthetic_photo.png",
  kind: "Image" as const,
  size: 2048,
  width: 640,
  height: 480,
  mimetype: "image/png",
  encrypted: false,
  saveName: null
};

function renderCard({
  media = fileMedia,
  progress = null,
  downloadState,
  canDownload = true,
  onDownload = vi.fn()
}: {
  media?: NonNullable<TimelineItem["media"]>;
  progress?: MediaTransferProgress | null;
  downloadState?: TimelineMediaDownloadState;
  canDownload?: boolean;
  onDownload?: () => void;
} = {}) {
  render(
    <TimelineMediaAttachment
      media={media}
      progress={progress}
      downloadState={downloadState}
      canDownload={canDownload}
      onDownload={onDownload}
      onOpenMediaViewer={vi.fn()}
      saveName={null}
      viewerEntry={viewerEntry}
    />
  );
  const card = document.querySelector<HTMLElement>(".message-media");
  if (!card) throw new Error("file card not rendered");
  return card;
}

// #1784: the file card follows the WeChat file message: filename and size on
// the left, a type tile on the right, and no raw MIME type or per-file
// encryption pill.
describe("timeline file card", () => {
  it("shows filename, size, and a spreadsheet tile instead of the raw MIME type", () => {
    const card = renderCard();

    expect(card.querySelector(".message-media-title")?.textContent).toBe("synthetic_report.xlsx");
    expect(card.querySelector(".message-media-meta")?.textContent).toBe("9 KB");
    expect(card.textContent).not.toContain(XLSX_MIME);
    expect(card.textContent).not.toContain(t("timeline.encryptedMedia"));
    expect(card.querySelector(".message-media-badge")).toBeNull();
    expect(card.querySelector(".message-media-type-tile")?.getAttribute("data-file-type")).toBe(
      "spreadsheet"
    );
  });

  it("keeps the type tile as the download button", () => {
    const onDownload = vi.fn();
    renderCard({ onDownload });

    const button = screen.getByRole("button", {
      name: t("timeline.downloadMedia", { filename: "synthetic_report.xlsx" })
    });
    expect(button.classList.contains("message-media-download")).toBe(true);
    expect(button.classList.contains("message-media-type-tile")).toBe(true);
    fireEvent.click(button);
    expect(onDownload).toHaveBeenCalledTimes(1);
  });

  it("downloads when the card background is clicked", () => {
    const onDownload = vi.fn();
    const card = renderCard({ onDownload });

    fireEvent.click(card.querySelector(".message-media-title")!);

    expect(onDownload).toHaveBeenCalledTimes(1);
  });

  it("does not download from the card when downloading is unavailable", () => {
    const onDownload = vi.fn();
    const card = renderCard({ onDownload, canDownload: false });

    fireEvent.click(card);

    expect(onDownload).not.toHaveBeenCalled();
    expect(card.querySelector("button")).toBeNull();
    expect(card.querySelector(".message-media-type-tile")).not.toBeNull();
  });

  it("replaces the size with the download status while pending", () => {
    const card = renderCard({
      downloadState: { kind: "pending", progress: { current: 45, total: 100 } }
    });

    expect(card.querySelector(".message-media-meta")?.textContent).toBe(
      `${t("timeline.mediaDownloadPending")} · ${t("timeline.mediaUploadProgress", { percent: 45 })}`
    );
    expect(card.querySelector(".message-media-type-ring")).not.toBeNull();
  });

  it("shows a failed status and retries from the card", () => {
    const onDownload = vi.fn();
    const card = renderCard({
      onDownload,
      downloadState: { kind: "failed", failure_kind: "network" }
    });

    expect(card.querySelector(".message-media-meta")?.textContent).toBe(
      t("timeline.mediaDownloadFailed")
    );
    expect(card.querySelector(".message-media-error")).not.toBeNull();
    const retry = screen.getByRole("button", { name: t("timeline.mediaDownloadRetry") });
    expect(retry.classList.contains("message-media-type-tile")).toBe(true);
    fireEvent.click(card.querySelector(".message-media-title")!);
    expect(onDownload).toHaveBeenCalledTimes(1);
  });

  it("shows upload progress next to the size with a bottom progress bar", () => {
    const card = renderCard({ canDownload: false, progress: { current: 7, total: 10 } });

    expect(card.querySelector(".message-media-meta")?.textContent).toBe(
      `9 KB · ${t("timeline.mediaUploadProgress", { percent: 70 })}`
    );
    const bar = card.querySelector<HTMLElement>(".message-media-progress");
    expect(bar?.getAttribute("aria-valuenow")).toBe("70");
  });
});

// #2096: the card previews a text file's first lines and plays audio and video
// inline, both from Core's media cache; nothing here writes to Downloads.
describe("timeline file card previews (#2096)", () => {
  it("fetches a small text file itself and shows its first lines", async () => {
    readDownloadedText.mockResolvedValue("one\ntwo\nthree\n");
    const onDownload = vi.fn();
    const card = renderCard({ media: textMedia(2048), onDownload });

    expect(onDownload).toHaveBeenCalledTimes(1);
    expect(card.getAttribute("data-download-state")).toBe("notRequested");
    // Nothing is read before the bytes are in the cache.
    expect(readDownloadedText).not.toHaveBeenCalled();
    void card;
  });

  it("reads a downloaded text file once and previews its first lines", async () => {
    readDownloadedText.mockResolvedValue("alpha\nbeta\ngamma\n");
    const card = renderCard({ media: textMedia(2048), downloadState: READY_TEXT_DOWNLOAD });

    await waitFor(() => expect(card.querySelector(".message-media-text-preview")).not.toBeNull());
    expect(card.querySelector(".message-media-text-preview")?.textContent).toBe("alpha\nbeta\ngamma");
    expect(readDownloadedText).toHaveBeenCalledTimes(1);
    expect(readDownloadedText).toHaveBeenCalledWith(READY_TEXT_DOWNLOAD.source_ref);
  });

  it("opens the whole text in a dialog from the card and keeps the tile for saving", async () => {
    const fullText = Array.from({ length: 40 }, (_, index) => `line ${index}`).join("\n");
    readDownloadedText.mockResolvedValue(fullText);
    const card = renderCard({ media: textMedia(2048), downloadState: READY_TEXT_DOWNLOAD });

    await waitFor(() => expect(card.querySelector(".message-media-text-preview")).not.toBeNull());
    // The card shows a few lines; the dialog is where the whole file is.
    expect(card.querySelector(".message-media-text-preview")?.textContent).not.toContain("line 30");
    fireEvent.click(card.querySelector(".message-media-title")!);

    const dialog = await screen.findByRole("dialog");
    expect(dialog.querySelector(".media-text-preview-body")?.textContent).toBe(fullText);
    expect(readDownloadedText).toHaveBeenCalledTimes(1);
    // The type tile is still the download/save control.
    expect(
      screen.getByRole("button", { name: t("timeline.downloadMedia", { filename: "notes.txt" }) })
        .classList.contains("message-media-type-tile")
    ).toBe(true);
  });

  it("neither fetches nor previews a text file above the limit", () => {
    const onDownload = vi.fn();
    const card = renderCard({ media: textMedia(256 * 1024 + 1), onDownload });

    expect(onDownload).not.toHaveBeenCalled();
    expect(readDownloadedText).not.toHaveBeenCalled();
    expect(card.querySelector(".message-media-text-preview")).toBeNull();
    expect(
      screen.getByRole("button", { name: t("timeline.downloadMedia", { filename: "notes.txt" }) })
    ).not.toBeNull();
  });

  it("keeps the plain card when the backend refuses the text", async () => {
    readDownloadedText.mockRejectedValue(new Error("media file is not text"));
    const card = renderCard({ media: textMedia(2048), downloadState: READY_TEXT_DOWNLOAD });

    await waitFor(() =>
      expect(card.querySelector(".message-media-preview-note")?.textContent).toBe(
        t("timeline.mediaTextPreviewFailed")
      )
    );
    expect(card.querySelector(".message-media-text-preview")).toBeNull();
  });

  it("plays a downloaded audio file inline instead of saving it", () => {
    const onDownload = vi.fn();
    const card = renderCard({
      media: audioMedia(),
      downloadState: readyDownload("audio/mpeg"),
      onDownload
    });

    const audio = card.querySelector("audio");
    expect(audio).not.toBeNull();
    expect(audio?.getAttribute("controls")).not.toBeNull();
    expect(audio?.getAttribute("src")).toBe(
      "koushi-media://localhost/download/mp3/1234567890123456/6543210987654321"
    );
    expect(audio?.getAttribute("aria-label")).toBe(
      t("timeline.mediaAudioPlayer", { filename: "clip.mp3" })
    );
    // A player type never fetches itself, and the card background does not save.
    expect(onDownload).not.toHaveBeenCalled();
    fireEvent.click(card);
    expect(onDownload).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("plays a downloaded video file inline", () => {
    const card = renderCard({ media: videoMedia(), downloadState: readyDownload("video/mp4") });

    const video = card.querySelector("video");
    expect(video?.getAttribute("src")).toBe(
      "koushi-media://localhost/download/mp4/1234567890123456/6543210987654321"
    );
    expect(video?.getAttribute("aria-label")).toBe(
      t("timeline.mediaVideoPlayer", { filename: "clip.mp4" })
    );
  });

  it("leaves a player type without a protocol token as a plain card", () => {
    const card = renderCard({
      media: { ...audioMedia("clip.wma"), mimetype: "audio/x-ms-wma" },
      downloadState: readyDownload("audio/x-ms-wma")
    });

    expect(card.querySelector("audio")).toBeNull();
    expect(
      screen.getByRole("button", { name: t("timeline.downloadMedia", { filename: "clip.wma" }) })
    ).not.toBeNull();
  });
});

describe("fileTypeCategory", () => {
  it("prefers the filename extension over the MIME type", () => {
    expect(fileTypeCategory("slides.pptx", "application/octet-stream", "File")).toBe("slides");
    expect(fileTypeCategory("table.CSV", "text/plain", "File")).toBe("spreadsheet");
    expect(fileTypeCategory("paper.pdf", null, "File")).toBe("pdf");
  });

  it("falls back to the MIME type when there is no known extension", () => {
    expect(fileTypeCategory("report", XLSX_MIME, "File")).toBe("spreadsheet");
    expect(
      fileTypeCategory(
        "letter",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "File"
      )
    ).toBe("document");
    expect(fileTypeCategory("bundle", "application/zip", "File")).toBe("archive");
    expect(fileTypeCategory("clip", "video/mp4", "File")).toBe("video");
    expect(fileTypeCategory("notes", "text/plain", "File")).toBe("text");
  });

  it("falls back to the media kind, then to a generic file", () => {
    expect(fileTypeCategory("voice", null, "Audio")).toBe("audio");
    expect(fileTypeCategory("recording", "application/octet-stream", "Video")).toBe("video");
    expect(fileTypeCategory("blob", "application/octet-stream", "File")).toBe("other");
    expect(fileTypeCategory(".hidden", null, "File")).toBe("other");
  });
});
