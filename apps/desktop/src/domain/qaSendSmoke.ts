import type { DesktopSnapshot, RoomSummary, SyncState } from "./types";
import type { CoreEventPayload } from "./coreEvents";

export type QaSendSmokeStatus = "idle" | "pending" | "sent" | "failed";

export function qaSendSmokeMessageFromEnv(value: string | undefined): string | null {
  const message = value?.trim();
  return message ? message : null;
}

export function qaSendSmokeTargetUserIdFromEnv(value: string | undefined): string | null {
  const trimmed = value?.trim();
  if (!trimmed) {
    return null;
  }
  return trimmed.startsWith("@") ? trimmed : `@${trimmed}`;
}

export function qaSendSmokeTargetRoom(
  snapshot: DesktopSnapshot,
  userId: string
): RoomSummary | null {
  return (
    snapshot.state.domain.rooms.find(
      (room) => room.is_dm && room.dm_user_ids.includes(userId)
    ) ?? null
  );
}

export function qaSendSmokeTargetDiagnosticTokens(
  snapshot: DesktopSnapshot,
  userId: string | null
): string[] {
  if (!userId) {
    return [];
  }
  const room = qaSendSmokeTargetRoom(snapshot, userId);
  if (!room) {
    return ["target_dm=missing", "target_selected=false", "target_members=unknown"];
  }
  const selected = snapshot.state.ui.timeline.room_id === room.room_id;
  return [
    `target_dm=${room.is_encrypted ? "encrypted" : "plain"}`,
    `target_selected=${selected}`,
    `target_members=${room.joined_members ?? "unknown"}`
  ];
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
