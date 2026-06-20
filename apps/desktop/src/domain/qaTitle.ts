import type { DesktopSnapshot, SearchCrawlerRoomState, SyncState } from "./types";
import type { RightPanelMode } from "./rightPanel";
import type { QaSendSmokeStatus } from "./qaSendSmoke";
import { desktopAttentionSummary } from "./desktopAttention";

export function qaWindowTitle(
  snapshot: DesktopSnapshot,
  panelMode?: RightPanelMode,
  sendStatus?: QaSendSmokeStatus,
  diagnosticTokens: string[] = []
): string {
  const attention = desktopAttentionSummary(snapshot.state.domain.native_attention);
  const roomInteractions = Object.values(snapshot.state.domain.room_interactions);
  const pinnedCount = roomInteractions.reduce(
    (count, interaction) => count + interaction.pinned_events.length,
    0
  );
  const pinOperationCount = roomInteractions.filter(
    (interaction) => interaction.pin_operation.kind !== "idle"
  ).length;
  const title = [
    "koushi-desktop qa",
    `session=${snapshot.state.domain.session.kind}`,
    `sync=${syncStateLabel(snapshot.state.domain.sync)}`,
    `rooms=${snapshot.state.domain.rooms.length}`,
    `spaces=${snapshot.state.domain.spaces.length}`,
    `active_room=${Boolean(snapshot.state.ui.navigation.active_room_id)}`,
    `timeline_room=${Boolean(snapshot.state.ui.timeline.room_id)}`,
    `timeline_subscribed=${snapshot.state.ui.timeline.is_subscribed}`,
    `timeline_items=${snapshot.timeline.length}`,
    ...qaSearchCrawlerDiagnosticTokens(snapshot),
    `pinned=${pinnedCount}`,
    `pin_ops=${pinOperationCount}`,
    `errors=${snapshot.state.ui.errors.length}`,
    `error_code=${latestErrorCode(snapshot)}`,
    `focused=${snapshot.state.ui.focused_context.kind}`,
    attention.qaTitleToken
  ];
  if (panelMode !== undefined) {
    title.push(`panel=${panelMode}`);
  }
  if (sendStatus !== undefined) {
    title.push(`send=${sendStatus}`);
  }
  title.push(...diagnosticTokens);
  return title.join(" ");
}

export interface QaTimelineDiagnostics {
  visibleItems: number;
  downloadedItems: number;
  backfill: string;
}

export interface QaDomDiagnostics {
  screen: string;
  rootChildren: number;
  bodyTextLength: number;
}

export function qaTimelineDiagnosticTokens(diagnostics: QaTimelineDiagnostics): string[] {
  return [
    `timeline_visible=${diagnostics.visibleItems}`,
    `timeline_dl=${diagnostics.downloadedItems}`,
    `timeline_backfill=${diagnostics.backfill}`
  ];
}

export function qaDomDiagnosticTokens(diagnostics: QaDomDiagnostics): string[] {
  return [
    `dom_screen=${safeQaToken(diagnostics.screen)}`,
    `dom_root_children=${Math.max(0, Math.trunc(diagnostics.rootChildren))}`,
    `dom_text_len=${Math.max(0, Math.trunc(diagnostics.bodyTextLength))}`
  ];
}

export function qaSearchCrawlerDiagnosticTokens(snapshot: DesktopSnapshot): string[] {
  const summary = Object.values(snapshot.state.domain.search_crawler.rooms).reduce(
    (current, roomState) => summarizeCrawlerRoomState(current, roomState),
    {
      running: 0,
      completed: 0,
      failed: 0,
      processed: 0,
      indexed: 0
    }
  );
  return [
    `crawler_running=${summary.running}`,
    `crawler_completed=${summary.completed}`,
    `crawler_failed=${summary.failed}`,
    `crawler_processed=${summary.processed}`,
    `crawler_indexed=${summary.indexed}`
  ];
}

function summarizeCrawlerRoomState(
  current: {
    running: number;
    completed: number;
    failed: number;
    processed: number;
    indexed: number;
  },
  roomState: SearchCrawlerRoomState
) {
  if (roomState.kind === "running") {
    current.running += 1;
    current.processed += roomState.processed;
    current.indexed += roomState.indexed;
  } else if (roomState.kind === "completed") {
    current.completed += 1;
    current.indexed += roomState.indexed;
  } else if (roomState.kind === "failed") {
    current.failed += 1;
  }
  return current;
}

function latestErrorCode(snapshot: DesktopSnapshot): string {
  return snapshot.state.ui.errors.at(-1)?.code ?? "none";
}

function safeQaToken(value: string): string {
  const safe = value.replace(/[^A-Za-z0-9_.-]/g, "_").slice(0, 48);
  return safe || "unknown";
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
