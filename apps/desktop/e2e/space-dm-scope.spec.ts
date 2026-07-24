import { expect, test } from "@playwright/test";
import { t } from "../src/i18n/messages";

test("active space sidebar hides direct messages outside that space", async ({ page }) => {
  await page.goto("/");

  await expect(page.getByRole("complementary", { name: t("workspace.rooms") })).toBeVisible();
  const peopleSection = page.locator('[data-room-section="people"]');
  await page
    .getByRole("navigation", { name: t("workspace.workspaces") })
    .getByRole("button", { name: "Home" })
    .click();
  await expect(page.getByRole("button", { name: "DMs, 1 unread, 2 total" })).toBeVisible();
  await page.getByRole("button", { name: "DMs, 1 unread, 2 total" }).click();
  await expect(peopleSection.getByRole("button", { name: "Member 1" })).toBeVisible();
  await expect(peopleSection.getByRole("button", { name: "Member 2" })).toBeVisible();

  await page
    .getByRole("navigation", { name: t("workspace.workspaces") })
    .getByRole("button", { name: "Synthetic Lab" })
    .click();

  await page.getByRole("button", { name: /^Rooms,/ }).click();
  await expect(page.getByRole("button", { name: "matrix-sdk-search" })).toBeVisible();
  await expect(page.getByRole("button", { name: "DMs, 0 unread, 0 total" })).toBeVisible();
  await page.getByRole("button", { name: "DMs, 0 unread, 0 total" }).click();
  await expect(peopleSection.getByRole("button", { name: "Member 1" })).toHaveCount(0);
  await expect(peopleSection.getByRole("button", { name: "Member 2" })).toHaveCount(0);
});
