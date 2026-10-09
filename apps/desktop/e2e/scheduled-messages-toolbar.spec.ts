/**
 * Headless spec: the unified Home/Space toolbar (#1160).
 *
 * The header row has a left context group (Home's Activity/Explore/Invites, or
 * the Space's Members) and a right group (Threads, Scheduled messages, Info).
 * `MIN_SIDEBAR_WIDTH` is 260px, so six 32px actions plus 8px gaps overflowed
 * before this change (232 + 32 = 264). These tests measure the real rendered
 * groups at the minimum width instead of trusting a breakpoint: one row, no
 * sibling overlap, the logical-start/logical-end split and intra-group order,
 * the contained invite badge, the truncated name, and keyboard reachability.
 * Japanese and pseudo-accented labels stand in for expanded localized text.
 */

import { expect, test, type Page } from "@playwright/test";
import { t } from "../src/i18n/messages";
import { gotoReadyShell } from "./support/basicOperations";

const MIN_SIDEBAR_WIDTH = 260;
const HARNESS_SPACE_ID = "!harness-space:example.invalid";
const LONG_NAME = "Synthetic Workspace With A Deliberately Long Local Name";

type Scenario = {
  name: string;
  home: boolean;
  lang: string;
  dir: "ltr" | "rtl";
  catalog_locale: "en" | "ja" | "pseudo";
  pseudo_locale: "none" | "accented" | "bidi";
};

const SCENARIOS: Scenario[] = [
  { name: "English Home", home: true, lang: "en", dir: "ltr", catalog_locale: "en", pseudo_locale: "none" },
  { name: "English Space", home: false, lang: "en", dir: "ltr", catalog_locale: "en", pseudo_locale: "none" },
  { name: "Japanese Home", home: true, lang: "ja", dir: "ltr", catalog_locale: "ja", pseudo_locale: "none" },
  { name: "Japanese Space", home: false, lang: "ja", dir: "ltr", catalog_locale: "ja", pseudo_locale: "none" },
  { name: "pseudo-accented Home", home: true, lang: "en-XA", dir: "ltr", catalog_locale: "pseudo", pseudo_locale: "accented" },
  { name: "pseudo-bidi Space", home: false, lang: "ar-XB", dir: "rtl", catalog_locale: "pseudo", pseudo_locale: "bidi" }
];

async function applyScenario(page: Page, scenario: Scenario): Promise<void> {
  await page.evaluate(
    ({ home, lang, dir, catalog_locale, pseudo_locale, longName, spaceId }) => {
      const next = structuredClone(window.__harness.currentSnapshot());
      next.state.domain.locale_profile = {
        ...next.state.domain.locale_profile,
        lang,
        dir,
        catalog_locale,
        pseudo_locale
      };
      next.sidebar.account_home.display_name = longName;
      next.sidebar.space_rail = next.sidebar.space_rail.map((space) =>
        space.space_id === spaceId ? { ...space, display_name: longName, is_active: !home } : { ...space, is_active: false }
      );
      if (home) {
        next.sidebar.account_home.is_active = true;
        next.sidebar.active_space_id = null;
        next.state.ui.navigation.active_space_id = null;
        next.state.domain.invites = Array.from({ length: 12 }, (_, index) => ({
          room_id: `!invite-${index}:example.invalid`,
          display_name: `Synthetic invite ${index}`,
          avatar: null,
          topic: null,
          inviter_display_name: "Synthetic inviter",
          inviter_user_id: "@inviter:example.invalid",
          is_dm: false,
          is_space: false
        }));
        next.sidebar.account_home.invite_count = 12;
      }
      window.__harness.setSnapshot(next);
      window.__harness.pushStateUpdate();
    },
    {
      home: scenario.home,
      lang: scenario.lang,
      dir: scenario.dir,
      catalog_locale: scenario.catalog_locale,
      pseudo_locale: scenario.pseudo_locale,
      longName: LONG_NAME,
      spaceId: HARNESS_SPACE_ID
    }
  );
  await expect
    .poll(() => page.evaluate(() => document.documentElement.dir))
    .toBe(scenario.dir);
}

async function minimizeSidebar(page: Page): Promise<void> {
  // The resizer's accessible name is localized, so drive it by its stable
  // shell class instead of an English label the JA/pseudo scenarios do not use.
  const resizer = page.locator(".app-grid-resizer");
  const box = await resizer.boundingBox();
  expect(box).not.toBeNull();
  await page.mouse.move(box!.x + box!.width / 2, box!.y + 4);
  await page.mouse.down();
  await page.mouse.move(box!.x + box!.width / 2 - 1000, box!.y + 4);
  await page.mouse.up();
  await expect
    .poll(() => page.locator(".sidebar").evaluate((element) => element.getBoundingClientRect().width))
    .toBe(MIN_SIDEBAR_WIDTH);
}

interface Box {
  left: number;
  right: number;
  top: number;
  bottom: number;
}

interface GeometryReport {
  direction: string;
  header: Box;
  actions: Box;
  groups: Array<{ name: string; box: Box; actions: string[]; buttons: Box[] }>;
  buttons: Array<{ action: string; box: Box }>;
  badge: { box: Box; scrollWidth: number; clientWidth: number } | null;
  name: { scrollWidth: number; clientWidth: number; textOverflow: string; whiteSpace: string };
}

async function toolbarGeometry(page: Page): Promise<GeometryReport> {
  return page.evaluate(() => {
    const box = (element: Element): Box => {
      const rect = element.getBoundingClientRect();
      return { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom };
    };
    const actions = document.querySelector<HTMLElement>(".workspace-header-actions")!;
    const groups = Array.from(document.querySelectorAll<HTMLElement>("[data-toolbar-group]")).map(
      (group) => {
        const buttons = Array.from(
          group.querySelectorAll<HTMLElement>("[data-header-action]")
        );
        return {
          name: group.dataset.toolbarGroup ?? "",
          box: box(group),
          actions: buttons.map((button) => button.dataset.headerAction ?? ""),
          buttons: buttons.map(box)
        };
      }
    );
    const badge = document.querySelector<HTMLElement>(".workspace-header-badge");
    const name = document.querySelector<HTMLElement>(".workspace-name")!;
    const nameStyle = getComputedStyle(name);
    return {
      direction: getComputedStyle(actions).direction,
      header: box(document.querySelector<HTMLElement>(".workspace-header")!),
      actions: box(actions),
      groups,
      buttons: Array.from(document.querySelectorAll<HTMLElement>("[data-header-action]")).map(
        (button) => ({ action: button.dataset.headerAction ?? "", box: box(button) })
      ),
      badge: badge
        ? { box: box(badge), scrollWidth: badge.scrollWidth, clientWidth: badge.clientWidth }
        : null,
      name: {
        scrollWidth: name.scrollWidth,
        clientWidth: name.clientWidth,
        textOverflow: nameStyle.textOverflow,
        whiteSpace: nameStyle.whiteSpace
      }
    };
  });
}

for (const scenario of SCENARIOS) {
  test(`${scenario.name} toolbar holds one row at the minimum sidebar width`, async ({ page }) => {
    await page.setViewportSize({ width: 1400, height: 800 });
    await gotoReadyShell(page);
    if (scenario.home) {
      await page
        .getByRole("navigation", { name: t("workspace.workspaces") })
        .getByRole("button", { name: /^Home/ })
        .click();
    }
    await applyScenario(page, scenario);
    await minimizeSidebar(page);

    const report = await toolbarGeometry(page);
    expect(report.buttons.length).toBeGreaterThanOrEqual(4);

    // One row: every action shares the same block band.
    const top = report.buttons[0]!.box.top;
    const bottom = report.buttons[0]!.box.bottom;
    for (const button of report.buttons) {
      expect(Math.abs(button.box.top - top)).toBeLessThanOrEqual(1);
      expect(Math.abs(button.box.bottom - bottom)).toBeLessThanOrEqual(1);
    }

    // No sibling overlap, measured left-to-right.
    const ordered = [...report.buttons].sort((a, b) => a.box.left - b.box.left);
    for (let index = 1; index < ordered.length; index += 1) {
      expect(ordered[index]!.box.left).toBeGreaterThanOrEqual(ordered[index - 1]!.box.right - 1);
    }

    // The context group sits at the logical start; Threads/Scheduled/Info at the
    // logical end, in that intra-group order.
    const context = report.groups.find((group) => group.name === "context");
    const end = report.groups.find((group) => group.name === "end");
    expect(context).toBeTruthy();
    expect(end).toBeTruthy();
    expect(end!.actions).toEqual(["threads", "scheduled", "info"]);
    const logicalStart = report.direction === "rtl" ? "right" : "left";
    const logicalEnd = report.direction === "rtl" ? "left" : "right";
    expect(Math.abs(context!.box[logicalStart] - report.actions[logicalStart])).toBeLessThanOrEqual(1);
    expect(Math.abs(end!.box[logicalEnd] - report.actions[logicalEnd])).toBeLessThanOrEqual(1);

    // The invite badge stays inside the context group and the header row.
    if (scenario.home) {
      expect(report.badge).not.toBeNull();
      expect(report.badge!.scrollWidth).toBeLessThanOrEqual(report.badge!.clientWidth + 1);
      expect(report.badge!.box.left).toBeGreaterThanOrEqual(context!.box.left - 1);
      expect(report.badge!.box.right).toBeLessThanOrEqual(context!.box.right + 1);
      expect(report.badge!.box.top).toBeGreaterThanOrEqual(report.header.top - 1);
      expect(report.badge!.box.bottom).toBeLessThanOrEqual(report.header.bottom + 1);
    }

    // A long name truncates rather than pushing the actions out of the row.
    expect(report.name.textOverflow).toBe("ellipsis");
    expect(report.name.whiteSpace).toBe("nowrap");
    expect(report.name.scrollWidth).toBeGreaterThan(report.name.clientWidth);
  });
}

test("every toolbar action is keyboard-reachable", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 800 });
  await gotoReadyShell(page);
  await page
    .getByRole("navigation", { name: t("workspace.workspaces") })
    .getByRole("button", { name: /^Home/ })
    .click();
  await minimizeSidebar(page);

  const first = page.locator('[data-header-action="activity"]');
  await first.focus();
  const reached: string[] = [];
  for (let index = 0; index < 6; index += 1) {
    reached.push(
      (await page.evaluate(() => document.activeElement?.getAttribute("data-header-action"))) ?? ""
    );
    await page.keyboard.press("Tab");
  }
  expect(reached).toEqual(["activity", "explore", "invites", "threads", "scheduled", "info"]);
});

test("opening another panel closes the scheduled-sends projection", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 800 });
  await gotoReadyShell(page);
  await page.evaluate(() => {
    window.__harness.setCommandResponse("open_scheduled_sends_list", () => {
      const current = window.__harness.currentSnapshot();
      const next = {
        ...current,
        state: {
          ...current.state,
          ui: {
            ...current.state.ui,
            scheduled_sends_list: {
              kind: "open" as const,
              scope: { kind: "space" as const, space_id: "!harness-space:example.invalid" },
              capability: "localFallback" as const,
              items: [
                {
                  scheduled_id: "synthetic-1160",
                  room_id: "!harness-room:example.invalid",
                  body: "Synthetic scheduled body",
                  send_at_ms: Date.now() + 3_600_000,
                  handle: { kind: "local" as const }
                }
              ]
            }
          }
        }
      };
      window.__harness.setSnapshot(next);
      return next;
    });
  });
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.locator('[data-header-action="scheduled"]').click();
  await expect(page.getByText("Synthetic scheduled body")).toBeVisible();

  // Info opens a frontend-only panel; the mode transition itself must retire
  // the body-bearing Rust projection.
  await page.locator('[data-header-action="info"]').click();
  await expect
    .poll(() => page.evaluate(() => window.__harness.invocationsOf("close_scheduled_sends_list").length))
    .toBeGreaterThan(0);
  await expect(page.getByText("Synthetic scheduled body")).not.toBeVisible();
});
