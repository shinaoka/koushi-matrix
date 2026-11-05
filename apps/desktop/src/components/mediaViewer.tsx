import {
  type PointerEvent as ReactPointerEvent,
  useCallback,
  useEffect,
  useRef,
  useState
} from "react";
import {
  ChevronLeft,
  ChevronRight,
  Download,
  FileCode2,
  FileText,
  Forward,
  Image as ImageIcon,
  MessageCircle,
  MoreHorizontal,
  Trash2,
  X,
  ZoomIn,
  ZoomOut
} from "lucide-react";

import { NativeModal } from "./ModalDialog";
import { Tooltip } from "./Tooltip";
import { onMenuKeyDown } from "./ContextMenuSurface";
import { t } from "../i18n/messages";
import { mediaSourceUrl } from "../backend/linkMediaRuntime";
import type { MediaSaveNameFacts } from "../backend/linkMediaPort";
import {
  createTrackedObjectURL,
  revokeTrackedObjectURL
} from "../domain/objectUrlDiagnostics";
import type { TimelineForwardDestination } from "../domain/projectionTypes";
import type {
  TimelineMediaDownloadState,
  TimelineMediaGalleryItem,
  TimelineMediaKind
} from "../domain/types";
import type { TimelineTransport } from "./timeline/TimelineTransport";
import { ICON_SIZE, formatDimensions, formatUploadBytes } from "../app/uiShared";

/**
 * The per-image actions the timeline supplies for one event (#1810). The
 * gallery mount passes none, so its footer keeps only the zoom and navigation
 * controls.
 */
export type MediaViewerActions = {
  canForward: boolean;
  forwardDestinations: readonly TimelineForwardDestination[];
  onForward: (destinationRoomId: string) => void;
  canViewSource: boolean;
  onViewSource: () => void;
  canRedact: boolean;
  onRedact: () => void;
};

/**
 * One image (or media) as the viewer displays it. Both mounts derive it from
 * their own Rust DTO: the timeline and the gallery from
 * `ui.timeline.media_gallery`, the quoted-media open from its reply quote. The
 * viewer never reorders or rescans anything, so the DTO order stays the only
 * navigation order (#1810).
 */
export type MediaViewerEntry = {
  eventId: string;
  filename: string;
  kind: TimelineMediaKind;
  size: number | null;
  width: number | null;
  height: number | null;
  mimetype: string | null;
  encrypted: boolean;
  saveName: MediaSaveNameFacts | null;
};

export function mediaViewerEntryFromGallery(item: TimelineMediaGalleryItem): MediaViewerEntry {
  return {
    eventId: item.event_id,
    filename: item.media.filename,
    kind: item.media.kind,
    size: item.media.size,
    width: item.media.width,
    height: item.media.height,
    mimetype: item.media.mimetype,
    encrypted: item.media.source.encrypted,
    saveName: {
      kind: item.media.kind === "Image" ? "image" : "file",
      timestampMs: item.timestamp_ms
    }
  };
}

/**
 * The image viewer's navigable set: the room media sequence restricted to
 * images, in gallery order (#1810).
 */
export function imageViewerEntriesFromGallery(
  items: readonly TimelineMediaGalleryItem[]
): MediaViewerEntry[] {
  return items.filter((item) => item.media.kind === "Image").map(mediaViewerEntryFromGallery);
}

/** The viewer entry for one timeline row, used by the single-item fallback. */
export function mediaViewerEntryFromMedia(
  eventId: string,
  media: {
    kind: TimelineMediaKind;
    filename: string;
    source: { encrypted: boolean };
    mimetype: string | null;
    size: number | null;
    width: number | null;
    height: number | null;
  },
  timestampMs: number | null
): MediaViewerEntry {
  return {
    eventId,
    filename: media.filename,
    kind: media.kind,
    size: media.size,
    width: media.width,
    height: media.height,
    mimetype: media.mimetype,
    encrypted: media.source.encrypted,
    saveName: { kind: media.kind === "Image" ? "image" : "file", timestampMs }
  };
}

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
        revokeUrl = createTrackedObjectURL(blob);
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
    window.setTimeout(() => revokeTrackedObjectURL(revokeUrl), 0);
  }
}

export async function saveMediaSource(
  sourceUrl: string,
  displayUrl: string,
  filename: string,
  saveName: MediaSaveNameFacts | null,
  saveMediaFile?: TimelineTransport["saveMediaFile"]
): Promise<void> {
  if (saveMediaFile) {
    await saveMediaFile(sourceUrl, filename, saveName);
    return;
  }
  await downloadMediaSource(displayUrl, filename);
}

type ViewerPan = { x: number; y: number };
const VIEWER_PAN_KEY_STEP = 24;
/** Arrow keys scroll the way a scroll container would: the pressed edge comes in. */
const VIEWER_PAN_KEYS: Record<string, ViewerPan> = {
  ArrowLeft: { x: 1, y: 0 },
  ArrowRight: { x: -1, y: 0 },
  ArrowUp: { x: 0, y: 1 },
  ArrowDown: { x: 0, y: -1 }
};
/**
 * A swipe navigates once it passes this much travel, on a pointer drag or an
 * accumulated horizontal wheel burst (#1810). It also scales with the stage so
 * a narrow stage stays reachable.
 */
const VIEWER_SWIPE_PX = 80;
const VIEWER_SWIPE_STAGE_RATIO = 0.25;
/** One trackpad swipe is one step: a burst ends after this much silence. */
const VIEWER_SWIPE_QUIET_MS = 250;

function swipeThreshold(stage: HTMLElement): number {
  return Math.min(VIEWER_SWIPE_PX, stage.clientWidth * VIEWER_SWIPE_STAGE_RATIO);
}

/** How far the scaled image overflows the stage on each axis, if at all. */
function panLimits(
  image: HTMLElement,
  stage: HTMLElement,
  zoomValue: number
): ViewerPan | "fit" {
  // offsetWidth/offsetHeight are the fitted layout box; transforms leave them alone.
  const x = (image.offsetWidth * zoomValue - stage.clientWidth) / 2;
  const y = (image.offsetHeight * zoomValue - stage.clientHeight) / 2;
  return x > 0 || y > 0 ? { x: Math.max(0, x), y: Math.max(0, y) } : "fit";
}

/**
 * The one image viewer (#1810). The timeline and the room media gallery both
 * mount it: the timeline resolves the opened event against the room media
 * sequence and supplies per-event actions, the gallery supplies its own index
 * and no actions.
 */
export function MediaViewer({
  index,
  items,
  mediaDownloads,
  onClose,
  onSelectIndex,
  onRequestDownload,
  actionsFor,
  onSaveMediaFile
}: {
  index: number;
  items: MediaViewerEntry[];
  mediaDownloads: Record<string, TimelineMediaDownloadState>;
  onClose: () => void;
  onSelectIndex: (index: number) => void;
  /** Asks Rust for a download the viewer needs; absent where none may start. */
  onRequestDownload?: (eventId: string) => void;
  actionsFor?: (eventId: string) => MediaViewerActions | null;
  onSaveMediaFile?: TimelineTransport["saveMediaFile"];
}) {
  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState<ViewerPan>({ x: 0, y: 0 });
  const [isPanning, setPanning] = useState(false);
  const [swipeX, setSwipeX] = useState(0);
  const [isActionMenuOpen, setActionMenuOpen] = useState(false);
  const [isForwardMenuOpen, setForwardMenuOpen] = useState(false);
  const stageRef = useRef<HTMLDivElement | null>(null);
  const imageRef = useRef<HTMLImageElement | null>(null);
  const closeButtonRef = useRef<HTMLButtonElement | null>(null);
  const actionMenuControlRef = useRef<HTMLDivElement>(null);
  const firstActionMenuItemRef = useRef<HTMLButtonElement>(null);
  const dragRef = useRef<{
    pointerId: number;
    x: number;
    y: number;
    pan: ViewerPan;
    /** Where the drag starts: a fitted image swipes, a zoomed one pans. */
    origin: "fitted" | "leading" | "trailing" | "free";
  } | null>(null);
  const wheelTravelRef = useRef(0);
  const wheelLastAtRef = useRef(0);
  const wheelBurstNavigatedRef = useRef(false);
  /** A `failed` download is retried once per viewer session, not on every render. */
  const retriedDownloadsRef = useRef(new Set<string>());
  // #163: a viewer opened for one image hides prev/next.
  const showNavigation = items.length > 1;
  const safeIndex = Math.min(Math.max(index, 0), Math.max(items.length - 1, 0));
  const item = items[safeIndex];
  const label = item ? item.filename || item.eventId : "";
  // #1784/#1792: the size line carries pixel dimensions for images and videos
  // only; the format is the timeline viewer's (`size · widthxheight`).
  const dimensions =
    item && (item.kind === "Image" || item.kind === "Video")
      ? formatDimensions(item.width, item.height)
      : null;
  const metadata = [
    item && item.size !== null ? formatUploadBytes(item.size) : null,
    dimensions
  ].filter((value): value is string => Boolean(value));
  const download = item ? mediaDownloads[item.eventId] : undefined;
  // The protocol mints one URL per ready download, whatever the kind.
  const readyUrl =
    download?.kind === "ready"
      ? mediaSourceUrl(download.source_ref, download.mime_type ?? item?.mimetype ?? null)
      : null;
  const sourceUrl = item && item.kind === "Image" ? readyUrl : null;
  const actions = item ? (actionsFor?.(item.eventId) ?? null) : null;
  const timelineFooter = actionsFor !== undefined;
  const canForward = Boolean(actions?.canForward && actions.forwardDestinations.length > 0);
  const hasActionMenu = Boolean(
    actions && (canForward || actions.canViewSource || actions.canRedact)
  );

  const goTo = useCallback(
    (target: number) => {
      if (target < 0 || target >= items.length || target === index) {
        return;
      }
      onSelectIndex(target);
    },
    [index, items.length, onSelectIndex]
  );
  const goPrevious = useCallback(() => goTo(index - 1), [goTo, index]);
  const goNext = useCallback(() => goTo(index + 1), [goTo, index]);

  /** Keeps the zoomed image inside the stage: it may only move by its overflow. */
  const clampPan = useCallback((next: ViewerPan, zoomValue: number): ViewerPan => {
    const stage = stageRef.current;
    const image = imageRef.current;
    const limits = stage && image ? panLimits(image, stage, zoomValue) : "fit";
    if (limits === "fit") {
      return { x: 0, y: 0 };
    }
    return {
      x: Math.min(limits.x, Math.max(-limits.x, next.x)),
      y: Math.min(limits.y, Math.max(-limits.y, next.y))
    };
  }, []);

  // A fitted image cannot pan, so zooming back in from a fitted view starts at
  // the centre again instead of keeping a stale offset.
  const changeZoom = (delta: number) => {
    const next = Math.min(3, Math.max(0.5, zoom + delta));
    setZoom(next);
    setPan((current) => clampPan(current, next));
  };

  const closeActionMenu = useCallback(() => {
    setActionMenuOpen(false);
    setForwardMenuOpen(false);
  }, []);

  useEffect(() => {
    closeButtonRef.current?.focus();
  }, []);

  // Switching images starts from the fitted view again (#1810), and a drag
  // still in flight must not move or navigate the replacement image.
  useEffect(() => {
    setZoom(1);
    setPan({ x: 0, y: 0 });
    setSwipeX(0);
    dragRef.current = null;
    setPanning(false);
    wheelTravelRef.current = 0;
    closeActionMenu();
  }, [item?.eventId, closeActionMenu]);

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

  // The viewer asks for whatever it shows and prefetches the immediate
  // neighbours, so a swipe does not land on a placeholder (#1810).
  useEffect(() => {
    if (!onRequestDownload || !item) {
      return;
    }
    const request = (eventId: string) => {
      const state = mediaDownloads[eventId];
      if (!state || state.kind === "notRequested") {
        onRequestDownload(eventId);
        return;
      }
      if (state.kind === "failed" && !retriedDownloadsRef.current.has(eventId)) {
        // #1810: a neighbour that failed is retried once per viewer session, so
        // a permanently failing download cannot flap on every state update.
        retriedDownloadsRef.current.add(eventId);
        onRequestDownload(eventId);
      }
    };
    if (download?.kind !== "ready") {
      // #1810: the viewer fetches the image it shows; files and videos keep the
      // gallery's placeholder instead of starting a download.
      if (item.kind === "Image") {
        request(item.eventId);
      }
      return;
    }
    for (const neighbour of [items[safeIndex - 1], items[safeIndex + 1]]) {
      if (neighbour?.kind === "Image") {
        request(neighbour.eventId);
      }
    }
  }, [download?.kind, item, items, mediaDownloads, onRequestDownload, safeIndex]);

  // React registers wheel listeners as passive, so the pan and the swipe-away
  // navigation own their own non-passive one; scrolling stays the dialog's
  // while the image fits and the gesture is not horizontal.
  useEffect(() => {
    const stage = stageRef.current;
    if (!stage) {
      return;
    }
    const handleWheel = (event: WheelEvent) => {
      const image = imageRef.current;
      if (event.deltaX === 0 && event.deltaY === 0) {
        return;
      }
      if (image && panLimits(image, stage, zoom) !== "fit") {
        event.preventDefault();
        setPan((current) =>
          clampPan({ x: current.x - event.deltaX, y: current.y - event.deltaY }, zoom)
        );
        return;
      }
      if (!showNavigation || Math.abs(event.deltaX) <= Math.abs(event.deltaY)) {
        return;
      }
      event.preventDefault();
      const now = Date.now();
      if (now - wheelLastAtRef.current > VIEWER_SWIPE_QUIET_MS) {
        wheelTravelRef.current = 0;
        wheelBurstNavigatedRef.current = false;
      }
      wheelLastAtRef.current = now;
      if (wheelBurstNavigatedRef.current) {
        return;
      }
      wheelTravelRef.current += event.deltaX;
      if (Math.abs(wheelTravelRef.current) < swipeThreshold(stage)) {
        return;
      }
      const forwards = wheelTravelRef.current < 0;
      wheelTravelRef.current = 0;
      wheelBurstNavigatedRef.current = true;
      if (forwards) {
        goNext();
      } else {
        goPrevious();
      }
    };
    stage.addEventListener("wheel", handleWheel, { passive: false });
    return () => {
      stage.removeEventListener("wheel", handleWheel);
    };
  }, [clampPan, goNext, goPrevious, showNavigation, zoom]);

  const clearDrag = (event: ReactPointerEvent<HTMLDivElement>) => {
    dragRef.current = null;
    setPanning(false);
    setSwipeX(0);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  const endPan = (event: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (drag?.pointerId !== event.pointerId) {
      return;
    }
    const stage = stageRef.current;
    const travelled = event.clientX - drag.x;
    clearDrag(event);
    if (!showNavigation || !stage || Math.abs(travelled) < swipeThreshold(stage)) {
      return;
    }
    // A fitted image swipes. A zoomed one pans (#1746) and only continues into
    // the neighbour once it is already pinned at that horizontal edge (#1810).
    const forwards = travelled < 0;
    if (drag.origin === "fitted" || (forwards ? drag.origin === "trailing" : drag.origin === "leading")) {
      if (forwards) {
        goNext();
      } else {
        goPrevious();
      }
    }
  };

  if (!item) {
    return null;
  }

  return (
    <NativeModal
      onDismiss={() => {
        if (isActionMenuOpen) {
          closeActionMenu();
        } else {
          onClose();
        }
      }}
      tabIndex={-1}
      className="media-viewer-backdrop"
      role="dialog"
      aria-modal="true"
      aria-label={t("mediaGallery.viewerTitle")}
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) {
          onClose();
        }
      }}
    >
      <div className="media-viewer" onMouseDown={(event) => event.stopPropagation()}>
        <header className="media-viewer-header">
          <div>
            <h2 dir="auto">{label}</h2>
            {/* #1784: size (and dimensions); the raw MIME type is not shown. */}
            {metadata.length > 0 || item.encrypted ? (
              <p>
                {[...metadata, ...(item.encrypted ? [t("timeline.encryptedMedia")] : [])].join(
                  " · "
                )}
              </p>
            ) : null}
          </div>
          <div className="media-viewer-header-end">
            {showNavigation ? (
              // #1810: `3/12`, announced when it changes.
              <span className="media-viewer-counter" aria-live="polite">
                {t("mediaGallery.position", {
                  index: String(safeIndex + 1),
                  total: String(items.length)
                })}
              </span>
            ) : null}
            <Tooltip label={t("mediaGallery.close")} placement="below" align="end">
              {(tooltipProps) => (
                <button
                  ref={closeButtonRef}
                  className="icon-button"
                  type="button"
                  aria-label={t("mediaGallery.close")}
                  onClick={onClose}
                  {...tooltipProps}
                >
                  <X size={ICON_SIZE.small} />
                </button>
              )}
            </Tooltip>
          </div>
        </header>
        <div
          ref={stageRef}
          className={`media-viewer-stage${zoom > 1 ? " is-pannable" : ""}${isPanning ? " is-panning" : ""}`}
          tabIndex={0}
          onPointerDown={(event) => {
            if (event.button !== 0) {
              return;
            }
            // A video's own controls must keep the pointer (#1810).
            if (event.target instanceof HTMLElement && event.target.closest("video")) {
              return;
            }
            // A stale burst from an earlier swipe must not join this gesture.
            wheelTravelRef.current = 0;
            const image = imageRef.current;
            const limits = image
              ? panLimits(image, event.currentTarget, zoom)
              : ("fit" as const);
            dragRef.current = {
              pointerId: event.pointerId,
              x: event.clientX,
              y: event.clientY,
              pan,
              origin:
                limits === "fit"
                  ? "fitted"
                  : pan.x >= limits.x - 0.5
                    ? "leading"
                    : pan.x <= -limits.x + 0.5
                      ? "trailing"
                      : "free"
            };
            event.currentTarget.setPointerCapture(event.pointerId);
            setPanning(true);
          }}
          onPointerMove={(event) => {
            const drag = dragRef.current;
            if (!drag || drag.pointerId !== event.pointerId) {
              return;
            }
            const image = imageRef.current;
            if (!image) {
              return;
            }
            const dx = event.clientX - drag.x;
            if (drag.origin === "fitted") {
              // A fitted image is dragged aside to reveal the neighbour (#1810).
              if (showNavigation) {
                const travel = event.currentTarget.clientWidth;
                setSwipeX(Math.max(-travel, Math.min(travel, dx)));
              }
              return;
            }
            setPan(
              clampPan(
                { x: drag.pan.x + dx, y: drag.pan.y + (event.clientY - drag.y) },
                zoom
              )
            );
          }}
          onPointerUp={endPan}
          onPointerCancel={clearDrag}
          onKeyDown={(event) => {
            if (event.altKey || event.ctrlKey || event.metaKey) {
              return;
            }
            if (showNavigation && (event.key === "Home" || event.key === "End")) {
              event.preventDefault();
              goTo(event.key === "Home" ? 0 : items.length - 1);
              return;
            }
            const direction = VIEWER_PAN_KEYS[event.key];
            if (!direction) {
              return;
            }
            const step = VIEWER_PAN_KEY_STEP * (event.shiftKey ? 4 : 1);
            const next = clampPan(
              { x: pan.x + direction.x * step, y: pan.y + direction.y * step },
              zoom
            );
            // A fitted image has nothing to reveal, so the key keeps its default.
            if (next.x === pan.x && next.y === pan.y) {
              // #1810: an arrow that cannot pan navigates instead.
              if (event.key === "ArrowLeft" && showNavigation) {
                event.preventDefault();
                goPrevious();
              } else if (event.key === "ArrowRight" && showNavigation) {
                event.preventDefault();
                goNext();
              }
              return;
            }
            event.preventDefault();
            setPan(next);
          }}
        >
          {item.kind === "Video" && readyUrl ? (
            <video
              className="media-viewer-video"
              src={readyUrl}
              aria-label={label}
              controls
              playsInline
            />
          ) : sourceUrl ? (
            <img
              ref={imageRef}
              className="media-viewer-image"
              src={sourceUrl}
              alt={label}
              draggable={false}
              style={{
                transform: `translate(${pan.x + swipeX}px, ${pan.y}px) scale(${zoom})`
              }}
            />
          ) : item.kind === "Image" ? (
            <div className="media-viewer-image-placeholder" style={{ transform: `scale(${zoom})` }}>
              <ImageIcon size={ICON_SIZE.emptyState} aria-hidden="true" />
              {/* #1810: the viewer asks for the image it shows, so a not-ready
                  one is on its way here. The header carries the filename. */}
              <span>
                {download?.kind === "failed"
                  ? t("timeline.mediaDownloadFailed")
                  : t("timeline.mediaDownloadPending")}
              </span>
            </div>
          ) : (
            <div className="media-viewer-file-placeholder">
              <FileText size={ICON_SIZE.emptyState} aria-hidden="true" />
            </div>
          )}
        </div>
        <footer className="media-viewer-actions">
          {showNavigation ? (
            <Tooltip label={t("mediaGallery.previous")} placement="above">
              {(tooltipProps) => (
                <button
                  className="icon-button"
                  type="button"
                  aria-label={t("mediaGallery.previous")}
                  disabled={safeIndex <= 0}
                  onClick={goPrevious}
                  {...tooltipProps}
                >
                  <ChevronLeft size={ICON_SIZE.control} />
                </button>
              )}
            </Tooltip>
          ) : null}
          <Tooltip label={t("mediaGallery.zoomOut")} placement="above">
            {(tooltipProps) => (
              <button
                className="icon-button"
                type="button"
                aria-label={t("mediaGallery.zoomOut")}
                onClick={() => changeZoom(-0.25)}
                {...tooltipProps}
              >
                <ZoomOut size={ICON_SIZE.control} />
              </button>
            )}
          </Tooltip>
          <Tooltip label={t("mediaGallery.zoomIn")} placement="above">
            {(tooltipProps) => (
              <button
                className="icon-button"
                type="button"
                aria-label={t("mediaGallery.zoomIn")}
                onClick={() => changeZoom(0.25)}
                {...tooltipProps}
              >
                <ZoomIn size={ICON_SIZE.control} />
              </button>
            )}
          </Tooltip>
          {showNavigation ? (
            <Tooltip label={t("mediaGallery.next")} placement="above">
              {(tooltipProps) => (
                <button
                  className="icon-button"
                  type="button"
                  aria-label={t("mediaGallery.next")}
                  disabled={safeIndex >= items.length - 1}
                  onClick={goNext}
                  {...tooltipProps}
                >
                  <ChevronRight size={ICON_SIZE.control} />
                </button>
              )}
            </Tooltip>
          ) : null}
          {timelineFooter ? (
            <Tooltip label={t("timeline.downloadMedia", { filename: label })} placement="above">
              {(tooltipProps) => (
                <button
                  className="icon-button"
                  type="button"
                  aria-label={t("timeline.downloadMedia", { filename: label })}
                  disabled={download?.kind !== "ready"}
                  onClick={() => {
                    if (download?.kind !== "ready") {
                      return;
                    }
                    void saveMediaSource(
                      download.source_ref,
                      mediaSourceUrl(download.source_ref, download.mime_type ?? item.mimetype),
                      item.filename,
                      item.saveName,
                      onSaveMediaFile
                    );
                  }}
                  {...tooltipProps}
                >
                  <Download size={ICON_SIZE.control} />
                </button>
              )}
            </Tooltip>
          ) : null}
          {hasActionMenu ? (
            <div className="media-viewer-menu-control" ref={actionMenuControlRef}>
              <Tooltip label={t("timeline.messageActions")} placement="above" align="end">
                {(tooltipProps) => (
                  <button
                    className="icon-button"
                    type="button"
                    aria-label={t("timeline.messageActions")}
                    aria-expanded={isActionMenuOpen}
                    aria-haspopup="menu"
                    onClick={() => {
                      setForwardMenuOpen(false);
                      setActionMenuOpen((current) => !current);
                    }}
                    {...tooltipProps}
                  >
                    <MoreHorizontal size={ICON_SIZE.control} />
                  </button>
                )}
              </Tooltip>
              {isActionMenuOpen && actions ? (
                <div
                  className="media-viewer-menu"
                  role="menu"
                  aria-label={t("timeline.messageActions")}
                  onKeyDown={(event) => {
                    if (
                      event.key === "Escape" &&
                      !event.nativeEvent.isComposing &&
                      event.keyCode !== 229
                    ) {
                      event.preventDefault();
                      closeActionMenu();
                      return;
                    }
                    onMenuKeyDown(event, event.currentTarget);
                  }}
                >
                  {canForward ? (
                    <div className="media-viewer-forward-control">
                      <button
                        ref={firstActionMenuItemRef}
                        className="media-viewer-menu-item"
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
                        <div className="media-viewer-forward-menu" role="menu">
                          {actions.forwardDestinations.map((destination) => (
                            <button
                              className="media-viewer-menu-item"
                              type="button"
                              role="menuitem"
                              key={destination.room_id}
                              onClick={() => {
                                actions.onForward(destination.room_id);
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
                  {actions.canViewSource ? (
                    <button
                      ref={!canForward ? firstActionMenuItemRef : undefined}
                      className="media-viewer-menu-item"
                      type="button"
                      role="menuitem"
                      onClick={() => {
                        actions.onViewSource();
                        onClose();
                      }}
                    >
                      <FileCode2 size={17} aria-hidden="true" />
                      <span>{t("timeline.viewSource")}</span>
                    </button>
                  ) : null}
                  {actions.canRedact ? (
                    <button
                      ref={!canForward && !actions.canViewSource ? firstActionMenuItemRef : undefined}
                      className="media-viewer-menu-item is-destructive"
                      type="button"
                      role="menuitem"
                      onClick={() => {
                        actions.onRedact();
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
        </footer>
      </div>
    </NativeModal>
  );
}
