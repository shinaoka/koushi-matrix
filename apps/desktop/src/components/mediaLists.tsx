import { type FormEvent, useState } from "react";
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
  UserProfile
} from "../domain/types";
import { contextMenuItems } from "../domain/contextMenus";
import { mediaSourceUrl } from "../domain/mediaUrl";
import { renderTimelineMessageText } from "./TimelineView";
import {
  ICON_SIZE,
  formatUploadBytes,
  mediaGalleryItemLabel,
  formatTime,
  formatScheduledSendTime,
  scheduledSendCapabilityLabel,
  datetimeLocalValueFromTimestamp,
  scheduledSendTimestampFromInput,
  initials,
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
            ? mediaSourceUrl(download.source_url)
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
  const item = items[index];
  const previousIndex = (index + items.length - 1) % items.length;
  const nextIndex = (index + 1) % items.length;
  const label = mediaGalleryItemLabel(item);
  const download = mediaDownloads[item.event_id];
  const sourceUrl =
    item.media.kind === "Image" && download?.kind === "ready"
      ? mediaSourceUrl(download.source_url)
      : null;

  return (
    <div className="media-viewer-backdrop" role="dialog" aria-label={t("mediaGallery.viewerTitle")}>
      <div className="media-viewer">
        <header className="media-viewer-header">
          <div>
            <h2 dir="auto">{label}</h2>
            <p>
              {item.media.mimetype ?? item.media.kind}
              {item.media.size !== null ? ` - ${formatUploadBytes(item.media.size)}` : ""}
            </p>
          </div>
          <button className="icon-button" type="button" aria-label={t("mediaGallery.close")} onClick={onClose}>
            <X size={ICON_SIZE.small} />
          </button>
        </header>
        <div className="media-viewer-stage">
          {sourceUrl ? (
            <img
              className="media-viewer-image"
              src={sourceUrl}
              alt={label}
              style={{ transform: `scale(${zoom})` }}
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
          <button
            className="icon-button"
            type="button"
            aria-label={t("mediaGallery.previous")}
            onClick={() => {
              setZoom(1);
              onSelectIndex(previousIndex);
            }}
          >
            <ChevronLeft size={ICON_SIZE.control} />
          </button>
          <button
            className="icon-button"
            type="button"
            aria-label={t("mediaGallery.zoomOut")}
            onClick={() => setZoom((value) => Math.max(0.5, value - 0.25))}
          >
            <ZoomOut size={ICON_SIZE.control} />
          </button>
          <button
            className="icon-button"
            type="button"
            aria-label={t("mediaGallery.zoomIn")}
            onClick={() => setZoom((value) => Math.min(3, value + 0.25))}
          >
            <ZoomIn size={ICON_SIZE.control} />
          </button>
          <button
            className="icon-button"
            type="button"
            aria-label={t("mediaGallery.next")}
            onClick={() => {
              setZoom(1);
              onSelectIndex(nextIndex);
            }}
          >
            <ChevronRight size={ICON_SIZE.control} />
          </button>
        </footer>
      </div>
    </div>
  );
}

function ScheduledMessagesList({
  capability,
  items,
  onCancel,
  onReschedule
}: {
  capability: ScheduledSendCapability;
  items: ScheduledSendItem[];
  onCancel: (scheduledId: string) => void;
  onReschedule: (scheduledId: string, sendAtMs: number) => void;
}) {
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editValue, setEditValue] = useState("");

  if (items.length === 0) {
    return null;
  }

  function openEdit(item: ScheduledSendItem) {
    setEditingId(item.scheduled_id);
    setEditValue(datetimeLocalValueFromTimestamp(item.send_at_ms));
  }

  function submitEdit(event: FormEvent<HTMLFormElement>, item: ScheduledSendItem) {
    event.preventDefault();
    const sendAtMs = scheduledSendTimestampFromInput(editValue);
    if (sendAtMs === null) {
      return;
    }
    onReschedule(item.scheduled_id, sendAtMs);
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
              </div>
              {isEditing ? (
                <form
                  className="scheduled-message-edit"
                  onSubmit={(event) => submitEdit(event, item)}
                >
                  <label className="scheduled-send-field">
                    <span>{t("scheduled.timeInput")}</span>
                    <input
                      aria-label={t("scheduled.timeInput")}
                      type="datetime-local"
                      value={editValue}
                      onChange={(event) => setEditValue(event.currentTarget.value)}
                    />
                  </label>
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
                      disabled={scheduledSendTimestampFromInput(editValue) === null}
                    >
                      {t("scheduled.save")}
                    </button>
                  </div>
                </form>
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
  onUnpin
}: {
  roomId: string;
  pinnedEvents: DesktopSnapshot["state"]["domain"]["room_interactions"][string]["pinned_events"];
  onUnpin: (roomId: string, eventId: string) => void;
}) {
  return (
    <section className="pinned-events" aria-label={t("timeline.pinnedMessages")}>
      <div className="pinned-events-heading">
        <Pin size={ICON_SIZE.compact} aria-hidden="true" />
        <span>{t("timeline.pinnedMessages")}</span>
      </div>
      <div className="pinned-events-list">
        {pinnedEvents.map((event) => (
          <div className="pinned-event" key={event.event_id}>
            <div className="pinned-event-main">
              {event.sender ? (
                <span className="pinned-event-sender" dir="auto">
                  {event.sender}
                </span>
              ) : null}
              <span className="pinned-event-body" dir="auto">
                {event.redacted
                  ? t("timeline.redactedMessage")
                  : event.body_preview ?? t("timeline.pinnedMessage")}
              </span>
            </div>
            <button
              className="pinned-event-action"
              type="button"
              aria-label={t("timeline.unpinMessage")}
              onClick={() => onUnpin(roomId, event.event_id)}
            >
              <PinOff size={ICON_SIZE.micro} aria-hidden="true" />
            </button>
          </div>
        ))}
      </div>
    </section>
  );
}

function SearchResults({
  query,
  results,
  rooms,
  onResultSelect
}: {
  query: string;
  results: SearchResult[];
  rooms: DesktopSnapshot["state"]["domain"]["rooms"];
  onResultSelect: (roomId: string, eventId: string) => void;
}) {
  if (!query.trim()) {
    return null;
  }

  return (
    <section className="search-results">
      <div className="search-results-header">
        <span dir="auto">
          {t(results.length === 1 ? "search.resultCountOne" : "search.resultCountMany", {
            count: results.length,
            query
          })}
        </span>
      </div>
      <div className="result-list">
        {results.length ? (
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
                  <span dir="auto">{room?.display_label ?? result.room_id}</span> ·{" "}
                  {matchFieldLabel(result.match_field)}
                </span>
              </button>
            );
          })
        ) : (
          <div className="empty-results">{t("search.noExactMatches")}</div>
        )}
      </div>
    </section>
  );
}

function MessageArticle({
  currentUserId,
  message,
  query,
  onOpenContextMenu,
  onEditMessage,
  onOpenThread,
  onRedactMessage,
  profileUsers,
  isIgnored
}: {
  currentUserId: string | null;
  message: TimelineMessage;
  query: string;
  onOpenContextMenu?: OpenContextMenu;
  onEditMessage: (message: { body: string | null; room_id: string; event_id: string }) => void;
  onOpenThread: (roomId: string, rootEventId: string) => void;
  onRedactMessage: (roomId: string, eventId: string) => void;
  profileUsers: Record<string, UserProfile>;
  isIgnored: boolean;
}) {
  const canManage = currentUserId === message.sender;

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
                  canReply: message.body !== null,
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
        {initials(message.sender)}
      </div>
      <div className="message-main">
        <div className="message-heading">
          <span className="sender" dir="auto">{message.sender}</span>
          <span className="time">{formatTime(message.timestamp_ms)}</span>
          {canManage ? (
            <span className="message-actions">
              <button
                className="message-action"
                type="button"
                aria-label={t("timeline.editMessage")}
                onClick={() => onEditMessage(message)}
              >
                <Edit3 size={ICON_SIZE.micro} />
              </button>
              <button
                className="message-action"
                type="button"
                aria-label={t("timeline.redactMessage")}
                onClick={() => onRedactMessage(message.room_id, message.event_id)}
              >
                <X size={ICON_SIZE.micro} />
              </button>
            </span>
          ) : null}
        </div>
        <div className="message-body" dir="auto">
          {renderTimelineMessageText(message.body, query, profileUsers)}
        </div>
        {message.attachment_filename ? (
          <div className="attachment">
            <Paperclip size={ICON_SIZE.small} />
            <span dir="auto">{highlightQueryLines(message.attachment_filename, query)}</span>
          </div>
        ) : null}
        {message.reply_count ? (
          <button
            className="reply-link"
            type="button"
            onClick={() => onOpenThread(message.room_id, message.event_id)}
          >
            {t("timeline.viewReplies", { count: message.reply_count })}
          </button>
        ) : null}
      </div>
    </article>
  );
}

// ===== Search-result helpers (moved from App.tsx, used by SearchResults and MessageArticle) =====

function highlightQueryLines(text: string, query: string) {
  if (!query.trim()) {
    return text.split("\n").map((line, index) => (
      <span key={`${line}:${index}`}>
        {index > 0 ? <br /> : null}
        {line}
      </span>
    ));
  }

  return text.split("\n").map((line, index) => (
    <span key={`${line}:${index}`}>
      {index > 0 ? <br /> : null}
      {highlightString(line, query)}
    </span>
  ));
}

function highlightString(text: string, query: string) {
  const index = text.indexOf(query);
  if (index < 0 || query.length === 0) {
    return text;
  }
  return (
    <>
      {text.slice(0, index)}
      <mark>{text.slice(index, index + query.length)}</mark>
      {text.slice(index + query.length)}
    </>
  );
}

function highlight(text: string, ranges: SearchResult["highlights"]) {
  if (!ranges.length) {
    return text;
  }

  const range = ranges[0];
  const chars = Array.from(text);
  const start = utf16OffsetToCodePointIndex(text, range.start_utf16);
  const end = utf16OffsetToCodePointIndex(text, range.end_utf16);
  return (
    <>
      {chars.slice(0, start).join("")}
      <mark>{chars.slice(start, end).join("")}</mark>
      {chars.slice(end).join("")}
    </>
  );
}

function utf16OffsetToCodePointIndex(value: string, offset: number): number {
  let utf16Count = 0;
  for (const [index, char] of Array.from(value).entries()) {
    if (utf16Count >= offset) {
      return index;
    }
    utf16Count += char.length;
  }
  return Array.from(value).length;
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
  SearchResults,
  MessageArticle
};
