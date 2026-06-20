import { convertFileSrc } from "@tauri-apps/api/core";

/**
 * Convert a Rust-returned local media path into a URL the WebView can render.
 * Data URLs and http(s) URLs pass through unchanged; filesystem paths are
 * converted through Tauri's asset protocol.
 */
export function mediaSourceUrl(sourceUrl: string): string {
  if (
    sourceUrl.startsWith("http://") ||
    sourceUrl.startsWith("https://") ||
    sourceUrl.startsWith("asset://") ||
    sourceUrl.startsWith("data:") ||
    sourceUrl.startsWith("blob:")
  ) {
    return sourceUrl;
  }
  try {
    const localPath = sourceUrl.startsWith("file://")
      ? decodeURIComponent(new URL(sourceUrl).pathname)
      : sourceUrl;
    return convertFileSrc(localPath);
  } catch {
    return sourceUrl;
  }
}
