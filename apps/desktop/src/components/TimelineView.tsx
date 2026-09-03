import {
ThreadRootStatusPlaceholder,
TimelineItemRow,
aliasTargetIsActive,
type TimelineAliasTarget,
type TimelineRowActionHandlers,
type TimelineThreadAttention
} from "./timeline/TimelineItemRow";
import type { TimelineTransport } from "./timeline/TimelineTransport";
import { useTimelineEventSubscription } from "./timeline/useTimelineEventSubscription";
import {
  ProjectionSnapshotBoundary,
  projectionStructureChanged,
  stableProjectionAnchorRowIds,
  timelineProjectionSignature,
  type PendingProjectionLayoutTransaction,
  type TimelineProjectionSnapshot
} from "./timeline/TimelineProjectionBoundary";
export {
  clearTimelineViewportSessionMemoryForTests,
  setTimelineViewportSessionAnchorForTests
} from "./timeline/TimelineViewportAnchors";
import {
  isScrolledToBottom,
  scrollContainerToBottom,
  SCROLL_EDGE_TOLERANCE_PX,
  timelineBackfillThreshold,
  timelineKeyShouldReleaseViewportIntent,
  visibleTimelineViewportFacts
} from "./timeline/TimelineViewportObservation";
export { timelineBackfillThresholdForTests } from "./timeline/TimelineViewportObservation";
import {
  canonicalTimelineContainsActivityEventId,
  captureAnchor,
  captureFreeScrollAnchor,
  captureRoomScrollAnchor,
  findTimelineEventNode,
  type PendingHeightModelCommit,
  restoreAnchor,
  restoreAnchorWithDelta,
  roomScrollAnchorSignature,
  roomScrollAnchorStableSignature,
  restoreRoomScrollAnchor,
  timelineSessionAnchorAgeBucket,
  timelineViewportSessionMemory,
  type ScrollAnchor,
  type TimelineSessionAnchorAgeBucket
} from "./timeline/TimelineViewportAnchors";
import {
  emitTimelineEventDiagnosticLog,
  latestEventBackedItemId,
  paginationStateDiagnosticLabel,
  timelineBackfillCompletionReason,
  timelineDiffsContainOwnOutgoingItem,
  timelineDiffsContainReset,
  timelineKindDiagnosticLabel,
  timelineRowsArePurePrepend
} from "./timeline/TimelineEventProjection";
export { timelineRowsArePurePrependForTests } from "./timeline/TimelineEventProjection";
import {
  buildTimelineHeightModel, calculateTimelineItemIndexRange, calculateTimelineVirtualRange,
  EMPTY_TIMELINE_ITEM_INDEX_RANGE, EMPTY_TIMELINE_RANGE, measuredItemHeight,
  timelineItemHeightAtIndex, timelineItemIndexInRange, timelineItemIndexRangeEquals,
  TIMELINE_ESTIMATED_ITEM_HEIGHT_PX, TIMELINE_VIRTUALIZATION_THRESHOLD, virtualRangeEquals,
  type TimelineItemIndexRange, type TimelineScheduledFrame,
  type TimelineViewportMetrics, type TimelineVirtualRangeState, type TimelineVirtualWindow
} from "./timeline/TimelineViewportVirtualization";
import {
  createTimelineViewportScheduler,
  type TimelineViewportScheduler
} from "./timeline/TimelineViewportScheduler";
export { TimelineItemRow };
export type { TimelineRowActionHandlers,TimelineThreadAttention };
export type { TimelineTransport } from "./timeline/TimelineTransport";

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
RefreshCw,
Trash2
} from "lucide-react";
import {
memo,
useCallback,
useEffect,
useLayoutEffect,
useMemo,
useRef,
useState,
type Dispatch,
type FormEvent,
type KeyboardEvent,
type MouseEvent,
type PointerEvent as ReactPointerEvent,
type SetStateAction
} from "react";

import { peopleFacingLabel,type MentionCandidate } from "../app/uiShared";
import { AVATAR_THUMBNAIL_DOWNLOADS_ENABLED } from "../domain/avatarThumbnails";
import {
type ContextMenuItem
} from "../domain/contextMenus";
import type { DiagnosticLogEntry } from "../domain/diagnostics";
import { t } from "../i18n/messages";

import type {
AvatarThumbnailState,
MediaTransferProgress,
CoreEventPayload,
TimelineItem,
TimelineKey,
TimelineMessageSource,
TimelineNavigationSnapshot,
TimelineReadStateSync
} from "../domain/coreEvents";
import {
timelineItemDomId,
timelineKeyEquals,
timelineKeyRoomId
} from "../domain/coreEvents";
import type { TimelineForwardDestination } from "../domain/projectionTypes";
import {
evaluateTimelineBackfill,
type TimelineBackfillDemand,
type TimelineBackfillEvaluationTrigger
} from "../domain/timelineBackfillPolicy";
import {
insertTimelineGapItems,
projectTimelineDisplayRows,
type TimelineDisplayRow
} from "../domain/timelineDisplayProjection";
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
import {
applyGlobalResync,
applyRoomKeyRequestStateChanged,
applyTimelineEvent,
batchContainsPrepend,
createTimelineStore,
getItems,
getKeyState,
getMediaUploadProgress,
getPaginationState,
timelineStoreKeyId,
timelineStoreLookupDiagnosticMessage,
type TimelineStoreState
} from "../domain/timelineStore";
import {
recordTimelineEventReceived,
recordTimelineInitialItems,
recordTimelineKeyMismatch,
recordTimelineResync
} from "../domain/timelineTransportStats";
import type {
LiveSignalsState,
ResolveComposerKeyAction,
RoomLatestEventSummary,
TimelineContinuityState,
TimelineMediaDownloadState,
TimelineScrollAnchor,
TextRange,
UserProfile
} from "../domain/types";
import { ImeSafeForm,ImeTextField } from "./ImeTextControl";
import {
receiptDisplayName
} from "./timeline/ReceiptReaders";
import {
TimelineMediaViewer,
type TimelineMediaViewerItem
} from "./timeline/TimelineMedia";
import { useTimelineStoreContext } from "./timelineStoreContext";
import {
  timelineAvatarDiagnostics,
  timelineRenderedAvatarDiagnostics,
  type TimelineDiagnostics
} from "./timeline/TimelineDiagnostics";
export type { TimelineDiagnostics };
export { MessageMeta } from "./timeline/MessageMeta";
export { timelineMediaDisplayBoxForTests } from "./timeline/TimelineMedia";
export { receiptDisplayName };

import {
renderTimelineMessageText,
type OpenMatrixTargetHandler
} from "./timeline/TimelineMessageBody";
import { MessageSourceDialog } from "./timeline/MessageSourceDialog";
import { useTimelineRowTransportActions } from "./timeline/useTimelineRowTransportActions";
export { MessageSourceDialog };
export { renderTimelineMessageText };
export type { OpenMatrixTargetHandler };

export type { TimelineForwardDestination } from "../domain/projectionTypes";

/**
 * Returns an authoritative display event ID from a room summary.
 *
 * The SDK summary describes the latest Matrix event, not the final projected
 * timeline row. A relation target therefore cannot prove that its target is
 * the display tail, so relation summaries remain unknown until the backend
 * exposes that fact directly.
 */
export function roomLatestDisplayEventId(
  summary: RoomLatestEventSummary | null | undefined
): string | null {
  if (!summary || summary.is_redacted) {
    return null;
  }
  if (summary.relation_type) {
    return null;
  }
  return summary.event_id || null;
}

export type ReturnToLiveHandler = () => void | Promise<void>;

/** Keep UI event handlers from leaking rejected async navigation callbacks. */
export function invokeReturnToLiveSafely(handler: ReturnToLiveHandler): void {
  void Promise.resolve()
    .then(() => handler())
    .catch(() => undefined);
}

type PendingMeasuredHeight = {
  height: number;
  epoch: number;
};

type TimelineBackfillRequestEpoch = {
  id: number;
  timelineKeyHash: string;
  demand: TimelineBackfillDemand;
  paginatingReceived: boolean;
  projectionObserved: boolean;
  terminalReceived: boolean;
};

type TimelineBackfillRetryFence =
  | "external_transition"
  | "gap_repair_release";

type PendingTimelineBackfillEvaluation = {
  trigger: TimelineBackfillEvaluationTrigger;
  genuineUserScroll: boolean;
};

type TimelineBackfillMetrics = {
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
  projectedContentHeight: number;
  threshold: number;
  maxScrollTop: number;
};

const TIMELINE_AVATAR_THUMBNAIL_OVERSCAN_ITEMS = 8;
const TIMELINE_LINK_PREVIEW_OVERSCAN_ITEMS = 8;
const TIMELINE_SCROLL_IDLE_FLUSH_MS = 100;
const TIMELINE_SCROLL_MAX_DEFER_MS = 500;
const ignoreComposerKeyAction: ResolveComposerKeyAction = async () => "noop";

type ViewportIntent = { kind: "free-scroll" } | { kind: "live-edge" };

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export type TimelineDiagnosticLogEntry = DiagnosticLogEntry;

export const TimelineView = memo(function TimelineView({
  timelineKey,
  roomId,
  presentationContext = "room",
  transport,
  onReply,
  onOpenMatrixTarget,
  onOpenSenderProfile,
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
  automaticBackfillEligible = true,
  initialTargetEventId = null,
  isAnchored = false,
  onReturnToLive,
  liveLatestEventId = null,
  autoLoadOlderMessages = false,
  codeBlockWrap = true,
  recentEmojis = [],
  onRecentEmojisChange,
  searchHighlightsByEventId = {},
  mediaDownloads = {},
  continuity = { kind: "unknown" },
  roomScrollAnchor: _persistedRoomScrollAnchor = null,
  enableAvatarThumbnailDownloads = AVATAR_THUMBNAIL_DOWNLOADS_ENABLED,
  onDiagnosticsChange,
  onScrollDiagnosticsChange,
  onDiagnosticLogEntry,
  timelineStore,
  setTimelineStore,
  viewportScheduler: injectedViewportScheduler,
  listRefCallback,
  onRegisterJumpToLatest,
  threadAttention = null,
  mentionCandidates = [],
  mentionCandidatesLoading = false,
  onMentionQueryChange
}: {
  timelineKey: TimelineKey;
  roomId: string;
  presentationContext?: "room" | "thread" | "focused";
  transport: TimelineTransport;
  onReply: TimelineRowActionHandlers["onReply"];
  onOpenMatrixTarget?: TimelineRowActionHandlers["onOpenMatrixTarget"];
  onOpenSenderProfile?: TimelineRowActionHandlers["onOpenSenderProfile"];
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
      message: {
        sender: string;
        room_id: string;
        event_id: string;
        body: string;
        reply_count: number;
      };
    },
    items: ContextMenuItem[]
  ) => void;
  currentUserId?: string;
  ignoredUserIds?: string[];
  suppressPaginationUi?: boolean;
  automaticBackfillEligible?: boolean;
  /** Event ID to center once a thread timeline has materialized its rows. */
  initialTargetEventId?: string | null;
  // #161: main pane is anchored to a jump-to-date event; the live-edge control
  // returns to the live timeline instead of scrolling within the focused window.
  isAnchored?: boolean;
  onReturnToLive?: ReturnToLiveHandler;
  liveLatestEventId?: string | null;
  autoLoadOlderMessages?: boolean;
  codeBlockWrap?: boolean;
  recentEmojis?: string[];
  onRecentEmojisChange?: (emojis: string[]) => void | Promise<void>;
  searchHighlightsByEventId?: Record<string, { snippet: string; ranges: TextRange[] }>;
  mediaDownloads?: Record<string, TimelineMediaDownloadState>;
  continuity?: TimelineContinuityState;
  roomScrollAnchor?: TimelineScrollAnchor | null;
  /**
   * #116 perf gate. Defaults to the enabled
   * AVATAR_THUMBNAIL_DOWNLOADS_ENABLED policy; tests may disable it to isolate
   * unrelated timeline behavior.
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
   * Test-only scheduler injection. Production views create their own scheduler.
   */
  viewportScheduler?: TimelineViewportScheduler;
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
  mentionCandidates?: MentionCandidate[];
  mentionCandidatesLoading?: boolean;
  onMentionQueryChange?: (roomId: string, query: string | null) => void;
}) {
  // Persisted restart anchors are intentionally ignored for restoration:
  // first entry after app startup goes to live edge, while in-session room
  // switches use timelineViewportSessionMemory. persistViewportAnchor still
  // writes these anchors for diagnostics and future cross-restart design work.
  void _persistedRoomScrollAnchor;
  const timelineStoreContext = useTimelineStoreContext();
  const viewportSchedulerRef = useRef<TimelineViewportScheduler | null>(null);
  if (viewportSchedulerRef.current === null) {
    viewportSchedulerRef.current =
      injectedViewportScheduler ?? createTimelineViewportScheduler();
  }
  const timelineViewportScheduler = viewportSchedulerRef.current;
  const viewportSchedulerLifecycleRef = useRef(0);
  useEffect(() => {
    const lifecycle = ++viewportSchedulerLifecycleRef.current;
    return () => {
      Promise.resolve().then(() => {
        if (viewportSchedulerLifecycleRef.current === lifecycle) {
          timelineViewportScheduler.dispose();
        }
      });
    };
  }, [timelineViewportScheduler]);
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
  const [projectionSettlementRevision, setProjectionSettlementRevision] = useState(0);
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
  const [, setDeferredPrependReleaseVersion] = useState(0);
  const containerRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const itemHeightByDomIdRef = useRef<Map<string, number>>(new Map());
  const committedVisibleRowsRef = useRef<{
    timelineKeyHash: string;
    rows: readonly TimelineDisplayRow[];
  } | null>(null);
  const deferredPrependPendingRef = useRef(false);
  const pendingHeightModelCommitRef = useRef<PendingHeightModelCommit | null>(null);
  const heightCompensationRecordedVersionRef = useRef<number | null>(null);
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
   * corrects scroll position when an above-viewport row resizes. The height
   * model commit consumes this stable local anchor before paint.
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
  /** Tracks whether the current focused key already centered its target row. */
  const focusedTargetRestoreAppliedRef = useRef<string | null>(null);
  const initialThreadTargetRestoreAppliedRef = useRef<string | null>(null);
  /** Deduplicates privacy-safe focused-target restoration diagnostics. */
  const lastFocusedTargetRestoreDiagnosticRef = useRef<string | null>(null);
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
  const viewportIntentResizeFrameRef = useRef<TimelineScheduledFrame | null>(null);
  /** Coalesces a structural display-projection correction to one frame. */
  const projectionLayoutFrameRef = useRef<TimelineScheduledFrame | null>(null);
  const pendingProjectionLayoutRef = useRef<PendingProjectionLayoutTransaction | null>(null);
  const projectionRenderStateRef = useRef<TimelineProjectionSnapshot | null>(null);
  const scrollFollowUpFramesRef = useRef<Set<TimelineScheduledFrame>>(new Set());
  /** Accepted backward-pagination command awaiting a terminal timeline event. */
  const backfillRequestEpochRef = useRef<TimelineBackfillRequestEpoch | null>(null);
  /** A rejected request must wait for the transition that releases its owner. */
  const backfillRetryFenceRef = useRef<TimelineBackfillRetryFence | null>(null);
  const nextBackfillRequestEpochRef = useRef(1);
  const pendingBackfillEvaluationRef = useRef<PendingTimelineBackfillEvaluation | null>(null);
  const previousAutoLoadOlderMessagesRef = useRef(autoLoadOlderMessages);
  const evaluateAndMaybeRequestBackfillRef = useRef<
    (trigger: TimelineBackfillEvaluationTrigger, genuineUserScroll?: boolean) => void
  >(() => undefined);
  const lastBackfillEvaluationDiagnosticSignatureRef = useRef<string | null>(null);
  const lastViewportObservationRef = useRef<string | null>(null);
  const autoReturnToLiveIdentityRef = useRef<string | null>(null);
  const autoReturnToLiveKeyRef = useRef<string | null>(null);
  const downloadedEventIdsRef = useRef<Set<string>>(new Set());
  const requestedImagePreviewEventIdsRef = useRef<Set<string>>(new Set());
  const relevantAvatarMxcsRef = useRef<Set<string>>(new Set());
  const requestedAvatarMxcsRef = useRef<Set<string>>(new Set());
  const initialItemsSeenForTimelineKeyRef = useRef<string | null>(null);
  const lastDiagnosticsEmissionRef = useRef<{
    callback: (diagnostics: TimelineDiagnostics) => void;
    signature: string;
  } | null>(null);
  const lastStoreLookupDiagnosticRef = useRef<string | null>(null);
  const lastThreadCommitDiagnosticRef = useRef<string | null>(null);
  const scrollDiagnosticsRef = useRef<TimelineScrollDiagnostics>(
    createInitialTimelineScrollDiagnostics()
  );
  const onScrollDiagnosticsChangeRef = useRef(onScrollDiagnosticsChange);
  onScrollDiagnosticsChangeRef.current = onScrollDiagnosticsChange;
  const profileUsersRef = useRef(profileUsers);
  profileUsersRef.current = profileUsers;
  const timelineKeyRef = useRef(timelineKey);
  timelineKeyRef.current = timelineKey;
  const timelineKeyHash = timelineStoreKeyId(timelineKey);
  const timelineKeyHashRef = useRef(timelineKeyHash);
  timelineKeyHashRef.current = timelineKeyHash;
  const sessionRoomScrollAnchorRef = useRef<TimelineScrollAnchor | null>(null);
  const initialRoomScrollAnchorPresentRef = useRef<boolean | null>(null);
  const roomReentrySessionModeRef = useRef<"none" | "live_edge" | "anchor">("none");
  const roomReentryAnchorAgeRef = useRef<TimelineSessionAnchorAgeBucket>("none");
  const roomReentryDiagnosticKeyRef = useRef<string | null>(null);
  const roomTimelineRoomId = "Room" in timelineKey.kind ? timelineKey.kind.Room.room_id : null;
  const viewportTimelineRoomId =
    "Room" in timelineKey.kind
      ? timelineKey.kind.Room.room_id
      : "Thread" in timelineKey.kind
        ? timelineKey.kind.Thread.room_id
        : null;
  const viewportThreadRootEventId =
    "Thread" in timelineKey.kind ? timelineKey.kind.Thread.root_event_id : null;
  const focusedTimelineTargetEventId =
    "Focused" in timelineKey.kind ? timelineKey.kind.Focused.event_id : null;
  const items = getItems(store, timelineKey);
  // Issue #460: immediate acknowledgment on the user's "Request keys and
  // retry" click: a toast + a local pending marker, until Rust publishes a
  // terminal request state via the timeline DTO. Presentation-only.
  const [keyRequestToast, setKeyRequestToast] = useState<string | null>(null);
  const [pendingKeyRequests, setPendingKeyRequests] = useState<Set<string>>(new Set());
  // Account switch: the same room/event may open under a different account
  // (the pane component is keyed by room/anchor, not account). Rust-owned
  // request state is per-actor, so the previous account's local optimistic
  // marker must not suppress the new account's legitimate request. The epoch
  // also fences delayed rejections across A->B->A navigation (same key hash).
  const previousTimelineKeyHashRef = useRef<string | null>(null);
  const keyRequestEpochRef = useRef(0);
  useEffect(() => {
    if (
      previousTimelineKeyHashRef.current !== null &&
      previousTimelineKeyHashRef.current !== timelineKeyHash
    ) {
      setPendingKeyRequests(new Set());
      setKeyRequestToast(null);
      keyRequestEpochRef.current += 1;
    }
    previousTimelineKeyHashRef.current = timelineKeyHash;
  }, [timelineKeyHash]);
  useEffect(() => {
    if (!keyRequestToast) {
      return undefined;
    }
    const timer = setTimeout(() => setKeyRequestToast(null), 4000);
    return () => clearTimeout(timer);
  }, [keyRequestToast]);
  // Clear the local pending marker only on a terminal Rust outcome or a
  // rejection: while Rust reports sent/automatic/still_waiting the request is
  // still pending, so repeat clicks must stay suppressed (no duplicate
  // commands while pending). The Rust-published waiting copy renders from the
  // DTO regardless of the local marker.
  const KEY_REQUEST_TERMINAL_STAGES = [
    "withheld",
    "decryption_recovered",
    "send_failed"
  ];
  useEffect(() => {
    if (pendingKeyRequests.size === 0) {
      return;
    }
    const settled = new Set<string>();
    for (const key of pendingKeyRequests) {
      for (const item of items) {
        if (`event:${timelineItemDomId(item.id)}` !== key) {
          continue;
        }
        if (
          item.request_state &&
          KEY_REQUEST_TERMINAL_STAGES.includes(item.request_state.stage)
        ) {
          settled.add(key);
        }
      }
    }
    if (settled.size > 0) {
      const next = new Set(pendingKeyRequests);
      for (const key of settled) {
        next.delete(key);
      }
      setPendingKeyRequests(next);
    }
  }, [items, pendingKeyRequests]);
  // A reaction preview can omit its display label even when the sender is
  // already represented in this room's timeline. Keep a room-scoped fallback
  // so the tooltip does not regress to "Unknown user" while profile data
  // catches up.
  const reactionSenderLabelsByUserId = useMemo(() => {
    const labels: Record<string, string> = {};
    for (const profile of Object.values(profileUsers)) {
      const label = profile.display_label?.trim();
      if (label) {
        labels[profile.user_id] = label;
      }
    }
    for (const item of items) {
      const label = item.sender_label?.trim();
      if (item.sender && label && !labels[item.sender]) {
        labels[item.sender] = label;
      }
    }
    return labels;
  }, [items, profileUsers]);
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
  useEffect(() => {
    if (!("Focused" in timelineKey.kind)) {
      return;
    }
    const message = timelineStoreLookupDiagnosticMessage(store, timelineKey);
    if (lastStoreLookupDiagnosticRef.current === message) {
      return;
    }
    lastStoreLookupDiagnosticRef.current = message;
    emitDiagnosticLog("timeline.store", message);
  }, [emitDiagnosticLog, store, timelineKey, timelineKeyHash]);
  useLayoutEffect(() => {
    if (!("Thread" in timelineKey.kind) || !timelineKeyState) {
      return;
    }
    const signature = [
      timelineKeyHash,
      timelineKeyState.actorGeneration,
      timelineKeyState.generation,
      timelineKeyState.lastAppliedBatchId ?? "none",
      items.length
    ].join(":");
    if (lastThreadCommitDiagnosticRef.current === signature) {
      return;
    }
    lastThreadCommitDiagnosticRef.current = signature;
    emitDiagnosticLog(
      "thread.timeline",
      `stage=committed actor=${timelineKeyState.actorGeneration} ` +
        `generation=${timelineKeyState.generation} ` +
        `batch=${timelineKeyState.lastAppliedBatchId ?? "none"} items=${items.length}`
    );
  }, [
    emitDiagnosticLog,
    items.length,
    timelineKey.kind,
    timelineKeyHash,
    timelineKeyState
  ]);
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
    const epoch = timelineViewportScheduler.currentEpoch();
    const frame = timelineViewportScheduler.schedule(epoch, (timestamp) => {
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
  }, [timelineViewportScheduler]);
  const advanceViewportEpoch = useCallback(() => {
    const epoch = timelineViewportScheduler.advance();
    timelineViewportScheduler.cancelBefore(epoch);
    cancelPendingScrollFrame();
    cancelScrollFollowUpFrames();
    viewportIntentResizeFrameRef.current = null;
    projectionLayoutFrameRef.current = null;
    return epoch;
  }, [cancelPendingScrollFrame, cancelScrollFollowUpFrames, timelineViewportScheduler]);
  const scheduleViewportFrame = useCallback(
    (
      callback: FrameRequestCallback,
      epoch = timelineViewportScheduler.currentEpoch()
    ) => timelineViewportScheduler.schedule(epoch, callback),
    [timelineViewportScheduler]
  );
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
      scheduleViewportFrame(() => {
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
    [scheduleViewportFrame, updateScrollDiagnostics]
  );

  const setViewportIntentToLiveEdge = useCallback(() => {
    viewportIntentRef.current = { kind: "live-edge" };
    timelineViewportSessionMemory.set(timelineKeyHash, { mode: "live-edge" });
  }, [timelineKeyHash]);

  const releaseDeferredPrepend = useCallback((): boolean => {
    if (!deferredPrependPendingRef.current) {
      return false;
    }
    const container = containerRef.current;
    const anchor =
      pendingAnchorRef.current ?? (container ? captureFreeScrollAnchor(container) : null);
    pendingAnchorRef.current = anchor;
    anchorRestorePendingRef.current = anchor !== null;
    deferredPrependPendingRef.current = false;
    setDeferredPrependReleaseVersion((current) => current + 1);
    return true;
  }, []);

  const flushPendingMeasurements = useCallback(
    (reason: "idle" | "maxDefer") => {
      const pending = pendingMeasuredHeightsRef.current;
      if (pending.size === 0) {
        clearMeasurementTimers();
        scrollActivityRef.current = "idle";
        clearPendingMeasurementDiagnostics();
        releaseDeferredPrepend();
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
      releaseDeferredPrepend();

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

      const heightAnchor =
        container &&
        viewportIntentRef.current.kind === "free-scroll" &&
        !jumpViewportControlRef.current &&
        !roomScrollAnchorRestorePendingRef.current
          ? pendingAnchorRef.current ?? freeScrollAnchorRef.current
          : null;
      pendingHeightModelCommitRef.current = heightAnchor
        ? { timelineKeyHash: timelineKeyHashRef.current, anchor: heightAnchor, changedRows }
        : null;

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
      releaseDeferredPrepend,
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
    // Input belongs to the user even when it leaves the logical intent kind
    // unchanged (for example another free-scroll wheel event). A queued
    // projection frame must never reclaim that viewport position.
    advanceViewportEpoch();
    // The programmatic write already completed synchronously. Its epoch-cancelled
    // cleanup frame must not leave later genuine anchor capture suppressed.
    suppressScrollAnchorCaptureRef.current = false;
    programmaticScrollSignatureRef.current = null;
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
  }, [advanceViewportEpoch, setViewportIntentToFreeScroll]);

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

  const scheduleBackfillEvaluation = useCallback(
    (
      trigger: TimelineBackfillEvaluationTrigger,
      genuineUserScroll = false
    ) => {
      const retryFence = backfillRetryFenceRef.current;
      if (
        trigger === "gap_repair_released" ||
        (retryFence === "external_transition" && trigger !== "pagination_terminal")
      ) {
        backfillRetryFenceRef.current = null;
      }
      pendingBackfillEvaluationRef.current = { trigger, genuineUserScroll };
      setProjectionSettlementRevision((current) => current + 1);
    },
    []
  );

  useEffect(() => {
    scrollDiagnosticsRef.current = recordTimelineScrollCommit(scrollDiagnosticsRef.current);
  });

  // --- Event subscription: local stores apply reducers; App stores keep view effects here. ---
  const handleTimelineCoreEvent = useCallback((payload: CoreEventPayload) => {
      if (payload.kind === "ResyncMarker") {
        // EventStreamLag: the core event broadcast overflowed and dropped
        // events for this consumer (likely including this room's InitialItems).
        // Clear, then RE-SUBSCRIBE so the core re-emits a fresh InitialItems;
        // clearing alone would leave the timeline permanently blank.
        advanceViewportEpoch();
        recordTimelineResync();
        pendingAnchorRef.current = null;
        anchorRestorePendingRef.current = false;
        roomScrollAnchorRestorePendingRef.current = false;
        viewportIntentRef.current = { kind: "free-scroll" };
        userScrollInputPendingRef.current = false;
        backfillRequestEpochRef.current = null;
        backfillRetryFenceRef.current = null;
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
        scheduleBackfillEvaluation("timeline_reset");
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
        emitDiagnosticLog("timeline.avatar", avatarThumbnailLogMessage(thumbnail));
        setAvatarThumbnails((current) => ({ ...current, [mxc_uri]: thumbnail }));
        return;
      }
      // Issue #460: Rust-published room-key request transitions update the
      // displayed item directly (a static timeline emits no diff for the
      // to-device withheld / operational-timeout outcomes).
      if (
        payload.kind === "Room" &&
        typeof payload.event === "object" &&
        payload.event !== null &&
        "RoomKeyRequestStateChanged" in payload.event
      ) {
        const change = payload.event.RoomKeyRequestStateChanged;
        if (!timelineKeyEquals(timelineKeyRef.current, change.key)) {
          return;
        }
        if (!isAppLevelStore) {
          setStore((current) => {
            const next = applyRoomKeyRequestStateChanged(
              current,
              change.key,
              change.event_id,
              change.stage,
              change.withheld_code
            );
            relevantAvatarMxcsRef.current = timelineAvatarMxcsForItems(
              getItems(next, timelineKeyRef.current),
              profileUsersRef.current
            );
            return next;
          });
        }
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
                  : "MediaSendQueued" in event
                    ? event.MediaSendQueued.key
                  : "SubmissionAccepted" in event
                    ? event.SubmissionAccepted.key
                    : "SubmissionRejected" in event
                      ? event.SubmissionRejected.key
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
                                : "GapPositionsUpdated" in event
                                    ? event.GapPositionsUpdated.key
                                    : "GapRepairReleased" in event
                                      ? event.GapRepairReleased.key
                                      : event.ResyncRequired.key;
      if (!timelineKeyEquals(eventKey, timelineKeyRef.current)) {
        const currentKey = timelineKeyRef.current;
        const currentKind = timelineKindDiagnosticLabel(currentKey);
        const eventKind = timelineKindDiagnosticLabel(eventKey);
        const accountMatch = currentKey.account_key === eventKey.account_key;
        const roomMatch = timelineKeyRoomId(currentKey) === timelineKeyRoomId(eventKey);
        if (recordTimelineKeyMismatch(currentKind, eventKind, accountMatch, roomMatch)) {
          emitDiagnosticLog(
            "timeline.key",
            [
              "stage=event_dropped_summary",
              `current_kind=${currentKind}`,
              `event_kind=${eventKind}`,
              `account_match=${accountMatch}`,
              `room_match=${roomMatch}`
            ].join(" ")
          );
        }
        return;
      }
      emitTimelineEventDiagnosticLog(event, eventKey, emitDiagnosticLog);
      if ("SubmissionAccepted" in event || "SubmissionRejected" in event) {
        return;
      }
      if ("InitialItems" in event) {
        backfillRequestEpochRef.current = null;
        backfillRetryFenceRef.current = null;
        initialItemsSeenForTimelineKeyRef.current = timelineKeyHashRef.current;
        const entryAnchor = sessionRoomScrollAnchorRef.current;
        initialRoomScrollAnchorPresentRef.current = entryAnchor
          ? canonicalTimelineContainsActivityEventId(event.InitialItems.items, entryAnchor.event_id)
          : null;
        recordTimelineInitialItems(event.InitialItems.items.length);
        advanceViewportEpoch();
        resetActiveMeasurementDeferral({ clearMountedIds: true });
        relevantAvatarMxcsRef.current = timelineAvatarMxcsForItems(
          event.InitialItems.items,
          profileUsersRef.current
        );
        scheduleBackfillEvaluation("initial_projection");
      }
      if (
        "ItemsUpdated" in event &&
        timelineDiffsContainReset(event.ItemsUpdated.diffs)
      ) {
        advanceViewportEpoch();
        resetActiveMeasurementDeferral({ clearMountedIds: true });
      }
      if ("GapPositionsUpdated" in event) {
        scheduleBackfillEvaluation("gap_projection_changed");
      }
      if ("GapRepairReleased" in event) {
        scheduleBackfillEvaluation("gap_repair_released");
      }
      if (
        "PaginationStateChanged" in event &&
        event.PaginationStateChanged.direction === "Backward" &&
        event.PaginationStateChanged.state === "Paginating" &&
        backfillRequestEpochRef.current !== null
      ) {
        backfillRetryFenceRef.current = null;
        backfillRequestEpochRef.current.paginatingReceived = true;
      }
      const backfillCompletionReason = timelineBackfillCompletionReason(event);
      if (backfillCompletionReason !== null) {
        const epoch = backfillRequestEpochRef.current;
        if (epoch !== null) {
          emitDiagnosticLog(
            "timeline.backfill",
            `stage=complete reason=${backfillCompletionReason}`
          );
        }
        const terminalCanPrecedeProjection =
          "PaginationStateChanged" in event &&
          event.PaginationStateChanged.state === "Idle";
        const acceptedIdle =
          epoch !== null &&
          terminalCanPrecedeProjection &&
          (epoch.paginatingReceived || epoch.projectionObserved);
        const acceptedIdleWithoutPrepend =
          acceptedIdle &&
          "PaginationStateChanged" in event &&
          event.PaginationStateChanged.prepend_expected === false;
        if (
          epoch !== null &&
          acceptedIdle &&
          !acceptedIdleWithoutPrepend &&
          !epoch.projectionObserved
        ) {
          epoch.terminalReceived = true;
        } else {
          backfillRequestEpochRef.current = null;
        }
        if (
          "PaginationStateChanged" in event &&
          event.PaginationStateChanged.state !== "EndReached" &&
          !acceptedIdle
        ) {
          backfillRetryFenceRef.current =
            event.PaginationStateChanged.state === "Idle" &&
            event.PaginationStateChanged.prepend_expected == null
              ? "gap_repair_release"
              : "external_transition";
        }
        const shouldReevaluate =
          "ResyncRequired" in event ||
          ("PaginationStateChanged" in event &&
            ((acceptedIdle &&
              (acceptedIdleWithoutPrepend || epoch?.projectionObserved === true)) ||
              event.PaginationStateChanged.state === "EndReached"));
        if (shouldReevaluate) {
          scheduleBackfillEvaluation(
            "PaginationStateChanged" in event ? "pagination_terminal" : "timeline_reset"
          );
        }
      }

      // Prepend batches: capture the anchor BEFORE the diff is applied to
      // React state, so the layout effect can restore it after commit.
      if (
        "ItemsUpdated" in event &&
        (batchContainsPrepend(event.ItemsUpdated.diffs) ||
          timelineDiffsContainReset(event.ItemsUpdated.diffs))
      ) {
        const epoch = backfillRequestEpochRef.current;
        if (epoch !== null) {
          epoch.projectionObserved = true;
          if (epoch.terminalReceived) {
            backfillRequestEpochRef.current = null;
          }
        }
        if (batchContainsPrepend(event.ItemsUpdated.diffs)) {
          const container = containerRef.current;
          if (container && !anchorRestorePendingRef.current) {
            pendingAnchorRef.current = captureFreeScrollAnchor(container);
          }
          if (scrollActivityRef.current === "active") {
            deferredPrependPendingRef.current = true;
            anchorRestorePendingRef.current = true;
          } else if (container) {
            anchorRestorePendingRef.current = pendingAnchorRef.current !== null;
          }
        }
        scheduleBackfillEvaluation("prepend_settled");
      }

      if ("ResyncRequired" in event) {
        advanceViewportEpoch();
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
  }, [
    advanceViewportEpoch,
    currentUserId,
    emitDiagnosticLog,
    isAppLevelStore,
    resetActiveMeasurementDeferral,
    scheduleBackfillEvaluation,
    setViewportIntentToLiveEdge,
    timelineKeyHash,
    transport
  ]);

  useTimelineEventSubscription({
    transport,
    onEvent: handleTimelineCoreEvent,
    itemCount: items.length,
    timelineKeyHash,
    timelineKeyHashRef,
    timelineKeyRef,
    initialItemsSeenForTimelineKeyRef
  });

  useEffect(
    () => () => {
      cancelPendingScrollFrame();
      cancelScrollFollowUpFrames();
      resetActiveMeasurementDeferral({ clearMountedIds: true });
    },
    [cancelPendingScrollFrame, cancelScrollFollowUpFrames, resetActiveMeasurementDeferral]
  );

  useLayoutEffect(() => {
    advanceViewportEpoch();
    const sessionViewport = timelineViewportSessionMemory.get(timelineKeyHash) ?? null;
    sessionRoomScrollAnchorRef.current =
      sessionViewport?.mode === "anchor" ? sessionViewport.anchor : null;
    initialRoomScrollAnchorPresentRef.current = null;
    roomReentrySessionModeRef.current =
      sessionViewport?.mode === "anchor"
        ? "anchor"
        : sessionViewport?.mode === "live-edge"
          ? "live_edge"
          : "none";
    roomReentryAnchorAgeRef.current = timelineSessionAnchorAgeBucket(
      sessionViewport?.mode === "anchor" ? sessionViewport.anchor : null
    );
    roomReentryDiagnosticKeyRef.current = null;
    anchorAsyncGenerationRef.current += 1;
    pendingAnchorRef.current = null;
    anchorRestorePendingRef.current = false;
    deferredPrependPendingRef.current = false;
    committedVisibleRowsRef.current = null;
    pendingHeightModelCommitRef.current = null;
    heightCompensationRecordedVersionRef.current = null;
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
    backfillRequestEpochRef.current = null;
    backfillRetryFenceRef.current = null;
    pendingBackfillEvaluationRef.current = {
      trigger: "timeline_reset",
      genuineUserScroll: false
    };
    lastPersistedViewportAnchorSignatureRef.current = null;
    avatarRequestRangeRef.current = EMPTY_TIMELINE_ITEM_INDEX_RANGE;
    setAvatarRequestRange(EMPTY_TIMELINE_ITEM_INDEX_RANGE);
    linkPreviewRequestRangeRef.current = EMPTY_TIMELINE_ITEM_INDEX_RANGE;
    setLinkPreviewRequestRange(EMPTY_TIMELINE_ITEM_INDEX_RANGE);
  }, [
    advanceViewportEpoch,
    resetActiveMeasurementDeferral,
    timelineKeyHash
  ]);

  useEffect(() => {
    const sessionViewport = timelineViewportSessionMemory.get(timelineKeyHash) ?? null;
    sessionRoomScrollAnchorRef.current =
      sessionViewport?.mode === "anchor" ? sessionViewport.anchor : null;
    initialRoomScrollAnchorPresentRef.current = null;
    roomReentrySessionModeRef.current =
      sessionViewport?.mode === "anchor"
        ? "anchor"
        : sessionViewport?.mode === "live-edge"
          ? "live_edge"
          : "none";
    roomReentryAnchorAgeRef.current = timelineSessionAnchorAgeBucket(
      sessionViewport?.mode === "anchor" ? sessionViewport.anchor : null
    );
    roomReentryDiagnosticKeyRef.current = null;
    setNavigationSnapshot(null);
    setViewportAtBottom(false);
    lastViewportObservationRef.current = null;
    downloadedEventIdsRef.current = new Set();
    requestedImagePreviewEventIdsRef.current = new Set();
    relevantAvatarMxcsRef.current = new Set();
    requestedAvatarMxcsRef.current = new Set();
    initialItemsSeenForTimelineKeyRef.current = null;
    lastDiagnosticsEmissionRef.current = null;
    initialLiveEdgeScrollAppliedRef.current = null;
    focusedTargetRestoreAppliedRef.current = null;
    lastFocusedTargetRestoreDiagnosticRef.current = null;
    stickToBottomAfterMeasurementRef.current = false;
    resetActiveMeasurementDeferral({ clearMountedIds: true });
    itemHeightByDomIdRef.current = new Map();
    committedVisibleRowsRef.current = null;
    deferredPrependPendingRef.current = false;
    pendingHeightModelCommitRef.current = null;
    heightCompensationRecordedVersionRef.current = null;
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
    backfillRequestEpochRef.current = null;
    lastBackfillEvaluationDiagnosticSignatureRef.current = null;
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
    setMeasuredHeightVersion((current) => current + 1);
    scheduleBackfillEvaluation("timeline_reset");
  }, [resetActiveMeasurementDeferral, scheduleBackfillEvaluation, timelineKeyHash]);

  useEffect(
    () => () => {
      anchorAsyncGenerationRef.current += 1;
      anchorRestorePendingRef.current = false;
      deferredPrependPendingRef.current = false;
      committedVisibleRowsRef.current = null;
      pendingHeightModelCommitRef.current = null;
      heightCompensationRecordedVersionRef.current = null;
      roomScrollAnchorRestorePendingRef.current = false;
      suppressScrollAnchorCaptureRef.current = false;
      viewportIntentRef.current = { kind: "free-scroll" };
      resetActiveMeasurementDeferral({ clearMountedIds: true });
      userScrollInputPendingRef.current = false;
      pendingScrollFrameUserInputRef.current = false;
      backfillRequestEpochRef.current = null;
      backfillRetryFenceRef.current = null;
      pendingBackfillEvaluationRef.current = null;
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
    },
    [resetActiveMeasurementDeferral]
  );

  useEffect(() => {
    relevantAvatarMxcsRef.current = timelineAvatarMxcsForItems(items, profileUsers);
  }, [items, profileUsers]);
  const visibleItems = useMemo(() => items.filter((item) => !item.is_hidden), [items]);
  // The SDK-owned store stays canonical. Only these presentation rows feed
  // rendering, measuring, and virtualization for an opt-in Room projection.
  const projectedVisibleRows = useMemo(() => {
    const itemsWithGaps = insertTimelineGapItems(
      items,
      timelineKeyState?.gapPositions ?? [],
      timelineKeyState?.gapGeneration ?? 0
    );
    return projectTimelineDisplayRows(itemsWithGaps).filter((row) => !row.item.is_hidden);
  }, [items, timelineKeyState]);
  const committedProjection =
    committedVisibleRowsRef.current?.timelineKeyHash === timelineKeyHash
      ? committedVisibleRowsRef.current.rows
      : projectedVisibleRows;
  const deferPurePrepend =
    scrollActivityRef.current === "active" &&
    timelineRowsArePurePrepend(
      committedProjection.map((row) => row.row_id),
      projectedVisibleRows.map((row) => row.row_id)
    );
  const visibleRows = deferPurePrepend ? committedProjection : projectedVisibleRows;
  useLayoutEffect(() => {
    if (deferPurePrepend) {
      deferredPrependPendingRef.current = true;
      anchorRestorePendingRef.current = true;
      return;
    }
    if (deferredPrependPendingRef.current) {
      deferredPrependPendingRef.current = false;
    }
    committedVisibleRowsRef.current = {
      timelineKeyHash,
      rows: projectedVisibleRows
    };
  }, [deferPurePrepend, projectedVisibleRows, timelineKeyHash]);
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
      if (anchorRestorePendingRef.current) {
        if (projectionLayoutFrameRef.current !== null) {
          projectionLayoutFrameRef.current.cancel();
          projectionLayoutFrameRef.current = null;
        }
        pendingProjectionLayoutRef.current = null;
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
      pendingProjectionLayoutRef.current = {
        timelineKeyHash: next.timelineKeyHash,
        generation: next.generation,
        signature: next.signature,
        viewportEpoch: timelineViewportScheduler.currentEpoch(),
        mode: viewportIntentRef.current.kind === "live-edge" ? "live-edge" : "free-scroll",
        anchor
      };
    },
    [timelineViewportScheduler]
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
  useLayoutEffect(() => {
    const transaction = pendingHeightModelCommitRef.current;
    if (transaction === null || transaction.timelineKeyHash !== timelineKeyHash) {
      return;
    }
    pendingHeightModelCommitRef.current = null;
    const container = containerRef.current;
    if (
      !container ||
      viewportIntentRef.current.kind !== "free-scroll" ||
      jumpViewportControlRef.current ||
      roomScrollAnchorRestorePendingRef.current
    ) {
      return;
    }
    let delta: number | null = null;
    runWithScrollWriteReason("backfillCompensation", () => {
      delta = restoreAnchorWithDelta(container, transaction.anchor);
    });
    if (delta === null) {
      return;
    }
    heightCompensationRecordedVersionRef.current = measuredHeightVersion;
    const userInputPending =
      pendingScrollFrameUserInputRef.current || userScrollInputPendingRef.current;
    updateScrollDiagnostics((current) =>
      recordTimelineScrollFrame(current, {
        scrollActivity: scrollActivityRef.current,
        viewportIntent: "freeScroll",
        userInputPending,
        virtualized: virtualWindow.virtualized,
        startIndex: virtualWindow.startIndex,
        endIndex: virtualWindow.endIndex,
        paddingTop: virtualWindow.paddingTop,
        paddingBottom: virtualWindow.paddingBottom,
        changedMeasuredRowCount: transaction.changedRows,
        heightDeltaAboveViewportPx: delta ?? 0,
        heightDeltaInsideViewportPx: 0,
        heightDeltaBelowViewportPx: 0,
        anchorTopDeltaPx: delta ?? 0
      })
    );
    freeScrollAnchorRef.current = captureFreeScrollAnchor(container);
    updateViewportMetrics();
  }, [
    measuredHeightVersion,
    runWithScrollWriteReason,
    timelineKeyHash,
    updateScrollDiagnostics,
    updateViewportMetrics,
    virtualWindow.endIndex,
    virtualWindow.paddingBottom,
    virtualWindow.paddingTop,
    virtualWindow.startIndex,
    virtualWindow.virtualized
  ]);
  useEffect(() => {
    if (heightCompensationRecordedVersionRef.current === measuredHeightVersion) {
      return;
    }
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
    measuredHeightVersion,
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
      if (thumbnail.kind !== "notRequested") {
        continue;
      }
      if (requestedAvatarMxcsRef.current.has(avatar.mxc_uri)) {
        continue;
      }
      requestedAvatarMxcsRef.current.add(avatar.mxc_uri);
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
  const isPaginating =
    automaticBackfillEligible && backwardState === "Paginating";
  const endReached =
    backwardState === "EndReached" &&
    continuity.kind === "healthy" &&
    continuity.authoritative_start;
  const roomSignals = liveSignals?.rooms[roomId] ?? null;
  // Read receipts and fully-read state remain canonical timeline facts. A
  // moved root only changes presentation; it must not cause the root id to be
  // sent as the room's latest readable event.
  const latestReadableEventId = latestEventBackedItemId(items);
  useEffect(() => {
    const anchorEventId = focusedTimelineTargetEventId ?? initialTargetEventId ?? "anchored";
    const identityKey = [roomId, anchorEventId].join("\u0000");
    if (!isAnchored) {
      autoReturnToLiveIdentityRef.current = null;
      autoReturnToLiveKeyRef.current = null;
      return;
    }
    if (autoReturnToLiveIdentityRef.current !== identityKey) {
      autoReturnToLiveIdentityRef.current = identityKey;
      autoReturnToLiveKeyRef.current = null;
    }
    if (
      !onReturnToLive ||
      !viewportAtBottom ||
      !latestReadableEventId ||
      !liveLatestEventId ||
      latestReadableEventId !== liveLatestEventId
    ) {
      return;
    }

    const key = [identityKey, liveLatestEventId].join("\u0000");
    if (autoReturnToLiveKeyRef.current === key) {
      return;
    }
    autoReturnToLiveKeyRef.current = key;
    void Promise.resolve()
      .then(() => onReturnToLive())
      .catch(() => {
        if (autoReturnToLiveKeyRef.current === key) {
          autoReturnToLiveKeyRef.current = null;
        }
      });
  }, [
    focusedTimelineTargetEventId,
    initialTargetEventId,
    isAnchored,
    latestReadableEventId,
    liveLatestEventId,
    onReturnToLive,
    roomId,
    viewportAtBottom
  ]);
  const timelineInitialized = Boolean(timelineKeyState && !timelineKeyState.awaitingResync);
  // Stable, render-visible timeline generation for this key. Bumps when the
  // store replaces the list for a new generation (InitialItems / resync), so
  // tests can poll a concrete attribute instead of sleeping. 0 is a valid
  // Core generation; use timelineInitialized to distinguish "not initialized".
  const initialLiveEdgeScrollKey = timelineInitialized
    ? `${timelineKeyHash}:${generation}`
    : null;
  const timelineDiagnosticKind = timelineKindDiagnosticLabel(timelineKey);
  const rowTransportActions = useTimelineRowTransportActions(
    transport,
    timelineDiagnosticKind,
    onDiagnosticLogEntry
  );
  const { onRetrySend, onCancelSend } = rowTransportActions;
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
  const onRequestRoomKey = useCallback(
    (targetRoomId: string, eventId: string) => {
      // User-triggered "Request keys and retry" action: immediate visible
      // acknowledgment, then the Rust command confirms the lifecycle. While a
      // request for this event is already pending locally, a repeat click only
      // re-shows the toast — no duplicate command is dispatched (the pending
      // marker is exactly the in-flight command marker). On rejection
      // (IPC/command failure) the optimistic marker and toast are reverted so
      // the UI never shows a stuck "waiting" state.
      const pendingKey = `event:${eventId}`;
      const capturedEpoch = keyRequestEpochRef.current;
      if (targetRoomId === roomId) {
        setKeyRequestToast(t("timeline.keyRequestToast"));
        if (pendingKeyRequests.has(pendingKey)) {
          return;
        }
        setPendingKeyRequests((current) => {
          const next = new Set(current);
          next.add(pendingKey);
          return next;
        });
      }
      void transport
        .requestRoomKey(targetRoomId, eventId, "user", timelineKey)
        .catch(() => {
          // Fence by timeline key AND view epoch: a delayed rejection from a
          // previous account/room — or an earlier visit to the same key
          // (A->B->A) — must not clear the current view's marker/toast.
          if (
            targetRoomId === roomId &&
            timelineKeyHashRef.current === timelineKeyHash &&
            keyRequestEpochRef.current === capturedEpoch
          ) {
            setPendingKeyRequests((current) => {
              const next = new Set(current);
              next.delete(pendingKey);
              return next;
            });
            setKeyRequestToast(null);
          }
        });
    },
    [pendingKeyRequests, roomId, t, timelineKey, transport]
  );
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
  const reportViewportObservation = useCallback(() => {
    const observeViewport = transport.observeViewport;
    const canObserveTimelineViewport = Boolean(
      observeViewport && viewportTimelineRoomId === roomId
    );
    const canComputeLocalViewport =
      focusedTimelineTargetEventId !== null || (isAnchored && onReturnToLive !== undefined);
    if (!canObserveTimelineViewport && !canComputeLocalViewport) {
      return;
    }
    const container = containerRef.current;
    if (!container) {
      return;
    }
    const visible = visibleTimelineViewportFacts(container);
    if (
      !visible.firstVisibleEventId &&
      !visible.lastVisibleEventId &&
      visible.visibleGapIds.length === 0
    ) {
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
    if (!canObserveTimelineViewport || !observeViewport) {
      return;
    }
    const signature = [
      roomId,
      viewportThreadRootEventId ?? "",
      visible.firstVisibleEventId ?? "",
      visible.lastVisibleEventId ?? "",
      visible.visibleGapIds
        .map((id) => `${id.topology_revision}:${id.ordinal}`)
        .join("\u0002"),
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
        visible.visibleGapIds,
        effectiveAtBottom,
        viewportThreadRootEventId
      )
      .catch(() => undefined);
  }, [
    focusedTimelineTargetEventId,
    isAnchored,
    latestReadableEventId,
    onReturnToLive,
    roomId,
    transport,
    viewportThreadRootEventId,
    viewportTimelineRoomId
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
        viewportIntentResizeFrameRef.current = scheduleViewportFrame(() => {
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
      // Free-scroll height changes are corrected synchronously with the height
      // model commit below. ResizeObserver is notification-only here; a queued
      // frame would allow a visible jump and race the committed model.
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
    scheduleViewportFrame,
    timelineKeyHash,
    updateViewportMetrics
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
    projectionLayoutFrameRef.current = scheduleViewportFrame(
      () => {
        if (projectionLayoutFrameRef.current !== null) {
          projectionLayoutFrameRef.current = null;
        }
        scheduleBackfillEvaluation("layout_settled");
        const current = projectionRenderStateRef.current;
        if (
          current === null ||
          timelineViewportScheduler.currentEpoch() !== scheduledTransaction.viewportEpoch ||
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
      },
      scheduledTransaction.viewportEpoch
    );
  }, [
    projectionSnapshot,
    reportViewportObservation,
    runWithScrollWriteReason,
    scheduleBackfillEvaluation,
    scheduleViewportFrame,
    timelineHeightModel,
    timelineViewportScheduler,
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
    const emitRoomReentryDecision = (
      path: "dom" | "virtual_fallback" | "cleared_to_live_edge" | "live_edge",
      anchorIsLive: boolean
    ) => {
      if (
        initialLiveEdgeScrollKey === null ||
        roomReentryDiagnosticKeyRef.current === initialLiveEdgeScrollKey
      ) {
        return;
      }
      roomReentryDiagnosticKeyRef.current = initialLiveEdgeScrollKey;
      emitDiagnosticLog(
        "timeline.scroll",
        `stage=room_reentry_restore session_mode=${roomReentrySessionModeRef.current} ` +
          `anchor_age=${roomReentryAnchorAgeRef.current} anchor_is_live=${String(anchorIsLive)} ` +
          `path=${path}`
      );
    };
    const emitFocusedTargetRestoreDecision = (
      path: "dom" | "virtual_fallback" | "pending",
      targetPresent: boolean
    ) => {
      const signature = `${initialLiveEdgeScrollKey ?? "none"}:${path}:${String(targetPresent)}`;
      if (lastFocusedTargetRestoreDiagnosticRef.current === signature) {
        return;
      }
      lastFocusedTargetRestoreDiagnosticRef.current = signature;
      emitDiagnosticLog(
        "timeline.scroll",
        `stage=focused_target_restore path=${path} target_present=${String(targetPresent)}`
      );
    };
    let roomAnchorRestored = false;
    if (
      focusedTimelineTargetEventId !== null
    ) {
      // A Focused timeline is an event-addressed navigation result, not a
      // normal room entry. Its target owns the initial viewport even when the
      // focused window also contains newer events. Treating this key like a
      // room timeline would immediately move the viewport to the window's
      // live edge and make older Activity results appear blank.
      releaseViewportIntent();
      if (
        timelineInitialized &&
        items.length > 0 &&
        container &&
        initialLiveEdgeScrollKey !== null &&
        focusedTargetRestoreAppliedRef.current !== initialLiveEdgeScrollKey
      ) {
        jumpViewportControlRef.current = true;
        advanceViewportEpoch();
        const targetRow = findTimelineEventNode(
          container,
          "activity",
          focusedTimelineTargetEventId
        );
        if (targetRow) {
          runWithScrollWriteReason("jumpToEvent", () => {
            targetRow.scrollIntoView({ block: "center", inline: "nearest" });
          });
          focusedTargetRestoreAppliedRef.current = initialLiveEdgeScrollKey;
          initialLiveEdgeScrollAppliedRef.current = initialLiveEdgeScrollKey;
          emitFocusedTargetRestoreDecision("dom", true);
          freeScrollAnchorRef.current = captureFreeScrollAnchor(container);
        } else {
          const targetIndex = visibleRows.findIndex(
            (row) => row.activity_event_id === focusedTimelineTargetEventId
          );
          if (targetIndex >= 0 && virtualWindow.virtualized) {
            const targetTop = timelineHeightModel.offsets[targetIndex] ?? 0;
            const targetHeight = timelineItemHeightAtIndex(
              timelineHeightModel,
              targetIndex
            );
            runWithScrollWriteReason("jumpToEvent", () => {
              container.scrollTop = Math.max(
                0,
                viewportMetricsRef.current.listOffsetTop +
                  targetTop +
                  targetHeight / 2 -
                  container.clientHeight / 2
              );
            });
            focusedTargetRestoreAppliedRef.current = initialLiveEdgeScrollKey;
            initialLiveEdgeScrollAppliedRef.current = initialLiveEdgeScrollKey;
            emitFocusedTargetRestoreDecision("virtual_fallback", true);
            scheduleScrollFollowUpFrame(() => {
              const mountedTarget = findTimelineEventNode(
                container,
                "activity",
                focusedTimelineTargetEventId
              );
              if (mountedTarget) {
                runWithScrollWriteReason("jumpToEvent", () => {
                  mountedTarget.scrollIntoView({ block: "center", inline: "nearest" });
                });
              }
              freeScrollAnchorRef.current = captureFreeScrollAnchor(container);
              updateViewportMetrics();
              reportViewportObservation();
            });
          } else {
            jumpViewportControlRef.current = false;
            emitFocusedTargetRestoreDecision("pending", targetIndex >= 0);
          }
        }
      }
    } else if (
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
          pendingBackfillEvaluationRef.current = {
            trigger: "room_anchor_settled",
            genuineUserScroll: false
          };
        }
        return restored;
      };

      const anchorIsLive =
        initialRoomScrollAnchorPresentRef.current !== false &&
        canonicalTimelineContainsActivityEventId(items, activeRoomAnchor.event_id);
      if (anchorIsLive) {
        roomScrollAnchorRestorePendingRef.current = true;
        runWithScrollWriteReason("roomRestore", () => {
          roomAnchorRestored = restoreActiveRoomAnchor();
        });
        if (roomAnchorRestored) {
          emitRoomReentryDecision("dom", true);
        }
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
            emitRoomReentryDecision("virtual_fallback", true);
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
              setProjectionSettlementRevision((current) => current + 1);
            });
            return;
          }
        }
        if (!roomAnchorRestored && !roomAnchorAlreadyRestored) {
          roomScrollAnchorRestorePendingRef.current = false;
        }
      } else if (container) {
        emitRoomReentryDecision("cleared_to_live_edge", false);
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
        emitRoomReentryDecision("live_edge", false);
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
      let followUpPending = false;
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
          followUpPending = true;
          scheduleScrollFollowUpFrame(() => {
            let followUpRestored = false;
            runWithScrollWriteReason("backfillCompensation", () => {
              followUpRestored = restoreAnchor(container, anchor);
            });
            if (followUpRestored) {
              freeScrollAnchorRef.current = anchor;
            }
            pendingAnchorRef.current = null;
            anchorRestorePendingRef.current = false;
            updateViewportMetrics();
            reportViewportObservation();
            setProjectionSettlementRevision((current) => current + 1);
            scheduleBackfillEvaluation("layout_settled");
          });
        }
      }
      if (!followUpPending) {
        pendingAnchorRef.current = null;
        // Restoration complete: the next automatic fill request is allowed again.
        anchorRestorePendingRef.current = false;
      }
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
    advanceViewportEpoch,
    applyViewportIntent,
    generation,
    emitDiagnosticLog,
    focusedTimelineTargetEventId,
    roomId,
    roomTimelineRoomId,
    initialLiveEdgeScrollKey,
    items,
    navigationSnapshot,
    reportViewportObservation,
    releaseViewportIntent,
    timelineHeightModel,
    timelineInitialized,
    updateViewportMetrics,
    virtualWindow.virtualized,
    visibleRows,
    runWithScrollWriteReason,
    scheduleBackfillEvaluation,
    scheduleScrollFollowUpFrame,
    setViewportIntentToLiveEdge,
    timelineKeyHash
  ]);

  useLayoutEffect(() => {
    if (
      presentationContext !== "thread" ||
      !initialTargetEventId ||
      !timelineInitialized ||
      initialThreadTargetRestoreAppliedRef.current === initialTargetEventId
    ) {
      return;
    }
    const container = containerRef.current;
    const targetRow = container
      ? findTimelineEventNode(container, "activity", initialTargetEventId)
      : null;
    if (targetRow) {
      runWithScrollWriteReason("jumpToEvent", () => {
        targetRow.scrollIntoView({ block: "center", inline: "nearest" });
      });
      initialThreadTargetRestoreAppliedRef.current = initialTargetEventId;
    }
  }, [
    initialTargetEventId,
    items,
    presentationContext,
    runWithScrollWriteReason,
    timelineInitialized,
    virtualWindow.virtualized,
    visibleRows
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
    let changedRows = 0;
    for (const domId of nextHeights.keys()) {
      if (!visibleDomIds.has(domId)) {
        nextHeights.delete(domId);
        changed = true;
        changedRows += 1;
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
      changedRows += 1;
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
    const heightAnchor =
      container &&
      viewportIntentRef.current.kind === "free-scroll" &&
      !jumpViewportControlRef.current &&
      !roomScrollAnchorRestorePendingRef.current
        ? pendingAnchorRef.current ?? freeScrollAnchorRef.current
        : null;
    pendingHeightModelCommitRef.current = heightAnchor
      ? {
          timelineKeyHash: timelineKeyHashRef.current,
          anchor: heightAnchor,
          changedRows
        }
      : null;
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

  const requestTimelineBackfill = useCallback(
    (
      demand: TimelineBackfillDemand,
      trigger: TimelineBackfillEvaluationTrigger,
      metrics: TimelineBackfillMetrics
    ): boolean => {
      if (backfillRequestEpochRef.current !== null) {
        return false;
      }
      const epoch: TimelineBackfillRequestEpoch = {
        id: nextBackfillRequestEpochRef.current,
        timelineKeyHash,
        demand,
        paginatingReceived: false,
        projectionObserved: false,
        terminalReceived: false
      };
      nextBackfillRequestEpochRef.current += 1;
      backfillRequestEpochRef.current = epoch;
      backfillRetryFenceRef.current = null;

      if (demand === "underfilled") {
        emitDiagnosticLog(
          "timeline.backfill",
          `stage=request trigger=underfilled_initial items=${items.length} scroll_height_px=${metrics.scrollHeight} client_height_px=${metrics.clientHeight} overflow_px=${Math.max(0, metrics.scrollHeight - metrics.clientHeight)} projected_height_px=${metrics.projectedContentHeight} auto_load=${autoLoadOlderMessages} state=${paginationStateDiagnosticLabel(backwardState)}`
        );
      } else {
        emitDiagnosticLog(
          "timeline.backfill",
          `stage=request trigger=${trigger === "user_scroll" ? "scroll" : trigger} scroll_top_px=${metrics.scrollTop} threshold_px=${metrics.threshold} max_scroll_top_px=${metrics.maxScrollTop} auto_load=${autoLoadOlderMessages}`
        );
      }

      void transport.paginateBackwards(timelineKeyRef.current).catch(() => {
        if (backfillRequestEpochRef.current?.id !== epoch.id) {
          return;
        }
        backfillRequestEpochRef.current = null;
        backfillRetryFenceRef.current = "external_transition";
        emitDiagnosticLog(
          "timeline.backfill",
          `stage=failed trigger=${trigger} reason=transport`
        );
      });
      return true;
    },
    [
      autoLoadOlderMessages,
      backwardState,
      emitDiagnosticLog,
      items.length,
      timelineKeyHash,
      transport
    ]
  );

  const evaluateAndMaybeRequestBackfill = useCallback(
    (
      trigger: TimelineBackfillEvaluationTrigger,
      genuineUserScroll = false
    ) => {
      const container = containerRef.current;
      if (!container) {
        return;
      }
      const clientHeight = Math.round(container.clientHeight);
      const scrollHeight = Math.round(container.scrollHeight);
      const scrollTop = Math.round(container.scrollTop);
      const projectedContentHeight = Math.round(timelineHeightModel.totalHeight);
      const maxScrollTop = Math.max(0, scrollHeight - clientHeight);
      const threshold = timelineBackfillThreshold(clientHeight, autoLoadOlderMessages);
      const projectionSettled =
        pendingProjectionLayoutRef.current === null &&
        projectionLayoutFrameRef.current === null;
      const physicalVirtualLayoutSettled = !(
        virtualWindow.virtualized &&
        projectedContentHeight > clientHeight + SCROLL_EDGE_TOLERANCE_PX &&
        scrollHeight <= clientHeight + SCROLL_EDGE_TOLERANCE_PX
      );
      const evaluation = evaluateTimelineBackfill({
        trigger,
        initialized: timelineKeyState !== undefined,
        awaitingResync: timelineKeyState?.awaitingResync ?? false,
        suppressPaginationUi,
        automaticBackfillEligible,
        autoLoadEnabled: autoLoadOlderMessages && clientHeight > 0,
        paginationState: backwardState,
        requestInFlight: backfillRequestEpochRef.current !== null,
        retryBlocked: backfillRetryFenceRef.current !== null,
        projectionSettled,
        virtualLayoutSettled:
          physicalVirtualLayoutSettled &&
          virtualRangeEquals(virtualRangeRef.current, virtualRange),
        anchorSettled:
          !anchorRestorePendingRef.current &&
          !roomScrollAnchorRestorePendingRef.current,
        itemCount: items.length,
        projectedContentHeight,
        clientHeight,
        scrollHeight,
        scrollTop,
        nearTopThreshold: threshold,
        genuineUserScroll
      });
      const demand = evaluation.kind === "idle" ? null : evaluation.demand;
      const reason = evaluation.kind === "request" ? null : evaluation.reason;
      const metricBucket = (value: number) => Math.max(0, Math.round(value / 100) * 100);
      const diagnosticSignature = [
        trigger,
        evaluation.kind,
        demand ?? "none",
        reason ?? "eligible",
        paginationStateDiagnosticLabel(backwardState),
        metricBucket(projectedContentHeight),
        metricBucket(clientHeight),
        metricBucket(scrollHeight),
        metricBucket(scrollTop),
        backfillRequestEpochRef.current?.id ?? "none"
      ].join("\u0000");
      if (lastBackfillEvaluationDiagnosticSignatureRef.current !== diagnosticSignature) {
        lastBackfillEvaluationDiagnosticSignatureRef.current = diagnosticSignature;
        emitDiagnosticLog(
          "timeline.backfill_evaluation",
          `trigger=${trigger} decision=${evaluation.kind} demand=${demand ?? "none"} reason=${reason ?? "none"} items=${items.length} projected_height_bucket=${metricBucket(projectedContentHeight)} client_height_bucket=${metricBucket(clientHeight)} scroll_height_bucket=${metricBucket(scrollHeight)} scroll_top_bucket=${metricBucket(scrollTop)} state=${paginationStateDiagnosticLabel(backwardState)} request_epoch=${backfillRequestEpochRef.current?.id ?? "none"}`
        );
      }
      if (evaluation.kind !== "request") {
        return;
      }
      requestTimelineBackfill(evaluation.demand, trigger, {
        scrollTop,
        scrollHeight,
        clientHeight,
        projectedContentHeight,
        threshold: Math.round(threshold),
        maxScrollTop: Math.round(maxScrollTop)
      });
    },
    [
      autoLoadOlderMessages,
      automaticBackfillEligible,
      backwardState,
      emitDiagnosticLog,
      items.length,
      requestTimelineBackfill,
      suppressPaginationUi,
      timelineHeightModel.totalHeight,
      timelineKeyState,
      virtualItemHeight,
      virtualRange,
      virtualWindow.virtualized
    ]
  );
  evaluateAndMaybeRequestBackfillRef.current = evaluateAndMaybeRequestBackfill;

  useLayoutEffect(() => {
    if (previousAutoLoadOlderMessagesRef.current === autoLoadOlderMessages) {
      return;
    }
    previousAutoLoadOlderMessagesRef.current = autoLoadOlderMessages;
    if (backfillRetryFenceRef.current === "external_transition") {
      backfillRetryFenceRef.current = null;
    }
    pendingBackfillEvaluationRef.current = {
      trigger: "setting_changed",
      genuineUserScroll: false
    };
  }, [autoLoadOlderMessages]);

  useLayoutEffect(() => {
    const pending = pendingBackfillEvaluationRef.current;
    pendingBackfillEvaluationRef.current = null;
    evaluateAndMaybeRequestBackfill(
      pending?.trigger ?? "layout_settled",
      pending?.genuineUserScroll ?? false
    );
  }, [
    evaluateAndMaybeRequestBackfill,
    generation,
    projectionSettlementRevision,
    timelineInitialized
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
    const userInputPending = userScrollInputPendingRef.current;
    const actuallyAtBottom = Boolean(container && isScrolledToBottom(container));
    if (userInputPending && actuallyAtBottom) {
      setViewportIntentToLiveEdge();
      if (isProgrammaticEcho) {
        userScrollInputPendingRef.current = false;
      }
    }
    const isUserDrivenScroll = userInputPending && !isProgrammaticEcho;
    pendingScrollFrameUserInputRef.current =
      pendingScrollFrameUserInputRef.current || isUserDrivenScroll;
    if (!isProgrammaticEcho && container) {
      const atBottom = actuallyAtBottom;
      if (isUserDrivenScroll) {
        // A genuine user scroll takes viewport control back from any jump.
        jumpViewportControlRef.current = false;
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
      pendingScrollFrameRef.current = scheduleViewportFrame(() => {
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
      if (isUserDrivenScroll) {
        if (backfillRetryFenceRef.current === "external_transition") {
          backfillRetryFenceRef.current = null;
        }
      }
      evaluateAndMaybeRequestBackfillRef.current("user_scroll", isUserDrivenScroll);
      persistViewportAnchor();
    }
  }, [
    setViewportIntentToLiveEdge,
    markScrollActivityActive,
    persistViewportAnchor,
    readViewportMetrics,
    reportViewportObservation,
    releaseViewportIntent,
    timelineKeyHash,
    commitVirtualRangeForMetrics,
    scheduleViewportFrame,
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
  const jumpToBottom = useCallback(() => {
    const container = containerRef.current;
    if (!container) {
      return;
    }
    const activeElement = document.activeElement;
    if (activeElement instanceof HTMLElement && container.contains(activeElement)) {
      activeElement.blur();
    }
    advanceViewportEpoch();
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
      scheduleBackfillEvaluation("live_edge_settled");
    });
  }, [
    setViewportIntentToLiveEdge,
    reportViewportObservation,
    runWithScrollWriteReason,
    scheduleBackfillEvaluation,
    scheduleScrollFollowUpFrame,
    updateViewportMetrics,
    advanceViewportEpoch
  ]);
  useEffect(() => {
    onRegisterJumpToLatest?.(jumpToBottom);
    return () => onRegisterJumpToLatest?.(null);
  }, [jumpToBottom, onRegisterJumpToLatest]);
  const canRenderRoomNavigation =
    roomTimelineRoomId === roomId;
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
  const readStateStatusMessage = navigationSnapshot
    ? readStateStatusMessageForSync(navigationSnapshot.read_state_sync)
    : null;

  return (
    <ProjectionSnapshotBoundary
      snapshot={projectionSnapshot}
      onBeforeProjectionChange={captureProjectionLayoutTransaction}
    >
      {keyRequestToast ? (
        <div className="message-send-actions" role="status" aria-live="polite">
          <span>{keyRequestToast}</span>
        </div>
      ) : null}
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
            visibility: canJumpToBottom ? "visible" : "hidden"
          }}
          aria-hidden={!canJumpToBottom}
        >
          <div className="timeline-navigation-pills">
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
              onClick={() => invokeReturnToLiveSafely(onReturnToLive)}
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
      {presentationContext !== "thread" && !suppressPaginationUi && endReached ? (
        <div className="timeline-start" data-testid="timeline-start">
          {t("timeline.conversationStart")}
        </div>
      ) : null}
      {readStateStatusMessage ? (
        <div
          className="timeline-read-state-status"
          data-testid="timeline-read-state-status"
          data-read-state-sync={
            navigationSnapshot ? readStateSyncToken(navigationSnapshot.read_state_sync) : undefined
          }
          role="status"
          aria-live="polite"
        >
          {readStateStatusMessage}
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
          const previousRow = visibleRows[visibleIndex - 1];
          const previousIsReadMarker = Boolean(
            previousRow?.activity_event_id &&
              readMarkerDisplayEventId === previousRow.activity_event_id &&
              !unreadMarkerEventId
          );
          const isContinuation = Boolean(
            (row.kind === "event" || row.kind === "threadRoot") &&
              (previousRow?.kind === "event" || previousRow?.kind === "threadRoot") &&
              item.sender &&
              previousRow?.item.sender &&
              item.sender === previousRow?.item.sender &&
              !isUnreadMarker &&
              !previousIsReadMarker
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
              {row.kind === "timelineGap" ? (
                <div
                  className={`timeline-gap-row${continuity.kind === "failedIncomplete" ? " failed" : ""}`}
                  role="status"
                  data-testid="timeline-gap-row"
                  data-gap-topology-revision={row.gap_id?.topology_revision}
                  data-gap-ordinal={row.gap_id?.ordinal}
                >
                  <span>
                    {continuity.kind === "failedIncomplete"
                      ? t("timeline.gapRepairFailed")
                      : t("timeline.gapRepairing")}
                  </span>
                  {continuity.kind === "failedIncomplete" && transport.repairTimeline ? (
                    <button type="button" onClick={() => void transport.repairTimeline?.(roomId)}>
                      {t("gate.retry")}
                    </button>
                  ) : null}
                </div>
              ) : row.kind === "threadRootPending" || row.kind === "threadRootFailed" ? (
                <ThreadRootStatusPlaceholder
                  row={row}
                  state={row.kind === "threadRootPending" ? "pending" : "failed"}
                  showThreadSummary={presentationContext !== "thread"}
                />
              ) : (
                <TimelineItemRow
                item={item}
                rowId={row.row_id}
                contentEventId={contentEventId}
                activityEventId={activityEventId}
                contentTimestampMs={row.content_timestamp_ms}
                roomId={roomId}
                keyRequestPending={pendingKeyRequests.has(`event:${timelineItemDomId(item.id)}`)}
                presentationContext={presentationContext}
                codeBlockWrap={codeBlockWrap}
                searchHighlights={
                  contentEventId &&
                  searchHighlightsByEventId[contentEventId]?.snippet ===
                    (item.formatted?.plain_text ?? item.body)
                    ? searchHighlightsByEventId[contentEventId]?.ranges ?? []
                    : []
                }
                onReply={onReply}
                onOpenThread={onOpenThread}
                resolveComposerKeyAction={resolveComposerKeyAction}
                recentEmojis={recentEmojis}
                onRecentEmojisChange={onRecentEmojisChange}
                mediaUploadProgress={mediaUploadProgressForItem(store, timelineKey, item)}
                {...rowTransportActions}
                isPinned={contentEventId ? pinnedEventIds.includes(contentEventId) : false}
                isContinuation={isContinuation}
                isTarget={
                  presentationContext === "thread" &&
                  initialTargetEventId !== null &&
                  (contentEventId === initialTargetEventId || activityEventId === initialTargetEventId)
                }
                onRequestRoomKey={onRequestRoomKey}
                autoLoadLinkPreviews={timelineItemIndexInRange(
                  visibleIndex,
                  linkPreviewRequestRange
                )}
                onOpenAliasDialog={onSetLocalUserAlias ? openAliasDialog : undefined}
                onOpenMediaViewer={openMediaViewer}
                onSaveMediaFile={transport.saveMediaFile}
                forwardDestinations={effectiveForwardDestinations}
                onOpenMatrixTarget={onOpenMatrixTarget}
                onOpenSenderProfile={
                  presentationContext === "room" ? onOpenSenderProfile : undefined
                }
                presence={item.sender ? liveSignals?.presence[item.sender] : undefined}
                profile={item.sender ? profileUsers[item.sender] : undefined}
                reactionSenderLabelsByUserId={reactionSenderLabelsByUserId}
                avatarThumbnails={avatarThumbnails}
                currentUserId={currentUserId}
                ignoredUserIds={ignoredUserIds}
                onOpenContextMenu={onOpenContextMenu}
                mentionProfileUsers={profileUsers}
                mentionCandidates={mentionCandidates}
                mentionCandidatesLoading={mentionCandidatesLoading}
                onMentionQueryChange={onMentionQueryChange}
                threadAttention={threadAttention}
                showThreadSummary={presentationContext !== "thread"}
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
      {roomSignals && roomSignals.typing_users.length > 0 ? (
        <div className="typing-indicator" dir="auto">
          {formatTypingUsers(roomSignals.typing_users)}
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
          <ImeSafeForm
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
            <ImeTextField
              className="dialog-input"
              aria-label={t("room.aliasInput")}
              value={aliasDraft}
              syncKey={aliasTarget.userId}
              onChange={(event) => updateAliasDraft(event.currentTarget.value)}
              autoFocus
            />
            <div className="dialog-actions">
              <button className="dialog-button is-primary" type="submit">
                {t("action.done")}
              </button>
            </div>
          </ImeSafeForm>
        </div>
      ) : null}
      </div>
    </ProjectionSnapshotBoundary>
  );
});

function readStateSyncToken(sync: TimelineReadStateSync): string {
  if (typeof sync === "string") {
    return sync;
  }
  return "failed";
}

function readStateStatusMessageForSync(sync: TimelineReadStateSync): string | null {
  if (sync === "pending") {
    return t("timeline.readStateSyncing");
  }
  if (sync === "synced" || sync === "notRequested") {
    return null;
  }
  const reason = sync.failed.kind;
  const reasonMessage = {
    authentication: "timeline.readStateReasonAuthentication",
    rate_limited: "timeline.readStateReasonRateLimited",
    timeout: "timeline.readStateReasonTimeout",
    transport: "timeline.readStateReasonTransport",
    server: "timeline.readStateReasonServer",
    capacity: "timeline.readStateReasonCapacity",
    sdk: "timeline.readStateReasonSdk"
  } as const;
  return `${t("timeline.readStateNotSynced")}: ${t(reasonMessage[reason])}`;
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

function avatarThumbnailLogMessage(thumbnail: AvatarThumbnailState): string {
  if (thumbnail.kind === "ready") {
    return "avatar thumbnail ready";
  }
  if (thumbnail.kind === "failed") {
    return `avatar thumbnail failed kind=${thumbnail.failureKind}`;
  }
  return `avatar thumbnail ${thumbnail.kind}`;
}

function formatTypingUsers(users: LiveSignalsState["rooms"][string]["typing_users"]): string {
  const [firstUser] = users;
  if (users.length === 1 && firstUser) {
    return t("timeline.typingOne", {
      user: peopleFacingLabel(firstUser.display_label)
    });
  }
  return t("timeline.typingMany", { count: users.length });
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
