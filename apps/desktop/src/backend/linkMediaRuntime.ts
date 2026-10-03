import { toExternalHttpUrl } from "../domain/externalLinks";
import { browserLinkMediaPort } from "./browser/linkMediaPort";
import type { LinkMediaPort } from "./linkMediaPort";
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

export function mediaSourceUrl(sourceUrl: string): string {
  return activePort().mediaSourceUrl(sourceUrl);
}

export function renderableThumbnailSourceUrl(sourceRef: string): string | null {
  return activePort().renderableThumbnailSourceUrl(sourceRef);
}

export async function saveReadyMediaFile(
  sourceUrl: string,
  filename: string
): Promise<void> {
  await activePort().saveMediaFile(sourceUrl, filename);
}
