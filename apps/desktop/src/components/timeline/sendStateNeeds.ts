/**
 * #2135: which send-state actions need the server, asked in one place.
 *
 * Taking back a local, unsent message (Cancel / Delete) writes nothing to the
 * server, so it stays available while sending is locked. Retrying a send does
 * write, so it needs `canWrite`. The row and the unsent bar both consult this.
 */
export function canRetrySend(canWrite: boolean): boolean {
  return canWrite;
}

/** Cancelling a queued or failed local message never needs the server. */
export function canCancelSend(): boolean {
  return true;
}

/**
 * A message still queued while the send lock is on cannot go anywhere, so it is
 * shown as not sent instead of "Sending".
 */
export function displaySendStateKind(kind: string | null, canWrite: boolean): string | null {
  return kind === "sending" && !canWrite ? "notSent" : kind;
}
