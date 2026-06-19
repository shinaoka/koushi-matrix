import type { DesktopSnapshot, SyncState } from "./types";
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
    "matrix-desktop qa",
    `session=${snapshot.state.domain.session.kind}`,
    `sync=${syncStateLabel(snapshot.state.domain.sync)}`,
    `rooms=${snapshot.state.domain.rooms.length}`,
    `spaces=${snapshot.state.domain.spaces.length}`,
    `active_room=${Boolean(snapshot.state.ui.navigation.active_room_id)}`,
    `timeline_room=${Boolean(snapshot.state.ui.timeline.room_id)}`,
    `timeline_subscribed=${snapshot.state.ui.timeline.is_subscribed}`,
    `timeline_items=${snapshot.timeline.length}`,
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

function latestErrorCode(snapshot: DesktopSnapshot): string {
  return snapshot.state.ui.errors.at(-1)?.code ?? "none";
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
