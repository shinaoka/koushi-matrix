/**
 * TimelineView: the event-driven timeline message list.
 *
 * Pure transport client of koushi-core: renders ONLY from the
 * timeline store fed by `koushi-desktop://event` CoreEvent payloads — never
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
  ArrowDown,
  Check,
  Copy,
  Download,
  Edit3,
  FileCode2,
  FileText,
  Forward,
  ImageIcon,
  KeyRound,
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
  type MouseEvent,
  type PointerEvent as ReactPointerEvent,
  memo,
  type ReactNode,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type Dispatch,
  type SetStateAction
} from "react";

import { getActiveLocale, t } from "../i18n/messages";
import { useRecoverableImageSource } from "./avatarImage";
import {
  contextMenuItems,
  type ContextMenuItem
} from "../domain/contextMenus";
import type { DiagnosticLogEntry } from "../domain/diagnostics";
import {
  AVATAR_THUMBNAIL_DOWNLOADS_ENABLED,
  avatarThumbnailFailureIsRetryable,
  avatarThumbnailRequestShouldBeSkipped,
  MAX_AVATAR_THUMBNAIL_ATTEMPTS
} from "../domain/avatarThumbnails";

import type {
  AvatarThumbnailState,
  CoreEventPayload,
  MediaTransferProgress,
  PaginationState,
  TimelineEvent,
  TimelineDiff,
  TimelineAnchorRestoreStatus,
  TimelineItem,
  TimelineKey,
  TimelineNavigationSnapshot,
  TimelineMessageSource
} from "../domain/coreEvents";
import { openExternalHttpUrl, toExternalHttpUrl } from "../domain/externalLinks";
import { mediaSourceUrl } from "../domain/mediaUrl";
import {
  recordTimelineEventReceived,
  recordTimelineInitialItems,
  recordTimelineKeyMismatch,
  recordTimelineResync
} from "../domain/timelineTransportStats";
import {
  timelineItemDomId,
  timelineKeyEquals
} from "../domain/coreEvents";
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
  createInitialTimelineScrollDiagnostics,
  recordTimelineScrollCommit,
  recordTimelineScrollFrame,
  recordTimelineScrollHeightCommit,
  recordTimelineScrollMeasurementFlush,
  recordTimelineScrollRangeCommit,
  recordTimelineScrollWrite,
  type TimelineScrollDiagnostics,
  type TimelineScrollWriteReason,
  type TimelineViewportIntentKind
} from "../domain/timelineScrollDiagnostics";
import { useTimelineStoreContext } from "./timelineStoreContext";
import {
  IS_MAC_PLATFORM,
  applyMacEmacsAction,
  composerKeyEventFromDom,
  insertNewlineAtSelection,
  macEmacsActionFromEvent,
  shouldLetNativeImeHandleComposerKeyEvent,
  shouldResolveComposerKeyEvent
} from "../domain/composerKeyEvents";
import type {
  LiveReadReceipt,
  LiveSignalsState,
  PresenceKind,
  ResolveComposerKeyAction,
  TimelineScrollAnchor,
  TimelineMediaDownloadState,
  UserProfile
} from "../domain/types";
import type { TimelineLinkRange } from "../domain/coreEvents";
import type { TimelineForwardDestination } from "../domain/projectionTypes";

export type { TimelineForwardDestination } from "../domain/projectionTypes";

// ---------------------------------------------------------------------------
// Transport interface (Tauri IPC, browser fake, or test mock)
// ---------------------------------------------------------------------------

export interface TimelineTransport {
  /** Subscribe to `koushi-desktop://event`; returns an unsubscribe fn. */
  listenCoreEvents(listener: (payload: CoreEventPayload) => void): () => void;
  /** Re/subscribe this key after the listener is active so InitialItems cannot be missed. */
  ensureSubscribed?(timelineKey: TimelineKey): Promise<void>;
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
  /** Download a Matrix avatar thumbnail for a visible sender avatar MXC. */
  downloadAvatarThumbnail?(mxcUri: string): Promise<void>;
  /** Request a Rust-owned safe source DTO for an event-backed item. */
  loadMessageSource(roomId: string, eventId: string): Promise<void>;
  /** Request missing room keys for an undecryptable event and retry decryption. */
  requestRoomKey(roomId: string, eventId: string): Promise<void>;
  /** Forward an event-backed message through Rust-owned source projection. */
  forwardMessage(
    roomId: string,
    sourceEventId: string,
    destinationRoomId: string
  ): Promise<void>;
  /** Request Rust-owned link-preview metadata for a timeline event. */
  loadLinkPreviews(roomId: string, eventId: string): Promise<void>;
  /** Hide the link previews for a timeline event. */
  hideLinkPreview(roomId: string, eventId: string): Promise<void>;
  /** Report viewport facts; Rust owns marker/count semantics. */
  observeViewport?(
    roomId: string,
    firstVisibleEventId: string | null,
    lastVisibleEventId: string | null,
    atBottom: boolean
  ): Promise<void>;
  /** Persist the current room-local read/scroll anchor. */
  updateScrollAnchor?(roomId: string, anchor: TimelineScrollAnchor): Promise<void>;
  /** Resolve a timestamp through Rust and open focused context. */
  openAtTimestamp?(roomId: string, timestampMs: number): Promise<void>;
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
  onRequestRoomKey: (roomId: string, eventId: string) => void;
  onForwardMessage: (roomId: string, sourceEventId: string, destinationRoomId: string) => void;
  onLoadLinkPreviews: (roomId: string, eventId: string) => void;
  onHideLinkPreview: (roomId: string, eventId: string) => void;
  onCopyText: (value: string) => void;
  onSetLocalUserAlias: (userId: string, alias: string | null) => void;
  onRetrySend: (roomId: string, transactionId: string) => void;
  onCancelSend: (roomId: string, transactionId: string) => void;
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

type PendingMeasuredHeight = {
  height: number;
  epoch: number;
};

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

type CapturedTimelineScrollAnchor = {
  event_id: string;
  edge: "bottom";
  offset_px: number;
};

type TimelineViewportSessionMemory =
  | { mode: "live-edge" }
  | { mode: "anchor"; anchor: TimelineScrollAnchor };

// UI-only memory for this JavaScript session. It intentionally resets on app
// restart: first entry into a room starts at live edge, while room switches
// during the same process can restore the user's last free-scroll anchor.
const timelineViewportSessionMemory = new Map<string, TimelineViewportSessionMemory>();

export function clearTimelineViewportSessionMemoryForTests(): void {
  timelineViewportSessionMemory.clear();
}

function captureRoomScrollAnchor(container: HTMLElement): CapturedTimelineScrollAnchor | null {
  const containerRect = container.getBoundingClientRect();
  const nodes = container.querySelectorAll<HTMLElement>("[data-event-id]");
  let captured: CapturedTimelineScrollAnchor | null = null;
  for (const node of nodes) {
    const rect = node.getBoundingClientRect();
    if (rect.bottom <= containerRect.top || rect.top >= containerRect.bottom) {
      continue;
    }
    const eventId = node.dataset["eventId"] ?? null;
    if (!eventId) {
      continue;
    }
    captured = {
      event_id: eventId,
      edge: "bottom",
      offset_px: Math.round(rect.bottom - containerRect.bottom)
    };
  }
  return captured;
}

function restoreRoomScrollAnchor(container: HTMLElement, anchor: TimelineScrollAnchor): boolean {
  const node = findRoomScrollAnchorNode(container, anchor);
  if (!node) {
    return false;
  }
  const currentOffset = currentRoomScrollAnchorOffset(container, node, anchor);
  container.scrollTop += currentOffset - anchor.offset_px;
  return true;
}

function currentRoomScrollAnchorOffset(
  container: HTMLElement,
  node: HTMLElement,
  anchor: TimelineScrollAnchor
): number {
  const containerRect = container.getBoundingClientRect();
  const nodeRect = node.getBoundingClientRect();
  return (anchor.edge ?? "top") === "bottom"
    ? nodeRect.bottom - containerRect.bottom
    : nodeRect.top - containerRect.top;
}

function findRoomScrollAnchorNode(
  container: HTMLElement,
  anchor: TimelineScrollAnchor
): HTMLElement | null {
  return container.querySelector<HTMLElement>(
    `[data-event-id="${cssEscape(anchor.event_id)}"]`
  );
}

function roomScrollAnchorSignature(roomId: string, anchor: TimelineScrollAnchor): string {
  return [
    roomId,
    anchor.event_id,
    anchor.edge ?? "top",
    anchor.offset_px,
    anchor.updated_at_ms
  ].join("\u0000");
}

function roomScrollAnchorStableSignature(
  roomId: string,
  anchor: Pick<TimelineScrollAnchor, "event_id" | "edge" | "offset_px">
): string {
  return [
    roomId,
    anchor.event_id,
    anchor.edge ?? "top",
    anchor.offset_px
  ].join("\u0000");
}

function timelineContainsEventId(items: readonly TimelineItem[], eventId: string): boolean {
  return items.some(
    (item) => "Event" in item.id && item.id.Event.event_id === eventId
  );
}

function visibleEventIds(container: HTMLElement): {
  firstVisibleEventId: string | null;
  lastVisibleEventId: string | null;
} {
  const containerRect = container.getBoundingClientRect();
  const nodes = container.querySelectorAll<HTMLElement>("[data-event-id]");
  let firstVisibleEventId: string | null = null;
  let lastVisibleEventId: string | null = null;
  for (const node of nodes) {
    const rect = node.getBoundingClientRect();
    if (rect.bottom <= containerRect.top || rect.top >= containerRect.bottom) {
      continue;
    }
    const eventId = node.dataset["eventId"] ?? null;
    if (!eventId) {
      continue;
    }
    firstVisibleEventId ??= eventId;
    lastVisibleEventId = eventId;
  }
  return { firstVisibleEventId, lastVisibleEventId };
}

function isScrolledToBottom(container: HTMLElement): boolean {
  return (
    container.scrollHeight - container.clientHeight - container.scrollTop <=
    SCROLL_EDGE_TOLERANCE_PX
  );
}

function scrollContainerToBottom(container: HTMLElement): void {
  container.scrollTop = container.scrollHeight - container.clientHeight;
}

function timelineKeyShouldReleaseViewportIntent(event: KeyboardEvent<HTMLDivElement>): boolean {
  if (event.altKey || event.ctrlKey || event.metaKey) {
    return false;
  }
  switch (event.key) {
    case "ArrowDown":
    case "ArrowUp":
    case "End":
    case "Home":
    case "PageDown":
    case "PageUp":
    case " ":
      return true;
    default:
      return false;
  }
}

function timelineDiffsContainOwnOutgoingItem(
  diffs: readonly TimelineDiff[],
  currentUserId: string | undefined
): boolean {
  if (!currentUserId) {
    return false;
  }
  return diffs.some((diff) => timelineDiffItems(diff).some((item) => timelineItemIsOwnOutgoing(item, currentUserId)));
}

function timelineDiffIsReset(diff: TimelineDiff): boolean {
  return diff === "Clear" || (typeof diff !== "string" && "Reset" in diff);
}

function timelineDiffsContainReset(diffs: readonly TimelineDiff[]): boolean {
  return diffs.some(timelineDiffIsReset);
}

function timelineDiffItems(diff: TimelineDiff): TimelineItem[] {
  if (typeof diff === "string") {
    return [];
  }
  if ("PushFront" in diff) {
    return [diff.PushFront.item];
  }
  if ("PushBack" in diff) {
    return [diff.PushBack.item];
  }
  if ("Insert" in diff) {
    return [diff.Insert.item];
  }
  if ("Set" in diff) {
    return [diff.Set.item];
  }
  if ("Reset" in diff) {
    return diff.Reset.items;
  }
  return [];
}

function timelineItemIsOwnOutgoing(item: TimelineItem, currentUserId: string): boolean {
  return item.sender === currentUserId && item.send_state != null;
}

function cssEscape(value: string): string {
  return value.replace(/["\\]/g, "\\$&");
}

/** Distance (px) from the top edge that triggers automatic backfill. */
const AUTO_BACKFILL_THRESHOLD_PX = 80;
const AUTO_BACKFILL_PREFETCH_ITEMS = 100;
const SCROLL_EDGE_TOLERANCE_PX = 2;
const TIMELINE_VIRTUALIZATION_THRESHOLD = 600;
const TIMELINE_VIRTUAL_OVERSCAN_ITEMS = 60;
const TIMELINE_ESTIMATED_ITEM_HEIGHT_PX = 72;
const TIMELINE_MIN_ITEM_HEIGHT_PX = 36;
const TIMELINE_MAX_ITEM_HEIGHT_PX = 480;
const TIMELINE_SCROLL_IDLE_FLUSH_MS = 100;
const TIMELINE_SCROLL_MAX_DEFER_MS = 500;
const REACTION_CHOICES = ["👍", "🎉", "❤️", "😂", "👀"] as const;

const ignoreComposerKeyAction: ResolveComposerKeyAction = async () => "noop";
const ignoreSendQueueAction = () => undefined;

type TimelineMentionToken = {
  token: string;
  userId: string;
};

type TimelineAliasTarget = {
  userId: string;
  displayLabel: string;
  originalDisplayLabel: string;
};

type TimelineViewportMetrics = {
  scrollTop: number;
  clientHeight: number;
  listOffsetTop: number;
};

type TimelineVirtualRangeState = {
  virtualized: boolean;
  startIndex: number;
  endIndex: number;
  paddingTop: number;
  paddingBottom: number;
};

type TimelineVirtualWindow = TimelineVirtualRangeState & {
  items: readonly TimelineItem[];
};

const EMPTY_TIMELINE_RANGE: TimelineVirtualRangeState = {
  virtualized: false,
  startIndex: 0,
  endIndex: 0,
  paddingTop: 0,
  paddingBottom: 0
};

type ViewportIntent = { kind: "free-scroll" } | { kind: "live-edge" };

type TimelineHeightModel = {
  fallbackHeight: number;
  offsets: number[];
  totalHeight: number;
};

function estimatedItemHeight(height: number): number {
  return Math.max(
    TIMELINE_MIN_ITEM_HEIGHT_PX,
    Math.min(TIMELINE_MAX_ITEM_HEIGHT_PX, height)
  );
}

function measuredItemHeight(height: number): number {
  return Math.max(1, Math.round(height));
}

function buildTimelineHeightModel(
  items: readonly TimelineItem[],
  measuredHeights: ReadonlyMap<string, number>,
  fallbackHeight: number
): TimelineHeightModel {
  const fallback = estimatedItemHeight(fallbackHeight);
  const offsets = new Array<number>(items.length + 1);
  offsets[0] = 0;
  for (const [index, item] of items.entries()) {
    const domId = timelineItemDomId(item.id);
    offsets[index + 1] = offsets[index] + (measuredHeights.get(domId) ?? fallback);
  }
  return {
    fallbackHeight: fallback,
    offsets,
    totalHeight: offsets[items.length] ?? 0
  };
}

function timelineIndexAtOffset(offsets: readonly number[], offset: number): number {
  if (offsets.length <= 1) {
    return 0;
  }
  const boundedOffset = Math.max(0, offset);
  let low = 0;
  let high = offsets.length - 2;
  while (low <= high) {
    const mid = Math.floor((low + high) / 2);
    if (offsets[mid + 1] <= boundedOffset) {
      low = mid + 1;
      continue;
    }
    if (offsets[mid] > boundedOffset) {
      high = mid - 1;
      continue;
    }
    return mid;
  }
  return Math.max(0, Math.min(offsets.length - 2, low));
}

function virtualRangeEquals(
  left: TimelineVirtualRangeState,
  right: TimelineVirtualRangeState
): boolean {
  return (
    left.virtualized === right.virtualized &&
    left.startIndex === right.startIndex &&
    left.endIndex === right.endIndex &&
    left.paddingTop === right.paddingTop &&
    left.paddingBottom === right.paddingBottom
  );
}

function calculateTimelineVirtualRange({
  visibleItemsLength,
  metrics,
  model
}: {
  visibleItemsLength: number;
  metrics: TimelineViewportMetrics;
  model: TimelineHeightModel;
}): TimelineVirtualRangeState {
  if (visibleItemsLength <= TIMELINE_VIRTUALIZATION_THRESHOLD) {
    return {
      virtualized: false,
      startIndex: 0,
      endIndex: visibleItemsLength,
      paddingTop: 0,
      paddingBottom: 0
    };
  }

  const viewportHeight = metrics.clientHeight || 600;
  const relativeScrollTop = Math.max(0, metrics.scrollTop - metrics.listOffsetTop);
  const firstVisibleIndex = timelineIndexAtOffset(model.offsets, relativeScrollTop);
  const lastVisibleIndex = timelineIndexAtOffset(
    model.offsets,
    relativeScrollTop + viewportHeight
  );
  const startIndex = Math.max(0, firstVisibleIndex - TIMELINE_VIRTUAL_OVERSCAN_ITEMS);
  const endIndex = Math.min(
    visibleItemsLength,
    Math.max(startIndex + 1, lastVisibleIndex + TIMELINE_VIRTUAL_OVERSCAN_ITEMS + 1)
  );

  return {
    virtualized: true,
    startIndex,
    endIndex,
    paddingTop: Math.round(model.offsets[startIndex] ?? 0),
    paddingBottom: Math.round(model.totalHeight - (model.offsets[endIndex] ?? 0))
  };
}

function timelineItemHeightAtIndex(model: TimelineHeightModel, index: number): number {
  return model.offsets[index + 1] - model.offsets[index] || model.fallbackHeight;
}

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

function renderTimelineMessageTextWithSpoilers(
  text: string,
  spoilerSpans: TimelineItem["spoiler_spans"] | undefined,
  query: string,
  profileUsers: Record<string, UserProfile>,
  spoilerState: SpoilerRevealState
): ReactNode {
  const spans = normalizeSpoilerSpans(spoilerSpans, text.length);
  if (spans.length === 0) {
    return renderTimelineMessageText(text, query, profileUsers);
  }

  const nodes: ReactNode[] = [];
  let cursor = 0;
  for (const [index, span] of spans.entries()) {
    if (span.start_utf16 > cursor) {
      const visibleText = text.slice(cursor, span.start_utf16);
      nodes.push(
        <Fragment key={`text:${cursor}`}>
          {renderTimelineMessageText(visibleText, query, profileUsers)}
        </Fragment>
      );
    }

    const spoilerText = text.slice(span.start_utf16, span.end_utf16);
    nodes.push(
      renderSpoiler(
        `plain:${span.start_utf16}:${span.end_utf16}:${index}`,
        renderTimelineMessageText(spoilerText, query, profileUsers),
        span.reason,
        spoilerState
      )
    );
    cursor = span.end_utf16;
  }

  if (cursor < text.length) {
    nodes.push(
      <Fragment key={`text:${cursor}`}>
        {renderTimelineMessageText(text.slice(cursor), query, profileUsers)}
      </Fragment>
    );
  }
  return nodes;
}

function renderPlainTextBody(
  text: string,
  linkRanges: TimelineLinkRange[],
  spoilerSpans: TimelineItem["spoiler_spans"] | undefined,
  query: string,
  profileUsers: Record<string, UserProfile>,
  spoilerState: SpoilerRevealState
): ReactNode {
  if (linkRanges.length === 0) {
    return renderTimelineMessageTextWithSpoilers(
      text,
      spoilerSpans,
      query,
      profileUsers,
      spoilerState
    );
  }
  const spans = normalizeSpoilerSpans(spoilerSpans, text.length);
  const sortedLinks = [...linkRanges].sort(
    (left, right) => left.start_utf16 - right.start_utf16
  );

  const nodes: ReactNode[] = [];
  let cursor = 0;
  for (const [index, span] of spans.entries()) {
    if (span.start_utf16 > cursor) {
      nodes.push(
        <Fragment key={`text:${cursor}`}>
          {renderPlainTextSegment(
            text,
            cursor,
            span.start_utf16,
            sortedLinks,
            query,
            profileUsers
          )}
        </Fragment>
      );
    }

    const spoilerText = renderPlainTextSegment(
      text,
      span.start_utf16,
      span.end_utf16,
      sortedLinks,
      query,
      profileUsers
    );
    nodes.push(
      renderSpoiler(
        `plain:${span.start_utf16}:${span.end_utf16}:${index}`,
        spoilerText,
        span.reason,
        spoilerState
      )
    );
    cursor = span.end_utf16;
  }

  if (cursor < text.length) {
    nodes.push(
      <Fragment key={`text:${cursor}`}>
        {renderPlainTextSegment(text, cursor, text.length, sortedLinks, query, profileUsers)}
      </Fragment>
    );
  }
  return nodes;
}

function renderPlainTextSegment(
  text: string,
  segStart: number,
  segEnd: number,
  sortedLinks: TimelineLinkRange[],
  query: string,
  profileUsers: Record<string, UserProfile>
): ReactNode {
  const nodes: ReactNode[] = [];
  let cursor = segStart;
  for (const range of sortedLinks) {
    if (range.end_utf16 <= cursor || range.start_utf16 >= segEnd) {
      continue;
    }
    const linkStart = Math.max(cursor, range.start_utf16);
    if (linkStart > cursor) {
      nodes.push(
        <Fragment key={`text:${cursor}`}>
          {renderTimelineMessageText(text.slice(cursor, linkStart), query, profileUsers)}
        </Fragment>
      );
    }
    const linkEnd = Math.min(segEnd, range.end_utf16);
    const href = toExternalHttpUrl(range.url);
    const linkContent = renderTimelineMessageText(
      text.slice(linkStart, linkEnd),
      query,
      profileUsers
    );
    nodes.push(
      href ? (
        <a
          key={`link:${range.start_utf16}`}
          href={href}
          rel="noopener noreferrer"
          target="_blank"
          onClick={(event) => {
            event.preventDefault();
            void openExternalHttpUrl(href);
          }}
        >
          {linkContent}
        </a>
      ) : (
        <Fragment key={`link:${range.start_utf16}`}>{linkContent}</Fragment>
      )
    );
    cursor = linkEnd;
  }

  if (cursor < segEnd) {
    nodes.push(
      <Fragment key={`text:${cursor}`}>
        {renderTimelineMessageText(text.slice(cursor, segEnd), query, profileUsers)}
      </Fragment>
    );
  }
  return nodes;
}

function normalizeSpoilerSpans(
  spoilerSpans: TimelineItem["spoiler_spans"] | undefined,
  textLength: number
) {
  let cursor = 0;
  return [...(spoilerSpans ?? [])]
    .sort((a, b) => a.start_utf16 - b.start_utf16 || a.end_utf16 - b.end_utf16)
    .flatMap((span) => {
      const start = Math.max(cursor, Math.min(span.start_utf16, textLength));
      const end = Math.max(start, Math.min(span.end_utf16, textLength));
      cursor = end;
      return start < end ? [{ ...span, start_utf16: start, end_utf16: end }] : [];
    });
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

type FormattedNode =
  | { kind: "text"; value: string }
  | {
      kind: "element";
      tagName: string;
      attrs: Record<string, string>;
      children: FormattedNode[];
    };

const FORMATTED_TAGS = new Set([
  "a",
  "b",
  "blockquote",
  "br",
  "code",
  "del",
  "em",
  "h1",
  "h2",
  "h3",
  "h4",
  "h5",
  "h6",
  "i",
  "li",
  "ol",
  "p",
  "pre",
  "s",
  "span",
  "strong",
  "ul"
]);

const VOID_FORMATTED_TAGS = new Set(["br"]);

function renderFormattedBody(
  formatted: NonNullable<TimelineItem["formatted"]>,
  linkRanges: TimelineLinkRange[],
  codeBlockWrap: boolean,
  onCopyText: TimelineRowActionHandlers["onCopyText"],
  searchQuery: string,
  spoilerState: SpoilerRevealState
): ReactNode {
  const nodes =
    linkRanges.length > 0 && !formatted.html.includes("<a")
      ? linkifyFormattedNodes(parseFormattedHtml(formatted.html), linkRanges)
      : parseFormattedHtml(formatted.html);
  const codeBlockIndexRef = { current: 0 };
  return renderFormattedNodes(
    nodes,
    formatted,
    codeBlockWrap,
    codeBlockIndexRef,
    onCopyText,
    searchQuery,
    spoilerState
  );
}

function parseFormattedHtml(html: string): FormattedNode[] {
  const root: Extract<FormattedNode, { kind: "element" }> = {
    kind: "element",
    tagName: "fragment",
    attrs: {},
    children: []
  };
  const stack: Array<Extract<FormattedNode, { kind: "element" }>> = [root];
  // Rust owns Matrix HTML safety and emits normalized sanitized HTML. This
  // tokenizer is only a renderer adapter for that DTO, not a sanitizer.
  const tokenPattern = /<!--[\s\S]*?-->|<\/?[^>]+>|[^<]+/g;
  for (const match of html.matchAll(tokenPattern)) {
    const token = match[0];
    if (token.startsWith("<!--")) {
      continue;
    }
    if (token.startsWith("</")) {
      const closeName = token.slice(2, -1).trim().toLowerCase();
      if (!closeName) {
        continue;
      }
      for (let index = stack.length - 1; index >= 0; index -= 1) {
        if (stack[index].tagName === closeName) {
          stack.length = index;
          break;
        }
      }
      continue;
    }
    if (token.startsWith("<")) {
      const parsed = parseFormattedStartTag(token);
      if (!parsed) {
        continue;
      }
      const node: FormattedNode = {
        kind: "element",
        tagName: parsed.tagName,
        attrs: parsed.attrs,
        children: []
      };
      stack[stack.length - 1].children.push(node);
      if (!parsed.selfClosing && !VOID_FORMATTED_TAGS.has(parsed.tagName)) {
        stack.push(node);
      }
      continue;
    }
    stack[stack.length - 1].children.push({ kind: "text", value: decodeHtmlEntities(token) });
  }
  return root.children;
}

function linkifyFormattedNodes(
  nodes: FormattedNode[],
  linkRanges: TimelineLinkRange[]
): FormattedNode[] {
  const sortedRanges = [...linkRanges].sort((left, right) => {
    if (left.start_utf16 !== right.start_utf16) {
      return left.start_utf16 - right.start_utf16;
    }
    return left.end_utf16 - right.end_utf16;
  });
  const cursor = { utf16: 0 };
  return linkifyFormattedNodeList(nodes, sortedRanges, cursor);
}

function linkifyFormattedNodeList(
  nodes: FormattedNode[],
  linkRanges: TimelineLinkRange[],
  cursor: { utf16: number }
): FormattedNode[] {
  return nodes.flatMap((node) => linkifyFormattedNode(node, linkRanges, cursor));
}

function linkifyFormattedNode(
  node: FormattedNode,
  linkRanges: TimelineLinkRange[],
  cursor: { utf16: number }
): FormattedNode[] {
  if (node.kind === "text") {
    const textStart = cursor.utf16;
    cursor.utf16 += node.value.length;
    return linkifyFormattedTextNode(node.value, textStart, linkRanges);
  }

  return [
    {
      ...node,
      children: linkifyFormattedNodeList(node.children, linkRanges, cursor)
    }
  ];
}

function linkifyFormattedTextNode(
  value: string,
  textStartUtf16: number,
  linkRanges: TimelineLinkRange[]
): FormattedNode[] {
  const textEndUtf16 = textStartUtf16 + value.length;
  const rangesInText = linkRanges.filter(
    (range) =>
      range.start_utf16 >= textStartUtf16 &&
      range.end_utf16 <= textEndUtf16 &&
      range.start_utf16 < range.end_utf16
  );
  if (rangesInText.length === 0) {
    return [{ kind: "text", value }];
  }

  const nodes: FormattedNode[] = [];
  let cursor = 0;
  for (const range of rangesInText) {
    const start = range.start_utf16 - textStartUtf16;
    const end = range.end_utf16 - textStartUtf16;
    if (start < cursor) {
      continue;
    }
    if (start > cursor) {
      nodes.push({ kind: "text", value: value.slice(cursor, start) });
    }
    nodes.push({
      kind: "element",
      tagName: "a",
      attrs: { href: range.url },
      children: [{ kind: "text", value: value.slice(start, end) }]
    });
    cursor = end;
  }
  if (cursor < value.length) {
    nodes.push({ kind: "text", value: value.slice(cursor) });
  }
  return nodes;
}

function parseFormattedStartTag(
  token: string
): { tagName: string; attrs: Record<string, string>; selfClosing: boolean } | null {
  const inner = token.slice(1, -1).trim();
  const selfClosing = inner.endsWith("/");
  const withoutSlash = selfClosing ? inner.slice(0, -1).trim() : inner;
  const tagMatch = withoutSlash.match(/^([a-z0-9-]+)/i);
  if (!tagMatch) {
    return null;
  }
  const tagName = tagMatch[1].toLowerCase();
  const attrs: Record<string, string> = {};
  if (FORMATTED_TAGS.has(tagName)) {
    const attrPattern = /([^\s=/>]+)(?:\s*=\s*("([^"]*)"|'([^']*)'|([^\s>]+)))?/g;
    for (const match of withoutSlash.slice(tagMatch[0].length).matchAll(attrPattern)) {
      const name = match[1].toLowerCase();
      const value = decodeHtmlEntities(match[3] ?? match[4] ?? match[5] ?? "");
      attrs[name] = value;
    }
  }
  return { tagName, attrs, selfClosing };
}

function renderFormattedNodes(
  nodes: FormattedNode[],
  formatted: NonNullable<TimelineItem["formatted"]>,
  codeBlockWrap: boolean,
  codeBlockIndexRef: { current: number },
  onCopyText: TimelineRowActionHandlers["onCopyText"],
  searchQuery: string,
  spoilerState: SpoilerRevealState,
  keyPrefix = ""
): ReactNode {
  return nodes.map((node, index) =>
    renderFormattedNode(
      node,
      keyPrefix ? `${keyPrefix}.${index}` : `${index}`,
      formatted,
      codeBlockWrap,
      codeBlockIndexRef,
      onCopyText,
      searchQuery,
      spoilerState
    )
  );
}

function renderFormattedNode(
  node: FormattedNode,
  key: string,
  formatted: NonNullable<TimelineItem["formatted"]>,
  codeBlockWrap: boolean,
  codeBlockIndexRef: { current: number },
  onCopyText: TimelineRowActionHandlers["onCopyText"],
  searchQuery: string,
  spoilerState: SpoilerRevealState
): ReactNode {
  if (node.kind === "text") {
    const lines = node.value.split("\n");
    if (lines.length === 1) {
      return <Fragment key={key}>{renderQueryHighlight(node.value, searchQuery)}</Fragment>;
    }
    return (
      <Fragment key={key}>
        {lines.map((line, lineIndex) => (
          <Fragment key={lineIndex}>
            {lineIndex > 0 ? <br /> : null}
            {renderQueryHighlight(line, searchQuery)}
          </Fragment>
        ))}
      </Fragment>
    );
  }
  const children = renderFormattedNodes(
    node.children,
    formatted,
    codeBlockWrap,
    codeBlockIndexRef,
    onCopyText,
    searchQuery,
    spoilerState,
    key
  );
  const renderer = formattedTagRenderers[node.tagName as keyof typeof formattedTagRenderers];
  if (!renderer) {
    return <Fragment key={key}>{children}</Fragment>;
  }
  return renderer(
    node,
    key,
    children,
    formatted,
    codeBlockWrap,
    codeBlockIndexRef,
    onCopyText,
    spoilerState
  );
}

type SpoilerRevealState = {
  revealed: ReadonlySet<string>;
  reveal: (spoilerKey: string) => void;
};

function renderSpoiler(
  key: string,
  children: ReactNode,
  reason: string | null | undefined,
  spoilerState: SpoilerRevealState
): ReactNode {
  const isRevealed = spoilerState.revealed.has(key);
  const normalizedReason = reason?.trim() || null;
  return (
    <button
      key={key}
      className="message-spoiler"
      type="button"
      data-revealed={isRevealed ? "true" : "false"}
      data-spoiler-reason={normalizedReason ?? undefined}
      aria-label={t("timeline.revealSpoiler")}
      onClick={() => spoilerState.reveal(key)}
    >
      {isRevealed ? children : <span aria-hidden="true">{t("timeline.spoiler")}</span>}
    </button>
  );
}

type FormattedTagRenderer = (
  node: Extract<FormattedNode, { kind: "element" }>,
  key: string,
  children: ReactNode,
  formatted: NonNullable<TimelineItem["formatted"]>,
  codeBlockWrap: boolean,
  codeBlockIndexRef: { current: number },
  onCopyText: TimelineRowActionHandlers["onCopyText"],
  spoilerState: SpoilerRevealState
) => ReactNode;

const formattedTagRenderers: Record<string, FormattedTagRenderer> = {
  a(
    node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    const href = toExternalHttpUrl(node.attrs.href?.trim());
    if (!href) {
      return <Fragment key={key}>{children}</Fragment>;
    }
    return (
      <a
        key={key}
        href={href}
        rel="noopener noreferrer"
        target="_blank"
        onClick={(event) => {
          event.preventDefault();
          void openExternalHttpUrl(href);
        }}
      >
        {children}
      </a>
    );
  },
  b(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <strong key={key}>{children}</strong>;
  },
  blockquote(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <blockquote key={key}>{children}</blockquote>;
  },
  br(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    _children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <br key={key} />;
  },
  code(
    node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    const className = node.attrs.class?.trim();
    return (
      <code key={key} className={className || undefined}>
        {children}
      </code>
    );
  },
  del(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <del key={key}>{children}</del>;
  },
  em(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <em key={key}>{children}</em>;
  },
  h1(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <h1 key={key}>{children}</h1>;
  },
  h2(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <h2 key={key}>{children}</h2>;
  },
  h3(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <h3 key={key}>{children}</h3>;
  },
  h4(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <h4 key={key}>{children}</h4>;
  },
  h5(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <h5 key={key}>{children}</h5>;
  },
  h6(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <h6 key={key}>{children}</h6>;
  },
  i(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <em key={key}>{children}</em>;
  },
  li(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <li key={key}>{children}</li>;
  },
  ol(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <ol key={key}>{children}</ol>;
  },
  p(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <p key={key}>{children}</p>;
  },
  pre(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    _children: ReactNode,
    formatted: NonNullable<TimelineItem["formatted"]>,
    codeBlockWrap: boolean,
    codeBlockIndexRef: { current: number },
    onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    const codeBlock = formatted.code_blocks[codeBlockIndexRef.current];
    codeBlockIndexRef.current += 1;
    if (!codeBlock) {
      return <pre key={key} />;
    }
    const languageClass = codeBlock.language ? `language-${codeBlock.language}` : null;
    return (
      <div key={key} className="message-code-block">
        <div className="message-code-block-actions">
          <button
            className="message-code-block-copy"
            type="button"
            aria-label={t("timeline.copyCode")}
            onClick={() => onCopyText(codeBlock.body)}
          >
            <Copy size={13} aria-hidden="true" />
            <span>{t("timeline.copyCode")}</span>
          </button>
        </div>
        <pre className="message-code-block-pre" data-code-block-wrap={codeBlockWrap ? "true" : "false"}>
          <code className={languageClass || undefined}>{codeBlock.body}</code>
        </pre>
      </div>
    );
  },
  s(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <del key={key}>{children}</del>;
  },
  span(
    node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"],
    spoilerState: SpoilerRevealState
  ) {
    const className = node.attrs.class?.trim();
    const spoiler = node.attrs["data-mx-spoiler"];
    const color = node.attrs["data-mx-color"];
    if (spoiler !== undefined) {
      return renderSpoiler(`formatted:${key}`, children, spoiler, spoilerState);
    }
    return (
      <span
        key={key}
        className={className || undefined}
        data-mx-color={color || undefined}
        data-mx-spoiler={spoiler ?? undefined}
      >
        {children}
      </span>
    );
  },
  strong(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <strong key={key}>{children}</strong>;
  },
  ul(
    _node: Extract<FormattedNode, { kind: "element" }>,
    key: string,
    children: ReactNode,
    _formatted: NonNullable<TimelineItem["formatted"]>,
    _codeBlockWrap: boolean,
    _codeBlockIndexRef: { current: number },
    _onCopyText: TimelineRowActionHandlers["onCopyText"]
  ) {
    return <ul key={key}>{children}</ul>;
  }
} as const;

function decodeHtmlEntities(text: string): string {
  return text.replace(/&(#x?[0-9a-fA-F]+|[a-zA-Z]+);/g, (match, entity: string) => {
    if (entity.startsWith("#x") || entity.startsWith("#X")) {
      const codePoint = Number.parseInt(entity.slice(2), 16);
      return isValidHtmlCodePoint(codePoint) ? String.fromCodePoint(codePoint) : match;
    }
    if (entity.startsWith("#")) {
      const codePoint = Number.parseInt(entity.slice(1), 10);
      return isValidHtmlCodePoint(codePoint) ? String.fromCodePoint(codePoint) : match;
    }
    switch (entity) {
      case "amp":
        return "&";
      case "lt":
        return "<";
      case "gt":
        return ">";
      case "quot":
        return '"';
      case "apos":
      case "nbsp":
        return entity === "nbsp" ? " " : "'";
      default:
        return match;
    }
  });
}

function isValidHtmlCodePoint(codePoint: number): boolean {
  return Number.isInteger(codePoint) && codePoint >= 0 && codePoint <= 0x10ffff;
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
    const terms = profile.mention_search_terms.length
      ? profile.mention_search_terms
      : [profile.display_label, profile.user_id];
    for (const term of terms) {
      const normalized = term.trim();
      if (normalized) {
        tokens.set(normalized.startsWith("@") ? normalized : `@${normalized}`, profile.user_id);
      }
    }
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

export interface TimelineDiagnostics {
  visibleItems: number;
  downloadedItems: number;
  backfill: string;
  avatarMxcItems: number;
  avatarReadyItems: number;
  avatarPendingItems: number;
  avatarFailedItems: number;
  avatarMissingItems: number;
  avatarRenderedImages: number;
  avatarBrokenImages: number;
}

export type TimelineDiagnosticLogEntry = DiagnosticLogEntry;

export const TimelineView = memo(function TimelineView({
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
  onSetLocalUserAlias,
  onOpenContextMenu,
  currentUserId,
  ignoredUserIds = [],
  suppressPaginationUi = false,
  autoLoadOlderMessages = false,
  codeBlockWrap = true,
  searchQuery = "",
  mediaDownloads = {},
  roomScrollAnchor: _persistedRoomScrollAnchor = null,
  enableAvatarThumbnailDownloads = AVATAR_THUMBNAIL_DOWNLOADS_ENABLED,
  onDiagnosticsChange,
  onScrollDiagnosticsChange,
  onDiagnosticLogEntry,
  timelineStore,
  setTimelineStore,
  listRefCallback
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
  onSetLocalUserAlias?: TimelineRowActionHandlers["onSetLocalUserAlias"];
  onOpenContextMenu?: (
    event: MouseEvent<HTMLElement>,
    target: {
      kind: "message";
      message: { sender: string; room_id: string; event_id: string; body: string };
    },
    items: ContextMenuItem[]
  ) => void;
  currentUserId?: string;
  ignoredUserIds?: string[];
  suppressPaginationUi?: boolean;
  autoLoadOlderMessages?: boolean;
  codeBlockWrap?: boolean;
  searchQuery?: string;
  mediaDownloads?: Record<string, TimelineMediaDownloadState>;
  roomScrollAnchor?: TimelineScrollAnchor | null;
  /**
   * Temporary #116 perf gate. Defaults to AVATAR_THUMBNAIL_DOWNLOADS_ENABLED
   * (currently false). Set to true in tests that verify the firing path, or
   * when re-enabling downloads behind a Rust-owned setting + cache.
   */
  enableAvatarThumbnailDownloads?: boolean;
  onDiagnosticsChange?: (diagnostics: TimelineDiagnostics) => void;
  onScrollDiagnosticsChange?: (diagnostics: TimelineScrollDiagnostics) => void;
  onDiagnosticLogEntry?: (entry: TimelineDiagnosticLogEntry) => void;
  /**
   * Optional App-level timeline store. When supplied, the view renders from
   * this store and leaves reducer application to the owner. It still listens for
   * view-local side-effect events such as source dialogs and anchor completion.
   */
  timelineStore?: TimelineStoreState;
  /**
   * Updater for the optional App-level store. Must be supplied together with
   * `timelineStore` by tests that explicitly own reducer application.
   */
  setTimelineStore?: Dispatch<SetStateAction<TimelineStoreState>>;
  /**
   * Optional callback receiving the timeline list element so parent chrome can
   * drive scroll actions such as "jump to latest".
   */
  listRefCallback?: (element: HTMLDivElement | null) => void;
}) {
  // Persisted restart anchors are intentionally ignored for restoration:
  // first entry after app startup goes to live edge, while in-session room
  // switches use timelineViewportSessionMemory. persistViewportAnchor still
  // writes these anchors for diagnostics and future cross-restart design work.
  void _persistedRoomScrollAnchor;
  const timelineStoreContext = useTimelineStoreContext();
  const [localStore, localSetStore] = useState<TimelineStoreState>(createTimelineStore);
  const store = timelineStore ?? timelineStoreContext?.store ?? localStore;
  const setStore = setTimelineStore ?? timelineStoreContext?.setStore ?? localSetStore;
  const isAppLevelStore = timelineStore !== undefined || timelineStoreContext !== null;
  const [messageSource, setMessageSource] = useState<TimelineMessageSource | null>(null);
  const [navigationSnapshot, setNavigationSnapshot] =
    useState<TimelineNavigationSnapshot | null>(null);
  const [avatarThumbnails, setAvatarThumbnails] = useState<Record<string, AvatarThumbnailState>>(
    {}
  );
  const [viewportAtBottom, setViewportAtBottom] = useState(false);
  const [aliasTarget, setAliasTarget] = useState<TimelineAliasTarget | null>(null);
  const [aliasDraft, setAliasDraft] = useState("");
  const viewportMetricsRef = useRef<TimelineViewportMetrics>({
    scrollTop: 0,
    clientHeight: 0,
    listOffsetTop: 0
  });
  const [virtualRange, setVirtualRange] =
    useState<TimelineVirtualRangeState>(EMPTY_TIMELINE_RANGE);
  const virtualRangeRef = useRef<TimelineVirtualRangeState>(EMPTY_TIMELINE_RANGE);
  const pendingScrollFrameRef = useRef<number | null>(null);
  const rangeModelEpochRef = useRef(0);
  const virtualItemHeight = TIMELINE_ESTIMATED_ITEM_HEIGHT_PX;
  const [measuredHeightVersion, setMeasuredHeightVersion] = useState(0);
  const containerRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const itemHeightByDomIdRef = useRef<Map<string, number>>(new Map());
  /** Anchor captured before the latest prepend batch was applied. */
  const pendingAnchorRef = useRef<ScrollAnchor | null>(null);
  /** True from prepend-apply until anchor restoration completed. */
  const anchorRestorePendingRef = useRef(false);
  /** True while the live-room scroll anchor is being restored. */
  const roomScrollAnchorRestorePendingRef = useRef(false);
  /** Suppresses capture while programmatic scroll adjustments are running. */
  const suppressScrollAnchorCaptureRef = useRef(false);
  /** Last reason-tagged programmatic scroll write, used to classify its echo. */
  const programmaticScrollSignatureRef = useRef<{
    scrollHeight: number;
    scrollTop: number;
    reason: TimelineScrollWriteReason;
    token: number;
  } | null>(null);
  const programmaticScrollTokenRef = useRef(0);
  const lastPersistedViewportAnchorSignatureRef = useRef<string | null>(null);
  const restoredRoomScrollAnchorSignatureRef = useRef<string | null>(null);
  const anchorAsyncGenerationRef = useRef(0);
  /** Tracks whether the current key already got its first live-edge scroll. */
  const initialLiveEdgeScrollAppliedRef = useRef<string | null>(null);
  /** Keeps the live edge pinned when measured virtual heights change. */
  const stickToBottomAfterMeasurementRef = useRef(false);
  /** Viewport intent that survives timeline re-renders until user scroll input changes it. */
  const viewportIntentRef = useRef<ViewportIntent>({ kind: "free-scroll" });
  const scrollActivityRef = useRef<"idle" | "active">("idle");
  const scrollIdleTimerRef = useRef<number | null>(null);
  const scrollMaxDeferTimerRef = useRef<number | null>(null);
  const pendingMeasuredHeightsRef = useRef<Map<string, PendingMeasuredHeight>>(new Map());
  const measurementEpochRef = useRef(0);
  const visibleItemDomIdsRef = useRef<Set<string>>(new Set());
  const mountedItemDomIdsRef = useRef<Set<string>>(new Set());
  /** Set by wheel/touch/keyboard/scrollbar intent; consumed by the next scroll event. */
  const userScrollInputPendingRef = useRef(false);
  const pendingScrollFrameUserInputRef = useRef(false);
  /** Coalesces ResizeObserver-driven live-edge corrections. */
  const viewportIntentResizeFrameRef = useRef<number | null>(null);
  /** Pagination request currently in flight (suppresses duplicates). */
  const backfillInFlightRef = useRef(false);
  const readSignalEventRef = useRef<string | null>(null);
  const lastViewportObservationRef = useRef<string | null>(null);
  const downloadedEventIdsRef = useRef<Set<string>>(new Set());
  const requestedImagePreviewEventIdsRef = useRef<Set<string>>(new Set());
  const relevantAvatarMxcsRef = useRef<Set<string>>(new Set());
  const requestedAvatarMxcsRef = useRef<Set<string>>(new Set());
  const avatarRetryCountsRef = useRef<Map<string, number>>(new Map());
  const emptyThreadBackfillRequestedRef = useRef(false);
  const lastDiagnosticsEmissionRef = useRef<{
    callback: (diagnostics: TimelineDiagnostics) => void;
    signature: string;
  } | null>(null);
  const scrollDiagnosticsRef = useRef<TimelineScrollDiagnostics>(
    createInitialTimelineScrollDiagnostics()
  );
  const onScrollDiagnosticsChangeRef = useRef(onScrollDiagnosticsChange);
  onScrollDiagnosticsChangeRef.current = onScrollDiagnosticsChange;
  const profileUsersRef = useRef(profileUsers);
  profileUsersRef.current = profileUsers;
  const timelineKeyRef = useRef(timelineKey);
  timelineKeyRef.current = timelineKey;
  const timelineKeyHash = JSON.stringify(timelineKey);
  const timelineKeyHashRef = useRef(timelineKeyHash);
  timelineKeyHashRef.current = timelineKeyHash;
  const sessionRoomScrollAnchorRef = useRef<TimelineScrollAnchor | null>(null);
  const roomTimelineRoomId = "Room" in timelineKey.kind ? timelineKey.kind.Room.room_id : null;
  const emitDiagnosticLog = useCallback(
    (source: string, message: string) => {
      onDiagnosticLogEntry?.({
        timestampMs: Date.now(),
        source,
        message
      });
    },
    [onDiagnosticLogEntry]
  );
  const emitScrollDiagnostics = useCallback(() => {
    onScrollDiagnosticsChangeRef.current?.(scrollDiagnosticsRef.current);
  }, []);
  const updateScrollDiagnostics = useCallback(
    (update: (current: TimelineScrollDiagnostics) => TimelineScrollDiagnostics) => {
      scrollDiagnosticsRef.current = update(scrollDiagnosticsRef.current);
      emitScrollDiagnostics();
    },
    [emitScrollDiagnostics]
  );
  const cancelPendingScrollFrame = useCallback(() => {
    if (pendingScrollFrameRef.current !== null) {
      window.cancelAnimationFrame(pendingScrollFrameRef.current);
      pendingScrollFrameRef.current = null;
    }
    pendingScrollFrameUserInputRef.current = false;
  }, []);
  const clearMeasurementTimers = useCallback(() => {
    if (scrollIdleTimerRef.current !== null) {
      window.clearTimeout(scrollIdleTimerRef.current);
      scrollIdleTimerRef.current = null;
    }
    if (scrollMaxDeferTimerRef.current !== null) {
      window.clearTimeout(scrollMaxDeferTimerRef.current);
      scrollMaxDeferTimerRef.current = null;
    }
  }, []);
  const clearPendingMeasurementDiagnostics = useCallback(() => {
    if (scrollDiagnosticsRef.current.pendingMeasuredRows === 0) {
      return;
    }
    scrollDiagnosticsRef.current = {
      ...scrollDiagnosticsRef.current,
      pendingMeasuredRows: 0
    };
    emitScrollDiagnostics();
  }, [emitScrollDiagnostics]);
  const resetActiveMeasurementDeferral = useCallback(
    (options: { clearMountedIds?: boolean } = {}) => {
      clearMeasurementTimers();
      scrollActivityRef.current = "idle";
      measurementEpochRef.current += 1;
      pendingMeasuredHeightsRef.current.clear();
      if (options.clearMountedIds) {
        mountedItemDomIdsRef.current = new Set();
      }
      clearPendingMeasurementDiagnostics();
    },
    [clearMeasurementTimers, clearPendingMeasurementDiagnostics]
  );
  const readViewportMetrics = useCallback((): TimelineViewportMetrics => {
    const container = containerRef.current;
    if (!container) {
      return viewportMetricsRef.current;
    }
    const next = {
      scrollTop: container.scrollTop,
      clientHeight: container.clientHeight,
      listOffsetTop: listRef.current?.offsetTop ?? 0
    };
    viewportMetricsRef.current = next;
    return next;
  }, []);

  const persistViewportAnchor = useCallback((options?: { allowSuppressed?: boolean }): boolean => {
    if (!transport.updateScrollAnchor || roomTimelineRoomId !== roomId) {
      return false;
    }
    if (
      anchorRestorePendingRef.current ||
      roomScrollAnchorRestorePendingRef.current ||
      (!options?.allowSuppressed && suppressScrollAnchorCaptureRef.current)
    ) {
      return false;
    }
    const container = containerRef.current;
    if (!container) {
      return false;
    }
    const captured = captureRoomScrollAnchor(container);
    if (!captured) {
      return false;
    }
    // Persist the observed viewport for diagnostics/future cross-restart use.
    // The active in-session viewport state is updated only when viewport intent
    // changes, not from every incidental scroll event.
    const stableSignature = roomScrollAnchorStableSignature(roomId, captured);
    if (lastPersistedViewportAnchorSignatureRef.current === stableSignature) {
      return false;
    }
    lastPersistedViewportAnchorSignatureRef.current = stableSignature;
    const updatedAtMs = Date.now();
    void transport
      .updateScrollAnchor(roomId, {
        ...captured,
        updated_at_ms: updatedAtMs
      })
      .catch(() => undefined);
    return true;
  }, [roomId, roomTimelineRoomId, transport]);

  const runWithScrollWriteReason = useCallback(
    (reason: TimelineScrollWriteReason, action: () => void) => {
      const asyncGeneration = anchorAsyncGenerationRef.current;
      suppressScrollAnchorCaptureRef.current = true;
      const container = containerRef.current;
      const beforeScrollTop = container?.scrollTop ?? 0;
      action();
      if (container && container.scrollTop !== beforeScrollTop) {
        const token = programmaticScrollTokenRef.current + 1;
        programmaticScrollTokenRef.current = token;
        programmaticScrollSignatureRef.current = {
          scrollHeight: container.scrollHeight,
          scrollTop: container.scrollTop,
          reason,
          token
        };
        updateScrollDiagnostics((current) => recordTimelineScrollWrite(current, reason));
      }
      requestAnimationFrame(() => {
        if (anchorAsyncGenerationRef.current !== asyncGeneration) {
          return;
        }
        suppressScrollAnchorCaptureRef.current = false;
        programmaticScrollSignatureRef.current = null;
      });
    },
    [updateScrollDiagnostics]
  );

  const setViewportIntentToLiveEdge = useCallback(() => {
    viewportIntentRef.current = { kind: "live-edge" };
    timelineViewportSessionMemory.set(timelineKeyHash, { mode: "live-edge" });
  }, [timelineKeyHash]);

  const flushPendingMeasurements = useCallback(
    (reason: "idle" | "maxDefer") => {
      const pending = pendingMeasuredHeightsRef.current;
      if (pending.size === 0) {
        clearMeasurementTimers();
        scrollActivityRef.current = "idle";
        clearPendingMeasurementDiagnostics();
        return;
      }

      const currentEpoch = measurementEpochRef.current;
      const visibleDomIds = visibleItemDomIdsRef.current;
      const measuredMountedHeights = new Map<string, number>();
      const mountedDomIds = new Set<string>();
      const list = listRef.current;
      if (list) {
        for (const node of Array.from(list.querySelectorAll<HTMLElement>(".timeline-item-frame"))) {
          const domId =
            node.dataset["frameItemId"] ??
            node.querySelector<HTMLElement>("[data-item-id]")?.dataset["itemId"];
          if (!domId) {
            continue;
          }
          mountedDomIds.add(domId);
          measuredMountedHeights.set(domId, measuredItemHeight(node.getBoundingClientRect().height));
        }
        mountedItemDomIdsRef.current = mountedDomIds;
      } else {
        for (const domId of mountedItemDomIdsRef.current) {
          mountedDomIds.add(domId);
        }
      }
      const nextHeights = new Map(itemHeightByDomIdRef.current);
      let changedRows = 0;
      const committedDomIds = new Set<string>();
      for (const domId of nextHeights.keys()) {
        if (!visibleDomIds.has(domId)) {
          nextHeights.delete(domId);
        }
      }
      for (const [domId, entry] of pending) {
        if (
          entry.epoch !== currentEpoch ||
          !visibleDomIds.has(domId) ||
          !mountedDomIds.has(domId)
        ) {
          continue;
        }
        const height = measuredMountedHeights.get(domId) ?? entry.height;
        if (Math.abs((nextHeights.get(domId) ?? 0) - height) > 1) {
          nextHeights.set(domId, height);
          changedRows += 1;
        }
        committedDomIds.add(domId);
      }
      for (const [domId, height] of measuredMountedHeights) {
        if (
          committedDomIds.has(domId) ||
          !visibleDomIds.has(domId) ||
          Math.abs((nextHeights.get(domId) ?? 0) - height) <= 1
        ) {
          continue;
        }
        nextHeights.set(domId, height);
        changedRows += 1;
      }
      pending.clear();
      clearMeasurementTimers();
      scrollActivityRef.current = "idle";

      if (changedRows === 0) {
        clearPendingMeasurementDiagnostics();
        return;
      }

      const container = containerRef.current;
      const measuredAtBottom = Boolean(container && isScrolledToBottom(container));
      stickToBottomAfterMeasurementRef.current = measuredAtBottom;
      if (measuredAtBottom) {
        setViewportIntentToLiveEdge();
      }

      itemHeightByDomIdRef.current = nextHeights;
      updateScrollDiagnostics((current) =>
        ({
          ...recordTimelineScrollMeasurementFlush(
            recordTimelineScrollHeightCommit(current, "idleFlush"),
            changedRows
          ),
          pendingMeasuredRows: 0
        })
      );
      setMeasuredHeightVersion((current) => current + 1);

      if (reason === "maxDefer") {
        emitDiagnosticLog("timeline.scroll", "measurement flush reason=max_defer");
      }
    },
    [
      clearMeasurementTimers,
      clearPendingMeasurementDiagnostics,
      emitDiagnosticLog,
      setViewportIntentToLiveEdge,
      updateScrollDiagnostics
    ]
  );

  const markScrollActivityActive = useCallback(() => {
    scrollActivityRef.current = "active";
    if (scrollIdleTimerRef.current !== null) {
      window.clearTimeout(scrollIdleTimerRef.current);
    }
    scrollIdleTimerRef.current = window.setTimeout(
      () => flushPendingMeasurements("idle"),
      TIMELINE_SCROLL_IDLE_FLUSH_MS
    );
    if (scrollMaxDeferTimerRef.current === null) {
      scrollMaxDeferTimerRef.current = window.setTimeout(
        () => flushPendingMeasurements("maxDefer"),
        TIMELINE_SCROLL_MAX_DEFER_MS
      );
    }
  }, [flushPendingMeasurements]);

  const setViewportIntentToFreeScroll = useCallback(() => {
    viewportIntentRef.current = { kind: "free-scroll" };
    stickToBottomAfterMeasurementRef.current = false;
  }, []);

  const releaseViewportIntent = useCallback(() => {
    setViewportIntentToFreeScroll();
    userScrollInputPendingRef.current = false;
  }, [setViewportIntentToFreeScroll]);

  const markUserScrollInput = useCallback((options: { keepLiveEdgeAtBottom?: boolean } = {}) => {
    userScrollInputPendingRef.current = true;
    const container = containerRef.current;
    if (
      options.keepLiveEdgeAtBottom &&
      container &&
      isScrolledToBottom(container)
    ) {
      return;
    }
    setViewportIntentToFreeScroll();
  }, [setViewportIntentToFreeScroll]);

  const applyViewportIntent = useCallback((): boolean => {
    const container = containerRef.current;
    if (!container || viewportIntentRef.current.kind !== "live-edge") {
      return false;
    }
    const targetScrollTop = Math.max(0, container.scrollHeight - container.clientHeight);
    let changed = false;
    if (Math.abs(container.scrollTop - targetScrollTop) > SCROLL_EDGE_TOLERANCE_PX) {
      runWithScrollWriteReason("liveEdge", () => {
        container.scrollTop = targetScrollTop;
      });
      changed = true;
    }
    return persistViewportAnchor({ allowSuppressed: true }) || changed;
  }, [persistViewportAnchor, runWithScrollWriteReason]);

  useEffect(() => {
    scrollDiagnosticsRef.current = recordTimelineScrollCommit(scrollDiagnosticsRef.current);
  });

  // --- Event subscription: local stores apply reducers; App stores keep view effects here. ---
  useEffect(() => {
    const unsubscribe = transport.listenCoreEvents((payload) => {
      if (payload.kind === "ResyncMarker") {
        // EventStreamLag: the core event broadcast overflowed and dropped
        // events for this consumer (likely including this room's InitialItems).
        // Clear, then RE-SUBSCRIBE so the core re-emits a fresh InitialItems;
        // clearing alone would leave the timeline permanently blank.
        recordTimelineResync();
        pendingAnchorRef.current = null;
        anchorRestorePendingRef.current = false;
        roomScrollAnchorRestorePendingRef.current = false;
        viewportIntentRef.current = { kind: "free-scroll" };
        userScrollInputPendingRef.current = false;
        resetActiveMeasurementDeferral({ clearMountedIds: true });
        lastPersistedViewportAnchorSignatureRef.current = null;
        restoredRoomScrollAnchorSignatureRef.current = null;
        setNavigationSnapshot(null);
        relevantAvatarMxcsRef.current = new Set();
        if (!isAppLevelStore) {
          setStore((current) => {
            const next = applyGlobalResync(current);
            relevantAvatarMxcsRef.current = timelineAvatarMxcsForItems(
              getItems(next, timelineKeyRef.current),
              profileUsersRef.current
            );
            return next;
          });
        }
        void transport.ensureSubscribed?.(timelineKeyRef.current).catch(() => undefined);
        return;
      }
      if (payload.kind === "Account" && "AvatarThumbnailDownloaded" in payload.event) {
        const { mxc_uri, thumbnail } = payload.event.AvatarThumbnailDownloaded;
        if (
          !requestedAvatarMxcsRef.current.has(mxc_uri) &&
          !relevantAvatarMxcsRef.current.has(mxc_uri)
        ) {
          return;
        }
        if (thumbnail.kind === "failed" && avatarThumbnailFailureIsRetryable(thumbnail)) {
          const attempts = avatarRetryCountsRef.current.get(mxc_uri) ?? 0;
          if (attempts < MAX_AVATAR_THUMBNAIL_ATTEMPTS) {
            requestedAvatarMxcsRef.current.delete(mxc_uri);
          }
        }
        emitDiagnosticLog("timeline.avatar", avatarThumbnailLogMessage(thumbnail));
        setAvatarThumbnails((current) => ({ ...current, [mxc_uri]: thumbnail }));
        return;
      }
      if (payload.kind !== "Timeline") {
        return;
      }
      recordTimelineEventReceived();
      const event = payload.event;

      if ("DisplayLabelsUpdated" in event || "DisplayPolicyUpdated" in event) {
        if (!isAppLevelStore) {
          setStore((current) => {
            const next = applyTimelineEvent(current, event);
            relevantAvatarMxcsRef.current = timelineAvatarMxcsForItems(
              getItems(next, timelineKeyRef.current),
              profileUsersRef.current
            );
            return next;
          });
        }
        return;
      }

      // Key filter: only this timeline's events.
      const eventKey =
        "InitialItems" in event
          ? event.InitialItems.key
          : "ItemsUpdated" in event
            ? event.ItemsUpdated.key
            : "PaginationStateChanged" in event
              ? event.PaginationStateChanged.key
              : "AnchorRestoreFinished" in event
                ? event.AnchorRestoreFinished.key
                : "SendCompleted" in event
                  ? event.SendCompleted.key
                  : "MediaUploadProgress" in event
                    ? event.MediaUploadProgress.key
                    : "MediaDownloadProgress" in event
                      ? event.MediaDownloadProgress.key
                      : "MediaDownloadCompleted" in event
                        ? event.MediaDownloadCompleted.key
                        : "MediaDownloadFailed" in event
                          ? event.MediaDownloadFailed.key
                          : "MessageForwarded" in event
                            ? event.MessageForwarded.key
                            : "MessageSourceLoaded" in event
                            ? event.MessageSourceLoaded.key
                              : "NavigationUpdated" in event
                                ? event.NavigationUpdated.key
                                : event.ResyncRequired.key;
      if (!timelineKeyEquals(eventKey, timelineKeyRef.current)) {
        recordTimelineKeyMismatch();
        return;
      }
      emitTimelineEventDiagnosticLog(event, eventKey, emitDiagnosticLog);
      if ("InitialItems" in event) {
        recordTimelineInitialItems(event.InitialItems.items.length);
        resetActiveMeasurementDeferral({ clearMountedIds: true });
      }
      if (
        "ItemsUpdated" in event &&
        timelineDiffsContainReset(event.ItemsUpdated.diffs)
      ) {
        resetActiveMeasurementDeferral({ clearMountedIds: true });
      }
      if (timelineEventCompletesBackfillRequest(event)) {
        backfillInFlightRef.current = false;
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
        roomScrollAnchorRestorePendingRef.current = false;
        viewportIntentRef.current = { kind: "free-scroll" };
        userScrollInputPendingRef.current = false;
        resetActiveMeasurementDeferral({ clearMountedIds: true });
        lastPersistedViewportAnchorSignatureRef.current = null;
        setNavigationSnapshot(null);
      }

      if ("MessageSourceLoaded" in event) {
        setMessageSource(event.MessageSourceLoaded.source);
        return;
      }

      if ("MessageForwarded" in event) {
        return;
      }

      if ("NavigationUpdated" in event) {
        setNavigationSnapshot(event.NavigationUpdated.snapshot);
        return;
      }

      if (
        "ItemsUpdated" in event &&
        timelineDiffsContainOwnOutgoingItem(event.ItemsUpdated.diffs, currentUserId)
      ) {
        setViewportIntentToLiveEdge();
        stickToBottomAfterMeasurementRef.current = true;
      }

      if (!isAppLevelStore) {
        setStore((current) => {
          const next = applyTimelineEvent(current, event);
          relevantAvatarMxcsRef.current = timelineAvatarMxcsForItems(
            getItems(next, timelineKeyRef.current),
            profileUsersRef.current
          );
          return next;
        });
      }
    });
    return unsubscribe;
  }, [
    currentUserId,
    emitDiagnosticLog,
    isAppLevelStore,
    resetActiveMeasurementDeferral,
    setViewportIntentToLiveEdge,
    timelineKeyHash,
    transport
  ]);

  useEffect(() => {
    void transport.ensureSubscribed?.(timelineKeyRef.current).catch(() => undefined);
  }, [timelineKeyHash, transport]);

  useEffect(
    () => () => {
      cancelPendingScrollFrame();
      resetActiveMeasurementDeferral({ clearMountedIds: true });
    },
    [cancelPendingScrollFrame, resetActiveMeasurementDeferral]
  );

  useLayoutEffect(() => {
    cancelPendingScrollFrame();
    const sessionViewport = timelineViewportSessionMemory.get(timelineKeyHash) ?? null;
    sessionRoomScrollAnchorRef.current =
      sessionViewport?.mode === "anchor" ? sessionViewport.anchor : null;
    anchorAsyncGenerationRef.current += 1;
    pendingAnchorRef.current = null;
    anchorRestorePendingRef.current = false;
    roomScrollAnchorRestorePendingRef.current = false;
    suppressScrollAnchorCaptureRef.current = false;
    restoredRoomScrollAnchorSignatureRef.current = null;
    viewportIntentRef.current =
      sessionViewport?.mode === "anchor" ? { kind: "free-scroll" } : { kind: "live-edge" };
    resetActiveMeasurementDeferral({ clearMountedIds: true });
    userScrollInputPendingRef.current = false;
    pendingScrollFrameUserInputRef.current = false;
    lastPersistedViewportAnchorSignatureRef.current = null;
  }, [cancelPendingScrollFrame, resetActiveMeasurementDeferral, timelineKeyHash]);

  useEffect(() => {
    const sessionViewport = timelineViewportSessionMemory.get(timelineKeyHash) ?? null;
    sessionRoomScrollAnchorRef.current =
      sessionViewport?.mode === "anchor" ? sessionViewport.anchor : null;
    setNavigationSnapshot(null);
    setViewportAtBottom(false);
    lastViewportObservationRef.current = null;
    readSignalEventRef.current = null;
    downloadedEventIdsRef.current = new Set();
    requestedImagePreviewEventIdsRef.current = new Set();
    relevantAvatarMxcsRef.current = new Set();
    requestedAvatarMxcsRef.current = new Set();
    avatarRetryCountsRef.current = new Map();
    emptyThreadBackfillRequestedRef.current = false;
    lastDiagnosticsEmissionRef.current = null;
    initialLiveEdgeScrollAppliedRef.current = null;
    stickToBottomAfterMeasurementRef.current = false;
    resetActiveMeasurementDeferral({ clearMountedIds: true });
    itemHeightByDomIdRef.current = new Map();
    roomScrollAnchorRestorePendingRef.current = false;
    suppressScrollAnchorCaptureRef.current = false;
    viewportIntentRef.current =
      sessionViewport?.mode === "anchor" ? { kind: "free-scroll" } : { kind: "live-edge" };
    userScrollInputPendingRef.current = false;
    pendingScrollFrameUserInputRef.current = false;
    lastPersistedViewportAnchorSignatureRef.current = null;
    restoredRoomScrollAnchorSignatureRef.current = null;
    if (viewportIntentResizeFrameRef.current !== null) {
      window.cancelAnimationFrame(viewportIntentResizeFrameRef.current);
      viewportIntentResizeFrameRef.current = null;
    }
    setMeasuredHeightVersion((current) => current + 1);
    backfillInFlightRef.current = false;
  }, [resetActiveMeasurementDeferral, timelineKeyHash]);

  useEffect(
    () => () => {
      anchorAsyncGenerationRef.current += 1;
      anchorRestorePendingRef.current = false;
      roomScrollAnchorRestorePendingRef.current = false;
      suppressScrollAnchorCaptureRef.current = false;
      viewportIntentRef.current = { kind: "free-scroll" };
      resetActiveMeasurementDeferral({ clearMountedIds: true });
      userScrollInputPendingRef.current = false;
      pendingScrollFrameUserInputRef.current = false;
      lastPersistedViewportAnchorSignatureRef.current = null;
      if (viewportIntentResizeFrameRef.current !== null) {
        window.cancelAnimationFrame(viewportIntentResizeFrameRef.current);
        viewportIntentResizeFrameRef.current = null;
      }
    },
    [resetActiveMeasurementDeferral]
  );

  const items = getItems(store, timelineKey);
  useEffect(() => {
    relevantAvatarMxcsRef.current = timelineAvatarMxcsForItems(items, profileUsers);
  }, [items, profileUsers]);
  const visibleItems = useMemo(() => items.filter((item) => !item.is_hidden), [items]);
  const visibleItemDomIds = useMemo(
    () => new Set(visibleItems.map((item) => timelineItemDomId(item.id))),
    [visibleItems]
  );
  visibleItemDomIdsRef.current = visibleItemDomIds;
  const timelineHeightModel = useMemo(
    () =>
      buildTimelineHeightModel(
        visibleItems,
        itemHeightByDomIdRef.current,
        virtualItemHeight
      ),
    [measuredHeightVersion, visibleItems, virtualItemHeight]
  );
  useLayoutEffect(() => {
    rangeModelEpochRef.current += 1;
  }, [timelineHeightModel, visibleItems]);
  const commitVirtualRangeForMetrics = useCallback(
    (metrics: TimelineViewportMetrics) => {
      const next = calculateTimelineVirtualRange({
        visibleItemsLength: visibleItems.length,
        metrics,
        model: timelineHeightModel
      });
      if (virtualRangeEquals(virtualRangeRef.current, next)) {
        return next;
      }
      virtualRangeRef.current = next;
      updateScrollDiagnostics(recordTimelineScrollRangeCommit);
      setVirtualRange(next);
      return next;
    },
    [timelineHeightModel, updateScrollDiagnostics, visibleItems.length]
  );
  const updateViewportMetrics = useCallback(() => {
    const metrics = readViewportMetrics();
    commitVirtualRangeForMetrics(metrics);
  }, [commitVirtualRangeForMetrics, readViewportMetrics]);
  const virtualWindow = useMemo<TimelineVirtualWindow>(() => {
    const range =
      visibleItems.length <= TIMELINE_VIRTUALIZATION_THRESHOLD
        ? {
            virtualized: false,
            startIndex: 0,
            endIndex: visibleItems.length,
            paddingTop: 0,
            paddingBottom: 0
          }
        : virtualRange;

    return {
      ...range,
      items: visibleItems.slice(range.startIndex, range.endIndex)
    };
  }, [virtualRange, visibleItems]);
  useLayoutEffect(() => {
    commitVirtualRangeForMetrics(readViewportMetrics());
  }, [commitVirtualRangeForMetrics, readViewportMetrics]);
  useEffect(() => {
    const intentKind: TimelineViewportIntentKind =
      viewportIntentRef.current.kind === "live-edge" ? "liveEdge" : "freeScroll";
    updateScrollDiagnostics((current) =>
      recordTimelineScrollFrame(current, {
        scrollActivity: "idle",
        viewportIntent: intentKind,
        userInputPending: userScrollInputPendingRef.current,
        virtualized: virtualWindow.virtualized,
        startIndex: virtualWindow.startIndex,
        endIndex: virtualWindow.endIndex,
        paddingTop: virtualWindow.paddingTop,
        paddingBottom: virtualWindow.paddingBottom,
        changedMeasuredRowCount: 0,
        heightDeltaAboveViewportPx: 0,
        heightDeltaInsideViewportPx: 0,
        heightDeltaBelowViewportPx: 0,
        anchorTopDeltaPx: 0
      })
    );
  }, [
    updateScrollDiagnostics,
    virtualWindow.endIndex,
    virtualWindow.paddingBottom,
    virtualWindow.paddingTop,
    virtualWindow.startIndex,
    virtualWindow.virtualized
  ]);
  const sideEffectItems =
    visibleItems.length > TIMELINE_VIRTUALIZATION_THRESHOLD ? virtualWindow.items : visibleItems;
  useEffect(() => {
    const avatarDiagnostics = timelineAvatarDiagnostics(
      visibleItems,
      profileUsers,
      avatarThumbnails
    );
    for (const item of items) {
      if ("Event" in item.id) {
        downloadedEventIdsRef.current.add(item.id.Event.event_id);
      }
    }
    const diagnostics = {
      visibleItems: visibleItems.length,
      downloadedItems: downloadedEventIdsRef.current.size,
      backfill: paginationStateDiagnosticLabel(getPaginationState(store, timelineKey, "Backward")),
      ...avatarDiagnostics,
      ...timelineRenderedAvatarDiagnostics(containerRef.current)
    };
    if (!onDiagnosticsChange) {
      lastDiagnosticsEmissionRef.current = null;
      return;
    }
    const diagnosticsSignature = `${timelineKeyHash}\u0000${JSON.stringify(diagnostics)}`;
    const lastEmission = lastDiagnosticsEmissionRef.current;
    if (
      lastEmission?.callback === onDiagnosticsChange &&
      lastEmission.signature === diagnosticsSignature
    ) {
      return;
    }
    lastDiagnosticsEmissionRef.current = {
      callback: onDiagnosticsChange,
      signature: diagnosticsSignature
    };
    onDiagnosticsChange(diagnostics);
  }, [
    avatarThumbnails,
    items,
    onDiagnosticsChange,
    profileUsers,
    store,
    timelineKeyHash,
    visibleItems
  ]);
  useEffect(() => {
    // #116 perf gate: skip avatar downloads when disabled (default).
    if (!enableAvatarThumbnailDownloads) {
      return;
    }
    if (!transport.downloadAvatarThumbnail) {
      return;
    }
    for (const item of sideEffectItems) {
      const profileAvatar = item.sender ? profileUsers[item.sender]?.avatar : null;
      const avatar = item.sender_avatar ?? profileAvatar;
      if (!avatar) {
        continue;
      }
      const thumbnail = avatarThumbnails[avatar.mxc_uri] ?? avatar.thumbnail;
      if (avatarThumbnailRequestShouldBeSkipped(thumbnail)) {
        continue;
      }
      const attempts = avatarRetryCountsRef.current.get(avatar.mxc_uri) ?? 0;
      if (attempts >= MAX_AVATAR_THUMBNAIL_ATTEMPTS) {
        continue;
      }
      if (requestedAvatarMxcsRef.current.has(avatar.mxc_uri)) {
        continue;
      }
      requestedAvatarMxcsRef.current.add(avatar.mxc_uri);
      avatarRetryCountsRef.current.set(avatar.mxc_uri, attempts + 1);
      emitDiagnosticLog("timeline.avatar", "avatar thumbnail request queued");
      void transport.downloadAvatarThumbnail(avatar.mxc_uri).catch(() => {
        requestedAvatarMxcsRef.current.delete(avatar.mxc_uri);
        emitDiagnosticLog("timeline.avatar", "avatar thumbnail command failed");
      });
    }
  }, [
    avatarThumbnails,
    emitDiagnosticLog,
    enableAvatarThumbnailDownloads,
    profileUsers,
    sideEffectItems,
    transport
  ]);
  useEffect(() => {
    for (const item of sideEffectItems) {
      if (!item.media || item.media.kind !== "Image" || !("Event" in item.id)) {
        continue;
      }
      const eventId = item.id.Event.event_id;
      const downloadState = mediaDownloads[eventId];
      if (downloadState?.kind === "ready" || downloadState?.kind === "pending") {
        continue;
      }
      if (requestedImagePreviewEventIdsRef.current.has(eventId)) {
        continue;
      }
      requestedImagePreviewEventIdsRef.current.add(eventId);
      void transport.downloadMedia(roomId, eventId).catch(() => {
        requestedImagePreviewEventIdsRef.current.delete(eventId);
      });
    }
  }, [mediaDownloads, roomId, sideEffectItems, transport]);
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
  const latestReadableEventId = latestEventBackedItemId(items);
  const timelineKeyState = getKeyState(store, timelineKey);
  const timelineInitialized = Boolean(timelineKeyState && !timelineKeyState.awaitingResync);
  // Stable, render-visible timeline generation for this key. Bumps when the
  // store replaces the list for a new generation (InitialItems / resync), so
  // tests can poll a concrete attribute instead of sleeping. 0 is a valid
  // Core generation; use timelineInitialized to distinguish "not initialized".
  const generation = timelineKeyState?.generation ?? 0;
  const initialLiveEdgeScrollKey = timelineInitialized
    ? `${timelineKeyHash}:${generation}`
    : null;
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
  const onRequestRoomKey = useCallback(
    (targetRoomId: string, eventId: string) => {
      onDiagnosticLogEntry?.({
        timestampMs: Date.now(),
        source: "e2ee.room_key",
        message: `request keys room=${targetRoomId} event=${eventId}`
      });
      void transport.requestRoomKey(targetRoomId, eventId).catch((error) => {
        onDiagnosticLogEntry?.({
          timestampMs: Date.now(),
          source: "e2ee.room_key",
          message: `request keys failed room=${targetRoomId} event=${eventId} error=${String(error)}`
        });
      });
    },
    [onDiagnosticLogEntry, transport]
  );
  const onForwardMessage = useCallback(
    (targetRoomId: string, sourceEventId: string, destinationRoomId: string) => {
      void transport
        .forwardMessage(targetRoomId, sourceEventId, destinationRoomId)
        .catch(() => undefined);
    },
    [transport]
  );
  const onLoadLinkPreviews = useCallback(
    (targetRoomId: string, eventId: string) => {
      void transport.loadLinkPreviews?.(targetRoomId, eventId)?.catch(() => undefined);
    },
    [transport]
  );
  const onHideLinkPreview = useCallback(
    (targetRoomId: string, eventId: string) => {
      void transport.hideLinkPreview?.(targetRoomId, eventId)?.catch(() => undefined);
    },
    [transport]
  );
  const onCopyText = useCallback((value: string) => {
    void writeClipboardText(value).catch(() => undefined);
  }, []);
  const openAliasDialog = useCallback((target: TimelineAliasTarget) => {
    setAliasTarget(target);
    setAliasDraft(aliasTargetIsActive(target) ? target.displayLabel : "");
  }, []);
  const closeAliasDialog = useCallback(() => {
    setAliasTarget(null);
    setAliasDraft("");
  }, []);
  const submitAliasDialog = useCallback(
    (event: FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      if (!aliasTarget || !onSetLocalUserAlias) {
        return;
      }
      onSetLocalUserAlias(aliasTarget.userId, aliasDraft.trim() || null);
      closeAliasDialog();
    },
    [aliasDraft, aliasTarget, closeAliasDialog, onSetLocalUserAlias]
  );
  const effectiveForwardDestinations =
    forwardDestinations.length > 0
      ? forwardDestinations
      : [{ room_id: roomId, display_name: roomId }];
  const sendReadSignalsForEvent = useCallback(
    (eventId: string) => {
      const signalKey = `${roomId}\u0000${eventId}`;
      if (readSignalEventRef.current === signalKey) {
        return;
      }
      readSignalEventRef.current = signalKey;
      void transport.sendReadReceipt(roomId, eventId).catch(() => undefined);
      void transport.setFullyRead(roomId, eventId).catch(() => undefined);
    },
    [roomId, transport]
  );
  const reportViewportObservation = useCallback(() => {
    if (!transport.observeViewport || roomTimelineRoomId !== roomId) {
      return;
    }
    const container = containerRef.current;
    if (!container) {
      return;
    }
    const visible = visibleEventIds(container);
    if (!visible.firstVisibleEventId && !visible.lastVisibleEventId) {
      return;
    }
    const atBottom = isScrolledToBottom(container);
    setViewportAtBottom((current) => (current === atBottom ? current : atBottom));
    if (atBottom && latestReadableEventId) {
      sendReadSignalsForEvent(latestReadableEventId);
    }
    const signature = [
      roomId,
      visible.firstVisibleEventId ?? "",
      visible.lastVisibleEventId ?? "",
      atBottom ? "bottom" : "not-bottom"
    ].join("\u0000");
    if (lastViewportObservationRef.current === signature) {
      return;
    }
    lastViewportObservationRef.current = signature;
    void transport
      .observeViewport(
        roomId,
        visible.firstVisibleEventId,
        visible.lastVisibleEventId,
        atBottom
      )
      .catch(() => undefined);
  }, [
    latestReadableEventId,
    roomId,
    roomTimelineRoomId,
    sendReadSignalsForEvent,
    transport
  ]);

  useEffect(() => {
    const list = listRef.current;
    if (!list || typeof ResizeObserver === "undefined") {
      return;
    }

    const observer = new ResizeObserver(() => {
      if (viewportIntentRef.current.kind !== "live-edge") {
        return;
      }
      if (viewportIntentResizeFrameRef.current !== null) {
        window.cancelAnimationFrame(viewportIntentResizeFrameRef.current);
      }
      viewportIntentResizeFrameRef.current = window.requestAnimationFrame(() => {
        viewportIntentResizeFrameRef.current = null;
        const changed = applyViewportIntent();
        if (!changed) {
          return;
        }
        updateViewportMetrics();
        reportViewportObservation();
      });
    });

    observer.observe(list);
    return () => {
      observer.disconnect();
      if (viewportIntentResizeFrameRef.current !== null) {
        window.cancelAnimationFrame(viewportIntentResizeFrameRef.current);
        viewportIntentResizeFrameRef.current = null;
      }
    };
  }, [
    applyViewportIntent,
    reportViewportObservation,
    timelineKeyHash,
    updateViewportMetrics
  ]);

  useEffect(() => {
    if (!latestReadableEventId || roomTimelineRoomId !== roomId) {
      return;
    }
    const container = containerRef.current;
    if (!container || !viewportAtBottom || !isScrolledToBottom(container)) {
      return;
    }
    sendReadSignalsForEvent(latestReadableEventId);
  }, [
    latestReadableEventId,
    roomId,
    roomTimelineRoomId,
    sendReadSignalsForEvent,
    viewportAtBottom
  ]);

  // --- Anchor restoration: after React commits the prepend ---
  useLayoutEffect(() => {
    const container = containerRef.current;
    const activeRoomAnchor =
      roomTimelineRoomId === roomId ? sessionRoomScrollAnchorRef.current : null;
    const activeRoomAnchorSignature = activeRoomAnchor
      ? roomScrollAnchorSignature(roomId, activeRoomAnchor)
      : null;
    const roomAnchorAlreadyRestored =
      activeRoomAnchorSignature !== null &&
      restoredRoomScrollAnchorSignatureRef.current === activeRoomAnchorSignature;
    let roomAnchorRestored = false;
    if (
      timelineInitialized &&
      items.length > 0 &&
      activeRoomAnchor &&
      activeRoomAnchorSignature !== null &&
      restoredRoomScrollAnchorSignatureRef.current !== activeRoomAnchorSignature
    ) {
      const restoreActiveRoomAnchor = () => {
        if (!container) {
          return false;
        }
        const restored = restoreRoomScrollAnchor(container, activeRoomAnchor);
        if (restored) {
          restoredRoomScrollAnchorSignatureRef.current = activeRoomAnchorSignature;
          roomScrollAnchorRestorePendingRef.current = false;
          initialLiveEdgeScrollAppliedRef.current = initialLiveEdgeScrollKey;
          sessionRoomScrollAnchorRef.current = null;
        }
        return restored;
      };

      const anchorIsLive = timelineContainsEventId(items, activeRoomAnchor.event_id);
      if (anchorIsLive) {
        roomScrollAnchorRestorePendingRef.current = true;
        runWithScrollWriteReason("roomRestore", () => {
          roomAnchorRestored = restoreActiveRoomAnchor();
        });
        if (
          !roomAnchorRestored &&
          container &&
          virtualWindow.virtualized &&
          roomTimelineRoomId === roomId
        ) {
          const anchorIndex = visibleItems.findIndex(
            (item) =>
              "Event" in item.id && item.id.Event.event_id === activeRoomAnchor.event_id
          );
          if (anchorIndex >= 0) {
            const anchorTop = timelineHeightModel.offsets[anchorIndex] ?? 0;
            const anchorHeight = timelineItemHeightAtIndex(timelineHeightModel, anchorIndex);
            const targetScrollTop =
              (activeRoomAnchor.edge ?? "top") === "bottom"
                ? viewportMetricsRef.current.listOffsetTop +
                  anchorTop +
                  anchorHeight -
                  container.clientHeight -
                  activeRoomAnchor.offset_px
                : viewportMetricsRef.current.listOffsetTop +
                  anchorTop -
                  activeRoomAnchor.offset_px;
            runWithScrollWriteReason("roomRestore", () => {
              container.scrollTop = Math.max(0, targetScrollTop);
            });
            requestAnimationFrame(() => {
              let roomAnchorRestoredInFrame = false;
              runWithScrollWriteReason("roomRestore", () => {
                roomAnchorRestoredInFrame = restoreActiveRoomAnchor();
                if (roomAnchorRestoredInFrame) {
                  updateViewportMetrics();
                  reportViewportObservation();
                }
              });
              if (!roomAnchorRestoredInFrame) {
                roomScrollAnchorRestorePendingRef.current = false;
              }
              updateViewportMetrics();
              reportViewportObservation();
            });
            return;
          }
        }
        if (!roomAnchorRestored && !roomAnchorAlreadyRestored) {
          roomScrollAnchorRestorePendingRef.current = false;
        }
      } else if (container) {
        sessionRoomScrollAnchorRef.current = null;
        setViewportIntentToLiveEdge();
        restoredRoomScrollAnchorSignatureRef.current = activeRoomAnchorSignature;
        initialLiveEdgeScrollAppliedRef.current = initialLiveEdgeScrollKey;
        runWithScrollWriteReason("roomRestore", () => {
          scrollContainerToBottom(container);
        });
      }
    } else if (
      timelineInitialized &&
      items.length > 0 &&
      initialLiveEdgeScrollKey !== null &&
      initialLiveEdgeScrollAppliedRef.current !== initialLiveEdgeScrollKey &&
      !roomAnchorAlreadyRestored &&
      !roomScrollAnchorRestorePendingRef.current
    ) {
      if (container) {
        setViewportIntentToLiveEdge();
        runWithScrollWriteReason("liveEdge", () => {
          scrollContainerToBottom(container);
        });
        // Only mark the live-edge scroll as applied once the content actually
        // overflows the viewport. If the first batch is too short to scroll,
        // leaving the ref unset lets later PushBack/PushFront growth re-enter
        // this branch and snap to the latest message on first launch.
        if (
          container.scrollHeight >
          container.clientHeight + SCROLL_EDGE_TOLERANCE_PX
        ) {
          initialLiveEdgeScrollAppliedRef.current = initialLiveEdgeScrollKey;
          // The DOM scrollHeight used above may be an underestimate before
          // variable-height rows are measured. Force a follow-up snap to the
          // new bottom once the measurement effect has actual heights.
          stickToBottomAfterMeasurementRef.current = true;
        }
      }
    }
    if (stickToBottomAfterMeasurementRef.current) {
      if (container) {
        runWithScrollWriteReason("liveEdge", () => {
          scrollContainerToBottom(container);
        });
      }
      stickToBottomAfterMeasurementRef.current = false;
    }
    if (anchorRestorePendingRef.current) {
      const anchor = pendingAnchorRef.current;
      let restored = false;
      if (container && anchor) {
        runWithScrollWriteReason("backfillCompensation", () => {
          restored = restoreAnchor(container, anchor);
        });
      }
      if (!restored && container && anchor && virtualWindow.virtualized) {
        const anchorIndex = visibleItems.findIndex(
          (item) => timelineItemDomId(item.id) === anchor.itemId
        );
        if (anchorIndex >= 0) {
          runWithScrollWriteReason("backfillCompensation", () => {
            container.scrollTop = Math.max(
              0,
              viewportMetricsRef.current.listOffsetTop +
                (timelineHeightModel.offsets[anchorIndex] ?? 0) -
                anchor.offsetTop
            );
          });
          requestAnimationFrame(() => {
            runWithScrollWriteReason("backfillCompensation", () => {
              restoreAnchor(container, anchor);
            });
            updateViewportMetrics();
            reportViewportObservation();
          });
        }
      }
      pendingAnchorRef.current = null;
      // Restoration complete: the next automatic fill request is allowed again.
      anchorRestorePendingRef.current = false;
    }
    if (
      container &&
      viewportIntentRef.current.kind === "live-edge" &&
      !anchorRestorePendingRef.current &&
      !roomScrollAnchorRestorePendingRef.current
    ) {
      applyViewportIntent();
    }
    updateViewportMetrics();
    reportViewportObservation();
  }, [
    applyViewportIntent,
    generation,
    roomId,
    roomTimelineRoomId,
    initialLiveEdgeScrollKey,
    items,
    reportViewportObservation,
    timelineHeightModel,
    timelineInitialized,
    updateViewportMetrics,
    virtualWindow.virtualized,
    visibleItems,
    runWithScrollWriteReason,
    setViewportIntentToLiveEdge,
    timelineKeyHash
  ]);

  useLayoutEffect(() => {
    if (!virtualWindow.virtualized) {
      mountedItemDomIdsRef.current = new Set();
      return;
    }
    const list = listRef.current;
    if (!list) {
      mountedItemDomIdsRef.current = new Set();
      return;
    }
    const nextHeights = new Map(itemHeightByDomIdRef.current);
    const visibleDomIds = visibleItemDomIdsRef.current;
    let changed = false;
    for (const domId of nextHeights.keys()) {
      if (!visibleDomIds.has(domId)) {
        nextHeights.delete(domId);
        changed = true;
      }
    }
    const nodes = Array.from(list.querySelectorAll<HTMLElement>(".timeline-item-frame"));
    if (nodes.length === 0) {
      mountedItemDomIdsRef.current = new Set();
      return;
    }
    const mountedDomIds = new Set<string>();
    for (const node of nodes) {
      const domId =
        node.dataset["frameItemId"] ??
        node.querySelector<HTMLElement>("[data-item-id]")?.dataset["itemId"];
      if (!domId) {
        continue;
      }
      mountedDomIds.add(domId);
      const height = measuredItemHeight(node.getBoundingClientRect().height);
      if (Math.abs((nextHeights.get(domId) ?? 0) - height) <= 1) {
        continue;
      }
      nextHeights.set(domId, height);
      changed = true;
    }
    mountedItemDomIdsRef.current = mountedDomIds;
    if (!changed) {
      return;
    }
    if (scrollActivityRef.current === "active") {
      for (const [domId, height] of nextHeights) {
        if (Math.abs((itemHeightByDomIdRef.current.get(domId) ?? 0) - height) > 1) {
          pendingMeasuredHeightsRef.current.set(domId, {
            height,
            epoch: measurementEpochRef.current
          });
        }
      }
      const userInputPending =
        pendingScrollFrameUserInputRef.current || userScrollInputPendingRef.current;
      updateScrollDiagnostics((current) =>
        recordTimelineScrollFrame(current, {
          scrollActivity: "active",
          viewportIntent:
            viewportIntentRef.current.kind === "live-edge" ? "liveEdge" : "freeScroll",
          userInputPending,
          virtualized: virtualWindow.virtualized,
          startIndex: virtualWindow.startIndex,
          endIndex: virtualWindow.endIndex,
          paddingTop: virtualWindow.paddingTop,
          paddingBottom: virtualWindow.paddingBottom,
          changedMeasuredRowCount: pendingMeasuredHeightsRef.current.size,
          heightDeltaAboveViewportPx: 0,
          heightDeltaInsideViewportPx: 0,
          heightDeltaBelowViewportPx: 0,
          anchorTopDeltaPx: 0
        })
      );
      return;
    }
    const container = containerRef.current;
    const measuredAtBottom = Boolean(container && isScrolledToBottom(container));
    stickToBottomAfterMeasurementRef.current = measuredAtBottom;
    if (measuredAtBottom) {
      setViewportIntentToLiveEdge();
    }
    itemHeightByDomIdRef.current = nextHeights;
    updateScrollDiagnostics((current) =>
      recordTimelineScrollHeightCommit(current, "initial")
    );
    setMeasuredHeightVersion((current) => current + 1);
  }, [
    setViewportIntentToLiveEdge,
    updateScrollDiagnostics,
    virtualWindow.endIndex,
    virtualWindow.paddingBottom,
    virtualWindow.paddingTop,
    virtualWindow.startIndex,
    virtualWindow.virtualized,
    visibleItems
  ]);

  // --- Automatic backfill on scroll near the top ---
  const maybeAutoBackfill = useCallback(() => {
    if (suppressPaginationUi) {
      return;
    }
    const container = containerRef.current;
    if (!container) {
      return;
    }
    const desiredBackfillThreshold = autoLoadOlderMessages
      ? Math.max(AUTO_BACKFILL_THRESHOLD_PX, virtualItemHeight * AUTO_BACKFILL_PREFETCH_ITEMS)
      : 0;
    const maxScrollTop = container.scrollHeight - container.clientHeight;
    // Only prefetch when the viewport is actually near the top edge. If the
    // loaded timeline is shorter than the desired prefetch window, fall back
    // to the near-top threshold so that a small scroll up from the live edge
    // does not immediately fire a backfill request (and the prepend/anchor
    // restoration that can follow).
    const backfillThreshold = autoLoadOlderMessages
      ? (maxScrollTop > desiredBackfillThreshold
          ? desiredBackfillThreshold
          : AUTO_BACKFILL_THRESHOLD_PX)
      : 0;
    if (container.scrollTop > backfillThreshold) {
      return;
    }
    // Block while: a previous diff's anchor restoration is pending, a
    // request is already in flight, or pagination is Paginating/EndReached.
    if (
      anchorRestorePendingRef.current ||
      roomScrollAnchorRestorePendingRef.current ||
      backfillInFlightRef.current
    ) {
      return;
    }
    if (shouldSuppressAutoBackfill(store, timelineKeyRef.current)) {
      return;
    }
    backfillInFlightRef.current = true;
    void transport
      .paginateBackwards(timelineKeyRef.current)
      .catch(() => {
        backfillInFlightRef.current = false;
      });
  }, [store, transport, suppressPaginationUi, autoLoadOlderMessages, virtualItemHeight]);
  const onTimelineScroll = useCallback(() => {
    const container = containerRef.current;
    const sig = programmaticScrollSignatureRef.current;
    const isProgrammaticEcho =
      sig !== null &&
      container !== null &&
      Math.abs(container.scrollTop - sig.scrollTop) <= SCROLL_EDGE_TOLERANCE_PX &&
      Math.abs(container.scrollHeight - sig.scrollHeight) <= SCROLL_EDGE_TOLERANCE_PX;
    if (!isProgrammaticEcho) {
      markScrollActivityActive();
    }
    const isUserDrivenScroll = userScrollInputPendingRef.current && !isProgrammaticEcho;
    pendingScrollFrameUserInputRef.current =
      pendingScrollFrameUserInputRef.current || isUserDrivenScroll;
    if (!isProgrammaticEcho && container) {
      const atBottom = isScrolledToBottom(container);
      if (isUserDrivenScroll) {
        if (atBottom) {
          setViewportIntentToLiveEdge();
        } else {
          releaseViewportIntent();
          const captured = captureRoomScrollAnchor(container);
          if (captured) {
            timelineViewportSessionMemory.set(timelineKeyHash, {
              mode: "anchor",
              anchor: {
                ...captured,
                updated_at_ms: Date.now()
              }
            });
          }
        }
        userScrollInputPendingRef.current = false;
      }
    }
    if (pendingScrollFrameRef.current === null) {
      const frameTimelineKeyHash = timelineKeyHash;
      const frameRangeModelEpoch = rangeModelEpochRef.current;
      pendingScrollFrameRef.current = window.requestAnimationFrame(() => {
        pendingScrollFrameRef.current = null;
        const userInputPending = pendingScrollFrameUserInputRef.current;
        pendingScrollFrameUserInputRef.current = false;
        if (
          timelineKeyHashRef.current !== frameTimelineKeyHash ||
          rangeModelEpochRef.current !== frameRangeModelEpoch
        ) {
          return;
        }
        const metrics = readViewportMetrics();
        const nextRange = commitVirtualRangeForMetrics(metrics);
        updateScrollDiagnostics((current) =>
          recordTimelineScrollFrame(current, {
            scrollActivity: "active",
            viewportIntent:
              viewportIntentRef.current.kind === "live-edge" ? "liveEdge" : "freeScroll",
            userInputPending,
            virtualized: nextRange.virtualized,
            startIndex: nextRange.startIndex,
            endIndex: nextRange.endIndex,
            paddingTop: nextRange.paddingTop,
            paddingBottom: nextRange.paddingBottom,
            changedMeasuredRowCount: 0,
            heightDeltaAboveViewportPx: 0,
            heightDeltaInsideViewportPx: 0,
            heightDeltaBelowViewportPx: 0,
            anchorTopDeltaPx: 0
          })
        );
      });
    }
    if (!isProgrammaticEcho) {
      reportViewportObservation();
      maybeAutoBackfill();
      persistViewportAnchor();
    }
  }, [
    setViewportIntentToLiveEdge,
    markScrollActivityActive,
    maybeAutoBackfill,
    persistViewportAnchor,
    readViewportMetrics,
    reportViewportObservation,
    releaseViewportIntent,
    timelineKeyHash,
    commitVirtualRangeForMetrics,
    updateScrollDiagnostics
  ]);
  const onTimelinePointerDown = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      if (event.target === event.currentTarget) {
        markUserScrollInput();
      }
    },
    [markUserScrollInput]
  );
  const onTimelineKeyDown = useCallback(
    (event: KeyboardEvent<HTMLDivElement>) => {
      if (timelineKeyShouldReleaseViewportIntent(event)) {
        markUserScrollInput();
      }
    },
    [markUserScrollInput]
  );
  useEffect(() => {
    if (!("Thread" in timelineKey.kind)) {
      return;
    }
    if (
      !timelineInitialized ||
      items.length > 0 ||
      suppressPaginationUi ||
      isPaginating ||
      endReached ||
      emptyThreadBackfillRequestedRef.current ||
      backfillInFlightRef.current
    ) {
      return;
    }
    emptyThreadBackfillRequestedRef.current = true;
    backfillInFlightRef.current = true;
    void transport
      .paginateBackwards(timelineKeyRef.current)
      .catch(() => {
        backfillInFlightRef.current = false;
      });
  }, [
    endReached,
    isPaginating,
    items.length,
    suppressPaginationUi,
    timelineKey.kind,
    timelineKeyHash,
    timelineInitialized,
    transport
  ]);
  const jumpToEvent = useCallback(
    (eventId: string) => {
      releaseViewportIntent();
      const container = containerRef.current;
      const scrollMountedRowIntoView = () => {
        const row = container?.querySelector<HTMLElement>(
          `[data-event-id="${cssEscape(eventId)}"]`
        );
        row?.scrollIntoView({ block: "center", inline: "nearest" });
      };
      const row = container?.querySelector<HTMLElement>(
        `[data-event-id="${cssEscape(eventId)}"]`
      );
      if (row) {
        runWithScrollWriteReason("jumpToEvent", () => {
          row.scrollIntoView({ block: "center", inline: "nearest" });
        });
        updateViewportMetrics();
        reportViewportObservation();
        return;
      }
      if (container && virtualWindow.virtualized) {
        const itemIndex = visibleItems.findIndex(
          (item) => "Event" in item.id && item.id.Event.event_id === eventId
        );
        if (itemIndex >= 0) {
          const itemTop = timelineHeightModel.offsets[itemIndex] ?? 0;
          const itemHeight = timelineItemHeightAtIndex(timelineHeightModel, itemIndex);
          runWithScrollWriteReason("jumpToEvent", () => {
            container.scrollTop = Math.max(
              0,
              viewportMetricsRef.current.listOffsetTop +
                itemTop +
                itemHeight / 2 -
                container.clientHeight / 2
            );
          });
          updateViewportMetrics();
          requestAnimationFrame(() => {
            runWithScrollWriteReason("jumpToEvent", () => {
              scrollMountedRowIntoView();
            });
            updateViewportMetrics();
            reportViewportObservation();
          });
          return;
        }
      }
      reportViewportObservation();
    },
    [
      releaseViewportIntent,
      runWithScrollWriteReason,
      reportViewportObservation,
      timelineHeightModel,
      updateViewportMetrics,
      virtualWindow.virtualized,
      visibleItems
    ]
  );
  const jumpToBottom = useCallback(() => {
    const container = containerRef.current;
    if (!container) {
      return;
    }
    const activeElement = document.activeElement;
    if (activeElement instanceof HTMLElement && container.contains(activeElement)) {
      activeElement.blur();
    }
    setViewportIntentToLiveEdge();
    runWithScrollWriteReason("jumpToBottom", () => {
      scrollContainerToBottom(container);
    });
    updateViewportMetrics();
    reportViewportObservation();
    requestAnimationFrame(() => {
      runWithScrollWriteReason("jumpToBottom", () => {
        scrollContainerToBottom(container);
      });
      updateViewportMetrics();
      reportViewportObservation();
    });
  }, [
    setViewportIntentToLiveEdge,
    reportViewportObservation,
    runWithScrollWriteReason,
    updateViewportMetrics
  ]);
  const canRenderRoomNavigation =
    roomTimelineRoomId === roomId;
  const firstUnreadEventId = navigationSnapshot?.first_unread_event_id ?? null;
  const firstUnreadCount = navigationSnapshot?.unread_event_count ?? 0;
  const canJumpToFirstUnread = Boolean(
    firstUnreadEventId &&
      firstUnreadCount > 0 &&
      navigationSnapshot?.unread_position !== "insideViewport" &&
      navigationSnapshot?.unread_position !== "none"
  );
  const canJumpToBottom = Boolean(
    navigationSnapshot?.can_jump_to_bottom &&
      (navigationSnapshot.newer_event_count > 0 || navigationSnapshot.unread_event_count > 0)
  );
  const unreadMarkerEventId = navigationSnapshot?.first_unread_event_id ?? null;
  const readMarkerDisplayEventId =
    navigationSnapshot?.read_marker_display_event_id ??
    navigationSnapshot?.read_marker_event_id ??
    roomSignals?.fully_read_event_id ??
    null;

  return (
    <div
      className="timeline-view"
      data-testid="timeline-view"
      data-end-reached={endReached || undefined}
      data-timeline-generation={generation}
      data-virtualized={virtualWindow.virtualized || undefined}
      data-total-items={visibleItems.length}
      data-rendered-items={virtualWindow.items.length}
      ref={containerRef}
      style={{ overflowY: "auto", height: "100%" }}
      onKeyDown={onTimelineKeyDown}
      onPointerDown={onTimelinePointerDown}
      onScroll={onTimelineScroll}
      onTouchMove={() => markUserScrollInput()}
      onWheel={(event) => markUserScrollInput({ keepLiveEdgeAtBottom: event.deltaY > 0 })}
    >
      {canRenderRoomNavigation ? (
        <div
          className="timeline-navigation-bar"
          style={{
            visibility:
              canJumpToFirstUnread || canJumpToBottom ? "visible" : "hidden"
          }}
          aria-hidden={!(canJumpToFirstUnread || canJumpToBottom)}
        >
          <div className="timeline-navigation-pills">
            {canJumpToFirstUnread && firstUnreadEventId ? (
              <button
                className="timeline-navigation-pill"
                type="button"
                onClick={() => jumpToEvent(firstUnreadEventId)}
              >
                <ArrowDown size={14} aria-hidden="true" />
                <span>{t("timeline.jumpToFirstUnread", { count: firstUnreadCount })}</span>
              </button>
            ) : null}
            {canJumpToBottom ? (
              <button
                className="timeline-navigation-pill"
                type="button"
                onClick={jumpToBottom}
              >
                <ArrowDown size={14} aria-hidden="true" />
                <span>
                  {t("timeline.jumpToBottom", {
                    count: navigationSnapshot?.newer_event_count ?? 0
                  })}
                </span>
              </button>
            ) : null}
          </div>
        </div>
      ) : null}
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
      <div
        className="timeline-item-list"
        ref={(element) => {
          listRef.current = element;
          listRefCallback?.(element);
        }}
      >
        {virtualWindow.virtualized ? (
          <div
            className="timeline-virtual-spacer"
            aria-hidden="true"
            style={{ blockSize: virtualWindow.paddingTop }}
          />
        ) : null}
        {virtualWindow.items.map((item) => {
          const itemDomId = timelineItemDomId(item.id);
          const eventId = "Event" in item.id ? item.id.Event.event_id : null;
          const isUnreadMarker = Boolean(eventId && unreadMarkerEventId === eventId);
          const isReadMarker = Boolean(
            eventId && readMarkerDisplayEventId === eventId && !unreadMarkerEventId
          );
          return (
            <div
              className="timeline-item-frame"
              key={itemDomId}
              data-frame-item-id={itemDomId}
            >
              {isUnreadMarker ? (
                <div className="read-marker" role="separator" aria-label={t("timeline.unreadMarker")}>
                  <span>{t("timeline.unreadMarker")}</span>
                </div>
              ) : null}
              <TimelineItemRow
                item={item}
                roomId={roomId}
                codeBlockWrap={codeBlockWrap}
                searchQuery={searchQuery}
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
                onRequestRoomKey={onRequestRoomKey}
                onForwardMessage={onForwardMessage}
                onLoadLinkPreviews={onLoadLinkPreviews}
                onHideLinkPreview={onHideLinkPreview}
                onCopyText={onCopyText}
                onOpenAliasDialog={onSetLocalUserAlias ? openAliasDialog : undefined}
                forwardDestinations={effectiveForwardDestinations}
                onRetrySend={onRetrySend}
                onCancelSend={onCancelSend}
                presence={item.sender ? liveSignals?.presence[item.sender] : undefined}
                profile={item.sender ? profileUsers[item.sender] : undefined}
                avatarThumbnails={avatarThumbnails}
                currentUserId={currentUserId}
                ignoredUserIds={ignoredUserIds}
                onOpenContextMenu={onOpenContextMenu}
                mentionProfileUsers={profileUsers}
                mediaDownload={eventId ? mediaDownloads[eventId] : undefined}
                receipts={eventId ? roomSignals?.receipts_by_event[eventId]?.readers ?? [] : []}
                receiptTotalCount={
                  eventId ? roomSignals?.receipts_by_event[eventId]?.total_count ?? 0 : 0
                }
                receiptOverflowCount={
                  eventId ? roomSignals?.receipts_by_event[eventId]?.overflow_count ?? 0 : 0
                }
              />
              {isReadMarker ? (
                <div className="read-marker" role="separator" aria-label={t("timeline.readMarker")}>
                  <span>{t("timeline.readMarker")}</span>
                </div>
              ) : null}
            </div>
          );
        })}
        {virtualWindow.virtualized ? (
          <div
            className="timeline-virtual-spacer"
            aria-hidden="true"
            style={{ blockSize: virtualWindow.paddingBottom }}
          />
        ) : null}
      </div>
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
      {aliasTarget ? (
        <div className="dialog-overlay" role="presentation" onMouseDown={closeAliasDialog}>
          <form
            className="dialog-box timeline-alias-dialog"
            aria-label={t("room.aliasDialogTitle", { name: aliasTarget.displayLabel })}
            onMouseDown={(event) => event.stopPropagation()}
            onSubmit={submitAliasDialog}
          >
            <h3 className="dialog-title">
              {t("room.aliasDialogTitle", { name: aliasTarget.displayLabel })}
            </h3>
            {aliasTargetIsActive(aliasTarget) ? (
              <p className="room-member-original-context" dir="auto">
                {t("room.memberOriginalName", {
                  name: aliasTarget.originalDisplayLabel
                })}
              </p>
            ) : null}
            <input
              className="dialog-input"
              aria-label={t("room.aliasInput")}
              value={aliasDraft}
              onChange={(event) => setAliasDraft(event.currentTarget.value)}
              autoFocus
            />
            <div className="dialog-actions">
              <button className="dialog-button" type="button" onClick={closeAliasDialog}>
                {t("action.cancel")}
              </button>
              <button className="dialog-button is-primary" type="submit">
                {t("room.saveAlias")}
              </button>
            </div>
          </form>
        </div>
      ) : null}
    </div>
  );
});

export function TimelineItemRow({
  item,
  roomId,
  codeBlockWrap = true,
  searchQuery = "",
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
  onRequestRoomKey = () => undefined,
  onForwardMessage = () => undefined,
  onLoadLinkPreviews = () => undefined,
  onHideLinkPreview = () => undefined,
  onCopyText = () => undefined,
  onOpenAliasDialog,
  forwardDestinations = [],
  onRetrySend = ignoreSendQueueAction,
  onCancelSend = ignoreSendQueueAction,
  presence,
  profile,
  avatarThumbnails = {},
  mentionProfileUsers = {},
  receipts = [],
  receiptTotalCount = receipts.length,
  receiptOverflowCount = 0,
  currentUserId,
  ignoredUserIds = [],
  onOpenContextMenu,
  mediaDownload
}: {
  item: TimelineItem;
  roomId: string;
  codeBlockWrap?: boolean;
  searchQuery?: string;
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
  onRequestRoomKey?: TimelineRowActionHandlers["onRequestRoomKey"];
  onForwardMessage?: TimelineRowActionHandlers["onForwardMessage"];
  onLoadLinkPreviews?: TimelineRowActionHandlers["onLoadLinkPreviews"];
  onHideLinkPreview?: TimelineRowActionHandlers["onHideLinkPreview"];
  onCopyText?: TimelineRowActionHandlers["onCopyText"];
  onOpenAliasDialog?: (target: TimelineAliasTarget) => void;
  forwardDestinations?: readonly TimelineForwardDestination[];
  onRetrySend?: TimelineRowActionHandlers["onRetrySend"];
  onCancelSend?: TimelineRowActionHandlers["onCancelSend"];
  presence?: PresenceKind;
  profile?: UserProfile;
  avatarThumbnails?: Record<string, AvatarThumbnailState>;
  mentionProfileUsers?: Record<string, UserProfile>;
  receipts?: LiveReadReceipt[];
  receiptTotalCount?: number;
  receiptOverflowCount?: number;
  currentUserId?: string;
  ignoredUserIds?: string[];
  onOpenContextMenu?: (
    event: MouseEvent<HTMLElement>,
    target: {
      kind: "message";
      message: { sender: string; room_id: string; event_id: string; body: string };
    },
    items: ContextMenuItem[]
  ) => void;
  mediaDownload?: TimelineMediaDownloadState;
}) {
  const domId = timelineItemDomId(item.id);
  const syntheticId = "Synthetic" in item.id ? item.id.Synthetic.synthetic_id : null;
  const dateDividerTimestampMs = syntheticDateDividerTimestampMs(syntheticId, item.timestamp_ms);
  if (dateDividerTimestampMs !== null) {
    return (
      <div className="read-marker timeline-date-divider" role="separator">
        <span>{formatDateDividerLabel(dateDividerTimestampMs)}</span>
      </div>
    );
  }
  if (syntheticId !== null) {
    return null;
  }
  const transactionId = "Transaction" in item.id ? item.id.Transaction.transaction_id : null;
  const eventId = "Event" in item.id ? item.id.Event.event_id : null;
  const isRedacted = item.is_redacted;
  const sendState = item.send_state ?? null;
  const sendStateKind = sendState?.kind ?? null;
  const messageKind = item.message_kind ?? "text";
  const [isEditing, setEditing] = useState(false);
  const [editDraft, setEditDraft] = useState(item.body ?? "");
  const [isReactionPickerOpen, setReactionPickerOpen] = useState(false);
  const [isActionMenuOpen, setActionMenuOpen] = useState(false);
  const [isForwardMenuOpen, setForwardMenuOpen] = useState(false);
  const [actionMenuPlacement, setActionMenuPlacement] = useState<"above" | "below">("above");
  const [revealedSpoilers, setRevealedSpoilers] = useState<ReadonlySet<string>>(
    () => new Set()
  );
  const reactionControlRef = useRef<HTMLDivElement>(null);
  const reactionTriggerRef = useRef<HTMLButtonElement>(null);
  const firstReactionRef = useRef<HTMLButtonElement>(null);
  const actionMenuControlRef = useRef<HTMLDivElement>(null);
  const actionMenuTriggerRef = useRef<HTMLButtonElement>(null);
  const firstActionMenuItemRef = useRef<HTMLButtonElement>(null);
  const editTextareaRef = useRef<HTMLTextAreaElement>(null);
  const editImeCompositionActiveRef = useRef(false);
  const editMacKillRingRef = useRef<string>("");
  const requestedLinkPreviewsRef = useRef<Set<string>>(new Set());

  useEffect(() => {
    if (!eventId || !item.link_previews?.some((preview) => preview.state === "pending")) {
      return;
    }
    if (requestedLinkPreviewsRef.current.has(eventId)) {
      return;
    }
    requestedLinkPreviewsRef.current.add(eventId);
    onLoadLinkPreviews(roomId, eventId);
  }, [eventId, item.link_previews, onLoadLinkPreviews, roomId]);

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

  const revealSpoiler = useCallback((spoilerKey: string) => {
    setRevealedSpoilers((current) => {
      if (current.has(spoilerKey)) {
        return current;
      }
      const next = new Set(current);
      next.add(spoilerKey);
      return next;
    });
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
      if (timelineEditImeShouldHandleKeyEvent(event, editImeCompositionActiveRef.current)) {
        return;
      }
      // macOS native Emacs text-editing bindings (Ctrl+F/B/P/N/K/Y).
      // Must not fire during IME composition.
      if (IS_MAC_PLATFORM && !event.nativeEvent.isComposing && !editImeCompositionActiveRef.current) {
        const emacsAction = macEmacsActionFromEvent(event);
        if (emacsAction !== null) {
          event.preventDefault();
          const ta = event.currentTarget;
          const effect = applyMacEmacsAction(
            emacsAction,
            editDraft,
            ta.selectionStart,
            ta.selectionEnd,
            editMacKillRingRef.current
          );
          if (effect !== null) {
            if (effect.newKillRing !== undefined) {
              editMacKillRingRef.current = effect.newKillRing;
            }
            if (effect.newValue !== undefined) {
              setEditDraft(effect.newValue);
            }
            const pos = effect.newSelectionPos;
            requestAnimationFrame(() => ta.setSelectionRange(pos, pos));
          }
          return;
        }
      }
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
    const control = actionMenuControlRef.current;
    if (control) {
      const controlRect = control.getBoundingClientRect();
      const panelTop =
        control.closest<HTMLElement>(".main-pane")?.getBoundingClientRect().top ?? 0;
      const availableAbove = controlRect.top - panelTop;
      setActionMenuPlacement(availableAbove < 180 ? "below" : "above");
    }
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
  const requestRoomKey = useCallback(() => {
    if (!eventId || !item.unable_to_decrypt?.can_request_keys) {
      return;
    }
    onRequestRoomKey(roomId, eventId);
  }, [eventId, item.unable_to_decrypt?.can_request_keys, onRequestRoomKey, roomId]);
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
  const canRequestRoomKey = Boolean(eventId && item.unable_to_decrypt?.can_request_keys);
  const canForward = Boolean(eventId && item.actions?.can_forward);
  const canSetSenderAlias = Boolean(eventId && item.sender && onOpenAliasDialog);
  const canShowMessageActionMenu =
    canSetSenderAlias || canCopyMessage || canCopyPermalink || canViewSource || canForward;
  const canShowThreadSummary = Boolean(eventId && item.thread_summary);
  const canShowReactions = !isRedacted && !isEditing && item.reactions.length > 0;
  const senderAvatar = item.sender_avatar ?? profile?.avatar ?? null;
  const avatarUrl =
    thumbnailSourceUrl(senderAvatar ? avatarThumbnails[senderAvatar.mxc_uri] : null) ??
    thumbnailSourceUrl(senderAvatar?.thumbnail);
  const {
    displaySourceUrl: displayAvatarUrl,
    onImageError: onAvatarImageError,
    onImageLoad: onAvatarImageLoad
  } = useRecoverableImageSource(avatarUrl);
  const showAvatarImage = Boolean(displayAvatarUrl);
  const senderDisplayLabel = item.sender_label?.trim() || item.sender || "";
  const senderOriginalLabel =
    profile?.original_display_label.trim() || profile?.display_name?.trim() || "";
  const senderAliasTarget =
    item.sender && canSetSenderAlias
      ? {
          userId: item.sender,
          displayLabel: senderDisplayLabel || item.sender,
          originalDisplayLabel: senderOriginalLabel
        }
      : null;
  const threadSummaryText = item.thread_summary
    ? formatThreadSummary(
        item.thread_summary.reply_count,
        item.thread_summary.latest_sender_label?.trim() || item.thread_summary.latest_sender,
        item.thread_summary.latest_body_preview
      )
    : "";
  const receiptDetails = formatReceiptDetails(receipts, receiptOverflowCount);
  const receiptLabel = t("timeline.readBy", { count: receiptTotalCount });
  const receiptAriaLabel =
    receiptDetails.length > 0 ? `${receiptLabel}: ${receiptDetails.join("; ")}` : receiptLabel;
  const receiptTitle = receiptDetails.join("\n");
  const reactionSenderLabelByUserId = reactionSenderLabelsByUserId(mentionProfileUsers);
  const spoilerState = { revealed: revealedSpoilers, reveal: revealSpoiler };
  const displayBody = localizedTimelineItemBody(item);
  const messageBodyClassName = [
    "message-body",
    item.formatted ? "message-formatted-body" : null,
    messageKind === "emote" ? "message-emote" : null,
    messageKind === "notice" ? "message-notice" : null
  ]
    .filter(Boolean)
    .join(" ");
  const messageBodyContent = item.formatted
    ? renderFormattedBody(
        item.formatted,
        item.link_ranges ?? [],
        codeBlockWrap,
        onCopyText,
        searchQuery,
        spoilerState
      )
    : renderPlainTextBody(
        displayBody,
        item.link_ranges ?? [],
        item.spoiler_spans,
        searchQuery,
        mentionProfileUsers,
        spoilerState
      );
  const emotePrefix =
    messageKind === "emote" ? (
      <span className="message-emote-prefix" dir="auto">
        * <span className="message-emote-sender">{senderDisplayLabel}</span>
      </span>
    ) : null;
  const replyQuoteContent =
    !isRedacted && item.reply_quote ? (
      <div className="reply-quote" data-reply-state={item.reply_quote.state}>
        <div className="reply-quote-sender" dir="auto">
          {item.reply_quote.sender_label?.trim() ||
            item.reply_quote.sender ||
            t("timeline.replyQuoteUnknownSender")}
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
        onCompositionStart={() => {
          editImeCompositionActiveRef.current = true;
        }}
        onCompositionEnd={() => {
          window.setTimeout(() => {
            editImeCompositionActiveRef.current = false;
          }, 0);
        }}
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
    <div
      className={messageBodyClassName}
      dir="auto"
      data-code-block-wrap={item.formatted && codeBlockWrap ? "true" : undefined}
    >
      {emotePrefix}
      {messageBodyContent}
    </div>
  );
  const mediaContent =
    !isRedacted && item.media ? (
      <TimelineMediaAttachment
        media={item.media}
        progress={mediaUploadProgress}
        downloadState={mediaDownload}
        canDownload={Boolean(eventId)}
        onDownload={submitDownloadMedia}
      />
    ) : null;
  function handleContextMenu(event: MouseEvent<HTMLElement>) {
    if (!onOpenContextMenu || !eventId || !item.sender) {
      return;
    }
    const items = contextMenuItems({
      kind: "message",
      canManage: currentUserId === item.sender,
      hasThread: item.thread_summary != null,
      senderUserId: item.sender,
      currentUserId: currentUserId ?? "",
      roomId,
      eventId,
      isIgnored: ignoredUserIds.includes(item.sender)
    });
    if (items.length === 0) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    onOpenContextMenu(event, {
      kind: "message",
      message: { sender: item.sender, room_id: roomId, event_id: eventId, body: item.body ?? "" }
    }, items);
  }

  return (
    <article
      className="message"
      data-item-id={domId}
      data-send-state={sendStateKind ?? undefined}
      data-event-id={eventId ?? undefined}
      data-redacted={isRedacted || undefined}
      data-reply={item.in_reply_to_event_id ? "true" : undefined}
      data-message-kind={messageKind}
      onContextMenu={handleContextMenu}
    >
      <div className="avatar" aria-hidden="true">
        {showAvatarImage ? (
          <img
            src={displayAvatarUrl ?? undefined}
            onError={onAvatarImageError}
            onLoad={onAvatarImageLoad}
          />
        ) : (
          senderInitials(senderDisplayLabel || item.sender)
        )}
      </div>
      <div className="message-main">
        <div className="message-heading">
          <MessageMeta
            senderDisplayLabel={senderDisplayLabel}
            timestampMs={item.timestamp_ms ?? null}
            isEdited={item.is_edited}
            isRedacted={isRedacted}
            sendStateKind={sendStateKind}
            presence={presence}
          />
        </div>
        {replyQuoteContent}
        {mediaContent ? (
          <>
            {mediaContent}
            {bodyContent}
          </>
        ) : (
          bodyContent
        )}
        {!isRedacted && eventId && item.link_previews && item.link_previews.length > 0 ? (
          <div className="link-preview-cards">
            {item.link_previews.map((preview) => {
              const previewUrl = toExternalHttpUrl(preview.url);
              return (
                <div key={preview.url} className="link-preview-card">
                  <a
                    className="link-preview-main"
                    href={previewUrl || undefined}
                    target="_blank"
                    rel="noopener noreferrer"
                    onClick={(event) => {
                      event.preventDefault();
                      if (previewUrl) {
                        void openExternalHttpUrl(previewUrl);
                      }
                    }}
                  >
                    {preview.image?.thumbnail && thumbnailSourceUrl(preview.image.thumbnail) ? (
                      <img
                        src={thumbnailSourceUrl(preview.image.thumbnail) ?? undefined}
                        alt={""}
                        className="link-preview-image"
                      />
                    ) : null}
                    <div className="link-preview-text">
                      {preview.title ? (
                        <div className="link-preview-title">{preview.title}</div>
                      ) : null}
                      {preview.description ? (
                        <div className="link-preview-description">{preview.description}</div>
                      ) : null}
                      <div className="link-preview-url">{preview.url}</div>
                    </div>
                  </a>
                  <button
                    type="button"
                    className="link-preview-hide"
                    onClick={() => onHideLinkPreview(roomId, eventId)}
                    aria-label={t("timeline.linkPreviewHide")}
                  >
                    ×
                  </button>
                </div>
              );
            })}
          </div>
        ) : null}
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
        {canRequestRoomKey ? (
          <div className="message-send-actions">
            <button className="message-send-action" type="button" onClick={requestRoomKey}>
              <KeyRound size={13} aria-hidden="true" />
              <span>{t("timeline.requestRoomKey")}</span>
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
        {canShowReactions || receiptTotalCount > 0 ? (
          <div className="message-status-row">
            {canShowReactions ? (
              <div className="message-reactions">
                {item.reactions.map((reaction, index) => {
                  const ariaLabel = t("timeline.reactionSummary", {
                    key: reaction.key,
                    count: reaction.count
                  });
                  const reactionTooltip = formatReactionTooltip(
                    reaction.key,
                    reaction.count,
                    reaction.sender_preview,
                    reactionSenderLabelByUserId
                  );
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
                        {reactionTooltip ? (
                          <span className="reaction-tooltip" role="tooltip" dir="auto">
                            {reactionTooltip}
                          </span>
                        ) : null}
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
                      {reactionTooltip ? (
                        <span className="reaction-tooltip" role="tooltip" dir="auto">
                          {reactionTooltip}
                        </span>
                      ) : null}
                    </button>
                  );
                })}
              </div>
            ) : null}
            {receiptTotalCount > 0 ? (
              <div
                className="message-receipts"
                aria-label={receiptAriaLabel}
                tabIndex={0}
                title={receiptTitle}
              >
                <span className="receipt-avatars" aria-hidden="true">
                  {receipts.map((receipt) => {
                    const sourceUrl = receiptAvatarSource(receipt);
                    return (
                      <span className="receipt-reader-avatar" key={receipt.user_id}>
                        {sourceUrl ? (
                          <img src={sourceUrl} alt={receiptDisplayName(receipt)} />
                        ) : (
                          <span dir="auto">{receiptInitials(receipt)}</span>
                        )}
                      </span>
                    );
                  })}
                  {receiptOverflowCount > 0 ? (
                    <span className="receipt-overflow">+{receiptOverflowCount}</span>
                  ) : null}
                </span>
                <span className="receipt-tooltip" role="tooltip">
                  {receiptDetails.map((detail, index) => (
                    <span key={`${detail}:${index}`} dir="auto">
                      {detail}
                    </span>
                  ))}
                </span>
              </div>
            ) : null}
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
                className={`message-action-menu is-${actionMenuPlacement}`}
                role="menu"
                aria-label={t("timeline.messageActions")}
                onKeyDown={(event) => {
                  if (event.key === "Escape") {
                    event.preventDefault();
                    closeActionMenu();
                  }
                }}
              >
                {senderAliasTarget ? (
                  <button
                    ref={firstActionMenuItemRef}
                    className="message-action-menu-item"
                    type="button"
                    role="menuitem"
                    onClick={() => {
                      onOpenAliasDialog?.(senderAliasTarget);
                      closeActionMenu();
                    }}
                  >
                    <Edit3 size={14} aria-hidden="true" />
                    <span>
                      {t(
                        aliasTargetIsActive(senderAliasTarget)
                          ? "room.editAliasForMember"
                          : "room.setAliasForMember",
                        { name: senderAliasTarget.displayLabel }
                      )}
                    </span>
                  </button>
                ) : null}
                {canCopyMessage ? (
                  <button
                    ref={!senderAliasTarget ? firstActionMenuItemRef : undefined}
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
                    ref={!senderAliasTarget && !canCopyMessage ? firstActionMenuItemRef : undefined}
                    className="message-action-menu-item"
                    type="button"
                    role="menuitem"
                    onClick={copyPermalink}
                  >
                    <span aria-hidden="true" />
                    <span>{t("timeline.copyPermalink")}</span>
                  </button>
                ) : null}
                {canViewSource ? (
                  <button
                    ref={
                      !senderAliasTarget && !canCopyMessage && !canCopyPermalink
                        ? firstActionMenuItemRef
                        : undefined
                    }
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
	                        !senderAliasTarget &&
	                        !canCopyMessage &&
	                        !canCopyPermalink &&
	                        !canViewSource
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

function emitTimelineEventDiagnosticLog(
  event: TimelineEvent,
  key: TimelineKey,
  emit: (source: string, message: string) => void
): void {
  const kind = timelineKindDiagnosticLabel(key);
  if ("InitialItems" in event) {
    emit(
      "timeline.event",
      `kind=${kind} initial items=${event.InitialItems.items.length} generation=${event.InitialItems.generation}`
    );
    return;
  }
  if ("ItemsUpdated" in event) {
    emit(
      "timeline.event",
      `kind=${kind} update diffs=${event.ItemsUpdated.diffs.length} generation=${event.ItemsUpdated.generation}`
    );
    return;
  }
  if ("PaginationStateChanged" in event) {
    emit(
      "timeline.event",
      `kind=${kind} pagination direction=${event.PaginationStateChanged.direction} state=${paginationStateLogLabel(event.PaginationStateChanged.state)}`
    );
    return;
  }
  if ("AnchorRestoreFinished" in event) {
    emit(
      "timeline.event",
      `kind=${kind} anchor restore status=${anchorRestoreStatusLogLabel(event.AnchorRestoreFinished.status)}`
    );
    return;
  }
  if ("NavigationUpdated" in event) {
    emit(
      "timeline.event",
      `kind=${kind} navigation unread=${event.NavigationUpdated.snapshot.unread_event_count} newer=${event.NavigationUpdated.snapshot.newer_event_count} bottom=${event.NavigationUpdated.snapshot.can_jump_to_bottom}`
    );
    return;
  }
  if ("ResyncRequired" in event) {
    emit("timeline.event", `kind=${kind} resync reason=${event.ResyncRequired.reason}`);
  }
}

function timelineEventCompletesBackfillRequest(event: TimelineEvent): boolean {
  if ("InitialItems" in event || "ResyncRequired" in event) {
    return true;
  }
  if ("ItemsUpdated" in event) {
    return batchContainsPrepend(event.ItemsUpdated.diffs);
  }
  if ("PaginationStateChanged" in event) {
    return (
      event.PaginationStateChanged.direction === "Backward" &&
      event.PaginationStateChanged.state !== "Paginating"
    );
  }
  return false;
}

function timelineKindDiagnosticLabel(key: TimelineKey): "room" | "thread" | "focused" {
  if ("Room" in key.kind) {
    return "room";
  }
  if ("Thread" in key.kind) {
    return "thread";
  }
  return "focused";
}

function paginationStateLogLabel(state: PaginationState): string {
  if (typeof state === "string") {
    return state;
  }
  return `Failed(${state.Failed.kind})`;
}

function anchorRestoreStatusLogLabel(status: TimelineAnchorRestoreStatus): string {
  if (typeof status === "string") {
    return status;
  }
  return `Failed(${status.Failed.kind})`;
}

function paginationStateDiagnosticLabel(
  state: ReturnType<typeof getPaginationState>
): string {
  if (typeof state === "string") {
    return state;
  }
  if ("Failed" in state) {
    return "Failed";
  }
  return "Unknown";
}

function timelineAvatarMxcsForItems(
  items: readonly TimelineItem[],
  profileUsers: Record<string, UserProfile>
): Set<string> {
  const mxcs = new Set<string>();
  for (const item of items) {
    const profileAvatar = item.sender ? profileUsers[item.sender]?.avatar : null;
    const avatar = item.sender_avatar ?? profileAvatar;
    if (avatar) {
      mxcs.add(avatar.mxc_uri);
    }
  }
  return mxcs;
}

function timelineAvatarDiagnostics(
  items: readonly TimelineItem[],
  profileUsers: Record<string, UserProfile>,
  avatarThumbnails: Record<string, AvatarThumbnailState>
): Omit<
  TimelineDiagnostics,
  "visibleItems" | "downloadedItems" | "backfill" | "avatarRenderedImages" | "avatarBrokenImages"
> {
  const diagnostics = {
    avatarMxcItems: 0,
    avatarReadyItems: 0,
    avatarPendingItems: 0,
    avatarFailedItems: 0,
    avatarMissingItems: 0
  };
  for (const item of items) {
    const profileAvatar = item.sender ? profileUsers[item.sender]?.avatar : null;
    const avatar = item.sender_avatar ?? profileAvatar;
    if (!avatar) {
      diagnostics.avatarMissingItems += 1;
      continue;
    }
    diagnostics.avatarMxcItems += 1;
    const thumbnail = avatarThumbnails[avatar.mxc_uri] ?? avatar.thumbnail;
    if (thumbnail.kind === "ready") {
      diagnostics.avatarReadyItems += 1;
    } else if (thumbnail.kind === "failed") {
      diagnostics.avatarFailedItems += 1;
    } else {
      diagnostics.avatarPendingItems += 1;
    }
  }
  return diagnostics;
}

function timelineRenderedAvatarDiagnostics(container: HTMLElement | null): {
  avatarRenderedImages: number;
  avatarBrokenImages: number;
} {
  if (!container) {
    return { avatarRenderedImages: 0, avatarBrokenImages: 0 };
  }
  const images = Array.from(container.querySelectorAll<HTMLImageElement>(".avatar img"));
  return {
    avatarRenderedImages: images.length,
    avatarBrokenImages: images.filter((image) => image.complete && image.naturalWidth === 0).length
  };
}

function avatarThumbnailLogMessage(thumbnail: AvatarThumbnailState): string {
  if (thumbnail.kind === "ready") {
    return "avatar thumbnail ready";
  }
  if (thumbnail.kind === "failed") {
    return `avatar thumbnail failed kind=${thumbnail.failureKind}`;
  }
  return `avatar thumbnail ${thumbnail.kind}`;
}

function aliasTargetIsActive(target: TimelineAliasTarget): boolean {
  const displayLabel = target.displayLabel.trim();
  const originalDisplayLabel = target.originalDisplayLabel.trim();
  return Boolean(displayLabel && originalDisplayLabel && displayLabel !== originalDisplayLabel);
}

export function MessageSourceDialog({
  source,
  onClose
}: {
  source: TimelineMessageSource;
  onClose: () => void;
}) {
  const sourceJson = messageSourceJson(source);
  const sourceText = JSON.stringify(sourceJson, null, 2);
  const copyEventId = useCallback(() => {
    void writeClipboardText(source.event_id);
  }, [source.event_id]);
  const copySource = useCallback(() => {
    void writeClipboardText(sourceText);
  }, [sourceText]);

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
      <div className="message-source-event-id">
        <span>{t("timeline.sourceEventId")}</span>
        <code>{source.event_id}</code>
        <button
          className="message-source-copy"
          type="button"
          aria-label={t("timeline.copyEventId")}
          onClick={copyEventId}
        >
          <Copy size={15} aria-hidden="true" />
          <span>{t("timeline.copyEventId")}</span>
        </button>
      </div>
      <div className="message-source-section-header">
        <h3>{t("timeline.originalEventSource")}</h3>
        <button
          className="message-source-copy"
          type="button"
          aria-label={t("timeline.copyOriginalEventSource")}
          onClick={copySource}
        >
          <Copy size={15} aria-hidden="true" />
          <span>{t("timeline.copyOriginalEventSource")}</span>
        </button>
      </div>
      <pre className="message-source-json">
        <code>{sourceText}</code>
      </pre>
    </div>
  );
}

function messageSourceJson(source: TimelineMessageSource): unknown {
  if (source.original_json && typeof source.original_json === "object") {
    return source.original_json;
  }

  const content: Record<string, unknown> = {};
  if (source.body) {
    content.body = source.body;
    content.msgtype = source.has_media ? "m.file" : "m.text";
  }
  if (source.in_reply_to_event_id) {
    content["m.relates_to"] = {
      "m.in_reply_to": {
        event_id: source.in_reply_to_event_id
      }
    };
  }
  if (source.thread_root) {
    content["m.relates_to"] = {
      ...(typeof content["m.relates_to"] === "object" && content["m.relates_to"] !== null
        ? (content["m.relates_to"] as Record<string, unknown>)
        : {}),
      rel_type: "m.thread",
      event_id: source.thread_root
    };
  }

  return {
    content,
    event_id: source.event_id,
    origin_server_ts: source.timestamp_ms,
    sender: source.sender,
    type: "m.room.message",
    unsigned: {
      redacted: source.is_redacted || undefined,
      edited: source.is_edited || undefined,
      media: source.has_media || undefined
    }
  };
}

function formatTypingUsers(userIds: string[]): string {
  const [firstUser] = userIds;
  if (userIds.length === 1 && firstUser) {
    return t("timeline.typingOne", { user: firstUser });
  }
  return t("timeline.typingMany", { count: userIds.length });
}

function formatReceiptDetails(receipts: LiveReadReceipt[], overflowCount: number): string[] {
  const details = receipts.map((receipt) => {
    const timestamp = formatReceiptTimestamp(receipt.timestamp_ms);
    const name = receiptDisplayName(receipt);
    return timestamp ? `${name} ${timestamp}` : name;
  });
  if (overflowCount > 0) {
    details.push(t("timeline.readReceiptOverflow", { count: overflowCount }));
  }
  return details;
}

export function receiptDisplayName(receipt: LiveReadReceipt): string {
  return receipt.display_name?.trim() || receipt.original_display_label.trim();
}

function receiptInitials(receipt: LiveReadReceipt): string {
  const label = receiptDisplayName(receipt);
  const ascii = label.match(/[A-Za-z]/g);
  if (ascii?.length) {
    return ascii.slice(0, 2).join("").toUpperCase();
  }
  return label.slice(0, 2);
}

function receiptAvatarSource(receipt: LiveReadReceipt): string | null {
  return receipt.avatar?.thumbnail.kind === "ready"
    ? mediaSourceUrl(receipt.avatar.thumbnail.source_url)
    : null;
}

function reactionSenderLabelsByUserId(
  profileUsers: Record<string, UserProfile>
): Record<string, string> {
  return Object.fromEntries(
    Object.entries(profileUsers).map(([userId, profile]) => [
      userId,
      profile.display_label?.trim() || profile.display_name?.trim() || profile.original_display_label.trim() || userId
    ])
  );
}

function formatReactionTooltip(
  reactionKey: string,
  totalCount: number,
  senderPreview: readonly string[],
  senderLabelsByUserId: Record<string, string>
): string | null {
  if (totalCount <= 0) {
    return null;
  }
  const previewLabels = senderPreview.map((userId) => senderLabelsByUserId[userId] ?? userId);
  const overflowCount = Math.max(0, totalCount - previewLabels.length);
  const labels =
    overflowCount > 0
      ? [...previewLabels, t("timeline.reactionSenderOverflow", { count: overflowCount })]
      : previewLabels;
  const names =
    labels.length > 0
      ? new Intl.ListFormat(getActiveLocale(), { style: "long", type: "conjunction" }).format(labels)
      : t("timeline.reactionSenderUnknown", { count: totalCount });
  return t("timeline.reactionTooltip", { names, key: reactionKey });
}

function syntheticDateDividerTimestampMs(
  syntheticId: string | null,
  timestampMs: number | null
): number | null {
  if (!syntheticId?.startsWith("date-divider-")) {
    return null;
  }
  if (timestampMs !== null) {
    return timestampMs;
  }
  const parsed = Number(syntheticId.slice("date-divider-".length));
  return Number.isFinite(parsed) ? parsed : null;
}

function formatDateDividerLabel(timestampMs: number): string {
  return new Intl.DateTimeFormat(getActiveLocale(), {
    weekday: "short",
    year: "numeric",
    month: "short",
    day: "numeric"
  }).format(new Date(timestampMs));
}

function thumbnailSourceUrl(thumbnail: AvatarThumbnailState | null | undefined): string | null {
  return thumbnail?.kind === "ready" ? mediaSourceUrl(thumbnail.source_url) : null;
}

function formatReceiptTimestamp(timestampMs: number | null): string | null {
  if (timestampMs === null) {
    return null;
  }
  return new Intl.DateTimeFormat(getActiveLocale(), {
    dateStyle: "medium",
    timeStyle: "short"
  }).format(new Date(timestampMs));
}

function formatMessageTimestamp(timestampMs: number | null): string | null {
  if (timestampMs === null) {
    return null;
  }
  return new Intl.DateTimeFormat(getActiveLocale(), {
    timeStyle: "short"
  }).format(new Date(timestampMs));
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

function localizedTimelineItemBody(item: TimelineItem): string {
  switch (item.notice_i18n_key) {
    case "timeline.notice.roomCreate":
      return t("timeline.notice.roomCreate");
    case "timeline.notice.roomPowerLevels":
      return t("timeline.notice.roomPowerLevels");
    case "timeline.notice.roomGuestAccess":
      return t("timeline.notice.roomGuestAccess");
    case "timeline.notice.roomEncryption":
      return t("timeline.notice.roomEncryption");
    case "timeline.notice.spaceParent":
      return t("timeline.notice.spaceParent");
    case "timeline.notice.roomJoinRules":
      return t("timeline.notice.roomJoinRules");
    case "timeline.notice.roomHistoryVisibility":
      return t("timeline.notice.roomHistoryVisibility");
    case "timeline.notice.roomPinnedEvents":
      return t("timeline.notice.roomPinnedEvents");
    default:
      return item.body ?? "";
  }
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

// ---------------------------------------------------------------------------
// MessageMeta: timestamp + send-state marks (extracted for testability, #83)
// ---------------------------------------------------------------------------

/**
 * Renders the heading-region metadata for a timeline message row:
 * sender label, timestamp, edited marker, send-state text labels, and the
 * sent checkmark. All data comes from Rust-owned DTO fields; no local
 * inference of send/edit state is performed here.
 */
export function MessageMeta({
  senderDisplayLabel,
  timestampMs,
  isEdited,
  isRedacted,
  sendStateKind,
  presence
}: {
  senderDisplayLabel: string;
  timestampMs: number | null;
  isEdited: boolean;
  isRedacted: boolean;
  sendStateKind: string | null;
  presence?: import("../domain/types").PresenceKind;
}): ReactNode {
  const messageTimestamp = formatMessageTimestamp(timestampMs);
  const sendStateLabel =
    sendStateKind === "sending"
      ? t("timeline.sending")
      : sendStateKind === "notSent"
        ? t("timeline.notSent")
        : sendStateKind === "cancelled"
          ? t("timeline.cancelledSend")
          : null;
  const sentStateMark =
    sendStateKind === "sent" ? (
      <span
        className="message-send-state"
        data-send-state="sent"
        aria-label={t("timeline.sent")}
      >
        <Check size={12} aria-hidden="true" />
      </span>
    ) : null;

  return (
    <>
      {presence ? (
        <span
          className="presence-dot message-presence"
          data-presence={presence}
          aria-label={presenceLabel(presence)}
        />
      ) : null}
      <span className="sender" dir="auto">{senderDisplayLabel}</span>
      {messageTimestamp ? (
        <time className="message-timestamp" dateTime={new Date(timestampMs!).toISOString()}>
          {messageTimestamp}
        </time>
      ) : null}
      {isEdited && !isRedacted ? (
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
      {sentStateMark}
    </>
  );
}

function TimelineMediaAttachment({
  media,
  progress,
  downloadState,
  canDownload,
  onDownload
}: {
  media: NonNullable<TimelineItem["media"]>;
  progress: MediaTransferProgress | null;
  downloadState?: TimelineMediaDownloadState;
  canDownload: boolean;
  onDownload: () => void;
}) {
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

  if (downloadState?.kind === "ready" && media.kind === "Image") {
    const sourceUrl = mediaSourceUrl(downloadState.source_url);
    return (
      <div
        className="message-media message-media-ready"
        data-media-kind={media.kind}
        data-media-encrypted={media.source.encrypted || undefined}
        data-download-state="ready"
      >
        <a
          className="message-media-preview-link"
          href={sourceUrl}
          target="_blank"
          rel="noreferrer"
          aria-label={t("timeline.mediaOpenFile")}
        >
          <img
            className="message-media-image"
            src={sourceUrl}
            alt={media.filename}
            width={downloadState.width ?? undefined}
            height={downloadState.height ?? undefined}
            loading="lazy"
          />
        </a>
      </div>
    );
  }

  const progressPercent =
    uploadProgressPercentValue ?? downloadProgressPercent;

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
            onClick={onDownload}
          >
            <RefreshCw size={15} />
          </button>
        ) : downloadState?.kind === "ready" ? (
          <a
            className="message-media-download"
            href={mediaSourceUrl(downloadState.source_url)}
            target="_blank"
            rel="noreferrer"
            aria-label={t("timeline.mediaOpenFile")}
            download={media.filename}
          >
            <Download size={15} />
          </a>
        ) : (
          <button
            className="message-media-download"
            type="button"
            disabled={downloadState?.kind === "pending"}
            aria-label={t("timeline.downloadMedia", { filename: media.filename })}
            onClick={onDownload}
          >
            <Download size={15} />
          </button>
        )
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

function timelineEditImeShouldHandleKeyEvent(
  event: KeyboardEvent<HTMLTextAreaElement>,
  compositionActive: boolean
): boolean {
  return (
    event.key === "Enter" &&
    (compositionActive ||
      event.nativeEvent.isComposing ||
      event.keyCode === 229)
  );
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
