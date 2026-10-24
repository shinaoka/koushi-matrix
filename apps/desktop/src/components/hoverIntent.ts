/**
 * Shared presentation-only hover-intent gate (#1603).
 *
 * A scrollport can move content under a stationary pointer. The browser then
 * delivers `mouseenter`/`:hover` to the rows that slide beneath it, even
 * though the reader never aimed at them. A timeline row's action bar and the
 * explanation bubble are pointer-owned chrome, so they must wait for both:
 *
 * 1. scrolling to stop, and
 * 2. the pointer to move after that scroll began.
 *
 * Until both hold, hover chrome stays suppressed. Keyboard focus is not part
 * of this gate: `:focus-within` still reveals the action bar immediately.
 *
 * This is renderer presentation state, not Matrix state. A scroll owner calls
 * `beginHoverScroll`/`endHoverScroll`; pointer movement is tracked here. The
 * module deliberately holds no timers: the owner's existing scroll-idle flush
 * decides when scrolling has stopped.
 */

export type HoverIntentSnapshot = {
  /** Content is moving because the owning scrollport is being scrolled. */
  scrolling: boolean;
  /** The pointer moved after the current scroll gesture began. */
  pointerMovedSinceScroll: boolean;
  /** Increments once per scroll gesture so open bubbles can dismiss. */
  scrollGeneration: number;
};

const INITIAL_SNAPSHOT: HoverIntentSnapshot = {
  scrolling: false,
  pointerMovedSinceScroll: true,
  scrollGeneration: 0
};

let snapshot: HoverIntentSnapshot = INITIAL_SNAPSHOT;
const listeners = new Set<() => void>();
let pointerTrackingInstalled = false;
let lastPointerX: number | null = null;
let lastPointerY: number | null = null;

function emit(next: HoverIntentSnapshot): void {
  if (next === snapshot) {
    return;
  }
  snapshot = next;
  for (const listener of listeners) {
    listener();
  }
}

function onPointerMove(event: PointerEvent): void {
  // Some engines synthesize a pointermove while content scrolls under a
  // stationary pointer. Only a position change counts as real intent.
  const moved =
    lastPointerX === null ||
    lastPointerY === null ||
    event.clientX !== lastPointerX ||
    event.clientY !== lastPointerY;
  lastPointerX = event.clientX;
  lastPointerY = event.clientY;
  if (!moved || snapshot.pointerMovedSinceScroll) {
    return;
  }
  emit({ ...snapshot, pointerMovedSinceScroll: true });
}

function installPointerTracking(): void {
  if (pointerTrackingInstalled || typeof window === "undefined") {
    return;
  }
  pointerTrackingInstalled = true;
  // Capture phase: the gate must already be open when a trigger's own
  // pointermove handler runs for the reader's first real movement.
  window.addEventListener("pointermove", onPointerMove, { capture: true, passive: true });
}

function uninstallPointerTracking(): void {
  if (!pointerTrackingInstalled || typeof window === "undefined") {
    return;
  }
  pointerTrackingInstalled = false;
  window.removeEventListener("pointermove", onPointerMove, { capture: true });
}

export function beginHoverScroll(): void {
  if (snapshot.scrolling) {
    return;
  }
  emit({
    scrolling: true,
    pointerMovedSinceScroll: false,
    scrollGeneration: snapshot.scrollGeneration + 1
  });
}

export function endHoverScroll(): void {
  if (!snapshot.scrolling) {
    return;
  }
  emit({ ...snapshot, scrolling: false });
}

/** True only when a hover began from real pointer intent, not moving content. */
export function hoverIntentAllowsPointerHover(): boolean {
  return !snapshot.scrolling && snapshot.pointerMovedSinceScroll;
}

export function subscribeHoverIntent(listener: () => void): () => void {
  listeners.add(listener);
  installPointerTracking();
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) {
      uninstallPointerTracking();
    }
  };
}

export function getHoverIntentSnapshot(): HoverIntentSnapshot {
  return snapshot;
}

/** Test-only reset; production state lasts for the mounted presentation. */
export function resetHoverIntentForTests(): void {
  uninstallPointerTracking();
  lastPointerX = null;
  lastPointerY = null;
  snapshot = INITIAL_SNAPSHOT;
  for (const listener of listeners) {
    listener();
  }
}
