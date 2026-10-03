/**
 * Headless spec: emoji picker (#79, #302).
 *
 * Proves that the emoji picker opens from the composer, supports search
 * and keyboard nav, inserts at the caret, and dismisses correctly — all
 * without any network call.
 *
 *  1. Emoji button opens the picker.
 *  2. Selecting an emoji by mouse inserts it into the composer draft.
 *  3. Typing a search term filters the emoji grid.
 *  4. Escape dismisses the picker.
 *  5. Clicking outside the picker dismisses it.
 *  6. "No results" message shown when search yields nothing.
 *  7. Picker does not make any network fetch request.
 *  8. Picker does not obstruct the send button (both visible simultaneously).
 *  9. Arrow key navigation then Enter inserts the highlighted emoji.
 * 10. The picker stays inside the viewport from the main composer, from the
 *     thread composer, and at a narrow window width (#302 placement).
 * 11. The rendered grid density matches the keyboard column step (#302).
 */

import { expect, test } from "@playwright/test";
import { t } from "../src/i18n/messages";
import { seedTimelineItems } from "./support/basicOperations";

const VIEWPORT_MARGIN_PX = 16;

async function gotoReadyShell(page: import("@playwright/test").Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: t("timeline.conversation") })).toBeVisible();
}

async function openEmojiPicker(page: import("@playwright/test").Page): Promise<void> {
  const emojiButton = page.getByRole("button", { name: t("composer.emoji") });
  await expect(emojiButton).toBeVisible();
  await emojiButton.click();
  await expect(page.getByRole("dialog", { name: t("composer.emoji") })).toBeVisible();
}

test("a failed picker chunk closes the picker without unmounting the shell (#1035)", async ({
  page
}) => {
  await page.route(/\/src\/components\/EmojiPicker\.tsx(\?.*)?$/, (route) => route.abort());
  await gotoReadyShell(page);

  const emojiButton = page.getByRole("button", { name: t("composer.emoji") });
  await emojiButton.click();
  // The rejected lazy import is contained: the picker closes instead of
  // unmounting the shell, and the trigger is usable again.
  await expect(emojiButton).toHaveAttribute("aria-expanded", "false");
  await expect(page.getByRole("main", { name: t("timeline.conversation") })).toBeVisible();
  await expect(page.getByRole("dialog", { name: t("composer.emoji") })).toHaveCount(0);

  // Opening again fails the same way and closes again.
  await emojiButton.click();
  await expect(emojiButton).toHaveAttribute("aria-expanded", "false");
  await expect(page.getByRole("main", { name: t("timeline.conversation") })).toBeVisible();
  await expect(page.getByRole("dialog", { name: t("composer.emoji") })).toHaveCount(0);
});

async function openThreadPane(page: import("@playwright/test").Page): Promise<void> {
  await page.getByRole("button", { name: /2 replies/ }).click();
  await expect(page.getByText(t("panel.thread"), { exact: true })).toBeVisible();
}

async function openThreadEmojiPicker(page: import("@playwright/test").Page): Promise<void> {
  const threadPane = page.locator('aside[aria-label="Context panel"]');
  const emojiButton = threadPane.getByRole("button", { name: t("composer.emoji") });
  await expect(emojiButton).toBeVisible();
  await emojiButton.click();
  await expect(page.getByRole("dialog", { name: t("composer.emoji") })).toBeVisible();
}

/** Viewport-relative geometry of the open picker. */
async function pickerViewportMetrics(page: import("@playwright/test").Page) {
  return page.getByRole("dialog", { name: t("composer.emoji") }).evaluate((element) => {
    const rect = element.getBoundingClientRect();
    return {
      bottom: rect.bottom,
      documentScrollWidth: document.documentElement.scrollWidth,
      left: rect.left,
      right: rect.right,
      top: rect.top,
      viewportHeight: window.innerHeight,
      viewportWidth: window.innerWidth,
      width: rect.width
    };
  });
}

test("emoji button opens and closes the picker", async ({ page }) => {
  await gotoReadyShell(page);
  const emojiButton = page.getByRole("button", { name: t("composer.emoji") });
  await expect(emojiButton).toBeVisible();

  // Opens
  await emojiButton.click();
  await expect(page.getByRole("dialog", { name: t("composer.emoji") })).toBeVisible();

  // Toggle closes
  await emojiButton.click();
  await expect(page.getByRole("dialog", { name: t("composer.emoji") })).not.toBeVisible();
});

test("selecting an emoji by mouse inserts it into the composer draft", async ({ page }) => {
  await gotoReadyShell(page);
  await openEmojiPicker(page);

  const picker = page.getByRole("dialog", { name: t("composer.emoji") });
  // Click the first emoji button in the grid
  const firstEmoji = picker.locator(".emoji-picker-item").first();
  await expect(firstEmoji).toBeVisible();
  const emojiChar = await firstEmoji.textContent();

  await firstEmoji.click();

  // Picker closes after selection
  await expect(picker).not.toBeVisible();

  // The emoji appears in the composer textarea
  const composer = page.getByRole("textbox", { name: t("composer.messageComposer") });
  await expect(composer).toHaveText(emojiChar ?? "");
});

test("recent emoji selection is projected through Rust-owned composer settings", async ({ page }) => {
  await gotoReadyShell(page);
  await openEmojiPicker(page);
  const picker = page.getByRole("dialog", { name: t("composer.emoji") });
  const selected = await picker.locator(".emoji-picker-item").first().textContent();
  await picker.locator(".emoji-picker-item").first().click();

  await expect
    .poll(() =>
      page.evaluate(
        () => window.__harness.currentSnapshot().state.domain.settings.values.composer.recent_emojis
      )
    )
    .toEqual([selected]);

  await openEmojiPicker(page);
  await expect(page.getByRole("heading", { name: t("composer.emojiRecent") })).toBeVisible();
});

test("composer and remote-message reaction pickers share Rust-owned recent emojis", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const next = structuredClone(window.__harness.currentSnapshot());
    next.state.domain.settings.values.composer.recent_emojis = ["😀", "🙂"];
    window.__harness.setSnapshot(next);
    window.__harness.pushStateUpdate();
  });
  await seedTimelineItems(page, [
    {
      id: { Event: { event_id: "$remote-reaction:example.invalid" } },
      sender: "@remote-user:example.invalid",
      sender_label: "Remote User",
      body: "Remote reaction target",
      timestamp_ms: 1_800_000_000_000,
      in_reply_to_event_id: null,
      thread_root: null,
      thread_summary: null,
      can_react: true,
      is_redacted: false,
      is_hidden: false,
      can_redact: false,
      is_edited: false,
      can_edit: false,
      reactions: []
    }
  ]);

  await openEmojiPicker(page);
  await expect(page.getByRole("heading", { name: t("composer.emojiRecent") })).toBeVisible();
  await page.keyboard.press("Escape");

  const row = page.locator('[data-event-id="$remote-reaction:example.invalid"]');
  await row.hover();
  await row.getByRole("button", { name: t("timeline.addReaction") }).click();
  const picker = page.getByRole("dialog", { name: t("composer.emoji") });
  await expect(picker.getByRole("heading", { name: t("composer.emojiRecent") })).toBeVisible();

  await page.evaluate(() => window.__harness.clearInvocations());
  await picker
    .locator(".emoji-picker-section")
    .first()
    .getByRole("button", { name: "slightly smiling face" })
    .click();

  await expect
    .poll(() => page.evaluate(() => window.__harness.invocationsOf("update_settings")[0]?.args))
    .toEqual({
      patch: {
        composer: { math_mode: true, recent_emojis: ["🙂", "😀"] }
      }
    });
});

test("lazily loaded reaction picker owns focus, Escape, and outside-click dismissal", async ({ page }) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [
    {
      id: { Event: { event_id: "$lazy-reaction:example.invalid" } },
      sender: "@remote-user:example.invalid",
      sender_label: "Remote User",
      body: "Lazy reaction target",
      timestamp_ms: 1_800_000_000_000,
      in_reply_to_event_id: null,
      thread_root: null,
      thread_summary: null,
      can_react: true,
      is_redacted: false,
      is_hidden: false,
      can_redact: false,
      is_edited: false,
      can_edit: false,
      reactions: []
    }
  ]);
  const row = page.locator('[data-event-id="$lazy-reaction:example.invalid"]');
  const trigger = row.getByRole("button", { name: t("timeline.addReaction") });
  const picker = page.getByRole("dialog", { name: t("composer.emoji") });

  await row.hover();
  await trigger.click();
  await expect(picker.getByRole("searchbox")).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(picker).not.toBeVisible();
  await expect(trigger).toBeFocused();

  await row.hover();
  await trigger.click();
  await expect(picker).toBeVisible();
  await page.locator(".sidebar").click();
  await expect(picker).not.toBeVisible();
});

test("typing a search term filters the emoji grid", async ({ page }) => {
  await gotoReadyShell(page);
  await openEmojiPicker(page);

  const searchInput = page.getByRole("searchbox", { name: t("composer.emojiSearch") });
  await expect(searchInput).toBeVisible();

  await searchInput.fill("smile");

  // Tabs disappear while searching
  await expect(page.locator(".emoji-picker-tabs")).not.toBeVisible();

  // Grid shows filtered results
  const grid = page.locator(".emoji-picker-grid");
  await expect(grid).toBeVisible();
  await expect(grid.locator(".emoji-picker-item").first()).toBeVisible();
});

test("no results message shown for an unmatchable search", async ({ page }) => {
  await gotoReadyShell(page);
  await openEmojiPicker(page);

  const searchInput = page.getByRole("searchbox", { name: t("composer.emojiSearch") });
  // Use a string that cannot match any emoji label
  await searchInput.fill("xyzzy_no_match_expected");

  const noResults = page.locator(".emoji-picker-empty");
  await expect(noResults).toBeVisible();
  await expect(noResults).toContainText(t("emoji.noResults"));
});

test("Escape dismisses the picker", async ({ page }) => {
  await gotoReadyShell(page);
  await openEmojiPicker(page);

  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: t("composer.emoji") })).not.toBeVisible();
});

test("clicking outside the picker dismisses it", async ({ page }) => {
  await gotoReadyShell(page);
  await openEmojiPicker(page);

  // Click somewhere outside the picker — the room list sidebar is always present
  await page.locator(".sidebar").click();
  await expect(page.getByRole("dialog", { name: t("composer.emoji") })).not.toBeVisible();
});

test("picker does not make any network fetch request", async ({ page }) => {
  const networkRequests: string[] = [];
  page.on("request", (req) => {
    // The harness server at 127.0.0.1 / localhost is OK; flag any external URL
    const url = req.url();
    if (!url.startsWith("http://localhost") && !url.startsWith("http://127.0.0.1")) {
      networkRequests.push(url);
    }
  });

  await gotoReadyShell(page);
  await openEmojiPicker(page);

  // Interact with the picker
  const searchInput = page.getByRole("searchbox", { name: t("composer.emojiSearch") });
  await searchInput.fill("grin");
  await page.locator(".emoji-picker-item").first().click();

  expect(networkRequests).toHaveLength(0);
});

test("send button and emoji picker are simultaneously visible", async ({ page }) => {
  await gotoReadyShell(page);

  // Pre-fill the composer so the send button is enabled
  const composer = page.getByRole("textbox", { name: t("composer.messageComposer") });
  await composer.fill("hello");

  await openEmojiPicker(page);

  const sendButton = page.getByRole("button", { name: t("action.send"), exact: true });
  const picker = page.getByRole("dialog", { name: t("composer.emoji") });
  await expect(sendButton).toBeVisible();
  await expect(picker).toBeVisible();
});

test("picker uses available room without horizontal scrolling", async ({ page }) => {
  await page.setViewportSize({ width: 1000, height: 900 });
  await gotoReadyShell(page);
  await openEmojiPicker(page);

  const picker = page.getByRole("dialog", { name: t("composer.emoji") });
  const metrics = await picker.evaluate((element) => {
    const rect = element.getBoundingClientRect();
    const tabs = element.querySelector<HTMLElement>(".emoji-picker-tabs");
    const body = element.querySelector<HTMLElement>(".emoji-picker-body");
    return {
      bodyClientWidth: body?.clientWidth ?? 0,
      bodyScrollWidth: body?.scrollWidth ?? 0,
      height: rect.height,
      tabsClientWidth: tabs?.clientWidth ?? 0,
      tabsScrollWidth: tabs?.scrollWidth ?? 0,
      width: rect.width
    };
  });

  // #302 trades the former 420px panel for a denser, narrower grid.
  expect(metrics.width).toBe(380);
  expect(metrics.height).toBeGreaterThan(360);
  expect(metrics.tabsScrollWidth).toBeLessThanOrEqual(metrics.tabsClientWidth + 1);
  expect(metrics.bodyScrollWidth).toBeLessThanOrEqual(metrics.bodyClientWidth + 1);
});

test("main composer picker stays inside the viewport", async ({ page }) => {
  await gotoReadyShell(page);
  await openEmojiPicker(page);

  const metrics = await pickerViewportMetrics(page);
  expect(metrics.left).toBeGreaterThanOrEqual(VIEWPORT_MARGIN_PX - 1);
  expect(metrics.top).toBeGreaterThanOrEqual(VIEWPORT_MARGIN_PX - 1);
  expect(metrics.right).toBeLessThanOrEqual(metrics.viewportWidth - VIEWPORT_MARGIN_PX + 1);
  expect(metrics.bottom).toBeLessThanOrEqual(metrics.viewportHeight - VIEWPORT_MARGIN_PX + 1);
});

test("thread composer picker is not clipped by the thread pane", async ({ page }) => {
  await gotoReadyShell(page);
  await openThreadPane(page);
  await openThreadEmojiPicker(page);

  const metrics = await pickerViewportMetrics(page);
  expect(metrics.width).toBe(380);
  expect(metrics.left).toBeGreaterThanOrEqual(VIEWPORT_MARGIN_PX - 1);
  expect(metrics.top).toBeGreaterThanOrEqual(VIEWPORT_MARGIN_PX - 1);
  expect(metrics.right).toBeLessThanOrEqual(metrics.viewportWidth - VIEWPORT_MARGIN_PX + 1);
  expect(metrics.bottom).toBeLessThanOrEqual(metrics.viewportHeight - VIEWPORT_MARGIN_PX + 1);
  // The picker must not widen the document either.
  expect(metrics.documentScrollWidth).toBeLessThanOrEqual(metrics.viewportWidth);
});

test("thread composer picker flips and clamps at a narrow window width", async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 620 });
  await gotoReadyShell(page);
  await openThreadPane(page);
  await openThreadEmojiPicker(page);

  const metrics = await pickerViewportMetrics(page);
  expect(metrics.left).toBeGreaterThanOrEqual(VIEWPORT_MARGIN_PX - 1);
  expect(metrics.top).toBeGreaterThanOrEqual(VIEWPORT_MARGIN_PX - 1);
  expect(metrics.right).toBeLessThanOrEqual(metrics.viewportWidth - VIEWPORT_MARGIN_PX + 1);
  expect(metrics.bottom).toBeLessThanOrEqual(metrics.viewportHeight - VIEWPORT_MARGIN_PX + 1);
});

test("grid density matches the keyboard column step", async ({ page }) => {
  await gotoReadyShell(page);
  await openEmojiPicker(page);

  const picker = page.getByRole("dialog", { name: t("composer.emoji") });
  const grid = picker.locator(".emoji-picker-grid").first();
  const columns = await grid.evaluate(
    (element) => getComputedStyle(element).gridTemplateColumns.split(" ").filter(Boolean).length
  );
  expect(columns).toBe(10);

  const items = grid.locator(".emoji-picker-item");
  const firstItem = items.nth(0);
  const steppedItem = items.nth(columns);
  const steppedEmoji = await steppedItem.textContent();

  await firstItem.focus();
  await expect(firstItem).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(steppedItem).toBeFocused();

  await page.keyboard.press("Enter");
  const composer = page.getByRole("textbox", { name: t("composer.messageComposer") });
  await expect(composer).toHaveText(steppedEmoji ?? "");
});

test("arrow key navigation then Enter inserts the highlighted emoji", async ({ page }) => {
  await gotoReadyShell(page);
  await openEmojiPicker(page);

  const picker = page.getByRole("dialog", { name: t("composer.emoji") });

  // Read what the first and third emoji in the grid are before interacting
  const firstItem = picker.locator(".emoji-picker-item").nth(0);
  const thirdItem = picker.locator(".emoji-picker-item").nth(2);
  await expect(firstItem).toBeVisible();
  const thirdEmojiChar = await thirdItem.textContent();

  // Focus the first grid item via the Playwright focus method (no click,
  // so onSelect is not triggered).
  await firstItem.focus();
  await expect(firstItem).toBeFocused();

  // Move two positions to the right with arrow keys
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("ArrowRight");

  // Confirm the third item is focused
  await expect(thirdItem).toBeFocused();

  // Press Enter to select the focused emoji
  await page.keyboard.press("Enter");

  // Picker closes after selection
  await expect(picker).not.toBeVisible();

  // The emoji at position 2 appears in the composer textarea
  const composer = page.getByRole("textbox", { name: t("composer.messageComposer") });
  await expect(composer).toHaveText(thirdEmojiChar ?? "");
});
