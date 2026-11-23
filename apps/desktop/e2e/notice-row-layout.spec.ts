/**
 * Headless spec: a notice is an information line, not a message (#2041).
 *
 * Runs on the full-app harness, so the real stylesheet is in effect: this is the
 * tier that can measure the compact row. `e2e/message-types.spec.ts` covers the
 * same row's DOM on the fixed-geometry timeline harness.
 */

import { expect, test, type Page } from "@playwright/test";
import { roomTimelineKey } from "../src/domain/coreEvents";
import { t } from "../src/i18n/messages";

const ACCOUNT_KEY = "@harness-user:example.invalid";
const ROOM_ID = "!harness-room:example.invalid";
const ROOM_KEY = roomTimelineKey(ACCOUNT_KEY, ROOM_ID);
const SENDER = "@harness-user:example.invalid";

async function gotoReadyShell(page: Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  await expect(page.getByRole("button", { name: t("timeline.replyToMessage") }).first()).toBeVisible();
}

function item(
  eventId: string,
  body: string,
  overrides: Record<string, unknown> = {}
): Record<string, unknown> {
  return {
    id: { Event: { event_id: eventId } },
    sender: SENDER,
    sender_label: "Harness User",
    body,
    timestamp_ms: 1_800_000_100_000,
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
    actions: {
      can_copy: true,
      can_forward: true,
      can_reply: true,
      can_pin: true,
      can_permalink: true,
      can_view_source: true
    },
    ...overrides
  };
}

/** A room-state notice: its text carries no sender, the line does. */
function topicNotice(eventId: string): Record<string, unknown> {
  return item(eventId, "topic", {
    message_kind: "notice",
    notice_i18n: { key: "timeline.notice.roomTopicSet", value: "Topic" }
  });
}

/** A profile notice: its sentence embeds the sender, so the line must not repeat it. */
function profileNotice(eventId: string): Record<string, unknown> {
  return item(eventId, "changed their display name", {
    message_kind: "notice",
    notice_i18n: { key: "timeline.notice.profileAvatarChanged" }
  });
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
            return nextItems.every((nextItem) =>
              document.querySelector(
                `[data-event-id="${CSS.escape(
                  (nextItem as { id: { Event: { event_id: string } } }).id.Event.event_id
                )}"]`
              )
            );
          },
          { key: ROOM_KEY, nextItems: items }
        ),
      { timeout: 10_000, intervals: [25, 50, 100, 250] }
    )
    .toBe(true);
}

test("a notice is one muted line with no avatar slot and no message actions", async ({ page }) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [
    item("$plain:example.invalid", "An ordinary message"),
    topicNotice("$topic-notice:example.invalid")
  ]);

  const row = page.locator('[data-event-id="$topic-notice:example.invalid"]');
  const plain = page.locator('[data-event-id="$plain:example.invalid"]');
  await expect(row).toBeVisible();
  await expect(row).toHaveClass(/message-notice-row/);
  await expect(row.locator(".avatar")).toHaveCount(0);

  // The line starts where message text starts: the avatar column stays in the
  // grid as an empty track, so the sender name lands on the message text column.
  const lineLeft = await row
    .locator(".message-heading")
    .evaluate((el) => el.getBoundingClientRect().left);
  const plainBodyLeft = await plain
    .locator(".message-body")
    .evaluate((el) => el.getBoundingClientRect().left);
  expect(Math.abs(lineLeft - plainBodyLeft)).toBeLessThan(1);
  // The body is inline, so it continues after the sender name on that line.
  const bodyLeft = await row
    .locator(".message-body")
    .evaluate((el) => el.getBoundingClientRect().left);
  expect(bodyLeft).toBeGreaterThan(lineLeft);

  // One line: the sender and the body share a text band, and the whole row is
  // shorter than the message row above it.
  const geometry = await row.evaluate((el) => {
    const heading = el.querySelector<HTMLElement>(".message-heading")!;
    const body = el.querySelector<HTMLElement>(".message-body")!;
    const style = getComputedStyle(el);
    const bodyStyle = getComputedStyle(body);
    return {
      rowTop: el.getBoundingClientRect().top,
      rowHeight: el.getBoundingClientRect().height,
      heading: heading.getBoundingClientRect().toJSON(),
      body: body.getBoundingClientRect().toJSON(),
      fontSize: Number.parseFloat(bodyStyle.fontSize),
      color: bodyStyle.color,
      bodyBackground: bodyStyle.backgroundColor,
      bodyBorder: bodyStyle.borderTopWidth
    };
  });
  const plainHeight = await plain.evaluate((el) => el.getBoundingClientRect().height);
  expect(geometry.rowHeight).toBeLessThan(plainHeight);
  expect(geometry.fontSize).toBe(12);
  // A notice body is not a box: no background and no border of its own.
  expect(geometry.bodyBackground).toBe("rgba(0, 0, 0, 0)");
  expect(geometry.bodyBorder).toBe("0px");
  expect(Math.abs(geometry.body.top - geometry.heading.top)).toBeLessThan(4);
  // Muted, not the message text colour.
  expect(geometry.color).not.toBe(
    await plain.locator(".message-body").evaluate((el) => getComputedStyle(el).color)
  );

  // No hover action bar at all; the two affordances a notice keeps are on the
  // line itself. Reply, Edit, Redact and Forward have no meaning for a state
  // event, so they are absent.
  await expect(row.locator(".message-actions")).toHaveCount(0);
  await expect(row.locator(".message-notice-controls")).toHaveCount(1);
  const scoped = row;
  await expect(scoped.getByRole("button", { name: t("timeline.replyToMessage") })).toHaveCount(0);
  await expect(scoped.getByRole("button", { name: t("timeline.editMessage") })).toHaveCount(0);
  await expect(scoped.getByRole("button", { name: t("timeline.redactMessage") })).toHaveCount(0);
  await expect(scoped.getByRole("button", { name: t("timeline.addReaction") })).toHaveCount(1);
  await expect(
    scoped.getByRole("button", { name: t("timeline.pinMessage") })
  ).toHaveCount(1);
  // Copy and View source stay reachable from the line's read-only menu, which is
  // what the help page promises for a visible event's details.
  await scoped.getByRole("button", { name: t("timeline.messageActions") }).click();
  const menu = page.getByRole("menu", { name: t("timeline.messageActions") });
  await expect(menu.getByRole("menuitem", { name: t("timeline.copyMessage") })).toBeVisible();
  await expect(menu.getByRole("menuitem", { name: t("timeline.viewSource") })).toBeVisible();
  await expect(menu.getByRole("menuitem", { name: t("timeline.forwardMessage") })).toHaveCount(0);
  await page.keyboard.press("Escape");

  // A topic notice's sentence carries no sender, so the line supplies the space:
  // without it the row reads "Harness User set the room topic to Topic" glued
  // together (#2041 review).
  expect((await row.textContent())?.replace(/\s+/g, " ").trim()).toBe(
    "Harness User set the room topic to Topic"
  );

});

test("a profile notice names its sender once", async ({ page }) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [profileNotice("$profile-notice:example.invalid")]);

  const row = page.locator('[data-event-id="$profile-notice:example.invalid"]');
  await expect(row).toBeVisible();
  // #2041: the sentence embeds the sender, and the line also shows it as the
  // clickable name; the row must still read it once.
  const text = (await row.textContent())?.replace(/\s+/g, " ").trim();
  expect(text).toBe("Harness User changed their profile picture");
  await expect(row.locator(".message-heading")).toHaveCount(1);
});

test("a message after its sender's own notice keeps its avatar and name (#2074)", async ({
  page
}) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [
    topicNotice("$before-notice:example.invalid"),
    item("$after-notice:example.invalid", "First message after the notice"),
    item("$continuation:example.invalid", "Second message from the same sender")
  ]);

  // Compact density is where the defect showed: it is the only density that
  // hides a continuation's avatar and name.
  await page.evaluate(() => {
    document.querySelector<HTMLElement>(".desktop")!.dataset.density = "compact";
  });

  const after = page.locator('[data-event-id="$after-notice:example.invalid"]');
  const continuation = page.locator('[data-event-id="$continuation:example.invalid"]');
  await expect(after).toBeVisible();

  // The message that follows the notice is not a continuation, so the header is
  // drawn: a visible avatar and the sender name as its profile button.
  await expect(after).not.toHaveClass(/is-continuation/);
  await expect(after.locator(".avatar").first()).toBeVisible();
  const nameButton = after.getByRole("button", { name: t("people.openProfile", { name: "Harness User" }) });
  await expect(nameButton).toBeVisible();
  await expect(nameButton).toHaveText("Harness User");

  // The same sender's next message still groups with it, which is the behaviour
  // the notice must not break. Compact density hides a continuation's header by
  // clipping the name to a 1px box rather than removing it, so measure it.
  await expect(continuation).toHaveClass(/is-continuation/);
  await expect(continuation.locator(".avatar").first()).toBeHidden();
  const senderWidth = async (row: typeof after) =>
    await row.locator(".sender").first().evaluate((el) => el.getBoundingClientRect().width);
  expect(await senderWidth(after)).toBeGreaterThan(20);
  expect(await senderWidth(continuation)).toBeLessThanOrEqual(1);

  // The header costs the row height, and that is the price of attributing the
  // message to its real sender; the continuation keeps the compact height.
  const heights = await page.evaluate(() => {
    const height = (eventId: string) =>
      document.querySelector<HTMLElement>(`[data-event-id="${eventId}"]`)!.getBoundingClientRect()
        .height;
    return {
      header: height("$after-notice:example.invalid"),
      continuation: height("$continuation:example.invalid")
    };
  });
  expect(heights.header).toBeGreaterThan(heights.continuation);

  // Default density always drew the header; the row must not have moved.
  await page.evaluate(() => {
    document.querySelector<HTMLElement>(".desktop")!.dataset.density = "default";
  });
  await expect(after).not.toHaveClass(/is-continuation/);
  await expect(after.locator(".avatar").first()).toBeVisible();
});

test("the notice line follows the density setting", async ({ page }) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [topicNotice("$density-notice:example.invalid")]);
  const row = page.locator('[data-event-id="$density-notice:example.invalid"]');
  await expect(row).toBeVisible();

  // The harness default is comfortable; the scale must stay ordered for a notice
  // too, instead of inverting (a notice row that is taller when compact than by
  // default would mean its own fourth scale).
  const paddingFor = async (density: string) =>
    Number.parseFloat(
      await page.evaluate((nextDensity) => {
        const desktop = document.querySelector<HTMLElement>(".desktop")!;
        desktop.dataset.density = nextDensity;
        const notice = document.querySelector<HTMLElement>(
          '[data-event-id="$density-notice:example.invalid"]'
        )!;
        return getComputedStyle(notice).paddingBlock;
      }, density)
    );
  const comfortable = await paddingFor("comfortable");
  const standard = await paddingFor("default");
  const compact = await paddingFor("compact");
  expect(compact).toBeLessThan(standard);
  expect(standard).toBeLessThan(comfortable);
  expect(comfortable).toBeLessThan(14);
});
