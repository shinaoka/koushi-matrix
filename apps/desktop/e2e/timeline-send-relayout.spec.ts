// #2012: a send must settle the timeline once. The read-marker divider used to
// sit above the local echo (the echo cannot take the anchor before the server
// names it), so the echo first painted below the divider and then re-painted
// above it once Core advanced the anchor — two layouts of the same row.
import { expect, test, type Page } from "@playwright/test";

import { HARNESS_ACCOUNT_KEY, HARNESS_ROOM_KEY } from "./support/basicOperations";

const OWN = HARNESS_ACCOUNT_KEY;
const READ = "$own-read:example.invalid";
const SENT = "$own-sent:example.invalid";

function item(id: Record<string, unknown>, overrides: Record<string, unknown> = {}) {
  return {
    id,
    sender: OWN,
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

const readItem = () => item({ Event: { event_id: READ } });
const echoItem = () =>
  item(
    { Transaction: { transaction_id: "txn-1" } },
    { body: "just sent", timestamp_ms: 1_800_000_002_000, send_state: { kind: "sending" } }
  );
const sentItem = () =>
  item({ Event: { event_id: SENT } }, { body: "just sent", timestamp_ms: 1_800_000_002_000 });

/** Per animation frame: which side of the divider the newest row is on. */
async function sampleSendRow(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const samples: string[] = [];
    (window as unknown as { __sendSamples: string[] }).__sendSamples = samples;
    const tick = () => {
      const messages = [...document.querySelectorAll<HTMLElement>(".timeline-view .message")];
      const last = messages[messages.length - 1];
      const id = last?.getAttribute("data-item-id") ?? null;
      if (last && (id === "txn:txn-1" || id === "$own-sent:example.invalid")) {
        const frames = [...document.querySelectorAll<HTMLElement>(".timeline-item-frame")];
        const index = frames.indexOf(last.closest<HTMLElement>(".timeline-item-frame")!);
        const divider = frames.findIndex((frame) => Boolean(frame.querySelector(".read-marker")));
        const side = divider === index ? "after" : divider < index ? "above" : "absent";
        samples.push(`${side}|${last.classList.contains("is-continuation") ? "continuation" : "full"}`);
      }
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
    return [];
  });
}

test("a send lays the new row out once as the read marker settles", async ({ page }) => {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();

  const push = (event: unknown) =>
    page.evaluate((payload) => {
      window.__harness.pushCoreEvent(payload as never);
    }, event);

  await push({
    kind: "Timeline",
    event: {
      InitialItems: { request_id: null, key: HARNESS_ROOM_KEY, generation: 1, items: [readItem()] }
    }
  });
  await push({
    kind: "Timeline",
    event: {
      NavigationUpdated: {
        key: HARNESS_ROOM_KEY,
        snapshot: {
          read_marker_event_id: READ,
          read_marker_display_event_id: READ,
          local_viewed_event_id: READ,
          server_confirmed_read_event_id: READ,
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
  });
  await expect(page.locator(".timeline-view .read-marker")).toHaveCount(1);

  await sampleSendRow(page);

  // 1. the local echo
  await push({
    kind: "Timeline",
    event: {
      ItemsUpdated: {
        key: HARNESS_ROOM_KEY,
        generation: 1,
        batch_id: 2,
        diffs: [{ PushBack: { item: echoItem() } }]
      }
    }
  });
  await expect(page.locator('[data-item-id="txn:txn-1"]')).toBeVisible();
  await page.waitForTimeout(250);

  // 2. confirmation and the anchor advance, published as one batch
  await page.evaluate(
    ({ key, sent, anchor }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          ItemsUpdated: { key, generation: 1, batch_id: 3, diffs: [{ Set: { index: 1, item: sent } }] }
        }
      } as never);
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          NavigationUpdated: {
            key,
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
      } as never);
    },
    { key: HARNESS_ROOM_KEY, sent: sentItem(), anchor: SENT }
  );
  await page.waitForTimeout(400);

  const samples = await page.evaluate(
    () => (window as unknown as { __sendSamples: string[] }).__sendSamples
  );
  expect(samples.length).toBeGreaterThan(20);
  // One shape from the echo until the server confirms it. Two layouts (the row
  // below the divider, then above it) are the #2012 regression.
  expect([...new Set(samples)]).toEqual(["after|continuation"]);
});
