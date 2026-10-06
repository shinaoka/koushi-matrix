/**
 * Headless spec: top-bar control alignment (#320).
 *
 * #320 reports that the macOS traffic lights and top-bar controls sit higher
 * than the search field. The native traffic lights are placed by
 * `tauri.conf.json` and cannot be measured here, but whether the DOM controls
 * agree with each other is a plain layout fact — so measure it rather than
 * reasoning about the CSS.
 *
 * This pins the answer: every top-bar control shares one center line. If a
 * future change to `--icon-button-size`, a control height, or the titlebar row
 * breaks that, this fails instead of becoming a pixel-hunting bug report.
 */

import { expect, test, type Page } from "@playwright/test";
import { t } from "../src/i18n/messages";

/** Every interactive control in the top bar, in visual order. */
const TOP_BAR_CONTROLS = [
  ".account-tab-strip .account-tab-add",
  ".top-search input",
  ".scope-select",
  ".top-actions .app-settings-button",
  ".top-actions .icon-button:not(.app-settings-button)"
] as const;

async function gotoReadyShell(page: Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: t("timeline.conversation") })).toBeVisible();
}

test("every top-bar control shares one vertical center line", async ({ page }) => {
  await gotoReadyShell(page);
  await expect(page.locator(".titlebar .history")).toHaveCount(0);

  const measured = await page.evaluate((selectors) => {
    const bar = document.querySelector<HTMLElement>(".titlebar");
    if (!bar) {
      return null;
    }
    const barRect = bar.getBoundingClientRect();
    const centers = selectors.map((selector) => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) {
        return { selector, center: null as number | null };
      }
      const rect = element.getBoundingClientRect();
      return { selector, center: rect.top + rect.height / 2 };
    });
    return {
      barCenter: barRect.top + barRect.height / 2,
      centers
    };
  }, TOP_BAR_CONTROLS);

  expect(measured).not.toBeNull();
  const { barCenter, centers } = measured!;

  for (const { selector, center } of centers) {
    expect(center, `${selector} should be present in the top bar`).not.toBeNull();
  }

  const values = centers.map((entry) => entry.center as number);
  const spread = Math.max(...values) - Math.min(...values);
  expect(
    spread,
    `top-bar controls disagree on their center line: ${JSON.stringify(centers)}`
  ).toBeLessThanOrEqual(0.5);

  // The controls are centered inside the titlebar's content box, which the 1px
  // bottom border makes 1px shorter than the window row the native macOS
  // traffic lights are positioned against. Half a pixel is not perceptible,
  // but a larger drift would mean the two coordinate systems have diverged.
  expect(Math.abs(values[0]! - barCenter)).toBeLessThanOrEqual(0.5);
});

test("account status and Diagnostics stay reachable at the minimum width", async ({ page }) => {
  await page.setViewportSize({ width: 760, height: 800 });
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          locale_profile: { ...snapshot.state.domain.locale_profile, platform: "macos" }
        }
      }
    });
    window.__harness.pushStateUpdate();
  });

  await expect(page.locator('.account-tab[data-status="ready"] .account-tab-ready-dot')).toBeVisible();
  await expect(page.getByRole("button", { name: t("diagnostics.open") })).toBeVisible();

  const layout = await page.evaluate(() => {
    const titlebar = document.querySelector<HTMLElement>(".titlebar")!;
    const actions = document.querySelector<HTMLElement>(".top-actions")!;
    return {
      overflow: titlebar.scrollWidth - titlebar.clientWidth,
      actionsWithinTitlebar: actions.getBoundingClientRect().right <= titlebar.getBoundingClientRect().right
    };
  });
  expect(layout.overflow).toBeLessThanOrEqual(1);
  expect(layout.actionsWithinTitlebar).toBe(true);
});

test("search scope selector fits Room/DM in English and Japanese", async ({ page }) => {
  await gotoReadyShell(page);

  const measureScope = async () =>
    page.evaluate(() => {
      const select = document.querySelector<HTMLSelectElement>(".scope-select");
      if (!select) {
        return null;
      }
      const label = [...select.options].find((option) => option.value === "currentRoom")?.text ?? "";
      const style = getComputedStyle(select);
      const canvas = document.createElement("canvas");
      const context = canvas.getContext("2d");
      if (!context) {
        return null;
      }
      context.font = style.font;
      const textWidth = context.measureText(label).width;
      const rect = select.getBoundingClientRect();
      const titlebar = document.querySelector<HTMLElement>(".titlebar")!;
      return {
        label,
        width: rect.width,
        requiredWidth: Math.ceil(textWidth + 48),
        titlebarRight: titlebar.getBoundingClientRect().right,
        selectRight: rect.right,
        titlebarOverflow: titlebar.scrollWidth - titlebar.clientWidth,
        viewportWidth: window.innerWidth
      };
    });

  const english = await measureScope();
  expect(english).not.toBeNull();
  expect(english!.label).toBe("Room/DM");
  expect(english!.width).toBeGreaterThanOrEqual(english!.requiredWidth);
  expect(english!.selectRight).toBeLessThanOrEqual(english!.titlebarRight);
  expect(english!.titlebarOverflow).toBeLessThanOrEqual(1);

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          locale_profile: {
            ...snapshot.state.domain.locale_profile,
            lang: "ja",
            catalog_locale: "ja",
            pseudo_locale: "none"
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
  });

  await expect(page.locator(".scope-select option[value=currentRoom]")).toHaveText("ルーム/DM");
  const japanese = await measureScope();
  expect(japanese).not.toBeNull();
  expect(japanese!.label).toBe("ルーム/DM");
  expect(japanese!.width).toBeGreaterThanOrEqual(japanese!.requiredWidth);
  expect(japanese!.selectRight).toBeLessThanOrEqual(japanese!.titlebarRight);
  expect(japanese!.titlebarOverflow).toBeLessThanOrEqual(1);
  expect(japanese!.selectRight).toBeLessThanOrEqual(japanese!.viewportWidth);
});
