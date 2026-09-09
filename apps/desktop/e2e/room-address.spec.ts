import { expect, test } from "@playwright/test";
import { gotoReadyShell, invocationCount } from "./support/basicOperations";

test("public address preserves manual edits and collision drafts until successful retry", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const suggestions: Record<string, string> = { "Example Room": "example-room", "Changed Name": "changed-name" };
    window.__harness.setCommandResponse("preview_room_address", ({ name, aliasLocalpart }) => {
      const localpart = aliasLocalpart ?? suggestions[name] ?? "";
      return { localpart, full_alias: localpart ? `#${localpart}:example.invalid` : null, error: localpart ? null : "empty" };
    });
    window.__harness.setCommandResponse("create_room", () => { throw { kind: "aliasInUse" }; });
    window.__harness.clearInvocations();
  });
  await page.getByRole("button", { name: "Create room", exact: true }).click();
  const name = page.getByRole("textbox", { name: "Room name" });
  await name.fill("Example Room");
  await page.getByRole("radio", { name: "Public room", exact: true }).check();
  const address = page.getByRole("textbox", { name: "Room address" });
  await expect(address).toHaveValue("example-room");
  await address.fill("manual");
  await name.fill("Changed Name");
  await expect(address).toHaveValue("manual");
  await page.getByRole("radio", { name: "Private room", exact: true }).check();
  await page.getByRole("radio", { name: "Public room", exact: true }).check();
  await expect(address).toHaveValue("manual");
  await expect(page.getByRole("status").filter({ hasText: "Full address:" })).toHaveText("Full address: #manual:example.invalid");
  await address.focus();
  await address.evaluate((input) => {
    input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", code: "Enter", keyCode: 229, isComposing: true, bubbles: true }));
    input.closest("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  });
  expect(await invocationCount(page, "create_room")).toBe(0);
  await address.dispatchEvent("compositionend", { data: "manual" });
  await address.dispatchEvent("keyup", { key: "Enter", code: "Enter" });
  await page.getByRole("button", { name: "Submit create room" }).click();
  await expect(page.getByRole("alert").filter({ hasText: "already in use" })).toBeVisible();
  await expect(name).toHaveValue("Changed Name");
  await expect(address).toHaveValue("manual");
  await address.fill("available");
  await page.evaluate(() => window.__harness.setCommandResponse("create_room", () => window.__harness.currentSnapshot()));
  await page.getByRole("button", { name: "Submit create room" }).click();
  await expect(name).toBeHidden();
  expect(await invocationCount(page, "create_room")).toBe(2);
});
