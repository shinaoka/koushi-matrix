/**
 * Headless spec: the Space description outside Space info (#1661).
 *
 * These cases select the harness Space and seed the synced Space
 * projection (`SpaceSummary.topic`) and check what the engine really lays out:
 * the clamp hides lines until the toggle expands them, a short description has
 * no toggle, every line resolves its own direction (#1084), and a Space invite
 * keeps its description's line breaks. All content is synthetic.
 */

import { expect, test, type Page } from "@playwright/test";
import { gotoReadyShell } from "./support/basicOperations";

const LONG_DESCRIPTION = [
  "Welcome to the synthetic workspace.",
  "Please join the general room first.",
  "Introduce yourself in the welcome room.",
  "Read the guide before posting announcements.",
  "Ask questions in the help room."
].join("\n");
const RTL_LINE = "שלום 42!";

const HARNESS_SPACE = "Harness Space";

/** Seed the harness Space's synced description, then select that Space. */
async function setSpaceTopic(page: Page, topic: string | null): Promise<void> {
  await page.evaluate((value) => {
    const next = structuredClone(window.__harness.currentSnapshot());
    next.state.domain.spaces = next.state.domain.spaces.map((space) => ({ ...space, topic: value }));
    window.__harness.setSnapshot(next);
    window.__harness.pushStateUpdate();
  }, topic);
  const space = page
    .getByRole("navigation", { name: "Workspaces" })
    .getByRole("button", { name: HARNESS_SPACE, exact: true });
  if ((await space.getAttribute("aria-current")) === null) {
    await space.click();
  }
  await expect(page.getByRole("complementary", { name: "Rooms" }).locator(".workspace-name")).toHaveText(
    HARNESS_SPACE
  );
}

function sidebarDescription(page: Page) {
  return page.getByRole("complementary", { name: "Rooms" }).getByRole("region", { name: "Description" });
}

test("a long Space description is clamped and expands and collapses from its toggle", async ({
  page
}, testInfo) => {
  await gotoReadyShell(page);
  await setSpaceTopic(page, LONG_DESCRIPTION);

  const region = sidebarDescription(page);
  const text = region.locator(".space-description-text");
  const toggle = region.getByRole("button", { name: "Show more" });
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await expect(toggle).toHaveAttribute("aria-controls", (await text.getAttribute("id")) ?? "");
  const collapsed = await text.evaluate((element) => ({
    height: element.getBoundingClientRect().height,
    lineHeight: Number.parseFloat(getComputedStyle(element).lineHeight),
    overflowing: element.scrollHeight > element.clientHeight
  }));
  expect(collapsed.overflowing).toBe(true);
  expect(collapsed.height).toBeLessThanOrEqual(collapsed.lineHeight * 3 + 1);
  await page.screenshot({ path: testInfo.outputPath("space-description-collapsed.png") });

  await toggle.click();
  const less = region.getByRole("button", { name: "Show less" });
  await expect(less).toHaveAttribute("aria-expanded", "true");
  const expandedHeight = await text.evaluate((element) => element.getBoundingClientRect().height);
  expect(expandedHeight).toBeGreaterThanOrEqual(collapsed.lineHeight * 5 - 1);
  await page.screenshot({ path: testInfo.outputPath("space-description-expanded.png") });

  await less.click();
  await expect(region.getByRole("button", { name: "Show more" })).toHaveAttribute(
    "aria-expanded",
    "false"
  );
});

test("a short Space description has no toggle and an empty one renders nothing", async ({ page }) => {
  await gotoReadyShell(page);
  await setSpaceTopic(page, "Short synthetic welcome");
  const region = sidebarDescription(page);
  await expect(region).toContainText("Short synthetic welcome");
  await expect(region.getByRole("button")).toHaveCount(0);

  await setSpaceTopic(page, null);
  await expect(sidebarDescription(page)).toHaveCount(0);
});

test("each description line resolves its own direction", async ({ page }) => {
  await gotoReadyShell(page);
  await setSpaceTopic(page, `Welcome line\n${RTL_LINE}`);
  const text = sidebarDescription(page).locator(".space-description-text");
  await expect(text).toContainText(RTL_LINE);

  // In the RTL line the trailing "!" is drawn at the line's left edge, beside
  // its digits; an LTR-resolved paragraph would strand it to their right.
  const edges = await text.evaluate((element, line) => {
    const node = element.firstChild;
    if (!node || node.nodeType !== Node.TEXT_NODE) return null;
    const content = node.textContent ?? "";
    const lineStart = content.indexOf(line);
    const rectOf = (offset: number, length: number) => {
      const range = document.createRange();
      range.setStart(node, lineStart + offset);
      range.setEnd(node, lineStart + offset + length);
      return range.getBoundingClientRect();
    };
    const bang = rectOf(line.indexOf("!"), 1);
    const digits = rectOf(line.indexOf("42"), 2);
    return { bangRight: bang.right, digitsLeft: digits.left };
  }, RTL_LINE);
  expect(edges).not.toBeNull();
  expect(edges!.bangRight).toBeLessThanOrEqual(edges!.digitsLeft + 0.5);
});

test("a Space invite shows its description with line breaks", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const next = structuredClone(window.__harness.currentSnapshot());
    next.state.domain.invites = [
      {
        room_id: "!qa-space-invite:example.invalid",
        display_name: "Synthetic invited Space",
        avatar: null,
        topic: "Welcome to the invited Space.\nPlease join the general room.",
        inviter_display_name: "Synthetic inviter",
        inviter_user_id: "@inviter:example.invalid",
        is_dm: false,
        is_space: true
      }
    ];
    window.__harness.setSnapshot(next);
    window.__harness.pushStateUpdate();
  });
  await page
    .getByRole("navigation", { name: "Workspaces" })
    .getByRole("button", { name: /^Home/ })
    .click();
  await page.getByRole("button", { name: "Invites", exact: true }).click();

  const preview = page.getByRole("region", { name: "Invite preview" });
  const tile = preview.locator(".settings-summary-tile", { hasText: "Description" });
  const value = tile.locator("strong");
  await expect(value).toHaveText("Welcome to the invited Space.\nPlease join the general room.");
  // Two rendered lines: the text's line boxes start at two distinct offsets.
  const lineTops = await value.evaluate((element) => {
    const range = document.createRange();
    range.selectNodeContents(element);
    return new Set(Array.from(range.getClientRects(), (rect) => Math.round(rect.top))).size;
  });
  expect(lineTops).toBeGreaterThanOrEqual(2);
});
