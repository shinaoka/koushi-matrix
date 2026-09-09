import type { AppStoreDeltaStats } from "./appStore";
import type { CapturedJsError } from "./jsErrorLog";
import type { TimelineTransportStats } from "./timelineTransportStats";
import type { DesktopSnapshot, SearchCrawlerRoomState, SyncState } from "./types";
import {
  qaDomDiagnosticTokens,
  qaSearchCrawlerDiagnosticTokens,
  qaTimelineDiagnosticTokens,
  qaUiLatencyDiagnosticTokens,
  timelineMatchesActiveRoom,
  type QaDomDiagnostics,
  type QaTimelineDiagnostics
} from "./qaTitle";
import type { RightPanelMode } from "./rightPanel";
import type { QaSendSmokeStatus } from "./qaSendSmoke";
import type { UiLatencyDiagnostics } from "./uiLatency";

export interface DiagnosticLogEntry {
  timestampMs: number;
  source: string;
  message: string;
}

export interface DiagnosticLogSnapshot {
  entries: DiagnosticLogEntry[];
  droppedEntries: number;
  slidingSync?: SlidingSyncDiagnostics;
}

export interface SlidingSyncDiagnostics {
  discoveryState:
    | "not_started"
    | "probing"
    | "supported"
    | "unsupported"
    | "unreachable"
    | "invalid_response";
  advertised: boolean;
  discoverySource: "unknown" | "versions";
  lastProbeAgeBucket: "never" | "<1m" | "1-5m" | "5-30m" | "30m-2h" | ">=2h";
  lastHttpStatusClass: "unknown" | "success" | "client_error" | "server_error" | "other";
  requestSchema: "element_x_all_rooms";
  engine: "SyncService";
  sdkSlidingSyncVersion: "unknown" | "none" | "native";
  roomListSharePos: boolean;
  encryptionSharePos: boolean;
  encryptionConnectionProfile: "sdk_default_encryption";
  encryptionExtensionProfile: "e2ee_to_device";
  provisionalEncryptionStarted: boolean;
  provisionalFirstResponseSeen: boolean;
  provisionalStoppedBeforeFirstResponse: boolean;
  provisionalToNormalHandoffBucket:
    | "never"
    | "under100_milliseconds"
    | "under_one_second"
    | "one_second_or_more";
  lifecycle: "stopped" | "starting" | "running" | "reconnecting" | "failed";
  connectivityProven: boolean;
  committedGeneration: number;
  lastSuccessAgeBucket: "never" | "<1m" | "1-5m" | "5-30m" | "30m-2h" | ">=2h";
  consecutiveFailureCount: number;
  lastFailureOrigin: "none" | "room_list" | "encryption" | "supervisor";
  lastFailureKind:
    | "none"
    | "sync_failed_http"
    | "sync_failed_auth"
    | "sync_failed_store"
    | "sync_failed_protocol"
    | "sync_failed_internal";
  lastFailureStage:
    | "none"
    | "room_list_sliding_sync"
    | "room_list_event_cache"
    | "room_list_projection"
    | "encryption_sliding_sync"
    | "encryption_lock"
    | "encryption_client"
    | "supervisor";
  lastHttpErrorSource:
    | "none"
    | "transport"
    | "server_response"
    | "response_decode"
    | "request_build"
    | "token_refresh"
    | "cached"
    | "tls"
    | "not_http";
  lastHttpStatus:
    | "none"
    | "bad_request"
    | "unauthorized"
    | "forbidden"
    | "not_found"
    | "rate_limited"
    | "client_error"
    | "server_error"
    | "other";
  lastMatrixErrorKind:
    | "none"
    | "unknown"
    | "bad_json"
    | "invalid_param"
    | "missing_param"
    | "not_json"
    | "not_found"
    | "unauthorized"
    | "missing_token"
    | "unknown_token"
    | "forbidden"
    | "unknown_pos"
    | "unrecognized"
    | "limit_exceeded"
    | "other";
  lastFailureRetryability: "none" | "transient" | "permanent" | "unknown";
  roomListTaskRunning: boolean;
  encryptionTaskRunning: boolean;
  posPresent: boolean;
  directAccountDataSource: "unavailable" | "local_store" | "sliding_sync_event";
  directMappedRoomCount: number;
  directTargetCount: number;
  projectedDmCount: number;
  explicitDmCount: number;
  fallbackDmCount: number;
  directNonDmCount: number;
  directInvalidEntryCount: number;
  directEventWakeCount: number;
  directEventAppliedCount: number;
  directEventStreamRunning: boolean;
}

export const DEFAULT_SLIDING_SYNC_DIAGNOSTICS: SlidingSyncDiagnostics = {
  discoveryState: "not_started",
  advertised: false,
  discoverySource: "unknown",
  lastProbeAgeBucket: "never",
  lastHttpStatusClass: "unknown",
  requestSchema: "element_x_all_rooms",
  engine: "SyncService",
  sdkSlidingSyncVersion: "unknown",
  roomListSharePos: true,
  encryptionSharePos: false,
  encryptionConnectionProfile: "sdk_default_encryption",
  encryptionExtensionProfile: "e2ee_to_device",
  provisionalEncryptionStarted: false,
  provisionalFirstResponseSeen: false,
  provisionalStoppedBeforeFirstResponse: false,
  provisionalToNormalHandoffBucket: "never",
  lifecycle: "stopped",
  connectivityProven: false,
  committedGeneration: 0,
  lastSuccessAgeBucket: "never",
  consecutiveFailureCount: 0,
  lastFailureOrigin: "none",
  lastFailureKind: "none",
  lastFailureStage: "none",
  lastHttpErrorSource: "none",
  lastHttpStatus: "none",
  lastMatrixErrorKind: "none",
  lastFailureRetryability: "none",
  roomListTaskRunning: false,
  encryptionTaskRunning: false,
  posPresent: false,
  directAccountDataSource: "unavailable",
  directMappedRoomCount: 0,
  directTargetCount: 0,
  projectedDmCount: 0,
  explicitDmCount: 0,
  fallbackDmCount: 0,
  directNonDmCount: 0,
  directInvalidEntryCount: 0,
  directEventWakeCount: 0,
  directEventAppliedCount: 0,
  directEventStreamRunning: false
};

export const DEFAULT_DIAGNOSTIC_LOG_LIMIT = 10_000;
export const DEFAULT_DIAGNOSTIC_PREVIEW_LIMIT = 100_000;

export function diagnosticReportPreview(
  report: string,
  limit = DEFAULT_DIAGNOSTIC_PREVIEW_LIMIT
): string {
  const normalizedLimit = Math.max(1, Math.trunc(limit));
  if (report.length <= normalizedLimit) {
    return report;
  }
  const heading = report.split("\n", 1)[0] || "Koushi diagnostics";
  const marker = `\npreview_truncated=true omitted_characters=${report.length - normalizedLimit}\n`;
  return `${heading}${marker}${report.slice(-normalizedLimit)}`;
}

export function schemaMismatchDiagnosticEntry(timestampMs: number): DiagnosticLogEntry {
  return { timestampMs, source: "snapshot", message: "schema_mismatch" };
}

export interface SecurityDiagnostics {
  secureContext: boolean;
  locationProtocol: string;
  locationOrigin: string;
  avatarImageSchemes: Record<string, number>;
  avatarBrokenImages: number;
}

export function createDiagnosticLogBuffer(
  limit = DEFAULT_DIAGNOSTIC_LOG_LIMIT
): {
  append(entry: DiagnosticLogEntry): void;
  snapshot(): Pick<DiagnosticLogSnapshot, "entries" | "droppedEntries">;
} {
  const normalizedLimit = Math.max(1, Math.trunc(limit));
  const entries = new Array<DiagnosticLogEntry>(normalizedLimit);
  let start = 0;
  let size = 0;
  let droppedEntries = 0;
  return {
    append(entry) {
      if (size < normalizedLimit) {
        entries[(start + size) % normalizedLimit] = entry;
        size += 1;
        return;
      }
      entries[start] = entry;
      start = (start + 1) % normalizedLimit;
      droppedEntries += 1;
    },
    snapshot() {
      return {
        entries: Array.from({ length: size }, (_, index) => entries[(start + index) % normalizedLimit]),
        droppedEntries
      };
    }
  };
}

export interface DiagnosticReportInput {
  snapshot: DesktopSnapshot;
  panelMode: RightPanelMode;
  sendStatus: QaSendSmokeStatus;
  timelineDiagnostics: QaTimelineDiagnostics;
  domDiagnostics: QaDomDiagnostics;
  uiLatencyDiagnostics: UiLatencyDiagnostics;
  stateDeltaStats?: AppStoreDeltaStats;
  timelineTransportStats?: TimelineTransportStats;
  jsErrors?: readonly CapturedJsError[];
  logEntries?: readonly DiagnosticLogEntry[];
  securityDiagnostics?: SecurityDiagnostics;
  droppedLogEntries?: number;
  slidingSyncDiagnostics?: SlidingSyncDiagnostics;
}

export function diagnosticReport({
  snapshot,
  panelMode,
  sendStatus,
  timelineDiagnostics,
  domDiagnostics,
  uiLatencyDiagnostics,
  stateDeltaStats,
  timelineTransportStats,
  jsErrors,
  logEntries = [],
  securityDiagnostics,
  droppedLogEntries,
  slidingSyncDiagnostics
}: DiagnosticReportInput): string {
  const crawler = summarizeCrawler(snapshot.state.domain.search_crawler.rooms);
  const roomClassification = summarizeRoomClassification(snapshot);
  const diagnosticLog = formatDiagnosticLog(logEntries);
  const securityDiagnosticLog = formatSecurityDiagnostics(securityDiagnostics);
  const lines = [
    "Koushi diagnostics",
    `Generated at: ${new Date().toISOString()}`,
    `Session: ${snapshot.state.domain.session.kind}`,
    `Sync: ${syncStateLabel(snapshot.state.domain.sync)}`,
    `Rooms: ${snapshot.state.domain.rooms.length}`,
    `Spaces: ${snapshot.state.domain.spaces.length}`,
    `Room classification: domain_dms=${roomClassification.domainDms} sidebar_dms=${roomClassification.sidebarDms} room_list_items=${roomClassification.roomListItems} room_list_dm_items=${roomClassification.roomListDmItems} active_filter=${roomClassification.activeFilter}`,
    `Active room selected: ${Boolean(snapshot.state.ui.navigation.active_room_id)}`,
    `Timeline room open: ${Boolean(snapshot.state.ui.timeline.room_id)}`,
    `Timeline matches active room: ${timelineMatchesActiveRoom(snapshot)}`,
    `Timeline subscribed: ${snapshot.state.ui.timeline.is_subscribed}`,
    `Timeline visible items: ${timelineDiagnostics.visibleItems}`,
    `Timeline downloaded event items: ${timelineDiagnostics.downloadedItems}`,
    `Timeline backfill: ${timelineDiagnostics.backfill}`,
    `Timeline avatars (item counts, not downloads; pending includes unrequested): mxc=${timelineDiagnostics.avatarMxcItems} ready=${timelineDiagnostics.avatarReadyItems} pending=${timelineDiagnostics.avatarPendingItems} failed=${timelineDiagnostics.avatarFailedItems} missing=${timelineDiagnostics.avatarMissingItems} rendered=${timelineDiagnostics.avatarRenderedImages} broken=${timelineDiagnostics.avatarBrokenImages}`,
    ...(crawler.running + crawler.queued > 0
      ? [
          `Potential UI load: search crawler running=${crawler.running} queued=${crawler.queued}; worker=1`
        ]
      : []),
    ...(uiLatencyDiagnostics.maxFrameGapMs >= 100
      ? [`Potential UI lag: max frame gap ${uiLatencyDiagnostics.maxFrameGapMs} ms`]
      : []),
    `UI frame gap: last=${uiLatencyDiagnostics.lastFrameGapMs}ms avg=${uiLatencyDiagnostics.averageFrameGapMs}ms max=${uiLatencyDiagnostics.maxFrameGapMs}ms longFrames=${uiLatencyDiagnostics.longFrameCount} samples=${uiLatencyDiagnostics.samples}`,
    ...(stateDeltaStats
      ? [
          `State transport: delta_applied=${stateDeltaStats.applied} stale_ignored=${stateDeltaStats.staleIgnored} gap_refresh=${stateDeltaStats.gapRefreshRequested}`
        ]
      : []),
    ...(timelineTransportStats
      ? [
          `Timeline transport: received=${timelineTransportStats.received} key_dropped=${timelineTransportStats.keyMismatchDropped} mismatch_groups=${formatCountGroups(timelineTransportStats.keyMismatchGroups)} initial_applied=${timelineTransportStats.initialItemsApplied} last_initial_items=${timelineTransportStats.lastInitialItemsCount} resync=${timelineTransportStats.resync}`
        ]
      : []),
    `Search crawler running=${crawler.running} queued=${crawler.queued}: processed=${crawler.processed} indexed=${crawler.indexed}`,
    `Search crawler completed=${crawler.completed} failed=${crawler.failed}`,
    `Right panel: ${panelMode}`,
    `Thread panel: ${threadPanelSummary(snapshot.state.ui.thread)}`,
    `Threads list: ${threadsListSummary(snapshot.state.ui.threads_list)}`,
    `QA send: ${sendStatus}`,
    `Errors: ${snapshot.state.ui.errors.length}`,
    `Latest error code: ${snapshot.state.ui.errors.at(-1)?.code ?? "none"}`,
    ...(jsErrors
      ? [
          `JS errors: ${jsErrors.length}`,
          ...jsErrors
            .slice(-5)
            .map(
              (error) =>
                `[js-error] channel=${error.channel} kind=${error.kind} age_bucket=${error.ageBucket} fingerprint=${error.fingerprint}`
            )
        ]
      : []),
    ...securityDiagnosticLog,
    ...formatSlidingSyncDiagnostics(slidingSyncDiagnostics),
    `Diagnostic records dropped: ${normalizeDroppedLogEntries(droppedLogEntries)}`,
    ...diagnosticLog,
    `timeline_matches_active=${timelineMatchesActiveRoom(snapshot)}`,
    ...qaSearchCrawlerDiagnosticTokens(snapshot),
    ...qaTimelineDiagnosticTokens(timelineDiagnostics),
    ...qaDomDiagnosticTokens(domDiagnostics),
    ...qaUiLatencyDiagnosticTokens(uiLatencyDiagnostics),
    ...(stateDeltaStats
      ? [
          `state_delta_applied=${stateDeltaStats.applied}`,
          `state_delta_stale_ignored=${stateDeltaStats.staleIgnored}`,
          `state_delta_gap_refresh=${stateDeltaStats.gapRefreshRequested}`
        ]
      : []),
    ...(timelineTransportStats
      ? [
          `timeline_evt_received=${timelineTransportStats.received}`,
          `timeline_evt_key_dropped=${timelineTransportStats.keyMismatchDropped}`,
          `timeline_evt_key_mismatch_groups=${formatCountGroups(timelineTransportStats.keyMismatchGroups)}`,
          `timeline_initial_applied=${timelineTransportStats.initialItemsApplied}`,
          `timeline_last_initial_items=${timelineTransportStats.lastInitialItemsCount}`,
          `timeline_resync=${timelineTransportStats.resync}`
        ]
      : []),
    ...(jsErrors ? [`js_error_count=${jsErrors.length}`] : [])
  ];
  return lines.join("\n");
}

function formatSlidingSyncDiagnostics(sync: SlidingSyncDiagnostics | undefined): string[] {
  if (!sync) {
    return [];
  }
  return [
    "Sliding Sync:",
    `sliding_sync.discovery_state=${sync.discoveryState}`,
    `sliding_sync.advertised=${sync.advertised}`,
    `sliding_sync.discovery_source=${sync.discoverySource}`,
    `sliding_sync.last_probe_age_bucket=${sync.lastProbeAgeBucket}`,
    `sliding_sync.last_http_status_class=${sync.lastHttpStatusClass}`,
    `sliding_sync.request_schema=${sync.requestSchema}`,
    `sync.engine=${sync.engine}`,
    `sync.sdk_sliding_sync_version=${sync.sdkSlidingSyncVersion}`,
    `sync.room_list_share_pos=${sync.roomListSharePos}`,
    `sync.encryption_share_pos=${sync.encryptionSharePos}`,
    `sync.encryption_connection_profile=${sync.encryptionConnectionProfile}`,
    `sync.encryption_extension_profile=${sync.encryptionExtensionProfile}`,
    `sync.provisional_encryption_started=${sync.provisionalEncryptionStarted}`,
    `sync.provisional_first_response_seen=${sync.provisionalFirstResponseSeen}`,
    `sync.provisional_stopped_before_first_response=${sync.provisionalStoppedBeforeFirstResponse}`,
    `sync.provisional_to_normal_handoff_bucket=${sync.provisionalToNormalHandoffBucket}`,
    `sync.lifecycle=${sync.lifecycle}`,
    `sync.connectivity_proven=${sync.connectivityProven}`,
    `sync.committed_generation=${Math.max(0, Math.trunc(sync.committedGeneration))}`,
    `sync.last_success_age_bucket=${sync.lastSuccessAgeBucket}`,
    `sync.consecutive_failure_count=${Math.max(0, Math.trunc(sync.consecutiveFailureCount))}`,
    `sync.last_failure_origin=${sync.lastFailureOrigin}`,
    `sync.last_failure_kind=${sync.lastFailureKind}`,
    `sync.last_failure_stage=${sync.lastFailureStage}`,
    `sync.last_http_error_source=${sync.lastHttpErrorSource}`,
    `sync.last_http_status=${sync.lastHttpStatus}`,
    `sync.last_matrix_error_kind=${sync.lastMatrixErrorKind}`,
    `sync.last_failure_retryability=${sync.lastFailureRetryability}`,
    `sync.room_list_task_running=${sync.roomListTaskRunning}`,
    `sync.encryption_task_running=${sync.encryptionTaskRunning}`,
    `sync.pos_present=${sync.posPresent}`,
    `direct_classification.source=${sync.directAccountDataSource}`,
    `direct_classification.mapped_room_count=${normalizedCount(sync.directMappedRoomCount)}`,
    `direct_classification.target_count=${normalizedCount(sync.directTargetCount)}`,
    `direct_classification.projected_dm_count=${normalizedCount(sync.projectedDmCount)}`,
    `direct_classification.explicit_dm_count=${normalizedCount(sync.explicitDmCount)}`,
    `direct_classification.fallback_dm_count=${normalizedCount(sync.fallbackDmCount)}`,
    `direct_classification.non_dm_count=${normalizedCount(sync.directNonDmCount)}`,
    `direct_classification.invalid_entry_count=${normalizedCount(sync.directInvalidEntryCount)}`,
    `direct_classification.event_wake_count=${normalizedCount(sync.directEventWakeCount)}`,
    `direct_classification.event_applied_count=${normalizedCount(sync.directEventAppliedCount)}`,
    `direct_classification.event_stream_running=${sync.directEventStreamRunning}`
  ];
}

function normalizedCount(value: number): number {
  return Number.isFinite(value) ? Math.max(0, Math.trunc(value)) : 0;
}

function formatCountGroups(groups: Record<string, number>): string {
  const entries = Object.entries(groups).sort(([left], [right]) => left.localeCompare(right));
  return entries.length === 0
    ? "none"
    : entries.map(([key, count]) => `${safeLogToken(key)}:${normalizedCount(count)}`).join(",");
}

function formatSecurityDiagnostics(security: SecurityDiagnostics | undefined): string[] {
  if (!security) {
    return [];
  }

  return [
    "Security diagnostics:",
    `security.secure_context=${security.secureContext}`,
    `security.location_protocol=${safeLogToken(security.locationProtocol)}`,
    `security.location_origin=${safeDiagnosticOrigin(security.locationOrigin)}`,
    `security.avatar_src_schemes=${formatSchemeCounts(security.avatarImageSchemes)}`,
    `security.avatar_broken_images=${Math.max(0, Math.trunc(security.avatarBrokenImages))}`
  ];
}

function normalizeDroppedLogEntries(value: number | undefined): number {
  return Number.isFinite(value) ? Math.max(0, Math.trunc(value ?? 0)) : 0;
}

function threadPanelSummary(thread: DiagnosticReportInput["snapshot"]["state"]["ui"]["thread"]): string {
  if (thread.kind !== "open") {
    return thread.kind;
  }
  return `open subscribed=${Boolean(thread.is_subscribed)}`;
}

function threadsListSummary(
  threadsList: DiagnosticReportInput["snapshot"]["state"]["ui"]["threads_list"]
): string {
  if (threadsList.kind !== "open") {
    return threadsList.kind;
  }
  return `open items=${threadsList.items.length} paginating=${threadsList.is_paginating} end=${threadsList.end_reached}`;
}

function summarizeRoomClassification(snapshot: DesktopSnapshot) {
  const dmRoomIds = new Set(
    snapshot.state.domain.rooms.filter((room) => room.is_dm).map((room) => room.room_id)
  );
  const roomListItems = snapshot.state.ui.room_list.items;
  return {
    domainDms: dmRoomIds.size,
    sidebarDms: snapshot.sidebar.global_dms.length,
    roomListItems: roomListItems?.length ?? 0,
    roomListDmItems:
      roomListItems?.filter(
        (item) =>
          (item.kind === "room" && dmRoomIds.has(item.room_id)) ||
          (item.kind === "invite" &&
            snapshot.state.domain.invites.some(
              (invite) => invite.room_id === item.room_id && invite.is_dm
            ))
      ).length ?? 0,
    activeFilter: safeLogToken(snapshot.state.ui.room_list.active_filter.kind)
  };
}

function formatDiagnosticLog(entries: readonly DiagnosticLogEntry[]): string[] {
  if (entries.length === 0) {
    return [];
  }
  return [
    "Diagnostic log:",
    ...[...entries]
      .sort((left, right) => left.timestampMs - right.timestampMs)
      .map((entry) => {
        const timestamp = new Date(entry.timestampMs);
        const timestampText = Number.isFinite(timestamp.getTime())
          ? timestamp.toISOString()
          : "invalid-time";
        return `[${timestampText}] ${safeLogToken(entry.source)} ${safeDiagnosticMessage(entry.message)}`;
      })
  ];
}

function safeLogToken(value: string): string {
  return value.replace(/[^a-z0-9_.:-]+/gi, "_").slice(0, 48) || "log";
}

function safeDiagnosticMessage(value: string): string {
  return value
    .replace(/![^\s]+/g, "<room>")
    .replace(/@[^\s]+/g, "<user>")
    .replace(/\$[^\s]+/g, "<event>");
}

function safeDiagnosticOrigin(value: string): string {
  try {
    const url = new URL(value);
    return `${safeLogToken(url.protocol)}//${safeLogToken(url.host)}`;
  } catch {
    return safeLogToken(value);
  }
}

function formatSchemeCounts(counts: Record<string, number>): string {
  const entries = Object.entries(counts)
    .map(([scheme, count]) => [safeLogToken(scheme), Math.max(0, Math.trunc(count))] as const)
    .filter(([, count]) => count > 0)
    .sort(([left], [right]) => left.localeCompare(right));
  return entries.length > 0
    ? entries.map(([scheme, count]) => `${scheme}:${count}`).join(",")
    : "none";
}

function summarizeCrawler(rooms: Record<string, SearchCrawlerRoomState>) {
  return Object.values(rooms).reduce(
    (summary, roomState) => {
      if (roomState.kind === "running") {
        summary.running += 1;
        summary.processed += roomState.processed;
        summary.indexed += roomState.indexed;
      } else if (roomState.kind === "queued") {
        summary.queued += 1;
      } else if (roomState.kind === "completed") {
        summary.completed += 1;
        summary.indexed += roomState.indexed;
      } else if (roomState.kind === "failed") {
        summary.failed += 1;
      }
      return summary;
    },
    { running: 0, queued: 0, completed: 0, failed: 0, processed: 0, indexed: 0 }
  );
}

function syncStateLabel(sync: SyncState): string {
  if (typeof sync === "string") {
    return sync;
  }
  if ("failed" in sync) {
    return "failed";
  }
  return "reconnecting";
}
