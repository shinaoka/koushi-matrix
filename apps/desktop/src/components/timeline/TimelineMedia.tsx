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
import { mediaSourceUrl, readDownloadedMarkdown, readDownloadedText } from "../../backend/linkMediaRuntime";
import type { MediaSaveNameFacts } from "../../backend/linkMediaPort";
import type { TimelineTransport } from "./TimelineTransport";
import { type FileTypeCategory, fileTypeCategory } from "./fileTypeCategory";
import {
  TEXT_PREVIEW_LINE_COUNT,
  TEXT_PREVIEW_MAX_BYTES,
  filePreviewKind,
  isMarkdownAttachment,
  textPreviewLines
} from "./filePreview";
import { type MarkdownDocument, useMarkdownDocumentOpener } from "./MarkdownDocumentContext";
import { renderMarkdownDocument } from "./TimelineMessageBody";
import { MediaTextPreviewDialog } from "./MediaTextPreviewDialog";
import { formatDimensions } from "../../app/uiShared";
import { saveMediaSource, type MediaViewerEntry } from "../mediaViewer";

/** Top-level blocks of a document the card draws before the reader opens it. */
const DOCUMENT_PREVIEW_BLOCKS = 4;

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
  onReadMediaText,
  onReadMediaMarkdown,
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
  /** The timeline's per-account read, so a preview never reads another tab. */
  onReadMediaText?: TimelineTransport["readMediaText"];
  onReadMediaMarkdown?: TimelineTransport["readMediaMarkdown"];
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
  // #2096: what this attachment can preview, decided from the category the card
  // already computes. Text and audio/video read Core's media cache; every other
  // type keeps the plain card until a later stage (PDF needs a renderer). An
  // image decides its own branch from the media kind, so it never previews here.
  const fileType = fileTypeCategory(media.filename, media.mimetype, media.kind);
  const previewKind = media.kind === "Image" ? "none" : filePreviewKind(fileType);
  const textPreviewAllowed =
    previewKind === "text" &&
    (media.size === null || media.size <= TEXT_PREVIEW_MAX_BYTES);
  // #2097: a `.md` attachment is drawn as a document instead of raw text, when
  // the shell can show it in the right panel. Every other text type, and a `.md`
  // with nowhere to open, keeps the stage-1 plain-text preview.
  const openMarkdownDocument = useMarkdownDocumentOpener();
  const markdownAllowed =
    textPreviewAllowed &&
    openMarkdownDocument !== null &&
    isMarkdownAttachment(media.filename, media.mimetype);
  const plainTextPreviewAllowed = textPreviewAllowed && !markdownAllowed;
  const readyDownload = downloadState?.kind === "ready" ? downloadState : null;
  const readyDownloadRef = readyDownload?.source_ref ?? null;
  const [previewText, setPreviewText] = useState<
    { kind: "loading" } | { kind: "ready"; text: string } | { kind: "failed" } | null
  >(null);
  const [textDialogOpen, setTextDialogOpen] = useState(false);
  const [previewDocument, setPreviewDocument] = useState<
    { kind: "loading" } | { kind: "ready"; document: MarkdownDocument } | { kind: "failed" } | null
  >(null);
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
  // #2096: one read per downloaded text attachment, kept for the card and the
  // full-text dialog; the backend caps it, so a rejection is the "cannot
  // preview" answer rather than an error the reader can act on. A file above the
  // card's limit is not read at all: the reader asked to save it, and a refusal
  // would be reported as a failure it is not.
  const readText = onReadMediaText ?? readDownloadedText;
  const readMarkdown = onReadMediaMarkdown ?? readDownloadedMarkdown;
  useEffect(() => {
    if (!markdownAllowed || readyDownloadRef === null) {
      setPreviewDocument(null);
      return;
    }
    let cancelled = false;
    setPreviewDocument({ kind: "loading" });
    readMarkdown(readyDownloadRef).then(
      (formatted) => {
        if (!cancelled) {
          setPreviewDocument({
            kind: "ready",
            document: { filename: media.filename, formatted }
          });
        }
      },
      () => {
        if (!cancelled) {
          setPreviewDocument({ kind: "failed" });
        }
      }
    );
    return () => {
      cancelled = true;
    };
  }, [markdownAllowed, media.filename, readMarkdown, readyDownloadRef]);
  useEffect(() => {
    if (!plainTextPreviewAllowed || readyDownloadRef === null) {
      setPreviewText(null);
      return;
    }
    let cancelled = false;
    setPreviewText({ kind: "loading" });
    readText(readyDownloadRef).then(
      (text) => {
        if (!cancelled) {
          setPreviewText({ kind: "ready", text });
        }
      },
      () => {
        if (!cancelled) {
          setPreviewText({ kind: "failed" });
        }
      }
    );
    return () => {
      cancelled = true;
    };
  }, [readText, plainTextPreviewAllowed, readyDownloadRef]);
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
  const saveAction = (ready: Extract<TimelineMediaDownloadState, { kind: "ready" }>) => {
    void saveMediaSource(
      ready.source_ref,
      mediaSourceUrl(ready.source_ref, ready.mime_type ?? media.mimetype),
      media.filename,
      saveName,
      onSaveMediaFile
    );
  };
  // #2096: a player is its own target, so its card background does nothing
  // rather than saving by accident, and its tile's not-ready click only fetches:
  // the reader who wanted to hear the file is not asked where to write it. A
  // ready text preview opens what the card would otherwise save; the tile keeps
  // saving, and nothing happens while the text is still being read (a click
  // there must not turn into a Save dialog).
  const playsInline = previewKind === "audio" || previewKind === "video";
  const cardAction: (() => void) | null = !canDownload
    ? null
    : downloadState?.kind === "pending"
      ? null
      : downloadState?.kind === "ready"
        ? () => saveAction(downloadState)
        : playsInline
          ? onDownload
          : requestDownload;
  const textReady = plainTextPreviewAllowed && previewText?.kind === "ready";
  const readyDocument = markdownAllowed && previewDocument?.kind === "ready" ? previewDocument.document : null;
  const openDocument = readyDocument && openMarkdownDocument
    ? () => openMarkdownDocument(readyDocument)
    : null;
  const backgroundAction: (() => void) | null = playsInline
    ? null
    : markdownAllowed
      ? openDocument
      : plainTextPreviewAllowed
        ? textReady
          ? () => setTextDialogOpen(true)
          : null
        : cardAction;
  // A type the player cannot render has no protocol token, and the empty URL
  // keeps the plain card rather than mounting a <video> that can never load.
  const inlinePlayerUrl = playsInline && readyDownload !== null
    ? mediaSourceUrl(
        readyDownload.source_ref,
        readyDownload.mime_type ?? media.mimetype
      ) || null
    : null;
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
      data-actionable={backgroundAction ? "true" : undefined}
      onClick={
        backgroundAction
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
              backgroundAction();
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
        {textReady && previewText?.kind === "ready" ? (
          // The preview is its own control so a keyboard reaches the whole file,
          // and the card's background click (the same action) covers the rest.
          <button
            className="message-media-text-preview-button"
            type="button"
            aria-label={t("timeline.mediaTextPreviewOpen", { filename: media.filename })}
            onClick={() => setTextDialogOpen(true)}
          >
            <pre
              className="message-media-text-preview"
              dir="auto"
              style={{ "--media-preview-lines": TEXT_PREVIEW_LINE_COUNT } as CSSProperties}
            >
              {textPreviewLines(previewText.text).join("\n")}
            </pre>
          </button>
        ) : null}
        {readyDocument && openDocument ? (
          // #2097: the first blocks, rendered. Its copy buttons are hidden in the
          // card, so the preview is one control that opens the whole document.
          <div
            className="message-media-doc-preview"
            role="button"
            tabIndex={0}
            dir="auto"
            aria-label={t("timeline.mediaDocumentOpen", { filename: media.filename })}
            style={{ "--media-preview-lines": TEXT_PREVIEW_LINE_COUNT } as CSSProperties}
            onClick={openDocument}
            onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                openDocument();
              }
            }}
          >
            {renderMarkdownDocument(readyDocument.formatted, () => undefined, DOCUMENT_PREVIEW_BLOCKS)}
          </div>
        ) : null}
        {markdownAllowed && previewDocument?.kind === "failed" ? (
          <p className="message-media-preview-note">
            {t("timeline.mediaTextPreviewFailed")}
          </p>
        ) : null}
        {plainTextPreviewAllowed && previewText?.kind === "failed" ? (
          <p className="message-media-preview-note">
            {t("timeline.mediaTextPreviewFailed")}
          </p>
        ) : null}
        {inlinePlayerUrl && previewKind === "audio" ? (
          <audio
            className="message-media-audio-player"
            src={inlinePlayerUrl}
            controls
            preload="metadata"
            aria-label={t("timeline.mediaAudioPlayer", { filename: media.filename })}
          />
        ) : null}
        {inlinePlayerUrl && previewKind === "video" ? (
          <video
            className="message-media-video-player"
            src={inlinePlayerUrl}
            controls
            preload="metadata"
            aria-label={t("timeline.mediaVideoPlayer", { filename: media.filename })}
          />
        ) : null}
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
      {textDialogOpen && previewText?.kind === "ready" ? (
        <MediaTextPreviewDialog
          filename={media.filename}
          text={previewText.text}
          onClose={() => setTextDialogOpen(false)}
        />
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
