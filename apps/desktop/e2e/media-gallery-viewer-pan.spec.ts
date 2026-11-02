/**
 * Headless spec: a zoomed-in image in the room media gallery viewer can be
 * panned, wheel-scrolled and arrow-keyed, bounded by the stage (#1746).
 *
 * The viewer zooms with `transform: scale()` on `.media-viewer-image` inside an
 * `overflow: hidden` `.media-viewer-stage`, so past 100% the scaled image is
 * clipped and every part outside the stage box is unreachable: no drag, no
 * wheel, no scrollbar. Since #1741 the 100% view fits the stage on both axes,
 * so this spec only covers what zoom can reveal.
 *
 * The pan is clamped to the overflow of the scaled image: its edges come flush
 * with the stage and stop, and a zoom level that fits again returns the image
 * to the centre. Every case seeds a synthetic canvas image (no external data),
 * drives the real gallery and viewer, and measures rectangles; `toBeVisible()`
 * says nothing about which part of an image is reachable.
 */

import { expect, test, type Page } from "@playwright/test";
import { gotoReadyShell } from "./support/basicOperations";

/** The geometry from #1739: a 692 × 400 stage inside a 720 px dialog. */
const STAGE_HEIGHT = 400;
/** 50dvh must stay above the 400 px cap for the reported stage height. */
const VIEWPORT = { width: 1200, height: 900 };

interface ImageCase {
  width: number;
  height: number;
}

/** 2:1 landscape: 1035 × 517.5 at 150%, so both axes overflow the stage. */
const LANDSCAPE: ImageCase = { width: 2000, height: 1000 };
const FILENAME = `pan-${LANDSCAPE.width}x${LANDSCAPE.height}.png`;

test.use({ viewport: VIEWPORT });

/** Seeds the room media gallery with one synthetic PNG data URL. */
async function seedGallery(page: Page, image: ImageCase): Promise<void> {
  await page.evaluate(
    ({ width, height, filename }) => {
      const canvas = document.createElement("canvas");
      canvas.width = width;
      canvas.height = height;
      const context = canvas.getContext("2d")!;
      context.fillStyle = "#000";
      context.fillRect(0, 0, width, height);
      context.fillStyle = "#1e6fd9";
      context.beginPath();
      context.arc(width / 2, height / 2, Math.min(width, height) * 0.35, 0, Math.PI * 2);
      context.fill();
      const url = canvas.toDataURL("image/png");

      const mediaGallery = [
        {
          event_id: "$pan:example.invalid",
          room_id: "!harness-room:example.invalid",
          sender: "@harness-user:example.invalid",
          sender_label: "Harness User",
          timestamp_ms: 1_900_000_000_000,
          media: {
            kind: "Image",
            filename,
            source: {
              mxc_uri: "mxc://example.invalid/pan",
              encrypted: false,
              encryption_version: null
            },
            mimetype: "image/png",
            size: 4096,
            width,
            height,
            thumbnail: null
          }
        }
      ];
      const mediaDownloads = {
        "$pan:example.invalid": {
          kind: "ready",
          source_ref: url,
          width,
          height,
          mime_type: "image/png"
        }
      };

      const snapshot = window.__harness.currentSnapshot();
      window.__harness.setSnapshot({
        ...snapshot,
        state: {
          ...snapshot.state,
          ui: {
            ...snapshot.state.ui,
            timeline: {
              ...snapshot.state.ui.timeline,
              media_gallery: mediaGallery,
              // eslint-disable-next-line @typescript-eslint/no-explicit-any
              media_downloads: mediaDownloads as any
            }
          }
        }
      });
      window.__harness.pushStateUpdate();
    },
    { width: image.width, height: image.height, filename: FILENAME }
  );
}

interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
  /** The content box: what a percentage size inside the stage resolves against. */
  clientWidth: number;
  clientHeight: number;
}

async function rectOf(page: Page, selector: string): Promise<Rect> {
  const rect = await page.evaluate((sel) => {
    const element = document.querySelector<HTMLElement>(sel);
    if (!element) {
      return null;
    }
    const box = element.getBoundingClientRect();
    return {
      x: box.x,
      y: box.y,
      width: box.width,
      height: box.height,
      clientWidth: element.clientWidth,
      clientHeight: element.clientHeight
    };
  }, selector);
  if (!rect) {
    throw new Error(`no element for ${selector}`);
  }
  return rect;
}

const imageRect = (page: Page) => rectOf(page, ".media-viewer-image");
const stageRect = (page: Page) => rectOf(page, ".media-viewer-stage");

/**
 * The stage's content box, which is where the image is centred and clipped: the
 * border-box rect plus the 1 px border on each side.
 */
async function stageContent(page: Page): Promise<Rect> {
  const stage = await stageRect(page);
  return {
    ...stage,
    x: stage.x + (stage.width - stage.clientWidth) / 2,
    y: stage.y + (stage.height - stage.clientHeight) / 2
  };
}

async function openViewer(page: Page): Promise<void> {
  await gotoReadyShell(page);
  await seedGallery(page, LANDSCAPE);
  await page.getByRole("button", { name: "Open media gallery" }).click();
  const gallery = page.getByRole("region", { name: "Room media gallery" });
  await gallery.getByRole("button", { name: `Open ${FILENAME}` }).click();
  await expect(page.getByRole("dialog", { name: "Media viewer" })).toBeVisible();
  // The image is decoded before its intrinsic size can be measured.
  await expect
    .poll(async () =>
      page.evaluate(
        () =>
          document.querySelector<HTMLImageElement>(".media-viewer-image")?.naturalWidth ?? 0
      )
    )
    .toBe(LANDSCAPE.width);
}

async function zoomIn(page: Page, clicks: number): Promise<void> {
  for (let index = 0; index < clicks; index += 1) {
    await page.getByRole("button", { name: "Zoom in" }).click();
  }
}

/**
 * How far the zoomed image may travel on each axis: half of what it overflows
 * the stage content box by, and zero once it fits again.
 */
async function panLimits(
  page: Page,
  zoom: number
): Promise<{ x: number; y: number; fitted: { width: number; height: number } }> {
  const stage = await stageRect(page);
  const scale = Math.min(
    1,
    stage.clientWidth / LANDSCAPE.width,
    stage.clientHeight / LANDSCAPE.height
  );
  const fitted = { width: LANDSCAPE.width * scale, height: LANDSCAPE.height * scale };
  return {
    x: Math.max(0, (fitted.width * zoom - stage.clientWidth) / 2),
    y: Math.max(0, (fitted.height * zoom - stage.clientHeight) / 2),
    fitted
  };
}

/** The image rectangle once the 160 ms zoom transition has settled. */
async function settledImage(page: Page, zoom: number): Promise<Rect> {
  const limits = await panLimits(page, zoom);
  await expect
    .poll(async () => Math.round((await imageRect(page)).width))
    .toBe(Math.round(limits.fitted.width * zoom));
  return imageRect(page);
}

async function drag(
  page: Page,
  delta: { x: number; y: number }
): Promise<void> {
  const stage = await stageRect(page);
  const startX = stage.x + stage.width / 2;
  const startY = stage.y + stage.height / 2;
  await page.mouse.move(startX, startY);
  await page.mouse.down();
  await page.mouse.move(startX + delta.x, startY + delta.y, { steps: 6 });
  await page.mouse.up();
}

test("the stage keeps measuring the reported 692 × 400 box (#1746)", async ({ page }) => {
  await openViewer(page);
  const stage = await stageRect(page);
  expect(Math.round(stage.height)).toBe(STAGE_HEIGHT);
});

test("a drag pans the zoomed image by the drag distance (#1746)", async ({ page }) => {
  await openViewer(page);
  await zoomIn(page, 2);
  const zoomed = await settledImage(page, 1.5);

  await drag(page, { x: 120, y: 0 });

  // 120 px stays inside the 172.5 px horizontal limit, so the image follows the
  // pointer exactly: the left part of the image comes into view.
  await expect
    .poll(async () => (await imageRect(page)).x - zoomed.x)
    .toBeCloseTo(120, 0);
  const panned = await imageRect(page);
  expect(panned.y, "a horizontal drag must not shift the image vertically").toBeCloseTo(
    zoomed.y,
    0
  );
  expect(panned.width, "the drag must not resize the image").toBeCloseTo(zoomed.width, 0);
});

test("the pan stops at the image edge instead of leaving the stage (#1746)", async ({ page }) => {
  await openViewer(page);
  await zoomIn(page, 2);
  await settledImage(page, 1.5);
  const limits = await panLimits(page, 1.5);
  const content = await stageContent(page);

  // Far beyond the limit on both axes: dragging right and down reveals the
  // left and top of the image.
  await drag(page, { x: 600, y: 400 });

  await expect
    .poll(async () => Math.round((await imageRect(page)).x - content.x))
    .toBe(0);
  const clamped = await imageRect(page);
  expect(
    Math.round(clamped.y - content.y),
    "the top edge must come flush with the stage, not leave it"
  ).toBe(0);
  expect(clamped.x).toBeGreaterThanOrEqual(content.x - 0.5);
  expect(clamped.y).toBeGreaterThanOrEqual(content.y - 0.5);
  expect(
    clamped.x + clamped.width,
    "the far edge stays clipped: the image is wider than the stage"
  ).toBeGreaterThan(content.x + content.clientWidth);

  // A further drag in the same direction moves nothing.
  await drag(page, { x: 200, y: 0 });
  const after = await imageRect(page);
  expect(after.x).toBeCloseTo(clamped.x, 0);
  expect(after.y).toBeCloseTo(clamped.y, 0);
  expect(limits.x, "the landscape case must overflow horizontally at 150%").toBeGreaterThan(100);
});

test("a trackpad scroll pans the zoomed image and clamps at the edge (#1746)", async ({
  page
}) => {
  await openViewer(page);
  await zoomIn(page, 2);
  const before = await settledImage(page, 1.5);
  const limits = await panLimits(page, 1.5);
  const stage = await stageRect(page);
  const content = await stageContent(page);

  await page.mouse.move(stage.x + stage.width / 2, stage.y + stage.height / 2);
  await page.mouse.wheel(0, 40);

  // Scrolling down reveals the bottom: the image moves up by the wheel delta.
  await expect.poll(async () => (await imageRect(page)).y - before.y).toBeCloseTo(-40, 0);

  await page.mouse.wheel(0, 400);
  await expect
    .poll(async () => {
      const box = await imageRect(page);
      return Math.round(box.y + box.height - (content.y + content.clientHeight));
    })
    .toBe(0);
  expect(limits.y, "the landscape case must overflow vertically at 150%").toBeGreaterThan(20);
});

test("arrow keys pan only while the stage has focus (#1746)", async ({ page }) => {
  await openViewer(page);
  const fitted = await imageRect(page);

  // Fitted: there is nothing to reveal, so the keys stay inert.
  await stageLocator(page).click();
  await page.keyboard.press("ArrowRight");
  expect((await imageRect(page)).x).toBeCloseTo(fitted.x, 0);

  await zoomIn(page, 2);
  const zoomed = await settledImage(page, 1.5);

  // Focus elsewhere in the viewer: the image must not move.
  await page.getByRole("button", { name: "Close media viewer" }).focus();
  await page.keyboard.press("ArrowRight");
  expect((await imageRect(page)).x).toBeCloseTo(zoomed.x, 0);

  await stageLocator(page).click();
  await page.keyboard.press("ArrowRight");
  // ArrowRight reveals the right side, so the image travels left.
  await expect.poll(async () => (await imageRect(page)).x - zoomed.x).toBeCloseTo(-24, 0);

  await page.keyboard.press("ArrowUp");
  await expect.poll(async () => (await imageRect(page)).y - zoomed.y).toBeCloseTo(24, 0);
});

function stageLocator(page: Page) {
  return page.locator(".media-viewer-stage");
}

test("zooming back out to the fitted view returns the image to the centre (#1746)", async ({
  page
}) => {
  await openViewer(page);
  const fitted = await imageRect(page);

  await zoomIn(page, 2);
  const zoomed = await settledImage(page, 1.5);
  await drag(page, { x: 120, y: 0 });
  await expect
    .poll(async () => (await imageRect(page)).x - zoomed.x)
    .toBeCloseTo(120, 0);

  await page.getByRole("button", { name: "Zoom out" }).click();
  await page.getByRole("button", { name: "Zoom out" }).click();

  const restored = await settledImage(page, 1);
  expect(restored.width).toBeCloseTo(fitted.width, 0);
  expect(restored.x, "the fitted view must be centred again").toBeCloseTo(fitted.x, 0);
  expect(restored.y).toBeCloseTo(fitted.y, 0);
});
