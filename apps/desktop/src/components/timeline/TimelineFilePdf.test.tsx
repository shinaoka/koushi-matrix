// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { TimelineItem } from "../../domain/coreEvents";
import type { TimelineMediaDownloadState } from "../../domain/types";
import { t } from "../../i18n/messages";
import { TimelineMediaAttachment } from "./TimelineMedia";
import {
  PDF_AUTO_THUMBNAIL_MAX_BYTES,
  PDF_PREVIEW_MAX_BYTES,
  PDF_PREVIEW_MAX_PAGES
} from "./filePreview";

const readDownloadedPdf = vi.fn<(sourceRef: string) => Promise<Uint8Array>>();
vi.mock("../../backend/linkMediaRuntime", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../backend/linkMediaRuntime")>()),
  readDownloadedPdf: (sourceRef: string) => readDownloadedPdf(sourceRef)
}));

// pdf.js needs a real canvas; the card and viewer logic is what these tests pin.
// The drawing itself is exercised in the browser by e2e/media-attachments.spec.ts.
const fakePdf = { numPages: 3, degraded: false };
const renderedPages: number[] = [];
const destroyed = vi.fn();
vi.mock("./pdfRenderer", () => ({
  openPdf: vi.fn(async () => ({
    numPages: fakePdf.numPages,
    renderPage: (page: number) => {
      renderedPages.push(page);
      return { done: Promise.resolve({ degraded: fakePdf.degraded }), cancel: () => undefined };
    },
    destroy: async () => {
      destroyed();
    }
  })),
  renderPdfThumbnail: vi.fn(async () => ({
    dataUrl: "data:image/png;base64,AAAA",
    degraded: fakePdf.degraded
  }))
}));

const pdfBytes = new TextEncoder().encode("%PDF-1.4\n");

beforeEach(() => {
  fakePdf.numPages = 3;
  fakePdf.degraded = false;
  renderedPages.length = 0;
  readDownloadedPdf.mockResolvedValue(pdfBytes);
});

afterEach(() => {
  cleanup();
  readDownloadedPdf.mockReset();
  destroyed.mockReset();
});

function pdfMedia(size: number): NonNullable<TimelineItem["media"]> {
  return {
    kind: "File",
    filename: "report.pdf",
    source: { mxc_uri: "mxc://example.invalid/pdf", encrypted: false, encryption_version: null },
    mimetype: "application/pdf",
    size,
    width: null,
    height: null,
    thumbnail: null
  };
}

const READY: Extract<TimelineMediaDownloadState, { kind: "ready" }> = {
  kind: "ready",
  source_ref: "download/1234567890123456/6543210987654321",
  width: null,
  height: null,
  mime_type: "application/pdf"
};

function renderCard(size: number, downloadState?: TimelineMediaDownloadState) {
  const onSaveMediaFile = vi.fn(async () => undefined);
  const onDownload = vi.fn();
  render(
    <TimelineMediaAttachment
      media={pdfMedia(size)}
      progress={null}
      downloadState={downloadState}
      canDownload
      onDownload={onDownload}
      onOpenMediaViewer={vi.fn()}
      onSaveMediaFile={onSaveMediaFile}
      saveName={null}
      viewerEntry={{
        eventId: "$pdf:example.invalid",
        filename: "report.pdf",
        kind: "File" as never,
        size,
        width: null,
        height: null,
        mimetype: "application/pdf",
        encrypted: false,
        saveName: null
      } as never}
    />
  );
  return { onSaveMediaFile, onDownload };
}

describe("PDF attachment card (#2096)", () => {
  it("pins the limits the backend enforces", () => {
    // Mirrors `MAX_PDF_PREVIEW_BYTES` in apps/desktop/src-tauri/src/media_download.rs.
    expect(PDF_PREVIEW_MAX_BYTES).toBe(32 * 1024 * 1024);
    expect(PDF_AUTO_THUMBNAIL_MAX_BYTES).toBe(8 * 1024 * 1024);
    expect(PDF_PREVIEW_MAX_PAGES).toBe(200);
  });

  it("shows page 1 as a thumbnail for a downloaded PDF inside the cap", async () => {
    renderCard(PDF_AUTO_THUMBNAIL_MAX_BYTES, READY);
    const thumbnail = await screen.findByAltText(
      t("timeline.pdfViewerThumbnail", { filename: "report.pdf" })
    );
    expect(thumbnail.getAttribute("src")).toBe("data:image/png;base64,AAAA");
    expect(readDownloadedPdf).toHaveBeenCalledTimes(1);
    expect(readDownloadedPdf).toHaveBeenCalledWith(READY.source_ref);
    // The thumbnail's document is released as soon as the image exists.
    expect(destroyed).toHaveBeenCalledTimes(1);
  });

  it("opens the viewer on click, pages, and never saves", async () => {
    const { onSaveMediaFile, onDownload } = renderCard(2048, READY);
    const open = await screen.findByRole("button", {
      name: t("timeline.mediaPdfPreviewOpen", { filename: "report.pdf" })
    });
    fireEvent.click(open);

    const dialog = await screen.findByRole("dialog", { name: "report.pdf" });
    await screen.findByText(t("timeline.pdfViewerPage", { page: 1, count: 3 }));
    const previous = screen.getByRole("button", { name: t("timeline.pdfViewerPrevious") });
    const next = screen.getByRole("button", { name: t("timeline.pdfViewerNext") });
    expect((previous as HTMLButtonElement).disabled).toBe(true);
    await waitFor(() => expect(renderedPages).toContain(1));

    fireEvent.click(next);
    await screen.findByText(t("timeline.pdfViewerPage", { page: 2, count: 3 }));
    await waitFor(() => expect(renderedPages).toContain(2));
    fireEvent.click(next);
    await screen.findByText(t("timeline.pdfViewerPage", { page: 3, count: 3 }));
    expect((next as HTMLButtonElement).disabled).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: t("timeline.pdfViewerZoomIn") }));
    expect(dialog.textContent).toContain("125%");

    // No save/download call from the card, the thumbnail or the viewer.
    expect(onSaveMediaFile).not.toHaveBeenCalled();
    expect(onDownload).not.toHaveBeenCalled();
  });

  it("neither reads nor previews a PDF above the cap, and says so", () => {
    const { onSaveMediaFile } = renderCard(PDF_PREVIEW_MAX_BYTES + 1, READY);
    expect(readDownloadedPdf).not.toHaveBeenCalled();
    expect(screen.queryByRole("img")).toBeNull();
    expect(screen.getByText(t("timeline.mediaPdfTooLarge"))).toBeTruthy();
    expect(onSaveMediaFile).not.toHaveBeenCalled();
  });

  it("keeps the icon and says so when the file has more pages than the limit", async () => {
    fakePdf.numPages = PDF_PREVIEW_MAX_PAGES + 1;
    renderCard(2048, READY);
    await screen.findByText(t("timeline.mediaPdfTooLarge"));
    expect(screen.queryByRole("img")).toBeNull();
  });

  it("keeps the plain card when the backend refuses the file", async () => {
    readDownloadedPdf.mockRejectedValue(new Error("media file is not a PDF"));
    renderCard(2048, READY);
    await screen.findByText(t("timeline.mediaPdfPreviewFailed"));
    expect(screen.queryByRole("img")).toBeNull();
  });

  it("does not read anything before the file is downloaded", () => {
    renderCard(2048);
    expect(readDownloadedPdf).not.toHaveBeenCalled();
  });

  it("leaves a PDF above the thumbnail cap plain, then opens the viewer once fetched", async () => {
    const size = PDF_AUTO_THUMBNAIL_MAX_BYTES + 1;
    // Not downloaded: a click on the card fetches (no save intent) and reads nothing.
    const first = renderCard(size);
    fireEvent.click(document.querySelector(".message-media-file") as HTMLElement);
    expect(first.onDownload).toHaveBeenCalledTimes(1);
    expect(readDownloadedPdf).not.toHaveBeenCalled();
    cleanup();
    // Downloaded: no automatic thumbnail, and the card opens the viewer.
    const second = renderCard(size, READY);
    expect(screen.queryByRole("img")).toBeNull();
    expect(readDownloadedPdf).not.toHaveBeenCalled();
    fireEvent.click(document.querySelector(".message-media-file") as HTMLElement);
    await screen.findByRole("dialog", { name: "report.pdf" });
    await screen.findByText(t("timeline.pdfViewerPage", { page: 1, count: 3 }));
    expect(second.onSaveMediaFile).not.toHaveBeenCalled();
  });

  it("says some content could not be displayed when the page drew partially", async () => {
    fakePdf.degraded = true;
    renderCard(2048, READY);
    await screen.findByText(t("timeline.pdfContentPartial"));
  });
});
