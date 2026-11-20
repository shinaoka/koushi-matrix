import { describe, expect, it } from "vitest";

import {
  TOUCHPAD_SCROLL,
  TouchpadScrollResampler,
  touchpadScrollResamplingEnabled
} from "./touchpadScrollResampler";

const FRAME_MS = 1000 / 60;

type Run = { steps: number[]; consumed: number; native: number; inputSum: number };

/**
 * Drive a gesture: `deltaAt(i)` is the i-th WebKitGTK touchpad delta,
 * delivered every `intervalMs` (the measured touchpad cadence is 4 ms), with
 * frames every 16.7 ms. Frames continue after the input until the resampler
 * goes idle or `tailMs` passes.
 */
function drive(
  resampler: TouchpadScrollResampler,
  events: number,
  deltaAt: (index: number) => number,
  { intervalMs = 4, startMs = 1000, tailMs = 3000 } = {}
): Run {
  const run: Run = { steps: [], consumed: 0, native: 0, inputSum: 0 };
  const inputEnd = startMs + events * intervalMs;
  let nextEvent = 0;
  let frameAt = startMs + FRAME_MS;
  while (frameAt < inputEnd + tailMs) {
    while (nextEvent < events && startMs + nextEvent * intervalMs <= frameAt) {
      const deltaY = deltaAt(nextEvent);
      run.inputSum += deltaY;
      if (resampler.wheel(deltaY, startMs + nextEvent * intervalMs) === "resample") run.consumed += 1;
      else run.native += 1;
      nextEvent += 1;
    }
    const frame = resampler.frame(frameAt);
    run.steps.push(frame.step);
    if (!frame.active && nextEvent >= events) break;
    frameAt += FRAME_MS;
  }
  return run;
}

// A slow swipe as recorded from WebKitGTK: fractional deltas every 4 ms that
// wobble between ~5 and ~6 px and ease off at the end.
function slowSwipeDelta(index: number): number {
  const ease = index > 80 ? Math.max(0.05, 1 - (index - 80) / 20) : 1;
  return -(5.3 + 0.6 * Math.sin(index * 1.7)) * ease;
}

describe("TouchpadScrollResampler", () => {
  it("moves an even amount per frame while the fingers move (no 1 px / 11 px frames)", () => {
    const resampler = new TouchpadScrollResampler();
    // 250 Hz input with timing jitter: 4 or 5 events land in each frame.
    const run = drive(resampler, 90, slowSwipeDelta);
    const steady = run.steps.slice(3, 18).map(Math.abs);
    expect(Math.min(...steady)).toBeGreaterThan(0);
    expect(Math.max(...steady) / Math.min(...steady)).toBeLessThan(1.3);
  });

  it("scales a swipe to Chromium's distance and stops with the fingers", () => {
    const resampler = new TouchpadScrollResampler();
    const run = drive(resampler, 100, slowSwipeDelta);
    const total = run.steps.reduce((sum, step) => sum + step, 0);
    // Only the first event goes native (gesture detection); the rest is scaled.
    expect(run.native).toBe(1);
    const resampledInput = run.inputSum - slowSwipeDelta(0);
    expect(total).toBeCloseTo(resampledInput * TOUCHPAD_SCROLL.speed, 0);
    // A slow release does not glide: Chromium did not fling these.
    expect(resampler.active).toBe(false);
  });

  it("flings after a quick flick with Chromium's decay, then stops", () => {
    const resampler = new TouchpadScrollResampler();
    // A flick as recorded: ~12 fractional deltas ramping to ~-50 px in 4 ms steps.
    const run = drive(resampler, 12, (index) => -(15 + index * 3.5) - 0.3);
    const releaseFrame = run.steps.findIndex((_, index) => index > 2 && run.steps[index + 1] !== undefined && Math.abs(run.steps[index + 1]) < Math.abs(run.steps[index]));
    const glide = run.steps.slice(releaseFrame + 2).filter((step) => step !== 0);
    expect(glide.length).toBeGreaterThan(30);
    expect(glide.every((step) => step < 0)).toBe(true);
    // Per-frame decay early in the glide: Element measured 0.80-0.94.
    for (let index = 1; index < 10; index += 1) {
      const ratio = glide[index] / glide[index - 1];
      expect(ratio).toBeGreaterThan(0.75);
      expect(ratio).toBeLessThan(0.97);
    }
    expect(resampler.active).toBe(false);
  });

  it("starts the glide at about a third of the release speed", () => {
    const resampler = new TouchpadScrollResampler();
    const deltas = Array.from({ length: 12 }, () => -40.5);
    const run = drive(resampler, deltas.length, (index) => deltas[index]);
    // Release velocity: 40.5 px per 4 ms, scaled.
    const releasePerFrame = (40.5 / 4) * TOUCHPAD_SCROLL.speed * FRAME_MS;
    const glideStart = Math.max(...run.steps.slice(-200).map((step) => -step).filter((step) => step < releasePerFrame * 0.5));
    expect(glideStart).toBeGreaterThan(releasePerFrame * TOUCHPAD_SCROLL.flingVelocityScale * 0.8);
    expect(glideStart).toBeLessThan(releasePerFrame * TOUCHPAD_SCROLL.flingVelocityScale * 1.2);
  });

  it("leaves whole-pixel wheel input (mouse notches, test drivers) to the browser", () => {
    const resampler = new TouchpadScrollResampler();
    const run = drive(resampler, 10, () => -120, { intervalMs: 4 });
    expect(run.consumed).toBe(0);
    expect(run.steps.every((step) => step === 0)).toBe(true);
    expect(resampler.wheel(-2, 5000)).toBe("native");
    expect(resampler.wheel(-20, 5004)).toBe("native");
  });

  it("needs a second fractional delta at touchpad cadence to start", () => {
    const resampler = new TouchpadScrollResampler();
    expect(resampler.wheel(-3.4, 1000)).toBe("native");
    // Too slow for a touchpad stream: still native.
    expect(resampler.wheel(-3.4, 1100)).toBe("native");
    expect(resampler.wheel(-3.4, 1104)).toBe("resample");
  });

  it("takes a fractional delta soon after a touchpad gesture as the touchpad", () => {
    const resampler = new TouchpadScrollResampler();
    drive(resampler, 100, slowSwipeDelta);
    expect(resampler.active).toBe(false);
    expect(resampler.wheel(-2.7, 1000 + 100 * 4 + 300)).toBe("resample");
  });

  it("stops a glide on a reverse swipe and boosts it on a same-direction flick", () => {
    const flick = (index: number) => -(15 + index * 3.5) - 0.3;
    const reverse = new TouchpadScrollResampler();
    drive(reverse, 12, flick, { tailMs: 150 });
    expect(reverse.active).toBe(true);
    reverse.wheel(3.3, 2000);
    const afterReverse = reverse.frame(2016.7);
    expect(afterReverse.step).toBeGreaterThanOrEqual(0);

    const single = new TouchpadScrollResampler();
    const one = drive(single, 12, flick, { tailMs: 3000 });
    const boosted = new TouchpadScrollResampler();
    drive(boosted, 12, flick, { tailMs: 150 });
    const two = drive(boosted, 12, flick, { startMs: 1300, tailMs: 3000 });
    const glideDistance = (run: Run) => -run.steps.reduce((sum, step) => sum + step, 0);
    expect(glideDistance(two)).toBeGreaterThan(glideDistance(one) * 1.1);
  });

  it("stops gliding at an edge and on cancel", () => {
    const flick = (index: number) => -(15 + index * 3.5) - 0.3;
    const edge = new TouchpadScrollResampler();
    drive(edge, 12, flick, { tailMs: 120 });
    expect(edge.active).toBe(true);
    edge.stopFling();
    expect(edge.active).toBe(false);

    const cancelled = new TouchpadScrollResampler();
    drive(cancelled, 12, flick, { tailMs: 120 });
    cancelled.cancel();
    expect(cancelled.active).toBe(false);
    expect(cancelled.frame(5000).step).toBe(0);
  });
});

describe("touchpadScrollResamplingEnabled", () => {
  const webkitGtk =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/60.5 Safari/605.1.15";
  const chromiumLinux =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
  const macWebKit =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)";

  it("is on for Linux WebKitGTK only", () => {
    expect(touchpadScrollResamplingEnabled("linux", webkitGtk)).toBe(true);
    expect(touchpadScrollResamplingEnabled("linux", chromiumLinux)).toBe(false);
    expect(touchpadScrollResamplingEnabled("macos", macWebKit)).toBe(false);
    expect(touchpadScrollResamplingEnabled("windows", chromiumLinux)).toBe(false);
    expect(touchpadScrollResamplingEnabled(undefined, webkitGtk)).toBe(false);
  });
});
