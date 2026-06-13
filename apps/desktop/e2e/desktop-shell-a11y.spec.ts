import { expect, test } from "@playwright/test";

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
  await page.goto("/");

  await expect(page.getByRole("navigation", { name: "Workspaces" })).toBeVisible();
  await expect(page.getByRole("complementary", { name: "Rooms" })).toBeVisible();
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  await expect(page.getByRole("complementary", { name: "Context panel" })).toBeVisible();

  const labels: string[] = [];
  for (let index = 0; index < 40; index += 1) {
    await page.keyboard.press("Tab");
    labels.push(await focusedLabel(page));
  }

  expect(labels).toContain("Search");
  expect(labels).toContain("Search scope");
  expect(labels).toContain("Keyboard settings");
  expect(labels).toContain("Synthetic Workspace");
  expect(labels).toContain("Synthetic Lab");
  expect(labels).toContain("Add workspace");
  expect(labels).toContain("User settings");
  expect(labels).toContain("Space info and settings");
  expect(labels).toContain("Message composer");
});
