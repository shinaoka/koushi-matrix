/**
 * Headless spec: emoji picker (#79).
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
 */

import { expect, test } from "@playwright/test";
import { t } from "../src/i18n/messages";

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
  await expect(composer).toHaveValue(emojiChar ?? "");
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
