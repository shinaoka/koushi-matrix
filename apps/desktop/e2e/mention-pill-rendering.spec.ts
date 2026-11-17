/**
 * Headless spec: the mention pill is a property of the message (#874).
 *
 * A message whose `m.mentions` names a user renders that mention as the
 * composer's pill, taken from the event's own formatted anchor. Text that only
 * looks like a mention — no `m.mentions` — stays plain text for every viewer,
 * whatever profiles the client happens to have loaded.
 */

import { expect, test, type Page } from "@playwright/test";
import { channelDelta, pillAppearance } from "./support/mentionAppearance";
import { t } from "../src/i18n/messages";
import { roomTimelineKey } from "../src/domain/coreEvents";

const HARNESS_ACCOUNT_KEY = "@harness-user:example.invalid";
const HARNESS_ROOM_ID = "!harness-room:example.invalid";
const HARNESS_ROOM_KEY = roomTimelineKey(HARNESS_ACCOUNT_KEY, HARNESS_ROOM_ID);
const MENTIONED_USER_ID = "@harness-ada:example.invalid";
const MENTION_LABEL = "Harness Ada";
const MENTION_URL = `https://matrix.to/#/${encodeURIComponent(MENTIONED_USER_ID)}`;

async function gotoReadyShell(page: Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Reply to message" }).first()).toBeVisible();
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

function itemWithBody(
  eventId: string,
  body: string,
  overrides: Record<string, unknown> = {}
): Record<string, unknown> {
  return {
    id: { Event: { event_id: eventId } },
    sender: "@harness-user:example.invalid",
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
    ...overrides
  };
}

/** A real mention: `m.mentions` plus the formatted anchor it produced. */
function realMentionItem(eventId: string): Record<string, unknown> {
  const body = `@${MENTION_LABEL} please take a look`;
  return itemWithBody(eventId, body, {
    formatted: {
      html: `<a href="${MENTION_URL}">@${MENTION_LABEL}</a> please take a look`,
      plain_text: body,
      code_blocks: []
    },
    mentioned_user_ids: [MENTIONED_USER_ID]
  });
}

/**
 * The same mention as Element writes it: the anchor text carries the label
 * without `@`, and the `href` is not percent-encoded (#1328).
 */
function elementMentionItem(eventId: string): Record<string, unknown> {
  const elementUrl = `https://matrix.to/#/${MENTIONED_USER_ID}`;
  return itemWithBody(eventId, `${MENTION_LABEL} please take a look`, {
    formatted: {
      html: `<a href="${elementUrl}">${MENTION_LABEL}</a> please take a look`,
      plain_text: `${MENTION_LABEL} please take a look`,
      code_blocks: []
    },
    mentioned_user_ids: [MENTIONED_USER_ID]
  });
}

test("a message that mentions a user renders that mention as a pill", async ({ page }) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [
    realMentionItem("$mentioned:example.invalid"),
    // Same visible text, no `m.mentions`: it notifies nobody and is not a pill.
    itemWithBody("$unmentioned:example.invalid", `@${MENTION_LABEL} please take a look`)
  ]);

  const mentionedRow = page.locator('[data-event-id="$mentioned:example.invalid"]');
  const pill = mentionedRow.locator("a.message-mention-pill");
  await expect(pill).toHaveCount(1);
  await expect(pill).toHaveAttribute("data-mention-user-id", MENTIONED_USER_ID);

  const unmentionedRow = page.locator('[data-event-id="$unmentioned:example.invalid"]');
  await expect(unmentionedRow).toBeVisible();
  await expect(unmentionedRow.locator(".message-mention-pill")).toHaveCount(0);
  await expect(unmentionedRow).toContainText(`@${MENTION_LABEL} please take a look`);

  // The pill keeps the in-app activation of a mention.
  await pill.click();
  const panel = page.getByRole("complementary", { name: t("panel.context") });
  await expect(panel.getByRole("heading", { name: t("panel.profile") })).toBeVisible();
  await expect(panel).toContainText(MENTIONED_USER_ID);
});

test("a mention pill opens the room member's full Profile without Room info first", async ({
  page
}) => {
  // #1540: the pill used to open the Profile without asking Rust for the
  // room's members, so the room section (membership, role, moderation) stayed
  // missing until Room info happened to load them.
  await gotoReadyShell(page);
  await seedTimelineItems(page, [realMentionItem("$pill-profile:example.invalid")]);

  await page
    .locator('[data-event-id="$pill-profile:example.invalid"] a.message-mention-pill')
    .click();

  const panel = page.getByRole("complementary", { name: t("panel.context") });
  await expect(panel.getByRole("heading", { name: t("panel.profile") })).toBeVisible();
  await expect(panel.getByText(t("people.membership"), { exact: true })).toBeVisible();
  await expect(panel.getByText(t("room.memberRole"), { exact: true })).toBeVisible();
});

test("an Element-shaped mention reads as the composer's pill, not as a link", async ({ page }) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [elementMentionItem("$element-mention:example.invalid")]);

  const row = page.locator('[data-event-id="$element-mention:example.invalid"]');
  const pill = row.locator("a.message-mention-pill");
  await expect(pill).toHaveCount(1);
  await expect(pill).toHaveAttribute("data-mention-user-id", MENTIONED_USER_ID);

  // #1328: the anchor text has no `@`, so the pill spells the mention the way the
  // composer's chip does. The event's own HTML and plain text are untouched: the
  // row text still reads the sender's spelling.
  const appearance = await pillAppearance(pill);
  expect(appearance.text).toBe(`@${MENTION_LABEL}`);
  await expect(row.locator(".message-body")).toContainText(`${MENTION_LABEL} please take a look`);

  // #1328: in a formatted body the pill is an `<a>`, and the formatted-body link
  // rule used to underline it, which is what made it read as an ordinary link.
  expect(appearance.decoration).toBe("none");
  expect(
    channelDelta(appearance.backgroundChannels, appearance.surfaceChannels),
    `pill ${appearance.background} on ${appearance.surface}`
  ).toBeGreaterThanOrEqual(8);
});

/** A mention whose formatted anchor carries `label` (Element spelling, no `@`). */
function labelledMentionItem(eventId: string, label: string): Record<string, unknown> {
  const elementUrl = `https://matrix.to/#/${MENTIONED_USER_ID}`;
  return itemWithBody(eventId, `${label}: please take a look`, {
    formatted: {
      html: `<a href="${elementUrl}">${label}</a>: please take a look`,
      plain_text: `${label}: please take a look`,
      code_blocks: []
    },
    mentioned_user_ids: [MENTIONED_USER_ID]
  });
}

/** The pill's box, the message body's box, and the text boxes on either side. */
async function pillGeometry(row: ReturnType<Page["locator"]>) {
  return row.locator("a.message-mention-pill").evaluate((pill) => {
    const body = pill.closest(".message-body")!;
    const style = getComputedStyle(pill);
    const textBox = (node: Node) => {
      const range = document.createRange();
      range.selectNodeContents(node);
      return range.getClientRects()[0]!;
    };
    const pillText = [...pill.childNodes].find((node) => node.nodeType === Node.TEXT_NODE)!;
    const after = pill.nextSibling!;
    return {
      text: pill.textContent ?? "",
      clipped: pill.scrollWidth > pill.clientWidth + 1,
      display: style.display,
      overflow: style.overflowX,
      textOverflow: style.textOverflow,
      whiteSpace: style.whiteSpace,
      pillRight: pill.getBoundingClientRect().right,
      bodyRight: body.getBoundingClientRect().right,
      pillTextBottom: textBox(pillText).bottom,
      afterTextBottom: textBox(after).bottom
    };
  });
}

// #1950: Element's pill is an inline-block capped only by its line
// (`max-width: 100%`) that ends a name that cannot fit in an ellipsis. Ours was an
// inline-flex box capped at 240px: an ordinary "Name (Affiliation)" display name
// was cut mid-word, and `text-overflow` never applies to a flex container, so the
// cut had no ellipsis and read as a wrong name.
test("a mention pill shows an ordinary display name in full", async ({ page }) => {
  const label = "Harness Ada Lovelace (Analytical Engine Society)";
  await gotoReadyShell(page);
  await seedTimelineItems(page, [labelledMentionItem("$full-name:example.invalid", label)]);

  const geometry = await pillGeometry(page.locator('[data-event-id="$full-name:example.invalid"]'));
  expect(geometry.text).toBe(`@${label}`);
  expect(geometry.clipped, "an ordinary name fits the row, so the pill must not clip it").toBe(
    false
  );
  // The pill's text sits on the line's baseline, like the text after it.
  expect(Math.abs(geometry.pillTextBottom - geometry.afterTextBottom)).toBeLessThanOrEqual(1);
});

test("a mention pill too wide for its line ends in an ellipsis inside the row", async ({
  page
}) => {
  const label = `Harness ${"Very Long Display Name ".repeat(12).trim()}`;
  await gotoReadyShell(page);
  await seedTimelineItems(page, [labelledMentionItem("$long-name:example.invalid", label)]);

  const geometry = await pillGeometry(page.locator('[data-event-id="$long-name:example.invalid"]'));
  expect(geometry.text).toBe(`@${label}`);
  expect(geometry.clipped).toBe(true);
  expect(geometry.pillRight).toBeLessThanOrEqual(geometry.bodyRight + 0.5);
  // `text-overflow` paints its ellipsis only for a block container's own inline
  // content; on a flex container the cut is silent.
  expect(["block", "inline-block"]).toContain(geometry.display);
  expect(geometry.overflow).toBe("hidden");
  expect(geometry.textOverflow).toBe("ellipsis");
  expect(geometry.whiteSpace).toBe("nowrap");
});
