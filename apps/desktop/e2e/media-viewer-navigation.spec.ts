/**
 * Headless spec: the image viewer opened from the timeline navigates through
 * the room's images in gallery order (#1810).
 *
 * Before #1810 a timeline image opened `TimelineMediaViewer`, which took one
 * image and handled only Escape: no previous/next control, no arrow-key
 * navigation, no counter. This spec drives the real app harness from a seeded
 * timeline row plus the room's `ui.timeline.media_gallery`, and asserts the
 * displayed image and the position counter change. Every image is a synthetic
 * canvas data URL; nothing external is contacted.
 */

import { expect, test, type Page } from "@playwright/test";

import { t } from "../src/i18n/messages";
import { gotoReadyShell, seedTimelineItems } from "./support/basicOperations";

const HARNESS_ROOM_ID = "!harness-room:example.invalid";

/** Distinct intrinsic widths identify which image the stage shows. */
const IMAGES = [
  { width: 320, height: 200 },
  { width: 480, height: 200 },
  { width: 640, height: 200 }
];

function eventIdOf(index: number): string {
  return `$nav-image-${index}:example.invalid`;
}

function timelineItem(index: number): Record<string, unknown> {
  return {
    id: { Event: { event_id: eventIdOf(index) } },
    sender: "@harness-user:example.invalid",
    sender_label: "Harness User",
    body: null,
    timestamp_ms: 1_800_000_300_000 + index * 1000,
    in_reply_to_event_id: null,
    thread_root: null,
    thread_summary: null,
    reactions: [],
    can_react: false,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false,
    media: {
      kind: "Image",
      filename: `nav-${index + 1}.png`,
      source: {
        mxc_uri: `mxc://example.invalid/nav-${index}`,
        encrypted: false,
        encryption_version: null
      },
      mimetype: "image/png",
      size: 4096,
      width: IMAGES[index].width,
      height: IMAGES[index].height,
      thumbnail: null
    },
    actions: {
      can_copy: false,
      can_reply: true,
      can_permalink: false,
      permalink: null,
      can_view_source: false,
      can_forward: false
    }
  };
}

/**
 * Seeds the room media sequence and its downloads: the gallery is the only
 * ordering source (#1810), so it lists the three images in timeline order and
 * the listed indexes carry synthetic data URLs as ready downloads.
 */
async function seedRoomImages(page: Page, ready: number[] = [0, 1, 2]): Promise<void> {
  await page.evaluate(({ roomId, images, ids, readyIndexes }) => {
    const mediaGallery = images.map((image, index) => ({
      event_id: ids[index],
      room_id: roomId,
      sender: "@harness-user:example.invalid",
      sender_label: "Harness User",
      timestamp_ms: 1_800_000_300_000 + index * 1000,
      media: {
        kind: "Image",
        filename: `nav-${index + 1}.png`,
        source: {
          mxc_uri: `mxc://example.invalid/nav-${index}`,
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
      mediaGallery.flatMap((item, index) => {
        if (!readyIndexes.includes(index)) {
          return [];
        }
        const canvas = document.createElement("canvas");
        canvas.width = images[index].width;
        canvas.height = images[index].height;
        const context = canvas.getContext("2d")!;
        context.fillStyle = "#000";
        context.fillRect(0, 0, canvas.width, canvas.height);
        context.fillStyle = "#1e6fd9";
        context.fillRect(0, 0, canvas.width / 2, canvas.height);
        return [
          [
            item.event_id,
            {
              kind: "ready",
              source_ref: canvas.toDataURL("image/png"),
              width: images[index].width,
              height: images[index].height,
              mime_type: "image/png"
            }
          ]
        ];
      })
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
  }, {
    roomId: HARNESS_ROOM_ID,
    images: IMAGES,
    ids: IMAGES.map((_, index) => eventIdOf(index)),
    readyIndexes: ready
  });
}

/** The intrinsic width of the image the viewer is currently showing. */
async function shownImageWidth(page: Page): Promise<number> {
  return page.evaluate(
    () =>
      document.querySelector<HTMLImageElement>(".media-viewer-image")?.naturalWidth ?? 0
  );
}

const counter = (page: Page) => page.locator(".media-viewer-counter");
const stage = (page: Page) => page.locator(".media-viewer-stage");

/** Opens the first image from the timeline, which is how a reader reaches it. */
async function openFirstImageFromTimeline(
  page: Page,
  ready: number[] = [0, 1, 2]
): Promise<void> {
  await gotoReadyShell(page);
  await seedTimelineItems(page, IMAGES.map((_, index) => timelineItem(index)));
  await seedRoomImages(page, ready);
  const row = page.locator(`[data-event-id="${eventIdOf(0)}"]`);
  await row.getByRole("button", { name: t("timeline.mediaOpenFile") }).click();
  const viewer = page.getByRole("dialog", { name: t("mediaGallery.viewerTitle") });
  await expect(viewer).toBeVisible();
  await expect.poll(() => shownImageWidth(page)).toBe(IMAGES[0].width);
}

test("a timeline image opens in the viewer at its position in the room sequence (#1810)", async ({
  page
}) => {
  await openFirstImageFromTimeline(page);

  await expect(counter(page)).toHaveText("1/3");
  await expect(page.getByRole("button", { name: t("mediaGallery.next") })).toBeEnabled();
  await expect(page.getByRole("button", { name: t("mediaGallery.previous") })).toBeDisabled();
});

test("the side control moves to the next and previous image (#1810)", async ({ page }) => {
  await openFirstImageFromTimeline(page);

  await page.getByRole("button", { name: t("mediaGallery.next") }).click();
  await expect.poll(() => shownImageWidth(page)).toBe(IMAGES[1].width);
  await expect(counter(page)).toHaveText("2/3");

  await page.getByRole("button", { name: t("mediaGallery.previous") }).click();
  await expect.poll(() => shownImageWidth(page)).toBe(IMAGES[0].width);
  await expect(counter(page)).toHaveText("1/3");
});

test("ArrowRight navigates a fitted image and ArrowLeft returns (#1810)", async ({ page }) => {
  await openFirstImageFromTimeline(page);

  await stage(page).click();
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => shownImageWidth(page)).toBe(IMAGES[1].width);
  await expect(counter(page)).toHaveText("2/3");

  await page.keyboard.press("ArrowLeft");
  await expect.poll(() => shownImageWidth(page)).toBe(IMAGES[0].width);
  await expect(counter(page)).toHaveText("1/3");
});

test("a horizontal trackpad swipe moves exactly one image (#1810)", async ({ page }) => {
  await openFirstImageFromTimeline(page);

  const box = (await stage(page).boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  // One swipe: a burst of horizontal deltas well past the threshold.
  await page.mouse.wheel(-120, 0);
  await page.mouse.wheel(-120, 0);

  await expect.poll(() => shownImageWidth(page)).toBe(IMAGES[1].width);
  await expect(counter(page)).toHaveText("2/3");
});

test("navigation stops at the first and last image instead of wrapping (#1810)", async ({
  page
}) => {
  await openFirstImageFromTimeline(page);

  // Backwards from the first image stays put.
  await page.getByRole("button", { name: t("mediaGallery.previous") }).click({ force: true });
  await stage(page).click();
  await page.keyboard.press("ArrowLeft");
  await expect.poll(() => shownImageWidth(page)).toBe(IMAGES[0].width);
  await expect(counter(page)).toHaveText("1/3");

  await page.getByRole("button", { name: t("mediaGallery.next") }).click();
  await page.getByRole("button", { name: t("mediaGallery.next") }).click();
  await expect(counter(page)).toHaveText("3/3");

  // Forwards from the last image stays put.
  await page.getByRole("button", { name: t("mediaGallery.next") }).click({ force: true });
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => shownImageWidth(page)).toBe(IMAGES[2].width);
  await expect(counter(page)).toHaveText("3/3");
});

test("switching images resets zoom and pan to the fitted view (#1810)", async ({ page }) => {
  await openFirstImageFromTimeline(page);
  const fitted = (await page.locator(".media-viewer-image").boundingBox())!;

  await page.getByRole("button", { name: t("mediaGallery.zoomIn") }).click();
  await page.getByRole("button", { name: t("mediaGallery.zoomIn") }).click();
  await expect
    .poll(async () => (await page.locator(".media-viewer-image").boundingBox())!.width)
    .toBeGreaterThan(fitted.width + 1);

  await page.getByRole("button", { name: t("mediaGallery.next") }).click();
  await expect.poll(() => shownImageWidth(page)).toBe(IMAGES[1].width);
  await expect
    .poll(async () =>
      page.evaluate(
        () =>
          document.querySelector<HTMLElement>(".media-viewer-image")?.style.transform ?? ""
      )
    )
    .toContain("scale(1)");
});

test("a zoomed drag pans first and only continues into the neighbour at the edge (#1746, #1810)", async ({
  page
}) => {
  await openFirstImageFromTimeline(page);
  // A middle image, so a wrongly triggered navigation cannot hide behind an
  // endpoint: both neighbours exist and the counter would change.
  await page.getByRole("button", { name: t("mediaGallery.next") }).click();
  await expect(counter(page)).toHaveText("2/3");
  await page.getByRole("button", { name: t("mediaGallery.zoomIn") }).click();
  await page.getByRole("button", { name: t("mediaGallery.zoomIn") }).click();

  const image = page.locator(".media-viewer-image");
  // The zoom transition settles before a rectangle means anything.
  await expect
    .poll(async () => Math.round((await image.boundingBox())!.width))
    .toBe(IMAGES[1].width * 1.5);
  const before = (await image.boundingBox())!;
  const drag = async (distance: number) => {
    const box = (await stage(page).boundingBox())!;
    const startX = box.x + box.width / 2;
    const startY = box.y + box.height / 2;
    await page.mouse.move(startX, startY);
    await page.mouse.down();
    await page.mouse.move(startX + distance, startY, { steps: 6 });
    await page.mouse.up();
  };

  // #1746 owns a drag that starts away from the edge: it pans (clamped by the
  // overflow) and must not jump to the neighbour.
  await drag(120);
  await expect
    .poll(async () => (await image.boundingBox())!.x)
    .toBeGreaterThan(before.x + 1);
  await expect(counter(page)).toHaveText("2/3");

  // From the horizontal pan limit the same drag continues into the previous
  // image, which is the one gesture the design lets a zoomed drag navigate.
  await drag(120);
  await expect(counter(page)).toHaveText("1/3");
  await expect.poll(() => shownImageWidth(page)).toBe(IMAGES[0].width);
});
