/**
 * Headless spec: the room media gallery viewer fits an image to BOTH stage
 * dimensions (#1739).
 *
 * Reported from macOS as "every image taller than ~1.7:1 loses its bottom at
 * every zoom level", measured in a 720 px wide dialog with a 692 × 400 stage:
 * a 2:1 image fitted, a 4:3 one showed 77% of its height, a square 58%, a 1:2
 * portrait 29%. The image was sized from the column width alone.
 *
 * Root cause is the stage's grid: `display: grid; place-items: center` with no
 * explicit row leaves the image in an implicit `auto` row, so the row is sized
 * FROM the image and the image's `max-block-size: 100%` has no definite height
 * to resolve against — it computes to `none`. The image is then as tall as the
 * column is wide, and `overflow: hidden` cuts everything below the stage.
 *
 * Every case seeds synthetic canvas images (no external data), drives the real
 * gallery and viewer, and measures rectangles: `toBeVisible()` is not evidence
 * for a fitting defect. The defect is not engine-native — the report measured
 * headless WebKit and Chromium identically — so the spec stays Chromium-only.
 */

import { expect, test, type Page } from "@playwright/test";
import { gotoReadyShell } from "./support/basicOperations";

/** The geometry from the report: a 692 × 400 stage inside a 720 px dialog. */
const STAGE_WIDTH = 692;
const STAGE_HEIGHT = 400;
/** 50dvh must stay above the 400 px cap for the reported stage height. */
const VIEWPORT = { width: 1200, height: 900 };

interface ImageCase {
  aspect: string;
  width: number;
  height: number;
}

const IMAGE_CASES: ImageCase[] = [
  { aspect: "2:1 landscape", width: 2000, height: 1000 },
  { aspect: "4:3 landscape", width: 1600, height: 1200 },
  { aspect: "1:1 square", width: 1200, height: 1200 },
  { aspect: "1:2 portrait", width: 800, height: 1600 },
  { aspect: "small 300 × 200", width: 300, height: 200 }
];

const SQUARE: ImageCase = { aspect: "1:1 square", width: 1200, height: 1200 };

test.use({ viewport: VIEWPORT });

function filenameOf(image: ImageCase): string {
  return `aspect-${image.width}x${image.height}.png`;
}

/**
 * Seeds one gallery holding every case. Each image is a synthetic canvas PNG of
 * the exact intrinsic size (a blue disc with a red bottom bar), rendered to a
 * data URL inside the page so the harness serves it as the ready download.
 */
async function seedGallery(page: Page): Promise<void> {
  await page.evaluate((cases) => {
    const urls = cases.map(({ width, height }) => {
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
      // The bottom 6%: a human sees clipping at a glance, the spec sees a box.
      context.fillStyle = "#d92b2b";
      context.fillRect(0, height * 0.94, width, height * 0.06);
      return canvas.toDataURL("image/png");
    });

    const mediaGallery = cases.map((image, index) => ({
      event_id: `$aspect-${index}:example.invalid`,
      room_id: "!harness-room:example.invalid",
      sender: "@harness-user:example.invalid",
      sender_label: "Harness User",
      timestamp_ms: 1_900_000_000_000 + index,
      media: {
        kind: "Image",
        filename: `aspect-${image.width}x${image.height}.png`,
        source: {
          mxc_uri: `mxc://example.invalid/aspect-${index}`,
          encrypted: false,
          encryption_version: null
        },
        mimetype: "image/png",
        size: 4096,
        width: image.width,
        height: image.height,
        thumbnail: null
      }
    }));

    const mediaDownloads = Object.fromEntries(
      mediaGallery.map((item, index) => [
        item.event_id,
        {
          kind: "ready",
          source_ref: urls[index],
          width: cases[index].width,
          height: cases[index].height,
          mime_type: "image/png"
        }
      ])
    );

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
  }, IMAGE_CASES);
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

async function openGalleryItem(page: Page, image: ImageCase): Promise<void> {
  await page.getByRole("button", { name: "Open media gallery" }).click();
  const gallery = page.getByRole("region", { name: "Room media gallery" });
  await gallery.getByRole("button", { name: `Open ${filenameOf(image)}` }).click();
  await expect(page.getByRole("dialog", { name: "Media viewer" })).toBeVisible();
  // The image is decoded before its intrinsic size can be measured.
  await expect
    .poll(async () =>
      page.evaluate(
        () =>
          document.querySelector<HTMLImageElement>(".media-viewer-image")?.naturalWidth ?? 0
      )
    )
    .toBe(image.width);
}

/**
 * The stage is the reported 692 × 400 box before any measurement means
 * anything. The dialog's own 1 px border sits inside its 720 px width, so the
 * stage measures 690 there, and 692 with the border counted outside.
 */
async function reportedStage(page: Page): Promise<Rect> {
  const stage = await rectOf(page, ".media-viewer-stage");
  expect(Math.round(stage.height), "stage height differs from the report").toBe(STAGE_HEIGHT);
  expect(
    Math.abs(stage.width - STAGE_WIDTH),
    `stage width ${stage.width} differs from the reported ${STAGE_WIDTH}`
  ).toBeLessThanOrEqual(2);
  return stage;
}

/**
 * The image fits inside the stage on both axes, keeps its aspect ratio, and is
 * never upscaled past its intrinsic size.
 */
async function expectFitted(
  page: Page,
  image: ImageCase,
  stage: Rect,
  label: string,
  zoom = 1
): Promise<void> {
  const box = await rectOf(page, ".media-viewer-image");
  const scale =
    Math.min(1, stage.clientWidth / image.width, stage.clientHeight / image.height) * zoom;

  expect(box.width, `${label}: width is not the two-axis fit`).toBeCloseTo(
    image.width * scale,
    0
  );
  expect(box.height, `${label}: height is not the two-axis fit`).toBeCloseTo(
    image.height * scale,
    0
  );
  expect(
    box.y + box.height,
    `${label}: bottom ${box.y + box.height} is below the stage bottom ${stage.y + stage.height}`
  ).toBeLessThanOrEqual(stage.y + stage.height + 0.5);
  expect(box.y, `${label}: top is above the stage top`).toBeGreaterThanOrEqual(stage.y - 0.5);
  expect(box.x, `${label}: left is left of the stage`).toBeGreaterThanOrEqual(stage.x - 0.5);
  expect(
    box.x + box.width,
    `${label}: right ${box.x + box.width} is right of the stage right ${stage.x + stage.width}`
  ).toBeLessThanOrEqual(stage.x + stage.width + 0.5);
  expect(box.height / box.width, `${label}: aspect ratio changed`).toBeCloseTo(
    image.height / image.width,
    2
  );
}

for (const image of IMAGE_CASES) {
  test(`the gallery viewer fits a ${image.aspect} image inside the stage (#1739)`, async ({
    page
  }) => {
    await gotoReadyShell(page);
    await seedGallery(page);
    await openGalleryItem(page, image);

    const stage = await reportedStage(page);
    await expectFitted(page, image, stage, image.aspect);
  });
}

test("zooming out of a square image keeps the whole image inside the stage (#1739)", async ({
  page
}) => {
  await gotoReadyShell(page);
  await seedGallery(page);
  await openGalleryItem(page, SQUARE);

  const stage = await reportedStage(page);
  await page.getByRole("button", { name: "Zoom out" }).click();
  await page.getByRole("button", { name: "Zoom out" }).click();

  // The fitted square fills the 398 px stage height; the 50% minimum is 199.
  await expect
    .poll(async () => Math.round((await rectOf(page, ".media-viewer-image")).width))
    .toBe(Math.round(Math.min(stage.clientWidth, stage.clientHeight) / 2));
  await expectFitted(page, SQUARE, stage, "square at 50% zoom", 0.5);
});
