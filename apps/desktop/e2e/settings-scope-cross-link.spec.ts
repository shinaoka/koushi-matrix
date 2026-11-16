/**
 * Headless spec: the settings cross-link at the bottom of the category list (#1921).
 *
 * The link is disabled when App Settings has no current session. `AppSettingsDialog`
 * — the only mount of App Settings in the shipped app — hard-coded
 * `currentSession={null}` even though its snapshot holds the signed-in session, so
 * the gate was on in every mount while the Search history page's own Account
 * Settings button next to it switched scope. This drives the dialog the way the
 * gear does and asserts the nav link acts.
 */

import { expect, test } from "@playwright/test";
import { t } from "../src/i18n/messages";

test("App Settings cross-link reaches Account Settings (#1921)", async ({ page }) => {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: t("timeline.conversation") })).toBeVisible();

  await page.getByRole("button", { name: t("settings.appSettings"), exact: true }).click();
  const appSettings = page.getByRole("dialog", { name: t("settings.appSettings"), exact: true });
  await expect(appSettings).toBeVisible();

  const toAccount = appSettings.locator("button.settings-scope-switch");
  await expect(toAccount).toHaveText(t("settings.accountSettings"));
  await expect(toAccount).toBeEnabled();
  await toAccount.click();

  const accountSettings = page.getByRole("dialog", {
    name: t("settings.accountSettings"),
    exact: true
  });
  await expect(accountSettings).toBeVisible();
});
