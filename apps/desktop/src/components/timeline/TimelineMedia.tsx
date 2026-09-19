import { NativeModal } from "../ModalDialog";
import { FloatingLayer, floatingPlacementStyle, useFloatingPlacement } from "../floatingLayer";
import {
  Download,
  FileCode2,
  FileText,
  Forward,
  ImageIcon,
  Info,
  MessageCircle,
  MoreHorizontal,
  RefreshCw,
  Trash2,
  XCircle
} from "lucide-react";
import { type CSSProperties, type ReactNode, type RefObject, useCallback, useEffect, useRef, useState } from "react";

import { t } from "../../i18n/messages";
import { onMenuKeyDown } from "../ContextMenuSurface";
import type { MediaTransferProgress, TimelineItem } from "../../domain/coreEvents";
import type { TimelineMediaDownloadState } from "../../domain/types";
import type { TimelineForwardDestination } from "../../domain/projectionTypes";
import { mediaSourceUrl } from "../../backend/linkMediaRuntime";
import type { TimelineTransport } from "./TimelineTransport";

export type TimelineMediaViewerItem = {
  sourceUrl: string;
  downloadSourceUrl: string;
  filename: string;
  size: number | null;
  mimeType: string | null;
  width: number | null;
  height: number | null;
  encrypted: boolean;
  actions: TimelineMediaViewerActions;
  saveMediaFile?: TimelineTransport["saveMediaFile"];
};

type TimelineMediaViewerActions = {
  canForward: boolean;
  forwardDestinations: readonly TimelineForwardDestination[];
  onForward: (destinationRoomId: string) => void;
  canViewSource: boolean;
  onViewSource: () => void;
  canRedact: boolean;
  onRedact: () => void;
};

async function downloadMediaSource(sourceUrl: string, filename: string): Promise<void> {
  if (typeof document === "undefined") {
    return;
  }

  const safeFilename = filename.trim() || "download";
  let downloadUrl: string | null = null;
  let revokeUrl: string | null = null;

  if (typeof fetch === "function" && typeof URL.createObjectURL === "function") {
    try {
      const response = await fetch(sourceUrl);
      if (response.ok) {
        const blob = await response.blob();
        revokeUrl = URL.createObjectURL(blob);
        downloadUrl = revokeUrl;
      }
    } catch {
      downloadUrl = null;
    }
  }

  if (
    downloadUrl === null &&
    (/^https?:\/\//.test(sourceUrl) || sourceUrl.startsWith("data:") || sourceUrl.startsWith("blob:"))
  ) {
    downloadUrl = sourceUrl;
  }
  if (downloadUrl === null) {
    return;
  }

  const anchor = document.createElement("a");
  anchor.href = downloadUrl;
  anchor.download = safeFilename;
  anchor.rel = "noreferrer";
  anchor.style.display = "none";
  document.body.appendChild(anchor);
  anchor.click();
  document.body.removeChild(anchor);
  if (revokeUrl) {
    window.setTimeout(() => URL.revokeObjectURL(revokeUrl), 0);
  }
}

async function saveMediaSource(
  sourceUrl: string,
  displayUrl: string,
  filename: string,
  saveMediaFile?: TimelineTransport["saveMediaFile"]
): Promise<void> {
  if (saveMediaFile) {
    await saveMediaFile(sourceUrl, filename);
    return;
  }
  await downloadMediaSource(displayUrl, filename);
}

export function TimelineMediaAttachment({
  media,
  progress,
  downloadState,
  canDownload,
  onDownload,
  onOpenMediaViewer,
  onSaveMediaFile,
  viewerActions
}: {
  media: NonNullable<TimelineItem["media"]>;
  progress: MediaTransferProgress | null;
  downloadState?: TimelineMediaDownloadState;
  canDownload: boolean;
  onDownload: () => void;
  onOpenMediaViewer: (item: TimelineMediaViewerItem) => void;
  onSaveMediaFile?: TimelineTransport["saveMediaFile"];
  viewerActions: TimelineMediaViewerActions;
}) {
  const [detailsOpen, setDetailsOpen] = useState(false);
  const detailsTrigger = useRef<HTMLButtonElement>(null);
  const metadata = [
    media.mimetype,
    formatBytes(media.size),
    formatDimensions(media.width, media.height)
  ].filter((value): value is string => Boolean(value));
  const uploadProgressPercentValue = uploadProgressPercent(progress);
  const downloadProgress =
    downloadState?.kind === "pending" ? downloadState.progress : null;
  const downloadProgressPercent = uploadProgressPercent(downloadProgress);
  const Icon = media.kind === "Image" ? ImageIcon : FileText;
  const displayBox = timelineMediaDisplayBox(media.width, media.height);
  const mediaFrameStyle = {
    "--media-frame-inline-size": `${displayBox.inlineSize}px`,
    "--media-frame-aspect-ratio": `${displayBox.inlineSize} / ${displayBox.blockSize}`
  } as CSSProperties;
  const readyImageDownload =
    downloadState?.kind === "ready" && media.kind === "Image" ? downloadState : null;
  const readyImagePreview =
    readyImageDownload === null
      ? null
      : {
          sourceUrl: mediaSourceUrl(readyImageDownload.source_url),
          width: readyImageDownload.width,
          height: readyImageDownload.height
        };
  const readyImageViewerItem =
    readyImageDownload === null || readyImagePreview === null
      ? null
      : {
          sourceUrl: readyImagePreview.sourceUrl,
          downloadSourceUrl: readyImageDownload.source_url,
          filename: media.filename,
          size: media.size,
          mimeType: readyImageDownload?.mime_type ?? media.mimetype,
          width: readyImagePreview.width,
          height: readyImagePreview.height,
          encrypted: media.source.encrypted,
          actions: viewerActions,
          saveMediaFile: onSaveMediaFile
        };
  const progressPercent =
    uploadProgressPercentValue ?? downloadProgressPercent;
  const saveIntentRef = useRef(false);
  const requestDownload = useCallback(() => {
    saveIntentRef.current = true;
    onDownload();
  }, [onDownload]);
  useEffect(() => {
    if (downloadState?.kind === "failed") {
      saveIntentRef.current = false;
      return;
    }
    if (downloadState?.kind !== "ready" || !saveIntentRef.current) {
      return;
    }
    saveIntentRef.current = false;
    void saveMediaSource(
      downloadState.source_url,
      mediaSourceUrl(downloadState.source_url),
      media.filename,
      onSaveMediaFile
    );
  }, [downloadState, media.filename, onSaveMediaFile]);
  useEffect(() => {
    if (!detailsOpen) {
      return;
    }
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape" && !event.defaultPrevented && !event.isComposing && event.keyCode !== 229) {
        event.preventDefault();
        setDetailsOpen(false);
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [detailsOpen]);

  // #277: every image state owns the same figure box. Event metadata (or the
  // fixed fallback) determines its size before download starts; ready-state
  // metadata may describe the pixels but must never resize the timeline row.
  if (media.kind === "Image") {
    return (
      <div
        className="message-media message-media-image-ready"
        data-media-kind={media.kind}
        data-media-encrypted={media.source.encrypted || undefined}
        data-download-state={downloadState?.kind ?? "notRequested"}
      >
        <div className="message-media-figure" style={mediaFrameStyle}>
          {readyImagePreview && readyImageDownload ? (
            <button
              className="message-media-open"
              type="button"
              aria-label={t("timeline.mediaOpenFile")}
              onClick={() => {
                if (readyImageViewerItem) {
                  onOpenMediaViewer(readyImageViewerItem);
                }
              }}
            >
              <img
                className="message-media-image"
                src={readyImagePreview.sourceUrl}
                alt={media.filename}
                title={media.filename}
                width={readyImagePreview.width ?? undefined}
                height={readyImagePreview.height ?? undefined}
                loading="lazy"
              />
            </button>
          ) : (
            <div className="message-media-image-placeholder" aria-hidden="true">
              <Icon className="message-media-icon" size={28} />
              <span className="message-media-placeholder-title" dir="auto">
                {media.filename}
              </span>
              {metadata.length > 0 ? (
                <span className="message-media-placeholder-meta">
                  {metadata.join(" · ")}
                </span>
              ) : null}
              {downloadState?.kind === "pending" ? (
                <span className="message-media-placeholder-meta">
                  {t("timeline.mediaDownloadPending")}
                </span>
              ) : null}
              {downloadState?.kind === "failed" ? (
                <span className="message-media-error">
                  {t("timeline.mediaDownloadFailed")}
                </span>
              ) : null}
              {progressPercent !== null ? (
                <span className="message-media-placeholder-meta">
                  {t("timeline.mediaUploadProgress", { percent: progressPercent })}
                </span>
              ) : null}
            </div>
          )}
          {media.source.encrypted ? (
            <span className="message-media-image-badge">{t("timeline.encryptedMedia")}</span>
          ) : null}
          <div className="message-media-hover-actions">
            <button
              ref={detailsTrigger}
              className="message-media-hover-action"
              type="button"
              aria-label={t("timeline.mediaDetails", { filename: media.filename })}
              aria-expanded={detailsOpen}
              aria-haspopup="dialog"
              onClick={(event) => {
                event.stopPropagation();
                setDetailsOpen((current) => !current);
              }}
            >
              <Info size={16} />
            </button>
            {canDownload && downloadState?.kind === "failed" ? (
              <button
                className="message-media-hover-action"
                type="button"
                aria-label={t("timeline.mediaDownloadRetry")}
                onClick={(event) => {
                  event.stopPropagation();
                  requestDownload();
                }}
              >
                <RefreshCw size={16} />
              </button>
            ) : canDownload && readyImageDownload && readyImagePreview ? (
              <button
                className="message-media-hover-action"
                type="button"
                aria-label={t("timeline.downloadMedia", { filename: media.filename })}
                onClick={(event) => {
                  event.stopPropagation();
                  void saveMediaSource(
                    readyImageDownload.source_url,
                    readyImagePreview.sourceUrl,
                    media.filename,
                    onSaveMediaFile
                  );
                }}
              >
                <Download size={16} />
              </button>
            ) : canDownload ? (
              <button
                className="message-media-hover-action"
                type="button"
                disabled={downloadState?.kind === "pending"}
                aria-label={t("timeline.downloadMedia", { filename: media.filename })}
                onClick={(event) => {
                  event.stopPropagation();
                  requestDownload();
                }}
              >
                <Download size={16} />
              </button>
            ) : null}
          </div>
          {detailsOpen ? (
            <MediaDetailsPopup anchor={detailsTrigger} onClose={() => setDetailsOpen(false)}>
              <div className="message-media-details-title" dir="auto">
                {media.filename}
              </div>
              <div className="message-media-details-list">
                {metadata.map((value) => (
                  <span key={value}>{value}</span>
                ))}
                {media.source.encrypted ? <span>{t("timeline.encryptedMedia")}</span> : null}
              </div>
              <button
                className="message-media-details-close"
                type="button"
                aria-label={t("timeline.closeMediaDetails")}
                onClick={() => setDetailsOpen(false)}
              >
                <XCircle size={16} />
              </button>
            </MediaDetailsPopup>
          ) : null}
          {progressPercent !== null ? (
            <div
              className="message-media-progress-overlay"
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={progressPercent}
            >
              <span style={{ width: `${progressPercent}%` }} />
            </div>
          ) : null}
        </div>
      </div>
    );
  }

  return (
    <div
      className="message-media"
      data-media-kind={media.kind}
      data-media-encrypted={media.source.encrypted || undefined}
      data-download-state={downloadState?.kind ?? "notRequested"}
    >
      <Icon className="message-media-icon" size={18} aria-hidden="true" />
      <div className="message-media-main">
        <div className="message-media-title" dir="auto">
          {media.filename}
        </div>
        <div className="message-media-meta">
          {metadata.length > 0 ? <span>{metadata.join(" · ")}</span> : null}
          {media.source.encrypted ? (
            <span className="message-media-badge">{t("timeline.encryptedMedia")}</span>
          ) : null}
          {downloadState?.kind === "pending" ? (
            <span>{t("timeline.mediaDownloadPending")}</span>
          ) : null}
          {downloadState?.kind === "failed" ? (
            <span className="message-media-error">
              {t("timeline.mediaDownloadFailed")}
            </span>
          ) : null}
          {progressPercent !== null ? (
            <span>{t("timeline.mediaUploadProgress", { percent: progressPercent })}</span>
          ) : null}
        </div>
        {progressPercent !== null ? (
          <div
            className="message-media-progress"
            role="progressbar"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={progressPercent}
          >
            <span style={{ width: `${progressPercent}%` }} />
          </div>
        ) : null}
      </div>
      {canDownload ? (
        downloadState?.kind === "failed" ? (
          <button
            className="message-media-download message-media-retry"
            type="button"
            aria-label={t("timeline.mediaDownloadRetry")}
            onClick={requestDownload}
          >
            <RefreshCw size={15} />
          </button>
        ) : downloadState?.kind === "ready" ? (
          <button
            className="message-media-download"
            type="button"
            aria-label={t("timeline.downloadMedia", { filename: media.filename })}
            onClick={() => {
              void saveMediaSource(
                downloadState.source_url,
                mediaSourceUrl(downloadState.source_url),
                media.filename,
                onSaveMediaFile
              );
            }}
          >
            <Download size={15} />
          </button>
        ) : (
          <button
            className="message-media-download"
            type="button"
            disabled={downloadState?.kind === "pending"}
            aria-label={t("timeline.downloadMedia", { filename: media.filename })}
            onClick={requestDownload}
          >
            <Download size={15} />
          </button>
        )
      ) : null}
    </div>
  );
}

function MediaDetailsPopup({ anchor, onClose, children }: {
  anchor: RefObject<HTMLButtonElement | null>; onClose: () => void; children: ReactNode;
}) {
  const placement = useFloatingPlacement({ anchorRef: anchor, placement: "below", align: "end", inlineSize: 260, blockSize: 180 });
  return <FloatingLayer><div className="message-media-details-popover" role="dialog"
    aria-label={t("timeline.mediaDetailsTitle")} style={floatingPlacementStyle(placement)}
    onKeyDown={event => {
      if (event.key === "Escape" && !event.nativeEvent.isComposing) {
        event.preventDefault(); event.stopPropagation(); onClose(); anchor.current?.focus();
      }
    }}>{children}</div></FloatingLayer>;
}

export function TimelineMediaViewer({
  item,
  onClose
}: {
  item: TimelineMediaViewerItem;
  onClose: () => void;
}) {
  const [isActionMenuOpen, setActionMenuOpen] = useState(false);
  const [isForwardMenuOpen, setForwardMenuOpen] = useState(false);
  const dialogRef = useRef<HTMLElement | null>(null);
  const closeButtonRef = useRef<HTMLButtonElement | null>(null);
  const actionMenuControlRef = useRef<HTMLDivElement>(null);
  const firstActionMenuItemRef = useRef<HTMLButtonElement>(null);

  const closeActionMenu = useCallback(() => {
    setActionMenuOpen(false);
    setForwardMenuOpen(false);
  }, []);

  useEffect(() => {
    closeButtonRef.current?.focus();
  }, []);

  useEffect(() => {
    if (!isActionMenuOpen) {
      return;
    }
    firstActionMenuItemRef.current?.focus();
  }, [isActionMenuOpen]);

  useEffect(() => {
    if (!isActionMenuOpen) {
      return;
    }
    const handlePointerDown = (event: PointerEvent) => {
      const control = actionMenuControlRef.current;
      if (!control || control.contains(event.target as Node)) {
        return;
      }
      closeActionMenu();
    };
    document.addEventListener("pointerdown", handlePointerDown);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
    };
  }, [closeActionMenu, isActionMenuOpen]);

  const metadata = [
    formatBytes(item.size),
    item.mimeType,
    formatDimensions(item.width, item.height)
  ].filter((value): value is string => Boolean(value));
  const canForward = item.actions.canForward && item.actions.forwardDestinations.length > 0;
  const hasActionMenu = canForward || item.actions.canViewSource || item.actions.canRedact;

  return (
    <NativeModal onDismiss={() => { if (isActionMenuOpen) closeActionMenu(); else onClose(); }} aria-label={t("timeline.mediaViewer")}
      className="timeline-media-viewer-overlay"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) {
          onClose();
        }
      }}
    >
      <section
        ref={dialogRef}
        className="timeline-media-viewer"
        tabIndex={-1}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <div className="timeline-media-viewer-toolbar">
          <div className="timeline-media-viewer-info">
            <div className="timeline-media-viewer-title" dir="auto">
              {item.filename}
            </div>
            {metadata.length > 0 || item.encrypted ? (
              <div className="timeline-media-viewer-meta">
                {metadata.length > 0 ? <span>{metadata.join(" · ")}</span> : null}
                {item.encrypted ? <span>{t("timeline.encryptedMedia")}</span> : null}
              </div>
            ) : null}
          </div>
          <div className="timeline-media-viewer-actions">
            <button
              className="timeline-media-viewer-action"
              type="button"
              aria-label={t("timeline.downloadMedia", { filename: item.filename })}
              onClick={() => {
                void saveMediaSource(
                  item.downloadSourceUrl,
                  item.sourceUrl,
                  item.filename,
                  item.saveMediaFile
                );
              }}
            >
              <Download size={20} />
            </button>
            {hasActionMenu ? (
              <div className="timeline-media-viewer-menu-control" ref={actionMenuControlRef}>
                <button
                  className="timeline-media-viewer-action"
                  type="button"
                  aria-label={t("timeline.messageActions")}
                  aria-expanded={isActionMenuOpen}
                  aria-haspopup="menu"
                  onClick={() => {
                    setForwardMenuOpen(false);
                    setActionMenuOpen((current) => !current);
                  }}
                >
                  <MoreHorizontal size={22} />
                </button>
                {isActionMenuOpen ? (
                  <div
                    className="timeline-media-viewer-menu"
                    role="menu"
                    aria-label={t("timeline.messageActions")}
                    onKeyDown={(event) => {
                      if (event.key === "Escape" && !event.nativeEvent.isComposing && event.keyCode !== 229) {
                        event.preventDefault();
                        closeActionMenu();
                        return;
                      }
                      onMenuKeyDown(event, event.currentTarget);
                    }}
                  >
                    {canForward ? (
                      <div className="timeline-media-viewer-forward-control">
                        <button
                          ref={firstActionMenuItemRef}
                          className="timeline-media-viewer-menu-item"
                          type="button"
                          role="menuitem"
                          aria-haspopup="menu"
                          aria-expanded={isForwardMenuOpen}
                          onClick={() => setForwardMenuOpen((current) => !current)}
                        >
                          <Forward size={17} aria-hidden="true" />
                          <span>{t("timeline.forwardMessage")}</span>
                        </button>
                        {isForwardMenuOpen ? (
                          <div className="timeline-media-viewer-forward-menu" role="menu">
                            {item.actions.forwardDestinations.map((destination) => (
                              <button
                                className="timeline-media-viewer-menu-item"
                                type="button"
                                role="menuitem"
                                key={destination.room_id}
                                onClick={() => {
                                  item.actions.onForward(destination.room_id);
                                  onClose();
                                }}
                              >
                                <MessageCircle size={17} aria-hidden="true" />
                                <span dir="auto">{destination.display_name}</span>
                              </button>
                            ))}
                          </div>
                        ) : null}
                      </div>
                    ) : null}
                    {item.actions.canViewSource ? (
                      <button
                        ref={!canForward ? firstActionMenuItemRef : undefined}
                        className="timeline-media-viewer-menu-item"
                        type="button"
                        role="menuitem"
                        onClick={() => {
                          item.actions.onViewSource();
                          onClose();
                        }}
                      >
                        <FileCode2 size={17} aria-hidden="true" />
                        <span>{t("timeline.viewSource")}</span>
                      </button>
                    ) : null}
                    {item.actions.canRedact ? (
                      <button
                        ref={
                          !canForward && !item.actions.canViewSource
                            ? firstActionMenuItemRef
                            : undefined
                        }
                        className="timeline-media-viewer-menu-item is-destructive"
                        type="button"
                        role="menuitem"
                        onClick={() => {
                          item.actions.onRedact();
                          onClose();
                        }}
                      >
                        <Trash2 size={17} aria-hidden="true" />
                        <span>{t("timeline.removeMessage")}</span>
                      </button>
                    ) : null}
                  </div>
                ) : null}
              </div>
            ) : null}
            <button
              ref={closeButtonRef}
              className="timeline-media-viewer-action timeline-media-viewer-close"
              type="button"
              aria-label={t("mediaGallery.close")}
              onClick={onClose}
            >
              <XCircle size={24} />
            </button>
          </div>
        </div>
        <div className="timeline-media-viewer-stage">
          <img
            className="timeline-media-viewer-image"
            src={item.sourceUrl}
            alt={item.filename}
            title={item.filename}
            width={item.width ?? undefined}
            height={item.height ?? undefined}
          />
        </div>
      </section>
    </NativeModal>
  );
}

function formatBytes(size: number | null): string | null {
  if (size === null || !Number.isFinite(size) || size < 0) {
    return null;
  }
  if (size < 1024) {
    return `${size} B`;
  }
  if (size < 1024 * 1024) {
    return `${Math.round(size / 1024)} KB`;
  }
  return `${(size / (1024 * 1024)).toFixed(1)} MB`;
}

function formatDimensions(width: number | null, height: number | null): string | null {
  if (!width || !height) {
    return null;
  }
  return `${width}x${height}`;
}

const TIMELINE_MEDIA_MAX_INLINE_PX = 420;
const TIMELINE_MEDIA_MAX_BLOCK_PX = 260;
const TIMELINE_MEDIA_FALLBACK_BOX = {
  inlineSize: 347,
  blockSize: TIMELINE_MEDIA_MAX_BLOCK_PX
} as const;

function timelineMediaDisplayBox(
  width: number | null | undefined,
  height: number | null | undefined
): { inlineSize: number; blockSize: number } {
  if (!width || !height || width <= 0 || height <= 0) {
    return TIMELINE_MEDIA_FALLBACK_BOX;
  }
  const scale = Math.min(
    TIMELINE_MEDIA_MAX_INLINE_PX / width,
    TIMELINE_MEDIA_MAX_BLOCK_PX / height,
    1
  );
  return {
    inlineSize: Math.round(width * scale),
    blockSize: Math.round(height * scale)
  };
}

export const timelineMediaDisplayBoxForTests = timelineMediaDisplayBox;

function uploadProgressPercent(progress: MediaTransferProgress | null): number | null {
  if (!progress || progress.total <= 0) {
    return null;
  }
  return Math.max(0, Math.min(100, Math.round((progress.current / progress.total) * 100)));
}
