import {
  type FormEvent,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useRef,
  useState
} from "react";
import { NativeModal } from "./ModalDialog";
import {
  ChevronLeft,
  ChevronRight,
  Clock3,
  Edit3,
  FileText,
  Image as ImageIcon,
  Paperclip,
  Pin,
  PinOff,
  X,
  ZoomIn,
  ZoomOut
} from "lucide-react";
import { t } from "../i18n/messages";
import type {
  DesktopSnapshot,
  ScheduledSendCapability,
  ScheduledSendItem,
  SearchResult,
  TimelineMediaDownloadState,
  TimelineMediaGalleryItem,
  TimelineMessage,
  TextRange,
  UserProfile
} from "../domain/types";
import { contextMenuItems } from "../domain/contextMenus";
import {
  mediaSourceUrl,
  renderableThumbnailSourceUrl
} from "../backend/linkMediaRuntime";
import {
  renderTimelineMessageText,
  type TimelineRowActionHandlers
} from "./TimelineView";
import { ImeSafeForm } from "./ImeTextControl";
import { Composer, ScheduledSendTimeAdjustments } from "./composer";
import { Tooltip } from "./Tooltip";
import { documentFromText, plainBodyFromDocument } from "../domain/composerDocument";
import {
  ICON_SIZE,
  formatUploadBytes,
  mediaGalleryItemLabel,
  formatTime,
  formatScheduledSendTime,
  scheduledSendCapabilityLabel,
  datetimeLocalValueFromTimestamp,
  scheduledSendTimestampFromInput,
  avatarInitial,
  roomDisplayLabel,
  peopleFacingLabel,
  type OpenContextMenu
} from "../app/uiShared";

function RoomMediaGallery({
  items,
  mediaDownloads,
  onOpenItem
}: {
  items: TimelineMediaGalleryItem[];
  mediaDownloads: Record<string, TimelineMediaDownloadState>;
  onOpenItem: (index: number) => void;
}) {
  if (items.length === 0) {
    return (
      <section className="room-media-gallery room-media-gallery-empty" role="region" aria-label={t("mediaGallery.region")}>
        <div className="room-media-gallery-empty-state">
          <ImageIcon size={ICON_SIZE.control} aria-hidden="true" />
          <span>{t("mediaGallery.empty")}</span>
        </div>
      </section>
    );
  }

  return (
    <section className="room-media-gallery" role="region" aria-label={t("mediaGallery.region")}>
      {items.map((item, index) => {
        const label = mediaGalleryItemLabel(item);
        const download = mediaDownloads[item.event_id];
        const previewUrl =
          item.media.kind === "Image" && download?.kind === "ready"
            ? mediaSourceUrl(
                download.source_ref,
                download.mime_type ?? item.media.mimetype
              )
            : null;
        return (
          <button
            className="room-media-gallery-item"
            key={item.event_id}
            type="button"
            aria-label={t("mediaGallery.openItem", { filename: label })}
            onClick={() => onOpenItem(index)}
          >
            {previewUrl ? (
              <img
                className="room-media-gallery-preview"
                src={previewUrl}
                alt={label}
                loading="lazy"
              />
            ) : item.media.kind === "Image" ? (
              <ImageIcon size={ICON_SIZE.control} aria-hidden="true" />
            ) : (
              <FileText size={ICON_SIZE.control} aria-hidden="true" />
            )}
            <span className="room-media-gallery-name" dir="auto">
              {label}
            </span>
            <span className="room-media-gallery-meta">
              {item.media.size !== null ? formatUploadBytes(item.media.size) : item.media.kind}
              {item.media.source.encrypted ? ` - ${t("mediaGallery.encrypted")}` : ""}
              {download?.kind === "pending" ? ` - ${t("mediaGallery.noPreview")}` : ""}
            </span>
          </button>
        );
      })}
    </section>
  );
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

function MediaViewer({
  index,
  items,
  mediaDownloads,
  onClose,
  onSelectIndex
}: {
  index: number;
  items: TimelineMediaGalleryItem[];
  mediaDownloads: Record<string, TimelineMediaDownloadState>;
  onClose: () => void;
  onSelectIndex: (index: number) => void;
}) {
  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState<ViewerPan>({ x: 0, y: 0 });
  const [isPanning, setPanning] = useState(false);
  const stageRef = useRef<HTMLDivElement | null>(null);
  const imageRef = useRef<HTMLImageElement | null>(null);
  const dragRef = useRef<{ pointerId: number; x: number; y: number; pan: ViewerPan } | null>(
    null
  );
  // #163: a viewer opened from a single timeline image hides prev/next.
  const showNavigation = items.length > 1;
  const item = items[index];
  const previousIndex = (index + items.length - 1) % items.length;
  const nextIndex = (index + 1) % items.length;
  const label = mediaGalleryItemLabel(item);
  const download = mediaDownloads[item.event_id];
  const sourceUrl =
    item.media.kind === "Image" && download?.kind === "ready"
      ? mediaSourceUrl(
          download.source_ref,
          download.mime_type ?? item.media.mimetype
        )
      : null;

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

  const resetView = () => {
    setZoom(1);
    setPan({ x: 0, y: 0 });
  };

  const endPan = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (dragRef.current?.pointerId !== event.pointerId) {
      return;
    }
    dragRef.current = null;
    setPanning(false);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  // React registers wheel listeners as passive, so the pan owns its own
  // non-passive one; scrolling stays the dialog's while the image fits.
  useEffect(() => {
    const stage = stageRef.current;
    if (!stage) {
      return;
    }
    const handleWheel = (event: WheelEvent) => {
      const image = imageRef.current;
      if (!image || (event.deltaX === 0 && event.deltaY === 0)) {
        return;
      }
      if (panLimits(image, stage, zoom) === "fit") {
        return;
      }
      event.preventDefault();
      setPan((current) =>
        clampPan({ x: current.x - event.deltaX, y: current.y - event.deltaY }, zoom)
      );
    };
    stage.addEventListener("wheel", handleWheel, { passive: false });
    return () => {
      stage.removeEventListener("wheel", handleWheel);
    };
  }, [clampPan, zoom]);

  return (
    <NativeModal onDismiss={onClose}
      tabIndex={-1}
      className="media-viewer-backdrop"
      role="dialog"
      aria-modal="true"
      aria-label={t("mediaGallery.viewerTitle")}
    >
      <div className="media-viewer">
        <header className="media-viewer-header">
          <div>
            <h2 dir="auto">{label}</h2>
            {/* #1784: size only; the raw MIME type is not shown. */}
            {item.media.size !== null ? <p>{formatUploadBytes(item.media.size)}</p> : null}
          </div>
          <Tooltip label={t("mediaGallery.close")} placement="below" align="end">
            {(tooltipProps) => (
              <button
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
        </header>
        <div
          ref={stageRef}
          className={`media-viewer-stage${zoom > 1 ? " is-pannable" : ""}${isPanning ? " is-panning" : ""}`}
          tabIndex={0}
          onPointerDown={(event) => {
            if (event.button !== 0 || !sourceUrl) {
              return;
            }
            dragRef.current = {
              pointerId: event.pointerId,
              x: event.clientX,
              y: event.clientY,
              pan
            };
            event.currentTarget.setPointerCapture(event.pointerId);
            setPanning(true);
          }}
          onPointerMove={(event) => {
            const drag = dragRef.current;
            if (!drag || drag.pointerId !== event.pointerId) {
              return;
            }
            setPan(
              clampPan(
                {
                  x: drag.pan.x + (event.clientX - drag.x),
                  y: drag.pan.y + (event.clientY - drag.y)
                },
                zoom
              )
            );
          }}
          onPointerUp={endPan}
          onPointerCancel={endPan}
          onKeyDown={(event) => {
            const direction = VIEWER_PAN_KEYS[event.key];
            if (!direction || event.altKey || event.ctrlKey || event.metaKey) {
              return;
            }
            const step = VIEWER_PAN_KEY_STEP * (event.shiftKey ? 4 : 1);
            const next = clampPan(
              { x: pan.x + direction.x * step, y: pan.y + direction.y * step },
              zoom
            );
            // A fitted image has nothing to reveal, so the key keeps its default.
            if (next.x === pan.x && next.y === pan.y) {
              return;
            }
            event.preventDefault();
            setPan(next);
          }}
        >
          {sourceUrl ? (
            <img
              ref={imageRef}
              className="media-viewer-image"
              src={sourceUrl}
              alt={label}
              draggable={false}
              style={{
                transform: `translate(${pan.x}px, ${pan.y}px) scale(${zoom})`
              }}
            />
          ) : item.media.kind === "Image" ? (
            <div className="media-viewer-image-placeholder" style={{ transform: `scale(${zoom})` }}>
              <ImageIcon size={ICON_SIZE.emptyState} aria-hidden="true" />
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
                  onClick={() => {
                    resetView();
                    onSelectIndex(previousIndex);
                  }}
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
                  onClick={() => {
                    setZoom(1);
                    onSelectIndex(nextIndex);
                  }}
                  {...tooltipProps}
                >
                  <ChevronRight size={ICON_SIZE.control} />
                </button>
              )}
            </Tooltip>
          ) : null}
        </footer>
      </div>
    </NativeModal>
  );
}

function ScheduledMessagesList({
  capability,
  items,
  onCancel,
  onReschedule,
  mathModeEnabled = true,
  onMathModeChange = () => undefined,
  renderItemMeta
}: {
  capability: ScheduledSendCapability;
  items: ScheduledSendItem[];
  onCancel: (scheduledId: string) => void;
  onReschedule: (scheduledId: string, body: string, sendAtMs: number) => void;
  /**
   * Optional per-item context (destination room label, thread-reply flag). The
   * selected-room pane omits it; the scoped right panel supplies it.
   */
  renderItemMeta?: (item: ScheduledSendItem) => ReactNode;
  mathModeEnabled?: boolean;
  onMathModeChange?: (enabled: boolean) => void | Promise<void>;
}) {
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editBody, setEditBody] = useState("");
  const [editValue, setEditValue] = useState("");

  if (items.length === 0) {
    return null;
  }

  function openEdit(item: ScheduledSendItem) {
    setEditingId(item.scheduled_id);
    setEditBody(item.body);
    setEditValue(datetimeLocalValueFromTimestamp(item.send_at_ms));
  }

  function submitEdit(event: FormEvent<HTMLFormElement>, item: ScheduledSendItem) {
    event.preventDefault();
    const sendAtMs = scheduledSendTimestampFromInput(editValue);
    if (sendAtMs === null) {
      return;
    }
    if (!editBody.trim()) {
      return;
    }
    onReschedule(item.scheduled_id, editBody, sendAtMs);
    setEditingId(null);
  }

  return (
    <section className="scheduled-messages" aria-label={t("scheduled.title")}>
      <div className="scheduled-messages-heading">
        <span>
          <Clock3 size={ICON_SIZE.compact} aria-hidden="true" />
          <strong>{t("scheduled.title")}</strong>
        </span>
        <span className="scheduled-messages-capability">
          {scheduledSendCapabilityLabel(capability)}
        </span>
      </div>
      {capability === "localFallback" ? (
        <p className="scheduled-messages-note">{t("scheduled.localFallbackNotice")}</p>
      ) : null}
      <ul className="scheduled-message-list">
        {items.map((item) => {
          const isEditing = editingId === item.scheduled_id;
          return (
            <li className="scheduled-message-item" key={item.scheduled_id}>
              <div className="scheduled-message-main">
                <span className="scheduled-message-time">
                  {formatScheduledSendTime(item.send_at_ms)}
                </span>
                <span className="scheduled-message-body" dir="auto">
                  {item.body}
                </span>
                {renderItemMeta ? renderItemMeta(item) : null}
              </div>
              {isEditing ? (
                <ImeSafeForm
                  className="scheduled-message-edit"
                  onSubmit={(event) => submitEdit(event, item)}
                >
                  <Composer
                    editorOnly
                    ariaLabel={t("scheduled.bodyInput")}
                    composerMode={{ kind: "plain" }}
                    draftKey={`scheduled:${item.scheduled_id}`}
                    isSending={false}
                    mathModeEnabled={mathModeEnabled}
                    onMathModeChange={onMathModeChange}
                    document={documentFromText(editBody)}
                    roomName={t("scheduled.title")}
                    onCancelReply={() => undefined}
                    onDocumentChange={(document) => setEditBody(plainBodyFromDocument(document))}
                    onSend={() => undefined}
                  />
                  <label className="scheduled-send-field">
                    <span>{t("scheduled.timeInput")}</span>
                    <input
                      aria-label={t("scheduled.timeInput")}
                      type="datetime-local"
                      value={editValue}
                      onChange={(event) => setEditValue(event.currentTarget.value)}
                    />
                  </label>
                  <ScheduledSendTimeAdjustments value={editValue} onChange={setEditValue} />
                  <div className="scheduled-message-actions">
                    <button
                      className="timeline-send-bar-action"
                      type="button"
                      onClick={() => setEditingId(null)}
                    >
                      {t("action.cancel")}
                    </button>
                    <button
                      className="timeline-send-bar-action"
                      type="submit"
                      aria-label={t("scheduled.save")}
                      disabled={
                        scheduledSendTimestampFromInput(editValue) === null || !editBody.trim()
                      }
                    >
                      {t("settings.propertySave")}
                    </button>
                  </div>
                </ImeSafeForm>
              ) : (
                <div className="scheduled-message-actions">
                  <button
                    className="timeline-send-bar-action"
                    type="button"
                    aria-label={t("scheduled.edit")}
                    onClick={() => openEdit(item)}
                  >
                    <Edit3 size={ICON_SIZE.micro} aria-hidden="true" />
                    <span>{t("context.editMessage")}</span>
                  </button>
                  <button
                    className="timeline-send-bar-action danger"
                    type="button"
                    aria-label={t("scheduled.cancel")}
                    onClick={() => onCancel(item.scheduled_id)}
                  >
                    <X size={ICON_SIZE.micro} aria-hidden="true" />
                    <span>{t("action.cancel")}</span>
                  </button>
                </div>
              )}
            </li>
          );
        })}
      </ul>
    </section>
  );
}

function PinnedEventsList({
  roomId,
  pinnedEvents,
  profileUsers = {},
  onOpen,
  onUnpin
}: {
  roomId: string;
  pinnedEvents: DesktopSnapshot["state"]["domain"]["room_interactions"][string]["pinned_events"];
  profileUsers?: Record<string, UserProfile>;
  onOpen?: (roomId: string, eventId: string, threadRootEventId: string | null) => void;
  onUnpin: (roomId: string, eventId: string) => void;
}) {
  return (
    <section className="pinned-events" aria-label={t("timeline.pinnedMessages")}>
      <div className="pinned-events-heading">
        <Pin size={ICON_SIZE.compact} aria-hidden="true" />
        <span>{t("timeline.pinnedMessages")}</span>
      </div>
      <div className="pinned-events-list">
        {pinnedEvents.map((event) => {
          const profile = event.sender ? profileUsers[event.sender] : undefined;
          const senderLabel = peopleFacingLabel(
            profile?.display_label,
            event.sender_label,
            event.sender
          );
          const avatarSource =
            profile?.avatar?.thumbnail.kind === "ready"
              ? renderableThumbnailSourceUrl(profile.avatar.thumbnail.source_ref)
              : null;
          return (
          <div className="pinned-event" key={event.event_id}>
            <button
              className="pinned-event-main pinned-event-open"
              type="button"
              aria-label={
                event.body_preview ??
                (event.state === "unableToDecrypt"
                  ? t("timeline.pinnedEventUnableToDecrypt")
                  : t("timeline.pinnedMessage"))
              }
              onClick={() =>
                onOpen?.(roomId, event.event_id, event.thread_root_event_id ?? null)
              }
            >
              <span className="pinned-event-avatar" aria-hidden="true">
                {avatarSource ? (
                  <img src={avatarSource} alt={undefined} />
                ) : (
                  avatarInitial(senderLabel)
                )}
              </span>
              <span className="pinned-event-details">
                <span className="pinned-event-sender" dir="auto">
                  {senderLabel}
                </span>
              <span className="pinned-event-body" dir="auto">
                {event.redacted
                  ? t("timeline.redactedMessage")
                  : event.state === "unableToDecrypt"
                    ? t("timeline.pinnedEventUnableToDecrypt")
                    : event.state === "unavailable"
                      ? t("timeline.pinnedEventUnavailable")
                  : event.body_preview ?? t("timeline.pinnedMessage")}
              </span>
              {event.timestamp_ms ? (
                <time className="pinned-event-time" dateTime={new Date(event.timestamp_ms).toISOString()}>
                  {formatScheduledSendTime(event.timestamp_ms)}
                </time>
              ) : null}
              </span>
            </button>
            <Tooltip label={t("timeline.unpinMessage")} placement="below" align="end">
              {(tooltipProps) => (
                <button
                  className="pinned-event-action"
                  type="button"
                  aria-label={t("timeline.unpinMessage")}
                  onClick={() => onUnpin(roomId, event.event_id)}
                  {...tooltipProps}
                >
                  <PinOff size={ICON_SIZE.micro} aria-hidden="true" />
                </button>
              )}
            </Tooltip>
          </div>
          );
        })}
      </div>
    </section>
  );
}

function PinnedMessagesEntry({
  count,
  onOpen
}: {
  count: number;
  onOpen: () => void;
}) {
  return (
    <button
      className="pinned-events-entry"
      type="button"
      aria-label={t("timeline.pinnedMessagesCount", { count })}
      onClick={onOpen}
    >
      <Pin size={ICON_SIZE.compact} aria-hidden="true" />
      <span>{t("timeline.pinnedMessagesCount", { count })}</span>
    </button>
  );
}

function SearchResults({
  indexingPending = false,
  pending = false,
  tooShortMinChars = null,
  query,
  results,
  rooms,
  onResultSelect
}: {
  indexingPending?: boolean;
  pending?: boolean;
  tooShortMinChars?: number | null;
  query: string;
  results: SearchResult[];
  rooms: DesktopSnapshot["state"]["domain"]["rooms"];
  onResultSelect: (roomId: string, eventId: string) => void;
}) {
  if (!query.trim()) {
    return null;
  }

  return (
    <section className="search-results" aria-busy={pending || undefined}>
      <div className="search-results-header">
        <span dir="auto">
            {pending
              ? t("search.searchingFor", { query })
              : tooShortMinChars !== null
                ? t("search.tooShort")
              : t(results.length === 1 ? "search.resultCountOne" : "search.resultCountMany", {
                  count: results.length,
                  query
                })}
        </span>
      </div>
      <div className="result-list">
        {!pending && results.length ? (
          results.map((result) => {
            const room = rooms.find((candidate) => candidate.room_id === result.room_id);
            return (
              <button
                className="result-button"
                key={`${result.room_id}:${result.event_id}`}
                type="button"
                onClick={() => onResultSelect(result.room_id, result.event_id)}
              >
                <span dir="auto">{highlight(result.snippet, result.highlights)}</span>
                <span className="result-meta">
                  <span dir="auto">{result.context_label ?? (room ? roomDisplayLabel(room) : null) ?? result.room_id}</span> ·{" "}
                  <time dateTime={new Date(result.timestamp_ms).toISOString()}>
                    {formatScheduledSendTime(result.timestamp_ms)}
                  </time>{" "}
                  · {matchFieldLabel(result.match_field)}
                </span>
              </button>
            );
          })
        ) : (
          <div className="empty-results">
            {t(
              pending
                ? "search.searching"
                : tooShortMinChars !== null
                  ? "search.tooShort"
                : indexingPending
                  ? "search.indexingPending"
                  : "search.noExactMatches"
            )}
          </div>
        )}
      </div>
    </section>
  );
}

function MessageArticle({
  currentUserId,
  message,
  highlights,
  onOpenContextMenu,
  onOpenThread,
  onRedactMessage,
  profileUsers,
  isIgnored
}: {
  currentUserId: string | null;
  message: TimelineMessage;
  highlights: TextRange[];
  onOpenContextMenu?: OpenContextMenu;
  onOpenThread: TimelineRowActionHandlers["onOpenThread"];
  onRedactMessage: (roomId: string, eventId: string) => void;
  profileUsers: Record<string, UserProfile>;
  isIgnored: boolean;
}) {
  const canManage = currentUserId === message.sender;
  const profile = profileUsers[message.sender];
  const senderDisplayLabel = peopleFacingLabel(
    profile?.display_label,
    profile?.display_name,
    profile?.original_display_label
  );

  return (
    <article
      className="message"
      data-event-id={message.event_id}
      onContextMenu={
        onOpenContextMenu
          ? (event) =>
              onOpenContextMenu(
                event,
                { kind: "message", message },
                contextMenuItems({
                  kind: "message",
                  canManage,
                  // This article renders `TimelineMessage`, which carries no
                  // Rust-owned `can_edit`, and it has no inline editing surface
                  // (#1100). Editing belongs to the timeline row.
                  canEdit: false,
                  canReply: false,
                  hasThread: true,
                  senderUserId: message.sender,
                  currentUserId: currentUserId ?? "",
                  roomId: message.room_id,
                  eventId: message.event_id,
                  isIgnored
                })
              )
          : undefined
      }
    >
      <div className="avatar" aria-hidden="true">
        {avatarInitial(senderDisplayLabel)}
      </div>
      <div className="message-main">
        <div className="message-heading">
          <span className="sender" dir="auto">{senderDisplayLabel}</span>
          <span className="time">{formatTime(message.timestamp_ms)}</span>
          {canManage ? (
            <span className="message-actions">
              <Tooltip label={t("timeline.redactMessage")} placement="below" align="end">
                {(tooltipProps) => (
                  <button
                    className="message-action"
                    type="button"
                    aria-label={t("timeline.redactMessage")}
                    onClick={() => onRedactMessage(message.room_id, message.event_id)}
                    {...tooltipProps}
                  >
                    <X size={ICON_SIZE.micro} />
                  </button>
                )}
              </Tooltip>
            </span>
          ) : null}
        </div>
        <div className="message-body" dir="auto">
          {renderTimelineMessageText(message.body, highlights, profileUsers)}
        </div>
        {message.attachment_filename ? (
          <div className="attachment">
            <Paperclip size={ICON_SIZE.small} />
            <span dir="auto">{message.attachment_filename}</span>
          </div>
        ) : null}
        {message.reply_count ? (
          <button
            className="reply-link"
            type="button"
            onClick={() =>
              onOpenThread(message.room_id, message.event_id, "existingThread")
            }
          >
            {t("timeline.viewReplies", { count: message.reply_count })}
          </button>
        ) : null}
      </div>
    </article>
  );
}

function highlight(text: string, ranges: SearchResult["highlights"]) {
  const valid = ranges
    .filter(
      ({ start_utf16, end_utf16 }) =>
        start_utf16 >= 0 && end_utf16 > start_utf16 && end_utf16 <= text.length
    )
    .sort((left, right) => left.start_utf16 - right.start_utf16);
  if (!valid.length) return text;

  const nodes: ReactNode[] = [];
  let cursor = 0;
  valid.forEach(({ start_utf16, end_utf16 }, index) => {
    if (start_utf16 < cursor) return;
    nodes.push(text.slice(cursor, start_utf16));
    nodes.push(
      <mark key={`${start_utf16}:${end_utf16}:${index}`}>
        {text.slice(start_utf16, end_utf16)}
      </mark>
    );
    cursor = end_utf16;
  });
  nodes.push(text.slice(cursor));
  return nodes;
}

function matchFieldLabel(field: SearchResult["match_field"]): string {
  switch (field) {
    case "messageBody":
      return t("search.matchMessage");
    case "attachmentFileName":
      return t("search.matchAttachmentFileName");
  }
}

export {
  RoomMediaGallery,
  MediaViewer,
  ScheduledMessagesList,
  PinnedEventsList,
  PinnedMessagesEntry,
  SearchResults,
  MessageArticle
};
