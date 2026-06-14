/**
 * Headless IPC-contract tier (QA Model layer 4): proves the Task 5 UI drives
 * the right Tauri COMMAND NAMES. Runs the FULL <App /> over a recording mock
 * Tauri IPC transport (appHarness.html → src/test/appHarnessMain.tsx, which
 * installs window.__TAURI_INTERNALS__ via Tauri's official mockIPC so the App
 * selects TauriDesktopApi → invoke()).
 *
 * No Tauri process, no native window, no network: headless Chromium only.
 *
 * Scenarios:
 *   1. Open create-room dialog → submit → invokes `create_room`; dialog closes.
 *   2. Open create-space dialog → submit → invokes `create_space`; dialog closes.
 *   3. Click a timeline row's reply action → invokes `set_composer_reply_target`.
 *   4. Click reaction affordances → invokes `toggle_reaction`.
 *   5. Redact a message → invokes `redact_message` and renders the tombstone.
 *   6. Edit a message → invokes `edit_message`, rejects whitespace-only saves,
 *      and renders the edited marker from the updated timeline row.
 *   7. Submit the composer while the snapshot says reply mode → invokes
 *      `send_reply` (not `send_text`). Reply mode is established by scenario 3's
 *      flow (the set_composer_reply_target response returns a reply-mode
 *      snapshot), then the composer is submitted.
 */

import { expect, test, type Page } from "@playwright/test";

import { focusedTimelineKey, threadTimelineKey } from "../src/domain/coreEvents";
import { t } from "../src/i18n/messages";

function makeThreadItem(index: number, rootEventId = "$seed-event:example.invalid") {
  return {
    id: { Event: { event_id: `$thread-page-${String(index).padStart(2, "0")}:example.invalid` } },
    sender: "@thread-user:example.invalid",
    body: `Thread overflow message ${index}`,
    timestamp_ms: 1_800_000_001_000 + index,
    in_reply_to_event_id: rootEventId,
    thread_root: rootEventId,
    thread_summary: null,
    reactions: [],
    can_react: true,
    is_redacted: false,
    can_redact: false,
    is_edited: false,
    can_edit: false
  };
}

async function gotoReadyShell(page: Page): Promise<void> {
  await page.goto("/appHarness.html");
  // The signed-in shell renders the three panes (not the AuthScreen).
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  // Wait for the seeded timeline row's reply action (proves the CoreEvent
  // stream + full App are wired) before clearing startup invocations.
  await expect(page.getByRole("button", { name: "Reply to message" }).first()).toBeVisible();
}

async function invocationCount(page: Page, command: string): Promise<number> {
  return page.evaluate((cmd) => window.__harness.invocationsOf(cmd).length, command);
}

test("create-room dialog submits create_room and closes on success", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.getByRole("button", { name: "Create room", exact: true }).click();
  const roomNameInput = page.getByRole("textbox", { name: "Room name" });
  await expect(roomNameInput).toBeVisible();

  await roomNameInput.fill("My New Room");
  await page.getByRole("button", { name: "Submit create room" }).click();

  // create_room was invoked.
  await expect.poll(() => invocationCount(page, "create_room")).toBeGreaterThanOrEqual(1);
  // Dialog closed on success (the name input is gone).
  await expect(roomNameInput).toBeHidden();
});

test("create-space dialog submits create_space and closes on success", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.getByRole("button", { name: "Create space", exact: true }).click();
  const spaceNameInput = page.getByRole("textbox", { name: "Space name" });
  await expect(spaceNameInput).toBeVisible();

  await spaceNameInput.fill("My New Space");
  await page.getByRole("button", { name: "Submit create space" }).click();

  await expect.poll(() => invocationCount(page, "create_space")).toBeGreaterThanOrEqual(1);
  await expect(spaceNameInput).toBeHidden();
});

test("timeline reply action invokes set_composer_reply_target", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.getByRole("button", { name: "Reply to message" }).first().click();

  await expect
    .poll(() => invocationCount(page, "set_composer_reply_target"))
    .toBeGreaterThanOrEqual(1);
  // The reply-mode snapshot returned by set_composer_reply_target surfaces the
  // composer reply banner (Cancel reply control), confirming reply mode.
  await expect(page.getByRole("button", { name: "Cancel reply" })).toBeVisible();
});

test("clicking a reaction pill invokes toggle_reaction", async ({ page }) => {
  await gotoReadyShell(page);
  await expect(page.getByRole("button", { name: "Reaction 👍, count 1" }).first()).toBeVisible();
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.getByRole("button", { name: "Reaction 👍, count 1" }).first().click();

  await expect.poll(() => invocationCount(page, "toggle_reaction")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("toggle_reaction")[0]?.args)
    )
    .toEqual({
      roomId: "!harness-room:example.invalid",
      eventId: "$seed-event:example.invalid",
      reactionKey: "👍"
    });
});

test("add reaction picker invokes toggle_reaction with the selected emoji", async ({ page }) => {
  await gotoReadyShell(page);
  await page.locator('[data-event-id="$seed-event:example.invalid"]').hover();
  await expect(page.getByRole("button", { name: "Add reaction" }).first()).toBeVisible();
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.getByRole("button", { name: "Add reaction" }).first().click();
  await expect(page.getByRole("button", { name: "React with 👀" })).toBeVisible();
  await page.getByRole("button", { name: "React with 👀" }).click();

  await expect.poll(() => invocationCount(page, "toggle_reaction")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("toggle_reaction")[0]?.args)
    )
    .toEqual({
      roomId: "!harness-room:example.invalid",
      eventId: "$seed-event:example.invalid",
      reactionKey: "👀"
    });
});

test("redact message invokes redact_message and shows the redacted placeholder", async ({
  page
}) => {
  await gotoReadyShell(page);
  const row = page.locator('[data-event-id="$seed-event:example.invalid"]');
  await row.hover();
  await expect(page.getByRole("button", { name: t("timeline.redactMessage") }).first()).toBeVisible();
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.getByRole("button", { name: t("timeline.redactMessage") }).first().click();

  await expect.poll(() => invocationCount(page, "redact_message")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("redact_message")[0]?.args)
    )
    .toEqual({
      roomId: "!harness-room:example.invalid",
      eventId: "$seed-event:example.invalid"
    });

  await page.evaluate(({ key, roomId }) => {
    window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        ItemsUpdated: {
          key: { account_key: "@harness-user:example.invalid", kind: { Room: { room_id: roomId } } },
          generation: 1,
          batch_id: 2,
          diffs: [
            {
              Set: {
                index: 0,
                item: {
                  id: { Event: { event_id: key } },
                  sender: "@harness-user:example.invalid",
                  body: "Visible message",
                  timestamp_ms: 1_800_000_000_000,
                  in_reply_to_event_id: null,
                  thread_root: null,
                  thread_summary: null,
                  can_react: false,
                  is_redacted: true,
                  can_redact: false,
                  is_edited: false,
                  can_edit: false,
                  reactions: []
                }
              }
            }
          ]
        }
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);
  }, { key: "$seed-event:example.invalid", roomId: "!harness-room:example.invalid" });

  await expect(row.getByText(t("timeline.redactedMessage"))).toBeVisible();
  await expect(row.getByRole("button", { name: t("timeline.replyToMessage") })).toHaveCount(0);
  await expect(row.getByRole("button", { name: t("timeline.addReaction") })).toHaveCount(0);
  await expect(row.getByRole("button", { name: t("timeline.redactMessage") })).toHaveCount(0);
});

test("editing a message invokes edit_message and renders the edited marker", async ({
  page
}) => {
  await gotoReadyShell(page);
  const row = page.locator('[data-event-id="$seed-event:example.invalid"]');
  await row.hover();
  await expect(page.getByRole("button", { name: t("timeline.editMessage") }).first()).toBeVisible();
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.getByRole("button", { name: t("timeline.editMessage") }).first().click();
  const editBody = page.getByRole("textbox", { name: t("timeline.editBody") });
  await expect(editBody).toBeVisible();
  await page.evaluate(() => window.__harness.clearInvocations());

  await editBody.fill("   ");
  await page.getByRole("button", { name: t("timeline.saveEdit") }).click();

  await expect.poll(() => invocationCount(page, "edit_message")).toBe(0);
  await expect(editBody).toBeVisible();

  await editBody.fill("Edited seed message");
  await page.getByRole("button", { name: t("timeline.saveEdit") }).click();

  await expect.poll(() => invocationCount(page, "edit_message")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("edit_message")[0]?.args)
    )
    .toEqual({
      roomId: "!harness-room:example.invalid",
      eventId: "$seed-event:example.invalid",
      body: "Edited seed message"
    });

  await page.evaluate(({ key, roomId }) => {
    window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        ItemsUpdated: {
          key: { account_key: "@harness-user:example.invalid", kind: { Room: { room_id: roomId } } },
          generation: 1,
          batch_id: 3,
          diffs: [
            {
              Set: {
                index: 0,
                item: {
                  id: { Event: { event_id: key } },
                  sender: "@harness-user:example.invalid",
                  body: "Edited seed message",
                  timestamp_ms: 1_800_000_000_000,
                  in_reply_to_event_id: null,
                  thread_root: null,
                  thread_summary: null,
                  can_react: true,
                  is_redacted: false,
                  can_redact: true,
                  is_edited: true,
                  can_edit: true,
                  reactions: [
                    {
                      key: "👍",
                      count: 1,
                      reacted_by_me: false,
                      my_reaction_event_id: null,
                      sender_preview: ["@other-user:example.invalid"]
                    }
                  ]
                }
              }
            }
          ]
        }
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);
  }, { key: "$seed-event:example.invalid", roomId: "!harness-room:example.invalid" });

  await expect(row.getByText("Edited seed message")).toBeVisible();
  await expect(row.locator(".message-edited")).toHaveText(t("timeline.editedMessage"));
});

test("selecting a search result opens focused context from Rust-owned state", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  const searchInput = page.getByRole("textbox", { name: "Search" });
  await searchInput.fill("Alpha");
  await searchInput.press("Enter");

  const resultButton = page
    .getByRole("button", { name: /Alpha keyword update from demo coordinator\./ })
    .first();
  await expect(resultButton).toBeVisible();
  await resultButton.click();

  await expect.poll(() => invocationCount(page, "select_search_result")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("select_search_result")[0]?.args)
    )
    .toEqual({
      roomId: "!harness-room:example.invalid",
      eventId: "$seed-event:example.invalid"
    });

  const focusedEventId = "$focused-event:example.invalid";
  const focusedKey = focusedTimelineKey(
    "@harness-user:example.invalid",
    "!harness-room:example.invalid",
    "$seed-event:example.invalid"
  );

  await page.evaluate(({ key, focusedEventId }) => {
    window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key,
          generation: 1,
          items: [
            {
              id: { Event: { event_id: focusedEventId } },
              sender: "@harness-user:example.invalid",
              body: "Focused context message",
              timestamp_ms: 1_800_000_000_100,
              in_reply_to_event_id: null,
              thread_root: null,
              thread_summary: null,
              reactions: [],
              can_react: true,
              is_redacted: false,
              can_redact: false,
              is_edited: false,
              can_edit: false
            }
          ]
        }
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);
  }, {
    key: focusedKey,
    focusedEventId
  });

  await expect(page.getByText(t("panel.focusedContext"), { exact: true })).toBeVisible();
  await expect(
    page.locator('aside[aria-label="Context panel"] [data-event-id="$focused-event:example.invalid"]')
  ).toBeVisible();

  await page.evaluate(() => window.__harness.clearInvocations());
  await page.getByRole("button", { name: t("room.roomInfo") }).click();
  await expect.poll(() => invocationCount(page, "close_focused_context")).toBeGreaterThanOrEqual(1);
  await expect(page.getByText(t("panel.roomInfo"), { exact: true })).toBeVisible();

  await page.evaluate(() => window.__harness.clearInvocations());
  await resultButton.click();
  await expect(page.getByText(t("panel.focusedContext"), { exact: true })).toBeVisible();

  await page.getByRole("button", { name: t("action.close", { title: t("panel.search") }) }).click();
  await expect.poll(() => invocationCount(page, "close_focused_context")).toBeGreaterThanOrEqual(1);
});

test("thread summary chip opens a thread timeline from keyed CoreEvents", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  await expect(page.getByRole("button", { name: /2 replies/ })).toBeVisible();
  await page.getByRole("button", { name: /2 replies/ }).click();

  await expect.poll(() => invocationCount(page, "open_thread")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("open_thread")[0]?.args)
    )
    .toEqual({
      roomId: "!harness-room:example.invalid",
      rootEventId: "$seed-event:example.invalid"
    });

  const threadEventId = "$thread-reply:example.invalid";
  const threadKey = threadTimelineKey(
    "@harness-user:example.invalid",
    "!harness-room:example.invalid",
    "$seed-event:example.invalid"
  );

  await page.evaluate(({ key, threadEventId }) => {
    window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key,
          generation: 1,
          items: [
            {
              id: { Event: { event_id: threadEventId } },
              sender: "@thread-user:example.invalid",
              body: "Thread panel reply from keyed event stream",
              timestamp_ms: 1_800_000_000_200,
              in_reply_to_event_id: "$seed-event:example.invalid",
              thread_root: "$seed-event:example.invalid",
              thread_summary: null,
              reactions: [],
              can_react: true,
              is_redacted: false,
              can_redact: false,
              is_edited: false,
              can_edit: false
            }
          ]
        }
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);
  }, {
    key: threadKey,
    threadEventId
  });

  await expect(page.getByText(t("panel.thread"), { exact: true })).toBeVisible();
  await expect(
    page.locator('aside[aria-label="Context panel"] [data-event-id="$thread-reply:example.invalid"]')
  ).toBeVisible();
  await expect(
    page
      .locator('aside[aria-label="Context panel"]')
      .getByText("Thread panel reply from keyed event stream", { exact: true })
  ).toBeVisible();
});

test("thread panel scrollback invokes thread pagination command only", async ({
  page
}) => {
  await gotoReadyShell(page);

  await expect(page.getByRole("button", { name: /2 replies/ })).toBeVisible();
  await page.getByRole("button", { name: /2 replies/ }).click();
  await expect(page.getByText(t("panel.thread"), { exact: true })).toBeVisible();

  const threadKey = threadTimelineKey(
    "@harness-user:example.invalid",
    "!harness-room:example.invalid",
    "$seed-event:example.invalid"
  );
  await page.evaluate(({ key, items }) => {
    window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key,
          generation: 1,
          items
        }
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);
  }, {
    key: threadKey,
    items: Array.from({ length: 48 }, (_, index) => makeThreadItem(index))
  });

  const threadTimeline = page.locator('aside[aria-label="Context panel"] [data-testid="timeline-view"]');
  await expect(threadTimeline.locator("[data-item-id]")).toHaveCount(48);
  await expect(
    page
      .locator('aside[aria-label="Context panel"]')
      .getByText("Thread overflow message 47", { exact: true })
  ).toBeVisible();

  await page.evaluate(() => window.__harness.clearInvocations());
  await threadTimeline.evaluate((node) => {
    node.scrollTop = 40;
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
  });

  await expect
    .poll(() => invocationCount(page, "paginate_thread_timeline_backwards"))
    .toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("paginate_thread_timeline_backwards")[0]?.args)
    )
    .toEqual({
      roomId: "!harness-room:example.invalid",
      rootEventId: "$seed-event:example.invalid"
    });
  expect(await invocationCount(page, "paginate_timeline_backwards")).toBe(0);

  await page.evaluate(({ key }) => {
    window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        PaginationStateChanged: {
          request_id: null,
          key,
          direction: "Backward",
          state: "Paginating"
        }
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);
  }, { key: threadKey });
  await expect(
    page.locator('aside[aria-label="Context panel"] [data-testid="timeline-spinner"]')
  ).toBeVisible();

  await page.evaluate(({ key }) => {
    window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        PaginationStateChanged: {
          request_id: null,
          key,
          direction: "Backward",
          state: "EndReached"
        }
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);
  }, { key: threadKey });
  await expect(
    page.locator('aside[aria-label="Context panel"] [data-testid="timeline-start"]')
  ).toBeVisible();
});

test("thread composer drafts and sends through thread reply commands only", async ({
  page
}) => {
  await gotoReadyShell(page);

  await expect(page.getByRole("button", { name: /2 replies/ })).toBeVisible();
  await page.getByRole("button", { name: /2 replies/ }).click();
  await expect(page.getByText(t("panel.thread"), { exact: true })).toBeVisible();

  await page.evaluate(() => window.__harness.clearInvocations());

  const threadComposer = page.getByRole("textbox", { name: t("timeline.threadComposer") });
  await expect(threadComposer).toBeVisible();
  await threadComposer.fill("Thread composer reply body");

  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("set_thread_composer_draft")[0]?.args)
    )
    .toEqual({
      roomId: "!harness-room:example.invalid",
      rootEventId: "$seed-event:example.invalid",
      draft: "Thread composer reply body"
    });

  await threadComposer.press("Enter");

  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("send_thread_reply")[0]?.args)
    )
    .toEqual({
      roomId: "!harness-room:example.invalid",
      rootEventId: "$seed-event:example.invalid",
      body: "Thread composer reply body"
    });
  expect(await invocationCount(page, "send_text")).toBe(0);
  expect(await invocationCount(page, "send_reply")).toBe(0);
});

test("submitting the composer in reply mode invokes send_reply, not send_text", async ({
  page
}) => {
  await gotoReadyShell(page);

  // Establish reply mode via the reply action (its response snapshot puts the
  // composer into reply mode), then submit the composer.
  await page.getByRole("button", { name: "Reply to message" }).first().click();
  await expect(page.getByRole("button", { name: "Cancel reply" })).toBeVisible();

  await page.evaluate(() => window.__harness.clearInvocations());

  const composer = page.getByRole("textbox", { name: "Message composer" });
  await composer.fill("A reply body");
  await page.getByRole("button", { name: "Send" }).click();

  await expect.poll(() => invocationCount(page, "send_reply")).toBeGreaterThanOrEqual(1);
  expect(await invocationCount(page, "send_text")).toBe(0);
});

test("reply send does not repair product state by cancelling reply mode", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.getByRole("button", { name: "Reply to message" }).first().click();
  await expect(page.getByRole("button", { name: "Cancel reply" })).toBeVisible();

  // Simulate the realistic backend timing where send_reply returns before the
  // Rust SendTextFinished action has cleared reply mode. React must NOT repair
  // product state by issuing cancel_composer_reply; the Rust state machine owns
  // the completion transition (driven asynchronously via the snapshot stream).
  await page.evaluate(() => {
    window.__harness.setCommandResponse(
      "send_reply",
      window.__harness.replyModeSnapshot()
    );
    window.__harness.clearInvocations();
  });

  await page.getByRole("textbox", { name: "Message composer" }).fill("A reply body");
  await page.getByRole("button", { name: "Send" }).click();

  await expect.poll(() => invocationCount(page, "send_reply")).toBeGreaterThanOrEqual(1);
  expect(await invocationCount(page, "cancel_composer_reply")).toBe(0);
});
