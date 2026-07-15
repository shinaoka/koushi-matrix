import type { PaginationState } from "./coreEvents";

export type TimelineBackfillEvaluationTrigger =
  | "initial_projection"
  | "layout_settled"
  | "room_anchor_settled"
  | "user_scroll"
  | "pagination_terminal"
  | "prepend_settled"
  | "resync_replayed"
  | "setting_changed"
  | "timeline_reset"
  | "live_edge_settled";

export type TimelineBackfillDemand =
  | "underfilled"
  | "near_top_prefetch"
  | "explicit_top_scroll";

export type TimelineBackfillBlocker =
  | "not_initialized"
  | "awaiting_resync"
  | "suppressed_ui"
  | "projection_unsettled"
  | "virtual_layout_unsettled"
  | "anchor_unsettled"
  | "request_in_flight"
  | "pagination_paginating"
  | "pagination_end_reached";

export type TimelineBackfillEvaluation =
  | { kind: "request"; demand: TimelineBackfillDemand }
  | {
      kind: "blocked";
      demand: TimelineBackfillDemand;
      reason: TimelineBackfillBlocker;
    }
  | { kind: "idle"; reason: "auto_load_disabled" | "no_demand" };

export interface TimelineBackfillSnapshot {
  trigger: TimelineBackfillEvaluationTrigger;
  initialized: boolean;
  awaitingResync: boolean;
  suppressPaginationUi: boolean;
  autoLoadEnabled: boolean;
  paginationState: PaginationState;
  requestInFlight: boolean;
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
  if (!snapshot.autoLoadEnabled) {
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
  if (snapshot.awaitingResync) return "awaiting_resync";
  if (snapshot.suppressPaginationUi) return "suppressed_ui";
  if (!snapshot.projectionSettled) return "projection_unsettled";
  if (!snapshot.virtualLayoutSettled) return "virtual_layout_unsettled";
  if (!snapshot.anchorSettled) return "anchor_unsettled";
  if (snapshot.requestInFlight) return "request_in_flight";
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
      reason: snapshot.autoLoadEnabled ? "no_demand" : "auto_load_disabled"
    };
  }

  const blocker = blockerForSnapshot(snapshot);
  if (blocker !== null) {
    return { kind: "blocked", demand, reason: blocker };
  }
  return { kind: "request", demand };
}
