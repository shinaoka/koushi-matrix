import type { DesktopSnapshot, SearchCrawlerRoomState, SyncState } from "./types";
import {
  qaDomDiagnosticTokens,
  qaSearchCrawlerDiagnosticTokens,
  qaTimelineDiagnosticTokens,
  type QaDomDiagnostics,
  type QaTimelineDiagnostics
} from "./qaTitle";
import type { RightPanelMode } from "./rightPanel";
import type { QaSendSmokeStatus } from "./qaSendSmoke";

export interface DiagnosticReportInput {
  snapshot: DesktopSnapshot;
  panelMode: RightPanelMode;
  sendStatus: QaSendSmokeStatus;
  timelineDiagnostics: QaTimelineDiagnostics;
  domDiagnostics: QaDomDiagnostics;
}

export function diagnosticReport({
  snapshot,
  panelMode,
  sendStatus,
  timelineDiagnostics,
  domDiagnostics
}: DiagnosticReportInput): string {
  const crawler = summarizeCrawler(snapshot.state.domain.search_crawler.rooms);
  const lines = [
    "Koushi diagnostics",
    `Session: ${snapshot.state.domain.session.kind}`,
    `Sync: ${syncStateLabel(snapshot.state.domain.sync)}`,
    `Rooms: ${snapshot.state.domain.rooms.length}`,
    `Spaces: ${snapshot.state.domain.spaces.length}`,
    `Active room selected: ${Boolean(snapshot.state.ui.navigation.active_room_id)}`,
    `Timeline room open: ${Boolean(snapshot.state.ui.timeline.room_id)}`,
    `Timeline subscribed: ${snapshot.state.ui.timeline.is_subscribed}`,
    `Timeline visible items: ${timelineDiagnostics.visibleItems}`,
    `Timeline downloaded event items: ${timelineDiagnostics.downloadedItems}`,
    `Timeline backfill: ${timelineDiagnostics.backfill}`,
    `Downloading messages from ${crawler.running} room(s): processed=${crawler.processed} indexed=${crawler.indexed}`,
    `Search crawler completed=${crawler.completed} failed=${crawler.failed}`,
    `Right panel: ${panelMode}`,
    `QA send: ${sendStatus}`,
    `Errors: ${snapshot.state.ui.errors.length}`,
    `Latest error code: ${snapshot.state.ui.errors.at(-1)?.code ?? "none"}`,
    ...qaSearchCrawlerDiagnosticTokens(snapshot),
    ...qaTimelineDiagnosticTokens(timelineDiagnostics),
    ...qaDomDiagnosticTokens(domDiagnostics)
  ];
  return lines.join("\n");
}

function summarizeCrawler(rooms: Record<string, SearchCrawlerRoomState>) {
  return Object.values(rooms).reduce(
    (summary, roomState) => {
      if (roomState.kind === "running") {
        summary.running += 1;
        summary.processed += roomState.processed;
        summary.indexed += roomState.indexed;
      } else if (roomState.kind === "completed") {
        summary.completed += 1;
        summary.indexed += roomState.indexed;
      } else if (roomState.kind === "failed") {
        summary.failed += 1;
      }
      return summary;
    },
    { running: 0, completed: 0, failed: 0, processed: 0, indexed: 0 }
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
