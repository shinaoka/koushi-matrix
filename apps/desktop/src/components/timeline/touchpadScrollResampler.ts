import type { DisplayPlatform } from "../../domain/types";

/**
 * Linux touchpad scrolling for the timeline (WebKitGTK only).
 *
 * WebKitGTK hands the page each touchpad delta as it arrives, at the
 * touchpad's own rate (about every 4 ms), with no frame alignment, no scaling
 * and no Chromium-style fling. Frames then move unevenly (1 px, then 11 px),
 * which reads as stutter and smearing, and a full swipe travels far less than
 * in Chromium. Chromium (Electron, WebView2) and macOS WKWebView already
 * deliver frame-aligned, scaled input, so this applies to Linux WebKitGTK only.
 * Upstream: https://bugs.webkit.org/show_bug.cgi?id=332612 (GTK3 touchpad
 * deltas use a fixed 40 px step). Revisit these constants when it is fixed.
 *
 * The constants were fitted on 2026-11-20 against Element 1.12.25 (Electron
 * 43, Ozone Wayland) and Koushi 0.36.2 (WebKitGTK 2.54.3) on one laptop
 * touchpad under KDE Wayland, recording the same gestures in both:
 * - a full-height slow swipe travelled about 2.4x further in Chromium;
 * - Chromium delivered one wheel event per frame (16.4 ms);
 * - Chromium did not fling after slow swipes (release velocity 0.04-0.14 px/ms)
 *   and, after a flick, its per-frame delta followed d_k = d0 / (1 + 0.21 k),
 *   with d0 about a third of the release velocity.
 */
export const TOUCHPAD_SCROLL = {
  /** Page pixels per WebKitGTK touchpad pixel. */
  speed: 2.4,
  /** The input curve is sampled this far behind the newest event. */
  resampleDelayMs: 20,
  /** No input for this long ends the gesture (fingers lifted). */
  gestureIdleMs: 40,
  /** Two fractional deltas closer than this start a touchpad gesture. */
  touchpadCadenceMs: 12,
  /** A fractional delta this soon after a touchpad gesture is the touchpad again. */
  touchpadMemoryMs: 1000,
  /** Scaled release velocity needed to fling. */
  flingMinVelocityPxPerMs: 5,
  /** Fling start velocity as a fraction of the release velocity. */
  flingVelocityScale: 0.33,
  /** v(t) = v0 / (1 + flingDecayPerMs * t): 0.21 per 16.7 ms frame. */
  flingDecayPerMs: 0.21 / 16.7,
  /** The fling fades to zero by this time. */
  flingMaxMs: 2000,
  flingStopPxPerMs: 0.03,
  /** Velocity window over the newest input. */
  velocityWindowMs: 50
} as const;

/** Whether this webview needs touchpad resampling: Linux WebKitGTK only. */
export function touchpadScrollResamplingEnabled(
  platform: DisplayPlatform | undefined,
  userAgent: string
): boolean {
  if (platform !== "linux") return false;
  return /AppleWebKit\//.test(userAgent) && /Linux/.test(userAgent) && !/(Chrome|Chromium|Edg)\//.test(userAgent);
}

/**
 * Whether a scrollable element between the wheel target and the timeline can
 * still scroll in this direction, so the wheel is its own and not the
 * timeline's.
 */
export function nestedScrollerTakesWheel(target: EventTarget | null, container: Element, deltaY: number): boolean {
  for (let element = target instanceof Element ? target : null; element && element !== container; element = element.parentElement) {
    if (element.scrollHeight <= element.clientHeight + 1) continue;
    const overflowY = getComputedStyle(element).overflowY;
    if (overflowY !== "auto" && overflowY !== "scroll") continue;
    const canMove =
      deltaY < 0 ? element.scrollTop > 0 : element.scrollTop + element.clientHeight < element.scrollHeight - 1;
    if (canMove) return true;
  }
  return false;
}

type Sample = { t: number; p: number };

export type TouchpadWheelDecision = "resample" | "native";

export type TouchpadScrollFrame = {
  /** Page pixels to scroll this frame (positive = down). */
  step: number;
  /** Whether another frame is needed. */
  active: boolean;
};

function isFractional(value: number): boolean {
  return Math.abs(value - Math.round(value)) > 1e-6;
}

/**
 * Turns WebKitGTK touchpad wheel deltas into evenly spaced per-frame scroll
 * steps, scaled and followed by a fling, to match Chromium. Pure: the caller
 * supplies times, applies the steps and reports when a step could not move.
 */
export class TouchpadScrollResampler {
  private samples: Sample[] = [];
  private inputPos = 0;
  private applied = 0;
  private lastInputT = Number.NEGATIVE_INFINITY;
  private lastTouchpadT = Number.NEGATIVE_INFINITY;
  private lastFrameT: number | null = null;
  private pendingNative: { t: number; deltaY: number } | null = null;
  private fling: { v0: number; t: number; v: number } | null = null;
  private carryVelocity = 0;

  get active(): boolean {
    return this.samples.length > 0 || this.fling !== null;
  }

  /**
   * Offer one vertical pixel-mode wheel delta. "resample" means the caller
   * must prevent the default scroll; this resampler now owns the movement.
   */
  wheel(deltaY: number, now: number): TouchpadWheelDecision {
    if (deltaY === 0) return this.samples.length > 0 ? "resample" : "native";
    if (this.samples.length === 0) {
      if (!this.startsTouchpadGesture(deltaY, now)) {
        this.pendingNative = { t: now, deltaY };
        return "native";
      }
      this.beginGesture(deltaY, now);
    }
    this.pendingNative = null;
    this.inputPos += deltaY * TOUCHPAD_SCROLL.speed;
    this.lastInputT = now;
    this.lastTouchpadT = now;
    this.samples.push({ t: now, p: this.inputPos });
    return "resample";
  }

  frame(now: number): TouchpadScrollFrame {
    const dt = Math.min(50, Math.max(1, this.lastFrameT === null ? 16.7 : now - this.lastFrameT));
    this.lastFrameT = now;
    if (this.samples.length > 0 && now - this.lastInputT < TOUCHPAD_SCROLL.gestureIdleMs) {
      return { step: this.trackingStep(now), active: true };
    }
    let step = 0;
    if (this.samples.length > 0) step += this.release();
    if (this.fling) step += this.flingStep(dt);
    if (!this.active) this.lastFrameT = null;
    return { step, active: this.active };
  }

  /** The last step could not move the page (an edge): stop gliding. */
  stopFling(): void {
    this.fling = null;
    if (this.samples.length === 0) this.lastFrameT = null;
  }

  /** A programmatic viewport write or other input takes over. */
  cancel(): void {
    this.samples = [];
    this.fling = null;
    this.carryVelocity = 0;
    this.pendingNative = null;
    this.lastFrameT = null;
  }

  private startsTouchpadGesture(deltaY: number, now: number): boolean {
    if (!isFractional(deltaY)) return false;
    if (this.fling || now - this.lastTouchpadT < TOUCHPAD_SCROLL.touchpadMemoryMs) return true;
    const previous = this.pendingNative;
    return (
      previous !== null &&
      now - previous.t <= TOUCHPAD_SCROLL.touchpadCadenceMs &&
      Math.sign(previous.deltaY) === Math.sign(deltaY)
    );
  }

  private beginGesture(deltaY: number, now: number): void {
    // Touching down during a fling stops it; a flick in the same direction
    // keeps its speed for the next fling, as Chromium's fling boost does.
    const flingV = this.fling?.v ?? 0;
    this.carryVelocity = this.fling && Math.sign(flingV) === Math.sign(deltaY) ? flingV : 0;
    this.fling = null;
    this.inputPos = 0;
    this.applied = 0;
    this.samples = [{ t: now - 4, p: 0 }];
  }

  private inputVelocity(): number {
    const last = this.samples.at(-1);
    if (!last || this.samples.length < 2) return 0;
    const first =
      this.samples.find((sample) => sample.t >= last.t - TOUCHPAD_SCROLL.velocityWindowMs) ?? this.samples[0];
    return last.t > first.t ? (last.p - first.p) / (last.t - first.t) : 0;
  }

  private positionAt(t: number): number {
    for (let index = this.samples.length - 1; index >= 0; index -= 1) {
      const sample = this.samples[index];
      if (t < sample.t) continue;
      const next = this.samples[index + 1];
      return next ? sample.p + (next.p - sample.p) * ((t - sample.t) / (next.t - sample.t)) : sample.p;
    }
    return this.samples[0]?.p ?? this.inputPos;
  }

  private trackingStep(now: number): number {
    // Sample the input curve a little in the past so every frame interpolates
    // between real events; no extrapolation, so the page never runs ahead of
    // the fingers and never has to step back.
    const t = now - TOUCHPAD_SCROLL.resampleDelayMs;
    const step = this.positionAt(t) - this.applied;
    this.applied += step;
    this.samples = this.samples.filter((sample) => sample.t > t - 200);
    return step;
  }

  private release(): number {
    const velocity = this.inputVelocity();
    const step = this.inputPos - this.applied;
    this.applied = this.inputPos;
    this.samples = [];
    const carry = Math.sign(this.carryVelocity) === Math.sign(velocity) ? this.carryVelocity : 0;
    this.carryVelocity = 0;
    if (Math.abs(velocity) >= TOUCHPAD_SCROLL.flingMinVelocityPxPerMs) {
      const v0 = velocity * TOUCHPAD_SCROLL.flingVelocityScale + carry;
      this.fling = { v0, t: 0, v: v0 };
    }
    return step;
  }

  private flingStep(dt: number): number {
    const fling = this.fling!;
    fling.t += dt;
    const fade = Math.max(0, 1 - fling.t / TOUCHPAD_SCROLL.flingMaxMs);
    fling.v = (fling.v0 / (1 + TOUCHPAD_SCROLL.flingDecayPerMs * fling.t)) * fade * fade;
    if (Math.abs(fling.v) < TOUCHPAD_SCROLL.flingStopPxPerMs) {
      this.fling = null;
      return 0;
    }
    return fling.v * dt;
  }
}
