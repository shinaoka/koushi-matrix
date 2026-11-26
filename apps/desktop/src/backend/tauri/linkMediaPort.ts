import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";

import { t, tInvariant } from "../../i18n/messages";
import type { LinkMediaPort, TimelineFormattedBodyDto } from "../linkMediaPort";
import { renderableMediaDownloadSourceUrl } from "../linkMediaPort";
import { isRendererSelectedAccountTabId } from "../client";

function safeDownloadFilename(filename: string): string {
  const trimmed = filename.trim();
  return (trimmed || "download").replace(/[\\/:*?"<>|]+/g, "_");
}

function platformProtocolBase(protocol: string): string {
  const platform =
    typeof navigator === "undefined"
      ? ""
      : ((navigator as Navigator & { userAgentData?: { platform?: string } })
          .userAgentData?.platform ?? navigator.platform);
  return /win/i.test(platform)
    ? `http://${protocol}.localhost/`
    : `${protocol}://localhost/`;
}

function renderableThumbnailProtocolBase(): string {
  return platformProtocolBase("koushi-thumbnail");
}

function mediaDownloadProtocolBase(): string {
  return platformProtocolBase("koushi-media");
}

export const tauriLinkMediaPort: LinkMediaPort = {
  async openHttpUrl(url) {
    try {
      await openUrl(url);
    } catch {
      window.open(url, "_blank", "noopener,noreferrer");
    }
  },
  mediaSourceUrl(sourceRef, mimeType) {
    if (sourceRef.startsWith("download/")) {
      return (
        renderableMediaDownloadSourceUrl(
          sourceRef,
          mimeType,
          mediaDownloadProtocolBase()
        ) ?? ""
      );
    }
    if (
      sourceRef.startsWith("http://") ||
      sourceRef.startsWith("https://") ||
      sourceRef.startsWith("asset://") ||
      sourceRef.startsWith("data:") ||
      sourceRef.startsWith("blob:")
    ) {
      return sourceRef;
    }
    try {
      const localPath = sourceRef.startsWith("file://")
        ? decodeURIComponent(new URL(sourceRef).pathname)
        : sourceRef;
      return convertFileSrc(localPath);
    } catch {
      return sourceRef;
    }
  },
  renderableMediaDownloadSourceUrl(sourceRef, mimeType) {
    return renderableMediaDownloadSourceUrl(
      sourceRef,
      mimeType,
      mediaDownloadProtocolBase()
    );
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
  async saveMediaFile(sourceRef, filename, saveName, accountTabId) {
    if (accountTabId !== undefined && !isRendererSelectedAccountTabId(accountTabId)) {
      throw new Error("account tab is no longer selected");
    }
    const safeFilename = safeDownloadFilename(filename);
    // #1135: Core owns the naming policy, including the generic-upload allowlist,
    // so it receives the attachment's own filename. The catalog owns the generated
    // prefix, so the renderer resolves it here.
    const timestampMs = saveName?.timestampMs ?? null;
    const defaultPath = await invoke<string>("default_media_save_path", {
      filename,
      mediaKind: saveName?.kind ?? "file",
      timestampMs,
      utcOffsetMinutes:
        timestampMs === null ? 0 : -new Date(timestampMs).getTimezoneOffset(),
      localNamePrefix: tInvariant("timeline.downloadDefaultImageName")
    }).catch(() => safeFilename);
    const selected = await saveDialog({
      title: t("timeline.downloadMedia", { filename: safeFilename }),
      defaultPath
    });
    if (!selected) {
      return;
    }
    if (accountTabId !== undefined && !isRendererSelectedAccountTabId(accountTabId)) {
      throw new Error("account tab is no longer selected");
    }
    await invoke("save_downloaded_media", {
      sourceRef,
      destinationPath: selected,
      ...(accountTabId === undefined ? {} : { accountTabId })
    });
  },
  async readDownloadedText(sourceRef, accountTabId) {
    return invoke<string>("read_downloaded_media_text", {
      sourceRef,
      ...(accountTabId === undefined ? {} : { accountTabId })
    });
  },
  async readDownloadedMarkdown(sourceRef, accountTabId) {
    return invoke<TimelineFormattedBodyDto>("read_downloaded_media_markdown", {
      sourceRef,
      ...(accountTabId === undefined ? {} : { accountTabId })
    });
  },
  async readDownloadedPdf(sourceRef, accountTabId) {
    // A raw IPC response arrives as an ArrayBuffer, not a JSON number array.
    const bytes = await invoke<ArrayBuffer | number[]>("read_downloaded_media_pdf", {
      sourceRef,
      ...(accountTabId === undefined ? {} : { accountTabId })
    });
    return new Uint8Array(bytes);
  }
};
