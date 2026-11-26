import * as pdfjs from "pdfjs-dist";
import type { PDFDocumentProxy, PDFPageProxy, RenderTask } from "pdfjs-dist";
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

/** Largest image pdf.js may decode, in pixels; a small file can inflate far past this. */
export const PDF_MAX_IMAGE_PIXELS = 16 * 1024 * 1024;
/** Largest canvas the viewer or thumbnail may allocate, in device pixels. */
export const PDF_MAX_CANVAS_PIXELS = 16 * 1024 * 1024;
/** How long a parse may take before the document is destroyed and reported failed. */
export const PDF_PARSE_TIMEOUT_MS = 15_000;

export interface PdfDocument {
  readonly numPages: number;
  /**
   * Draw one page, 1-based, onto `canvas`. `scale` is CSS pixels per PDF point;
   * `maxCssWidth` shrinks it so the page is no wider than that. The page's own
   * size is untrusted, so the result is also clamped to a fixed pixel budget.
   */
  renderPage(
    pageNumber: number,
    canvas: HTMLCanvasElement,
    options: { scale?: number; maxCssWidth?: number }
  ): PdfRender;
  destroy(): Promise<void>;
}

export interface PdfRender {
  /** Resolves when the page is drawn; rejects with `cancelled: true` when superseded. */
  done: Promise<PdfRenderResult>;
  cancel(): void;
}

export interface PdfRenderResult {
  /** The page drew, but part of it (a font the file does not embed) could not be. */
  degraded: boolean;
}

export class PdfRenderCancelled extends Error {
  constructor() {
    super("pdf render cancelled");
  }
}

export class PdfWorkerError extends Error {
  constructor(message: string) {
    super(message);
  }
}

/**
 * The worker is built here, not by pdf.js: a webview that refuses it (CSP,
 * missing asset) must surface as a failure. Left to itself pdf.js would fall
 * back to parsing the untrusted file on the main thread.
 */
function createWorker(): Worker {
  try {
    return new Worker(pdfWorkerUrl, { type: "module" });
  } catch (error) {
    throw new PdfWorkerError(error instanceof Error ? error.message : "worker unavailable");
  }
}

export async function openPdf(bytes: Uint8Array): Promise<PdfDocument> {
  const worker = createWorker();
  let failWorker: (error: Error) => void = () => undefined;
  const workerFailed = new Promise<never>((_resolve, reject) => {
    failWorker = reject;
  });
  worker.addEventListener("error", () => failWorker(new PdfWorkerError("worker failed")));
  pdfjs.GlobalWorkerOptions.workerPort = worker;
  let task: ReturnType<typeof pdfjs.getDocument>;
  try {
    task = pdfjs.getDocument({
      data: bytes,
      // The document is untrusted: no XFA, no network fetches for resources the
      // file does not embed, and a decode ceiling. pdf.js 6 has no eval path
      // (`isEvalSupported` is gone) and scripting lives in its viewer layer,
      // which this renderer does not use, so the CSP needs no `unsafe-eval`.
      enableXfa: false,
      useWorkerFetch: false,
      disableAutoFetch: true,
      disableStream: true,
      disableRange: true,
      maxImageSize: PDF_MAX_IMAGE_PIXELS
    });
  } catch (error) {
    worker.terminate();
    throw error;
  } finally {
    pdfjs.GlobalWorkerOptions.workerPort = null;
  }
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timedOut = new Promise<never>((_resolve, reject) => {
    timer = setTimeout(() => reject(new PdfWorkerError("parse timed out")), PDF_PARSE_TIMEOUT_MS);
  });
  const release = async () => {
    try {
      await task.destroy();
    } finally {
      worker.terminate();
    }
  };
  let document: PDFDocumentProxy;
  try {
    document = await Promise.race([task.promise, timedOut, workerFailed]);
  } catch (error) {
    // A corrupt, encrypted or hung file must not leak its worker.
    await release().catch(() => undefined);
    throw error;
  } finally {
    clearTimeout(timer);
  }
  // One RenderTask may hold a canvas at a time; the next render waits for it.
  const canvasBusy = new WeakMap<HTMLCanvasElement, Promise<unknown>>();
  return {
    numPages: document.numPages,
    renderPage(pageNumber, canvas, options) {
      let renderTask: RenderTask | null = null;
      let cancelled = false;
      const previous = canvasBusy.get(canvas);
      const done = (async () => {
        await previous;
        if (cancelled) {
          throw new PdfRenderCancelled();
        }
        const page = await document.getPage(pageNumber);
        try {
          if (cancelled) {
            throw new PdfRenderCancelled();
          }
          const base = page.getViewport({ scale: 1 });
          if (!(base.width > 0 && base.height > 0)) {
            throw new Error("page has no size");
          }
          const ratio = Math.max(1, globalThis.devicePixelRatio || 1);
          const scale = fitScale(base.width, base.height, ratio, options);
          const viewport = page.getViewport({ scale });
          canvas.width = Math.max(1, Math.ceil(viewport.width * ratio));
          canvas.height = Math.max(1, Math.ceil(viewport.height * ratio));
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
          }
          return { degraded: await hasMissingFonts(page) };
        } finally {
          page.cleanup();
        }
      })();
      canvasBusy.set(
        canvas,
        done.then(
          () => undefined,
          () => undefined
        )
      );
      return {
        done,
        cancel() {
          cancelled = true;
          renderTask?.cancel();
        }
      };
    },
    destroy: release
  };
}

/** CSS pixels per PDF point, within the requested width and the canvas pixel budget. */
function fitScale(
  width: number,
  height: number,
  ratio: number,
  options: { scale?: number; maxCssWidth?: number }
): number {
  let scale = options.scale ?? Number.POSITIVE_INFINITY;
  if (options.maxCssWidth !== undefined) {
    scale = Math.min(scale, options.maxCssWidth / width);
  }
  if (!Number.isFinite(scale)) {
    scale = 1;
  }
  const budgetScale = Math.sqrt(PDF_MAX_CANVAS_PIXELS / (width * height)) / ratio;
  return Math.max(Math.min(scale, budgetScale), Number.MIN_VALUE);
}

/**
 * pdf.js reports no "unsupported feature" event any more, and with `connect-src`
 * closed it cannot fetch CMaps or standard-font data. A font that fails to load
 * (a CJK font without an embedded CMap) or that the system cannot stand in for
 * is the visible case: its text draws wrongly or not at all while the render
 * still resolves. Images (JPX, JBIG2) fail inside the worker and are not
 * reported by pdf.js to the main thread, so they are not detected here.
 */
async function hasMissingFonts(page: PDFPageProxy): Promise<boolean> {
  try {
    const list = await page.getOperatorList();
    for (let index = 0; index < list.fnArray.length; index += 1) {
      if (list.fnArray[index] !== pdfjs.OPS.setFont) {
        continue;
      }
      const fontId = (list.argsArray[index] as unknown[] | null)?.[0];
      if (typeof fontId !== "string" || !page.commonObjs.has(fontId)) {
        continue;
      }
      const font = page.commonObjs.get(fontId) as
        | { missingFile?: boolean; systemFontInfo?: unknown }
        | string
        | undefined;
      // A font that failed to load resolves to its error text, not an object.
      if (typeof font !== "object" || font === null) {
        return true;
      }
      if (font.missingFile && !font.systemFontInfo) {
        return true;
      }
    }
  } catch {
    // The page drew; a failed inspection must not turn that into an error.
  }
  return false;
}

/** Page 1 as a data URL no wider than `maxWidth` CSS pixels (times the pixel ratio). */
export async function renderPdfThumbnail(
  document: PdfDocument,
  maxWidth: number
): Promise<{ dataUrl: string; degraded: boolean }> {
  const canvas = globalThis.document.createElement("canvas");
  try {
    const { degraded } = await document.renderPage(1, canvas, { maxCssWidth: maxWidth, scale: 2 })
      .done;
    return { dataUrl: canvas.toDataURL("image/png"), degraded };
  } finally {
    // Release the backing store now rather than when the canvas is collected.
    canvas.width = 0;
    canvas.height = 0;
  }
}
