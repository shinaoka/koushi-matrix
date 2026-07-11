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
  Info,
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
  Component,
  Fragment,
  type CSSProperties,
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
  Suspense,
  lazy,
  type Dispatch,
  type SetStateAction
} from "react";

import { getActiveLocale, t } from "../i18n/messages";
import { useRecoverableImageSource } from "./avatarImage";
import { findQueryHighlightRange } from "./searchHighlight";
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
  getThreadRootProjections,
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
  TimelineThreadRootOrder,
  UserProfile
} from "../domain/types";
import type { TimelineLinkRange } from "../domain/coreEvents";
import type { TimelineForwardDestination } from "../domain/projectionTypes";
import {
  projectTimelineDisplayRows,
  type TimelineDisplayRow
} from "../domain/timelineDisplayProjection";

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
  sendReadReceipt(roomId: string, eventId: string, threadRootEventId?: string | null): Promise<void>;
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
  /** Save an already downloaded media file through the host desktop shell. */
  saveMediaFile?(sourceUrl: string, filename: string): Promise<void>;
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

export type TimelineThreadAttention = {
  rootEventId: string;
  notificationCount: number;
  highlightCount: number;
  liveEventMarkerCount: number;
};

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
  onLoadLinkPreviews: (roomId: string, eventId: string, pendingCount?: number) => void;
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

type ScrollAnchorCaptureOptions = {
  /**
   * Lets a caller exclude a row whose presentation position is itself being
   * changed. This is essential for the latest-reply projection: restoring a
   * moved root would preserve the wrong visual intent.
   */
  isEligible?: (node: HTMLElement) => boolean;
};

type TimelineEventIdentity = "content" | "activity";

type TimelineProjectionSnapshot = {
  timelineKeyHash: string;
  generation: number;
  signature: string;
  rows: readonly TimelineDisplayRow[];
};

type PendingProjectionLayoutTransaction = {
  timelineKeyHash: string;
  generation: number;
  signature: string;
  revision: number;
  intentRevision: number;
  mode: "free-scroll" | "live-edge";
  anchor: ScrollAnchor | null;
};

type ProjectionSnapshotBoundaryProps = {
  snapshot: TimelineProjectionSnapshot;
  onBeforeProjectionChange: (
    previous: TimelineProjectionSnapshot,
    next: TimelineProjectionSnapshot
  ) => void;
  children: ReactNode;
};

/**
 * Function components do not expose `getSnapshotBeforeUpdate`. This boundary
 * provides the commit-safe pre-mutation point needed to capture an old DOM
 * anchor without allowing an abandoned render to touch scroll transaction
 * refs. It renders no DOM of its own.
 */
class ProjectionSnapshotBoundary extends Component<ProjectionSnapshotBoundaryProps> {
  override getSnapshotBeforeUpdate(previousProps: ProjectionSnapshotBoundaryProps): null {
    this.props.onBeforeProjectionChange(previousProps.snapshot, this.props.snapshot);
    return null;
  }

  override componentDidUpdate(): void {}

  override render(): ReactNode {
    return this.props.children;
  }
}

type PendingMeasuredHeight = {
  height: number;
  epoch: number;
};

/** Capture the first eligible visible item as the anchor (id + pixel offset). */
function captureAnchor(
  container: HTMLElement,
  options: ScrollAnchorCaptureOptions = {}
): ScrollAnchor | null {
  const containerTop = container.getBoundingClientRect().top;
  const nodes = container.querySelectorAll<HTMLElement>("[data-item-id]");
  for (const node of nodes) {
    if (options.isEligible && !options.isEligible(node)) {
      continue;
    }
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

/**
 * A root shown at a reply's activity position is not a stable free-scroll
 * anchor: the next reply/redaction can relocate it again. Prefer a normal
 * material row and leave the anchor empty when no such row is mounted.
 */
function captureFreeScrollAnchor(container: HTMLElement): ScrollAnchor | null {
  return captureAnchor(container, {
    isEligible: (node) => {
      const contentEventId = node.dataset["contentEventId"] ?? null;
      const activityEventId = node.dataset["activityEventId"] ?? null;
      return contentEventId === null || activityEventId === null || contentEventId === activityEventId;
    }
  });
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
  const nodes = container.querySelectorAll<HTMLElement>("[data-activity-event-id]");
  let captured: CapturedTimelineScrollAnchor | null = null;
  for (const node of nodes) {
    const rect = node.getBoundingClientRect();
    if (rect.bottom <= containerRect.top || rect.top >= containerRect.bottom) {
      continue;
    }
    const eventId = eventIdForTimelineIdentity(node, "activity");
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
  return findTimelineEventNode(container, "activity", anchor.event_id);
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

function canonicalTimelineContainsActivityEventId(
  items: readonly TimelineItem[],
  eventId: string
): boolean {
  return items.some(
    (item) => "Event" in item.id && item.id.Event.event_id === eventId
  );
}

function timelineEventIdentityAttribute(identity: TimelineEventIdentity): string {
  return identity === "activity" ? "data-activity-event-id" : "data-content-event-id";
}

function eventIdForTimelineIdentity(
  node: HTMLElement,
  identity: TimelineEventIdentity
): string | null {
  return identity === "activity"
    ? node.dataset["activityEventId"] ?? null
    : node.dataset["contentEventId"] ?? null;
}

function findTimelineEventNode(
  container: HTMLElement,
  identity: TimelineEventIdentity,
  eventId: string
): HTMLElement | null {
  return container.querySelector<HTMLElement>(
    `[${timelineEventIdentityAttribute(identity)}="${cssEscape(eventId)}"]`
  );
}

function visibleEventIds(container: HTMLElement): {
  firstVisibleEventId: string | null;
  lastVisibleEventId: string | null;
} {
  const containerRect = container.getBoundingClientRect();
  const nodes = container.querySelectorAll<HTMLElement>("[data-activity-event-id]");
  let firstVisibleEventId: string | null = null;
  let lastVisibleEventId: string | null = null;
  for (const node of nodes) {
    const rect = node.getBoundingClientRect();
    if (rect.bottom <= containerRect.top || rect.top >= containerRect.bottom) {
      continue;
    }
    const eventId = eventIdForTimelineIdentity(node, "activity");
    if (!eventId) {
      continue;
    }
    firstVisibleEventId ??= eventId;
    lastVisibleEventId = eventId;
  }
  return { firstVisibleEventId, lastVisibleEventId };
}

function timelineProjectionSignature(rows: readonly TimelineDisplayRow[]): string {
  return rows
    .map((row) =>
      [
        row.row_id,
        row.kind,
        row.content_event_id ?? "",
        row.activity_event_id ?? "",
        row.display_timestamp_ms ?? ""
      ].join("\u0000")
    )
    .join("\u0001");
}

function projectionStructureChanged(
  previous: TimelineProjectionSnapshot,
  next: TimelineProjectionSnapshot
): boolean {
  return previous.signature !== next.signature;
}

/**
 * Pick only ordinary rows that survive a projection change with both
 * identities intact. Thread-root rows are intentionally excluded even when
 * their row id survives: their visual placement is the mutation in progress.
 */
function stableProjectionAnchorRowIds(
  previousRows: readonly TimelineDisplayRow[],
  nextRows: readonly TimelineDisplayRow[]
): ReadonlySet<string> {
  const nextByRowId = new Map(nextRows.map((row) => [row.row_id, row]));
  const stable = new Set<string>();
  for (const previous of previousRows) {
    const next = nextByRowId.get(previous.row_id);
    if (
      next === undefined ||
      previous.kind === "threadRoot" ||
      next.kind === "threadRoot" ||
      previous.content_event_id !== next.content_event_id ||
      previous.activity_event_id !== next.activity_event_id
    ) {
      continue;
    }
    stable.add(previous.row_id);
  }
  return stable;
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

type ReactionPickerLayout = {
  placement: "above" | "below";
  maxBlockSize: number;
};

function reactionPickerLayoutForControl(control: HTMLElement): ReactionPickerLayout {
  const controlRect = control.getBoundingClientRect();
  const boundary =
    control.closest<HTMLElement>(".timeline-view") ??
    control.closest<HTMLElement>(".main-pane");
  const boundaryRect = boundary?.getBoundingClientRect();
  const boundaryTop = boundaryRect?.top ?? 0;
  const boundaryBottom =
    boundaryRect && boundaryRect.bottom > boundaryRect.top
      ? boundaryRect.bottom
      : typeof window === "undefined"
        ? controlRect.bottom
        : window.innerHeight;
  const availableAbove = Math.max(
    0,
    controlRect.top - boundaryTop - REACTION_PICKER_GAP_PX
  );
  const availableBelow = Math.max(
    0,
    boundaryBottom - controlRect.bottom - REACTION_PICKER_GAP_PX
  );

  if (availableBelow >= REACTION_PICKER_COMFORTABLE_BLOCK_SIZE_PX) {
    return {
      placement: "below",
      maxBlockSize: Math.floor(availableBelow)
    };
  }
  if (availableAbove >= REACTION_PICKER_COMFORTABLE_BLOCK_SIZE_PX) {
    return {
      placement: "above",
      maxBlockSize: Math.floor(availableAbove)
    };
  }
  const placement = availableAbove >= availableBelow ? "above" : "below";
  return {
    placement,
    maxBlockSize: Math.max(
      0,
      Math.floor(placement === "above" ? availableAbove : availableBelow)
    )
  };
}

/** Distance (px) from the top edge that triggers automatic backfill. */
const AUTO_BACKFILL_THRESHOLD_PX = 80;
const AUTO_BACKFILL_PREFETCH_ITEMS = 100;
const SCROLL_EDGE_TOLERANCE_PX = 2;
const TIMELINE_VIRTUALIZATION_THRESHOLD = 600;
const TIMELINE_VIRTUAL_OVERSCAN_ITEMS = 60;
const TIMELINE_AVATAR_THUMBNAIL_OVERSCAN_ITEMS = 8;
const TIMELINE_LINK_PREVIEW_OVERSCAN_ITEMS = 8;
const TIMELINE_ESTIMATED_ITEM_HEIGHT_PX = 72;
const TIMELINE_MIN_ITEM_HEIGHT_PX = 36;
const TIMELINE_MAX_ITEM_HEIGHT_PX = 480;
const TIMELINE_SCROLL_IDLE_FLUSH_MS = 100;
const TIMELINE_SCROLL_MAX_DEFER_MS = 500;
const TIMELINE_SUBSCRIBE_FALLBACK_DELAY_MS = 120;
const REACTION_PICKER_COMFORTABLE_BLOCK_SIZE_PX = 360;
const REACTION_PICKER_GAP_PX = 6;

const ignoreComposerKeyAction: ResolveComposerKeyAction = async () => "noop";
const ignoreSendQueueAction = () => undefined;

const LazyEmojiPicker = lazy(() =>
  import("./EmojiPicker").then((module) => ({ default: module.EmojiPicker }))
);

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

type TimelineItemIndexRange = {
  startIndex: number;
  endIndex: number;
};

type TimelineVirtualWindow = TimelineVirtualRangeState & {
  items: readonly TimelineDisplayRow[];
};

const EMPTY_TIMELINE_RANGE: TimelineVirtualRangeState = {
  virtualized: false,
  startIndex: 0,
  endIndex: 0,
  paddingTop: 0,
  paddingBottom: 0
};

const EMPTY_TIMELINE_ITEM_INDEX_RANGE: TimelineItemIndexRange = {
  startIndex: 0,
  endIndex: 0
};

const ROOT_EVENT_THREAD_ORDER: TimelineThreadRootOrder = { kind: "rootEvent" };

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

const TIMELINE_FRAME_FALLBACK_MS = 16;

type TimelineScheduledFrame = {
  cancel: () => void;
};

function scheduleTimelineFrame(callback: FrameRequestCallback): TimelineScheduledFrame {
  let cancelled = false;
  let frameId: number | null = null;
  let timeoutId: number | null = null;
  const run = (timestamp: number) => {
    if (cancelled) {
      return;
    }
    cancelled = true;
    if (frameId !== null && typeof window.cancelAnimationFrame === "function") {
      window.cancelAnimationFrame(frameId);
      frameId = null;
    }
    if (timeoutId !== null) {
      window.clearTimeout(timeoutId);
      timeoutId = null;
    }
    callback(timestamp);
  };

  if (typeof window.requestAnimationFrame === "function") {
    frameId = window.requestAnimationFrame(run);
  }
  timeoutId = window.setTimeout(() => run(window.performance.now()), TIMELINE_FRAME_FALLBACK_MS);

  return {
    cancel() {
      if (cancelled) {
        return;
      }
      cancelled = true;
      if (frameId !== null && typeof window.cancelAnimationFrame === "function") {
        window.cancelAnimationFrame(frameId);
      }
      if (timeoutId !== null) {
        window.clearTimeout(timeoutId);
      }
    }
  };
}

function buildTimelineHeightModel(
  rows: readonly TimelineDisplayRow[],
  measuredHeights: ReadonlyMap<string, number>,
  fallbackHeight: number
): TimelineHeightModel {
  const fallback = estimatedItemHeight(fallbackHeight);
  const offsets = new Array<number>(rows.length + 1);
  offsets[0] = 0;
  for (const [index, row] of rows.entries()) {
    offsets[index + 1] = offsets[index] + (measuredHeights.get(row.row_id) ?? fallback);
  }
  return {
    fallbackHeight: fallback,
    offsets,
    totalHeight: offsets[rows.length] ?? 0
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

function timelineItemIndexRangeEquals(
  left: TimelineItemIndexRange,
  right: TimelineItemIndexRange
): boolean {
  return left.startIndex === right.startIndex && left.endIndex === right.endIndex;
}

function timelineItemIndexInRange(index: number, range: TimelineItemIndexRange): boolean {
  return index >= range.startIndex && index < range.endIndex;
}

function calculateTimelineItemIndexRange({
  visibleItemsLength,
  metrics,
  model,
  overscanItems
}: {
  visibleItemsLength: number;
  metrics: TimelineViewportMetrics;
  model: TimelineHeightModel;
  overscanItems: number;
}): TimelineItemIndexRange {
  if (visibleItemsLength <= 0) {
    return EMPTY_TIMELINE_ITEM_INDEX_RANGE;
  }

  const viewportHeight = metrics.clientHeight || 600;
  const relativeScrollTop = Math.max(0, metrics.scrollTop - metrics.listOffsetTop);
  const firstVisibleIndex = timelineIndexAtOffset(model.offsets, relativeScrollTop);
  const lastVisibleIndex = timelineIndexAtOffset(
    model.offsets,
    relativeScrollTop + viewportHeight
  );
  const startIndex = Math.max(0, firstVisibleIndex - overscanItems);
  const endIndex = Math.min(
    visibleItemsLength,
    Math.max(startIndex + 1, lastVisibleIndex + overscanItems + 1)
  );

  return { startIndex, endIndex };
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

  const { startIndex, endIndex } = calculateTimelineItemIndexRange({
    visibleItemsLength,
    metrics,
    model,
    overscanItems: TIMELINE_VIRTUAL_OVERSCAN_ITEMS
  });

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
  // #162: match with the same NFKC + case-fold rule as the Rust search matcher
  // so a visible highlight and the Search panel's exact-match count agree.
  const range = findQueryHighlightRange(text, query);
  if (!range) {
    return text;
  }
  return (
    <>
      {text.slice(0, range.start)}
      <mark>{text.slice(range.start, range.end)}</mark>
      {text.slice(range.end)}
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

type TimelineMediaViewerItem = {
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
  isAnchored = false,
  onReturnToLive,
  autoLoadOlderMessages = false,
  threadRootOrder = ROOT_EVENT_THREAD_ORDER,
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
  listRefCallback,
  onRegisterJumpToLatest,
  threadAttention = null
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
  // #161: main pane is anchored to a jump-to-date event; the live-edge control
  // returns to the live timeline instead of scrolling within the focused window.
  isAnchored?: boolean;
  onReturnToLive?: () => void;
  autoLoadOlderMessages?: boolean;
  /** Presentation-only Room order; the canonical store remains SDK ordered. */
  threadRootOrder?: TimelineThreadRootOrder;
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
   * inspect the committed list node without owning viewport semantics.
   */
  listRefCallback?: (element: HTMLDivElement | null) => void;
  /**
   * Optional callback registering the TimelineView-owned live-edge jump action
   * for parent chrome controls.
   */
  onRegisterJumpToLatest?: (handler: (() => void) | null) => void;
  /** Thread attention counters for the root row in the currently selected room. */
  threadAttention?: TimelineThreadAttention | null;
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
  const [mediaViewerItem, setMediaViewerItem] = useState<TimelineMediaViewerItem | null>(null);
  const mediaViewerReturnFocusRef = useRef<HTMLElement | null>(null);
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
  const [avatarRequestRange, setAvatarRequestRange] = useState<TimelineItemIndexRange>(
    EMPTY_TIMELINE_ITEM_INDEX_RANGE
  );
  const avatarRequestRangeRef = useRef<TimelineItemIndexRange>(
    EMPTY_TIMELINE_ITEM_INDEX_RANGE
  );
  const [linkPreviewRequestRange, setLinkPreviewRequestRange] =
    useState<TimelineItemIndexRange>(EMPTY_TIMELINE_ITEM_INDEX_RANGE);
  const linkPreviewRequestRangeRef = useRef<TimelineItemIndexRange>(
    EMPTY_TIMELINE_ITEM_INDEX_RANGE
  );
  const pendingScrollFrameRef = useRef<TimelineScheduledFrame | null>(null);
  const rangeModelEpochRef = useRef(0);
  const virtualItemHeight = TIMELINE_ESTIMATED_ITEM_HEIGHT_PX;
  const [measuredHeightVersion, setMeasuredHeightVersion] = useState(0);
  const containerRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const itemHeightByDomIdRef = useRef<Map<string, number>>(new Map());
  const openMediaViewer = useCallback((item: TimelineMediaViewerItem) => {
    const activeElement =
      typeof document !== "undefined" && document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    mediaViewerReturnFocusRef.current = activeElement;
    setMediaViewerItem(item);
  }, []);
  const closeMediaViewer = useCallback(() => {
    const returnFocusTarget = mediaViewerReturnFocusRef.current;
    setMediaViewerItem(null);
    mediaViewerReturnFocusRef.current = null;
    window.setTimeout(() => returnFocusTarget?.focus(), 0);
  }, []);
  /** Anchor captured before the latest prepend batch was applied. */
  const pendingAnchorRef = useRef<ScrollAnchor | null>(null);
  /** True from prepend-apply until anchor restoration completed. */
  const anchorRestorePendingRef = useRef(false);
  /** True while the live-room scroll anchor is being restored. */
  const roomScrollAnchorRestorePendingRef = useRef(false);
  /**
   * Last first-visible anchor captured while in free-scroll. Because
   * `.timeline-view` uses `overflow-anchor: none`, the browser no longer
   * corrects scroll position when an above-viewport row resizes (image load,
   * deferred measurement, CSS growth). This anchor lets the ResizeObserver
   * restore the viewport so the visible row stays put — Koushi-owned anchoring.
   */
  const freeScrollAnchorRef = useRef<ScrollAnchor | null>(null);
  /**
   * True while a programmatic jump (jump-to-event/bottom) owns the viewport.
   * A jump centers/targets a specific row and runs its own follow-up
   * re-centering across measurement frames, so free-scroll resize anchoring
   * must stand down until the user scrolls and takes control again. Otherwise
   * the two corrections fight and the jump target drifts.
   */
  const jumpViewportControlRef = useRef(false);
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
  /** Session-anchor restores can emit layout scroll events; wait for user intent before backfill. */
  const autoBackfillRequiresUserScrollRef = useRef(false);
  /** Coalesces ResizeObserver-driven live-edge corrections. */
  const viewportIntentResizeFrameRef = useRef<TimelineScheduledFrame | null>(null);
  /** Coalesces a structural display-projection correction to one frame. */
  const projectionLayoutFrameRef = useRef<TimelineScheduledFrame | null>(null);
  const pendingProjectionLayoutRef = useRef<PendingProjectionLayoutTransaction | null>(null);
  const projectionRenderStateRef = useRef<TimelineProjectionSnapshot | null>(null);
  const projectionLayoutRevisionRef = useRef(0);
  /** Invalidates a queued projection correction when viewport ownership changes. */
  const viewportIntentRevisionRef = useRef(0);
  const scrollFollowUpFramesRef = useRef<Set<TimelineScheduledFrame>>(new Set());
  /** Pagination request currently in flight (suppresses duplicates). */
  const backfillInFlightRef = useRef(false);
  const underfilledInitialBackfillDiagnosticSignatureRef = useRef<string | null>(null);
  const readSignalEventRef = useRef<string | null>(null);
  const lastViewportObservationRef = useRef<string | null>(null);
  const downloadedEventIdsRef = useRef<Set<string>>(new Set());
  const requestedImagePreviewEventIdsRef = useRef<Set<string>>(new Set());
  const relevantAvatarMxcsRef = useRef<Set<string>>(new Set());
  const requestedAvatarMxcsRef = useRef<Set<string>>(new Set());
  const avatarRetryCountsRef = useRef<Map<string, number>>(new Map());
  const initialItemsSeenForTimelineKeyRef = useRef<string | null>(null);
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
  const readSignalRoomId =
    "Room" in timelineKey.kind
      ? timelineKey.kind.Room.room_id
      : "Thread" in timelineKey.kind
        ? timelineKey.kind.Thread.room_id
        : null;
  const readSignalThreadRootEventId =
    "Thread" in timelineKey.kind ? timelineKey.kind.Thread.root_event_id : null;
  const items = getItems(store, timelineKey);
  // The selector returns an array. Memoize it by the separately-owned map so
  // ordinary scroll/measurement renders keep the existing display-row
  // identity; otherwise an empty projection source would churn Task 4's
  // height-model transaction on every render.
  const threadRootProjections = useMemo(
    () => getThreadRootProjections(store, timelineKey),
    [store.threadRootProjections, timelineKey]
  );
  const timelineKeyState = getKeyState(store, timelineKey);
  const generation = timelineKeyState?.generation ?? 0;
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
      pendingScrollFrameRef.current.cancel();
      pendingScrollFrameRef.current = null;
    }
    pendingScrollFrameUserInputRef.current = false;
  }, []);
  const cancelScrollFollowUpFrames = useCallback(() => {
    for (const frame of scrollFollowUpFramesRef.current) {
      frame.cancel();
    }
    scrollFollowUpFramesRef.current.clear();
  }, []);
  const scheduleScrollFollowUpFrame = useCallback((callback: FrameRequestCallback) => {
    const frameRef: { current: TimelineScheduledFrame | null } = { current: null };
    let completed = false;
    const frame = scheduleTimelineFrame((timestamp) => {
      completed = true;
      if (frameRef.current) {
        scrollFollowUpFramesRef.current.delete(frameRef.current);
      }
      callback(timestamp);
    });
    frameRef.current = frame;
    if (!completed) {
      scrollFollowUpFramesRef.current.add(frame);
    }
    return frame;
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
      let clearSuppressionToken = programmaticScrollTokenRef.current;
      let clearSignatureToken: number | null = null;
      action();
      if (container && container.scrollTop !== beforeScrollTop) {
        const token = programmaticScrollTokenRef.current + 1;
        programmaticScrollTokenRef.current = token;
        clearSuppressionToken = token;
        clearSignatureToken = token;
        programmaticScrollSignatureRef.current = {
          scrollHeight: container.scrollHeight,
          scrollTop: container.scrollTop,
          reason,
          token
        };
        updateScrollDiagnostics((current) => recordTimelineScrollWrite(current, reason));
      }
      scheduleTimelineFrame(() => {
        if (anchorAsyncGenerationRef.current !== asyncGeneration) {
          return;
        }
        if (programmaticScrollTokenRef.current === clearSuppressionToken) {
          suppressScrollAnchorCaptureRef.current = false;
        }
        if (
          clearSignatureToken !== null &&
          programmaticScrollSignatureRef.current?.token === clearSignatureToken
        ) {
          programmaticScrollSignatureRef.current = null;
        }
      });
    },
    [updateScrollDiagnostics]
  );

  const setViewportIntentToLiveEdge = useCallback(() => {
    if (viewportIntentRef.current.kind !== "live-edge") {
      viewportIntentRevisionRef.current += 1;
    }
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
    if (viewportIntentRef.current.kind !== "free-scroll") {
      viewportIntentRevisionRef.current += 1;
    }
    viewportIntentRef.current = { kind: "free-scroll" };
    stickToBottomAfterMeasurementRef.current = false;
  }, []);

  const releaseViewportIntent = useCallback(() => {
    setViewportIntentToFreeScroll();
    userScrollInputPendingRef.current = false;
  }, [setViewportIntentToFreeScroll]);

  const markUserScrollInput = useCallback((options: { keepLiveEdgeAtBottom?: boolean } = {}) => {
    // Input belongs to the user even when it leaves the logical intent kind
    // unchanged (for example another free-scroll wheel event). A queued
    // projection frame must never reclaim that viewport position.
    viewportIntentRevisionRef.current += 1;
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
        cancelScrollFollowUpFrames();
        recordTimelineResync();
        pendingAnchorRef.current = null;
        anchorRestorePendingRef.current = false;
        roomScrollAnchorRestorePendingRef.current = false;
        viewportIntentRef.current = { kind: "free-scroll" };
        userScrollInputPendingRef.current = false;
        autoBackfillRequiresUserScrollRef.current = false;
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
                                : "ThreadRootProjection" in event
                                  ? event.ThreadRootProjection.key
                                  : event.ResyncRequired.key;
      if (!timelineKeyEquals(eventKey, timelineKeyRef.current)) {
        recordTimelineKeyMismatch();
        return;
      }
      emitTimelineEventDiagnosticLog(event, eventKey, emitDiagnosticLog);
      if ("InitialItems" in event) {
        initialItemsSeenForTimelineKeyRef.current = timelineKeyHashRef.current;
        recordTimelineInitialItems(event.InitialItems.items.length);
        cancelScrollFollowUpFrames();
        resetActiveMeasurementDeferral({ clearMountedIds: true });
        relevantAvatarMxcsRef.current = timelineAvatarMxcsForItems(
          event.InitialItems.items,
          profileUsersRef.current
        );
      }
      if (
        "ItemsUpdated" in event &&
        timelineDiffsContainReset(event.ItemsUpdated.diffs)
      ) {
        cancelScrollFollowUpFrames();
        resetActiveMeasurementDeferral({ clearMountedIds: true });
      }
      const backfillCompletionReason = timelineBackfillCompletionReason(event);
      if (backfillCompletionReason !== null) {
        if (backfillInFlightRef.current) {
          emitDiagnosticLog(
            "timeline.backfill",
            `stage=complete reason=${backfillCompletionReason}`
          );
        }
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
        cancelScrollFollowUpFrames();
        pendingAnchorRef.current = null;
        anchorRestorePendingRef.current = false;
        roomScrollAnchorRestorePendingRef.current = false;
        viewportIntentRef.current = { kind: "free-scroll" };
        userScrollInputPendingRef.current = false;
        autoBackfillRequiresUserScrollRef.current = false;
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
    cancelScrollFollowUpFrames,
    emitDiagnosticLog,
    isAppLevelStore,
    resetActiveMeasurementDeferral,
    setViewportIntentToLiveEdge,
    timelineKeyHash,
    transport
  ]);

  useEffect(() => {
    if (!transport.ensureSubscribed) {
      return;
    }
    if (items.length > 0) {
      return;
    }
    const timelineKeyHashAtSchedule = timelineKeyHash;
    const timeoutId = window.setTimeout(() => {
      if (timelineKeyHashRef.current !== timelineKeyHashAtSchedule) {
        return;
      }
      if (initialItemsSeenForTimelineKeyRef.current === timelineKeyHashAtSchedule) {
        return;
      }
      void transport.ensureSubscribed?.(timelineKeyRef.current).catch(() => undefined);
    }, TIMELINE_SUBSCRIBE_FALLBACK_DELAY_MS);
    return () => {
      window.clearTimeout(timeoutId);
    };
  }, [items.length, timelineKeyHash, transport]);

  useEffect(
    () => () => {
      cancelPendingScrollFrame();
      cancelScrollFollowUpFrames();
      resetActiveMeasurementDeferral({ clearMountedIds: true });
    },
    [cancelPendingScrollFrame, cancelScrollFollowUpFrames, resetActiveMeasurementDeferral]
  );

  useLayoutEffect(() => {
    cancelPendingScrollFrame();
    cancelScrollFollowUpFrames();
    const sessionViewport = timelineViewportSessionMemory.get(timelineKeyHash) ?? null;
    sessionRoomScrollAnchorRef.current =
      sessionViewport?.mode === "anchor" ? sessionViewport.anchor : null;
    anchorAsyncGenerationRef.current += 1;
    pendingAnchorRef.current = null;
    anchorRestorePendingRef.current = false;
    roomScrollAnchorRestorePendingRef.current = false;
    freeScrollAnchorRef.current = null;
    jumpViewportControlRef.current = false;
    suppressScrollAnchorCaptureRef.current = false;
    restoredRoomScrollAnchorSignatureRef.current = null;
    viewportIntentRef.current =
      sessionViewport?.mode === "anchor" ? { kind: "free-scroll" } : { kind: "live-edge" };
    resetActiveMeasurementDeferral({ clearMountedIds: true });
    userScrollInputPendingRef.current = false;
    pendingScrollFrameUserInputRef.current = false;
    autoBackfillRequiresUserScrollRef.current = sessionViewport?.mode === "anchor";
    lastPersistedViewportAnchorSignatureRef.current = null;
    avatarRequestRangeRef.current = EMPTY_TIMELINE_ITEM_INDEX_RANGE;
    setAvatarRequestRange(EMPTY_TIMELINE_ITEM_INDEX_RANGE);
    linkPreviewRequestRangeRef.current = EMPTY_TIMELINE_ITEM_INDEX_RANGE;
    setLinkPreviewRequestRange(EMPTY_TIMELINE_ITEM_INDEX_RANGE);
  }, [
    cancelPendingScrollFrame,
    cancelScrollFollowUpFrames,
    resetActiveMeasurementDeferral,
    timelineKeyHash
  ]);

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
    initialItemsSeenForTimelineKeyRef.current = null;
    emptyThreadBackfillRequestedRef.current = false;
    lastDiagnosticsEmissionRef.current = null;
    initialLiveEdgeScrollAppliedRef.current = null;
    stickToBottomAfterMeasurementRef.current = false;
    resetActiveMeasurementDeferral({ clearMountedIds: true });
    itemHeightByDomIdRef.current = new Map();
    avatarRequestRangeRef.current = EMPTY_TIMELINE_ITEM_INDEX_RANGE;
    setAvatarRequestRange(EMPTY_TIMELINE_ITEM_INDEX_RANGE);
    linkPreviewRequestRangeRef.current = EMPTY_TIMELINE_ITEM_INDEX_RANGE;
    setLinkPreviewRequestRange(EMPTY_TIMELINE_ITEM_INDEX_RANGE);
    roomScrollAnchorRestorePendingRef.current = false;
    suppressScrollAnchorCaptureRef.current = false;
    viewportIntentRef.current =
      sessionViewport?.mode === "anchor" ? { kind: "free-scroll" } : { kind: "live-edge" };
    userScrollInputPendingRef.current = false;
    pendingScrollFrameUserInputRef.current = false;
    autoBackfillRequiresUserScrollRef.current = sessionViewport?.mode === "anchor";
    lastPersistedViewportAnchorSignatureRef.current = null;
    restoredRoomScrollAnchorSignatureRef.current = null;
    if (viewportIntentResizeFrameRef.current !== null) {
      viewportIntentResizeFrameRef.current.cancel();
      viewportIntentResizeFrameRef.current = null;
    }
    if (projectionLayoutFrameRef.current !== null) {
      projectionLayoutFrameRef.current.cancel();
      projectionLayoutFrameRef.current = null;
    }
    pendingProjectionLayoutRef.current = null;
    projectionLayoutRevisionRef.current += 1;
    viewportIntentRevisionRef.current += 1;
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
      autoBackfillRequiresUserScrollRef.current = false;
      lastPersistedViewportAnchorSignatureRef.current = null;
      if (viewportIntentResizeFrameRef.current !== null) {
        viewportIntentResizeFrameRef.current.cancel();
        viewportIntentResizeFrameRef.current = null;
      }
      if (projectionLayoutFrameRef.current !== null) {
        projectionLayoutFrameRef.current.cancel();
        projectionLayoutFrameRef.current = null;
      }
      pendingProjectionLayoutRef.current = null;
      projectionLayoutRevisionRef.current += 1;
      viewportIntentRevisionRef.current += 1;
    },
    [resetActiveMeasurementDeferral]
  );

  useEffect(() => {
    relevantAvatarMxcsRef.current = timelineAvatarMxcsForItems(items, profileUsers);
  }, [items, profileUsers]);
  const visibleItems = useMemo(() => items.filter((item) => !item.is_hidden), [items]);
  // The SDK-owned store stays canonical. Only these presentation rows feed
  // rendering, measuring, and virtualization for an opt-in Room projection.
  const visibleRows = useMemo(
    () =>
      projectTimelineDisplayRows(items, timelineKey, threadRootOrder, threadRootProjections).filter(
        (row) => !row.item.is_hidden
      ),
    [items, threadRootOrder, threadRootProjections, timelineKey]
  );
  const projectionSnapshot = useMemo<TimelineProjectionSnapshot>(
    () => ({
      timelineKeyHash,
      generation,
      signature: timelineProjectionSignature(visibleRows),
      rows: visibleRows
    }),
    [generation, timelineKeyHash, visibleRows]
  );
  const captureProjectionLayoutTransaction = useCallback(
    (previous: TimelineProjectionSnapshot, next: TimelineProjectionSnapshot) => {
      if (!projectionStructureChanged(previous, next)) {
        return;
      }
      if (
        next.timelineKeyHash !== previous.timelineKeyHash ||
        next.generation !== previous.generation
      ) {
        if (projectionLayoutFrameRef.current !== null) {
          projectionLayoutFrameRef.current.cancel();
          projectionLayoutFrameRef.current = null;
        }
        pendingProjectionLayoutRef.current = null;
        projectionLayoutRevisionRef.current += 1;
        return;
      }
      const container = containerRef.current;
      const stableRowIds = stableProjectionAnchorRowIds(previous.rows, next.rows);
      const anchor =
        container && viewportIntentRef.current.kind !== "live-edge"
          ? captureAnchor(container, {
              isEligible: (node) => stableRowIds.has(node.dataset["itemId"] ?? "")
            })
          : null;
      if (projectionLayoutFrameRef.current !== null) {
        projectionLayoutFrameRef.current.cancel();
        projectionLayoutFrameRef.current = null;
      }
      const revision = projectionLayoutRevisionRef.current + 1;
      projectionLayoutRevisionRef.current = revision;
      pendingProjectionLayoutRef.current = {
        timelineKeyHash: next.timelineKeyHash,
        generation: next.generation,
        signature: next.signature,
        revision,
        intentRevision: viewportIntentRevisionRef.current,
        mode: viewportIntentRef.current.kind === "live-edge" ? "live-edge" : "free-scroll",
        anchor
      };
    },
    []
  );
  useLayoutEffect(() => {
    projectionRenderStateRef.current = projectionSnapshot;
  }, [projectionSnapshot]);
  const visibleItemDomIds = useMemo(
    () => new Set(visibleRows.map((row) => row.row_id)),
    [visibleRows]
  );
  visibleItemDomIdsRef.current = visibleItemDomIds;
  const timelineHeightModel = useMemo(
    () =>
      buildTimelineHeightModel(
        visibleRows,
        itemHeightByDomIdRef.current,
        virtualItemHeight
      ),
    [measuredHeightVersion, visibleRows, virtualItemHeight]
  );
  useLayoutEffect(() => {
    rangeModelEpochRef.current += 1;
  }, [timelineHeightModel, visibleRows]);
  const commitVirtualRangeForMetrics = useCallback(
    (metrics: TimelineViewportMetrics) => {
      const nextAvatarRequestRange = calculateTimelineItemIndexRange({
        visibleItemsLength: visibleRows.length,
        metrics,
        model: timelineHeightModel,
        overscanItems: TIMELINE_AVATAR_THUMBNAIL_OVERSCAN_ITEMS
      });
      if (
        !timelineItemIndexRangeEquals(
          avatarRequestRangeRef.current,
          nextAvatarRequestRange
        )
      ) {
        avatarRequestRangeRef.current = nextAvatarRequestRange;
        setAvatarRequestRange(nextAvatarRequestRange);
      }

      const nextLinkPreviewRequestRange = calculateTimelineItemIndexRange({
        visibleItemsLength: visibleRows.length,
        metrics,
        model: timelineHeightModel,
        overscanItems: TIMELINE_LINK_PREVIEW_OVERSCAN_ITEMS
      });
      if (
        !timelineItemIndexRangeEquals(
          linkPreviewRequestRangeRef.current,
          nextLinkPreviewRequestRange
        )
      ) {
        linkPreviewRequestRangeRef.current = nextLinkPreviewRequestRange;
        setLinkPreviewRequestRange(nextLinkPreviewRequestRange);
      }

      const next = calculateTimelineVirtualRange({
        visibleItemsLength: visibleRows.length,
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
    [timelineHeightModel, updateScrollDiagnostics, visibleRows.length]
  );
  const updateViewportMetrics = useCallback(() => {
    const metrics = readViewportMetrics();
    commitVirtualRangeForMetrics(metrics);
  }, [commitVirtualRangeForMetrics, readViewportMetrics]);
  const virtualWindow = useMemo<TimelineVirtualWindow>(() => {
    const range =
      visibleRows.length <= TIMELINE_VIRTUALIZATION_THRESHOLD
        ? {
            virtualized: false,
            startIndex: 0,
            endIndex: visibleRows.length,
            paddingTop: 0,
            paddingBottom: 0
          }
        : virtualRange;

    return {
      ...range,
      items: visibleRows.slice(range.startIndex, range.endIndex)
    };
  }, [virtualRange, visibleRows]);
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
  const sideEffectRows =
    visibleRows.length > TIMELINE_VIRTUALIZATION_THRESHOLD ? virtualWindow.items : visibleRows;
  const sideEffectItems = useMemo(
    () => sideEffectRows.map((row) => row.item),
    [sideEffectRows]
  );
  const avatarSideEffectItems = useMemo(
    () =>
      visibleRows
        .slice(avatarRequestRange.startIndex, avatarRequestRange.endIndex)
        .map((row) => row.item),
    [avatarRequestRange.endIndex, avatarRequestRange.startIndex, visibleRows]
  );
  useEffect(() => {
    const avatarDiagnostics = timelineAvatarDiagnostics(
      visibleRows.map((row) => row.item),
      profileUsers,
      avatarThumbnails
    );
    for (const item of items) {
      if ("Event" in item.id) {
        downloadedEventIdsRef.current.add(item.id.Event.event_id);
      }
    }
    const diagnostics = {
      visibleItems: visibleRows.length,
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
    visibleRows
  ]);
  useEffect(() => {
    // #116 perf gate: skip avatar downloads when disabled (default).
    if (!enableAvatarThumbnailDownloads) {
      return;
    }
    if (!transport.downloadAvatarThumbnail) {
      return;
    }
    for (const item of avatarSideEffectItems) {
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
    avatarSideEffectItems,
    emitDiagnosticLog,
    enableAvatarThumbnailDownloads,
    profileUsers,
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
  // Read receipts and fully-read state remain canonical timeline facts. A
  // moved root only changes presentation; it must not cause the root id to be
  // sent as the room's latest readable event.
  const latestReadableEventId = latestEventBackedItemId(items);
  const timelineInitialized = Boolean(timelineKeyState && !timelineKeyState.awaitingResync);
  // Stable, render-visible timeline generation for this key. Bumps when the
  // store replaces the list for a new generation (InitialItems / resync), so
  // tests can poll a concrete attribute instead of sleeping. 0 is a valid
  // Core generation; use timelineInitialized to distinguish "not initialized".
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
        message: "operation=request_keys stage=request"
      });
      void transport.requestRoomKey(targetRoomId, eventId).catch(() => {
        onDiagnosticLogEntry?.({
          timestampMs: Date.now(),
          source: "e2ee.room_key",
          message: "operation=request_keys stage=failed kind=transport"
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
  const timelineDiagnosticKind = timelineKindDiagnosticLabel(timelineKey);
  const onLoadLinkPreviews = useCallback(
    (targetRoomId: string, eventId: string, pendingCount = 0) => {
      onDiagnosticLogEntry?.({
        timestampMs: Date.now(),
        source: "timeline.preview",
        message: `kind=${timelineDiagnosticKind} stage=request trigger=viewport_pending pending=${pendingCount}`
      });
      void transport.loadLinkPreviews?.(targetRoomId, eventId)?.catch(() => {
        onDiagnosticLogEntry?.({
          timestampMs: Date.now(),
          source: "timeline.preview",
          message: `kind=${timelineDiagnosticKind} stage=failed trigger=viewport_pending`
        });
      });
    },
    [onDiagnosticLogEntry, timelineDiagnosticKind, transport]
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
      closeAliasDialog();
    },
    [closeAliasDialog]
  );
  const updateAliasDraft = useCallback(
    (nextAlias: string) => {
      setAliasDraft(nextAlias);
      if (!aliasTarget || !onSetLocalUserAlias) {
        return;
      }
      onSetLocalUserAlias(aliasTarget.userId, nextAlias.trim() || null);
    },
    [aliasTarget, onSetLocalUserAlias]
  );
  const effectiveForwardDestinations =
    forwardDestinations.length > 0
      ? forwardDestinations
      : [{ room_id: roomId, display_name: roomId }];
  const sendReadSignalsForEvent = useCallback(
    (eventId: string) => {
      const signalKey = `${roomId}\u0000${readSignalThreadRootEventId ?? ""}\u0000${eventId}`;
      if (readSignalEventRef.current === signalKey) {
        return;
      }
      readSignalEventRef.current = signalKey;
      const sendReadReceipt =
        readSignalThreadRootEventId === null
          ? transport.sendReadReceipt(roomId, eventId)
          : transport.sendReadReceipt(roomId, eventId, readSignalThreadRootEventId);
      void sendReadReceipt.catch(() => undefined);
      void transport.setFullyRead(roomId, eventId).catch(() => undefined);
    },
    [readSignalThreadRootEventId, roomId, transport]
  );
  const reportViewportObservation = useCallback(() => {
    const observeViewport = transport.observeViewport;
    const canObserveRoomViewport = Boolean(observeViewport && roomTimelineRoomId === roomId);
    const canSendReadSignals = readSignalRoomId === roomId;
    if (!canObserveRoomViewport && !canSendReadSignals) {
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
    const latestVisible =
      latestReadableEventId !== null &&
      visible.lastVisibleEventId === latestReadableEventId;
    const effectiveAtBottom = atBottom || latestVisible;
    setViewportAtBottom((current) =>
      current === effectiveAtBottom ? current : effectiveAtBottom
    );
    if (canSendReadSignals && effectiveAtBottom && latestReadableEventId) {
      sendReadSignalsForEvent(latestReadableEventId);
    }
    if (!canObserveRoomViewport || !observeViewport) {
      return;
    }
    const signature = [
      roomId,
      visible.firstVisibleEventId ?? "",
      visible.lastVisibleEventId ?? "",
      effectiveAtBottom ? "bottom" : "not-bottom"
    ].join("\u0000");
    if (lastViewportObservationRef.current === signature) {
      return;
    }
    lastViewportObservationRef.current = signature;
    void observeViewport(
        roomId,
        visible.firstVisibleEventId,
        visible.lastVisibleEventId,
        effectiveAtBottom
      )
      .catch(() => undefined);
  }, [
    latestReadableEventId,
    readSignalRoomId,
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
      if (viewportIntentRef.current.kind === "live-edge") {
        if (viewportIntentResizeFrameRef.current !== null) {
          viewportIntentResizeFrameRef.current.cancel();
        }
        viewportIntentResizeFrameRef.current = scheduleTimelineFrame(() => {
          viewportIntentResizeFrameRef.current = null;
          const changed = applyViewportIntent();
          if (!changed) {
            return;
          }
          updateViewportMetrics();
          reportViewportObservation();
        });
        return;
      }
      // Free-scroll: `.timeline-view` is `overflow-anchor: none`, so the browser
      // no longer corrects scroll position when an above-viewport row resizes.
      // Restore the last captured first-visible anchor so the visible row stays
      // put. Prepend restoration owns its own correction, so defer while pending.
      const container = containerRef.current;
      const anchor = freeScrollAnchorRef.current;
      if (
        !container ||
        !anchor ||
        anchorRestorePendingRef.current ||
        roomScrollAnchorRestorePendingRef.current ||
        jumpViewportControlRef.current
      ) {
        return;
      }
      if (viewportIntentResizeFrameRef.current !== null) {
        viewportIntentResizeFrameRef.current.cancel();
      }
      viewportIntentResizeFrameRef.current = scheduleTimelineFrame(() => {
        viewportIntentResizeFrameRef.current = null;
        runWithScrollWriteReason("backfillCompensation", () => {
          restoreAnchor(container, anchor);
        });
        updateViewportMetrics();
        reportViewportObservation();
      });
    });

    observer.observe(list);
    return () => {
      observer.disconnect();
      if (viewportIntentResizeFrameRef.current !== null) {
        viewportIntentResizeFrameRef.current.cancel();
        viewportIntentResizeFrameRef.current = null;
      }
    };
  }, [
    applyViewportIntent,
    reportViewportObservation,
    runWithScrollWriteReason,
    timelineKeyHash,
    updateViewportMetrics
  ]);

  useEffect(() => {
    if (!latestReadableEventId || readSignalRoomId !== roomId) {
      return;
    }
    const container = containerRef.current;
    if (!container || !viewportAtBottom || !isScrolledToBottom(container)) {
      return;
    }
    sendReadSignalsForEvent(latestReadableEventId);
  }, [
    latestReadableEventId,
    readSignalRoomId,
    roomId,
    sendReadSignalsForEvent,
    viewportAtBottom
  ]);

  useLayoutEffect(() => {
    const transaction = pendingProjectionLayoutRef.current;
    if (
      transaction === null ||
      transaction.timelineKeyHash !== projectionSnapshot.timelineKeyHash ||
      transaction.generation !== projectionSnapshot.generation ||
      transaction.signature !== projectionSnapshot.signature
    ) {
      return;
    }
    pendingProjectionLayoutRef.current = null;
    const scheduledTransaction = transaction;
    projectionLayoutFrameRef.current = scheduleTimelineFrame(() => {
      if (projectionLayoutFrameRef.current !== null) {
        projectionLayoutFrameRef.current = null;
      }
      const current = projectionRenderStateRef.current;
      if (
        current === null ||
        projectionLayoutRevisionRef.current !== scheduledTransaction.revision ||
        viewportIntentRevisionRef.current !== scheduledTransaction.intentRevision ||
        current.timelineKeyHash !== scheduledTransaction.timelineKeyHash ||
        current.generation !== scheduledTransaction.generation ||
        current.signature !== scheduledTransaction.signature ||
        (scheduledTransaction.mode === "live-edge" &&
          viewportIntentRef.current.kind !== "live-edge") ||
        (scheduledTransaction.mode === "free-scroll" &&
          (viewportIntentRef.current.kind !== "free-scroll" || jumpViewportControlRef.current))
      ) {
        return;
      }
      const container = containerRef.current;
      if (!container) {
        return;
      }
      if (scheduledTransaction.mode === "live-edge") {
        runWithScrollWriteReason("projectionCompensation", () => {
          scrollContainerToBottom(container);
        });
      } else if (scheduledTransaction.anchor !== null) {
        let restored = false;
        runWithScrollWriteReason("projectionCompensation", () => {
          restored = restoreAnchor(container, scheduledTransaction.anchor!);
        });
        if (!restored && virtualWindow.virtualized) {
          const anchorIndex = visibleRows.findIndex(
            (row) => row.row_id === scheduledTransaction.anchor?.itemId
          );
          if (anchorIndex >= 0) {
            runWithScrollWriteReason("projectionCompensation", () => {
              container.scrollTop = Math.max(
                0,
                viewportMetricsRef.current.listOffsetTop +
                  (timelineHeightModel.offsets[anchorIndex] ?? 0) -
                  scheduledTransaction.anchor!.offsetTop
              );
            });
          }
        }
      }
      if (viewportIntentRef.current.kind !== "live-edge") {
        freeScrollAnchorRef.current = captureFreeScrollAnchor(container);
      }
      updateViewportMetrics();
      reportViewportObservation();
    });
  }, [
    projectionSnapshot,
    reportViewportObservation,
    runWithScrollWriteReason,
    timelineHeightModel,
    updateViewportMetrics,
    virtualWindow.virtualized,
    visibleRows
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

      const anchorIsLive = canonicalTimelineContainsActivityEventId(
        items,
        activeRoomAnchor.event_id
      );
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
          const anchorIndex = visibleRows.findIndex(
            (row) => row.activity_event_id === activeRoomAnchor.event_id
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
            scheduleScrollFollowUpFrame(() => {
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
        const anchorIndex = visibleRows.findIndex((row) => row.row_id === anchor.itemId);
        if (anchorIndex >= 0) {
          runWithScrollWriteReason("backfillCompensation", () => {
            container.scrollTop = Math.max(
              0,
              viewportMetricsRef.current.listOffsetTop +
                (timelineHeightModel.offsets[anchorIndex] ?? 0) -
                anchor.offsetTop
            );
          });
          scheduleScrollFollowUpFrame(() => {
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
    } else if (
      container &&
      !anchorRestorePendingRef.current &&
      !roomScrollAnchorRestorePendingRef.current &&
      !jumpViewportControlRef.current
    ) {
      // Refresh the free-scroll anchor after layout/measurement commits so a
      // later above-viewport resize (overflow-anchor: none) restores the
      // viewport against the settled position rather than a stale pre-measure one.
      freeScrollAnchorRef.current = captureFreeScrollAnchor(container);
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
    navigationSnapshot,
    reportViewportObservation,
    timelineHeightModel,
    timelineInitialized,
    updateViewportMetrics,
    virtualWindow.virtualized,
    visibleRows,
    runWithScrollWriteReason,
    scheduleScrollFollowUpFrame,
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

  useLayoutEffect(() => {
    if (!timelineInitialized || roomTimelineRoomId === null || suppressPaginationUi) {
      return;
    }
    const container = containerRef.current;
    if (!container) {
      return;
    }
    const clientHeight = Math.round(container.clientHeight);
    if (clientHeight <= 0) {
      return;
    }
    const scrollHeight = Math.round(container.scrollHeight);
    const overflowPx = Math.max(0, scrollHeight - clientHeight);
    if (overflowPx > SCROLL_EDGE_TOLERANCE_PX) {
      return;
    }
    const stateLabel = paginationStateDiagnosticLabel(backwardState);
    const skipReasons = [
      !autoLoadOlderMessages ? "auto_load_disabled" : null,
      anchorRestorePendingRef.current ? "anchor_restore" : null,
      roomScrollAnchorRestorePendingRef.current ? "room_anchor_restore" : null,
      autoBackfillRequiresUserScrollRef.current ? "await_user_scroll_after_room_restore" : null,
      backfillInFlightRef.current ? "in_flight" : null,
      shouldSuppressAutoBackfill(store, timelineKeyRef.current) ? "pagination_state" : null
    ].filter((reason): reason is string => reason !== null);
    const stage = skipReasons.length > 0 ? "skip" : "request";
    const reason = skipReasons.length > 0 ? ` reason=${skipReasons.join("+")}` : "";
    const signature = [
      timelineKeyHash,
      generation,
      items.length,
      scrollHeight,
      clientHeight,
      autoLoadOlderMessages ? "auto" : "manual",
      stateLabel,
      stage,
      reason
    ].join("\u0000");
    if (underfilledInitialBackfillDiagnosticSignatureRef.current === signature) {
      return;
    }
    underfilledInitialBackfillDiagnosticSignatureRef.current = signature;
    const message = `${stage === "request" ? "stage=request" : "stage=skip"} trigger=underfilled_initial${reason} items=${items.length} scroll_height_px=${scrollHeight} client_height_px=${clientHeight} overflow_px=${overflowPx} auto_load=${autoLoadOlderMessages} state=${stateLabel}`;
    emitDiagnosticLog(
      "timeline.backfill",
      message
    );
    if (stage !== "request") {
      return;
    }
    backfillInFlightRef.current = true;
    void transport
      .paginateBackwards(timelineKeyRef.current)
      .catch(() => {
        emitDiagnosticLog(
          "timeline.backfill",
          "stage=failed trigger=underfilled_initial reason=transport"
        );
        backfillInFlightRef.current = false;
      });
  }, [
    autoLoadOlderMessages,
    backwardState,
    emitDiagnosticLog,
    generation,
    items.length,
    roomTimelineRoomId,
    store,
    suppressPaginationUi,
    timelineInitialized,
    timelineKeyHash,
    transport
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
    const scrollTopPx = Math.round(container.scrollTop);
    const thresholdPx = Math.round(backfillThreshold);
    const maxScrollTopPx = Math.round(maxScrollTop);
    const skipReasons = [
      anchorRestorePendingRef.current ? "anchor_restore" : null,
      roomScrollAnchorRestorePendingRef.current ? "room_anchor_restore" : null,
      autoBackfillRequiresUserScrollRef.current ? "await_user_scroll_after_room_restore" : null,
      backfillInFlightRef.current ? "in_flight" : null
    ].filter((reason): reason is string => reason !== null);
    if (skipReasons.length > 0) {
      emitDiagnosticLog(
        "timeline.backfill",
        `stage=skip trigger=scroll reason=${skipReasons.join("+")} scroll_top_px=${scrollTopPx} threshold_px=${thresholdPx} max_scroll_top_px=${maxScrollTopPx}`
      );
      return;
    }
    if (shouldSuppressAutoBackfill(store, timelineKeyRef.current)) {
      emitDiagnosticLog(
        "timeline.backfill",
        `stage=skip trigger=scroll reason=pagination_state scroll_top_px=${scrollTopPx} threshold_px=${thresholdPx} max_scroll_top_px=${maxScrollTopPx}`
      );
      return;
    }
    backfillInFlightRef.current = true;
    emitDiagnosticLog(
      "timeline.backfill",
      `stage=request trigger=scroll scroll_top_px=${scrollTopPx} threshold_px=${thresholdPx} max_scroll_top_px=${maxScrollTopPx} auto_load=${autoLoadOlderMessages}`
    );
    void transport
      .paginateBackwards(timelineKeyRef.current)
      .catch(() => {
        emitDiagnosticLog("timeline.backfill", "stage=failed trigger=scroll reason=transport");
        backfillInFlightRef.current = false;
      });
  }, [
    store,
    transport,
    suppressPaginationUi,
    autoLoadOlderMessages,
    virtualItemHeight,
    emitDiagnosticLog
  ]);
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
        // A genuine user scroll takes viewport control back from any jump.
        jumpViewportControlRef.current = false;
        autoBackfillRequiresUserScrollRef.current = false;
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
      // In free-scroll, remember the first-visible anchor synchronously (not in
      // the range-gated frame below) so the ResizeObserver can restore it when
      // an above-viewport row resizes under `overflow-anchor: none`. Skip while
      // a jump owns the viewport so its re-centering is not fought.
      if (
        viewportIntentRef.current.kind !== "live-edge" &&
        !jumpViewportControlRef.current
      ) {
        freeScrollAnchorRef.current = captureFreeScrollAnchor(container);
      }
    }
    if (pendingScrollFrameRef.current === null) {
      const frameTimelineKeyHash = timelineKeyHash;
      const frameRangeModelEpoch = rangeModelEpochRef.current;
      pendingScrollFrameRef.current = scheduleTimelineFrame(() => {
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
    emitDiagnosticLog("timeline.backfill", "stage=request trigger=empty_thread");
    void transport
      .paginateBackwards(timelineKeyRef.current)
      .catch(() => {
        emitDiagnosticLog("timeline.backfill", "stage=failed trigger=empty_thread reason=transport");
        backfillInFlightRef.current = false;
      });
  }, [
    endReached,
    emitDiagnosticLog,
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
      jumpViewportControlRef.current = true;
      viewportIntentRevisionRef.current += 1;
      const container = containerRef.current;
      const scrollMountedRowIntoView = () => {
        const row = container ? findTimelineEventNode(container, "activity", eventId) : null;
        row?.scrollIntoView({ block: "center", inline: "nearest" });
      };
      const row = container ? findTimelineEventNode(container, "activity", eventId) : null;
      if (row) {
        runWithScrollWriteReason("jumpToEvent", () => {
          row.scrollIntoView({ block: "center", inline: "nearest" });
        });
        updateViewportMetrics();
        reportViewportObservation();
        return;
      }
      if (container && virtualWindow.virtualized) {
        const itemIndex = visibleRows.findIndex(
          (row) => row.activity_event_id === eventId
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
          scheduleScrollFollowUpFrame(() => {
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
      scheduleScrollFollowUpFrame,
      reportViewportObservation,
      timelineHeightModel,
      updateViewportMetrics,
      virtualWindow.virtualized,
      visibleRows
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
    viewportIntentRevisionRef.current += 1;
    setViewportIntentToLiveEdge();
    runWithScrollWriteReason("jumpToBottom", () => {
      scrollContainerToBottom(container);
    });
    updateViewportMetrics();
    reportViewportObservation();
    scheduleScrollFollowUpFrame(() => {
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
    scheduleScrollFollowUpFrame,
    updateViewportMetrics
  ]);
  useEffect(() => {
    onRegisterJumpToLatest?.(jumpToBottom);
    return () => onRegisterJumpToLatest?.(null);
  }, [jumpToBottom, onRegisterJumpToLatest]);
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
    <ProjectionSnapshotBoundary
      snapshot={projectionSnapshot}
      onBeforeProjectionChange={captureProjectionLayoutTransaction}
    >
      <div
        className="timeline-view"
        data-testid="timeline-view"
        data-end-reached={endReached || undefined}
        data-timeline-generation={generation}
        data-virtualized={virtualWindow.virtualized || undefined}
        data-total-items={visibleRows.length}
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
      {isAnchored && onReturnToLive ? (
        <div className="timeline-navigation-bar">
          <div className="timeline-navigation-pills">
            <button
              className="timeline-navigation-pill"
              type="button"
              onClick={onReturnToLive}
            >
              <ArrowDown size={14} aria-hidden="true" />
              <span>{t("shortcut.jumpToLatestMessage")}</span>
            </button>
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
        {virtualWindow.items.map((row, windowIndex) => {
          const { item } = row;
          const visibleIndex = virtualWindow.startIndex + windowIndex;
          const contentEventId = row.content_event_id;
          const activityEventId = row.activity_event_id;
          const isUnreadMarker = Boolean(
            activityEventId && unreadMarkerEventId === activityEventId
          );
          const isReadMarker = Boolean(
            activityEventId &&
              readMarkerDisplayEventId === activityEventId &&
              !unreadMarkerEventId
          );
          return (
            <div
              className="timeline-item-frame"
              key={row.row_id}
              data-frame-item-id={row.row_id}
            >
              {isUnreadMarker ? (
                <div className="read-marker" role="separator" aria-label={t("timeline.unreadMarker")}>
                  <span>{t("timeline.unreadMarker")}</span>
                </div>
              ) : null}
              {row.kind === "threadRootPending" || row.kind === "threadRootFailed" ? (
                <ThreadRootProjectionPlaceholder
                  row={row}
                  state={row.kind === "threadRootPending" ? "pending" : "failed"}
                />
              ) : (
                <TimelineItemRow
                item={item}
                rowId={row.row_id}
                contentEventId={contentEventId}
                activityEventId={activityEventId}
                contentTimestampMs={row.content_timestamp_ms}
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
                isPinned={contentEventId ? pinnedEventIds.includes(contentEventId) : false}
                onPin={onPin}
                onUnpin={onUnpin}
                onDownloadMedia={onDownloadMedia}
                onLoadMessageSource={onLoadMessageSource}
                onRequestRoomKey={onRequestRoomKey}
                onForwardMessage={onForwardMessage}
                autoLoadLinkPreviews={timelineItemIndexInRange(
                  visibleIndex,
                  linkPreviewRequestRange
                )}
                onLoadLinkPreviews={onLoadLinkPreviews}
                onHideLinkPreview={onHideLinkPreview}
                onCopyText={onCopyText}
                onOpenAliasDialog={onSetLocalUserAlias ? openAliasDialog : undefined}
                onOpenMediaViewer={openMediaViewer}
                onSaveMediaFile={transport.saveMediaFile}
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
                threadAttention={threadAttention}
                mediaDownload={contentEventId ? mediaDownloads[contentEventId] : undefined}
                receipts={
                  contentEventId
                    ? roomSignals?.receipts_by_event[contentEventId]?.readers ?? []
                    : []
                }
                receiptTotalCount={
                  contentEventId
                    ? roomSignals?.receipts_by_event[contentEventId]?.total_count ?? 0
                    : 0
                }
                receiptOverflowCount={
                  contentEventId
                    ? roomSignals?.receipts_by_event[contentEventId]?.overflow_count ?? 0
                    : 0
                }
                />
              )}
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
      {mediaViewerItem ? (
        <TimelineMediaViewer
          item={mediaViewerItem}
          onClose={closeMediaViewer}
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
              onChange={(event) => updateAliasDraft(event.currentTarget.value)}
              autoFocus
            />
            <div className="dialog-actions">
              <button className="dialog-button is-primary" type="submit">
                {t("action.done")}
              </button>
            </div>
          </form>
        </div>
      ) : null}
      </div>
    </ProjectionSnapshotBoundary>
  );
});

/**
 * A projection-only placeholder for an old root which was intentionally kept
 * out of canonical SDK items. Its stable row/content/activity identities let
 * virtual layout and viewport observation continue to use the latest reply
 * while the bounded root request settles. It has no message actions, so a
 * pending or terminal failure can never trigger navigation/pagination.
 */
function ThreadRootProjectionPlaceholder({
  row,
  state
}: {
  row: TimelineDisplayRow;
  state: "pending" | "failed";
}) {
  const summary = row.item.thread_summary;
  const replyCount = summary?.reply_count ?? 0;
  return (
    <article
      className="message thread-root-projection-placeholder"
      data-item-id={row.row_id}
      data-row-id={row.row_id}
      data-content-event-id={row.content_event_id ?? undefined}
      data-activity-event-id={row.activity_event_id ?? undefined}
      data-event-id={row.activity_event_id ?? undefined}
      data-thread-root-projection-state={state}
    >
      <p className="timeline-thread-root-projection-status" role="status">
        {state === "pending"
          ? t("timeline.threadRootLoading")
          : t("timeline.threadRootUnavailable")}
      </p>
      <span className="thread-reply-count">
        {replyCount === 1 ? "1 reply" : `${replyCount} replies`}
      </span>
    </article>
  );
}

export function TimelineItemRow({
  item,
  rowId,
  contentEventId,
  activityEventId,
  contentTimestampMs,
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
  autoLoadLinkPreviews = false,
  onLoadLinkPreviews = () => undefined,
  onHideLinkPreview = () => undefined,
  onCopyText = () => undefined,
  onOpenAliasDialog,
  onOpenMediaViewer = () => undefined,
  onSaveMediaFile,
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
  threadAttention = null,
  mediaDownload
}: {
  item: TimelineItem;
  /** Stable presentation identity used by DOM/virtualization rows. */
  rowId?: string;
  /** Root/content identity for every message action. */
  contentEventId?: string | null;
  /** Latest-activity identity for Room viewport observation. */
  activityEventId?: string | null;
  /** The content event timestamp, independent of presentation placement. */
  contentTimestampMs?: number | null;
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
  autoLoadLinkPreviews?: boolean;
  onLoadLinkPreviews?: TimelineRowActionHandlers["onLoadLinkPreviews"];
  onHideLinkPreview?: TimelineRowActionHandlers["onHideLinkPreview"];
  onCopyText?: TimelineRowActionHandlers["onCopyText"];
  onOpenAliasDialog?: (target: TimelineAliasTarget) => void;
  onOpenMediaViewer?: (item: TimelineMediaViewerItem) => void;
  onSaveMediaFile?: TimelineTransport["saveMediaFile"];
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
  threadAttention?: TimelineThreadAttention | null;
  mediaDownload?: TimelineMediaDownloadState;
}) {
  const itemDomId = timelineItemDomId(item.id);
  const domId = rowId ?? itemDomId;
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
  const itemEventId = "Event" in item.id ? item.id.Event.event_id : null;
  const eventId = contentEventId ?? itemEventId;
  const activityId = activityEventId ?? eventId;
  const isRedacted = item.is_redacted;
  const sendState = item.send_state ?? null;
  const sendStateKind = sendState?.kind ?? null;
  const messageKind = item.message_kind ?? "text";
  const [isEditing, setEditing] = useState(false);
  const [editDraft, setEditDraft] = useState(item.body ?? "");
  const [isReactionPickerOpen, setReactionPickerOpen] = useState(false);
  const [reactionPickerLayout, setReactionPickerLayout] = useState<ReactionPickerLayout>({
    placement: "above",
    maxBlockSize: REACTION_PICKER_COMFORTABLE_BLOCK_SIZE_PX
  });
  const [isActionMenuOpen, setActionMenuOpen] = useState(false);
  const [isForwardMenuOpen, setForwardMenuOpen] = useState(false);
  const [actionMenuPlacement, setActionMenuPlacement] = useState<"above" | "below">("above");
  const [revealedSpoilers, setRevealedSpoilers] = useState<ReadonlySet<string>>(
    () => new Set()
  );
  const reactionControlRef = useRef<HTMLDivElement>(null);
  const reactionTriggerRef = useRef<HTMLButtonElement>(null);
  const actionMenuControlRef = useRef<HTMLDivElement>(null);
  const actionMenuTriggerRef = useRef<HTMLButtonElement>(null);
  const firstActionMenuItemRef = useRef<HTMLButtonElement>(null);
  const editTextareaRef = useRef<HTMLTextAreaElement>(null);
  const editImeCompositionActiveRef = useRef(false);
  const editMacKillRingRef = useRef<string>("");
  const requestedLinkPreviewsRef = useRef<Set<string>>(new Set());

  useEffect(() => {
    if (!autoLoadLinkPreviews) {
      return;
    }
    const pendingCount =
      item.link_previews?.filter((preview) => preview.state === "pending").length ?? 0;
    if (!eventId || pendingCount === 0) {
      return;
    }
    if (requestedLinkPreviewsRef.current.has(eventId)) {
      return;
    }
    requestedLinkPreviewsRef.current.add(eventId);
    onLoadLinkPreviews(roomId, eventId, pendingCount);
  }, [autoLoadLinkPreviews, eventId, item.link_previews, onLoadLinkPreviews, roomId]);

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

  const updateReactionPickerLayout = useCallback(() => {
    const control = reactionControlRef.current;
    if (control) {
      setReactionPickerLayout(reactionPickerLayoutForControl(control));
    }
  }, []);

  useEffect(() => {
    if (!isReactionPickerOpen) {
      return;
    }
    window.addEventListener("resize", updateReactionPickerLayout);
    document.addEventListener("scroll", updateReactionPickerLayout, true);
    return () => {
      window.removeEventListener("resize", updateReactionPickerLayout);
      document.removeEventListener("scroll", updateReactionPickerLayout, true);
    };
  }, [isReactionPickerOpen, updateReactionPickerLayout]);

  const closeReactionPicker = useCallback(() => {
    setReactionPickerOpen(false);
    reactionTriggerRef.current?.focus();
  }, []);

  const toggleReactionPicker = useCallback(() => {
    updateReactionPickerLayout();
    setActionMenuOpen(false);
    setForwardMenuOpen(false);
    setReactionPickerOpen((current) => !current);
  }, [updateReactionPickerLayout]);

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
    canSetSenderAlias ||
    canCopyMessage ||
    canCopyPermalink ||
    canViewSource ||
    canForward;
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
  const newThreadReplyCount =
    eventId && threadAttention?.rootEventId === eventId
      ? threadAttention.liveEventMarkerCount
      : 0;
  const newThreadRepliesText =
    newThreadReplyCount > 0
      ? t("timeline.viewReplies", { count: newThreadReplyCount })
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
        onOpenMediaViewer={onOpenMediaViewer}
        onSaveMediaFile={onSaveMediaFile}
        viewerActions={{
          canForward,
          forwardDestinations,
          onForward: submitForward,
          canViewSource,
          onViewSource: loadMessageSource,
          canRedact: Boolean(canShowActionButtons && item.can_redact),
          onRedact: submitRedaction
        }}
      />
    ) : null;
  function handleContextMenu(event: MouseEvent<HTMLElement>) {
    if (!onOpenContextMenu || !eventId || !item.sender) {
      return;
    }
    const items = contextMenuItems({
      kind: "message",
      canManage: currentUserId === item.sender,
      canReply: canShowReply,
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
      data-row-id={domId}
      data-content-event-id={eventId ?? undefined}
      data-activity-event-id={activityId ?? undefined}
      data-send-state={sendStateKind ?? undefined}
      data-event-id={activityId ?? undefined}
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
            timestampMs={contentTimestampMs ?? item.timestamp_ms ?? null}
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
        {newThreadReplyCount > 0 ? (
          <button
            className="thread-summary-chip thread-new-replies-chip"
            type="button"
            aria-label={t("timeline.openThreadSummary", { summary: newThreadRepliesText })}
            onClick={submitOpenThread}
          >
            <MessageCircle size={13} />
            <span>{newThreadRepliesText}</span>
          </button>
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
              aria-haspopup="dialog"
              onClick={toggleReactionPicker}
            >
              <SmilePlus size={14} />
            </button>
            {isReactionPickerOpen ? (
              <Suspense fallback={null}>
                <LazyEmojiPicker
                  anchorRef={reactionTriggerRef}
                  align="end"
                  placement={reactionPickerLayout.placement}
                  style={{
                    "--emoji-picker-max-block-size": `${reactionPickerLayout.maxBlockSize}px`
                  } as CSSProperties}
                  className="timeline-reaction-emoji-picker"
                  onSelect={submitReaction}
                  onClose={closeReactionPicker}
                />
              </Suspense>
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
                    ref={
                      !senderAliasTarget && !canCopyMessage
                        ? firstActionMenuItemRef
                        : undefined
                    }
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
    const linkPreviewSummary = timelineDiffLinkPreviewSummary(event.ItemsUpdated.diffs);
    if (linkPreviewSummary.items > 0) {
      emit(
        "timeline.preview",
        `kind=${kind} stage=update items=${linkPreviewSummary.items} pending=${linkPreviewSummary.pending} loading=${linkPreviewSummary.loading} ready=${linkPreviewSummary.ready} failed=${linkPreviewSummary.failed}`
      );
    }
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

function timelineDiffLinkPreviewSummary(diffs: readonly TimelineDiff[]): {
  items: number;
  pending: number;
  loading: number;
  ready: number;
  failed: number;
} {
  const summary = {
    items: 0,
    pending: 0,
    loading: 0,
    ready: 0,
    failed: 0
  };
  for (const diff of diffs) {
    for (const item of timelineDiffItems(diff)) {
      const previews = item.link_previews ?? [];
      if (previews.length === 0) {
        continue;
      }
      summary.items += 1;
      for (const preview of previews) {
        summary[preview.state] += 1;
      }
    }
  }
  return summary;
}

function timelineBackfillCompletionReason(event: TimelineEvent): string | null {
  if ("InitialItems" in event || "ResyncRequired" in event) {
    return "reset";
  }
  if ("ItemsUpdated" in event) {
    return batchContainsPrepend(event.ItemsUpdated.diffs) ? "prepend" : null;
  }
  if ("PaginationStateChanged" in event) {
    if (
      event.PaginationStateChanged.direction !== "Backward" ||
      event.PaginationStateChanged.state === "Paginating"
    ) {
      return null;
    }
    return paginationStateBackfillCompletionReason(event.PaginationStateChanged.state);
  }
  return null;
}

function paginationStateBackfillCompletionReason(state: PaginationState): string {
  if (state === "Idle") {
    return "pagination_idle";
  }
  if (state === "EndReached") {
    return "pagination_end_reached";
  }
  return "pagination_failed";
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
  const readyDisplayBox =
    downloadState?.kind === "ready"
      ? timelineMediaDisplayBox(
          downloadState.width ?? media.width,
          downloadState.height ?? media.height
        )
      : displayBox;
  const mediaFrameStyle =
    readyDisplayBox === null
      ? undefined
      : ({
          inlineSize: `${readyDisplayBox.inlineSize}px`,
          blockSize: `${readyDisplayBox.blockSize}px`
        } satisfies CSSProperties);
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
  useEffect(() => {
    if (!detailsOpen) {
      return;
    }
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        setDetailsOpen(false);
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [detailsOpen]);

  // #163: a ready image is rendered image-first. The preview is the primary
  // block; filename/metadata are not laid out over it, and actions appear on
  // hover/focus as an overlay. The encrypted badge stays visible as a security
  // signal.
  if (readyImagePreview && readyImageDownload) {
    return (
      <div
        className="message-media message-media-image-ready"
        data-media-kind={media.kind}
        data-media-encrypted={media.source.encrypted || undefined}
        data-download-state={downloadState?.kind ?? "notRequested"}
      >
        <div className="message-media-figure" style={mediaFrameStyle}>
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
          {media.source.encrypted ? (
            <span className="message-media-image-badge">{t("timeline.encryptedMedia")}</span>
          ) : null}
          <div className="message-media-hover-actions">
            <button
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
            {canDownload ? (
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
            ) : null}
          </div>
          {detailsOpen ? (
            <div
              className="message-media-details-popover"
              role="dialog"
              aria-label={t("timeline.mediaDetailsTitle")}
            >
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
            </div>
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
      {media.kind === "Image" && displayBox ? (
        <span
          className="message-media-image-frame message-media-image-frame-reserved"
          style={mediaFrameStyle}
          aria-hidden="true"
        >
          <Icon className="message-media-icon" size={22} aria-hidden="true" />
        </span>
      ) : (
        <Icon className="message-media-icon" size={18} aria-hidden="true" />
      )}
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
            onClick={onDownload}
          >
            <Download size={15} />
          </button>
        )
      ) : null}
    </div>
  );
}

function TimelineMediaViewer({
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
    function onKeyDown(event: globalThis.KeyboardEvent) {
      if (event.key === "Escape") {
        if (isActionMenuOpen) {
          closeActionMenu();
          return;
        }
        onClose();
      }
      if (event.key === "Tab") {
        const dialog = dialogRef.current;
        if (!dialog) {
          return;
        }
        const focusable = Array.from(
          dialog.querySelectorAll<HTMLElement>(
            'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'
          )
        ).filter((element) => !element.hasAttribute("aria-hidden"));
        if (focusable.length === 0) {
          event.preventDefault();
          dialog.focus();
          return;
        }
        const first = focusable[0]!;
        const last = focusable[focusable.length - 1]!;
        if (event.shiftKey && document.activeElement === first) {
          event.preventDefault();
          last.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault();
          first.focus();
        }
      }
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [closeActionMenu, isActionMenuOpen, onClose]);

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
    <div
      className="timeline-media-viewer-overlay"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) {
          onClose();
        }
      }}
    >
      <section
        ref={dialogRef}
        className="timeline-media-viewer"
        role="dialog"
        aria-modal="true"
        aria-label={t("timeline.mediaViewer")}
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
                      if (event.key === "Escape") {
                        event.preventDefault();
                        closeActionMenu();
                      }
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

const TIMELINE_MEDIA_MAX_INLINE_PX = 420;
const TIMELINE_MEDIA_MAX_BLOCK_PX = 260;

function timelineMediaDisplayBox(
  width: number | null | undefined,
  height: number | null | undefined
): { inlineSize: number; blockSize: number } | null {
  if (!width || !height || width <= 0 || height <= 0) {
    return null;
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
