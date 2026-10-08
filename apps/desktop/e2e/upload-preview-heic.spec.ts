import { expect, test } from "@playwright/test";

import { gotoReadyShell, invocationCount } from "./support/basicOperations";

/// #1147: an Original+Keep HEIC selection previews converted, renderable bytes.
///
/// Opted into the WebKit project as well as Chromium: the engine family used by
/// the macOS WKWebView is the one that decides whether the converted bytes render
/// while the item still carries its own HEIC MIME type.
test("an Original+Keep HEIC selection previews renderable bytes", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  // Synthetic upload bytes: this lane never decodes the upload payload, it checks
  // what the preview surface receives and whether the engine can render it.
  await page.getByRole("button", { name: "Attach file", exact: true }).click();
  await page.locator('input[type="file"][aria-label="Attach file input"]').setInputFiles({
    name: "camera.HEIC",
    mimeType: "image/heic",
    buffer: Buffer.from("synthetic heic payload")
  });

  const dialog = page.getByRole("dialog", { name: "Upload attachments" });
  await expect(dialog).toBeVisible();

  // The selection stays the exact original payload.
  await expect
    .poll(() =>
      page.evaluate(
        () => window.__harness.invocationsOf("stage_upload_bytes").at(-1)?.args.items[0].mimeType
      )
    )
    .toBe("image/heic");
  await expect(dialog).toContainText("camera.HEIC");

  await expect.poll(() => invocationCount(page, "prepared_upload_preview")).toBeGreaterThanOrEqual(1);
  const preview = dialog.locator("img.upload-staging-preview");
  await expect(preview).toHaveCount(1);
  await expect
    .poll(() => preview.evaluate((node) => (node as HTMLImageElement).naturalWidth))
    .toBeGreaterThan(0);
  await expect
    .poll(() => preview.evaluate((node) => (node as HTMLImageElement).naturalHeight))
    .toBeGreaterThan(0);
});
