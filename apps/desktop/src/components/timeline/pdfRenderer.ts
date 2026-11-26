import * as pdfjs from "pdfjs-dist";
import type { PDFDocumentProxy, RenderTask } from "pdfjs-dist";
import pdfWorkerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";

/**
 * #2096 stage 2: the only module that imports pdf.js. Callers load it with a
 * dynamic `import()` so the library stays out of the timeline's first chunk.
 *
 * The document is parsed from bytes the Rust command returned: nothing here
 * fetches a URL, so the page's `connect-src` stays closed and no third party
 * sees the file. The worker is bundled by Vite and runs under `worker-src
 * 'self' blob:`.
 */
pdfjs.GlobalWorkerOptions.workerSrc = pdfWorkerUrl;

export interface PdfDocument {
  readonly numPages: number;
  /** Draw one page, 1-based, onto `canvas` at `scale` (CSS pixels per PDF point). */
  renderPage(pageNumber: number, canvas: HTMLCanvasElement, scale: number): PdfRender;
  destroy(): Promise<void>;
}

export interface PdfRender {
  /** Resolves when the page is drawn; rejects with `cancelled: true` when superseded. */
  done: Promise<void>;
  cancel(): void;
}

export async function openPdf(bytes: Uint8Array): Promise<PdfDocument> {
  const task = pdfjs.getDocument({
    data: bytes,
    // The document is untrusted: no scripts, no XFA,
    // and no network fetches for resources the file does not embed.
    enableXfa: false,
    useWorkerFetch: false
  });
  const document: PDFDocumentProxy = await task.promise;
  return {
    numPages: document.numPages,
    renderPage(pageNumber, canvas, scale) {
      let renderTask: RenderTask | null = null;
      let cancelled = false;
      const done = (async () => {
        const page = await document.getPage(pageNumber);
        if (cancelled) {
          page.cleanup();
          throw new PdfRenderCancelled();
        }
        const viewport = page.getViewport({ scale });
        const ratio = Math.max(1, globalThis.devicePixelRatio || 1);
        canvas.width = Math.ceil(viewport.width * ratio);
        canvas.height = Math.ceil(viewport.height * ratio);
        canvas.style.inlineSize = `${Math.ceil(viewport.width)}px`;
        canvas.style.blockSize = `${Math.ceil(viewport.height)}px`;
        renderTask = page.render({
          canvas,
          viewport: page.getViewport({ scale: scale * ratio }),
          background: "#ffffff"
        });
        try {
          await renderTask.promise;
        } catch (error) {
          if (cancelled) {
            throw new PdfRenderCancelled();
          }
          throw error;
        } finally {
          page.cleanup();
        }
      })();
      return {
        done,
        cancel() {
          cancelled = true;
          renderTask?.cancel();
        }
      };
    },
    destroy: () => task.destroy()
  };
}

export class PdfRenderCancelled extends Error {
  constructor() {
    super("pdf render cancelled");
  }
}

/** Page 1 as a data URL no wider than `maxWidth`, drawn on an offscreen canvas. */
export async function renderPdfThumbnail(
  document: PdfDocument,
  maxWidth: number
): Promise<string> {
  const canvas = globalThis.document.createElement("canvas");
  // Page 1's size is only known once it is drawn, so draw at a small fixed scale
  // and shrink further only if the result is still wider than the card wants.
  const render = document.renderPage(1, canvas, 0.5);
  await render.done;
  const scale = canvas.width > maxWidth ? maxWidth / canvas.width : 1;
  if (scale === 1) {
    return canvas.toDataURL("image/png");
  }
  const small = globalThis.document.createElement("canvas");
  small.width = Math.max(1, Math.round(canvas.width * scale));
  small.height = Math.max(1, Math.round(canvas.height * scale));
  const context = small.getContext("2d");
  if (!context) {
    throw new Error("canvas is unavailable");
  }
  context.drawImage(canvas, 0, 0, small.width, small.height);
  return small.toDataURL("image/png");
}
