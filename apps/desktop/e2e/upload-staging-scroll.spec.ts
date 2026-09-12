import { expect, test, type Page } from "@playwright/test";
import { t } from "../src/i18n/messages";

/** Short viewport representative of the reported packaged window. */
const SHORT_VIEWPORT = { width: 900, height: 520 };
/** Ordinary desktop viewport where attachment controls must be visible immediately. */
const STANDARD_VIEWPORT = { width: 1200, height: 800 };
/** Mid-height window where a tall card forces the staging list to scroll. */
const MID_VIEWPORT = { width: 1000, height: 640 };

/** A small portrait image (240x640) so the preview keeps a tall aspect. */
const PORTRAIT_PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAPAAAAKACAIAAAAtimItAAAGsUlEQVR4nO3SUQkAIBTAwBfbOAYzjCUEYRxcgH1s1j6QMd8L4CFDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphibF0KQYmhRDk2JoUgxNiqFJMTQphiblArUiJK23gEvYAAAAAElFTkSuQmCC",
  "base64"
);

async function gotoReadyShell(page: Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Reply to message" }).first()).toBeVisible();
}

function stagedPortraitImage() {
  return {
    name: "portrait.png",
    mimeType: "image/png",
    buffer: PORTRAIT_PNG
  };
}

/** Rendered geometry of the staging dialog, its scroll list, and its actions. */
async function stagingGeometry(page: Page) {
  return page.evaluate(() => {
    const dialog = document.querySelector<HTMLElement>(".upload-staging-dialog");
    if (!dialog) {
      return null;
    }
    const box = dialog.getBoundingClientRect();
    const list = dialog.querySelector<HTMLElement>(".upload-staging-list");
    const actions = Array.from(
      dialog.querySelectorAll<HTMLElement>(".upload-staging-actions button")
    );
    const actionRects = actions.map((action) => action.getBoundingClientRect());
    return {
      viewportHeight: window.innerHeight,
      top: box.top,
      bottom: box.bottom,
      list: list
        ? {
            height: list.clientHeight,
            scrollHeight: list.scrollHeight,
            scrollTop: list.scrollTop,
            overflow: getComputedStyle(list).overflowY
          }
        : null,
      actions: actionRects.map((rect) => ({ top: rect.top, bottom: rect.bottom })),
      pageScrollY: window.scrollY
    };
  });
}

function expectStagingBounded(
  geometry: NonNullable<Awaited<ReturnType<typeof stagingGeometry>>>,
  label: string
) {
  const { viewportHeight, top, bottom, actions } = geometry;
  expect(top, `${label}: dialog top ${top} escapes the viewport`).toBeGreaterThanOrEqual(0);
  expect(
    bottom,
    `${label}: dialog bottom ${bottom} exceeds viewport height ${viewportHeight}`
  ).toBeLessThanOrEqual(viewportHeight + 1);
  for (const [index, action] of actions.entries()) {
    expect(
      action.bottom,
      `${label}: action ${index} bottom ${action.bottom} exceeds viewport height ${viewportHeight}`
    ).toBeLessThanOrEqual(viewportHeight + 1);
  }
}

for (const surface of ["main", "thread"] as const) {
  for (const viewport of [STANDARD_VIEWPORT, MID_VIEWPORT, SHORT_VIEWPORT]) {
    test(`${surface} attachment dialog fills the window and fits the whole image at ${viewport.height}px`, async ({ page }) => {
      await page.setViewportSize(viewport);
      await gotoReadyShell(page);
      if (surface === "thread") {
        await page.getByRole("button", { name: /2 replies/ }).click();
      }
      const pane = surface === "thread" ? page.locator('aside[aria-label="Context panel"]') : page;
      await pane.getByRole("button", { name: "Attach file", exact: true }).click();
      await pane.locator('input[type="file"][aria-label="Attach file input"]').setInputFiles(stagedPortraitImage());
      const dialog = page.getByRole("dialog", { name: t("upload.dialogTitle"), exact: true });
      await expect(dialog).toBeVisible();
      await expect(dialog).toHaveAttribute("aria-modal", "true");
      const bounds = await dialog.boundingBox();
      expect(bounds!.x).toBeLessThanOrEqual(16);
      expect(bounds!.y).toBeLessThanOrEqual(16);
      expect(bounds!.width).toBeGreaterThanOrEqual(viewport.width - 32);
      expect(bounds!.height).toBeGreaterThanOrEqual(viewport.height - 32);
      expectStagingBounded((await stagingGeometry(page))!, surface);
      const image = dialog.locator(".upload-staging-preview");
      await expect(image).toBeVisible();
      await image.evaluate((img: HTMLImageElement) => img.decode());
      const geometry = await image.evaluate((img: HTMLImageElement) => {
        const box = img.getBoundingClientRect();
        const preview = img.closest(".upload-preview-viewport")!;
        const frame = preview.getBoundingClientRect();
        return { width: box.width, height: box.height, ratio: img.naturalWidth / img.naturalHeight,
          inside: box.top >= frame.top && box.bottom <= frame.bottom + 1 && box.left >= frame.left && box.right <= frame.right + 1,
          frameHeight: frame.height, scrollHeight: preview.scrollHeight, clientHeight: preview.clientHeight };
      });
      expect(geometry.inside).toBe(true);
      expect(geometry.frameHeight).toBeGreaterThanOrEqual(180);
      expect(geometry.width / geometry.height).toBeCloseTo(geometry.ratio, 2);
      expect(geometry.scrollHeight).toBe(geometry.clientHeight);
      await expect(dialog.getByRole("group", { name: t("upload.previewMode") })).toHaveCount(0);
      await expect(dialog.locator(".upload-output-toolbar")).toBeInViewport();
      await expect(dialog.getByRole("textbox")).toBeInViewport();
      await expect(dialog.getByRole("button", { name: t("upload.sendAttachments") })).toBeInViewport();
    });
  }
}

test("clicking the fitted image opens a separate original-size popup and restores focus on close", async ({ page }) => {
  await page.setViewportSize(SHORT_VIEWPORT);
  await gotoReadyShell(page);
  const buffer = Buffer.from(await page.evaluate(() => {
    const canvas = document.createElement("canvas");
    canvas.width = 1600;
    canvas.height = 1200;
    canvas.getContext("2d")!.fillRect(0, 0, 1600, 1200);
    return canvas.toDataURL("image/png").split(",")[1];
  }), "base64");
  await page.getByRole("button", { name: "Attach file", exact: true }).click();
  await page.locator('input[type="file"][aria-label="Attach file input"]').setInputFiles({ name: "synthetic-landscape.png", mimeType: "image/png", buffer });
  const staging = page.getByRole("dialog", { name: t("upload.dialogTitle"), exact: true });
  const trigger = staging.getByRole("button", { name: t("upload.previewActualSize"), exact: true });
  await expect(staging.locator(".upload-staging-preview")).toBeVisible();
  await trigger.click();
  const popup = page.getByRole("dialog", { name: t("upload.previewActualSize"), exact: true });
  await expect(popup).toBeVisible();
  const img = popup.getByRole("img");
  await img.evaluate((image: HTMLImageElement) => image.decode());
  expect(await img.evaluate((image: HTMLImageElement) => ({ width: image.clientWidth, height: image.clientHeight }))).toEqual({ width: 1600, height: 1200 });
  const scroll = popup.locator(".upload-actual-size-viewport");
  await scroll.hover();
  await page.mouse.wheel(160, 160);
  await expect.poll(() => scroll.evaluate((el) => el.scrollTop)).toBeGreaterThan(0);
  await expect.poll(() => scroll.evaluate((el) => el.scrollLeft)).toBeGreaterThan(0);
  await page.keyboard.press("Escape");
  await expect(popup).toHaveCount(0);
  await expect(trigger).toBeFocused();
  await expect(staging).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(popup).toBeVisible();
  await popup.getByRole("button", { name: t("action.close", { title: t("upload.previewActualSize") }), exact: true }).click();
  await expect(popup).toHaveCount(0);
  await expect(trigger).toBeFocused();
});

for (const count of [1, 3]) {
  test(`${count} attachments remain reachable in a very short window`, async ({ page }) => {
    await page.setViewportSize({ width: 900, height: 360 });
    await gotoReadyShell(page);
    await page.getByRole("button", { name: "Attach file", exact: true }).click();
    await page.locator('input[type="file"][aria-label="Attach file input"]').setInputFiles(
      Array.from({ length: count }, (_, index) => ({ ...stagedPortraitImage(), name: `synthetic-${index}.png` }))
    );
    const dialog = page.getByRole("dialog", { name: t("upload.dialogTitle"), exact: true });
    await expect(dialog).toBeVisible();
    const caption = dialog.getByRole("textbox").last();
    await caption.fill("Synthetic caption");
    await expect(caption).toBeInViewport();
    expectStagingBounded((await stagingGeometry(page))!, "short staging");
    await expect(dialog.getByRole("button", { name: t("upload.sendAttachments") })).toBeInViewport();
    expect((await stagingGeometry(page))!.list!.scrollTop).toBeGreaterThan(0);
    expect((await stagingGeometry(page))!.pageScrollY).toBe(0);
  });
}

for (const height of [800, 520]) {
  test(`macOS attachment dialogs stay below the native titlebar at ${height}px`, async ({ page }) => {
    await page.setViewportSize({ width: 1200, height });
    await gotoReadyShell(page);
    await page.evaluate(() => {
      const snapshot = window.__harness.currentSnapshot();
      snapshot.state.domain.locale_profile.platform = "macos";
      window.__harness.setSnapshot(snapshot);
    });
    const titlebar = page.locator('.titlebar[data-platform="macos"]');
    await expect(titlebar).toBeVisible();
    const titlebarBottom = await titlebar.evaluate((element) => element.getBoundingClientRect().bottom);
    await page.getByRole("button", { name: "Attach file", exact: true }).click();
    await page.locator('input[type="file"][aria-label="Attach file input"]').setInputFiles(stagedPortraitImage());
    const dialog = page.getByRole("dialog", { name: t("upload.dialogTitle"), exact: true });
    await expect(dialog).toBeVisible();
    expect((await dialog.boundingBox())!.y).toBeGreaterThanOrEqual(titlebarBottom);
    expectStagingBounded((await stagingGeometry(page))!, "macOS staging");
    await expect(dialog.getByRole("button", { name: t("upload.sendAttachments") })).toBeInViewport();
    await expect(dialog.locator(".upload-staging-preview")).toBeVisible();
    await dialog.locator(".upload-preview-open").click();
    const popup = page.getByRole("dialog", { name: t("upload.previewActualSize"), exact: true });
    await expect(popup).toBeVisible();
    const box = (await popup.boundingBox())!;
    expect(box.y).toBeGreaterThanOrEqual(titlebarBottom);
    expect(box.y + box.height).toBeLessThanOrEqual(height);
    await popup.getByRole("button", { name: t("action.close", { title: t("upload.previewActualSize") }) }).click();
    await expect(popup).toHaveCount(0);
    await expect(dialog).toBeVisible();
  });
}
