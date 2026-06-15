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

import {
  Copy,
  Download,
  Edit3,
  FileCode2,
  FileText,
  Forward,
  ImageIcon,
  Link2,
  MessageCircle,
  MoreHorizontal,
  Pin,
  PinOff,
  RefreshCw,
  SmilePlus,
  Trash2,
  XCircle
} from "lucide-react";
import {
  Fragment,
  type FormEvent,
  type KeyboardEvent,
  type ReactNode,
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
  TimelineKey,
  TimelineMessageSource
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
  shouldLetNativeImeHandleComposerKeyEvent,
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
  /** Retry a failed outbound send queue item. */
  retrySend(roomId: string, transactionId: string): Promise<void>;
  /** Cancel/delete an outbound send queue item. */
  cancelSend(roomId: string, transactionId: string): Promise<void>;
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
  /** Pin a timeline event in the room. */
  pinEvent(roomId: string, eventId: string): Promise<void>;
  /** Unpin a timeline event in the room. */
  unpinEvent(roomId: string, eventId: string): Promise<void>;
  /** Download an event-backed media attachment. */
  downloadMedia(roomId: string, eventId: string): Promise<void>;
  /** Request a Rust-owned safe source DTO for an event-backed item. */
  loadMessageSource(roomId: string, eventId: string): Promise<void>;
  /** Forward an event-backed message through Rust-owned source projection. */
  forwardMessage(
    roomId: string,
    sourceEventId: string,
    destinationRoomId: string
  ): Promise<void>;
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
  onPin: (roomId: string, eventId: string) => void;
  onUnpin: (roomId: string, eventId: string) => void;
  onDownloadMedia: (roomId: string, eventId: string) => void;
  onLoadMessageSource: (roomId: string, eventId: string) => void;
  onForwardMessage: (roomId: string, sourceEventId: string, destinationRoomId: string) => void;
  onCopyText: (value: string) => void;
  onRetrySend: (roomId: string, transactionId: string) => void;
  onCancelSend: (roomId: string, transactionId: string) => void;
}

export interface TimelineForwardDestination {
  room_id: string;
  display_name: string;
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

const ignoreComposerKeyAction: ResolveComposerKeyAction = async () => "noop";
const ignoreSendQueueAction = () => undefined;

type TimelineMentionToken = {
  token: string;
  userId: string;
};

export function renderTimelineMessageText(
  text: string,
  query = "",
  profileUsers: Record<string, UserProfile> = {}
) {
  const mentionTokens = timelineMentionTokens(profileUsers);
  return text.split("\n").map((line, index) => (
    <span key={`${line}:${index}`}>
      {index > 0 ? <br /> : null}
      {renderTimelineMessageLine(line, query, mentionTokens)}
    </span>
  ));
}

function renderTimelineMessageLine(
  line: string,
  query: string,
  mentionTokens: TimelineMentionToken[]
): ReactNode {
  if (mentionTokens.length === 0) {
    return renderQueryHighlight(line, query);
  }

  const nodes: ReactNode[] = [];
  let cursor = 0;
  while (cursor < line.length) {
    const next = findNextMentionToken(line, cursor, mentionTokens);
    if (!next) {
      nodes.push(
        <Fragment key={`text:${cursor}`}>{renderQueryHighlight(line.slice(cursor), query)}</Fragment>
      );
      break;
    }
    if (next.start > cursor) {
      nodes.push(
        <Fragment key={`text:${cursor}`}>
          {renderQueryHighlight(line.slice(cursor, next.start), query)}
        </Fragment>
      );
    }
    const token = line.slice(next.start, next.end);
    nodes.push(
      <span
        className="message-mention-pill"
        data-mention-user-id={next.userId}
        dir="auto"
        key={`${next.userId}:${next.start}`}
      >
        {renderQueryHighlight(token, query)}
      </span>
    );
    cursor = next.end;
  }

  return nodes.length > 0 ? nodes : renderQueryHighlight(line, query);
}

function renderQueryHighlight(text: string, query: string): ReactNode {
  const trimmed = query.trim();
  if (!trimmed) {
    return text;
  }
  const index = text.indexOf(trimmed);
  if (index < 0) {
    return text;
  }
  return (
    <>
      {text.slice(0, index)}
      <mark>{text.slice(index, index + trimmed.length)}</mark>
      {text.slice(index + trimmed.length)}
    </>
  );
}

function findNextMentionToken(
  line: string,
  start: number,
  mentionTokens: TimelineMentionToken[]
): { start: number; end: number; userId: string } | null {
  for (let index = start; index < line.length; index += 1) {
    for (const mention of mentionTokens) {
      const end = index + mention.token.length;
      if (
        line.startsWith(mention.token, index) &&
        hasMentionTokenBoundary(line, index, end)
      ) {
        return { start: index, end, userId: mention.userId };
      }
    }
  }
  return null;
}

function timelineMentionTokens(
  profileUsers: Record<string, UserProfile>
): TimelineMentionToken[] {
  const tokens = new Map<string, string>();
  for (const profile of Object.values(profileUsers)) {
    const displayName = profile.display_name?.trim();
    if (displayName) {
      tokens.set(displayName.startsWith("@") ? displayName : `@${displayName}`, profile.user_id);
    }
    tokens.set(profile.user_id, profile.user_id);
  }
  return Array.from(tokens, ([token, userId]) => ({ token, userId }))
    .filter((mention) => mention.token.length > 1)
    .sort((a, b) => b.token.length - a.token.length || a.token.localeCompare(b.token));
}

function hasMentionTokenBoundary(line: string, start: number, end: number): boolean {
  return isMentionStartBoundary(line[start - 1]) && isMentionEndBoundary(line[end]);
}

function isMentionStartBoundary(value: string | undefined): boolean {
  return value === undefined || /\s|[([{<]/u.test(value);
}

function isMentionEndBoundary(value: string | undefined): boolean {
  return value === undefined || /\s|[.,!?;:)\]}>]/u.test(value);
}

async function writeClipboardText(value: string): Promise<void> {
  if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(value);
    return;
  }
  if (typeof document === "undefined") {
    return;
  }
  const textarea = document.createElement("textarea");
  textarea.value = value;
  textarea.setAttribute("readonly", "");
  textarea.style.position = "fixed";
  textarea.style.insetInlineStart = "-9999px";
  document.body.appendChild(textarea);
  textarea.select();
  document.execCommand("copy");
  document.body.removeChild(textarea);
}

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
  pinnedEventIds = [],
  forwardDestinations = [],
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
  pinnedEventIds?: readonly string[];
  forwardDestinations?: readonly TimelineForwardDestination[];
  suppressPaginationUi?: boolean;
}) {
  const [store, setStore] = useState<TimelineStoreState>(createTimelineStore);
  const [messageSource, setMessageSource] = useState<TimelineMessageSource | null>(null);
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
                    : "MessageForwarded" in event
                      ? event.MessageForwarded.key
                      : "MessageSourceLoaded" in event
                        ? event.MessageSourceLoaded.key
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

      if ("MessageSourceLoaded" in event) {
        setMessageSource(event.MessageSourceLoaded.source);
        return;
      }

      if ("MessageForwarded" in event) {
        return;
      }

      setStore((current) => applyTimelineEvent(current, event));
    });
    return unsubscribe;
  }, [transport]);

  const items = getItems(store, timelineKey);
  const notSentTransactionIds = items.flatMap((item) => {
    if (item.send_state?.kind !== "notSent" || !("Transaction" in item.id)) {
      return [];
    }
    return [item.id.Transaction.transaction_id];
  });
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
  const onRetrySend = useCallback(
    (targetRoomId: string, transactionId: string) => {
      void transport.retrySend(targetRoomId, transactionId).catch(() => undefined);
    },
    [transport]
  );
  const onCancelSend = useCallback(
    (targetRoomId: string, transactionId: string) => {
      void transport.cancelSend(targetRoomId, transactionId).catch(() => undefined);
    },
    [transport]
  );
  const onRetryAllNotSent = useCallback(() => {
    for (const transactionId of notSentTransactionIds) {
      onRetrySend(roomId, transactionId);
    }
  }, [notSentTransactionIds, onRetrySend, roomId]);
  const onCancelAllNotSent = useCallback(() => {
    for (const transactionId of notSentTransactionIds) {
      onCancelSend(roomId, transactionId);
    }
  }, [notSentTransactionIds, onCancelSend, roomId]);
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
  const onPin = useCallback(
    (targetRoomId: string, eventId: string) => {
      void transport.pinEvent(targetRoomId, eventId).catch(() => undefined);
    },
    [transport]
  );
  const onUnpin = useCallback(
    (targetRoomId: string, eventId: string) => {
      void transport.unpinEvent(targetRoomId, eventId).catch(() => undefined);
    },
    [transport]
  );
  const onDownloadMedia = useCallback(
    (targetRoomId: string, eventId: string) => {
      void transport.downloadMedia(targetRoomId, eventId).catch(() => undefined);
    },
    [transport]
  );
  const onLoadMessageSource = useCallback(
    (targetRoomId: string, eventId: string) => {
      void transport.loadMessageSource(targetRoomId, eventId).catch(() => undefined);
    },
    [transport]
  );
  const onForwardMessage = useCallback(
    (targetRoomId: string, sourceEventId: string, destinationRoomId: string) => {
      void transport
        .forwardMessage(targetRoomId, sourceEventId, destinationRoomId)
        .catch(() => undefined);
    },
    [transport]
  );
  const onCopyText = useCallback((value: string) => {
    void writeClipboardText(value).catch(() => undefined);
  }, []);
  const effectiveForwardDestinations =
    forwardDestinations.length > 0
      ? forwardDestinations
      : [{ room_id: roomId, display_name: roomId }];

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
      {notSentTransactionIds.length > 0 ? (
        <div className="timeline-send-bar" data-testid="timeline-send-bar">
          <span className="timeline-send-bar-label">
            {t("timeline.unsentBar")}
          </span>
          <div className="timeline-send-bar-actions">
            <button
              className="timeline-send-bar-action"
              type="button"
              onClick={onRetryAllNotSent}
            >
              <RefreshCw size={13} aria-hidden="true" />
              <span>{t("timeline.resendAll")}</span>
            </button>
            <button
              className="timeline-send-bar-action danger"
              type="button"
              onClick={onCancelAllNotSent}
            >
              <Trash2 size={13} aria-hidden="true" />
              <span>{t("timeline.cancelAll")}</span>
            </button>
          </div>
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
              isPinned={eventId ? pinnedEventIds.includes(eventId) : false}
              onPin={onPin}
              onUnpin={onUnpin}
              onDownloadMedia={onDownloadMedia}
              onLoadMessageSource={onLoadMessageSource}
              onForwardMessage={onForwardMessage}
              onCopyText={onCopyText}
              forwardDestinations={effectiveForwardDestinations}
              onRetrySend={onRetrySend}
              onCancelSend={onCancelSend}
              presence={item.sender ? liveSignals?.presence[item.sender] : undefined}
              profile={item.sender ? profileUsers[item.sender] : undefined}
              mentionProfileUsers={profileUsers}
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
      {messageSource ? (
        <MessageSourceDialog
          source={messageSource}
          onClose={() => setMessageSource(null)}
        />
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
  isPinned = false,
  onPin = () => undefined,
  onUnpin = () => undefined,
  onDownloadMedia = () => undefined,
  onLoadMessageSource = () => undefined,
  onForwardMessage = () => undefined,
  onCopyText = () => undefined,
  forwardDestinations = [],
  onRetrySend = ignoreSendQueueAction,
  onCancelSend = ignoreSendQueueAction,
  presence,
  profile,
  mentionProfileUsers = {},
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
  isPinned?: boolean;
  onPin?: TimelineRowActionHandlers["onPin"];
  onUnpin?: TimelineRowActionHandlers["onUnpin"];
  onDownloadMedia?: TimelineRowActionHandlers["onDownloadMedia"];
  onLoadMessageSource?: TimelineRowActionHandlers["onLoadMessageSource"];
  onForwardMessage?: TimelineRowActionHandlers["onForwardMessage"];
  onCopyText?: TimelineRowActionHandlers["onCopyText"];
  forwardDestinations?: readonly TimelineForwardDestination[];
  onRetrySend?: TimelineRowActionHandlers["onRetrySend"];
  onCancelSend?: TimelineRowActionHandlers["onCancelSend"];
  presence?: PresenceKind;
  profile?: UserProfile;
  mentionProfileUsers?: Record<string, UserProfile>;
  receipts?: LiveReadReceipt[];
}) {
  const domId = timelineItemDomId(item.id);
  const transactionId = "Transaction" in item.id ? item.id.Transaction.transaction_id : null;
  const eventId = "Event" in item.id ? item.id.Event.event_id : null;
  const isRedacted = item.is_redacted;
  const sendState = item.send_state ?? null;
  const sendStateKind = sendState?.kind ?? null;
  const [isEditing, setEditing] = useState(false);
  const [editDraft, setEditDraft] = useState(item.body ?? "");
  const [isReactionPickerOpen, setReactionPickerOpen] = useState(false);
  const [isActionMenuOpen, setActionMenuOpen] = useState(false);
  const [isForwardMenuOpen, setForwardMenuOpen] = useState(false);
  const reactionControlRef = useRef<HTMLDivElement>(null);
  const reactionTriggerRef = useRef<HTMLButtonElement>(null);
  const firstReactionRef = useRef<HTMLButtonElement>(null);
  const actionMenuControlRef = useRef<HTMLDivElement>(null);
  const actionMenuTriggerRef = useRef<HTMLButtonElement>(null);
  const firstActionMenuItemRef = useRef<HTMLButtonElement>(null);
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
    if (!isActionMenuOpen) {
      return;
    }
    firstActionMenuItemRef.current?.focus();
  }, [isActionMenuOpen]);

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

  useEffect(() => {
    if (!isActionMenuOpen) {
      return;
    }
    const handlePointerDown = (event: PointerEvent) => {
      const control = actionMenuControlRef.current;
      if (!control || control.contains(event.target as Node)) {
        return;
      }
      setActionMenuOpen(false);
      setForwardMenuOpen(false);
      actionMenuTriggerRef.current?.focus();
    };
    document.addEventListener("pointerdown", handlePointerDown);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
    };
  }, [isActionMenuOpen]);

  const closeReactionPicker = useCallback(() => {
    setReactionPickerOpen(false);
    reactionTriggerRef.current?.focus();
  }, []);

  const closeActionMenu = useCallback(() => {
    setActionMenuOpen(false);
    setForwardMenuOpen(false);
    actionMenuTriggerRef.current?.focus();
  }, []);

  const openEditForm = useCallback(() => {
    if (!eventId || isRedacted) {
      return;
    }
    setReactionPickerOpen(false);
    setActionMenuOpen(false);
    setForwardMenuOpen(false);
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

      const textarea = event.currentTarget;
      const selectionStart = textarea.selectionStart;
      const selectionEnd = textarea.selectionEnd;
      const keyEvent = composerKeyEventFromDom(event, {
        start: selectionStart,
        end: selectionEnd
      });
      const resolverOptions = {
        autocomplete_open: false,
        send_enabled: Boolean(eventId && editDraft.trim())
      };
      if (shouldLetNativeImeHandleComposerKeyEvent(keyEvent)) {
        void resolveComposerKeyAction("edit", keyEvent, resolverOptions).catch(() => undefined);
        return;
      }
      event.preventDefault();

      void resolveComposerKeyAction("edit", keyEvent, resolverOptions)
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
  const submitPin = useCallback(() => {
    if (!eventId) {
      return;
    }
    onPin(roomId, eventId);
  }, [eventId, onPin, roomId]);
  const submitUnpin = useCallback(() => {
    if (!eventId) {
      return;
    }
    onUnpin(roomId, eventId);
  }, [eventId, onUnpin, roomId]);
  const submitDownloadMedia = useCallback(() => {
    if (!eventId) {
      return;
    }
    onDownloadMedia(roomId, eventId);
  }, [eventId, onDownloadMedia, roomId]);
  const openActionMenu = useCallback(() => {
    setReactionPickerOpen(false);
    setForwardMenuOpen(false);
    setActionMenuOpen((current) => !current);
  }, []);
  const copyMessageBody = useCallback(() => {
    if (!item.actions?.can_copy || item.body === null) {
      return;
    }
    onCopyText(item.body);
    closeActionMenu();
  }, [closeActionMenu, item.actions?.can_copy, item.body, onCopyText]);
  const copyPermalink = useCallback(() => {
    const permalink = item.actions?.permalink;
    if (!item.actions?.can_permalink || !permalink) {
      return;
    }
    onCopyText(permalink);
    closeActionMenu();
  }, [closeActionMenu, item.actions?.can_permalink, item.actions?.permalink, onCopyText]);
  const loadMessageSource = useCallback(() => {
    if (!eventId || !item.actions?.can_view_source) {
      return;
    }
    onLoadMessageSource(roomId, eventId);
    closeActionMenu();
  }, [closeActionMenu, eventId, item.actions?.can_view_source, onLoadMessageSource, roomId]);
  const submitForward = useCallback(
    (destinationRoomId: string) => {
      if (!eventId || !item.actions?.can_forward) {
        return;
      }
      onForwardMessage(roomId, eventId, destinationRoomId);
      closeActionMenu();
    },
    [closeActionMenu, eventId, item.actions?.can_forward, onForwardMessage, roomId]
  );
  const submitRetrySend = useCallback(() => {
    if (!transactionId) {
      return;
    }
    onRetrySend(roomId, transactionId);
  }, [onRetrySend, roomId, transactionId]);
  const submitCancelSend = useCallback(() => {
    if (!transactionId) {
      return;
    }
    onCancelSend(roomId, transactionId);
  }, [onCancelSend, roomId, transactionId]);
  const canShowActionButtons = Boolean(eventId) && !isRedacted;
  const canShowReply = canShowActionButtons && item.body !== null;
  const canCopyMessage = Boolean(eventId && item.actions?.can_copy && item.body !== null);
  const canCopyPermalink = Boolean(
    eventId && item.actions?.can_permalink && item.actions.permalink
  );
  const canViewSource = Boolean(eventId && item.actions?.can_view_source);
  const canForward = Boolean(eventId && item.actions?.can_forward);
  const canShowMessageActionMenu =
    canCopyMessage || canCopyPermalink || canViewSource || canForward;
  const canShowThreadSummary = Boolean(eventId && item.thread_summary);
  const canShowReactions = !isRedacted && !isEditing && item.reactions.length > 0;
  const sendStateLabel =
    sendStateKind === "sending"
      ? t("timeline.sending")
      : sendStateKind === "notSent"
        ? t("timeline.notSent")
        : sendStateKind === "cancelled"
          ? t("timeline.cancelledSend")
          : null;
  const avatarUrl =
    profile?.avatar?.thumbnail.kind === "ready" ? profile.avatar.thumbnail.source_url : null;
  const threadSummaryText = item.thread_summary
    ? formatThreadSummary(
        item.thread_summary.reply_count,
        item.thread_summary.latest_sender,
        item.thread_summary.latest_body_preview
      )
    : "";
  const replyQuoteContent =
    !isRedacted && item.reply_quote ? (
      <div className="reply-quote" data-reply-state={item.reply_quote.state}>
        <div className="reply-quote-sender" dir="auto">
          {item.reply_quote.sender ?? t("timeline.replyQuoteUnknownSender")}
        </div>
        <div className="reply-quote-body" dir="auto">
          {replyQuoteBody(item.reply_quote)}
        </div>
      </div>
    ) : null;
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
      {renderTimelineMessageText(item.body ?? "", "", mentionProfileUsers)}
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
      data-send-state={sendStateKind && sendStateKind !== "sent" ? sendStateKind : undefined}
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
          {sendStateLabel ? (
            <span
              className="message-send-state"
              data-send-state={sendStateKind ?? undefined}
            >
              {sendStateLabel}
            </span>
          ) : null}
        </div>
        {replyQuoteContent}
        {bodyContent}
        {mediaContent}
        {transactionId && sendStateKind === "notSent" ? (
          <div className="message-send-actions">
            <button className="message-send-action" type="button" onClick={submitRetrySend}>
              <RefreshCw size={13} aria-hidden="true" />
              <span>{t("timeline.resendSend")}</span>
            </button>
            <button
              className="message-send-action danger"
              type="button"
              onClick={submitCancelSend}
            >
              <Trash2 size={13} aria-hidden="true" />
              <span>{t("timeline.deleteSend")}</span>
            </button>
          </div>
        ) : null}
        {transactionId && sendStateKind === "sending" ? (
          <div className="message-send-actions">
            <button className="message-send-action" type="button" onClick={submitCancelSend}>
              <XCircle size={13} aria-hidden="true" />
              <span>{t("timeline.cancelSend")}</span>
            </button>
          </div>
        ) : null}
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
        {!isEditing && canShowActionButtons ? (
          <button
            className="message-action"
            type="button"
            aria-label={isPinned ? t("timeline.unpinMessage") : t("timeline.pinMessage")}
            aria-pressed={isPinned}
            onClick={isPinned ? submitUnpin : submitPin}
          >
            {isPinned ? <PinOff size={14} /> : <Pin size={14} />}
          </button>
        ) : null}
        {!isEditing && canShowMessageActionMenu ? (
          <div className="message-action-menu-control" ref={actionMenuControlRef}>
            <button
              ref={actionMenuTriggerRef}
              className="message-action"
              type="button"
              aria-label={t("timeline.messageActions")}
              aria-expanded={isActionMenuOpen}
              aria-haspopup="menu"
              onClick={openActionMenu}
            >
              <MoreHorizontal size={14} />
            </button>
            {isActionMenuOpen ? (
              <div
                className="message-action-menu"
                role="menu"
                aria-label={t("timeline.messageActions")}
                onKeyDown={(event) => {
                  if (event.key === "Escape") {
                    event.preventDefault();
                    closeActionMenu();
                  }
                }}
              >
                {canCopyMessage ? (
                  <button
                    ref={firstActionMenuItemRef}
                    className="message-action-menu-item"
                    type="button"
                    role="menuitem"
                    onClick={copyMessageBody}
                  >
                    <Copy size={14} aria-hidden="true" />
                    <span>{t("timeline.copyMessage")}</span>
                  </button>
                ) : null}
                {canCopyPermalink ? (
                  <button
                    ref={!canCopyMessage ? firstActionMenuItemRef : undefined}
                    className="message-action-menu-item"
                    type="button"
                    role="menuitem"
                    onClick={copyPermalink}
                  >
                    <Link2 size={14} aria-hidden="true" />
                    <span>{t("timeline.copyPermalink")}</span>
                  </button>
                ) : null}
                {canViewSource ? (
                  <button
                    ref={!canCopyMessage && !canCopyPermalink ? firstActionMenuItemRef : undefined}
                    className="message-action-menu-item"
                    type="button"
                    role="menuitem"
                    onClick={loadMessageSource}
                  >
                    <FileCode2 size={14} aria-hidden="true" />
                    <span>{t("timeline.viewSource")}</span>
                  </button>
                ) : null}
                {canForward ? (
                  <div className="message-forward-menu-control">
                    <button
                      ref={
                        !canCopyMessage && !canCopyPermalink && !canViewSource
                          ? firstActionMenuItemRef
                          : undefined
                      }
                      className="message-action-menu-item"
                      type="button"
                      role="menuitem"
                      aria-haspopup="menu"
                      aria-expanded={isForwardMenuOpen}
                      onClick={() => setForwardMenuOpen((current) => !current)}
                    >
                      <Forward size={14} aria-hidden="true" />
                      <span>{t("timeline.forwardMessage")}</span>
                    </button>
                    {isForwardMenuOpen ? (
                      <div className="message-forward-menu" role="menu">
                        {forwardDestinations.map((destination) => (
                          <button
                            className="message-action-menu-item"
                            type="button"
                            role="menuitem"
                            key={destination.room_id}
                            onClick={() => submitForward(destination.room_id)}
                          >
                            <MessageCircle size={14} aria-hidden="true" />
                            <span dir="auto">{destination.display_name}</span>
                          </button>
                        ))}
                      </div>
                    ) : null}
                  </div>
                ) : null}
              </div>
            ) : null}
          </div>
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

function MessageSourceDialog({
  source,
  onClose
}: {
  source: TimelineMessageSource;
  onClose: () => void;
}) {
  const metadata: string[] = [];
  if (source.is_edited) {
    metadata.push(t("timeline.editedMessage"));
  }
  if (source.is_redacted) {
    metadata.push(t("timeline.redactedMessage"));
  }
  if (source.has_media) {
    metadata.push(t("timeline.sourceHasMedia"));
  }

  return (
    <div
      className="message-source-dialog"
      role="dialog"
      aria-label={t("timeline.messageSource")}
    >
      <div className="message-source-dialog-header">
        <span>{t("timeline.messageSource")}</span>
        <button
          className="message-source-close"
          type="button"
          aria-label={t("timeline.closeMessageSource")}
          onClick={onClose}
        >
          <XCircle size={15} aria-hidden="true" />
        </button>
      </div>
      <dl className="message-source-fields">
        <div>
          <dt>{t("timeline.sourceSender")}</dt>
          <dd dir="auto">{source.sender ?? t("timeline.replyQuoteUnknownSender")}</dd>
        </div>
        <div>
          <dt>{t("timeline.sourceBody")}</dt>
          <dd dir="auto">{source.body ?? t("timeline.sourceNoBody")}</dd>
        </div>
        {metadata.length > 0 ? (
          <div>
            <dt>{t("timeline.sourceMetadata")}</dt>
            <dd>{metadata.join(" · ")}</dd>
          </div>
        ) : null}
      </dl>
    </div>
  );
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

function replyQuoteBody(quote: NonNullable<TimelineItem["reply_quote"]>): string {
  if (quote.body_preview) {
    return quote.body_preview;
  }
  if (quote.state === "redacted") {
    return t("timeline.redactedMessage");
  }
  if (quote.state === "missing") {
    return t("timeline.replyQuoteMissing");
  }
  if (quote.state === "unsupported") {
    return t("timeline.replyQuoteUnsupported");
  }
  return t("timeline.replyQuoteUnavailable");
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
