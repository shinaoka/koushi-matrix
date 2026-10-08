import { expect, test } from "@playwright/test";
import {
  focusRailButton,
  railButtonState,
  seedSelectionRail
} from "./support/railSelection";

async function focusedLabel(page: import("@playwright/test").Page): Promise<string> {
  return page.evaluate(() => {
    const element = document.activeElement as HTMLElement | null;
    if (!element) {
      return "";
    }
    return (
      element.getAttribute("aria-label") ||
      element.getAttribute("data-testid") ||
      element.textContent?.trim() ||
      element.tagName.toLowerCase()
    );
  });
}

test("the three-pane shell exposes landmarks and reachable keyboard focus stops", async ({
  page
}) => {
  await page.goto("/appHarness.html");

  await expect(page.getByRole("navigation", { name: "Workspaces" })).toBeVisible();
  await expect(page.getByRole("complementary", { name: "Rooms" })).toBeVisible();
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  await page.getByRole("button", { name: "Space info and settings" }).click();
  await expect(page.getByRole("complementary", { name: "Context panel" })).toBeVisible();

  const labels: string[] = [];
  for (let index = 0; index < 100; index += 1) {
    await page.keyboard.press("Tab");
    labels.push(await focusedLabel(page));
  }

  expect(labels).toContain("Search");
  expect(labels).toContain("Search scope");
  expect(labels).not.toContain("Keyboard settings");
  expect(labels).toContain("Harness Space");
  expect(labels).toContain("Create space");
  expect(labels).toContain("Account Settings");
  expect(labels).toContain("App Settings");
  expect(labels).toContain("Message composer");
});

test("a focused rail button keeps a non-shadow focus indicator in forced colors", async ({
  page
}) => {
  await page.emulateMedia({ forcedColors: "active" });
  await page.goto("/appHarness.html");
  await expect(page.getByRole("navigation", { name: "Workspaces" })).toBeVisible();
  await seedSelectionRail(page);

  // Forced-colors mode drops box-shadows, so the focus ring must not depend on
  // `.workspace-button:focus-visible`'s `box-shadow`. Both the plain and the
  // `.is-active` case have to expose an outline fallback.
  for (const name of ["Idle Space", "Active Space"]) {
    await focusRailButton(page, name);
    const focused = await railButtonState(page, name);
    expect(focused.boxShadow).toBe("none");
    expect(focused.outlineStyle).not.toBe("none");
    expect(focused.outlineWidth).not.toBe("0px");
  }
});
