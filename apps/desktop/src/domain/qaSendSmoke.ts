import type { DesktopSnapshot, SyncState } from "./types";
import type { CoreEventPayload } from "./coreEvents";

export type QaSendSmokeStatus = "idle" | "pending" | "sent" | "failed";

export function qaSendSmokeMessageFromEnv(value: string | undefined): string | null {
  const message = value?.trim();
  return message ? message : null;
}

export function qaSendSmokeCanStart(snapshot: DesktopSnapshot): boolean {
  return (
    snapshot.state.domain.session.kind === "ready" &&
    syncStateLabel(snapshot.state.domain.sync) === "running" &&
    Boolean(snapshot.state.ui.timeline.room_id) &&
    snapshot.state.ui.timeline.is_subscribed &&
    snapshot.state.ui.timeline.composer.pending_transaction_id === null &&
    snapshot.state.ui.errors.length === 0
  );
}

export function qaSendSmokeCompletionStatus(
  snapshot: DesktopSnapshot,
  baselineErrorCount: number,
  baselineTimelineItems = 0
): QaSendSmokeStatus {
  if (snapshot.state.ui.errors.length > baselineErrorCount) {
    return "failed";
  }
  if (snapshot.state.ui.timeline.composer.pending_transaction_id !== null) {
    return "pending";
  }
  if (snapshot.timeline.length > baselineTimelineItems) {
    return "sent";
  }
  return "idle";
}

export function qaSendCompletionStatusFromCoreEvent(
  payload: CoreEventPayload
): Exclude<QaSendSmokeStatus, "idle" | "pending"> | null {
  if (payload.kind === "Timeline" && "SendCompleted" in payload.event) {
    return "sent";
  }
  if (payload.kind === "OperationFailed") {
    return "failed";
  }
  return null;
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
