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

export const DEFAULT_DIAGNOSTIC_LOG_LIMIT = 10_000;

export interface SecurityDiagnostics {
  secureContext: boolean;
  locationProtocol: string;
  locationOrigin: string;
  avatarImageSchemes: Record<string, number>;
  avatarBrokenImages: number;
}

export interface VerboseDiagnostics {
  enabled: boolean;
  security?: SecurityDiagnostics;
}

export function appendDiagnosticLogEntry(
  entries: readonly DiagnosticLogEntry[],
  entry: DiagnosticLogEntry,
  limit = DEFAULT_DIAGNOSTIC_LOG_LIMIT
): DiagnosticLogEntry[] {
  const normalizedLimit = Math.max(1, Math.trunc(limit));
  return [...entries, entry].slice(-normalizedLimit);
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
  verboseDiagnostics?: VerboseDiagnostics;
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
  verboseDiagnostics
}: DiagnosticReportInput): string {
  const crawler = summarizeCrawler(snapshot.state.domain.search_crawler.rooms);
  const roomClassification = summarizeRoomClassification(snapshot);
  const diagnosticLog = formatDiagnosticLog(logEntries);
  const verboseDiagnosticLog = formatVerboseDiagnostics(verboseDiagnostics);
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
    `Timeline avatars: mxc=${timelineDiagnostics.avatarMxcItems} ready=${timelineDiagnostics.avatarReadyItems} pending=${timelineDiagnostics.avatarPendingItems} failed=${timelineDiagnostics.avatarFailedItems} missing=${timelineDiagnostics.avatarMissingItems} rendered=${timelineDiagnostics.avatarRenderedImages} broken=${timelineDiagnostics.avatarBrokenImages}`,
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
          `Timeline transport: received=${timelineTransportStats.received} key_dropped=${timelineTransportStats.keyMismatchDropped} initial_applied=${timelineTransportStats.initialItemsApplied} last_initial_items=${timelineTransportStats.lastInitialItemsCount}`
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
                `[js-error] kind=${error.kind} source=${error.source} message=${error.message}`
            )
        ]
      : []),
    ...verboseDiagnosticLog,
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
          `timeline_initial_applied=${timelineTransportStats.initialItemsApplied}`,
          `timeline_last_initial_items=${timelineTransportStats.lastInitialItemsCount}`
        ]
      : []),
    ...(jsErrors ? [`js_error_count=${jsErrors.length}`] : [])
  ];
  return lines.join("\n");
}

function formatVerboseDiagnostics(verboseDiagnostics: VerboseDiagnostics | undefined): string[] {
  if (!verboseDiagnostics?.enabled) {
    return ["Verbose diagnostics: disabled"];
  }

  const lines = ["Verbose diagnostics: enabled"];
  if (!verboseDiagnostics.security) {
    return lines;
  }

  const security = verboseDiagnostics.security;
  lines.push(
    "Security diagnostics:",
    `security.secure_context=${security.secureContext}`,
    `security.location_protocol=${safeLogToken(security.locationProtocol)}`,
    `security.location_origin=${safeDiagnosticOrigin(security.locationOrigin)}`,
    `security.avatar_src_schemes=${formatSchemeCounts(security.avatarImageSchemes)}`,
    `security.avatar_broken_images=${Math.max(0, Math.trunc(security.avatarBrokenImages))}`
  );
  return lines;
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
    "Timeline log:",
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
