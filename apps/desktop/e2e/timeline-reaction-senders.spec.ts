/**
 * Headless spec: everyone who reacted is reachable (#2111).
 *
 * A chip's tooltip used to name at most three people and then say "N more",
 * and no surface listed the rest. Core now names up to a documented limit, the
 * tooltip shows those names wrapped and scrollable, and a right-click (or a
 * hold, where there is no right button) opens a fixed list that reuses the
 * read-receipt popover from #2098. A chip's own click still toggles.
 */

import { expect, test, type Page } from "@playwright/test";
import { t } from "../src/i18n/messages";
import { roomTimelineKey } from "../src/domain/coreEvents";

const HARNESS_ACCOUNT_KEY = "@harness-user:example.invalid";
const HARNESS_ROOM_ID = "!harness-room:example.invalid";
const HARNESS_ROOM_KEY = roomTimelineKey(HARNESS_ACCOUNT_KEY, HARNESS_ROOM_ID);
const REACTED_EVENT_ID = "$many-reactors:example.invalid";

async function gotoReadyShell(page: Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Reply to message" }).first()).toBeVisible();
}

function reactor(index: number): { user_id: string; display_label: string } {
  return {
    user_id: `@reactor-${index}:example.invalid`,
    display_label: `Reactor ${index}`
  };
}

/** One reaction by `count` people, of which Core names the first `named`. */
function reactedItem(count: number, named: number) {
  return {
    id: { Event: { event_id: REACTED_EVENT_ID } },
    sender: "@harness-user:example.invalid",
    sender_label: "Harness User",
    body: "A message with many reactions",
    timestamp_ms: 1_800_000_200_000,
    in_reply_to_event_id: null,
    thread_root: null,
    thread_summary: null,
    reactions: [
      {
        key: "🎉",
        count,
        reacted_by_me: false,
        my_reaction_event_id: null,
        // The signed-in user is the first reactor, so the list can mark them.
        sender_preview: [
          { user_id: HARNESS_ACCOUNT_KEY, display_label: "Harness User" },
          ...Array.from({ length: named - 1 }, (_, index) => reactor(index + 1))
        ]
      }
    ],
    can_react: true,
    is_redacted: false,
    is_hidden: false,
    can_redact: true,
    is_edited: false,
    can_edit: true
  };
}

async function seedTimelineItems(page: Page, items: unknown[]): Promise<void> {
  await expect
    .poll(
      async () =>
        page.evaluate(
          async ({ key, nextItems }) => {
            await window.__harness.pushCoreEvent({
              kind: "Timeline",
              event: {
                InitialItems: { request_id: null, key, generation: 2, items: nextItems }
              }
              // eslint-disable-next-line @typescript-eslint/no-explicit-any
            } as any);
            await new Promise((resolve) => setTimeout(resolve, 25));
            return nextItems.every((item) =>
              document.querySelector(
                `[data-event-id="${CSS.escape(
                  (item as { id: { Event: { event_id: string } } }).id.Event.event_id
                )}"]`
              )
            );
          },
          { key: HARNESS_ROOM_KEY, nextItems: items }
        ),
      { timeout: 10_000, intervals: [25, 50, 100, 250] }
    )
    .toBe(true);
}

test("the tooltip names every reactor Core names, and scrolls when the list is long", async ({
  page
}) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [reactedItem(7, 7)]);

  const chip = page.locator("[data-testid=timeline-view] .reaction-pill").first();
  await expect(chip).toBeVisible();
  await chip.hover();

  const tooltip = chip.locator(".reaction-tooltip");
  await expect(tooltip).toBeVisible();
  // Seven people, seven names: nothing hides behind "N more".
  await expect(tooltip).toContainText("Harness User");
  for (let index = 1; index <= 6; index += 1) {
    await expect(tooltip).toContainText(`Reactor ${index}`);
  }
  await expect(tooltip).not.toContainText("more");

  // A group Core only partly names says so instead of pretending.
  await seedTimelineItems(page, [reactedItem(60, 50)]);
  await chip.hover();
  await expect(tooltip).toContainText("10 more");
  const metrics = await tooltip.evaluate((element) => ({
    client: element.clientHeight,
    scroll: element.scrollHeight,
    overflowing: getComputedStyle(element).overflowY
  }));
  // Fifty names outgrow the box, so it wraps and scrolls rather than covering
  // the timeline.
  expect(metrics.scroll).toBeGreaterThan(metrics.client);
  expect(metrics.overflowing).toBe("auto");
});

test("a tooltip near the pane's top edge is placed where the pane can show it", async ({
  page
}) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [reactedItem(60, 50)]);

  // The single row is the topmost row, so there is no room above it: the box has
  // to open downwards or the pane's `overflow: hidden` would cut off the first
  // names - including the reader's own.
  const chip = page.locator("[data-testid=timeline-view] .reaction-pill").first();
  await chip.hover();
  const tooltip = chip.locator(".reaction-tooltip");
  await expect(tooltip).toBeVisible();
  const geometry = await tooltip.evaluate((element) => {
    const box = element.getBoundingClientRect();
    const pane = element.closest(".message-list")?.getBoundingClientRect() ?? null;
    return {
      top: box.top,
      bottom: box.bottom,
      paneTop: pane?.top ?? null,
      paneBottom: pane?.bottom ?? null,
      scrollTop: element.scrollTop
    };
  });
  expect(geometry.paneTop).not.toBeNull();
  expect(geometry.top).toBeGreaterThanOrEqual(geometry.paneTop ?? 0);
  expect(geometry.bottom).toBeLessThanOrEqual(geometry.paneBottom ?? 0);
  // Its own scroll starts at the top, so the first names are the visible ones.
  expect(geometry.scrollTop).toBe(0);
});

test("a right-click opens the reactor list, and the chip still toggles on a click", async ({
  page
}) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [reactedItem(7, 7)]);

  const chip = page.locator("[data-testid=timeline-view] .reaction-pill").first();
  await expect(chip).toBeVisible();
  await chip.click({ button: "right" });

  const popup = page.locator("body > .receipt-tooltip");
  await expect(popup).toBeVisible();
  await expect(popup).toHaveAttribute("role", "dialog");
  await expect(popup).toHaveAccessibleName(t("timeline.reactionSenders", { key: "🎉" }));
  // Every reactor is a row, with the signed-in user marked.
  await expect(popup.getByRole("listitem")).toHaveCount(7);
  await expect(popup).toContainText("Harness User");
  for (let index = 1; index <= 6; index += 1) {
    await expect(popup).toContainText(`Reactor ${index}`);
  }
  await expect(popup.locator(".reaction-sender-self")).toHaveText(t("people.you"));
  await expect(popup).not.toContainText("more");
  // The message's own context menu did not open instead.
  await expect(page.getByRole("menuitem", { name: t("context.redactMessage") })).toHaveCount(0);

  // Escape closes it.
  await page.keyboard.press("Escape");
  await expect(popup).toHaveCount(0);

  // A click on the chip is still the toggle: it sends this reaction.
  await page.evaluate(() => window.__harness.clearInvocations());
  await chip.click();
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("send_reaction").length))
    .toBe(1);
});

test("the reactor list scrolls, moves with the keyboard and opens a profile", async ({ page }) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [reactedItem(40, 40)]);

  const chip = page.locator("[data-testid=timeline-view] .reaction-pill").first();
  await chip.click({ button: "right" });
  const popup = page.locator("body > .receipt-tooltip");
  await expect(popup.getByRole("listitem")).toHaveCount(40);

  const box = await popup.evaluate((element) => ({
    client: element.clientHeight,
    scroll: element.scrollHeight
  }));
  expect(box.scroll).toBeGreaterThan(box.client);

  // Arrow keys move a focus that follows into view, from the top of the list.
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  const focused = await page.evaluate(() => {
    const row = document.activeElement as HTMLElement | null;
    return row?.querySelector(".receipt-reader-name")?.textContent ?? null;
  });
  expect(focused).toBe("Reactor 2");

  // A row opens that person's profile in the context panel.
  await popup.getByRole("button", { name: "Reactor 1", exact: true }).click();
  const panel = page.getByRole("complementary", { name: t("panel.context") });
  await expect(panel.getByRole("heading", { name: t("panel.profile") })).toBeVisible();
  await expect(panel).toContainText("@reactor-1:example.invalid");
  await expect(popup).toHaveCount(0);
});

test("a read-only chip still opens the reactor list, and still cannot send", async ({ page }) => {
  await gotoReadyShell(page);
  // The read-only shell of #2066: cached content that Rust has not admitted for
  // writing yet. `pushStateUpdate` is what re-derives the access the row reads.
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: { ...snapshot.state.domain, secure_backup_gate: { kind: "checking" } as never },
        ui: {
          ...snapshot.state.ui,
          room_list: {
            ...snapshot.state.ui.room_list,
            readiness: { kind: "loading", source: "cache", generation: 6 } as never
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
  });

  await seedTimelineItems(page, [reactedItem(3, 3)]);
  const shell = page.locator("[data-testid=timeline-view] .reaction-pill-shell").first();
  const chip = shell.locator(".reaction-pill");
  // #2066 keeps the chip unpressed; #2111 keeps the people behind it reachable.
  await expect(chip).toBeDisabled();

  await shell.click({ button: "right" });
  const popup = page.locator("body > .receipt-tooltip");
  await expect(popup.getByRole("listitem")).toHaveCount(3);
  await expect(popup).toContainText("Reactor 2");
  await expect(popup.locator(".reaction-sender-self")).toHaveText(t("people.you"));

  // Pressing the disabled chip sends nothing.
  await page.evaluate(() => window.__harness.clearInvocations());
  await chip.click({ force: true });
  expect(
    await page.evaluate(() => window.__harness.invocationsOf("send_reaction").length)
  ).toBe(0);
});
