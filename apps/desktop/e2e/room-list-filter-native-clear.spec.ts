import { expect, test, type Page } from "@playwright/test";
import { t } from "../src/i18n/messages";

// #1145: the room-list Filter is a native `type="search"` input. WebKit (macOS
// WKWebView, Linux WebKitGTK) and Chromium (Windows WebView2) draw their own
// cancel control inside a non-empty search input, which duplicated the
// application-owned clear button. Only one DOM button exists either way, so
// this spec probes the native hit target instead: it clicks where the engine
// places its cancel control and asserts the text survives. The playwright
// config runs this file in both Chromium and WebKit.

const SAMPLE = "Sample";

async function openFilter(page: Page) {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("complementary", { name: t("workspace.rooms") })).toBeVisible();
  const input = page.getByRole("searchbox", {
    name: t("roomList.filterConversationsPlaceholder")
  });
  await expect(input).toBeVisible();
  await input.fill(SAMPLE);
  await expect(input).toHaveValue(SAMPLE);
  return input;
}

test("the room-list filter exposes no native search cancel control", async ({ page }) => {
  const input = await openFilter(page);

  // The native cancel control sits at the inline end of the input's content
  // box. Hover first so engines that show it only on hover/focus render it,
  // then click a few points across that region.
  const box = await input.boundingBox();
  expect(box).not.toBeNull();
  const { x, y, width, height } = box!;
  await input.hover({ position: { x: width - 8, y: height / 2 } });
  for (const inset of [4, 8, 12]) {
    await page.mouse.click(x + width - inset, y + height / 2);
    await expect(input).toHaveValue(SAMPLE);
  }

  // Visual evidence: with the cursor over it, the input's inline end must
  // render exactly as it does once the field stops being a search input
  // (`type="text"` never draws a cancel decoration). Engines do not expose
  // the pseudo-element's computed style, so compare pixels instead.
  const endRegion = { x: x + width - 24, y, width: 24, height };
  await page.mouse.move(x + width - 8, y + height / 2);
  const asSearch = await page.screenshot({ clip: endRegion, animations: "disabled" });
  await input.evaluate((element) => element.setAttribute("type", "text"));
  const asText = await page.screenshot({ clip: endRegion, animations: "disabled" });
  expect(asSearch.equals(asText)).toBe(true);
});

test("the application clear button and Escape still clear the filter", async ({ page }) => {
  const input = await openFilter(page);
  const clear = page.getByRole("button", { name: t("roomList.clearFilter") });
  await expect(clear).toHaveCount(1);
  await clear.click();
  await expect(input).toHaveValue("");
  await expect(clear).toHaveCount(0);

  await input.fill(SAMPLE);
  await input.press("Escape");
  await expect(input).toHaveValue("");
});
