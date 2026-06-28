import type { PaginationState } from "./coreEvents";

export type TimelineViewportCoverageSource =
  | "user-scroll"
  | "programmatic-restore"
  | "programmatic-navigation"
  | "layout-settle";

export type TimelineViewportCoverageInput = {
  source: TimelineViewportCoverageSource;
  relativeScrollTopPx: number;
  clientHeightPx: number;
  loadedContentHeightPx: number;
  coverageMarginPx: number;
  backwardPaginationState: PaginationState;
  autoLoadOlderMessages: boolean;
  forceUserBackfill: boolean;
  suppressPaginationUi: boolean;
  backfillInFlight: boolean;
  blockingAnchorWork: boolean;
};

export type TimelineViewportCoverageDecision =
  | { kind: "request-backward"; reason: "required-range-before-loaded-start" }
  | { kind: "covered"; reason: "required-range-within-loaded-content" }
  | {
      kind: "blocked";
      reason:
        | "pagination-ui-suppressed"
        | "blocking-anchor-work"
        | "backfill-in-flight"
        | "backward-pagination-unavailable"
        | "automatic-backfill-disabled";
    };

export function decideTimelineViewportCoverage(
  input: TimelineViewportCoverageInput
): TimelineViewportCoverageDecision {
  if (input.suppressPaginationUi) {
    return { kind: "blocked", reason: "pagination-ui-suppressed" };
  }
  if (input.blockingAnchorWork) {
    return { kind: "blocked", reason: "blocking-anchor-work" };
  }
  if (input.backfillInFlight) {
    return { kind: "blocked", reason: "backfill-in-flight" };
  }
  if (
    input.backwardPaginationState === "Paginating" ||
    input.backwardPaginationState === "EndReached"
  ) {
    return { kind: "blocked", reason: "backward-pagination-unavailable" };
  }
  if (!input.autoLoadOlderMessages && !input.forceUserBackfill) {
    return { kind: "blocked", reason: "automatic-backfill-disabled" };
  }

  const requiredTopPx = input.relativeScrollTopPx - input.coverageMarginPx;
  if (requiredTopPx <= 0) {
    return {
      kind: "request-backward",
      reason: "required-range-before-loaded-start"
    };
  }
  return { kind: "covered", reason: "required-range-within-loaded-content" };
}
