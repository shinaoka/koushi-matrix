import { expect, test, type Page } from "@playwright/test";
import { t } from "../src/i18n/messages";
import { gotoReadyShell } from "./support/basicOperations";

/**
 * #1509: a rail with more Spaces than fit keeps the active Space in view, shows
 * that it scrolls, and surfaces attention on Spaces scrolled out of view.
 */

const SPACE_COUNT = 60;

function spaceName(index: number): string {
  return `Rail Space ${String(index).padStart(2, "0")}`;
}

async function seedRail(
  page: Page,
  options: { active: number | null; unread?: Record<number, number>; highlight?: Record<number, number> }
): Promise<void> {
  await page.evaluate(
    ({ count, active, unread, highlight }) => {
      const next = structuredClone(window.__harness.currentSnapshot());
      next.sidebar.account_home.is_active = active === null;
      next.sidebar.space_rail = Array.from({ length: count }, (_, index) => ({
        space_id: `!rail-${index}:example.invalid`,
        display_name: `Rail Space ${String(index).padStart(2, "0")}`,
        local_icon: null,
        avatar: null,
        unread_count: unread[index] ?? 0,
        highlight_count: highlight[index] ?? 0,
        is_active: index === active,
        leave_candidates: []
      }));
      window.__harness.setSnapshot(next);
      window.__harness.pushStateUpdate();
    },
    {
      count: SPACE_COUNT,
      active: options.active,
      unread: options.unread ?? {},
      highlight: options.highlight ?? {}
    }
  );
  await expect(page.getByRole("button", { name: spaceName(SPACE_COUNT - 1), exact: true })).toHaveCount(1);
}

/** Whether the named rail button is fully inside the rail's scrollport. */
async function fullyVisibleInRail(page: Page, name: string): Promise<boolean> {
  return page.evaluate((label) => {
    const list = document.querySelector<HTMLElement>(".workspace-space-list");
    const button = Array.from(
      document.querySelectorAll<HTMLElement>(".workspace-space-button")
    ).find((candidate) => candidate.getAttribute("aria-label") === label);
    if (!list || !button) throw new Error("rail list or button is missing");
    const listBox = list.getBoundingClientRect();
    const box = button.getBoundingClientRect();
    return box.top >= listBox.top - 1 && box.bottom <= listBox.bottom + 1;
  }, name);
}

async function railScrollTop(page: Page): Promise<number> {
  return page.evaluate(
    () => document.querySelector<HTMLElement>(".workspace-space-list")?.scrollTop ?? -1
  );
}

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 760, height: 520 });
  await gotoReadyShell(page);
});

test("the footer button shows the selected account's avatar (#2088)", async ({ page }) => {
  // The single line that feeds the drawing is the app's own prop lookup, so
  // this asserts the real shell end to end: the harness publishes one ready tab
  // with no thumbnail, whose display name starts "@harness-user", so the button
  // draws that account's initial instead of the generic account glyph.
  const avatar = page.locator(".rail-footer .rail-account-avatar");
  await expect(avatar).toBeVisible();
  await expect(avatar).toHaveText("H");
  await expect(page.locator(".rail-footer svg.lucide-user-round")).toHaveCount(0);
});

test("selecting an off-screen Space by a non-rail path scrolls it into view", async ({ page }) => {
  await seedRail(page, { active: 0 });
  expect(await railScrollTop(page)).toBe(0);
  expect(await fullyVisibleInRail(page, spaceName(50))).toBe(false);

  // A notification, Activity, or restored navigation only moves `is_active`.
  await seedRail(page, { active: 50 });
  await expect.poll(() => fullyVisibleInRail(page, spaceName(50))).toBe(true);
});

test("an unrelated rail update does not reset a manual rail scroll", async ({ page }) => {
  await seedRail(page, { active: 0 });
  await page.evaluate(() => {
    const list = document.querySelector<HTMLElement>(".workspace-space-list");
    if (list) list.scrollTop = 600;
  });
  await expect.poll(() => railScrollTop(page)).toBe(600);
  await seedRail(page, { active: 0, unread: { 3: 2 } });
  await expect(page.getByRole("button", { name: spaceName(3), exact: true })).toHaveAttribute(
    "data-count",
    "2"
  );
  expect(await railScrollTop(page)).toBe(600);
});

test("the rail marks which edges overflow", async ({ page }) => {
  await seedRail(page, { active: 0 });
  const scroller = page.locator(".workspace-space-scroller");
  await expect(scroller).toHaveAttribute("data-overflow-below", "true");
  await expect(scroller).not.toHaveAttribute("data-overflow-above", "true");

  await page.evaluate(() => {
    const list = document.querySelector<HTMLElement>(".workspace-space-list");
    if (list) list.scrollTop = list.scrollHeight;
  });
  await expect(scroller).toHaveAttribute("data-overflow-above", "true");
  await expect(scroller).not.toHaveAttribute("data-overflow-below", "true");
});

test("attention below the visible range is surfaced and reachable from the rail edge", async ({
  page
}) => {
  await seedRail(page, { active: 0, unread: { 55: 9, 57: 2 } });
  const below = page.getByRole("button", {
    name: t("workspace.railActivityBelow", { count: "11" }),
    exact: true
  });
  await expect(below).toBeVisible();
  await expect(
    page.getByRole("button", { name: t("workspace.railActivityAbove", { count: "11" }) })
  ).toHaveCount(0);

  await below.click();
  await expect.poll(() => fullyVisibleInRail(page, spaceName(55))).toBe(true);
});

test("attention above the visible range is surfaced with mentions called out", async ({ page }) => {
  await seedRail(page, { active: 58, unread: { 2: 4 }, highlight: { 2: 1 } });
  await expect.poll(() => fullyVisibleInRail(page, spaceName(58))).toBe(true);
  const above = page.getByRole("button", {
    name: t("workspace.railMentionsAbove", { count: "4" }),
    exact: true
  });
  await expect(above).toBeVisible();
  await expect(above).toHaveAttribute("data-highlight", "true");

  await above.click();
  await expect.poll(() => fullyVisibleInRail(page, spaceName(2))).toBe(true);
  await expect(above).toHaveCount(0);
});

test("a rail that fits shows no overflow indicators", async ({ page }) => {
  await page.evaluate(() => {
    const next = structuredClone(window.__harness.currentSnapshot());
    next.sidebar.space_rail = next.sidebar.space_rail.slice(0, 2);
    window.__harness.setSnapshot(next);
    window.__harness.pushStateUpdate();
  });
  const scroller = page.locator(".workspace-space-scroller");
  await expect(scroller).not.toHaveAttribute("data-overflow-above", "true");
  await expect(scroller).not.toHaveAttribute("data-overflow-below", "true");
  await expect(page.locator(".workspace-rail-edge-indicator")).toHaveCount(0);
});
