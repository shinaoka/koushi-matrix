import { expect, test, type Page } from "@playwright/test";
import {
  focusRailButton,
  homeButtonState,
  railButtonState,
  seedSelectionRail
} from "./support/railSelection";

async function railBackground(page: Page): Promise<string> {
  return page.evaluate(() => {
    const rail = document.querySelector(".workspace-rail");
    return rail ? getComputedStyle(rail).backgroundColor : "";
  });
}

test("the space rail follows the OS color scheme", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await page.goto("/appHarness.html");
  await expect(page.getByRole("navigation", { name: "Workspaces" })).toBeVisible();
  const light = await railBackground(page);

  await page.emulateMedia({ colorScheme: "dark" });
  const dark = await railBackground(page);

  // --rail is #f7f8fa light, #151719 dark.
  expect(light).toBe("rgb(247, 248, 250)");
  expect(dark).toBe("rgb(21, 23, 25)");
  expect(light).not.toBe(dark);
});

// #1206: the selected destination must be distinguishable from an unselected
// one and from the transient hover/focus states in every color scheme, so the
// theme spec owns the rendered light/dark comparison.
for (const colorScheme of ["light", "dark"] as const) {
  test(`the selected Space differs from hover and focus in ${colorScheme}`, async ({ page }) => {
    await page.emulateMedia({ colorScheme });
    await page.goto("/appHarness.html");
    await expect(page.getByRole("navigation", { name: "Workspaces" })).toBeVisible();
    await seedSelectionRail(page);

    const selected = await railButtonState(page, "Active Space");
    const idle = await railButtonState(page, "Idle Space");
    await page.getByRole("button", { name: "Idle Space" }).hover();
    const hovered = await railButtonState(page, "Idle Space");
    await focusRailButton(page, "Idle Space");
    const focused = await railButtonState(page, "Idle Space");

    // Non-forced-colors keeps the box-shadow indicator; the forced-colors
    // fallback is covered by the a11y spec.
    for (const state of [selected, idle, hovered, focused]) {
      expect(state.outlineStyle).toBe("none");
    }
    expect(selected.backgroundColor).not.toBe(idle.backgroundColor);
    expect(selected.boxShadow).not.toBe(idle.boxShadow);
    expect([hovered.backgroundColor, hovered.boxShadow]).not.toEqual([
      selected.backgroundColor,
      selected.boxShadow
    ]);
    expect([focused.backgroundColor, focused.boxShadow]).not.toEqual([
      selected.backgroundColor,
      selected.boxShadow
    ]);
    expect([focused.backgroundColor, focused.boxShadow]).not.toEqual([
      hovered.backgroundColor,
      hovered.boxShadow
    ]);
  });
}

test("Home reuses the selected convention when it is active", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await page.goto("/appHarness.html");
  await expect(page.getByRole("navigation", { name: "Workspaces" })).toBeVisible();

  await expect(page.locator(".workspace-home-button")).toHaveAttribute(
    "aria-current",
    "page"
  );
  const homeActive = await homeButtonState(page);

  await seedSelectionRail(page);
  const homeInactive = await homeButtonState(page);
  const spaceActive = await railButtonState(page, "Active Space");

  // Both active destinations mark selection with an inset ring over their own
  // selected background; clearing `is_active` removes the marker again.
  expect(homeActive.boxShadow).toContain("inset");
  expect(spaceActive.boxShadow).toContain("inset");
  expect(homeInactive.boxShadow).not.toContain("inset");
  expect(homeActive.backgroundColor).not.toBe(homeInactive.backgroundColor);
  expect(spaceActive.backgroundColor).not.toBe(homeInactive.backgroundColor);
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
    window.__harness.pushStateUpdate();
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
    window.__harness.pushStateUpdate();
  });
  await expect.poll(() => page.evaluate(() => document.documentElement.dataset.theme)).toBe(
    "light"
  );
  await expect
    .poll(() => page.evaluate(() => getComputedStyle(document.documentElement).colorScheme))
    .toBe("light");
});
