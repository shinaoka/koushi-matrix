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
 * A message still queued while the send lock is on cannot go anywhere, and it
 * leaves by itself when the lock lifts. It therefore gets its own display kind,
 * "locked" ("Waiting to send"), instead of "Sending" (untrue) or "Not sent"
 * (which a real failure also reads, and which needs a manual Resend).
 */
export function displaySendStateKind(kind: string | null, canWrite: boolean): string | null {
  return kind === "sending" && !canWrite ? "locked" : kind;
}

/**
 * Whether a row counts as unsent for the bar. The bar and the row share this
 * rule so two rows that read as unsent are never split between them: a real
 * failure, or a message waiting behind the send lock.
 */
export function isUnsentForDisplay(kind: string | null, canWrite: boolean): boolean {
  const shown = displaySendStateKind(kind, canWrite);
  return shown === "notSent" || shown === "locked";
}
