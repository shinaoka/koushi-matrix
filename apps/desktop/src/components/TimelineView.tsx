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

import { MessageCircle } from "lucide-react";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState
} from "react";

import { t } from "../i18n/messages";

import type {
  CoreEventPayload,
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
  getKeyState,
  getPaginationState,
  shouldSuppressAutoBackfill,
  type TimelineStoreState
} from "../domain/timelineStore";

// ---------------------------------------------------------------------------
// Transport interface (Tauri IPC, browser fake, or test mock)
// ---------------------------------------------------------------------------

export interface TimelineTransport {
  /** Subscribe to `matrix-desktop://event`; returns an unsubscribe fn. */
  listenCoreEvents(listener: (payload: CoreEventPayload) => void): () => void;
  /** Invoke a backward-pagination command for the room. */
  paginateBackwards(roomId: string): Promise<void>;
}

/**
 * Row-level actions surfaced on timeline items. Matrix semantics stay
 * Rust-owned: the row only reports the (roomId, eventId) intent; the reply
 * target is recorded by the backend (setComposerReplyTarget).
 */
export interface TimelineRowActionHandlers {
  onReply: (roomId: string, eventId: string) => void;
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

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function TimelineView({
  timelineKey,
  roomId,
  transport,
  onReply
}: {
  timelineKey: TimelineKey;
  roomId: string;
  transport: TimelineTransport;
  onReply: TimelineRowActionHandlers["onReply"];
}) {
  const [store, setStore] = useState<TimelineStoreState>(createTimelineStore);
  const containerRef = useRef<HTMLDivElement>(null);
  /** Anchor captured before the latest prepend batch was applied. */
  const pendingAnchorRef = useRef<ScrollAnchor | null>(null);
  /** True from prepend-apply until anchor restoration completed. */
  const anchorRestorePendingRef = useRef(false);
  /** Pagination request currently in flight (suppresses duplicates). */
  const backfillInFlightRef = useRef(false);
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
  // Stable, render-visible timeline generation for this key. Bumps when the
  // store replaces the list for a new generation (InitialItems / resync), so
  // tests can poll a concrete attribute instead of sleeping. 0 = no
  // InitialItems received yet.
  const generation = getKeyState(store, timelineKey)?.generation ?? 0;

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
      .paginateBackwards(roomId)
      .catch(() => undefined)
      .finally(() => {
        backfillInFlightRef.current = false;
      });
  }, [store, roomId, transport]);

  return (
    <div
      className="timeline-view"
      data-testid="timeline-view"
      data-end-reached={endReached || undefined}
      data-timeline-generation={generation}
      ref={containerRef}
      style={{ overflowY: "auto", height: "100%" }}
      onScroll={maybeAutoBackfill}
    >
      {isPaginating ? (
        <div className="timeline-spinner" data-testid="timeline-spinner">
          読み込み中
        </div>
      ) : null}
      {endReached ? (
        <div className="timeline-start" data-testid="timeline-start">
          会話のはじまり
        </div>
      ) : null}
      {items.map((item) => (
        <TimelineItemRow
          item={item}
          key={timelineItemDomId(item.id)}
          roomId={roomId}
          onReply={onReply}
        />
      ))}
    </div>
  );
}

export function TimelineItemRow({
  item,
  roomId,
  onReply
}: {
  item: TimelineItem;
  roomId: string;
  onReply: TimelineRowActionHandlers["onReply"];
}) {
  const domId = timelineItemDomId(item.id);
  const isLocalEcho = "Transaction" in item.id;
  const eventId = "Event" in item.id ? item.id.Event.event_id : null;
  return (
    <article
      className="message"
      data-item-id={domId}
      data-send-state={isLocalEcho ? "unsent" : undefined}
      data-event-id={eventId ?? undefined}
      data-reply={item.in_reply_to_event_id ? "true" : undefined}
    >
      <div className="message-main">
        <div className="message-heading">
          <span className="sender">{item.sender ?? ""}</span>
          {isLocalEcho ? (
            <span className="message-send-state" data-send-state="unsent">
              Unsent
            </span>
          ) : null}
        </div>
        <div className="message-body">{item.body ?? ""}</div>
      </div>
      <div className="message-actions">
        {/* Only message events (with a body) are replyable. State events
            (room create, membership, ...) carry no body and the SDK's
            make_reply_event rejects them, so they get no reply affordance. */}
        {eventId && item.body !== null ? (
          <button
            className="message-action"
            type="button"
            aria-label={t("timeline.replyToMessage")}
            onClick={() => onReply(roomId, eventId)}
          >
            <MessageCircle size={14} />
          </button>
        ) : null}
      </div>
    </article>
  );
}
