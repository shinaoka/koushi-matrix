import { expect, test } from "@playwright/test";

async function railBackground(page: import("@playwright/test").Page): Promise<string> {
  return page.evaluate(() => {
    const rail = document.querySelector(".workspace-rail");
    return rail ? getComputedStyle(rail).backgroundColor : "";
  });
}

// The app sets no explicit data-theme yet (an explicit user choice is deferred to
// Rust SettingsState, Issue #6), so the dark token set must be driven purely by
// the OS color scheme via @media (prefers-color-scheme: dark).
test("the space rail follows the OS color scheme", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await page.goto("/");
  await expect(page.getByRole("navigation", { name: "Workspaces" })).toBeVisible();
  const light = await railBackground(page);

  await page.emulateMedia({ colorScheme: "dark" });
  const dark = await railBackground(page);

  // --rail is #16213E light, #0A111F dark.
  expect(light).toBe("rgb(22, 33, 62)");
  expect(dark).toBe("rgb(10, 17, 31)");
  expect(light).not.toBe(dark);
});
