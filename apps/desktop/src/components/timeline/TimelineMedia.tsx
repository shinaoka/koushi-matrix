import { FloatingLayer, floatingPlacementStyle, useFloatingPlacement } from "../floatingLayer";
import {
  Download,
  File as FileGenericIcon,
  FileArchive,
  FileAudio,
  FileCode2,
  FileImage,
  FileSpreadsheet,
  FileText,
  FileVideo,
  Presentation,
  ImageIcon,
  Info,
  RefreshCw,
  XCircle
} from "lucide-react";
import { type CSSProperties, Fragment, type ReactNode, type RefObject, useCallback, useEffect, useRef, useState } from "react";

import { t } from "../../i18n/messages";
import { Tooltip } from "../Tooltip";
import { useOutsidePressDismissal } from "../popupDismissal";
import type { MediaTransferProgress, TimelineItem } from "../../domain/coreEvents";
import type { TimelineMediaDownloadState } from "../../domain/types";
import { mediaSourceUrl } from "../../backend/linkMediaRuntime";
import type { MediaSaveNameFacts } from "../../backend/linkMediaPort";
import type { TimelineTransport } from "./TimelineTransport";
import { type FileTypeCategory, fileTypeCategory } from "./fileTypeCategory";
import { formatDimensions } from "../../app/uiShared";
import { saveMediaSource, type MediaViewerEntry } from "../mediaViewer";

const FILE_TYPE_ICONS: Record<FileTypeCategory, typeof FileText> = {
  spreadsheet: FileSpreadsheet,
  document: FileText,
  slides: Presentation,
  pdf: FileText,
  archive: FileArchive,
  image: FileImage,
  audio: FileAudio,
  video: FileVideo,
  text: FileCode2,
  other: FileGenericIcon
};

export function TimelineMediaAttachment({
  media,
  progress,
  downloadState,
  canDownload,
  onDownload,
  onOpenMediaViewer,
  onSaveMediaFile,
  saveName,
  viewerEntry
}: {
  media: NonNullable<TimelineItem["media"]>;
  progress: MediaTransferProgress | null;
  downloadState?: TimelineMediaDownloadState;
  canDownload: boolean;
  onDownload: () => void;
  onOpenMediaViewer: (entry: MediaViewerEntry) => void;
  onSaveMediaFile?: TimelineTransport["saveMediaFile"];
  saveName: MediaSaveNameFacts | null;
  /** The same image as the shared viewer displays it (#1810). */
  viewerEntry: MediaViewerEntry | null;
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
  const readyImageMimeType = readyImageDownload?.mime_type ?? media.mimetype;
  // An unknown image type has no protocol token; fall back to the plain
  // download card instead of an <img> with an empty src.
  const readyImageSourceUrl =
    readyImageDownload === null
      ? null
      : mediaSourceUrl(readyImageDownload.source_ref, readyImageMimeType) || null;
  const readyImagePreview =
    readyImageDownload === null || readyImageSourceUrl === null
      ? null
      : {
          sourceUrl: readyImageSourceUrl,
          width: readyImageDownload.width,
          height: readyImageDownload.height
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
      downloadState.source_ref,
      mediaSourceUrl(downloadState.source_ref, downloadState.mime_type ?? media.mimetype),
      media.filename,
      saveName,
      onSaveMediaFile
    );
  }, [downloadState, media.filename, media.mimetype, onSaveMediaFile, saveName]);
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
            <Tooltip label={t("timeline.mediaOpenFile")} placement="above">
              {(tooltipProps) => (
                <button
                  className="message-media-open"
                  type="button"
                  aria-label={t("timeline.mediaOpenFile")}
                  onClick={() => {
                    if (viewerEntry) {
                      onOpenMediaViewer(viewerEntry);
                    }
                  }}
                  {...tooltipProps}
                >
                  {/* No native `title` here: the button's explanation is the
                      action, and the attachment name stays in `alt` and in the
                      media details popover. A native filename popup would paint
                      over the styled bubble. */}
                  <img
                    className="message-media-image"
                    src={readyImagePreview.sourceUrl}
                    alt={media.filename}
                    width={readyImagePreview.width ?? undefined}
                    height={readyImagePreview.height ?? undefined}
                    loading="lazy"
                  />
                </button>
              )}
            </Tooltip>
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
            <Tooltip
              label={t("timeline.mediaDetails", { filename: media.filename })}
              placement="above"
            >
              {(tooltipProps) => (
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
                  {...tooltipProps}
                >
                  <Info size={16} />
                </button>
              )}
            </Tooltip>
            {canDownload && downloadState?.kind === "failed" ? (
              <Tooltip label={t("timeline.mediaDownloadRetry")} placement="above">
                {(tooltipProps) => (
                  <button
                    className="message-media-hover-action"
                    type="button"
                    aria-label={t("timeline.mediaDownloadRetry")}
                    onClick={(event) => {
                      event.stopPropagation();
                      requestDownload();
                    }}
                    {...tooltipProps}
                  >
                    <RefreshCw size={16} />
                  </button>
                )}
              </Tooltip>
            ) : canDownload && readyImageDownload && readyImagePreview ? (
              <Tooltip
                label={t("timeline.downloadMedia", { filename: media.filename })}
                placement="above"
              >
                {(tooltipProps) => (
                  <button
                    className="message-media-hover-action"
                    type="button"
                    aria-label={t("timeline.downloadMedia", { filename: media.filename })}
                    onClick={(event) => {
                      event.stopPropagation();
                      void saveMediaSource(
                        readyImageDownload.source_ref,
                        readyImagePreview.sourceUrl,
                        media.filename,
                        saveName,
                        onSaveMediaFile
                      );
                    }}
                    {...tooltipProps}
                  >
                    <Download size={16} />
                  </button>
                )}
              </Tooltip>
            ) : canDownload ? (
              <Tooltip
                label={t("timeline.downloadMedia", { filename: media.filename })}
                placement="above"
              >
                {(tooltipProps) => (
                  <button
                    className="message-media-hover-action"
                    type="button"
                    disabled={downloadState?.kind === "pending"}
                    aria-label={t("timeline.downloadMedia", { filename: media.filename })}
                    onClick={(event) => {
                      event.stopPropagation();
                      requestDownload();
                    }}
                    {...tooltipProps}
                  >
                    <Download size={16} />
                  </button>
                )}
              </Tooltip>
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
              <Tooltip label={t("timeline.closeMediaDetails")} placement="above">
                {(tooltipProps) => (
                  <button
                    className="message-media-details-close"
                    type="button"
                    aria-label={t("timeline.closeMediaDetails")}
                    onClick={() => setDetailsOpen(false)}
                    {...tooltipProps}
                  >
                    <XCircle size={16} />
                  </button>
                )}
              </Tooltip>
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

  // #1784: WeChat-style file card. The type tile carries the file type, so the
  // card never prints the raw MIME type or a per-file encryption pill; the
  // size line doubles as the transfer status line.
  const fileType = fileTypeCategory(media.filename, media.mimetype, media.kind);
  const FileTypeIcon = FILE_TYPE_ICONS[fileType];
  const statusParts =
    downloadState?.kind === "failed"
      ? [t("timeline.mediaDownloadFailed")]
      : downloadState?.kind === "pending"
        ? [
            t("timeline.mediaDownloadPending"),
            downloadProgressPercent === null
              ? null
              : t("timeline.mediaUploadProgress", { percent: downloadProgressPercent })
          ]
        : [
            formatBytes(media.size),
            uploadProgressPercentValue === null
              ? null
              : t("timeline.mediaUploadProgress", { percent: uploadProgressPercentValue })
          ];
  const statusText = statusParts.filter((value): value is string => Boolean(value));
  const cardAction: (() => void) | null = !canDownload
    ? null
    : downloadState?.kind === "pending"
      ? null
      : downloadState?.kind === "ready"
        ? () => {
            void saveMediaSource(
              downloadState.source_ref,
              mediaSourceUrl(downloadState.source_ref, downloadState.mime_type ?? media.mimetype),
              media.filename,
              saveName,
              onSaveMediaFile
            );
          }
        : requestDownload;
  const failed = downloadState?.kind === "failed";
  const tileLabel = failed
    ? t("timeline.mediaDownloadRetry")
    : t("timeline.downloadMedia", { filename: media.filename });
  const tileContent = (
    <>
      {failed ? (
        <RefreshCw className="message-media-type-icon" size={22} aria-hidden="true" />
      ) : (
        <FileTypeIcon
          className="message-media-type-icon"
          size={downloadState?.kind === "pending" ? 18 : 24}
          aria-hidden="true"
        />
      )}
      {canDownload && !failed && downloadState?.kind !== "pending" ? (
        <span className="message-media-type-overlay" aria-hidden="true">
          <Download size={20} />
        </span>
      ) : null}
      {downloadState?.kind === "pending" ? (
        <svg
          className="message-media-type-ring"
          data-indeterminate={downloadProgressPercent === null || undefined}
          viewBox="0 0 52 52"
          aria-hidden="true"
        >
          <circle className="message-media-type-ring-track" cx="26" cy="26" r="23" />
          <circle
            className="message-media-type-ring-value"
            cx="26"
            cy="26"
            r="23"
            pathLength={100}
            strokeDasharray={`${downloadProgressPercent ?? 25} 100`}
          />
        </svg>
      ) : null}
    </>
  );

  return (
    <div
      className="message-media message-media-file"
      data-media-kind={media.kind}
      data-media-encrypted={media.source.encrypted || undefined}
      data-download-state={downloadState?.kind ?? "notRequested"}
      data-actionable={cardAction ? "true" : undefined}
      onClick={
        cardAction
          ? (event) => {
              const target = event.target;
              // The tile is its own button; only the card background acts
              // here, and never while the user is selecting the filename.
              if (target instanceof Element && target.closest("button, a")) {
                return;
              }
              const selection = window.getSelection();
              if (selection && !selection.isCollapsed && event.currentTarget.contains(selection.anchorNode)) {
                return;
              }
              cardAction();
            }
          : undefined
      }
    >
      <div className="message-media-main">
        <div className="message-media-title" dir="auto" title={media.filename}>
          {media.filename}
        </div>
        <div className="message-media-meta">
          {statusText.map((part, index) => (
            <Fragment key={part}>
              {index > 0 ? " · " : null}
              <span className={failed ? "message-media-error" : undefined}>{part}</span>
            </Fragment>
          ))}
        </div>
      </div>
      {canDownload ? (
        <Tooltip label={tileLabel} placement="above">
          {(tooltipProps) => (
            <button
              className={`message-media-type-tile message-media-download${failed ? " message-media-retry" : ""}`}
              type="button"
              data-file-type={fileType}
              disabled={downloadState?.kind === "pending"}
              aria-label={tileLabel}
              onClick={cardAction ?? undefined}
              {...tooltipProps}
            >
              {tileContent}
            </button>
          )}
        </Tooltip>
      ) : (
        <div className="message-media-type-tile" data-file-type={fileType} aria-hidden="true">
          {tileContent}
        </div>
      )}
      {downloadState?.kind === "pending" && downloadProgressPercent !== null ? (
        <span
          className="sr-only"
          role="progressbar"
          aria-label={tileLabel}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={downloadProgressPercent}
        />
      ) : null}
      {uploadProgressPercentValue !== null ? (
        <div
          className="message-media-progress"
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={uploadProgressPercentValue}
        >
          <span style={{ width: `${uploadProgressPercentValue}%` }} />
        </div>
      ) : null}
    </div>
  );
}

function MediaDetailsPopup({ anchor, onClose, children }: {
  anchor: RefObject<HTMLButtonElement | null>; onClose: () => void; children: ReactNode;
}) {
  const placement = useFloatingPlacement({ anchorRef: anchor, placement: "below", align: "end", inlineSize: 260, blockSize: 180 });
  const popupRef = useRef<HTMLDivElement>(null);
  // #1368: the details panel renders in the body-level floating layer with no
  // backdrop, so the shared owner dismisses it on an outside press and keeps
  // one row's details open at a time.
  useOutsidePressDismissal({
    open: true,
    kind: "media-details",
    contentRef: popupRef,
    triggerRef: anchor,
    onDismiss: onClose
  });
  return <FloatingLayer><div className="message-media-details-popover" role="dialog"
    ref={popupRef}
    aria-label={t("timeline.mediaDetailsTitle")} style={floatingPlacementStyle(placement)}
    onKeyDown={event => {
      if (event.key === "Escape" && !event.nativeEvent.isComposing) {
        event.preventDefault(); event.stopPropagation(); onClose(); anchor.current?.focus();
      }
    }}>{children}</div></FloatingLayer>;
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
