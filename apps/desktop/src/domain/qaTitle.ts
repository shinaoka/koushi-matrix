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
    `errors=${snapshot.state.errors.length}`,
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
