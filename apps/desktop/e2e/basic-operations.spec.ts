/**
 * Headless IPC-contract tier (QA Model layer 4): proves the Task 5 UI drives
 * the right Tauri COMMAND NAMES. Runs the FULL <App /> over a recording mock
 * Tauri IPC transport (appHarness.html → src/test/appHarnessMain.tsx, which
 * installs window.__TAURI_INTERNALS__ via Tauri's official mockIPC so the App
 * selects TauriDesktopApi → invoke()).
 *
 * No Tauri process, no native window, no network: headless Chromium only.
 *
 * Scenarios:
 *   1. Open create-room dialog → submit → invokes `create_room`; dialog closes.
 *   2. Open create-space dialog → submit → invokes `create_space`; dialog closes.
 *   3. Click a timeline row's reply action → invokes `set_composer_reply_target`.
 *   4. Submit the composer while the snapshot says reply mode → invokes
 *      `send_reply` (not `send_text`). Reply mode is established by scenario 3's
 *      flow (the set_composer_reply_target response returns a reply-mode
 *      snapshot), then the composer is submitted.
 */

import { expect, test, type Page } from "@playwright/test";

async function gotoReadyShell(page: Page): Promise<void> {
  await page.goto("/appHarness.html");
  // The signed-in shell renders the three panes (not the AuthScreen).
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  // Wait for the seeded timeline row's reply action (proves the CoreEvent
  // stream + full App are wired) before clearing startup invocations.
  await expect(page.getByRole("button", { name: "Reply to message" }).first()).toBeVisible();
}

async function invocationCount(page: Page, command: string): Promise<number> {
  return page.evaluate((cmd) => window.__harness.invocationsOf(cmd).length, command);
}

test("create-room dialog submits create_room and closes on success", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.getByRole("button", { name: "Create room", exact: true }).click();
  const roomNameInput = page.getByRole("textbox", { name: "Room name" });
  await expect(roomNameInput).toBeVisible();

  await roomNameInput.fill("My New Room");
  await page.getByRole("button", { name: "Submit create room" }).click();

  // create_room was invoked.
  await expect.poll(() => invocationCount(page, "create_room")).toBeGreaterThanOrEqual(1);
  // Dialog closed on success (the name input is gone).
  await expect(roomNameInput).toBeHidden();
});

test("create-space dialog submits create_space and closes on success", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.getByRole("button", { name: "Create space", exact: true }).click();
  const spaceNameInput = page.getByRole("textbox", { name: "Space name" });
  await expect(spaceNameInput).toBeVisible();

  await spaceNameInput.fill("My New Space");
  await page.getByRole("button", { name: "Submit create space" }).click();

  await expect.poll(() => invocationCount(page, "create_space")).toBeGreaterThanOrEqual(1);
  await expect(spaceNameInput).toBeHidden();
});

test("timeline reply action invokes set_composer_reply_target", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.getByRole("button", { name: "Reply to message" }).first().click();

  await expect
    .poll(() => invocationCount(page, "set_composer_reply_target"))
    .toBeGreaterThanOrEqual(1);
  // The reply-mode snapshot returned by set_composer_reply_target surfaces the
  // composer reply banner (Cancel reply control), confirming reply mode.
  await expect(page.getByRole("button", { name: "Cancel reply" })).toBeVisible();
});

test("submitting the composer in reply mode invokes send_reply, not send_text", async ({
  page
}) => {
  await gotoReadyShell(page);

  // Establish reply mode via the reply action (its response snapshot puts the
  // composer into reply mode), then submit the composer.
  await page.getByRole("button", { name: "Reply to message" }).first().click();
  await expect(page.getByRole("button", { name: "Cancel reply" })).toBeVisible();

  await page.evaluate(() => window.__harness.clearInvocations());

  const composer = page.getByRole("textbox", { name: "Message composer" });
  await composer.fill("A reply body");
  await page.getByRole("button", { name: "Send" }).click();

  await expect.poll(() => invocationCount(page, "send_reply")).toBeGreaterThanOrEqual(1);
  expect(await invocationCount(page, "send_text")).toBe(0);
});
