/**
 * Headless spec: per-message timeline state (#83).
 *
 * Proves that TimelineItemRow renders Rust-owned state correctly and never
 * invents its own send/read/edit semantics:
 *
 *  1. Timestamp is rendered for event-backed rows.
 *  2. Send-state marks — sending / notSent / cancelled — label/aria rendered.
 *  3. Sent mark (check icon) appears for send_state=sent, not for other states.
 *  4. Read receipts: 1 reader shows avatar/initials and count.
 *  5. Read receipts: multiple readers show stacked avatars and count.
 *  6. Read receipts: overflow count (+N) when receiptOverflowCount > 0.
 *  7. Edited marker present after is_edited flag set; re-edit dispatches edit_message.
 *  8. Reply does not surface for a non-message (no body) timeline row.
 */

import { expect, test } from "@playwright/test";
import { t } from "../src/i18n/messages";
import { roomTimelineKey } from "../src/domain/coreEvents";

const HARNESS_ACCOUNT_KEY = "@harness-user:example.invalid";
const HARNESS_ROOM_ID = "!harness-room:example.invalid";
const HARNESS_ROOM_KEY = roomTimelineKey(HARNESS_ACCOUNT_KEY, HARNESS_ROOM_ID);

async function gotoReadyShell(page: import("@playwright/test").Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Reply to message" }).first()).toBeVisible();
}

async function seedTimelineItems(
  page: import("@playwright/test").Page,
  items: unknown[],
  generation = 2,
  requestId: { connection_id: number; sequence: number } | null = null
): Promise<void> {
  await expect
    .poll(
      async () =>
        page.evaluate(
          async ({ key, nextItems, nextGeneration, nextRequestId }) => {
            const itemDomIds = nextItems.map((item) => {
              if ("Transaction" in (item as { id: Record<string, unknown> }).id) {
                return `txn:${(item as { id: { Transaction: { transaction_id: string } } }).id.Transaction.transaction_id}`;
              }
              if ("Event" in (item as { id: Record<string, unknown> }).id) {
                return (item as { id: { Event: { event_id: string } } }).id.Event.event_id;
              }
              return `syn:${(item as { id: { Synthetic: { synthetic_id: string } } }).id.Synthetic.synthetic_id}`;
            });
            await window.__harness.pushCoreEvent({
              kind: "Timeline",
              event: {
                InitialItems: {
                  request_id: nextRequestId,
                  key,
                  generation: nextGeneration,
                  items: nextItems
                }
              }
              // eslint-disable-next-line @typescript-eslint/no-explicit-any
            } as any);
            await new Promise((resolve) => setTimeout(resolve, 25));
            return itemDomIds.every((id) =>
              document.querySelector(`[data-item-id="${CSS.escape(id)}"]`)
            );
          },
          { key: HARNESS_ROOM_KEY, nextItems: items, nextGeneration: generation, nextRequestId: requestId }
        ),
      { timeout: 10_000, intervals: [25, 50, 100, 250] }
    )
    .toBe(true);
}

function makeEventItem(
  eventId: string,
  overrides: Record<string, unknown> = {}
): Record<string, unknown> {
  return {
    id: { Event: { event_id: eventId } },
    sender: "@harness-user:example.invalid",
    sender_label: "Harness User",
    body: "Test message body",
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

function makeTransactionItem(
  transactionId: string,
  sendState: Record<string, unknown>,
  overrides: Record<string, unknown> = {}
): Record<string, unknown> {
  return {
    id: { Transaction: { transaction_id: transactionId } },
    sender: "@harness-user:example.invalid",
    sender_label: "Harness User",
    body: "Outbound message",
    timestamp_ms: 1_800_000_200_000,
    in_reply_to_event_id: null,
    thread_root: null,
    thread_summary: null,
    reactions: [],
    can_react: false,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false,
    send_state: sendState,
    ...overrides
  };
}

// ---------------------------------------------------------------------------
// 1. Timestamp rendering
// ---------------------------------------------------------------------------

test("timestamp is visible on event-backed message rows", async ({ page }) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [makeEventItem("$ts-test:example.invalid")]);
  const article = page.locator('[data-event-id="$ts-test:example.invalid"]');
  await expect(article).toBeVisible();
  // The <time> element carries the formatted timestamp.
  const timeEl = article.locator("time.message-timestamp");
  await expect(timeEl).toBeVisible();
  // Timestamp must be non-empty (locale-formatted time string).
  const text = await timeEl.textContent();
  expect(text?.trim().length).toBeGreaterThan(0);
});

// ---------------------------------------------------------------------------
// 2. Send-state labels for outbound items
// ---------------------------------------------------------------------------

test("sending state renders label and cancel button", async ({ page }) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [
    makeTransactionItem("txn-sending-1", { kind: "sending" })
  ]);
  const article = page.locator('[data-item-id="txn:txn-sending-1"]');
  await expect(article).toBeVisible();
  // Label rendered in message heading.
  await expect(article.getByText(t("timeline.sending"))).toBeVisible();
  // Cancel button available while sending.
  await expect(
    article.getByRole("button", { name: t("timeline.cancelSend") })
  ).toBeVisible();
});

test("notSent state renders label, retry and delete buttons", async ({ page }) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [
    makeTransactionItem("txn-notsent-1", { kind: "notSent", reason: "recoverable" })
  ]);
  const article = page.locator('[data-item-id="txn:txn-notsent-1"]');
  await expect(article).toBeVisible();
  await expect(article.getByText(t("timeline.notSent"))).toBeVisible();
  await expect(
    article.getByRole("button", { name: t("timeline.resendSend") })
  ).toBeVisible();
  await expect(
    article.getByRole("button", { name: t("timeline.deleteSend") })
  ).toBeVisible();
});

test("cancelled state renders cancelled label", async ({ page }) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [
    makeTransactionItem("txn-cancelled-1", { kind: "cancelled" })
  ]);
  const article = page.locator('[data-item-id="txn:txn-cancelled-1"]');
  await expect(article).toBeVisible();
  await expect(article.getByText(t("timeline.cancelledSend"))).toBeVisible();
});

// ---------------------------------------------------------------------------
// 3. Sent mark (check icon) for send_state=sent
// ---------------------------------------------------------------------------

test("sent mark is rendered for send_state=sent rows", async ({ page }) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [
    makeTransactionItem("txn-sent-1", { kind: "sent" })
  ]);
  const article = page.locator('[data-item-id="txn:txn-sent-1"]');
  await expect(article).toBeVisible();
  const sentMark = article.locator('.message-send-state[data-send-state="sent"]');
  await expect(sentMark).toBeVisible();
  // Must carry an accessible label, not just a visual icon.
  await expect(sentMark).toHaveAttribute("aria-label", t("timeline.sent"));
});

test("sent mark is absent for non-sent rows", async ({ page }) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [makeEventItem("$no-sent-mark:example.invalid")]);
  const article = page.locator('[data-event-id="$no-sent-mark:example.invalid"]');
  await expect(article).toBeVisible();
  // Event-backed rows (no send_state) must not show the sent mark.
  await expect(
    article.locator('.message-send-state[data-send-state="sent"]')
  ).toHaveCount(0);
});

// ---------------------------------------------------------------------------
// 4. Read receipts — single reader
// ---------------------------------------------------------------------------

test("single read receipt renders avatar initials and count", async ({ page }) => {
  await gotoReadyShell(page);
  const eventId = "$receipt-single:example.invalid";
  await seedTimelineItems(page, [makeEventItem(eventId)]);

  // Push a live-signals snapshot with one reader for this event.
  await page.evaluate(
    ({ roomId, evId }) => {
      const snap = window.__harness.currentSnapshot();
      window.__harness.setSnapshot({
        ...snap,
        state: {
          ...snap.state,
          domain: {
            ...snap.state.domain,
            live_signals: {
              ...snap.state.domain.live_signals,
              rooms: {
                [roomId]: {
                  receipts_by_event: {
                    [evId]: {
                      readers: [
                        {
                          user_id: "@reader-a:example.invalid",
                          display_name: "Alice Reader",
                          original_display_label: "Alice Reader",
                          avatar: null,
                          timestamp_ms: null
                        }
                      ],
                      total_count: 1,
                      overflow_count: 0
                    }
                  },
                  fully_read_event_id: null,
                  typing_user_ids: [],
                  typing_users: []
                }
              }
            }
          }
        }
      });
      window.__harness.pushStateUpdate();
    },
    { roomId: "!harness-room:example.invalid", evId: eventId }
  );

  const article = page.locator(`[data-event-id="${eventId}"]`);
  const receipts = article.locator(".message-receipts");
  await expect(receipts).toBeVisible();
  // Receipt aria-label includes count.
  await expect(receipts).toHaveAttribute("aria-label", /1/);
  // Initials rendered (Alice Reader → AR).
  await expect(article.locator(".receipt-reader-avatar")).toBeVisible();
});

// ---------------------------------------------------------------------------
// 5. Read receipts — multiple readers
// ---------------------------------------------------------------------------

test("reader avatar observations follow real popup clipping and stop on close", async ({ page }) => {
  await gotoReadyShell(page);
  const eventId = "$receipt-geometry:example.invalid";
  await seedTimelineItems(page, [makeEventItem(eventId)], 2, { connection_id: 1, sequence: 2 });
  await page.evaluate(({ roomId, eventId }) => {
    const readers = Array.from({ length: 80 }, (_, index) => ({
      user_id: `@geometry-${index}:example.invalid`, display_name: `Reader ${index}`,
      original_display_label: `Reader ${index}`, avatar: null, timestamp_ms: null
    }));
    const snap = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({ ...snap, state: { ...snap.state, domain: {
      ...snap.state.domain, live_signals: { ...snap.state.domain.live_signals, rooms: {
        ...snap.state.domain.live_signals.rooms,
        [roomId]: { receipts_by_event: { [eventId]: { readers: readers.slice(0, 3), total_count: 80, overflow_count: 77 } },
          fully_read_event_id: null, typing_user_ids: [], typing_users: [] }
      } }
    } } });
    window.__harness.pushStateUpdate();
    let source: unknown;
    let delivered = false;
    window.__harness.setCommandResponse("subscribe_receipt_reader", (args: { source: unknown }) => {
      source = args.source;
      return "geometry-scope";
    });
    window.__harness.setCommandResponse("receive_receipt_reader", () => {
      if (delivered) return null;
      delivered = true;
      return { kind: "model", scope: "geometry-scope", revision: "7", model: {
        kind: "readerReady", source, total_count: 80, start: 0, window_sequence: "0",
        source_revision: "1", dependency_revision: "1", resolved_anchor: { kind: "notRequested" },
        rows: readers.map((reader) => ({ user_id: reader.user_id, display_label: reader.display_name,
          original_display_label: reader.display_name, initials: "R", timestamp: null,
          avatar: { kind: "notRequested" } }))
      } };
    });
  }, { roomId: HARNESS_ROOM_ID, eventId });
  await page.locator(`[data-event-id="${eventId}"] .message-receipts`).focus();
  const popup = page.getByRole("dialog", { name: "Read by 80" });
  await expect(popup).toBeVisible();
  const matchesGeometry = () => page.evaluate(() => {
    const popup = document.querySelector(".receipt-tooltip");
    const request = window.__harness.invocationsOf("observe_receipt_reader_avatars").at(-1)?.args.request;
    if (!popup || !request) return false;
    const bounds = popup.getBoundingClientRect();
    const visible = Array.from(popup.querySelectorAll("[aria-posinset]")).filter((row) => {
      const rect = row.getBoundingClientRect();
      return rect.bottom > bounds.top && rect.top < bounds.bottom;
    }).map((row) => `@geometry-${Number(row.getAttribute("aria-posinset")) - 1}:example.invalid`);
    return visible.length > 0 && visible.length < 80 && request.installed_revision === "7"
      && JSON.stringify(request.visible_user_ids) === JSON.stringify(visible)
      && request.prefetch_user_ids.length === 8;
  });
  await expect.poll(matchesGeometry).toBe(true);
  const before = await page.evaluate(() => window.__harness.invocationsOf("observe_receipt_reader_avatars").at(-1)?.args.request);
  await popup.evaluate((element) => { element.scrollTop = 200; });
  await expect.poll(matchesGeometry).toBe(true);
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("observe_receipt_reader_avatars").at(-1)?.args.request.visible_user_ids[0])).not.toBe(before.visible_user_ids[0]);
  await popup.getByRole("button", { name: t("shortcut.closeDialogOrMenu") }).click();
  await expect(popup).not.toBeVisible();
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("close_receipt_reader").length)).toBe(1);
  const count = await page.evaluate(() => window.__harness.invocationsOf("observe_receipt_reader_avatars").length);
  await page.evaluate(async () => {
    window.dispatchEvent(new Event("resize"));
    await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
  });
  expect(await page.evaluate(() => window.__harness.invocationsOf("observe_receipt_reader_avatars").length)).toBe(count);
});

test("multiple read receipts render stacked avatars", async ({ page }) => {
  await gotoReadyShell(page);
  const eventId = "$receipt-multi:example.invalid";
  await seedTimelineItems(page, [makeEventItem(eventId)]);

  await page.evaluate(
    ({ roomId, evId }) => {
      const snap = window.__harness.currentSnapshot();
      window.__harness.setSnapshot({
        ...snap,
        state: {
          ...snap.state,
          domain: {
            ...snap.state.domain,
            live_signals: {
              ...snap.state.domain.live_signals,
              rooms: {
                [roomId]: {
                  receipts_by_event: {
                    [evId]: {
                      readers: [
                        {
                          user_id: "@reader-b:example.invalid",
                          display_name: "Bob",
                          original_display_label: "Bob",
                          avatar: null,
                          timestamp_ms: null
                        },
                        {
                          user_id: "@reader-c:example.invalid",
                          display_name: "Carol",
                          original_display_label: "Carol",
                          avatar: null,
                          timestamp_ms: null
                        }
                      ],
                      total_count: 2,
                      overflow_count: 0
                    }
                  },
                  fully_read_event_id: null,
                  typing_user_ids: [],
                  typing_users: []
                }
              }
            }
          }
        }
      });
      window.__harness.pushStateUpdate();
    },
    { roomId: "!harness-room:example.invalid", evId: eventId }
  );

  const article = page.locator(`[data-event-id="${eventId}"]`);
  const receipts = article.locator(".message-receipts");
  await expect(receipts).toBeVisible();
  await expect(receipts).toHaveAttribute("aria-label", /2/);
  // Two avatar elements.
  await expect(article.locator(".receipt-reader-avatar")).toHaveCount(2);
});

// ---------------------------------------------------------------------------
// 6. Read receipts — overflow count
// ---------------------------------------------------------------------------

test("overflow count is shown when overflow_count > 0", async ({ page }) => {
  await gotoReadyShell(page);
  const eventId = "$receipt-overflow:example.invalid";
  await seedTimelineItems(page, [makeEventItem(eventId)]);

  await page.evaluate(
    ({ roomId, evId }) => {
      const snap = window.__harness.currentSnapshot();
      window.__harness.setSnapshot({
        ...snap,
        state: {
          ...snap.state,
          domain: {
            ...snap.state.domain,
            live_signals: {
              ...snap.state.domain.live_signals,
              rooms: {
                [roomId]: {
                  receipts_by_event: {
                    [evId]: {
                      readers: [
                        {
                          user_id: "@reader-d:example.invalid",
                          display_name: "Diana",
                          original_display_label: "Diana",
                          avatar: null,
                          timestamp_ms: null
                        }
                      ],
                      total_count: 6,
                      overflow_count: 5
                    }
                  },
                  fully_read_event_id: null,
                  typing_user_ids: [],
                  typing_users: []
                }
              }
            }
          }
        }
      });
      window.__harness.pushStateUpdate();
    },
    { roomId: "!harness-room:example.invalid", evId: eventId }
  );

  const article = page.locator(`[data-event-id="${eventId}"]`);
  await expect(article.locator(".receipt-overflow")).toBeVisible();
  await expect(article.locator(".receipt-overflow")).toContainText("+5");
});

// ---------------------------------------------------------------------------
// 7. Edited marker and re-edit capability
// ---------------------------------------------------------------------------

test("edited marker is shown and row is re-editable when is_edited=true and can_edit=true", async ({
  page
}) => {
  await gotoReadyShell(page);
  const eventId = "$edited-item:example.invalid";
  await seedTimelineItems(page, [
    makeEventItem(eventId, { is_edited: true, can_edit: true, body: "Original body" })
  ]);

  const article = page.locator(`[data-event-id="${eventId}"]`);
  await expect(article).toBeVisible();
  // Edited label present in the heading.
  await expect(article.getByText(t("timeline.editedMessage"))).toBeVisible();
  // Edit action present (hover region).
  await expect(
    article.getByRole("button", { name: t("timeline.editMessage") })
  ).toBeVisible();
});

test("re-edit submits edit_message command with updated body", async ({ page }) => {
  await gotoReadyShell(page);
  const eventId = "$re-edit-item:example.invalid";
  await seedTimelineItems(page, [
    makeEventItem(eventId, { is_edited: true, can_edit: true, body: "First edit body" })
  ]);

  await page.evaluate(() => window.__harness.clearInvocations());

  const article = page.locator(`[data-event-id="${eventId}"]`);
  await article.getByRole("button", { name: t("timeline.editMessage") }).click();

  const editTextarea = article.getByRole("textbox", { name: t("timeline.editBody") });
  await expect(editTextarea).toBeVisible();
  await editTextarea.fill("Second edit body");
  await article.getByRole("button", { name: t("timeline.saveEdit") }).click();

  await expect
    .poll(() => page.evaluate(() => window.__harness.invocationsOf("edit_message").length))
    .toBeGreaterThanOrEqual(1);
  const args = await page.evaluate(
    () => window.__harness.invocationsOf("edit_message")[0]?.args
  );
  expect(args).toMatchObject({
    document: { version: 2, inlines: [{ kind: "text", text: "Second edit body" }] }
  });
});

test("message edit Shift+Enter shows and saves a newline", async ({ page }) => {
  await gotoReadyShell(page);
  const eventId = "$edit-newline-item:example.invalid";
  await seedTimelineItems(page, [
    makeEventItem(eventId, { can_edit: true, body: "Line oneLine two" })
  ]);
  await page.evaluate(() => window.__harness.clearInvocations());

  const article = page.locator(`[data-event-id="${eventId}"]`);
  await article.getByRole("button", { name: t("timeline.editMessage") }).click();
  const editTextarea = article.getByRole("textbox", { name: t("timeline.editBody") });
  await expect(editTextarea).toBeVisible();
  await editTextarea.evaluate((element) => {
    const text = element.querySelector("[data-composer-text]")?.firstChild;
    if (!text) throw new Error("edit text node missing");
    const range = document.createRange();
    range.setStart(text, "Line one".length);
    range.collapse(true);
    const selection = window.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);
  });
  await editTextarea.press("Shift+Enter");

  await expect(editTextarea).toHaveText("Line one\nLine two");
  await article.getByRole("button", { name: t("timeline.saveEdit") }).click();

  await expect
    .poll(() => page.evaluate(() => window.__harness.invocationsOf("edit_message").length))
    .toBeGreaterThanOrEqual(1);
  const args = await page.evaluate(
    () => window.__harness.invocationsOf("edit_message")[0]?.args
  );
  expect(args).toMatchObject({
    document: { version: 2, inlines: [{ kind: "text", text: "Line one\nLine two" }] }
  });
});

// ---------------------------------------------------------------------------
// 8. Reply not shown for null-body items
// ---------------------------------------------------------------------------

test("reply action is absent for rows with null body", async ({ page }) => {
  await gotoReadyShell(page);
  const eventId = "$no-reply-row:example.invalid";
  await seedTimelineItems(page, [makeEventItem(eventId, { body: null })]);

  const article = page.locator(`[data-event-id="${eventId}"]`);
  await expect(article).toBeVisible();
  await expect(
    article.getByRole("button", { name: t("timeline.replyToMessage") })
  ).toHaveCount(0);
});

// ---------------------------------------------------------------------------
// 9. Read receipts bottom-right alignment
// ---------------------------------------------------------------------------

test("read receipt row is right-aligned within the message column", async ({ page }) => {
  // display:flex (block-level) on .message-receipts lets margin-inline-start:auto
  // push the row to the right edge of .message-main.  display:inline-flex was
  // inline-level so the margin had no pushing effect.
  await gotoReadyShell(page);
  const eventId = "$receipt-align:example.invalid";
  await seedTimelineItems(page, [makeEventItem(eventId)]);

  // Seed a single reader so .message-receipts is rendered.
  await page.evaluate(
    ({ roomId, evId }) => {
      const snap = window.__harness.currentSnapshot();
      window.__harness.setSnapshot({
        ...snap,
        state: {
          ...snap.state,
          domain: {
            ...snap.state.domain,
            live_signals: {
              ...snap.state.domain.live_signals,
              rooms: {
                [roomId]: {
                  receipts_by_event: {
                    [evId]: {
                      readers: [
                        {
                          user_id: "@reader-align:example.invalid",
                          display_name: "Align Tester",
                          original_display_label: "Align Tester",
                          avatar: null,
                          timestamp_ms: null
                        }
                      ],
                      total_count: 1,
                      overflow_count: 0
                    }
                  },
                  fully_read_event_id: null,
                  typing_user_ids: [],
                  typing_users: []
                }
              }
            }
          }
        }
      });
      window.__harness.pushStateUpdate();
    },
    { roomId: HARNESS_ROOM_ID, evId: eventId }
  );

  const article = page.locator(`[data-event-id="${eventId}"]`);
  const receipts = article.locator(".message-receipts");
  await expect(receipts).toBeVisible();

  // The receipt row right edge must align with the message-main column right
  // edge (i.e. margin-inline-start:auto right-aligns the block-flex row).
  // We allow 2 px tolerance for sub-pixel rounding.
  const receiptBox = await receipts.boundingBox();
  const messageMains = article.locator(".message-main");
  const mainBox = await messageMains.boundingBox();

  expect(receiptBox).not.toBeNull();
  expect(mainBox).not.toBeNull();

  const receiptRight = (receiptBox!.x + receiptBox!.width);
  const mainRight = (mainBox!.x + mainBox!.width);
  expect(Math.abs(receiptRight - mainRight)).toBeLessThanOrEqual(2);

  // Also confirm the receipt row does NOT start at the left edge of message-main
  // (it should be pushed right, not left-aligned).
  expect(receiptBox!.x).toBeGreaterThan(mainBox!.x);
});
