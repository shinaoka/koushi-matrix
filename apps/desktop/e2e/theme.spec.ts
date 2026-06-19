import { expect, test } from "@playwright/test";

async function railBackground(page: import("@playwright/test").Page): Promise<string> {
  return page.evaluate(() => {
    const rail = document.querySelector(".workspace-rail");
    return rail ? getComputedStyle(rail).backgroundColor : "";
  });
}

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

test("explicit Rust-owned theme selection sets the root data-theme", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await page.goto("/appHarness.html");
  await expect(page.getByRole("navigation", { name: "Workspaces" })).toBeVisible();
  await expect.poll(() => page.evaluate(() => document.documentElement.dataset.theme)).toBe(
    undefined
  );

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          settings: {
            ...snapshot.state.domain.settings,
            values: {
              ...snapshot.state.domain.settings.values,
              appearance: { theme: "dark" }
            }
          }
        }
      }
    });
    window.__harness.pushStateChanged();
  });
  await expect.poll(() => page.evaluate(() => document.documentElement.dataset.theme)).toBe(
    "dark"
  );
  await expect
    .poll(() => page.evaluate(() => getComputedStyle(document.documentElement).colorScheme))
    .toBe("dark");

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          settings: {
            ...snapshot.state.domain.settings,
            values: {
              ...snapshot.state.domain.settings.values,
              appearance: { theme: "light" }
            }
          }
        }
      }
    });
    window.__harness.pushStateChanged();
  });
  await expect.poll(() => page.evaluate(() => document.documentElement.dataset.theme)).toBe(
    "light"
  );
  await expect
    .poll(() => page.evaluate(() => getComputedStyle(document.documentElement).colorScheme))
    .toBe("light");
});
