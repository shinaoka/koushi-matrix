import { expect, test, type Page } from "@playwright/test";
import { gotoReadyShell } from "./support/basicOperations";

/**
 * #1871 "Migrate to Koushi", browser tier: the route order the owner chose is
 * what a user reads — key file first and marked, the recovery key second, the
 * Element Desktop import only when the adapter enables it, device verification
 * last with its known stall named, and the "nothing recoverable" exit with the
 * Element Web link. Detection comes from the adapter command, so this stubs it
 * instead of touching a real Element installation.
 */

async function openEncryptionSettings(page: Page): Promise<void> {
  await page.getByRole("button", { name: "Account Settings" }).click();
  await page.getByRole("tab", { name: "Encryption", exact: true }).click();
}

async function stubDetection(page: Page, element: Record<string, unknown>): Promise<void> {
  await page.evaluate((desktop) => {
    window.__harness.setCommandResponse("migration_detect_sources", {
      element_x: { sessions: 0 },
      element_desktop: desktop,
      unsupported_reason: null
    });
  }, element);
  await page.evaluate(() => window.__harness.pushStateUpdate());
}

const INSTALLED = {
  installed: true,
  running: false,
  profile_hint: "Element",
  import_supported: true,
  import_enabled: true,
  unavailable_reason: null
};

test("the migration routes read in the order that works, key file first", async ({ page }) => {
  await gotoReadyShell(page);
  await stubDetection(page, INSTALLED);
  await openEncryptionSettings(page);

  const section = page.locator("[data-migration-section]");
  await expect(section).toBeVisible();
  await expect(section.getByRole("heading", { name: "Migrate to Koushi" })).toBeVisible();

  const order = await page
    .locator("[data-migration-route]")
    .evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-migration-route")));
  expect(order).toEqual([
    "key-file",
    "recovery-key",
    "element-desktop",
    "verify-device",
    "nothing"
  ]);

  // The first route is the one the screen recommends, and it is the import the
  // Security section already runs — same form, same status row.
  const keyFile = page.locator('[data-migration-route="key-file"]');
  await expect(keyFile.getByText("most reliable")).toBeVisible();
  await expect(keyFile.getByText("1. Import an exported key file")).toBeVisible();
  await expect(keyFile.getByTestId("migrate-key-file-state")).toBeVisible();
  await expect(page.getByRole("heading", { name: "Security" })).toBeVisible();

  // Device verification is last and says it can stall, with a way back to 1/2.
  const verify = page.locator('[data-migration-route="verify-device"]');
  await expect(verify.getByText(/known to stall/i)).toBeVisible();
  await expect(verify.getByRole("button", { name: "Go to route 2" })).toBeVisible();

  // Detection reported a source, so its route is offered.
  await expect(page.locator('[data-migration-route="element-desktop"]')).toBeVisible();
  await expect(page.locator("[data-migration-detection]")).toContainText(
    "Element Desktop is installed on this computer."
  );
});

test("a build with the Element import turned off hides that route and says why", async ({ page }) => {
  await gotoReadyShell(page);
  await stubDetection(page, {
    ...INSTALLED,
    running: true,
    import_enabled: false,
    unavailable_reason: "migration_element_desktop_import_disabled"
  });
  await openEncryptionSettings(page);

  await expect(page.locator("[data-migration-section]")).toBeVisible();
  await expect(page.locator('[data-migration-route="element-desktop"]')).toHaveCount(0);
  await expect(page.locator("[data-migration-detection]")).toContainText(
    "turned off in this build"
  );
  // The routes that need no other app stay.
  await expect(page.locator('[data-migration-route="key-file"]')).toBeVisible();
  await expect(page.locator('[data-migration-route="recovery-key"]')).toBeVisible();
});

test("the nothing-recoverable exit states the loss and offers Element Web", async ({ page }) => {
  await gotoReadyShell(page);
  await stubDetection(page, { ...INSTALLED, installed: false, import_supported: false });
  await openEncryptionSettings(page);

  const nothing = page.locator('[data-migration-route="nothing"]');
  await expect(nothing).toBeVisible();
  await expect(nothing).toContainText("stays unreadable");
  await expect(nothing).toContainText("future messages only");
  await expect(
    nothing.getByRole("button", { name: "Open Element Web security settings" })
  ).toBeVisible();
  // A backup that is already set up offers no setup button: the section it
  // would scroll to has no form, so the action would do nothing (#1871 review).
  await expect(
    nothing.getByRole("button", { name: "Create a new secure backup" })
  ).toHaveCount(0);
});
