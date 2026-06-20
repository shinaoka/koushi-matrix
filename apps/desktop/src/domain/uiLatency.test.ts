import { describe, expect, test } from "vitest";

import { createUiLatencySampler } from "./uiLatency";

describe("createUiLatencySampler", () => {
  test("summarizes requestAnimationFrame gaps without storing private UI data", () => {
    const sampler = createUiLatencySampler({ longFrameMs: 50 });

    expect(sampler.recordFrame(16)).toEqual({
      samples: 1,
      lastFrameGapMs: 16,
      averageFrameGapMs: 16,
      maxFrameGapMs: 16,
      longFrameCount: 0
    });
    expect(sampler.recordFrame(75)).toEqual({
      samples: 2,
      lastFrameGapMs: 75,
      averageFrameGapMs: 45.5,
      maxFrameGapMs: 75,
      longFrameCount: 1
    });
  });

  test("ignores invalid frame gaps", () => {
    const sampler = createUiLatencySampler();
    sampler.recordFrame(20);

    expect(sampler.recordFrame(Number.NaN)).toEqual({
      samples: 1,
      lastFrameGapMs: 20,
      averageFrameGapMs: 20,
      maxFrameGapMs: 20,
      longFrameCount: 0
    });
    expect(sampler.recordFrame(-1)).toEqual({
      samples: 1,
      lastFrameGapMs: 20,
      averageFrameGapMs: 20,
      maxFrameGapMs: 20,
      longFrameCount: 0
    });
  });
});
