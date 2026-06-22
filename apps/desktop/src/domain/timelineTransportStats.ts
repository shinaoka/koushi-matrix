/**
 * Private-data-free counters for the event-driven timeline CoreEvent channel
 * (separate from the appStore StateDelta path), surfaced in the diagnostic
 * report. They disambiguate an empty timeline:
 *   - received=0                 -> Rust is not emitting Timeline events / the
 *                                   listener is not attached.
 *   - received>0, keyMismatch≈received -> events arrive but are dropped by the
 *                                   timelineKey equality filter (key mismatch).
 *   - initialItemsApplied>0, lastInitialItemsCount=0 -> events arrive and apply
 *                                   but carry no items (empty InitialItems).
 * Counts are cumulative for the session; no Matrix ids or bodies are recorded.
 */
export interface TimelineTransportStats {
  received: number;
  keyMismatchDropped: number;
  initialItemsApplied: number;
  lastInitialItemsCount: number;
}

let stats: TimelineTransportStats = {
  received: 0,
  keyMismatchDropped: 0,
  initialItemsApplied: 0,
  lastInitialItemsCount: 0
};

export function recordTimelineEventReceived(): void {
  stats.received += 1;
}

export function recordTimelineKeyMismatch(): void {
  stats.keyMismatchDropped += 1;
}

export function recordTimelineInitialItems(count: number): void {
  stats.initialItemsApplied += 1;
  stats.lastInitialItemsCount = Math.max(0, Math.trunc(count));
}

export function getTimelineTransportStats(): TimelineTransportStats {
  return { ...stats };
}

export function resetTimelineTransportStats(): void {
  stats = {
    received: 0,
    keyMismatchDropped: 0,
    initialItemsApplied: 0,
    lastInitialItemsCount: 0
  };
}
