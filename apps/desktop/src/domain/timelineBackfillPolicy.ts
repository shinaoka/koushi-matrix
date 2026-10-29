import type { PaginationState } from "./coreEvents";

export type TimelineBackfillEvaluationTrigger =
  | "initial_projection"
  | "layout_settled"
  | "room_anchor_settled"
  | "user_scroll"
  | "pagination_terminal"
  | "prepend_settled"
  | "gap_projection_changed"
  | "gap_repair_released"
  | "resync_replayed"
  | "timeline_reset"
  | "live_edge_settled"
  | "retry_backoff_elapsed";

/**
 * #1473: whether a trigger may clear the `external_transition` retry fence set
 * by a failed or non-terminal pagination. Self-induced and geometry-only
 * settlements (the pagination's own terminal, layout, anchor, live edge) do not
 * count as an external transition, otherwise a permanently failing request
 * (for example a thread whose root the server withholds) retries in a tight
 * loop. Only a genuine user scroll, a real projection change, or a
 * bounded backoff retry ({@link externalTransitionAutoRetryDelayMs}) clears it.
 */
export function clearsExternalTransitionFence(
  trigger: TimelineBackfillEvaluationTrigger,
  genuineUserScroll: boolean
): boolean {
  switch (trigger) {
    case "pagination_terminal":
    case "layout_settled":
    case "room_anchor_settled":
    case "live_edge_settled":
      return false;
    case "user_scroll":
      return genuineUserScroll;
    case "initial_projection":
    case "prepend_settled":
    case "gap_projection_changed":
    case "gap_repair_released":
    case "resync_replayed":
    case "timeline_reset":
    case "retry_backoff_elapsed":
      return true;
  }
}

/**
 * #1473: backoff before each automatic retry of a fenced backfill, so a
 * transient failure in an underfilled timeline (where no user scroll is
 * possible) still recovers while a permanent failure stops after a few tries.
 */
export const EXTERNAL_TRANSITION_AUTO_RETRY_DELAYS_MS: readonly number[] = [2_000, 8_000, 30_000];

/** Delay before the next automatic retry, or `null` once the budget is spent. */
export function externalTransitionAutoRetryDelayMs(attemptsUsed: number): number | null {
  return EXTERNAL_TRANSITION_AUTO_RETRY_DELAYS_MS[attemptsUsed] ?? null;
}

export type TimelineBackfillDemand =
  | "underfilled"
  | "near_top_prefetch"
  | "explicit_top_scroll";

export type TimelineBackfillBlocker =
  | "not_initialized"
  | "semantic_ineligible"
  | "awaiting_resync"
  | "suppressed_ui"
  | "projection_unsettled"
  | "virtual_layout_unsettled"
  | "anchor_unsettled"
  | "request_in_flight"
  | "retry_waiting_for_transition"
  | "pagination_paginating"
  | "pagination_end_reached";

export type TimelineBackfillEvaluation =
  | { kind: "request"; demand: TimelineBackfillDemand }
  | {
      kind: "blocked";
      demand: TimelineBackfillDemand;
      reason: TimelineBackfillBlocker;
    }
  | { kind: "idle"; reason: "viewport_unmeasured" | "no_demand" };

export interface TimelineBackfillSnapshot {
  trigger: TimelineBackfillEvaluationTrigger;
  initialized: boolean;
  awaitingResync: boolean;
  suppressPaginationUi: boolean;
  automaticBackfillEligible: boolean;
  paginationState: PaginationState;
  requestInFlight: boolean;
  retryBlocked: boolean;
  projectionSettled: boolean;
  virtualLayoutSettled: boolean;
  anchorSettled: boolean;
  itemCount: number;
  projectedContentHeight: number;
  clientHeight: number;
  scrollHeight: number;
  scrollTop: number;
  nearTopThreshold: number;
  genuineUserScroll: boolean;
}

function demandForSnapshot(
  snapshot: TimelineBackfillSnapshot
): TimelineBackfillDemand | null {
  if (snapshot.genuineUserScroll && snapshot.scrollTop <= 0) {
    return "explicit_top_scroll";
  }
  if (snapshot.clientHeight <= 0) {
    return null;
  }

  const projectedUnderfilled =
    snapshot.projectedContentHeight <= 0 ||
    snapshot.projectedContentHeight <= snapshot.clientHeight;
  const domUnderfilled = snapshot.scrollHeight <= snapshot.clientHeight;
  if (projectedUnderfilled && domUnderfilled) {
    return "underfilled";
  }
  if (snapshot.scrollTop <= snapshot.nearTopThreshold) {
    return "near_top_prefetch";
  }
  return null;
}

function blockerForSnapshot(
  snapshot: TimelineBackfillSnapshot
): TimelineBackfillBlocker | null {
  if (!snapshot.initialized) return "not_initialized";
  if (!snapshot.automaticBackfillEligible) return "semantic_ineligible";
  if (snapshot.awaitingResync) return "awaiting_resync";
  if (snapshot.suppressPaginationUi) return "suppressed_ui";
  if (!snapshot.projectionSettled) return "projection_unsettled";
  if (!snapshot.virtualLayoutSettled) return "virtual_layout_unsettled";
  if (!snapshot.anchorSettled) return "anchor_unsettled";
  if (snapshot.requestInFlight) return "request_in_flight";
  if (snapshot.retryBlocked) return "retry_waiting_for_transition";
  if (snapshot.paginationState === "Paginating") return "pagination_paginating";
  if (snapshot.paginationState === "EndReached") return "pagination_end_reached";
  return null;
}

export function evaluateTimelineBackfill(
  snapshot: TimelineBackfillSnapshot
): TimelineBackfillEvaluation {
  const demand = demandForSnapshot(snapshot);
  if (demand === null) {
    return {
      kind: "idle",
      reason: snapshot.clientHeight > 0 ? "no_demand" : "viewport_unmeasured"
    };
  }

  const blocker = blockerForSnapshot(snapshot);
  if (blocker !== null) {
    return { kind: "blocked", demand, reason: blocker };
  }
  return { kind: "request", demand };
}

export interface TimelineForwardFillSnapshot {
  initialized: boolean;
  awaitingResync: boolean;
  paginationState: PaginationState;
  requestInFlight: boolean;
  projectionSettled: boolean;
  genuineUserScroll: boolean;
  clientHeight: number;
  scrollHeight: number;
  scrollTop: number;
  nearBottomThreshold: number;
}

/**
 * #1668: an anchored (`Focused`) timeline reaches newer history only through
 * forward pagination. Request the next newer page while the reader is near the
 * loaded bottom, until Rust reports `EndReached`. A failed page retries only on
 * a genuine user scroll, so a permanent failure cannot loop.
 */
export function shouldRequestTimelineForwardFill(
  snapshot: TimelineForwardFillSnapshot
): boolean {
  if (!snapshot.initialized || snapshot.awaitingResync) return false;
  if (snapshot.requestInFlight || !snapshot.projectionSettled) return false;
  if (snapshot.paginationState === "Paginating" || snapshot.paginationState === "EndReached") {
    return false;
  }
  if (typeof snapshot.paginationState === "object" && !snapshot.genuineUserScroll) return false;
  if (snapshot.clientHeight <= 0) return false;
  const distanceToBottom = snapshot.scrollHeight - snapshot.clientHeight - snapshot.scrollTop;
  return distanceToBottom <= snapshot.nearBottomThreshold;
}
