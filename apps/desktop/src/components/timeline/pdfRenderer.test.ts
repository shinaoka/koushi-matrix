// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { destroy, getDocument, globalOptions } = vi.hoisted(() => ({
  destroy: vi.fn(async () => undefined),
  getDocument: vi.fn(),
  globalOptions: { workerPort: null as unknown }
}));
vi.mock("pdfjs-dist", () => ({
  getDocument: (options: unknown) => getDocument(options),
  GlobalWorkerOptions: globalOptions,
  OPS: { setFont: 37 }
}));
vi.mock("pdfjs-dist/build/pdf.worker.min.mjs?url", () => ({ default: "/worker.mjs" }));

import {
  PDF_MAX_CANVAS_PIXELS,
  PDF_MAX_IMAGE_PIXELS,
  PDF_PARSE_TIMEOUT_MS,
  PdfWorkerError,
  openPdf,
  renderPdfThumbnail
} from "./pdfRenderer";

class FakeWorker extends EventTarget {
  static instances: FakeWorker[] = [];
  terminate = vi.fn();
  constructor() {
    super();
    FakeWorker.instances.push(this);
  }
}

const bytes = new TextEncoder().encode("%PDF-1.4\n");

beforeEach(() => {
  FakeWorker.instances = [];
  vi.stubGlobal("Worker", FakeWorker);
  getDocument.mockReset();
  destroy.mockClear();
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("pdf.js hardening (#2096)", () => {
  it("surfaces a worker that cannot be constructed instead of parsing on the main thread", async () => {
    vi.stubGlobal(
      "Worker",
      class {
        constructor() {
          throw new Error("worker-src refused");
        }
      }
    );
    await expect(openPdf(bytes)).rejects.toBeInstanceOf(PdfWorkerError);
    // pdf.js was never asked to open the file, so no fake-worker fallback ran.
    expect(getDocument).not.toHaveBeenCalled();
  });

  it("fails when the worker errors after construction and releases it", async () => {
    getDocument.mockReturnValue({ promise: new Promise(() => undefined), destroy });
    const opening = openPdf(bytes);
    FakeWorker.instances[0].dispatchEvent(new Event("error"));
    await expect(opening).rejects.toBeInstanceOf(PdfWorkerError);
    expect(destroy).toHaveBeenCalledTimes(1);
    expect(FakeWorker.instances[0].terminate).toHaveBeenCalledTimes(1);
  });

  it("destroys the task and the worker when parsing rejects (corrupt or encrypted file)", async () => {
    getDocument.mockReturnValue({ promise: Promise.reject(new Error("password")), destroy });
    await expect(openPdf(bytes)).rejects.toThrow("password");
    expect(destroy).toHaveBeenCalledTimes(1);
    expect(FakeWorker.instances[0].terminate).toHaveBeenCalledTimes(1);
  });

  it("times a hung parse out and destroys it", async () => {
    vi.useFakeTimers();
    getDocument.mockReturnValue({ promise: new Promise(() => undefined), destroy });
    const opening = openPdf(bytes);
    const outcome = expect(opening).rejects.toThrow("parse timed out");
    await vi.advanceTimersByTimeAsync(PDF_PARSE_TIMEOUT_MS + 1);
    await outcome;
    expect(destroy).toHaveBeenCalledTimes(1);
    expect(FakeWorker.instances[0].terminate).toHaveBeenCalledTimes(1);
  });

  it("opens with the explicit worker and the untrusted-document limits", async () => {
    getDocument.mockImplementation(() => {
      expect(globalOptions.workerPort).toBe(FakeWorker.instances[0]);
      return { promise: Promise.resolve({ numPages: 1 }), destroy };
    });
    const document = await openPdf(bytes);
    expect(document.numPages).toBe(1);
    expect(getDocument).toHaveBeenCalledWith(
      expect.objectContaining({
        disableAutoFetch: true,
        disableStream: true,
        disableRange: true,
        maxImageSize: PDF_MAX_IMAGE_PIXELS
      })
    );
    await document.destroy();
    expect(FakeWorker.instances[0].terminate).toHaveBeenCalledTimes(1);
  });

  function fakePage(width: number, height: number, onRender: (canvas: HTMLCanvasElement) => Promise<void>) {
    return {
      getViewport: ({ scale }: { scale: number }) => ({ width: width * scale, height: height * scale }),
      render: ({ canvas }: { canvas: HTMLCanvasElement }) => ({
        promise: onRender(canvas),
        cancel: () => undefined
      }),
      getOperatorList: async () => ({ fnArray: [], argsArray: [] }),
      commonObjs: { has: () => false, get: () => undefined },
      cleanup: () => undefined
    };
  }

  it("sizes the thumbnail from the page, not from a hostile MediaBox", async () => {
    vi.spyOn(HTMLCanvasElement.prototype, "toDataURL").mockReturnValue("data:image/png;base64,AAAA");
    vi.stubGlobal("devicePixelRatio", 2);
    const sizes: Array<[number, number]> = [];
    const page = fakePage(14400, 14400, async (canvas) => {
      sizes.push([canvas.width, canvas.height]);
    });
    getDocument.mockReturnValue({
      promise: Promise.resolve({ numPages: 1, getPage: async () => page }),
      destroy
    });
    const document = await openPdf(bytes);
    await renderPdfThumbnail(document, 320);
    const [[width, height]] = sizes;
    expect(width).toBeLessThanOrEqual(320 * 2 + 1);
    expect(width * height).toBeLessThanOrEqual(PDF_MAX_CANVAS_PIXELS);
  });

  it("clamps a zoomed viewer render to the pixel budget", async () => {
    vi.stubGlobal("devicePixelRatio", 1);
    const sizes: Array<[number, number]> = [];
    const page = fakePage(14400, 14400, async (canvas) => {
      sizes.push([canvas.width, canvas.height]);
    });
    getDocument.mockReturnValue({
      promise: Promise.resolve({ numPages: 1, getPage: async () => page }),
      destroy
    });
    const document = await openPdf(bytes);
    await document.renderPage(1, window.document.createElement("canvas"), { scale: 3 }).done;
    const [[width, height]] = sizes;
    expect(width * height).toBeLessThanOrEqual(PDF_MAX_CANVAS_PIXELS * 1.01);
  });

  it("waits for the previous render to release the canvas before starting the next", async () => {
    const events: string[] = [];
    let releaseFirst: () => void = () => undefined;
    let calls = 0;
    const page = fakePage(100, 100, async () => {
      const index = (calls += 1);
      events.push(`start${index}`);
      if (index === 1) {
        await new Promise<void>((resolve) => {
          releaseFirst = resolve;
        });
      }
      events.push(`end${index}`);
    });
    getDocument.mockReturnValue({
      promise: Promise.resolve({ numPages: 1, getPage: async () => page }),
      destroy
    });
    const document = await openPdf(bytes);
    const canvas = window.document.createElement("canvas");
    const first = document.renderPage(1, canvas, { scale: 1 });
    await vi.waitFor(() => expect(events).toContain("start1"));
    const second = document.renderPage(1, canvas, { scale: 2 });
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(events).toEqual(["start1"]);
    releaseFirst();
    await first.done;
    await second.done;
    expect(events).toEqual(["start1", "end1", "start2", "end2"]);
  });
});
