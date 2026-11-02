import type {
  ConversationAccess,
  RoomListProjection,
  SecureBackupFailureDetail,
  SecureBackupGateFailureKind,
  SecureBackupGateState,
  SessionState
} from "../domain/types";

/**
 * Test-only mirror of `AppState::conversation_access()` (#1748).
 *
 * Product code reads the Rust-projected `domain.conversation_access`; it never
 * re-derives admission. A headless fixture, though, injects a session, a backup
 * gate and a list readiness directly, so it has to stamp the same admission the
 * real projection would produce or the shell gate would disagree with the state
 * the spec just pushed. Keep this in step with
 * `crates/koushi-state/src/state/mod.rs`, `state/session.rs`, and
 * `crates/koushi-state/src/lib.rs`.
 */
export function conversationAccessFixture(
  session: SessionState,
  secureBackupGate: SecureBackupGateState,
  roomList: Pick<RoomListProjection, "readiness"> | null | undefined
): ConversationAccess {
  const readable = session.kind === "ready" && admitsCachedReading(secureBackupGate);
  return {
    readable,
    // A snapshot that carries no readiness at all cannot claim authority.
    list_authoritative: readable && roomList?.readiness?.kind === "ready",
    send_admitted: session.kind === "ready" && backupIsReady(secureBackupGate)
  };
}

function backupIsReady(gate: SecureBackupGateState): boolean {
  return (
    gate.kind === "ready" ||
    gate.kind === "uploadingExistingKeys" ||
    gate.kind === "degradedRetrying"
  );
}

function secureBackupFailureIsRetryable(
  kind: SecureBackupGateFailureKind,
  detail: SecureBackupFailureDetail | null | undefined
): boolean {
  if (detail) {
    return detail.retryable;
  }
  return kind === "network" || kind === "rateLimited" || kind === "timeout";
}

/** Mirror of `SecureBackupGateState::admits_cached_reading`. */
function admitsCachedReading(gate: SecureBackupGateState): boolean {
  switch (gate.kind) {
    case "inactive":
    case "checking":
    case "ready":
    case "uploadingExistingKeys":
    case "degradedRetrying":
      return true;
    case "blockedFailed":
      return secureBackupFailureIsRetryable(gate.failure, gate.detail);
    case "existingBackupNeedsRecovery":
    case "secureStorageIncomplete":
    case "setupRequired":
    case "explicitlyDisabledRequiresSetup":
    case "creatingBackup":
    case "recoveryKeyDeliveryRequired":
      return false;
  }
}
