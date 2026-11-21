// #2012: a send must settle the timeline once. The read-marker divider used to
// sit above the local echo (the echo cannot take the anchor before the server
// names it), so the echo first painted below the divider and then re-painted
// above it once Core advanced the anchor — two layouts of the same row.
import { expect, test, type Page } from "@playwright/test";

import {
  HARNESS_ACCOUNT_KEY,
  HARNESS_ROOM_KEY,
  gotoReadyShell,
  seedTimelineItems
} from "./support/basicOperations";

const READ = "$own-read:example.invalid";
const SENT = "$own-sent:example.invalid";
const ECHO = "txn:txn-1";

function item(id: Record<string, unknown>, overrides: Record<string, unknown> = {}) {
  return {
    id,
    sender: HARNESS_ACCOUNT_KEY,
    sender_label: "Harness Sender",
    body: "hello there",
    timestamp_ms: 1_800_000_001_000,
    in_reply_to_event_id: null,
    thread_root: null,
    thread_summary: null,
    media: null,
    reactions: [],
    can_react: true,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false,
    ...overrides
  };
}

const echoItem = () =>
  item(
    { Transaction: { transaction_id: "txn-1" } },
    { body: "just sent", timestamp_ms: 1_800_000_002_000, send_state: { kind: "sending" } }
  );
const sentItem = () =>
  item({ Event: { event_id: SENT } }, { body: "just sent", timestamp_ms: 1_800_000_002_000 });

function navigation(anchor: string) {
  return {
    kind: "Timeline",
    event: {
      NavigationUpdated: {
        key: HARNESS_ROOM_KEY,
        snapshot: {
          read_marker_event_id: anchor,
          read_marker_display_event_id: anchor,
          local_viewed_event_id: anchor,
          server_confirmed_read_event_id: anchor,
          read_state_sync: "synced",
          first_unread_event_id: null,
          first_unread_display_event_id: null,
          unread_event_count: 0,
          unread_position: "unknown",
          newer_event_count: 0,
          can_jump_to_bottom: false
        }
      }
    }
  };
}

function push(page: Page, event: unknown) {
  return page.evaluate((payload) => {
    window.__harness.pushCoreEvent(payload as never);
  }, event);
}

/** Per animation frame: which side of the divider the sent row is on. */
async function sampleSentRow(page: Page, sent: string): Promise<void> {
  await page.evaluate(({ sentId, echoId }) => {
    const samples: string[] = [];
    (window as unknown as { __sendSamples: string[] }).__sendSamples = samples;
    const tick = () => {
      const messages = [...document.querySelectorAll<HTMLElement>(".timeline-view .message")];
      const last = messages[messages.length - 1];
      const id = last?.getAttribute("data-item-id") ?? null;
      if (last && (id === echoId || id === sentId)) {
        const frames = [...document.querySelectorAll<HTMLElement>(".timeline-item-frame")];
        const index = frames.indexOf(last.closest<HTMLElement>(".timeline-item-frame")!);
        // The day heading also carries .read-marker; only the two message
        // dividers have an aria-label.
        const divider = frames.findIndex((frame) =>
          Boolean(frame.querySelector('.read-marker[aria-label]'))
        );
        const side = divider === index ? "after" : divider < index ? "above" : "absent";
        samples.push(
          `${side}|${last.classList.contains("is-continuation") ? "continuation" : "full"}`
        );
      }
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  }, { sentId: sent, echoId: ECHO });
}

/** Seed the previous own message, send, and report what the sent row did. */
async function samplesOfOneSend(page: Page, between: unknown[] = []): Promise<string[]> {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [item({ Event: { event_id: READ } })], 2);
  await push(page, navigation(READ));
  await expect(page.locator(".timeline-view .read-marker")).toHaveCount(1);

  for (const [index, presentation] of between.entries()) {
    await push(page, {
      kind: "Timeline",
      event: {
        ItemsUpdated: {
          key: HARNESS_ROOM_KEY,
          generation: 2,
          batch_id: 2 + index,
          diffs: [{ PushBack: { item: presentation } }]
        }
      }
    });
    // The precondition the spec is about: the heading is mounted between the
    // anchor row and the echo, so a swallowed diff cannot pass this test.
    await expect(page.locator(".timeline-date-divider")).toHaveCount(index + 1);
    expect(
      await page.evaluate(() => {
        const frames = [...document.querySelectorAll<HTMLElement>(".timeline-item-frame")];
        return frames.findIndex((frame) => Boolean(frame.querySelector(".timeline-date-divider")));
      })
    ).toBeGreaterThan(0);
  }

  await sampleSentRow(page, SENT);

  await push(page, {
    kind: "Timeline",
    event: {
      ItemsUpdated: {
        key: HARNESS_ROOM_KEY,
        generation: 2,
        batch_id: 3,
        diffs: [{ PushBack: { item: echoItem() } }]
      }
    }
  });
  await expect(page.locator(`[data-item-id="${ECHO}"]`)).toBeVisible();
  await page.waitForTimeout(250);

  // Confirmation and the anchor advance, published as one batch.
  await page.evaluate(
    ({ key, sent, advance }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          ItemsUpdated: { key, generation: 2, batch_id: 9, diffs: [{ Set: { index: 1, item: sent } }] }
        }
      } as never);
      window.__harness.pushCoreEvent(advance as never);
    },
    { key: HARNESS_ROOM_KEY, sent: sentItem(), advance: navigation(SENT) }
  );
  await page.waitForTimeout(400);

  const samples = await page.evaluate(
    () => (window as unknown as { __sendSamples: string[] }).__sendSamples
  );
  expect(samples.length).toBeGreaterThan(20);
  return samples;
}

test("a send lays the new row out once as the read marker settles", async ({ page }) => {
  // One shape from the echo until the server confirms it. Two layouts (the row
  // below the divider, then above it) are the #2012 regression.
  expect([...new Set(await samplesOfOneSend(page))]).toEqual(["after|continuation"]);
});

test("a send across a date divider keeps the read marker on the new row (#2021)", async ({
  page
}) => {
  // The first message of a new local day: Core's day heading sits between the
  // old anchor and the echo and must not capture the divider. Two layouts here
  // are the #2021 regression.
  const samples = await samplesOfOneSend(page, [
    {
      ...item({ Synthetic: { synthetic_id: "date-divider-1800000002000" } }),
      sender: null,
      body: null
    }
  ]);
  // The day heading still breaks the continuation run (a separate rule), so
  // only the divider's side is compared here.
  expect([...new Set(samples.map((sample) => sample.split("|")[0]))]).toEqual(["after"]);
});
