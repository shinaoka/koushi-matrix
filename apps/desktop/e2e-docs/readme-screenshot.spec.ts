import { createRequire } from "node:module";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

import { expect, test } from "@playwright/test";

import { createReadmeFixture } from "./readmeFixture";

const require = createRequire(import.meta.url);
const PLAYWRIGHT_VERSION = (require("@playwright/test/package.json") as { version: string }).version;
const SCREENSHOT_PATH = fileURLToPath(
  new URL("../../../assets/screenshots/koushi-main.png", import.meta.url)
);

async function settleLayout(page: import("@playwright/test").Page): Promise<void> {
  await page.evaluate(
    () =>
      new Promise<void>((resolve) => {
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
      })
  );
}

test("generates the deterministic README application composition", async ({ page }) => {
  expect(PLAYWRIGHT_VERSION).toBe("1.60.0");

  await page.goto("/appHarness.html");
  await page.addStyleTag({ content: ".sync-status-server { display: none !important; }" });
  await expect(page.getByRole("main")).toBeVisible();

  const sourceSnapshot = await page.evaluate(() => window.__harness.currentSnapshot());
  const fixture = createReadmeFixture(sourceSnapshot);
  await page.evaluate(async ({ stateUpdate, initialItems }) => {
    window.__harness.pushStateUpdate(stateUpdate);
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    await window.__harness.pushCoreEvent(initialItems);
  }, fixture);

  await expect(page.locator("article.message[data-event-id] .message-body").filter({ hasText: "Welcome to the planning room." })).toHaveCount(1);
  const workspaces = page.getByRole("navigation", { name: "Workspaces" });
  await expect(workspaces.getByRole("button", { name: "Lattice Lab", exact: true })).toHaveClass(/is-active/);
  await expect(workspaces.getByRole("button", { name: "Photon Reading Group", exact: true })).toBeVisible();
  await expect(workspaces.getByRole("button", { name: "Release Crew", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "General", exact: true })).toHaveClass(/is-active/);
  await expect(page.getByRole("button", { name: "Design", exact: true })).toContainText("3");
  await expect(page.locator('[data-room-section="favourites"]')).toContainText("Papers");
  await expect(page.getByRole("button", { name: "Aki", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: /DMs.*1 total/ })).toBeVisible();
  await expect(page.locator("article.message[data-event-id]")).toHaveCount(9);
  const dateLabel = new Intl.DateTimeFormat("en-US", {
    weekday: "short",
    month: "short",
    day: "numeric",
    year: "numeric",
    timeZone: "UTC"
  }).format(new Date(1773133200000));
  await expect(page.locator(".timeline-date-divider")).toHaveCount(1);
  await expect(page.locator(".timeline-date-divider")).toHaveText(dateLabel);
  await expect(page.locator(".reply-quote")).toHaveCount(1);
  await expect(page.locator(".thread-summary-chip")).toHaveCount(1);
  await expect(page.locator(".reaction-pill")).toHaveCount(2);
  await expect(page.locator('.reaction-pill[data-reacted-by-me="true"]')).toHaveCount(1);
  await expect(page.locator(".message-edited")).toHaveCount(1);
  expect(await page.locator("article.message.is-continuation").count()).toBeGreaterThan(0);

  const composer = page.locator('[role="textbox"][data-placeholder="Message General"]');
  await expect(composer).toBeVisible();
  await expect(composer).toHaveText("");
  await expect(page.locator(".app-grid")).toHaveClass(/thread-closed/);
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.getByRole("alert")).toHaveCount(0);

  const fontState = await page.evaluate(async () => {
    await document.fonts.ready;
    await Promise.all([
      document.fonts.load('400 14px "Inter"', "Koushi"),
      document.fonts.load('14px "Twemoji"', "👍")
    ]);
    return {
      inter: document.fonts.check('400 14px "Inter"', "Koushi"),
      twemoji: document.fonts.check('14px "Twemoji"', "👍"),
      uiFont: document.documentElement.dataset.uiFont,
      emojiFont: document.documentElement.dataset.emojiFont
    };
  });
  expect(fontState).toEqual({
    inter: true,
    twemoji: true,
    uiFont: "inter",
    emojiFont: "twemojiColr"
  });

  await settleLayout(page);
  await page.evaluate(() => document.activeElement instanceof HTMLElement && document.activeElement.blur());
  await page.locator('[data-testid="timeline-view"]').evaluate((element) => {
    element.scrollTop = 0;
  });
  await settleLayout(page);

  const visibleText = await page.locator("body").innerText();
  expect(visibleText).not.toMatch(
    /Harness Room|Harness Space|harness\.example\.invalid|example\.invalid|e2e-docs|\/tmp\/|date-divider-|\$readme-|!general:|@(?:aki|ren|sora):/
  );
  expect(visibleText).not.toContain(new Date().toISOString().slice(0, 10));
  expect(visibleText).not.toContain(
    new Intl.DateTimeFormat("en-US", { timeZone: "UTC" }).format(new Date())
  );

  await page.screenshot({ path: SCREENSHOT_PATH, animations: "disabled", caret: "hide" });
  const png = await readFile(SCREENSHOT_PATH);
  expect(png.subarray(0, 8)).toEqual(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]));
  expect(png.readUInt32BE(16)).toBe(2560);
  expect(png.readUInt32BE(20)).toBe(1600);
});
