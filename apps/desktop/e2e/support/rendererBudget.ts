/**
 * Count-based renderer performance budget for the headless DOM tier (#1895).
 *
 * A frame time is a machine fact, not an app fact, so this harness counts the
 * work the app *does* and lets the machine decide how long it takes:
 *
 * - forced layout reads (wrapped geometry getters and measurements),
 * - React commits (devtools hook),
 * - `requestAnimationFrame` registrations,
 * - live window/document listeners and live observers,
 * - mounted rows and the mounted timeline subtree.
 *
 * Counts are structural, but they are not independent of batching: an extra
 * frame or a `ResizeObserver` delivery inside the measurement window can move
 * them. Every scenario therefore brackets its action with `reset()` and
 * `settle()` (quiescence, not a fixed delay), and every budget is a ceiling.
 * The exceptions are the zero targets: an idle app must not commit, and a
 * closed popover must not read layout at all (#1892).
 *
 * Attribution is the whole point of the counts: a budget without a caller is a
 * number nobody can act on. Each read is bucketed by its first two frames under
 * `/src/` (`nativeTitlebarSafeTop<-useFloatingPlacement`), skipping frames under
 * `/src/test/`, so the failing line names the code that measured. Playwright's
 * own injected script and a spec's `page.evaluate` never sit under `/src/`, so
 * they never raise the app's count.
 *
 * Nothing here changes the app: the wrappers always return the original value,
 * keep `this`, and pass getters' setters through, and only this spec's pages
 * install them.
 */

import { expect, type Page } from "@playwright/test";

/** One caller chain: the two innermost app frames, deepest first. */
export type RendererBudgetBucket = { key: string; count: number };

export type RendererBudgetRead = {
  commits: number;
  layoutReads: number;
  /** Reads whose innermost app frame lives in `floatingLayer.tsx`. */
  floatingLayerReads: number;
  rafRegistrations: number;
  layoutReadBuckets: RendererBudgetBucket[];
  /** Callers of the counted rAF registrations, deepest app frame first. */
  rafBuckets: RendererBudgetBucket[];
  /** `window`/`document` listener sets, so two reads can be compared. */
  listenerSignature: string;
  observerTargets: number;
  rows: number;
  timelineSubtree: number;
};

type RendererBudgetRuntime = {
  reset(): void;
  read(): RendererBudgetRead;
  settle(options?: {
    quietFrames?: number;
    debounceMs?: number;
    timeoutMs?: number;
  }): Promise<void>;
};

declare global {
  interface Window {
    __rendererBudget: RendererBudgetRuntime;
  }
}

/**
 * Install the counters before any app module runs, so the React devtools hook
 * exists when React asks for it and no early layout read escapes.
 */
export async function installRendererBudget(page: Page): Promise<void> {
  await page.addInitScript(initRendererBudget);
}

export async function resetRendererBudget(page: Page): Promise<void> {
  await page.evaluate(() => window.__rendererBudget.reset());
}

/** Wait for quiescence: counters unchanged for ten frames, capped at 5 s. */
export async function settleRendererBudget(
  page: Page,
  options?: { quietFrames?: number; debounceMs?: number; timeoutMs?: number }
): Promise<void> {
  await page.evaluate((settleOptions) => window.__rendererBudget.settle(settleOptions), options);
}

export async function readRendererBudget(page: Page): Promise<RendererBudgetRead> {
  return page.evaluate(() => window.__rendererBudget.read());
}

/**
 * The failure line a breach prints: the measured count, the budget, and the
 * five callers that produced it. Keep this shape stable — the qa-lanes note
 * tells a reader to start from it.
 */
export function formatBreach(
  name: string,
  measured: number,
  budget: number,
  top: readonly RendererBudgetBucket[] = []
): string {
  const callers =
    top
      .slice(0, 5)
      .map((bucket) => `${bucket.key} (${bucket.count})`)
      .join(", ") || "none";
  return `renderer budget breach: ${name} measured ${measured}, budget ${budget}, over by ${measured - budget}; top callers: ${callers}`;
}

export function expectWithinBudget(
  name: string,
  measured: number,
  budget: number,
  top: readonly RendererBudgetBucket[] = []
): void {
  expect(measured, formatBreach(name, measured, budget, top)).toBeLessThanOrEqual(budget);
}

function counterValue(read: RendererBudgetRead, name: string): number {
  switch (name) {
    case "commits":
      return read.commits;
    case "layoutReads":
      return read.layoutReads;
    case "rows":
      return read.rows;
    case "timelineSubtree":
      return read.timelineSubtree;
    default:
      throw new Error(`renderer budget: unknown counter ${name}`);
  }
}

/**
 * Assert one read against a scenario's ceilings. Each breach names its counter,
 * so a failing run says which structural cost grew.
 */
export async function expectBudget(
  page: Page,
  scenario: string,
  budgets: Readonly<Record<string, number>>
): Promise<void> {
  const read = await readRendererBudget(page);
  for (const [name, budget] of Object.entries(budgets)) {
    expectWithinBudget(
      `${scenario} ${name}`,
      counterValue(read, name),
      budget,
      read.layoutReadBuckets
    );
  }
}

/**
 * The in-page runtime. Playwright serialises this function into the page, so it
 * must stay self-contained: no imports, no outer bindings.
 */
function initRendererBudget(): void {
  /** The diagnostics sampler runs one rAF per frame on purpose (#969 plumbing). */
  const samplerFile = "useUiLatencyDiagnostics.ts";
  /** Vite dev serves app modules under `/src/`; the harness itself sits in `/src/test/`. */
  const appFramePrefix = "/src/";
  const testFramePrefix = "/src/test/";
  const self = window as unknown as { __rendererBudget?: RendererBudgetRuntime };
  if (self.__rendererBudget) {
    return;
  }

  const counters = { commits: 0, layoutReads: 0, floatingLayerReads: 0, raf: 0 };
  const buckets = new Map<string, number>();
  const rafBuckets = new Map<string, number>();

  /** `url:line:col` → `file.ts`. The `:line:col` suffix is why this is not a split. */
  const fileName = (location: string): string => {
    const withoutQuery = location.split("?")[0].split("#")[0];
    const withoutPosition = withoutQuery.replace(/:\d+:\d+$/, "");
    const slash = withoutPosition.lastIndexOf("/");
    return slash >= 0 ? withoutPosition.slice(slash + 1) : withoutPosition;
  };

  type Frame = { name: string; file: string };

  /** V8 writes `at fn (url:l:c)`, JSC writes `fn@url:l:c`. */
  function parseFrames(stack: string): Frame[] {
    const frames: Frame[] = [];
    for (const rawLine of stack.split("\n")) {
      const line = rawLine.trim();
      if (!line || line === "Error") {
        continue;
      }
      const withoutAt = line.startsWith("at ") ? line.slice(3) : null;
      let name = "";
      let location = "";
      if (withoutAt !== null) {
        const open = withoutAt.indexOf(" (");
        if (open >= 0 && withoutAt.endsWith(")")) {
          name = withoutAt.slice(0, open);
          location = withoutAt.slice(open + 2, -1);
        } else {
          location = withoutAt;
        }
      } else {
        const at = line.indexOf("@");
        name = at >= 0 ? line.slice(0, at) : "";
        location = at >= 0 ? line.slice(at + 1) : line;
      }
      if (!location.includes(appFramePrefix)) {
        continue;
      }
      if (location.includes(testFramePrefix)) {
        continue;
      }
      const file = fileName(location);
      const cleaned = name
        .replace(/^Object\./, "")
        .replace(/^async /, "")
        .replace(/^new /, "")
        .trim();
      frames.push({ name: cleaned || file, file });
      if (frames.length >= 2) {
        break;
      }
    }
    return frames;
  }

  function appFrames(): Frame[] {
    return parseFrames(new Error().stack ?? "");
  }

  function recordLayoutRead(): void {
    const frames = appFrames();
    if (frames.length === 0) {
      return;
    }
    counters.layoutReads += 1;
    const key =
      frames.length > 1 ? `${frames[0].name}<-${frames[1].name}` : frames[0].name;
    buckets.set(key, (buckets.get(key) ?? 0) + 1);
    if (frames[0].file === "floatingLayer.tsx") {
      counters.floatingLayerReads += 1;
    }
  }

  function wrapGetter(owner: object, key: string): void {
    const descriptor = Object.getOwnPropertyDescriptor(owner, key);
    if (!descriptor?.get || descriptor.configurable !== true) {
      return;
    }
    const originalGet = descriptor.get;
    const originalSet = descriptor.set;
    Object.defineProperty(owner, key, {
      configurable: true,
      enumerable: descriptor.enumerable,
      get(this: unknown) {
        recordLayoutRead();
        return originalGet.call(this);
      },
      ...(originalSet
        ? {
            set(this: unknown, value: unknown) {
              originalSet.call(this, value);
            }
          }
        : {})
    });
  }

  function wrapMethod(owner: object, key: string): void {
    const descriptor = Object.getOwnPropertyDescriptor(owner, key);
    if (!descriptor || typeof descriptor.value !== "function" || descriptor.configurable !== true) {
      return;
    }
    const original = descriptor.value as (...args: unknown[]) => unknown;
    Object.defineProperty(owner, key, {
      configurable: true,
      enumerable: descriptor.enumerable,
      writable: true,
      value(this: unknown, ...args: unknown[]) {
        recordLayoutRead();
        return original.apply(this, args);
      }
    });
  }

  for (const key of [
    "clientTop",
    "clientLeft",
    "clientWidth",
    "clientHeight",
    "scrollTop",
    "scrollLeft",
    "scrollWidth",
    "scrollHeight"
  ]) {
    wrapGetter(Element.prototype, key);
  }
  for (const key of [
    "offsetTop",
    "offsetLeft",
    "offsetWidth",
    "offsetHeight",
    "offsetParent"
  ]) {
    wrapGetter(HTMLElement.prototype, key);
  }
  wrapMethod(Element.prototype, "getBoundingClientRect");
  wrapMethod(Element.prototype, "getClientRects");

  // React reports every commit through the devtools hook it finds at startup.
  // The canary in the spec fails if this protocol ever changes, because a hooks
  // counter stuck at zero would let every budget pass.
  const hook = {
    supportsFiber: true,
    isDisabled: false,
    renderers: new Map<number, unknown>(),
    inject: () => 1,
    checkDCE: () => undefined,
    onCommitFiberRoot: () => {
      counters.commits += 1;
    },
    onPostCommitFiberRoot: () => undefined,
    onCommitFiberUnmount: () => undefined,
    onScheduleFiberRoot: () => undefined,
    onError: () => undefined
  };
  Object.defineProperty(window, "__REACT_DEVTOOLS_GLOBAL_HOOK__", {
    configurable: true,
    writable: false,
    value: hook
  });

  const originalRaf = window.requestAnimationFrame.bind(window);
  window.requestAnimationFrame = function (callback: FrameRequestCallback): number {
    const frames = appFrames();
    // The sampler schedules its next frame from inside itself once per frame;
    // counting that loop would measure the diagnostic, not the app.
    if (frames.length > 0 && frames[0].file !== samplerFile) {
      counters.raf += 1;
      const key =
        frames.length > 1 ? `${frames[0].name}<-${frames[1].name}` : frames[0].name;
      rafBuckets.set(key, (rafBuckets.get(key) ?? 0) + 1);
    }
    return originalRaf(callback);
  };

  // Live `window`/`document` listeners, tracked as sets so two reads compare the
  // set itself rather than a call count.
  const listeners = new Map<string, Set<unknown>>();
  const originalAdd = EventTarget.prototype.addEventListener;
  const originalRemove = EventTarget.prototype.removeEventListener;
  const trackedTarget = (target: unknown): "window" | "document" | null => {
    if (target === window) return "window";
    if (target === document) return "document";
    return null;
  };
  EventTarget.prototype.addEventListener = function (
    this: EventTarget,
    type: string,
    listener: EventListenerOrEventListenerObject | null,
    eventOptions?: boolean | AddEventListenerOptions
  ): void {
    const owner = trackedTarget(this);
    if (owner !== null && listener) {
      const key = `${owner}:${String(type)}`;
      const set = listeners.get(key) ?? new Set<unknown>();
      set.add(listener);
      listeners.set(key, set);
    }
    originalAdd.call(this, type, listener, eventOptions);
  };
  EventTarget.prototype.removeEventListener = function (
    this: EventTarget,
    type: string,
    listener: EventListenerOrEventListenerObject | null,
    eventOptions?: boolean | EventListenerOptions
  ): void {
    const owner = trackedTarget(this);
    if (owner !== null && listener) {
      listeners.get(`${owner}:${String(type)}`)?.delete(listener);
    }
    originalRemove.call(this, type, listener, eventOptions);
  };

  // Live observer targets. `observe` on an already-observed target is a no-op in
  // the platform, so the count is per call and `disconnect` clears the whole
  // instance's share.
  const observerTargetsByInstance = new Map<object, number>();
  let liveObserverTargets = 0;
  function wrapObserver(ctor: unknown): void {
    if (typeof ctor !== "function") {
      return;
    }
    const proto = (ctor as { prototype: Record<string, unknown> }).prototype;
    const observe = proto.observe as (...args: unknown[]) => unknown;
    const disconnect = proto.disconnect as (...args: unknown[]) => unknown;
    const unobserve = proto.unobserve as ((...args: unknown[]) => unknown) | undefined;
    if (typeof observe !== "function" || typeof disconnect !== "function") {
      return;
    }
    proto.observe = function (this: object, ...args: unknown[]) {
      observerTargetsByInstance.set(
        this,
        (observerTargetsByInstance.get(this) ?? 0) + 1
      );
      liveObserverTargets += 1;
      return observe.apply(this, args);
    };
    proto.disconnect = function (this: object, ...args: unknown[]) {
      liveObserverTargets -= observerTargetsByInstance.get(this) ?? 0;
      observerTargetsByInstance.delete(this);
      return disconnect.apply(this, args);
    };
    if (typeof unobserve === "function") {
      proto.unobserve = function (this: object, ...args: unknown[]) {
        const current = observerTargetsByInstance.get(this) ?? 0;
        if (current > 0) {
          observerTargetsByInstance.set(this, current - 1);
          liveObserverTargets -= 1;
        }
        return unobserve.apply(this, args);
      };
    }
  }
  wrapObserver(window.MutationObserver);
  wrapObserver((window as unknown as { ResizeObserver?: unknown }).ResizeObserver);
  wrapObserver((window as unknown as { IntersectionObserver?: unknown }).IntersectionObserver);

  function read(): RendererBudgetRead {
    return {
      commits: counters.commits,
      layoutReads: counters.layoutReads,
      floatingLayerReads: counters.floatingLayerReads,
      rafRegistrations: counters.raf,
      layoutReadBuckets: [...buckets.entries()]
        .sort((left, right) => right[1] - left[1])
        .map(([key, count]) => ({ key, count })),
      rafBuckets: [...rafBuckets.entries()]
        .sort((left, right) => right[1] - left[1])
        .map(([key, count]) => ({ key, count })),
      listenerSignature: JSON.stringify(
        [...listeners.entries()]
          .map(([key, set]) => [key, set.size] as const)
          .sort((left, right) => left[0].localeCompare(right[0]))
      ),
      observerTargets: liveObserverTargets,
      rows: document.querySelectorAll("[data-event-id]").length,
      timelineSubtree: document.querySelectorAll(".timeline-view *").length
    };
  }

  function reset(): void {
    counters.commits = 0;
    counters.layoutReads = 0;
    counters.floatingLayerReads = 0;
    counters.raf = 0;
    buckets.clear();
    rafBuckets.clear();
  }

  async function settle(
    options?: { quietFrames?: number; debounceMs?: number; timeoutMs?: number }
  ): Promise<void> {
    const quietFrames = options?.quietFrames ?? 10;
    const timeoutMs = options?.timeoutMs ?? 5000;
    // Advance past the anchor-persist debounce (#1894) before watching for
    // quiescence, so the write that debounce exists for is inside the bracket.
    await new Promise<void>((resolve) => setTimeout(resolve, options?.debounceMs ?? 300));
    const signature = () =>
      `${counters.commits}:${counters.layoutReads}:${counters.raf}`;
    let previous = signature();
    let stable = 0;
    const deadline = performance.now() + timeoutMs;
    while (performance.now() < deadline) {
      await new Promise<void>((resolve) => originalRaf(() => resolve()));
      const current = signature();
      stable = current === previous ? stable + 1 : 0;
      previous = current;
      if (stable >= quietFrames) {
        return;
      }
    }
    throw new Error(
      `renderer budget: the app did not settle within ${timeoutMs} ms ` +
        `(commits=${counters.commits} layoutReads=${counters.layoutReads} raf=${counters.raf} ` +
        `rafTop=${[...rafBuckets.entries()]
          .sort((left, right) => right[1] - left[1])
          .slice(0, 5)
          .map(([key, count]) => `${key}(${count})`)
          .join(",")})`
    );
  }

  self.__rendererBudget = { reset, read, settle };
}
