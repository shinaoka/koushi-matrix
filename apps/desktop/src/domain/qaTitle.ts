import type { DesktopSnapshot, SyncState } from "./types";

export function qaWindowTitle(snapshot: DesktopSnapshot): string {
  return [
    "matrix-desktop qa",
    `session=${snapshot.state.session.kind}`,
    `sync=${syncStateLabel(snapshot.state.sync)}`,
    `rooms=${snapshot.state.rooms.length}`,
    `spaces=${snapshot.state.spaces.length}`,
    `active_room=${Boolean(snapshot.state.navigation.active_room_id)}`,
    `timeline_subscribed=${snapshot.state.timeline.is_subscribed}`,
    `timeline_items=${snapshot.timeline.length}`,
    `errors=${snapshot.state.errors.length}`
  ].join(" ");
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
