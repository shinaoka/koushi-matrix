import { Minus, Plus } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { t } from "../../i18n/messages";
import { ModalDialog } from "../ModalDialog";
import type { PdfDocument } from "./pdfRenderer";
import { PDF_PREVIEW_MAX_PAGES } from "./filePreview";

const ZOOM_STEPS = [0.5, 0.75, 1, 1.25, 1.5, 2, 3] as const;
const DEFAULT_ZOOM_INDEX = 2;

/**
 * #2096 stage 2: the whole PDF, with page navigation, the page count and zoom.
 *
 * The bytes come from Core's media cache through `readPdf` and are handed to
 * pdf.js as data; this dialog never fetches a URL and never writes to the user's
 * Downloads folder (the card's tile still does that). One page is drawn at a
 * time, so a 200-page file holds one canvas.
 */
export function MediaPdfViewerDialog({
  filename,
  readPdf,
  onClose
}: {
  filename: string;
  readPdf: () => Promise<Uint8Array>;
  onClose: () => void;
}) {
  const [state, setState] = useState<
    | { kind: "loading" }
    | { kind: "ready"; document: PdfDocument }
    | { kind: "failed" }
  >({ kind: "loading" });
  const [page, setPage] = useState(1);
  const [renderNote, setRenderNote] = useState<"none" | "partial" | "failed">("none");
  const [zoomIndex, setZoomIndex] = useState(DEFAULT_ZOOM_INDEX);
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    let cancelled = false;
    let opened: PdfDocument | null = null;
    void (async () => {
      try {
        const [bytes, { openPdf }] = await Promise.all([readPdf(), import("./pdfRenderer")]);
        const document = await openPdf(bytes);
        if (cancelled || document.numPages > PDF_PREVIEW_MAX_PAGES || document.numPages < 1) {
          void document.destroy();
          if (!cancelled) {
            setState({ kind: "failed" });
          }
          return;
        }
        opened = document;
        setState({ kind: "ready", document });
      } catch {
        if (!cancelled) {
          setState({ kind: "failed" });
        }
      }
    })();
    return () => {
      cancelled = true;
      void opened?.destroy();
    };
  }, [readPdf]);

  const document = state.kind === "ready" ? state.document : null;
  const zoom = ZOOM_STEPS[zoomIndex];
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!document || !canvas) {
      return;
    }
    let stale = false;
    setRenderNote("none");
    const render = document.renderPage(page, canvas, { scale: zoom });
    render.done.then(
      ({ degraded }) => {
        if (!stale && degraded) {
          setRenderNote("partial");
        }
      },
      () => {
        // A superseded render rejects too; only a current one is a failure the
        // reader must see instead of a stale page.
        if (!stale) {
          setRenderNote("failed");
        }
      }
    );
    return () => {
      stale = true;
      render.cancel();
    };
  }, [document, page, zoom]);

  const pageCount = document?.numPages ?? 0;
  const percent = Math.round(zoom * 100);
  return (
    <ModalDialog className="media-pdf-viewer-dialog" title={filename} onClose={onClose}>
      <div className="media-pdf-viewer-toolbar" role="toolbar" aria-label={filename}>
        <button
          type="button"
          className="media-pdf-viewer-button"
          aria-label={t("timeline.pdfViewerPrevious")}
          disabled={!document || page <= 1}
          onClick={() => setPage((current) => Math.max(1, current - 1))}
        >
          ‹
        </button>
        <span className="media-pdf-viewer-page" aria-live="polite">
          {document ? t("timeline.pdfViewerPage", { page, count: pageCount }) : ""}
        </span>
        <button
          type="button"
          className="media-pdf-viewer-button"
          aria-label={t("timeline.pdfViewerNext")}
          disabled={!document || page >= pageCount}
          onClick={() => setPage((current) => Math.min(pageCount, current + 1))}
        >
          ›
        </button>
        <span className="media-pdf-viewer-spacer" />
        <button
          type="button"
          className="media-pdf-viewer-button"
          aria-label={t("timeline.pdfViewerZoomOut")}
          disabled={!document || zoomIndex <= 0}
          onClick={() => setZoomIndex((current) => Math.max(0, current - 1))}
        >
          <Minus size={14} aria-hidden="true" />
        </button>
        <span className="media-pdf-viewer-zoom" aria-label={t("timeline.pdfViewerZoomLevel", { percent })}>
          {percent}%
        </span>
        <button
          type="button"
          className="media-pdf-viewer-button"
          aria-label={t("timeline.pdfViewerZoomIn")}
          disabled={!document || zoomIndex >= ZOOM_STEPS.length - 1}
          onClick={() => setZoomIndex((current) => Math.min(ZOOM_STEPS.length - 1, current + 1))}
        >
          <Plus size={14} aria-hidden="true" />
        </button>
      </div>
      <div className="media-pdf-viewer-body" tabIndex={0}>
        {state.kind === "loading" ? (
          <p className="media-pdf-viewer-note">{t("timeline.pdfViewerLoading")}</p>
        ) : null}
        {state.kind === "failed" ? (
          <p className="media-pdf-viewer-note">{t("timeline.mediaPdfPreviewFailed")}</p>
        ) : null}
        {renderNote === "partial" ? (
          <p className="media-pdf-viewer-note">{t("timeline.pdfContentPartial")}</p>
        ) : null}
        {renderNote === "failed" ? (
          <p className="media-pdf-viewer-note">{t("timeline.mediaPdfPreviewFailed")}</p>
        ) : null}
        {document ? <canvas ref={canvasRef} className="media-pdf-viewer-canvas" /> : null}
      </div>
    </ModalDialog>
  );
}
