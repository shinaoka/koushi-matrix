import { toExternalHttpUrl } from "../domain/externalLinks";
import { browserLinkMediaPort } from "./browser/linkMediaPort";
import type { LinkMediaPort, MediaSaveNameFacts } from "./linkMediaPort";
import { isTauriRuntime } from "./runtimeEnvironment";
import { tauriLinkMediaPort } from "./tauri/linkMediaPort";

function activePort(): LinkMediaPort {
  return isTauriRuntime() ? tauriLinkMediaPort : browserLinkMediaPort;
}

export async function openExternalHttpUrl(rawUrl: string): Promise<void> {
  const url = toExternalHttpUrl(rawUrl);
  if (!url) {
    return;
  }
  await activePort().openHttpUrl(url);
}

export function mediaSourceUrl(sourceRef: string, mimeType?: string | null): string {
  return activePort().mediaSourceUrl(sourceRef, mimeType);
}

export function renderableThumbnailSourceUrl(sourceRef: string): string | null {
  return activePort().renderableThumbnailSourceUrl(sourceRef);
}

export async function saveReadyMediaFile(
  sourceRef: string,
  filename: string,
  accountTabId?: string,
  saveName: MediaSaveNameFacts | null = null
): Promise<void> {
  const port = activePort();
  await port.saveMediaFile(sourceRef, filename, saveName, accountTabId);
}

/**
 * #2096: the bounded text of a downloaded attachment, for an inline preview.
 *
 * The backend refuses a file above its own preview limit, so the renderer never
 * holds a whole large document; a rejection is the "cannot preview" answer.
 */
export async function readDownloadedText(
  sourceRef: string,
  accountTabId?: string
): Promise<string> {
  return activePort().readDownloadedText(sourceRef, accountTabId);
}
