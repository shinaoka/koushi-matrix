/**
 * Count-based renderer performance budget (#1895).
 *
 * This tier answers "did the app start doing more work than it used to?", not
 * "how fast was this machine today?". A frame time measures the runner, so the
 * budgets here count what the app *does*: forced layout reads (bucketed by
 * caller), React commits, rAF registrations, live listeners and observers, and
 * mounted rows. Every one of those is structural; every budget is a ceiling.
 *
 * The spec is a ratchet. A budget is the value measured on the day it was
 * written, plus headroom, with a comment saying what was measured and when.
 * Raising one is allowed — silently raising it is not: say in the PR which
 * user-visible cost grew and why it is worth paying.
 *
 * Three scenarios are deliberately zero targets, because a nonzero count there
 * is a defect rather than a cost: an idle app must not commit, must not register
 * another frame, and must not grow its listener or observer sets; a closed
 * receipts popover must not read layout at all (#1892); and a spec-side
 * `getBoundingClientRect` must never be attributed to the app.
 */

import { expect, test, type Page, type TestInfo } from "@playwright/test";

import { t } from "../src/i18n/messages";
import {
  TIMELINE_MIN_ITEM_HEIGHT_PX,
  TIMELINE_VIRTUAL_OVERSCAN_SCREENS
} from "../src/components/timeline/TimelineViewportVirtualization";
import {
  expectBudget,
  expectWithinBudget,
  installRendererBudget,
  readRendererBudget,
  resetRendererBudget,
  settleRendererBudget,
  type RendererBudgetRead
} from "./support/rendererBudget";
import {
  BUDGET_ROOMS,
  UNOPENED_BURST_ROOM_ID,
  gotoBudgetHarness,
  openBudgetRoom,
  seedBudgetRooms
} from "./support/rendererBudgetFixtures";
import { roomTimelineKey } from "../src/domain/coreEvents";

/** 1280x800 keeps the row window in one place across engines and machines. */
const VIEWPORT = { width: 1280, height: 800 };

const SWITCH_CYCLES = 5;
const SCROLL_STEPS = 20;
const SCROLL_STEP_PX = 160;
/** A real scroll delivers many events; the app must not pay per event. */
const SCROLL_EVENTS_PER_STEP = 10;
const IDLE_WINDOW_MS = 3000;
const BURST_EVENTS = 5000;

type Engine = "chromium" | "webkit";
type Budgets = Readonly<Record<string, number>>;

/**
 * Engine budgets. Measured 2026-11-14 in `mcr.microsoft.com/playwright:v1.60.0-noble`
 * (the CI image, so the fonts match) with `--repeat-each=20`, which landed the
 * same counts 20 times over; a budget is the highest observed value plus about
 * 20 %, and a counter that reads below 5 gets a few units of slack so a tiny
 * number cannot pin to an accident. WebKit counts are its own because its
 * layout and resize deliveries are.
 *
 * Every number here is a ceiling, so a run that lands under it is a pass; the
 * measured value in each comment is what the spec printed on that date.
 */
const ENGINE_BUDGETS: Record<
  Engine,
  {
    startup: Budgets;
    openBusy: Budgets;
    switchRoom: Budgets;
    scrollStep: Budgets;
    burst: Budgets;
    burstReportLines: number;
  }
> = {
  chromium: {
    // 15 / 224 / 1 row / 78 nodes.
    startup: { commits: 18, layoutReads: 270, rows: 4, timelineSubtree: 100 },
    // 1247 reads / 9 commits; the row count is derived from the viewport (#1890).
    openBusy: { layoutReads: 1500, commits: 12 },
    // Per switch, worst of five: 1489 / 14 commits. Refs #1889.
    switchRoom: { layoutReads: 1800, commits: 18 },
    // Per step of 10 scroll events: 1055 / 8 commits / 1 anchor write. Refs #1894.
    scrollStep: { layoutReads: 1300, commits: 10, anchorWrites: 2 },
    // 5,000 dropped events: 2 commits, 383 reads.
    burst: { commits: 6 },
    burstReportLines: 400
  },
  webkit: {
    // 14 / 209 / 1 row / 78 nodes.
    startup: { commits: 18, layoutReads: 270, rows: 4, timelineSubtree: 100 },
    // 1127 reads / 8 commits.
    openBusy: { layoutReads: 1500, commits: 12 },
    // Per switch, worst of five: 1362 / 13 commits.
    switchRoom: { layoutReads: 1800, commits: 18 },
    // Per step of 10 scroll events: 1055 / 5 commits / 1 anchor write.
    scrollStep: { layoutReads: 1300, commits: 10, anchorWrites: 2 },
    // 5,000 dropped events: 2 commits, 383 reads.
    burst: { commits: 6 },
    burstReportLines: 400
  }
};

function budgetsFor(testInfo: TestInfo) {
  const engine = testInfo.project.name as Engine;
  const budgets = ENGINE_BUDGETS[engine];
  if (!budgets) {
    throw new Error(`renderer budget: no budgets for project ${testInfo.project.name}`);
  }
  return budgets;
}

test.beforeEach(async ({ page }) => {
  await page.setViewportSize(VIEWPORT);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await installRendererBudget(page);
});

async function gotoBudgetApp(page: Page): Promise<void> {
  await gotoBudgetHarness(page);
  await seedBudgetRooms(page);
}

/** Opt-in: print the measured counts so a maintainer can recalibrate. */
function report(scenario: string, read: RendererBudgetRead): void {
  if (process.env.KOUSHI_RENDERER_BUDGET_REPORT !== "1") {
    return;
  }
  const top = read.layoutReadBuckets
    .slice(0, 5)
    .map((bucket) => `${bucket.key}(${bucket.count})`)
    .join(",");
  console.log(
    `renderer budget ${scenario}: commits=${read.commits} layoutReads=${read.layoutReads} ` +
      `floatingLayerReads=${read.floatingLayerReads} raf=${read.rafRegistrations} ` +
      `rows=${read.rows} subtree=${read.timelineSubtree} top=${top} rafTop=${read.rafBuckets
        .slice(0, 5)
        .map((bucket) => `${bucket.key}(${bucket.count})`)
        .join(",")}`
  );
}

/** Bracket one action: reset the counters, do it, wait for quiescence. */
async function measure(page: Page, action: () => Promise<void>): Promise<RendererBudgetRead> {
  await settleRendererBudget(page);
  await resetRendererBudget(page);
  await action();
  await settleRendererBudget(page);
  return readRendererBudget(page);
}

test("the budget counters themselves are alive", async ({ page }) => {
  await gotoBudgetApp(page);
  await settleRendererBudget(page);
  const read = await readRendererBudget(page);
  report("canary", read);

  // A hook the app stopped reporting through would leave every zero-budget
  // green, so the canary is what makes the rest of the file mean anything.
  expect(read.commits, "no React commit was counted: the devtools hook is not installed")
    .toBeGreaterThan(0);
  expect(read.layoutReads, "no layout read was counted: the getters are not wrapped")
    .toBeGreaterThan(0);
  expect(read.rows).toBeGreaterThan(0);
  expect(read.observerTargets).toBeGreaterThanOrEqual(1);

  // Playwright's injected script and this spec run outside `/src/`, so their
  // measurements must not be attributed to the app.
  const appReadsBefore = read.layoutReads;
  const measured = await page.evaluate(() => document.body.getBoundingClientRect().height);
  expect(measured).toBeGreaterThan(0);
  const after = await readRendererBudget(page);
  expect(
    after.layoutReads,
    "a spec-side getBoundingClientRect was attributed to the app"
  ).toBe(appReadsBefore);
});

test("startup stays inside its commit and layout-read budget", async ({ page }, testInfo) => {
  await gotoBudgetApp(page);
  await settleRendererBudget(page);
  report("startup", await readRendererBudget(page));
  await expectBudget(page, "startup", budgetsFor(testInfo).startup);
});

/** Rows the window may hold: three screens of the shortest measurable row. */
function windowRowBound(viewportHeight: number): number {
  const screenRows = Math.ceil(viewportHeight / TIMELINE_MIN_ITEM_HEIGHT_PX);
  return screenRows + 2 * TIMELINE_VIRTUAL_OVERSCAN_SCREENS * screenRows + 1;
}

test("opening a 120-row room mounts a window, not every row (#1890)", async ({
  page
}, testInfo) => {
  await gotoBudgetApp(page);
  const read = await measure(page, () => openBudgetRoom(page, BUDGET_ROOMS.busy));
  report("open busy", read);

  const viewportHeight = await page.evaluate(
    () => document.querySelector<HTMLElement>("[data-testid=timeline-view]")?.clientHeight ?? 0
  );
  expect(viewportHeight).toBeGreaterThan(0);
  // Before #1890 every row of a room at the replay cap mounted; the ratchet is
  // now the viewport, so a room of 120 rows and one of 200 mount alike.
  expectWithinBudget(
    "open busy rows",
    read.rows,
    windowRowBound(viewportHeight),
    read.layoutReadBuckets
  );
  expect(read.rows).toBeLessThan(BUDGET_ROOMS.busy.rowCount);
  await expectBudget(page, "open busy", {
    layoutReads: budgetsFor(testInfo).openBusy.layoutReads,
    commits: budgetsFor(testInfo).openBusy.commits
  });
});

test("opening a virtualized room mounts one window plus overscan", async ({ page }) => {
  await gotoBudgetApp(page);
  await measure(page, () => openBudgetRoom(page, BUDGET_ROOMS.long));

  const viewportHeight = await page.evaluate(
    () => document.querySelector<HTMLElement>("[data-testid=timeline-view]")?.clientHeight ?? 0
  );
  expect(viewportHeight).toBeGreaterThan(0);
  // One screenful of the shortest possible rows, plus overscan on both sides.
  // `TIMELINE_MIN_ITEM_HEIGHT_PX` is the estimator's floor for a row it has not
  // measured; a measured row can be shorter (`measuredItemHeight` floors at
  // 1 px). The fixture's rows are two-line text, so this bound holds for them —
  // it is a derivation from the fixture, not a bound for any timeline.
  const bound = windowRowBound(viewportHeight);
  const read = await readRendererBudget(page);
  report("open long", read);
  const timeline = page.locator("[data-testid=timeline-view]");
  await expect(timeline).toHaveAttribute("data-total-items", String(BUDGET_ROOMS.long.rowCount));
  expectWithinBudget("open long rows", read.rows, bound, read.layoutReadBuckets);
});

test("switching rooms stays inside its per-switch read budget (#1889)", async ({
  page
}, testInfo) => {
  await gotoBudgetApp(page);
  const sequence = [BUDGET_ROOMS.busy, BUDGET_ROOMS.quiet, BUDGET_ROOMS.busy, BUDGET_ROOMS.long];
  const { switchRoom } = budgetsFor(testInfo);

  let worstReads = 0;
  let worstCommits = 0;
  let worstBuckets: RendererBudgetRead["layoutReadBuckets"] = [];
  for (let cycle = 0; cycle < SWITCH_CYCLES; cycle += 1) {
    const room = sequence[cycle % sequence.length];
    const read = await measure(page, () => openBudgetRoom(page, room));
    report(`switch ${room.name}`, read);
    if (read.layoutReads > worstReads) {
      worstReads = read.layoutReads;
      worstBuckets = read.layoutReadBuckets;
    }
    worstCommits = Math.max(worstCommits, read.commits);
    // No popover is open during a switch, so the receipts hook every row owns
    // must not measure at all (#1892/#1894).
    expect(
      read.floatingLayerReads,
      `a closed receipts popover measured layout while switching to ${room.name}; top callers: ${read.layoutReadBuckets
        .slice(0, 5)
        .map((bucket) => `${bucket.key} (${bucket.count})`)
        .join(", ")}`
    ).toBe(0);
  }

  expectWithinBudget("switch rooms layoutReads", worstReads, switchRoom.layoutReads, worstBuckets);
  expectWithinBudget("switch rooms commits", worstCommits, switchRoom.commits, worstBuckets);
});

async function anchorWriteCount(page: Page): Promise<number> {
  return page.evaluate(
    () => window.__harness.invocationsOf("update_navigation_scroll_anchor").length
  );
}

test("scrolling a virtualized room stays inside its per-step budget", async ({
  page
}, testInfo) => {
  await gotoBudgetApp(page);
  await measure(page, () => openBudgetRoom(page, BUDGET_ROOMS.long));
  const { scrollStep } = budgetsFor(testInfo);

  let worstReads = 0;
  let worstCommits = 0;
  let worstAnchorWrites = 0;
  let worstBuckets: RendererBudgetRead["layoutReadBuckets"] = [];
  for (let step = 0; step < SCROLL_STEPS; step += 1) {
    const writesBefore = await anchorWriteCount(page);
    // A native wheel would put engine-specific input handling between the test
    // and the app (the recorded WebKit flake, #1507), so move the scrollport
    // directly and dispatch the scrolls the app listens for — many of them, as
    // a real scroll does.
    const read = await measure(page, async () => {
      await page.evaluate(
        ({ delta, events }) => {
          const container = document.querySelector<HTMLElement>("[data-testid=timeline-view]");
          if (!container) {
            throw new Error("renderer budget: timeline viewport is missing");
          }
          for (let event = 0; event < events; event += 1) {
            container.scrollTop = Math.max(0, container.scrollTop - delta / events);
            container.dispatchEvent(new Event("scroll", { bubbles: true }));
          }
        },
        { delta: SCROLL_STEP_PX, events: SCROLL_EVENTS_PER_STEP }
      );
    });
    const anchorWrites = (await anchorWriteCount(page)) - writesBefore;
    worstAnchorWrites = Math.max(worstAnchorWrites, anchorWrites);
    report(`scroll step ${step}`, read);
    if (read.layoutReads > worstReads) {
      worstReads = read.layoutReads;
      worstBuckets = read.layoutReadBuckets;
    }
    worstCommits = Math.max(worstCommits, read.commits);
    expect(
      read.floatingLayerReads,
      `a closed receipts popover measured layout while scrolling; top callers: ${read.layoutReadBuckets
        .slice(0, 5)
        .map((bucket) => `${bucket.key} (${bucket.count})`)
        .join(", ")}`
    ).toBe(0);
  }

  expectWithinBudget("scroll step layoutReads", worstReads, scrollStep.layoutReads, worstBuckets);
  expectWithinBudget("scroll step commits", worstCommits, scrollStep.commits, worstBuckets);
  // #1894: the room anchor is persisted once the scroll settles, not once per
  // scroll event. This is the count that #1894 itself lowered, and the only
  // place in this tier where the difference is visible: the harness does not
  // model the core's snapshot reply, so the extra writes would not show up as
  // commits.
  expectWithinBudget(
    `scroll step anchor writes for ${SCROLL_EVENTS_PER_STEP} scroll events`,
    worstAnchorWrites,
    scrollStep.anchorWrites,
    worstBuckets
  );
});

test("an idle app commits nothing and keeps its listener and observer sets", async ({ page }) => {
  await gotoBudgetApp(page);
  await measure(page, () => openBudgetRoom(page, BUDGET_ROOMS.busy));
  await resetRendererBudget(page);

  const before = await readRendererBudget(page);
  await page.waitForTimeout(IDLE_WINDOW_MS);
  const after = await readRendererBudget(page);
  report("idle", after);

  // #969: publishing latency samples from the app's own frame loop re-renders
  // the whole tree once a second. `live` stays off until the dialog asks.
  expect(after.commits, "an idle app committed: something published state per frame or per timer")
    .toBe(0);
  expect(
    after.rafRegistrations,
    "an idle app registered another animation frame (the diagnostics sampler is excluded)"
  ).toBe(0);
  expect(after.layoutReads, "an idle app read layout").toBe(0);
  expect(after.listenerSignature, "the live window/document listener set changed while idle").toBe(
    before.listenerSignature
  );
  expect(after.observerTargets, "the live observer set changed while idle").toBe(
    before.observerTargets
  );
});

test("a burst of events for an unopened key stays batched and logged once", async ({
  page
}, testInfo) => {
  await gotoBudgetApp(page);
  await measure(page, () => openBudgetRoom(page, BUDGET_ROOMS.busy));

  const burstKey = roomTimelineKey("@harness-user:example.invalid", UNOPENED_BURST_ROOM_ID);
  const read = await measure(page, async () => {
    await page.evaluate(
      async ({ key, count }) => {
        const pushes: Array<Promise<void>> = [];
        for (let index = 0; index < count; index += 1) {
          pushes.push(
            window.__harness.pushCoreEvent({
              kind: "Timeline",
              event: {
                ItemsUpdated: {
                  key,
                  generation: 2 + index,
                  batch_id: index + 1,
                  diffs: []
                }
              }
              // eslint-disable-next-line @typescript-eslint/no-explicit-any
            } as any)
          );
        }
        await Promise.all(pushes);
      },
      { key: burstKey, count: BURST_EVENTS }
    );
    // 5,000 events need longer than the default settle window to drain fully;
    // not settling at all is itself a finding.
    await settleRendererBudget(page, { timeoutMs: 20_000 });
  });

  report("burst", read);
  expectWithinBudget(
    "burst commits",
    read.commits,
    budgetsFor(testInfo).burst.commits,
    read.layoutReadBuckets
  );

  // The report is what a user would get, not an internal read: open the dialog
  // and save it, then parse the text the adapter was handed.
  await page
    .getByRole("button", { name: t("diagnostics.open") })
    .first()
    .evaluate((element: HTMLElement) => element.click());
  const dialog = page.getByRole("dialog", { name: t("diagnostics.title") });
  await expect(dialog).toBeVisible();
  await dialog
    .getByRole("button", { name: t("diagnostics.save") })
    .evaluate((element: HTMLElement) => element.click());
  await expect(dialog.getByRole("status")).toHaveText(t("diagnostics.saved"));

  const savedReport = await page.evaluate(() => {
    const invocations = window.__harness.invocationsOf("save_diagnostics_report");
    const args = invocations.at(-1)?.args as { report?: string } | undefined;
    return args?.report ?? null;
  });
  expect(savedReport, "the Diagnostics save reached no report").not.toBeNull();
  const text = savedReport ?? "";

  const received = Number(/timeline_evt_received=(\d+)/.exec(text)?.[1] ?? -1);
  const dropped = Number(/timeline_evt_key_dropped=(\d+)/.exec(text)?.[1] ?? -1);
  expect(received).toBeGreaterThanOrEqual(BURST_EVENTS);
  expect(dropped).toBeGreaterThanOrEqual(BURST_EVENTS);
  // Everything except the fixtures' own batches was dropped by the key filter.
  expect(received - dropped).toBeLessThanOrEqual(50);
  // One summary record per mismatch window, not one per event: a per-event log
  // would have overflowed the report and the log buffer.
  expect(text).toContain("Diagnostic records dropped: 0");
  expectWithinBudget(
    "burst report lines",
    text.split("\n").length,
    budgetsFor(testInfo).burstReportLines
  );
});
