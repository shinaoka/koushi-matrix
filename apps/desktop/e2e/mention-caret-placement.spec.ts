/**
 * Headless spec: the composer caret is painted clear of a mention pill (#875).
 *
 * A caret at a parent-level offset beside an atomic inline has no geometry of
 * its own, so the browser paints it against a guess: Chromium lands on the
 * pill's border box, WebKit lands inside it over the leading "@". The composer
 * renders a zero-width caret anchor on the pill's outside so the caret resolves
 * to real text instead.
 *
 * These tests drive the real composer, measure the caret rectangle at both pill
 * boundaries, and require it to sit outside the pill's border box.
 */

import { expect, test, type Page } from "@playwright/test";
import { t } from "../src/i18n/messages";

const ROOM_ID = "!harness-room:example.invalid";

async function seedMentionCandidates(page: Page): Promise<void> {
  await page.evaluate((roomId) => {
    const candidates = [
      {
        user_id: "@mention-0:example.invalid",
        display_label: "Mention Person 0",
        original_display_label: "Mention Person 0",
        avatar: null,
        membership: "joined"
      }
    ];
    const withCandidates = (snapshot: typeof window.__harness.currentSnapshot) => ({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          mention_candidates: {
            targets: [
              {
                room_id: roomId,
                generation: 1,
                request_id: 875,
                query: "",
                surface: "main",
                completeness: "complete",
                candidates,
                room_mention_allowed: "denied",
                failure_kind: null
              }
            ]
          }
        }
      }
    });
    const seeded = withCandidates(window.__harness.currentSnapshot());
    window.__harness.setSnapshot(seeded);
    window.__harness.setCommandResponse("query_mention_candidates", () => seeded);
    window.__harness.pushStateUpdate();
  }, ROOM_ID);
}

/** Compose a mention pill followed by a single trailing character. */
async function composeMentionWithTrailingText(page: Page): Promise<void> {
  const composer = page.getByRole("textbox", { name: t("composer.messageComposer") });
  await composer.fill("@");
  const option = page.locator(".composer-autocomplete [role='option']").first();
  await expect(option).toBeVisible();
  await option.click();
  await page.keyboard.type("x");
}

/** The caret rectangle and the pill's border box, as the browser reports them. */
async function caretAndPill(page: Page) {
  return page.evaluate(() => {
    const control = document.querySelector<HTMLElement>(".composer-inline-editor");
    const pill = control?.querySelector<HTMLElement>("[data-composer-mention]");
    const selection = document.getSelection();
    if (!control || !pill || !selection || selection.rangeCount === 0) {
      throw new Error("composer mention selection unavailable");
    }
    const range = selection.getRangeAt(0);
    const caret = range.getBoundingClientRect();
    const pillRect = pill.getBoundingClientRect();
    return {
      caretIsTextNode: selection.anchorNode?.nodeType === Node.TEXT_NODE,
      caretInsidePill: pill.contains(selection.anchorNode),
      caretX: caret.x,
      caretHeight: caret.height,
      pillLeft: pillRect.x,
      pillRight: pillRect.right
    };
  });
}

test("the composer caret is painted outside the mention pill on both sides", async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 700 });
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: t("timeline.conversation") })).toBeVisible();
  await seedMentionCandidates(page);

  await composeMentionWithTrailingText(page);

  // Two ArrowLefts: past the trailing character, then between the pill and it.
  await page.keyboard.press("ArrowLeft");
  await page.keyboard.press("ArrowLeft");
  const between = await caretAndPill(page);
  expect(between.caretIsTextNode).toBe(true);
  expect(between.caretHeight).toBeGreaterThan(10);
  expect(between.caretX).toBeGreaterThanOrEqual(between.pillRight);

  // A third ArrowLeft reaches the reported position: before the pill.
  await page.keyboard.press("ArrowLeft");
  const before = await caretAndPill(page);
  expect(before.caretInsidePill).toBe(false);
  expect(before.caretIsTextNode).toBe(true);
  expect(before.caretHeight).toBeGreaterThan(10);
  expect(before.caretX).toBeLessThanOrEqual(before.pillLeft);
});
