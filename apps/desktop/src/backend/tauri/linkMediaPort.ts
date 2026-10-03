import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";

import { t } from "../../i18n/messages";
import type { LinkMediaPort } from "../linkMediaPort";

function safeDownloadFilename(filename: string): string {
  const trimmed = filename.trim();
  return (trimmed || "download").replace(/[\\/:*?"<>|]+/g, "_");
}

function renderableThumbnailProtocolBase(): string {
  const platform =
    typeof navigator === "undefined"
      ? ""
      : ((navigator as Navigator & { userAgentData?: { platform?: string } })
          .userAgentData?.platform ?? navigator.platform);
  return /win/i.test(platform)
    ? "http://koushi-thumbnail.localhost/"
    : "koushi-thumbnail://localhost/";
}

export const tauriLinkMediaPort: LinkMediaPort = {
  async openHttpUrl(url) {
    try {
      await openUrl(url);
    } catch {
      window.open(url, "_blank", "noopener,noreferrer");
    }
  },
  mediaSourceUrl(sourceUrl) {
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
  },
  renderableThumbnailSourceUrl(sourceRef) {
    if (sourceRef.startsWith("data:") || sourceRef.startsWith("blob:")) {
      return sourceRef;
    }
    if (!/^(?:avatar|link-preview)\/[0-9a-f]{16}$/.test(sourceRef)) {
      return null;
    }
    return `${renderableThumbnailProtocolBase()}${sourceRef}`;
  },
  async saveMediaFile(sourceUrl, filename) {
    const safeFilename = safeDownloadFilename(filename);
    const defaultPath = await invoke<string>("default_media_save_path", {
      filename: safeFilename
    }).catch(() => safeFilename);
    const selected = await saveDialog({
      title: t("timeline.downloadMedia", { filename: safeFilename }),
      defaultPath
    });
    if (!selected) {
      return;
    }
    await invoke("save_downloaded_media", {
      sourceUrl,
      destinationPath: selected
    });
  }
};
