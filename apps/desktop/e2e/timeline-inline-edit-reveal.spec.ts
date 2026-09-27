/**
 * Headless spec (#1001): entering inline edit reveals the whole edit form.
 *
 * Editing a long message near the bottom of the timeline used to keep the
 * row's top fixed while the editor grew downward, hiding Save/Cancel behind
 * the main composer. Edit is clicked through `evaluate` because Playwright's
 * own click scrolls the row into view and would hide the defect.
 */

import { expect, test, type Page } from "@playwright/test";
import { t } from "../src/i18n/messages";

const ACCOUNT = "@harness-user:example.invalid";
const KEY = { account_key: ACCOUNT, kind: { Room: { room_id: "!harness-room:example.invalid" } } };
const LONG = Array.from({ length: 40 }, (_, index) => `long line ${index} of the edited message`).join("\n");
const SHORT = "short line a\nshort line b\nshort line c";

function item(index: number, body?: string) {
  return {
    id: { Event: { event_id: `$e${index}:example.invalid` } },
    sender: ACCOUNT,
    sender_label: "Harness User",
    body: body ?? `message ${index}\nsecond line`,
    timestamp_ms: 1_800_000_000_000 + index * 1000,
    in_reply_to_event_id: null,
    thread_root: null,
    thread_summary: null,
    reactions: [],
    can_react: true,
    is_redacted: false,
    is_hidden: false,
    can_redact: true,
    is_edited: false,
    can_edit: true,
    send_state: null
  };
}

async function seed(page: Page, items: unknown[]): Promise<void> {
  await page.setViewportSize({ width: 1100, height: 720 });
  await page.goto("/appHarness.html");
  await expect(page.locator(`[data-event-id="$seed-event:example.invalid"]`)).toBeVisible();
  await expect
    .poll(
      () =>
        page.evaluate(
          async ({ items, key }) => {
            const harness = (window as unknown as { __harness: { pushCoreEvent(event: unknown): Promise<void> } })
              .__harness;
            await harness.pushCoreEvent({
              kind: "Timeline",
              event: { InitialItems: { request_id: null, key, generation: 2, items } }
            });
            await new Promise((resolve) => setTimeout(resolve, 25));
            const last = (items as Array<{ id: { Event: { event_id: string } } }>).at(-1)!.id.Event.event_id;
            return Boolean(document.querySelector(`[data-item-id="${CSS.escape(last)}"]`));
          },
          { items, key: KEY }
        ),
      { timeout: 10_000 }
    )
    .toBe(true);
  await page.waitForTimeout(300);
  await page.mouse.move(500, 300);
}

type Geometry = {
  viewportBottom: number;
  scrollTop: number;
  rowTop: number;
  rowBottom: number;
  actionsTop: number | null;
  actionsBottom: number | null;
  composerTop: number | null;
};

async function geometry(page: Page, eventId: string): Promise<Geometry> {
  return page.evaluate((eventId) => {
    const row = document.querySelector<HTMLElement>(`[data-event-id="${CSS.escape(eventId)}"]`)!;
    const view = row.closest<HTMLElement>("[data-testid=timeline-view]")!;
    const actions = row.querySelector<HTMLElement>(".message-edit-actions")?.getBoundingClientRect();
    const composer = document.querySelector<HTMLElement>(".composer:not(.is-editor-only)")?.getBoundingClientRect();
    const rowRect = row.getBoundingClientRect();
    return {
      viewportBottom: view.getBoundingClientRect().top + view.clientHeight,
      scrollTop: view.scrollTop,
      rowTop: rowRect.top,
      rowBottom: rowRect.bottom,
      actionsTop: actions?.top ?? null,
      actionsBottom: actions?.bottom ?? null,
      composerTop: composer?.top ?? null
    };
  }, eventId);
}

/** Wheel up until `placed` holds, leaving the timeline in free scroll. */
async function scrollUntil(page: Page, eventId: string, placed: (geometry: Geometry) => boolean): Promise<void> {
  for (let step = 0; step < 200; step += 1) {
    if (placed(await geometry(page, eventId))) return;
    await page.mouse.wheel(0, -20);
    await page.waitForTimeout(40);
  }
  throw new Error("could not place the row");
}

async function startEditing(page: Page, eventId: string): Promise<void> {
  const row = page.locator(`[data-event-id="${eventId}"]`);
  await row
    .getByRole("button", { name: t("timeline.editMessage") })
    .evaluate((button) => (button as HTMLButtonElement).click());
  await expect(row.getByRole("button", { name: t("timeline.saveEdit") })).toBeAttached();
}

async function expectActionsVisible(page: Page, eventId: string): Promise<Geometry> {
  let last: Geometry | null = null;
  await expect
    .poll(async () => {
      last = await geometry(page, eventId);
      return (
        last.actionsBottom !== null &&
        last.actionsBottom <= last.viewportBottom + 1 &&
        (last.composerTop === null || last.actionsBottom <= last.composerTop + 1)
      );
    }, { timeout: 3_000 })
    .toBe(true);
  return last!;
}

test("free scroll: editing a long message near the bottom reveals Save and Cancel", async ({ page }) => {
  const items = Array.from({ length: 24 }, (_, index) => item(index));
  items[18] = item(18, LONG);
  await seed(page, items);
  const id = "$e18:example.invalid";
  await scrollUntil(page, id, (g) => g.rowTop >= g.viewportBottom - 110);
  await startEditing(page, id);
  await expectActionsVisible(page, id);
});

test("free scroll: editing a short message at the bottom edge reveals Save and Cancel", async ({ page }) => {
  const items = Array.from({ length: 24 }, (_, index) => item(index));
  items[18] = item(18, SHORT);
  await seed(page, items);
  const id = "$e18:example.invalid";
  await scrollUntil(page, id, (g) => g.rowBottom >= g.viewportBottom - 6);
  await startEditing(page, id);
  const revealed = await expectActionsVisible(page, id);
  expect(revealed.actionsTop).not.toBeNull();
});

test("live edge: typing in the editor keeps the edit form pinned above the composer", async ({ page }) => {
  const items = Array.from({ length: 24 }, (_, index) => item(index));
  items[23] = item(23, SHORT);
  await seed(page, items);
  const id = "$e23:example.invalid";
  await startEditing(page, id);
  await expectActionsVisible(page, id);
  await page.keyboard.press("End");
  await page.keyboard.type(" x");
  for (let line = 0; line < 6; line += 1) {
    await page.keyboard.press("Shift+Enter");
    await page.keyboard.type(`n${line}`);
  }
  await expectActionsVisible(page, id);
});

test("live edge: editing the last message without typing stays visible", async ({ page }) => {
  const items = Array.from({ length: 24 }, (_, index) => item(index));
  items[23] = item(23, SHORT);
  await seed(page, items);
  const id = "$e23:example.invalid";
  await startEditing(page, id);
  await expectActionsVisible(page, id);
});

test("the reveal does not fight the reader's own scrolling afterwards", async ({ page }) => {
  const items = Array.from({ length: 24 }, (_, index) => item(index));
  items[18] = item(18, SHORT);
  await seed(page, items);
  const id = "$e18:example.invalid";
  await scrollUntil(page, id, (g) => g.rowBottom >= g.viewportBottom - 6);
  await startEditing(page, id);
  await expectActionsVisible(page, id);
  await page.mouse.wheel(0, -60);
  await page.waitForTimeout(300);
  const scrolledUp = await geometry(page, id);
  // Growing the form past the viewport bottom after the reader scrolled must
  // not pull the view back down.
  for (let line = 0; line < 6; line += 1) {
    await page.keyboard.press("Shift+Enter");
    await page.keyboard.type(`more ${line}`);
  }
  await page.waitForTimeout(500);
  const later = await geometry(page, id);
  expect(later.actionsBottom).not.toBeNull();
  expect(later.actionsBottom!).toBeGreaterThan(later.viewportBottom);
  expect(Math.abs(later.scrollTop - scrolledUp.scrollTop)).toBeLessThanOrEqual(1);
});

test("typing at the end of a long edit keeps the caret inside the editor", async ({ page }) => {
  const items = Array.from({ length: 24 }, (_, index) => item(index));
  items[23] = item(23, LONG);
  await seed(page, items);
  const id = "$e23:example.invalid";
  await startEditing(page, id);
  const editor = page.locator(`[data-event-id="${id}"]`).getByRole("textbox");
  await editor.evaluate((element) => {
    const range = document.createRange();
    range.selectNodeContents(element);
    range.collapse(false);
    const selection = window.getSelection()!;
    selection.removeAllRanges();
    selection.addRange(range);
  });
  await page.keyboard.type(" end");
  await expect
    .poll(() =>
      editor.evaluate((element) => {
        const selection = window.getSelection();
        if (!selection?.rangeCount) return false;
        const caret = selection.getRangeAt(0).getBoundingClientRect();
        const box = element.getBoundingClientRect();
        return element.scrollTop > 0 && caret.top >= box.top - 1 && caret.bottom <= box.bottom + 1;
      })
    )
    .toBe(true);
});

test("growth above the viewport is compensated before any reveal, keeping the reader's rows in place", async ({ page }) => {
  const items = Array.from({ length: 24 }, (_, index) => item(index));
  items[18] = item(18, SHORT);
  await seed(page, items);
  const id = "$e18:example.invalid";
  await scrollUntil(page, id, (g) => g.rowBottom >= g.viewportBottom - 6);
  await startEditing(page, id);
  await expectActionsVisible(page, id);
  await page.waitForTimeout(300);
  // The first visible row, and the mounted row just above it.
  const probe = await page.evaluate(() => {
    const view = document.querySelector<HTMLElement>("[data-testid=timeline-view]")!;
    const top = view.getBoundingClientRect().top;
    const rows = Array.from(view.querySelectorAll<HTMLElement>("[data-event-id^='$e']"));
    const firstVisible = rows.find((row) => row.getBoundingClientRect().bottom > top + 1)!;
    const index = Number(firstVisible.dataset.eventId!.slice(2).split(":")[0]);
    return { index, eventId: firstVisible.dataset.eventId!, top: firstVisible.getBoundingClientRect().top };
  });
  expect(probe.index).toBeGreaterThan(0);
  const grown = item(probe.index - 1, LONG);
  await page.evaluate(
    async ({ key, index, grown }) => {
      const harness = (window as unknown as { __harness: { pushCoreEvent(event: unknown): Promise<void> } }).__harness;
      await harness.pushCoreEvent({
        kind: "Timeline",
        event: { ItemsUpdated: { key, generation: 2, batch_id: 3, diffs: [{ Set: { index, item: grown } }] } }
      });
    },
    { key: KEY, index: probe.index - 1, grown }
  );
  await expect
    .poll(() => page.locator(`[data-event-id="$e${probe.index - 1}:example.invalid"]`).innerText())
    .toContain("long line 39");
  await page.waitForTimeout(500);
  const after = await page.evaluate(
    (eventId) => document.querySelector<HTMLElement>(`[data-event-id="${CSS.escape(eventId)}"]`)!.getBoundingClientRect().top,
    probe.eventId
  );
  expect(Math.abs(after - probe.top)).toBeLessThanOrEqual(2);
  await expectActionsVisible(page, id);
});

// #1027: both catalogs keep concise actions reachable in the real editor.
for (const locale of ["en", "ja"] as const) {
  test(`inline edit actions fit and retain accessible context in ${locale}`, async ({ page }) => {
    await seed(page, [item(0, SHORT)]);
    await page.evaluate((locale) => {
      const snapshot = window.__harness.currentSnapshot();
      snapshot.state.domain.locale_profile = { ...snapshot.state.domain.locale_profile, lang: locale, catalog_locale: locale };
      window.__harness.setSnapshot(snapshot);
      window.__harness.pushStateUpdate();
    }, locale);
    const row = page.locator('[data-event-id="$e0:example.invalid"]');
    await row.getByRole("button", { name: t("timeline.editMessage", {}, locale) }).click();
    await expect(row.getByRole("button", { name: t("timeline.saveEdit", {}, locale) })).toHaveText(t("settings.propertySave", {}, locale));
    await expect(row.getByRole("button", { name: t("timeline.cancelEdit", {}, locale) })).toHaveText(t("action.cancel", {}, locale));
    await expectActionsVisible(page, "$e0:example.invalid");
  });
}
