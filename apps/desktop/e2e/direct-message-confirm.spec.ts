/**
 * Headless spec: starting a direct message from a person asks first (#1702).
 *
 * People list "Message" and the Profile "Send message" button ask Core whether
 * a direct chat exists (`createIfMissing: false`). An existing chat opens with
 * no dialog. A missing one shows a confirmation; only "Start chat" sends the
 * command again with `createIfMissing: true`, exactly once. The New DM dialog,
 * where the user types the Matrix ID, still creates directly.
 */

import { expect, test, type Page } from "@playwright/test";
import { t } from "../src/i18n/messages";

const MEMBER = { label: "Harness Grace", userId: "@harness-grace:example.invalid" } as const;

type StartArgs = { userId: string; createIfMissing?: boolean };

interface Harness {
  invocationsOf(command: string): { args: unknown }[];
  clearInvocations(): void;
  currentSnapshot(): Record<string, unknown>;
  setCommandResponse(command: string, handler: (args: StartArgs) => unknown): void;
}

async function gotoReadyShell(page: Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: t("timeline.conversation") })).toBeVisible();
}

/** Core reports `missing` unless the call asks to create, as Rust's policy does. */
async function respondWithoutExistingChat(page: Page): Promise<void> {
  await page.evaluate(() => {
    const harness = (window as unknown as { __harness: Harness }).__harness;
    harness.setCommandResponse("start_direct_message", (args) => ({
      ...harness.currentSnapshot(),
      status: args.createIfMissing ? "opened" : "missing"
    }));
  });
}

async function respondWithExistingChat(page: Page): Promise<void> {
  await page.evaluate(() => {
    const harness = (window as unknown as { __harness: Harness }).__harness;
    harness.setCommandResponse("start_direct_message", () => ({
      ...harness.currentSnapshot(),
      status: "opened"
    }));
  });
}

async function startCalls(page: Page): Promise<StartArgs[]> {
  return page.evaluate(() =>
    (window as unknown as { __harness: Harness })
      .__harness.invocationsOf("start_direct_message")
      .map((call) => call.args as StartArgs)
  );
}

async function clickPeopleMessage(page: Page): Promise<void> {
  await page.locator(".channel-actions").getByRole("button", { name: t("panel.people") }).click();
  const panel = page.getByRole("complementary", { name: t("panel.context") });
  await panel
    .getByRole("list", { name: t("room.members") })
    .getByRole("button", { name: t("room.messageMember", { name: MEMBER.label }) })
    .click();
}

function confirmDialog(page: Page) {
  return page.getByRole("dialog", { name: t("room.startDirectMessageConfirmTitle") });
}

test("a person without a direct chat is asked about before anything is created", async ({
  page
}) => {
  await gotoReadyShell(page);
  await respondWithoutExistingChat(page);
  await page.evaluate(() => (window as unknown as { __harness: Harness }).__harness.clearInvocations());

  await clickPeopleMessage(page);

  const dialog = confirmDialog(page);
  await expect(dialog).toBeVisible();
  expect(await startCalls(page)).toEqual([{ userId: MEMBER.userId, createIfMissing: false }]);
  await expect(dialog).toContainText(
    t("room.startDirectMessageConfirmBody", { name: MEMBER.label, userId: MEMBER.userId })
  );
  await expect(dialog.getByRole("button", { name: t("action.cancel") })).toBeFocused();
  await dialog.screenshot({ path: "test-results/direct-message-confirm-dialog.png" });

  // Cancel creates nothing.
  await dialog.getByRole("button", { name: t("action.cancel") }).click();
  await expect(dialog).toBeHidden();
  expect(await startCalls(page)).toEqual([{ userId: MEMBER.userId, createIfMissing: false }]);

  // Asking again and confirming creates exactly one, even on a double click.
  await page.evaluate(() => (window as unknown as { __harness: Harness }).__harness.clearInvocations());
  await clickPeopleMessage(page);
  await expect(dialog).toBeVisible();
  await dialog
    .getByRole("button", { name: t("room.startDirectMessageConfirmAction") })
    .dblclick();
  await expect(dialog).toBeHidden();
  expect(await startCalls(page)).toEqual([
    { userId: MEMBER.userId, createIfMissing: false },
    { userId: MEMBER.userId, createIfMissing: true }
  ]);
});

test("an existing direct chat opens on a single click with no dialog", async ({ page }) => {
  await gotoReadyShell(page);
  await respondWithExistingChat(page);
  await page.evaluate(() => (window as unknown as { __harness: Harness }).__harness.clearInvocations());

  await clickPeopleMessage(page);

  await expect.poll(() => startCalls(page)).toEqual([
    { userId: MEMBER.userId, createIfMissing: false }
  ]);
  await expect(confirmDialog(page)).toHaveCount(0);
});

test("the New DM dialog still creates directly", async ({ page }) => {
  await gotoReadyShell(page);
  await respondWithoutExistingChat(page);
  await page
    .getByRole("navigation", { name: "Workspaces" })
    .getByRole("button", { name: /^Home/ })
    .click();
  await expect(page.getByRole("main", { name: "Activity" })).toBeVisible();
  await page.getByRole("button", { name: "Invites", exact: true }).click();
  await page.getByRole("main", { name: "Invites" }).getByRole("button", { name: "New DM" }).click();
  await page.getByRole("textbox", { name: "Matrix user ID" }).fill("@target:example.invalid");
  await page.evaluate(() => (window as unknown as { __harness: Harness }).__harness.clearInvocations());
  await page.getByRole("button", { name: "Start DM" }).click();

  await expect.poll(() => startCalls(page)).toEqual([
    { userId: "@target:example.invalid", createIfMissing: true }
  ]);
  await expect(confirmDialog(page)).toHaveCount(0);
});
