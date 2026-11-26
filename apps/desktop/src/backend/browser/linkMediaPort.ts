import type { LinkMediaPort } from "../linkMediaPort";
import { renderableMediaDownloadSourceUrl } from "../linkMediaPort";

const BROWSER_MEDIA_PROTOCOL_BASE = "koushi-media://localhost/";

export const browserLinkMediaPort: LinkMediaPort = {
  async openHttpUrl(url) {
    window.open(url, "_blank", "noopener,noreferrer");
  },
  mediaSourceUrl(sourceRef, mimeType) {
    if (sourceRef.startsWith("download/")) {
      return (
        renderableMediaDownloadSourceUrl(
          sourceRef,
          mimeType,
          BROWSER_MEDIA_PROTOCOL_BASE
        ) ?? ""
      );
    }
    return sourceRef;
  },
  renderableThumbnailSourceUrl(sourceRef) {
    return /^(?:data:|blob:|asset:|https?:\/\/)/.test(sourceRef) ? sourceRef : null;
  },
  renderableMediaDownloadSourceUrl(sourceRef, mimeType) {
    return renderableMediaDownloadSourceUrl(
      sourceRef,
      mimeType,
      BROWSER_MEDIA_PROTOCOL_BASE
    );
  },
  async saveMediaFile() {},
  async readDownloadedText() {
    // The browser QA shell has no media cache to read; the card falls back to
    // the plain download card (#2096).
    throw new Error("text preview is unavailable in the browser shell");
  },
  async readDownloadedMarkdown() {
    throw new Error("document preview is unavailable in the browser shell");
  },
  async readDownloadedPdf() {
    throw new Error("PDF preview is unavailable in the browser shell");
  }
};
