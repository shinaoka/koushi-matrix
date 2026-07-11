export type TimelineScrollActivity = "idle" | "active";

const WRITE_REASONS = [
  "liveEdge",
  "jumpToEvent",
  "jumpToBottom",
  "roomRestore",
  "backfillCompensation",
  "projectionCompensation",
  "measurementFlush"
] as const;

const ROW_KINDS = [
  "text",
  "formatted",
  "reply",
  "reactionReceiptThread",
  "linkPreview",
  "media",
  "redactedSystem"
] as const;

export type TimelineViewportIntentKind =
  | "freeScroll"
  | "liveEdge"
  | "anchored"
  | "target"
  | "startupRestore"
  | "roomRestore"
  | "backfillCompensation"
  | "measurementFlush";

export type TimelineScrollWriteReason = (typeof WRITE_REASONS)[number];

export type TimelineScrollHeightCommitReason = "initial" | "idleFlush" | "timelineReset";

export type TimelineScrollRowKind = (typeof ROW_KINDS)[number];

export interface TimelineScrollFrameSample {
  scrollActivity: TimelineScrollActivity;
  viewportIntent: TimelineViewportIntentKind;
  userInputPending: boolean;
  virtualized: boolean;
  startIndex: number;
  endIndex: number;
  paddingTop: number;
  paddingBottom: number;
  changedMeasuredRowCount: number;
  heightDeltaAboveViewportPx: number;
  heightDeltaInsideViewportPx: number;
  heightDeltaBelowViewportPx: number;
  anchorTopDeltaPx: number;
}

export interface TimelineScrollEstimateSample {
  rowKind: TimelineScrollRowKind;
  estimatedPx: number;
  measuredPx: number;
}

export interface TimelineScrollEstimateBucket {
  count: number;
  maxAbsErrorPx: number;
  totalAbsErrorPx: number;
}

export interface TimelineScrollDiagnostics {
  renderCommits: number;
  scrollFrames: number;
  activeScrollFrames: number;
  rangeCommits: number;
  heightModelCommits: number;
  measurementFlushes: number;
  pendingMeasuredRows: number;
  scrollWrites: Record<TimelineScrollWriteReason, number>;
  maxAnchorTopDeltaPx: number;
  latestFrame: TimelineScrollFrameSample | null;
  estimateBuckets: Record<TimelineScrollRowKind, TimelineScrollEstimateBucket>;
}

function emptyWriteCounts(): Record<TimelineScrollWriteReason, number> {
  return Object.fromEntries(WRITE_REASONS.map((reason) => [reason, 0])) as Record<
    TimelineScrollWriteReason,
    number
  >;
}

function emptyEstimateBuckets(): Record<TimelineScrollRowKind, TimelineScrollEstimateBucket> {
  return Object.fromEntries(
    ROW_KINDS.map((kind) => [
      kind,
      {
        count: 0,
        maxAbsErrorPx: 0,
        totalAbsErrorPx: 0
      }
    ])
  ) as Record<TimelineScrollRowKind, TimelineScrollEstimateBucket>;
}

export function createInitialTimelineScrollDiagnostics(): TimelineScrollDiagnostics {
  return {
    renderCommits: 0,
    scrollFrames: 0,
    activeScrollFrames: 0,
    rangeCommits: 0,
    heightModelCommits: 0,
    measurementFlushes: 0,
    pendingMeasuredRows: 0,
    scrollWrites: emptyWriteCounts(),
    maxAnchorTopDeltaPx: 0,
    latestFrame: null,
    estimateBuckets: emptyEstimateBuckets()
  };
}

export function recordTimelineScrollCommit(
  diagnostics: TimelineScrollDiagnostics
): TimelineScrollDiagnostics {
  return { ...diagnostics, renderCommits: diagnostics.renderCommits + 1 };
}

export function recordTimelineScrollRangeCommit(
  diagnostics: TimelineScrollDiagnostics
): TimelineScrollDiagnostics {
  return { ...diagnostics, rangeCommits: diagnostics.rangeCommits + 1 };
}

export function recordTimelineScrollHeightCommit(
  diagnostics: TimelineScrollDiagnostics,
  _reason: TimelineScrollHeightCommitReason
): TimelineScrollDiagnostics {
  return { ...diagnostics, heightModelCommits: diagnostics.heightModelCommits + 1 };
}

export function recordTimelineScrollMeasurementFlush(
  diagnostics: TimelineScrollDiagnostics,
  changedRows: number
): TimelineScrollDiagnostics {
  return {
    ...diagnostics,
    measurementFlushes: diagnostics.measurementFlushes + 1,
    pendingMeasuredRows: Math.max(0, diagnostics.pendingMeasuredRows - changedRows)
  };
}

export function recordTimelineScrollWrite(
  diagnostics: TimelineScrollDiagnostics,
  reason: TimelineScrollWriteReason
): TimelineScrollDiagnostics {
  return {
    ...diagnostics,
    scrollWrites: {
      ...diagnostics.scrollWrites,
      [reason]: diagnostics.scrollWrites[reason] + 1
    }
  };
}

export function recordTimelineScrollFrame(
  diagnostics: TimelineScrollDiagnostics,
  sample: TimelineScrollFrameSample
): TimelineScrollDiagnostics {
  return {
    ...diagnostics,
    scrollFrames: diagnostics.scrollFrames + 1,
    activeScrollFrames:
      diagnostics.activeScrollFrames + (sample.scrollActivity === "active" ? 1 : 0),
    pendingMeasuredRows: Math.max(
      diagnostics.pendingMeasuredRows,
      sample.changedMeasuredRowCount
    ),
    maxAnchorTopDeltaPx: Math.max(
      diagnostics.maxAnchorTopDeltaPx,
      Math.abs(Math.round(sample.anchorTopDeltaPx))
    ),
    latestFrame: {
      ...sample,
      startIndex: Math.max(0, Math.trunc(sample.startIndex)),
      endIndex: Math.max(0, Math.trunc(sample.endIndex)),
      paddingTop: Math.max(0, Math.round(sample.paddingTop)),
      paddingBottom: Math.max(0, Math.round(sample.paddingBottom))
    }
  };
}

export function recordTimelineScrollEstimate(
  diagnostics: TimelineScrollDiagnostics,
  sample: TimelineScrollEstimateSample
): TimelineScrollDiagnostics {
  const absError = Math.abs(Math.round(sample.measuredPx - sample.estimatedPx));
  const bucket = diagnostics.estimateBuckets[sample.rowKind];
  return {
    ...diagnostics,
    estimateBuckets: {
      ...diagnostics.estimateBuckets,
      [sample.rowKind]: {
        count: bucket.count + 1,
        maxAbsErrorPx: Math.max(bucket.maxAbsErrorPx, absError),
        totalAbsErrorPx: bucket.totalAbsErrorPx + absError
      }
    }
  };
}

export function timelineScrollDiagnosticTokens(
  diagnostics: TimelineScrollDiagnostics
): string[] {
  const totalWrites = Object.values(diagnostics.scrollWrites).reduce(
    (sum, value) => sum + value,
    0
  );
  return [
    `timeline_scroll_commits=${diagnostics.renderCommits}`,
    `timeline_scroll_frames=${diagnostics.scrollFrames}`,
    `timeline_scroll_active_frames=${diagnostics.activeScrollFrames}`,
    `timeline_scroll_range_commits=${diagnostics.rangeCommits}`,
    `timeline_scroll_height_commits=${diagnostics.heightModelCommits}`,
    `timeline_scroll_flushes=${diagnostics.measurementFlushes}`,
    `timeline_scroll_pending_heights=${diagnostics.pendingMeasuredRows}`,
    `timeline_scroll_writes=${totalWrites}`,
    `timeline_scroll_max_anchor_delta_px=${diagnostics.maxAnchorTopDeltaPx}`,
    `timeline_scroll_media_estimate_error_px=${diagnostics.estimateBuckets.media.maxAbsErrorPx}`
  ];
}
