/**
 * Headless spec: a mention inside a reply quote renders as the same pill as the
 * message it quotes (#1328, following the mention-pill contract of #874/#1123).
 *
 * The pill is a property of the message that carries `m.mentions`. A reply quote
 * previews another message, so the quote must carry that message's mention facts
 * to the renderer; without them the quoted anchor stays a bare `matrix.to` link.
 */

import { expect, test, type Page } from "@playwright/test";
import { roomTimelineKey } from "../src/domain/coreEvents";

const HARNESS_ACCOUNT_KEY = "@harness-user:example.invalid";
const HARNESS_ROOM_ID = "!harness-room:example.invalid";
const HARNESS_ROOM_KEY = roomTimelineKey(HARNESS_ACCOUNT_KEY, HARNESS_ROOM_ID);
const MENTIONED_USER_ID = "@harness-ada:example.invalid";
const MENTION_LABEL = "Harness Ada";
/** Element's own shape: a bare `matrix.to` href, label without a leading `@`. */
const ELEMENT_MENTION_HREF = `https://matrix.to/#/${MENTIONED_USER_ID}`;
const QUOTED_BODY = "Harness Ada please take a look";

async function gotoReadyShell(page: Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Reply to message" }).first()).toBeVisible();
}

function item(eventId: string, overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    id: { Event: { event_id: eventId } },
    sender: "@harness-bob:example.invalid",
    sender_label: "Harness Bob",
    body: "body",
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

function quote(
  mentions: { mentionedUserIds?: string[]; mentionsRoom?: boolean } = {}
): Record<string, unknown> {
  return {
    event_id: "$quoted:example.invalid",
    sender: MENTIONED_USER_ID,
    sender_label: MENTION_LABEL,
    body_preview: QUOTED_BODY,
    formatted: {
      html: `<a href="${ELEMENT_MENTION_HREF}">${MENTION_LABEL}</a> please take a look`,
      plain_text: QUOTED_BODY,
      code_blocks: []
    },
    state: "ready",
    ...(mentions.mentionedUserIds
      ? { mentioned_user_ids: mentions.mentionedUserIds }
      : {}),
    ...(mentions.mentionsRoom ? { mentions_room: true } : {})
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
            return nextItems.every((entry) =>
              document.querySelector(
                `[data-event-id="${CSS.escape(
                  (entry as { id: { Event: { event_id: string } } }).id.Event.event_id
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

test("a reply quote keeps the quoted message's mention as a pill", async ({ page }) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [
    item("$quoted:example.invalid", {
      body: QUOTED_BODY,
      formatted: {
        html: `<a href="${ELEMENT_MENTION_HREF}">${MENTION_LABEL}</a> please take a look`,
        plain_text: QUOTED_BODY,
        code_blocks: []
      },
      mentioned_user_ids: [MENTIONED_USER_ID]
    }),
    item("$quoter:example.invalid", {
      body: "replying",
      in_reply_to_event_id: "$quoted:example.invalid",
      // The row's own mention is a different user: the quote must pill the
      // user the quoted message named, not the user the row named.
      mentioned_user_ids: ["@harness-bob:example.invalid"],
      reply_quote: quote({ mentionedUserIds: [MENTIONED_USER_ID] })
    })
  ]);

  const quoter = page.locator('[data-event-id="$quoter:example.invalid"]');
  const quoteBody = quoter.locator(".reply-quote-body");
  const pill = quoteBody.locator("a.message-mention-pill");
  await expect(pill).toHaveCount(1);
  await expect(pill).toHaveAttribute("data-mention-user-id", MENTIONED_USER_ID);
  // #1328: the quoted anchor carries the label without `@`, so the pill spells it
  // the way the composer's chip does.
  await expect(pill).toHaveText(`@${MENTION_LABEL}`);
  await expect(quoter).toBeVisible();

  // The message that carries the mention still pills on its own row.
  await expect(
    page.locator('[data-event-id="$quoted:example.invalid"] a.message-mention-pill')
  ).toHaveCount(1);
});

test("a quoted mention pill has no underline, and a quoted link still has one", async ({ page }) => {
  await gotoReadyShell(page);
  const QUOTED_URL = "https://example.invalid/doc";
  const formatted = {
    html:
      `<a href="${ELEMENT_MENTION_HREF}">${MENTION_LABEL}</a> see ` +
      `<a href="${QUOTED_URL}">${QUOTED_URL}</a>`,
    plain_text: `${MENTION_LABEL} see ${QUOTED_URL}`,
    code_blocks: []
  };
  await seedTimelineItems(page, [
    item("$quoted-underlined:example.invalid", {
      body: formatted.plain_text,
      formatted,
      mentioned_user_ids: [MENTIONED_USER_ID]
    }),
    item("$quoter-underlined:example.invalid", {
      body: "replying",
      in_reply_to_event_id: "$quoted-underlined:example.invalid",
      reply_quote: {
        ...quote({ mentionedUserIds: [MENTIONED_USER_ID] }),
        body_preview: formatted.plain_text,
        formatted
      }
    })
  ]);

  // #1960: one mention looked plain in its message and underlined in the quote
  // below it. Both are the same pill, so both must read `none`.
  const decoration = (locator: ReturnType<ReturnType<Page["locator"]>["locator"]>) =>
    locator.evaluate((node) => getComputedStyle(node).textDecorationLine);
  const quotedPill = page.locator(
    '[data-event-id="$quoter-underlined:example.invalid"] .reply-quote-body a.message-mention-pill'
  );
  await expect(quotedPill).toHaveCount(1);
  expect(await decoration(quotedPill)).toBe("none");
  expect(
    await decoration(
      page.locator(
        '[data-event-id="$quoted-underlined:example.invalid"] .message-body a.message-mention-pill'
      )
    )
  ).toBe("none");

  // A quoted URL is still an ordinary link: the pill lost an underline, nothing
  // else in the quote did. The quote body has no link styling of its own, so
  // this `underline` is the engine's own `a` default — the appearance the fix
  // must leave alone, pinned so a future rule cannot quietly start styling
  // quoted links.
  const quotedLink = page.locator(
    '[data-event-id="$quoter-underlined:example.invalid"] .reply-quote-body a:not(.message-mention-pill)'
  );
  await expect(quotedLink).toHaveCount(1);
  expect(await decoration(quotedLink)).toBe("underline");
});

test("a quote without mention facts stays a plain anchor, not a pill", async ({ page }) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [
    item("$quoted-plain:example.invalid", { body: QUOTED_BODY }),
    item("$quoter-plain:example.invalid", {
      body: "replying",
      in_reply_to_event_id: "$quoted-plain:example.invalid",
      // The row itself names a mentioned user, so an empty quote that stayed
      // un-pilled proves the preview reads the quote's facts, not the row's.
      mentioned_user_ids: [MENTIONED_USER_ID],
      reply_quote: quote()
    })
  ]);

  const quoteBody = page.locator('[data-event-id="$quoter-plain:example.invalid"] .reply-quote-body');
  await expect(quoteBody).toBeVisible();
  await expect(quoteBody.locator(".message-mention-pill")).toHaveCount(0);
  await expect(quoteBody).toContainText(QUOTED_BODY);
});
