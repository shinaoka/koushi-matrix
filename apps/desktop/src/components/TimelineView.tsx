/**
 * TimelineView: the event-driven timeline message list.
 *
 * Pure transport client of matrix-desktop-core: renders ONLY from the
 * timeline store fed by `matrix-desktop://event` CoreEvent payloads — never
 * from AppState timeline fields (Async rule 4).
 *
 * Viewport/Scrollback contract (docs/architecture/overview.md):
 *  - Before a prepend (backward-pagination) batch affects the viewport, an
 *    anchor is captured: first fully-or-partially visible stable item id plus
 *    its pixel offset from the scroll container top.
 *  - The diff is applied to React state; after React commits, the anchor is
 *    restored in a layout effect by adjusting scrollTop so the anchor item
 *    sits at the same pixel offset.
 *  - The next automatic backfill request is blocked until that restoration
 *    has completed.
 *  - EndReached (per-direction PaginationStateChanged) stops automatic
 *    backward pagination; Paginating drives the spinner.
 *
 * Transport abstraction: the component takes a `TimelineTransport` so the
 * same code runs against real Tauri IPC, the browser fixture preview, and
 * the headless test harness (mock IPC).
 */

import { Download, Edit3, FileText, ImageIcon, MessageCircle, SmilePlus, Trash2 } from "lucide-react";
import {
  type FormEvent,
  type KeyboardEvent,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState
} from "react";

import { t } from "../i18n/messages";

import type {
  CoreEventPayload,
  MediaTransferProgress,
  TimelineItem,
  TimelineKey
} from "../domain/coreEvents";
import { timelineItemDomId, timelineKeyEquals } from "../domain/coreEvents";
import {
  applyGlobalResync,
  applyTimelineEvent,
  batchContainsPrepend,
  createTimelineStore,
  getItems,
  getMediaUploadProgress,
  getKeyState,
  getPaginationState,
  shouldSuppressAutoBackfill,
  type TimelineStoreState
} from "../domain/timelineStore";
import {
  composerKeyEventFromDom,
  insertNewlineAtSelection,
  shouldResolveComposerKeyEvent
} from "../domain/composerKeyEvents";
import type {
  LiveReadReceipt,
  LiveSignalsState,
  PresenceKind,
  ResolveComposerKeyAction,
  UserProfile
} from "../domain/types";

// ---------------------------------------------------------------------------
// Transport interface (Tauri IPC, browser fake, or test mock)
// ---------------------------------------------------------------------------

export interface TimelineTransport {
  /** Subscribe to `matrix-desktop://event`; returns an unsubscribe fn. */
  listenCoreEvents(listener: (payload: CoreEventPayload) => void): () => void;
  /** Invoke a backward-pagination command for this timeline key. */
  paginateBackwards(timelineKey: TimelineKey): Promise<void>;
  /** Send a reaction command for a timeline event. */
  sendReaction(roomId: string, eventId: string, reactionKey: string): Promise<void>;
  /** Redact a reaction event. */
  redactReaction(
    roomId: string,
    eventId: string,
    reactionKey: string,
    reactionEventId: string
  ): Promise<void>;
  /** Send a read receipt for a room event. */
  sendReadReceipt(roomId: string, eventId: string): Promise<void>;
  /** Advance the fully-read marker for a room event. */
  setFullyRead(roomId: string, eventId: string): Promise<void>;
  /** Set typing state for a room. */
  setTyping(roomId: string, isTyping: boolean): Promise<void>;
  /** Edit a timeline event's message body. */
  editMessage(roomId: string, eventId: string, body: string): Promise<void>;
  /** Redact a timeline event. */
  redactMessage(roomId: string, eventId: string): Promise<void>;
  /** Download an event-backed media attachment. */
  downloadMedia(roomId: string, eventId: string): Promise<void>;
}

/**
 * Row-level actions surfaced on timeline items. Matrix semantics stay
 * Rust-owned: the row reports event-backed intent plus Rust-projected reaction
 * ownership; reply targeting, reaction send/redact, edits, redaction, and
 * download all travel through typed core transport paths.
 */
export interface TimelineRowActionHandlers {
  onReply: (roomId: string, eventId: string) => void;
  onOpenThread: (roomId: string, rootEventId: string) => void;
  onSendReaction: (roomId: string, eventId: string, reactionKey: string) => void;
  onRedactReaction: (
    roomId: string,
    eventId: string,
    reactionKey: string,
    reactionEventId: string
  ) => void;
  onEdit: (roomId: string, eventId: string, body: string) => void;
  onRedact: (roomId: string, eventId: string) => void;
  onDownloadMedia: (roomId: string, eventId: string) => void;
}

// ---------------------------------------------------------------------------
// Scroll anchor
// ---------------------------------------------------------------------------

interface ScrollAnchor {
  /** Stable item id of the anchor element. */
  itemId: string;
  /** Pixel offset of the anchor element top from the container's top edge. */
  offsetTop: number;
}

/** Capture the first visible item as the anchor (id + pixel offset). */
function captureAnchor(container: HTMLElement): ScrollAnchor | null {
  const containerTop = container.getBoundingClientRect().top;
  const nodes = container.querySelectorAll<HTMLElement>("[data-item-id]");
  for (const node of nodes) {
    const rect = node.getBoundingClientRect();
    if (rect.bottom > containerTop) {
      return {
        itemId: node.dataset["itemId"] ?? "",
        offsetTop: rect.top - containerTop
      };
    }
  }
  return null;
}

/** Restore the anchor by adjusting scrollTop; true if the anchor was found. */
function restoreAnchor(container: HTMLElement, anchor: ScrollAnchor): boolean {
  const node = container.querySelector<HTMLElement>(
    `[data-item-id="${cssEscape(anchor.itemId)}"]`
  );
  if (!node) {
    return false;
  }
  const containerTop = container.getBoundingClientRect().top;
  const currentOffset = node.getBoundingClientRect().top - containerTop;
  container.scrollTop += currentOffset - anchor.offsetTop;
  return true;
}

function cssEscape(value: string): string {
  return value.replace(/["\\]/g, "\\$&");
}

/** Distance (px) from the top edge that triggers automatic backfill. */
const AUTO_BACKFILL_THRESHOLD_PX = 80;
const REACTION_CHOICES = ["👍", "🎉", "❤️", "😂", "👀"] as const;

const ignoreComposerKeyAction: ResolveComposerKeyAction = async () => "ignore";

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function TimelineView({
  timelineKey,
  roomId,
  transport,
  onReply,
  onOpenThread = () => undefined,
  resolveComposerKeyAction = ignoreComposerKeyAction,
  liveSignals,
  profileUsers = {},
  suppressPaginationUi = false
}: {
  timelineKey: TimelineKey;
  roomId: string;
  transport: TimelineTransport;
  onReply: TimelineRowActionHandlers["onReply"];
  onOpenThread?: TimelineRowActionHandlers["onOpenThread"];
  resolveComposerKeyAction?: ResolveComposerKeyAction;
  liveSignals?: LiveSignalsState;
  profileUsers?: Record<string, UserProfile>;
  suppressPaginationUi?: boolean;
}) {
  const [store, setStore] = useState<TimelineStoreState>(createTimelineStore);
  const containerRef = useRef<HTMLDivElement>(null);
  /** Anchor captured before the latest prepend batch was applied. */
  const pendingAnchorRef = useRef<ScrollAnchor | null>(null);
  /** True from prepend-apply until anchor restoration completed. */
  const anchorRestorePendingRef = useRef(false);
  /** Pagination request currently in flight (suppresses duplicates). */
  const backfillInFlightRef = useRef(false);
  const readSignalEventRef = useRef<string | null>(null);
  const timelineKeyRef = useRef(timelineKey);
  timelineKeyRef.current = timelineKey;

  // --- Event subscription: apply CoreEvents to the store ---
  useEffect(() => {
    const unsubscribe = transport.listenCoreEvents((payload) => {
      if (payload.kind === "ResyncMarker") {
        // EventStreamLag: clear and await fresh InitialItems.
        pendingAnchorRef.current = null;
        anchorRestorePendingRef.current = false;
        setStore((current) => applyGlobalResync(current));
        return;
      }
      if (payload.kind !== "Timeline") {
        return;
      }
      const event = payload.event;

      // Key filter: only this timeline's events.
      const eventKey =
        "InitialItems" in event
          ? event.InitialItems.key
          : "ItemsUpdated" in event
            ? event.ItemsUpdated.key
            : "PaginationStateChanged" in event
              ? event.PaginationStateChanged.key
              : "SendCompleted" in event
                ? event.SendCompleted.key
                : "MediaUploadProgress" in event
                  ? event.MediaUploadProgress.key
                  : "MediaDownloadCompleted" in event
                    ? event.MediaDownloadCompleted.key
                    : event.ResyncRequired.key;
      if (!timelineKeyEquals(eventKey, timelineKeyRef.current)) {
        return;
      }

      // Prepend batches: capture the anchor BEFORE the diff is applied to
      // React state, so the layout effect can restore it after commit.
      if ("ItemsUpdated" in event && batchContainsPrepend(event.ItemsUpdated.diffs)) {
        const container = containerRef.current;
        if (container) {
          pendingAnchorRef.current = captureAnchor(container);
          anchorRestorePendingRef.current = true;
        }
      }

      if ("ResyncRequired" in event) {
        pendingAnchorRef.current = null;
        anchorRestorePendingRef.current = false;
      }

      setStore((current) => applyTimelineEvent(current, event));
    });
    return unsubscribe;
  }, [transport]);

  const items = getItems(store, timelineKey);
  const backwardState = getPaginationState(store, timelineKey, "Backward");
  const isPaginating = backwardState === "Paginating";
  const endReached = backwardState === "EndReached";
  const roomSignals = liveSignals?.rooms[roomId] ?? null;
  const roomTimelineRoomId = "Room" in timelineKey.kind ? timelineKey.kind.Room.room_id : null;
  const latestReadableEventId = latestEventBackedItemId(items);
  // Stable, render-visible timeline generation for this key. Bumps when the
  // store replaces the list for a new generation (InitialItems / resync), so
  // tests can poll a concrete attribute instead of sleeping. 0 = no
  // InitialItems received yet.
  const generation = getKeyState(store, timelineKey)?.generation ?? 0;
  const onSendReaction = useCallback(
    (targetRoomId: string, eventId: string, reactionKey: string) => {
      void transport.sendReaction(targetRoomId, eventId, reactionKey).catch(() => undefined);
    },
    [transport]
  );
  const onRedactReaction = useCallback(
    (targetRoomId: string, eventId: string, reactionKey: string, reactionEventId: string) => {
      void transport
        .redactReaction(targetRoomId, eventId, reactionKey, reactionEventId)
        .catch(() => undefined);
    },
    [transport]
  );
  const onEdit = useCallback(
    (targetRoomId: string, eventId: string, body: string) => {
      void transport.editMessage(targetRoomId, eventId, body).catch(() => undefined);
    },
    [transport]
  );
  const onRedact = useCallback(
    (targetRoomId: string, eventId: string) => {
      void transport.redactMessage(targetRoomId, eventId).catch(() => undefined);
    },
    [transport]
  );
  const onDownloadMedia = useCallback(
    (targetRoomId: string, eventId: string) => {
      void transport.downloadMedia(targetRoomId, eventId).catch(() => undefined);
    },
    [transport]
  );

  useEffect(() => {
    if (!latestReadableEventId || roomTimelineRoomId !== roomId) {
      return;
    }
    const signalKey = `${roomId}\u0000${latestReadableEventId}`;
    if (readSignalEventRef.current === signalKey) {
      return;
    }
    readSignalEventRef.current = signalKey;
    void transport.sendReadReceipt(roomId, latestReadableEventId).catch(() => undefined);
    void transport.setFullyRead(roomId, latestReadableEventId).catch(() => undefined);
  }, [latestReadableEventId, roomId, roomTimelineRoomId, transport]);

  // --- Anchor restoration: after React commits the prepend ---
  useLayoutEffect(() => {
    if (!anchorRestorePendingRef.current) {
      return;
    }
    const container = containerRef.current;
    const anchor = pendingAnchorRef.current;
    if (container && anchor) {
      restoreAnchor(container, anchor);
    }
    pendingAnchorRef.current = null;
    // Restoration complete: the next automatic fill request is allowed again.
    anchorRestorePendingRef.current = false;
  }, [items]);

  // --- Automatic backfill on scroll near the top ---
  const maybeAutoBackfill = useCallback(() => {
    if (suppressPaginationUi) {
      return;
    }
    const container = containerRef.current;
    if (!container) {
      return;
    }
    if (container.scrollTop > AUTO_BACKFILL_THRESHOLD_PX) {
      return;
    }
    // Block while: a previous diff's anchor restoration is pending, a
    // request is already in flight, or pagination is Paginating/EndReached.
    if (anchorRestorePendingRef.current || backfillInFlightRef.current) {
      return;
    }
    if (shouldSuppressAutoBackfill(store, timelineKeyRef.current)) {
      return;
    }
    backfillInFlightRef.current = true;
    void transport
      .paginateBackwards(timelineKeyRef.current)
      .catch(() => undefined)
      .finally(() => {
        backfillInFlightRef.current = false;
      });
  }, [store, transport, suppressPaginationUi]);

  return (
    <div
      className="timeline-view"
      data-testid="timeline-view"
      data-end-reached={endReached || undefined}
      data-timeline-generation={generation}
      ref={containerRef}
      style={{ overflowY: "auto", height: "100%" }}
      onScroll={suppressPaginationUi ? undefined : maybeAutoBackfill}
    >
      {!suppressPaginationUi && isPaginating ? (
        <div className="timeline-spinner" data-testid="timeline-spinner">
          {t("timeline.loading")}
        </div>
      ) : null}
      {!suppressPaginationUi && endReached ? (
        <div className="timeline-start" data-testid="timeline-start">
          {t("timeline.conversationStart")}
        </div>
      ) : null}
      {items.map((item) => {
        const eventId = "Event" in item.id ? item.id.Event.event_id : null;
        const isFullyReadMarker = Boolean(
          eventId && roomSignals?.fully_read_event_id === eventId
        );
        return (
          <div className="timeline-item-frame" key={timelineItemDomId(item.id)}>
            {isFullyReadMarker ? (
              <div className="read-marker" role="separator">
                <span>{t("timeline.readMarker")}</span>
              </div>
            ) : null}
            <TimelineItemRow
              item={item}
              roomId={roomId}
              onReply={onReply}
              onOpenThread={onOpenThread}
              resolveComposerKeyAction={resolveComposerKeyAction}
              mediaUploadProgress={mediaUploadProgressForItem(store, timelineKey, item)}
              onSendReaction={onSendReaction}
              onRedactReaction={onRedactReaction}
              onEdit={onEdit}
              onRedact={onRedact}
              onDownloadMedia={onDownloadMedia}
              presence={item.sender ? liveSignals?.presence[item.sender] : undefined}
              profile={item.sender ? profileUsers[item.sender] : undefined}
              receipts={eventId ? roomSignals?.receipts_by_event[eventId] ?? [] : []}
            />
          </div>
        );
      })}
      {roomSignals && roomSignals.typing_user_ids.length > 0 ? (
        <div className="typing-indicator" dir="auto">
          {formatTypingUsers(roomSignals.typing_user_ids)}
        </div>
      ) : null}
    </div>
  );
}

export function TimelineItemRow({
  item,
  roomId,
  onReply,
  onOpenThread = () => undefined,
  resolveComposerKeyAction = ignoreComposerKeyAction,
  mediaUploadProgress = null,
  onSendReaction,
  onRedactReaction,
  onEdit,
  onRedact,
  onDownloadMedia = () => undefined,
  presence,
  profile,
  receipts = []
}: {
  item: TimelineItem;
  roomId: string;
  onReply: TimelineRowActionHandlers["onReply"];
  onOpenThread?: TimelineRowActionHandlers["onOpenThread"];
  resolveComposerKeyAction?: ResolveComposerKeyAction;
  mediaUploadProgress?: MediaTransferProgress | null;
  onSendReaction: TimelineRowActionHandlers["onSendReaction"];
  onRedactReaction: TimelineRowActionHandlers["onRedactReaction"];
  onEdit: TimelineRowActionHandlers["onEdit"];
  onRedact: TimelineRowActionHandlers["onRedact"];
  onDownloadMedia?: TimelineRowActionHandlers["onDownloadMedia"];
  presence?: PresenceKind;
  profile?: UserProfile;
  receipts?: LiveReadReceipt[];
}) {
  const domId = timelineItemDomId(item.id);
  const isLocalEcho = "Transaction" in item.id;
  const eventId = "Event" in item.id ? item.id.Event.event_id : null;
  const isRedacted = item.is_redacted;
  const [isEditing, setEditing] = useState(false);
  const [editDraft, setEditDraft] = useState(item.body ?? "");
  const [isReactionPickerOpen, setReactionPickerOpen] = useState(false);
  const reactionControlRef = useRef<HTMLDivElement>(null);
  const reactionTriggerRef = useRef<HTMLButtonElement>(null);
  const firstReactionRef = useRef<HTMLButtonElement>(null);
  const editTextareaRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    if (!isReactionPickerOpen) {
      return;
    }
    firstReactionRef.current?.focus();
  }, [isReactionPickerOpen]);

  useEffect(() => {
    if (!isEditing) {
      return;
    }
    editTextareaRef.current?.focus();
  }, [isEditing]);

  useEffect(() => {
    if (!isReactionPickerOpen) {
      return;
    }
    const handlePointerDown = (event: PointerEvent) => {
      const control = reactionControlRef.current;
      if (!control || control.contains(event.target as Node)) {
        return;
      }
      setReactionPickerOpen(false);
      reactionTriggerRef.current?.focus();
    };
    document.addEventListener("pointerdown", handlePointerDown);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
    };
  }, [isReactionPickerOpen]);

  const closeReactionPicker = useCallback(() => {
    setReactionPickerOpen(false);
    reactionTriggerRef.current?.focus();
  }, []);

  const openEditForm = useCallback(() => {
    if (!eventId || isRedacted) {
      return;
    }
    setReactionPickerOpen(false);
    setEditDraft(item.body ?? "");
    setEditing(true);
  }, [eventId, isRedacted, item.body]);

  const closeEditForm = useCallback(() => {
    setEditing(false);
    setEditDraft(item.body ?? "");
  }, [item.body]);

  const submitEdit = useCallback(
    (event: FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      if (!eventId) {
        return;
      }
      const nextBody = editDraft.trim();
      if (!nextBody) {
        return;
      }
      onEdit(roomId, eventId, nextBody);
      closeEditForm();
    },
    [closeEditForm, editDraft, eventId, onEdit, roomId]
  );

  const onEditKeyDown = useCallback(
    (event: KeyboardEvent<HTMLTextAreaElement>) => {
      if (!shouldResolveComposerKeyEvent(event)) {
        return;
      }

      const keyEvent = composerKeyEventFromDom(event);
      const textarea = event.currentTarget;
      const selectionStart = textarea.selectionStart;
      const selectionEnd = textarea.selectionEnd;
      event.preventDefault();

      void resolveComposerKeyAction("edit", keyEvent, {
        autocomplete_open: false,
        send_enabled: Boolean(eventId && editDraft.trim())
      })
        .then((action) => {
          if (action === "send") {
            if (eventId && editDraft.trim()) {
              onEdit(roomId, eventId, editDraft.trim());
              closeEditForm();
            }
            return;
          }
          if (action === "insertNewline") {
            const nextDraft = insertNewlineAtSelection(
              editDraft,
              selectionStart,
              selectionEnd
            );
            setEditDraft(nextDraft.value);
            requestAnimationFrame(() => {
              textarea.selectionStart = nextDraft.cursor;
              textarea.selectionEnd = nextDraft.cursor;
            });
            return;
          }
          if (action === "cancel") {
            closeEditForm();
          }
        })
        .catch(() => undefined);
    },
    [closeEditForm, editDraft, eventId, onEdit, resolveComposerKeyAction, roomId]
  );

  const submitReaction = useCallback(
    (reactionKey: string) => {
      if (!eventId) {
        return;
      }
      const existingOwnReaction = item.reactions.find(
        (reaction) => reaction.key === reactionKey && reaction.reacted_by_me
      );
      if (existingOwnReaction) {
        if (existingOwnReaction.my_reaction_event_id) {
          onRedactReaction(
            roomId,
            eventId,
            reactionKey,
            existingOwnReaction.my_reaction_event_id
          );
        }
      } else {
        onSendReaction(roomId, eventId, reactionKey);
      }
      closeReactionPicker();
    },
    [closeReactionPicker, eventId, item.reactions, onRedactReaction, onSendReaction, roomId]
  );
  const submitReply = useCallback(() => {
    if (!eventId) {
      return;
    }
    onReply(roomId, eventId);
  }, [eventId, onReply, roomId]);
  const submitOpenThread = useCallback(() => {
    if (!eventId) {
      return;
    }
    onOpenThread(roomId, eventId);
  }, [eventId, onOpenThread, roomId]);
  const submitRedaction = useCallback(() => {
    if (!eventId) {
      return;
    }
    onRedact(roomId, eventId);
  }, [eventId, onRedact, roomId]);
  const submitDownloadMedia = useCallback(() => {
    if (!eventId) {
      return;
    }
    onDownloadMedia(roomId, eventId);
  }, [eventId, onDownloadMedia, roomId]);
  const canShowActionButtons = Boolean(eventId) && !isRedacted;
  const canShowReply = canShowActionButtons && item.body !== null;
  const canShowThreadSummary = Boolean(eventId && item.thread_summary);
  const canShowReactions = !isRedacted && !isEditing && item.reactions.length > 0;
  const avatarUrl =
    profile?.avatar?.thumbnail.kind === "ready" ? profile.avatar.thumbnail.source_url : null;
  const threadSummaryText = item.thread_summary
    ? formatThreadSummary(
        item.thread_summary.reply_count,
        item.thread_summary.latest_sender,
        item.thread_summary.latest_body_preview
      )
    : "";
  const bodyContent = isRedacted ? (
    <div className="message-body message-redacted" dir="auto">
      {t("timeline.redactedMessage")}
    </div>
  ) : isEditing ? (
    <form className="message-edit-form" onSubmit={submitEdit}>
      <textarea
        ref={editTextareaRef}
        aria-label={t("timeline.editBody")}
        className="message-edit-body"
        value={editDraft}
        onChange={(event) => setEditDraft(event.target.value)}
        onKeyDown={onEditKeyDown}
      />
      <div className="message-edit-actions">
        <button className="message-edit-button" type="submit">
          {t("timeline.saveEdit")}
        </button>
        <button
          className="message-edit-button"
          type="button"
          onClick={closeEditForm}
        >
          {t("timeline.cancelEdit")}
        </button>
      </div>
    </form>
  ) : (
    <div className="message-body" dir="auto">
      {item.body ?? ""}
    </div>
  );
  const mediaContent =
    !isRedacted && item.media ? (
      <TimelineMediaAttachment
        media={item.media}
        progress={mediaUploadProgress}
        canDownload={Boolean(eventId)}
        onDownload={submitDownloadMedia}
      />
    ) : null;
  return (
    <article
      className="message"
      data-item-id={domId}
      data-send-state={isLocalEcho ? "unsent" : undefined}
      data-event-id={eventId ?? undefined}
      data-redacted={isRedacted || undefined}
      data-reply={item.in_reply_to_event_id ? "true" : undefined}
    >
      <div className="avatar" aria-hidden="true">
        {avatarUrl ? <img src={avatarUrl} /> : senderInitials(item.sender)}
      </div>
      <div className="message-main">
        <div className="message-heading">
          {presence ? (
            <span
              className="presence-dot message-presence"
              data-presence={presence}
              aria-label={presenceLabel(presence)}
            />
          ) : null}
          <span className="sender" dir="auto">{item.sender ?? ""}</span>
          {item.is_edited && !isRedacted ? (
            <span className="message-edited">{t("timeline.editedMessage")}</span>
          ) : null}
          {isLocalEcho ? (
            <span className="message-send-state" data-send-state="unsent">
              {t("timeline.unsent")}
            </span>
          ) : null}
        </div>
        {bodyContent}
        {mediaContent}
        {canShowThreadSummary ? (
          <button
            className="thread-summary-chip"
            type="button"
            aria-label={t("timeline.openThreadSummary", { summary: threadSummaryText })}
            onClick={submitOpenThread}
          >
            <MessageCircle size={13} />
            <span>{threadSummaryText}</span>
          </button>
        ) : null}
        {receipts.length > 0 ? (
          <div className="message-receipts" aria-label={t("timeline.readBy", { count: receipts.length })}>
            <span className="receipt-dots" aria-hidden="true">
              {receipts.slice(0, 3).map((receipt) => (
                <span className="receipt-dot" key={receipt.user_id} />
              ))}
            </span>
            <span>{t("timeline.readBy", { count: receipts.length })}</span>
          </div>
        ) : null}
        {canShowReactions ? (
          <div className="message-reactions">
            {item.reactions.map((reaction, index) => {
              const ariaLabel = t("timeline.reactionSummary", {
                key: reaction.key,
                count: reaction.count
              });
              const pillKey = `${reaction.key}:${reaction.my_reaction_event_id ?? index}`;
              if (!eventId) {
                return (
                  <span
                    aria-label={ariaLabel}
                    className="reaction-pill"
                    data-reacted-by-me={reaction.reacted_by_me || undefined}
                    key={pillKey}
                  >
                    <span className="reaction-pill-key" dir="auto">
                      {reaction.key}
                    </span>
                    <span className="reaction-pill-count">{reaction.count}</span>
                  </span>
                );
              }
              return (
                <button
                  aria-label={ariaLabel}
                  className="reaction-pill"
                  data-reacted-by-me={reaction.reacted_by_me || undefined}
                  key={pillKey}
                  type="button"
                  aria-pressed={reaction.reacted_by_me}
                  onClick={() => {
                    if (reaction.reacted_by_me) {
                      if (reaction.my_reaction_event_id) {
                        onRedactReaction(
                          roomId,
                          eventId,
                          reaction.key,
                          reaction.my_reaction_event_id
                        );
                      }
                    } else {
                      onSendReaction(roomId, eventId, reaction.key);
                    }
                  }}
                >
                  <span className="reaction-pill-key" dir="auto">
                    {reaction.key}
                  </span>
                  <span className="reaction-pill-count">{reaction.count}</span>
                </button>
              );
            })}
          </div>
        ) : null}
      </div>
      <div className="message-actions">
        {!isEditing && canShowActionButtons && item.can_edit ? (
          <button
            className="message-action"
            type="button"
            aria-label={t("timeline.editMessage")}
            onClick={openEditForm}
          >
            <Edit3 size={14} />
          </button>
        ) : null}
        {!isEditing && canShowActionButtons && item.can_react ? (
          <div className="reaction-control" ref={reactionControlRef}>
            <button
              ref={reactionTriggerRef}
              className="message-action"
              type="button"
              aria-label={t("timeline.addReaction")}
              aria-expanded={isReactionPickerOpen}
              aria-haspopup="true"
              onClick={() => setReactionPickerOpen((current) => !current)}
            >
              <SmilePlus size={14} />
            </button>
            {isReactionPickerOpen ? (
              <div
                className="reaction-picker"
                role="group"
                aria-label={t("timeline.reactionPicker")}
                onKeyDown={(event) => {
                  if (event.key === "Escape") {
                    event.preventDefault();
                    closeReactionPicker();
                  }
                }}
              >
                {REACTION_CHOICES.map((reactionKey, index) => (
                  <button
                    key={reactionKey}
                    ref={index === 0 ? firstReactionRef : undefined}
                    className="reaction-picker-option"
                    type="button"
                    aria-label={t("timeline.reactionOption", { emoji: reactionKey })}
                    onClick={() => submitReaction(reactionKey)}
                  >
                    <span dir="auto">{reactionKey}</span>
                  </button>
                ))}
              </div>
            ) : null}
          </div>
        ) : null}
        {!isEditing && canShowReply ? (
          <button
            className="message-action"
            type="button"
            aria-label={t("timeline.replyToMessage")}
            onClick={submitReply}
          >
            <MessageCircle size={14} />
          </button>
        ) : null}
        {!isEditing && canShowActionButtons && item.can_redact ? (
          <button
            className="message-action"
            type="button"
            aria-label={t("timeline.redactMessage")}
            onClick={submitRedaction}
          >
            <Trash2 size={14} />
          </button>
        ) : null}
      </div>
    </article>
  );
}

function latestEventBackedItemId(items: TimelineItem[]): string | null {
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const item = items[index];
    if ("Event" in item.id) {
      return item.id.Event.event_id;
    }
  }
  return null;
}

function formatTypingUsers(userIds: string[]): string {
  const [firstUser] = userIds;
  if (userIds.length === 1 && firstUser) {
    return t("timeline.typingOne", { user: firstUser });
  }
  return t("timeline.typingMany", { count: userIds.length });
}

function presenceLabel(presence: PresenceKind): string {
  if (presence === "online") {
    return t("timeline.presenceOnline");
  }
  if (presence === "away") {
    return t("timeline.presenceAway");
  }
  return t("timeline.presenceOffline");
}

function senderInitials(sender: string | null): string {
  if (!sender) {
    return "?";
  }
  const ascii = sender.match(/[A-Za-z]/g);
  if (ascii?.length) {
    return ascii.slice(0, 2).join("").toUpperCase();
  }
  return sender.slice(0, 2);
}

function mediaUploadProgressForItem(
  store: TimelineStoreState,
  key: TimelineKey,
  item: TimelineItem
): MediaTransferProgress | null {
  if (!("Transaction" in item.id)) {
    return null;
  }
  return getMediaUploadProgress(store, key, item.id.Transaction.transaction_id);
}

function TimelineMediaAttachment({
  media,
  progress,
  canDownload,
  onDownload
}: {
  media: NonNullable<TimelineItem["media"]>;
  progress: MediaTransferProgress | null;
  canDownload: boolean;
  onDownload: () => void;
}) {
  const metadata = [
    media.mimetype,
    formatBytes(media.size),
    formatDimensions(media.width, media.height)
  ].filter((value): value is string => Boolean(value));
  const progressPercent = uploadProgressPercent(progress);
  const Icon = media.kind === "Image" ? ImageIcon : FileText;

  return (
    <div
      className="message-media"
      data-media-kind={media.kind}
      data-media-encrypted={media.source.encrypted || undefined}
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
        <button
          className="message-media-download"
          type="button"
          aria-label={t("timeline.downloadMedia", { filename: media.filename })}
          onClick={onDownload}
        >
          <Download size={15} />
        </button>
      ) : null}
    </div>
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

function uploadProgressPercent(progress: MediaTransferProgress | null): number | null {
  if (!progress || progress.total <= 0) {
    return null;
  }
  return Math.max(0, Math.min(100, Math.round((progress.current / progress.total) * 100)));
}

function formatThreadSummary(
  replyCount: number,
  latestSender: string | null,
  latestPreview: string | null
): string {
  const countText = t(
    replyCount === 1 ? "timeline.threadReplyCountOne" : "timeline.threadReplyCountMany",
    { count: replyCount }
  );
  if (latestSender && latestPreview) {
    return t("timeline.threadSummaryWithPreview", {
      count: countText,
      sender: latestSender,
      preview: latestPreview
    });
  }
  if (latestPreview) {
    return t("timeline.threadSummaryWithBody", {
      count: countText,
      preview: latestPreview
    });
  }
  if (latestSender) {
    return t("timeline.threadSummaryWithSender", {
      count: countText,
      sender: latestSender
    });
  }
  return countText;
}
