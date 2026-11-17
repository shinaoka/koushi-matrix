/**
 * Headless spec: the composer's formatting shortcuts (#1949).
 *
 * The Bold and Italic toolbar buttons inserted their inline markdown, but the
 * Element-compatible keys behind them (Cmd+B / Cmd+I, Ctrl on Windows and Linux)
 * did nothing: the composer key resolver only handled Enter and Escape, and the
 * settings shortcut list advertised the pair anyway. This drives the real
 * composer with the platform modifier and asks for the same result the buttons
 * produce.
 */

import { expect, test } from "@playwright/test";
import { gotoReadyShell } from "./support/basicOperations";

// Playwright's `ControlOrMeta` sends Cmd on macOS and Ctrl elsewhere, which is
// exactly the split the composer implements (macOS keeps Ctrl for AppKit's
// text-editing bindings).
const MOD = "ControlOrMeta";

test("Cmd/Ctrl+B and Cmd/Ctrl+I format the composer like the toolbar", async ({ page }) => {
  await gotoReadyShell(page);
  const editor = page.locator(".composer-inline-editor").first();
  await editor.click();
  await editor.pressSequentially("world");

  await editor.press(`${MOD}+a`);
  await editor.press(`${MOD}+b`);
  await expect(editor).toHaveText("**world**");

  await editor.press(`${MOD}+a`);
  await editor.press(`${MOD}+i`);
  await expect(editor).toHaveText("_**world**_");

  // The same selection through the toolbar button, to keep the two paths pinned
  // to one result as the composer changes.
  await editor.press(`${MOD}+a`);
  await page.getByRole("button", { name: "Bold", exact: true }).click();
  await expect(editor).toHaveText("**_**world**_**");
});
