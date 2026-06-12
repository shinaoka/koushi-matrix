import type { DesktopSnapshot, SyncState } from "./types";

export type QaSendSmokeStatus = "none" | "sending" | "sent" | "failed";

export function qaSendSmokeMessageFromEnv(value: string | undefined): string | null {
  const message = value?.trim();
  return message ? message : null;
}

export function qaSendSmokeCanStart(snapshot: DesktopSnapshot): boolean {
  return (
    snapshot.state.session.kind === "ready" &&
    syncStateLabel(snapshot.state.sync) === "running" &&
    Boolean(snapshot.state.timeline.room_id) &&
    snapshot.state.timeline.is_subscribed &&
    snapshot.state.timeline.composer.pending_transaction_id === null &&
    snapshot.state.errors.length === 0
  );
}

export function qaSendSmokeCompletionStatus(
  snapshot: DesktopSnapshot,
  baselineErrorCount: number,
  baselineTimelineItems = 0
): QaSendSmokeStatus {
  if (snapshot.state.errors.length > baselineErrorCount) {
    return "failed";
  }
  if (snapshot.state.timeline.composer.pending_transaction_id !== null) {
    return "sending";
  }
  if (snapshot.timeline.length > baselineTimelineItems) {
    return "sent";
  }
  return "sending";
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
