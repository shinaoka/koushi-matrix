// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { TimelineMediaAttachment } from "./TimelineMedia";
import { fileTypeCategory } from "./fileTypeCategory";
import type { TimelineItem } from "../../domain/coreEvents";
import type { MediaTransferProgress } from "../../domain/coreEvents";
import type { TimelineMediaDownloadState } from "../../domain/types";
import { t } from "../../i18n/messages";

afterEach(cleanup);

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
