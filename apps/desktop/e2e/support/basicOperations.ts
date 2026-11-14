import { expect, type Page } from "@playwright/test";

import { roomTimelineKey } from "../../src/domain/coreEvents";

export const HARNESS_ACCOUNT_KEY = "@harness-user:example.invalid";
export const HARNESS_ROOM_ID = "!harness-room:example.invalid";
export const HARNESS_ROOM_KEY = roomTimelineKey(HARNESS_ACCOUNT_KEY, HARNESS_ROOM_ID);

export async function gotoReadyShell(page: Page): Promise<void> {
  await page.goto("/appHarness.html");
  // The signed-in shell renders the three panes (not the AuthScreen).
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  // Wait for the seeded timeline row's reply action (proves the CoreEvent
  // stream + full App are wired) before clearing startup invocations.
  await expect(page.getByRole("button", { name: "Reply to message" }).first()).toBeVisible();
}

export async function invocationCount(page: Page, command: string): Promise<number> {
  return page.evaluate((cmd) => window.__harness.invocationsOf(cmd).length, command);
}

export async function seedTimelineItems(
  page: Page,
  items: unknown[],
  generation = 2,
  key: unknown = HARNESS_ROOM_KEY
): Promise<void> {
  await expect
    .poll(
      async () =>
        page.evaluate(
          async ({ key, nextItems, nextGeneration }) => {
            const itemDomIds = nextItems.map((item) => {
              if ("Transaction" in item.id) {
                return `txn:${item.id.Transaction.transaction_id}`;
              }
              if ("Event" in item.id) {
                return item.id.Event.event_id;
              }
              return `syn:${item.id.Synthetic.synthetic_id}`;
            });
            await window.__harness.pushCoreEvent({
              kind: "Timeline",
              event: {
                InitialItems: {
                  request_id: null,
                  key,
                  generation: nextGeneration,
                  items: nextItems
                }
              }
              // eslint-disable-next-line @typescript-eslint/no-explicit-any
            } as any);
            await new Promise((resolve) => setTimeout(resolve, 25));
            const mounted = (id: string) =>
              document.querySelector(`[data-item-id="${CSS.escape(id)}"]`) !== null;
            if (itemDomIds.every(mounted)) {
              return true;
            }
            // The timeline mounts only the window around the viewport, so a long
            // batch is never all in the DOM. Its tail is at the live edge and the
            // store-side total proves the batch itself landed.
            const total = Number(
              document
                .querySelector('[data-testid="timeline-view"]')
                ?.getAttribute("data-total-items") ?? 0
            );
            return total >= itemDomIds.length && mounted(itemDomIds[itemDomIds.length - 1] ?? "");
          },
          { key, nextItems: items, nextGeneration: generation }
        ),
      { timeout: 10_000, intervals: [25, 50, 100, 250] }
    )
    .toBe(true);
}

export async function pushTimelineDiffs(
  page: Page,
  diffs: unknown[],
  generation = 2,
  batchId = 2
): Promise<void> {
  await page.evaluate(
    async ({ key, nextDiffs, nextGeneration, nextBatchId }) => {
      await window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key,
            generation: nextGeneration,
            batch_id: nextBatchId,
            diffs: nextDiffs
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    { key: HARNESS_ROOM_KEY, nextDiffs: diffs, nextGeneration: generation, nextBatchId: batchId }
  );
}
