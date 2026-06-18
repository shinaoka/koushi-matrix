import type { DesktopSnapshot, SyncState } from "./types";
import type { RightPanelMode } from "./rightPanel";
import type { QaSendSmokeStatus } from "./qaSendSmoke";
import { desktopAttentionSummary } from "./desktopAttention";

export function qaWindowTitle(
  snapshot: DesktopSnapshot,
  panelMode?: RightPanelMode,
  sendStatus?: QaSendSmokeStatus
): string {
  const attention = desktopAttentionSummary(snapshot.state.native_attention);
  const roomInteractions = Object.values(snapshot.state.room_interactions);
  const pinnedCount = roomInteractions.reduce(
    (count, interaction) => count + interaction.pinned_events.length,
    0
  );
  const pinOperationCount = roomInteractions.filter(
    (interaction) => interaction.pin_operation.kind !== "idle"
  ).length;
  const title = [
    "matrix-desktop qa",
    `session=${snapshot.state.session.kind}`,
    `sync=${syncStateLabel(snapshot.state.sync)}`,
    `rooms=${snapshot.state.rooms.length}`,
    `spaces=${snapshot.state.spaces.length}`,
    `active_room=${Boolean(snapshot.state.navigation.active_room_id)}`,
    `timeline_room=${Boolean(snapshot.state.timeline.room_id)}`,
    `timeline_subscribed=${snapshot.state.timeline.is_subscribed}`,
    `timeline_items=${snapshot.timeline.length}`,
    `pinned=${pinnedCount}`,
    `pin_ops=${pinOperationCount}`,
    `errors=${snapshot.state.errors.length}`,
    `focused=${snapshot.state.focused_context.kind}`,
    attention.qaTitleToken
  ];
  if (panelMode !== undefined) {
    title.push(`panel=${panelMode}`);
  }
  if (sendStatus !== undefined) {
    title.push(`send=${sendStatus}`);
  }
  return title.join(" ");
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
