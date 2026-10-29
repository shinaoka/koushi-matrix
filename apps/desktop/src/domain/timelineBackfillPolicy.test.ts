import { describe, expect, test } from "vitest";

import type { PaginationState } from "./coreEvents";
import {
  clearsExternalTransitionFence,
  EXTERNAL_TRANSITION_AUTO_RETRY_DELAYS_MS,
  evaluateTimelineBackfill,
  externalTransitionAutoRetryDelayMs,
  shouldRequestTimelineForwardFill,
  type TimelineBackfillBlocker,
  type TimelineBackfillSnapshot,
  type TimelineForwardFillSnapshot
} from "./timelineBackfillPolicy";

function snapshot(
  overrides: Partial<TimelineBackfillSnapshot> = {}
): TimelineBackfillSnapshot {
  return {
    trigger: "layout_settled",
    initialized: true,
    awaitingResync: false,
    suppressPaginationUi: false,
    automaticBackfillEligible: true,
    paginationState: "Idle",
    requestInFlight: false,
    retryBlocked: false,
    projectionSettled: true,
    virtualLayoutSettled: true,
    anchorSettled: true,
    itemCount: 20,
    projectedContentHeight: 1_200,
    clientHeight: 600,
    scrollHeight: 1_200,
    scrollTop: 400,
    nearTopThreshold: 160,
    genuineUserScroll: false,
    ...overrides
  };
}

describe("evaluateTimelineBackfill", () => {
  test("blocks every settlement trigger for a new-thread draft", () => {
    const triggers: TimelineBackfillSnapshot["trigger"][] = [
      "initial_projection",
      "layout_settled",
      "pagination_terminal",
      "gap_projection_changed",
      "gap_repair_released",
      "resync_replayed",
      "timeline_reset",
      "live_edge_settled"
    ];

    for (const trigger of triggers) {
      expect(
        evaluateTimelineBackfill(
          snapshot({
            trigger,
            automaticBackfillEligible: false,
            projectedContentHeight: 0,
            scrollHeight: 0,
            itemCount: 0
          })
        ),
        trigger
      ).toEqual({
        kind: "blocked",
        demand: "underfilled",
        reason: "semantic_ineligible"
      });
    }

    expect(
      evaluateTimelineBackfill(
        snapshot({
          automaticBackfillEligible: false,
          genuineUserScroll: true,
          scrollTop: 0
        })
      )
    ).toEqual({
      kind: "blocked",
      demand: "explicit_top_scroll",
      reason: "semantic_ineligible"
    });
  });

  test("evaluates demand and blockers without event-order state", () => {
    const blockerCases: Array<{
      reason: TimelineBackfillBlocker;
      overrides: Partial<TimelineBackfillSnapshot>;
    }> = [
      { reason: "not_initialized", overrides: { initialized: false } },
      { reason: "awaiting_resync", overrides: { awaitingResync: true } },
      { reason: "suppressed_ui", overrides: { suppressPaginationUi: true } },
      { reason: "projection_unsettled", overrides: { projectionSettled: false } },
      {
        reason: "virtual_layout_unsettled",
        overrides: { virtualLayoutSettled: false }
      },
      { reason: "anchor_unsettled", overrides: { anchorSettled: false } },
      { reason: "request_in_flight", overrides: { requestInFlight: true } },
      {
        reason: "retry_waiting_for_transition",
        overrides: { retryBlocked: true }
      },
      {
        reason: "pagination_paginating",
        overrides: { paginationState: "Paginating" }
      },
      {
        reason: "pagination_end_reached",
        overrides: { paginationState: "EndReached" }
      }
    ];

    for (const blocker of blockerCases) {
      expect(
        evaluateTimelineBackfill(
          snapshot({
            projectedContentHeight: 300,
            scrollHeight: 300,
            ...blocker.overrides
          })
        ),
        blocker.reason
      ).toEqual({
        kind: "blocked",
        demand: "underfilled",
        reason: blocker.reason
      });
    }

    expect(
      evaluateTimelineBackfill(
        snapshot({ projectedContentHeight: 300, scrollHeight: 300 })
      )
    ).toEqual({ kind: "request", demand: "underfilled" });

    expect(evaluateTimelineBackfill(snapshot({ scrollTop: 80 }))).toEqual({
      kind: "request",
      demand: "near_top_prefetch"
    });

    expect(
      evaluateTimelineBackfill(
        snapshot({ clientHeight: 0, genuineUserScroll: true, scrollTop: 0 })
      )
    ).toEqual({ kind: "request", demand: "explicit_top_scroll" });

    expect(
      evaluateTimelineBackfill(snapshot({ clientHeight: 0 }))
    ).toEqual({ kind: "idle", reason: "viewport_unmeasured" });

    expect(evaluateTimelineBackfill(snapshot())).toEqual({
      kind: "idle",
      reason: "no_demand"
    });

    const failedState: PaginationState = { Failed: { kind: "Network" } };
    expect(
      evaluateTimelineBackfill(
        snapshot({
          paginationState: failedState,
          projectedContentHeight: 300,
          scrollHeight: 300
        })
      )
    ).toEqual({ kind: "request", demand: "underfilled" });
  });

  test("does not let projected height hide a DOM underfill", () => {
    expect(
      evaluateTimelineBackfill(
        snapshot({
          projectedContentHeight: 0,
          scrollHeight: 320,
          clientHeight: 600
        })
      )
    ).toEqual({ kind: "request", demand: "underfilled" });
  });

  test("does not treat a transient virtual DOM window as underfilled", () => {
    expect(
      evaluateTimelineBackfill(
        snapshot({
          projectedContentHeight: 240_000,
          scrollHeight: 367,
          clientHeight: 367,
          scrollTop: 200
        })
      )
    ).toEqual({ kind: "idle", reason: "no_demand" });
  });

  test("prefers explicit user top-scroll over automatic demand", () => {
    expect(
      evaluateTimelineBackfill(
        snapshot({
          genuineUserScroll: true,
          scrollTop: 0,
          projectedContentHeight: 300,
          scrollHeight: 300
        })
      )
    ).toEqual({ kind: "request", demand: "explicit_top_scroll" });
  });

  test("backfill eligibility is independent of settlement event order", () => {
    const orders: Array<Array<"projection" | "virtual" | "anchor" | "terminal">> = [
      ["terminal", "projection", "virtual", "anchor"],
      ["projection", "terminal", "anchor", "virtual"],
      ["anchor", "virtual", "terminal", "projection"],
      ["virtual", "anchor", "projection", "terminal"]
    ];

    for (const order of orders) {
      const current = snapshot({
        projectedContentHeight: 300,
        scrollHeight: 300,
        projectionSettled: false,
        virtualLayoutSettled: false,
        anchorSettled: false,
        requestInFlight: true
      });

      order.forEach((transition, index) => {
        if (transition === "projection") current.projectionSettled = true;
        if (transition === "virtual") current.virtualLayoutSettled = true;
        if (transition === "anchor") current.anchorSettled = true;
        if (transition === "terminal") current.requestInFlight = false;

        const evaluation = evaluateTimelineBackfill(current);
        if (index < order.length - 1) {
          expect(evaluation.kind, order.join(" → ")).toBe("blocked");
        } else {
          expect(evaluation, order.join(" → ")).toEqual({
            kind: "request",
            demand: "underfilled"
          });
        }
      });
    }
  });
});

describe("clearsExternalTransitionFence (#1473)", () => {
  test("self-induced and geometry settlements keep the fence", () => {
    for (const trigger of [
      "pagination_terminal",
      "layout_settled",
      "room_anchor_settled",
      "live_edge_settled"
    ] as const) {
      expect(clearsExternalTransitionFence(trigger, false)).toBe(false);
      expect(clearsExternalTransitionFence(trigger, true)).toBe(false);
    }
  });

  test("only a genuine user scroll clears the fence", () => {
    expect(clearsExternalTransitionFence("user_scroll", false)).toBe(false);
    expect(clearsExternalTransitionFence("user_scroll", true)).toBe(true);
  });

  test("real projection and setting transitions clear the fence", () => {
    for (const trigger of [
      "initial_projection",
      "prepend_settled",
      "gap_projection_changed",
      "gap_repair_released",
      "resync_replayed",
      "timeline_reset",
      "retry_backoff_elapsed"
    ] as const) {
      expect(clearsExternalTransitionFence(trigger, false)).toBe(true);
    }
  });

  test("automatic retries back off and stop after a bounded budget", () => {
    const delays: number[] = [];
    for (let attempt = 0; ; attempt += 1) {
      const delay = externalTransitionAutoRetryDelayMs(attempt);
      if (delay === null) break;
      delays.push(delay);
    }
    expect(delays).toEqual([...EXTERNAL_TRANSITION_AUTO_RETRY_DELAYS_MS]);
    expect(delays.length).toBeGreaterThan(0);
    expect(delays.length).toBeLessThanOrEqual(5);
    for (let i = 1; i < delays.length; i += 1) {
      expect(delays[i]).toBeGreaterThan(delays[i - 1]!);
    }
  });
});

describe("shouldRequestTimelineForwardFill (#1668)", () => {
  function forward(
    overrides: Partial<TimelineForwardFillSnapshot> = {}
  ): TimelineForwardFillSnapshot {
    return {
      initialized: true,
      awaitingResync: false,
      paginationState: "Idle",
      requestInFlight: false,
      projectionSettled: true,
      genuineUserScroll: false,
      clientHeight: 600,
      scrollHeight: 3_000,
      scrollTop: 2_400,
      nearBottomThreshold: 600,
      ...overrides
    };
  }

  test("requests newer history at the loaded bottom and when underfilled", () => {
    expect(shouldRequestTimelineForwardFill(forward())).toBe(true);
    expect(shouldRequestTimelineForwardFill(forward({ scrollTop: 1_900 }))).toBe(true);
    expect(
      shouldRequestTimelineForwardFill(forward({ scrollHeight: 400, scrollTop: 0 }))
    ).toBe(true);
  });

  test("waits while the reader is far above the loaded bottom", () => {
    expect(shouldRequestTimelineForwardFill(forward({ scrollTop: 1_000 }))).toBe(false);
  });

  test.each<[string, Partial<TimelineForwardFillSnapshot>]>([
    ["uninitialized", { initialized: false }],
    ["awaiting resync", { awaitingResync: true }],
    ["in flight", { requestInFlight: true }],
    ["unsettled projection", { projectionSettled: false }],
    ["paginating", { paginationState: "Paginating" }],
    ["at the live end", { paginationState: "EndReached" }],
    ["unmeasured", { clientHeight: 0 }]
  ])("is blocked when %s", (_label, overrides) => {
    expect(shouldRequestTimelineForwardFill(forward(overrides))).toBe(false);
  });

  test("retries a failed page only on a genuine user scroll", () => {
    const failed: PaginationState = { Failed: { kind: "Sdk" } };
    expect(shouldRequestTimelineForwardFill(forward({ paginationState: failed }))).toBe(false);
    expect(
      shouldRequestTimelineForwardFill(
        forward({ paginationState: failed, genuineUserScroll: true })
      )
    ).toBe(true);
  });
});
