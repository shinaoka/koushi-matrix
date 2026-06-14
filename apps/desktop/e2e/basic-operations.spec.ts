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
 *   8. Drive E2EE trust controls from User settings → invokes Rust-owned trust
 *      commands and renders the returned `e2ee_trust` snapshot.
 *   9. Drive invite acceptance and New DM from Rust-owned snapshots → invokes
 *      room commands and renders joined/DM rooms from the returned snapshot.
 */

import { expect, test, type Page } from "@playwright/test";

import { focusedTimelineKey, roomTimelineKey, threadTimelineKey } from "../src/domain/coreEvents";
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

test("invites view accepts a seeded invite and New DM renders the returned direct room", async ({
  page
}) => {
  await gotoReadyShell(page);

  await page.evaluate(() => {
    const base = window.__harness.currentSnapshot();
    const invite = {
      room_id: "!invite-seed:example.invalid",
      display_name: "Seeded Invite",
      topic: "Synthetic invite topic",
      inviter_display_name: "Synthetic Inviter",
      is_dm: false
    };
    window.__harness.setSnapshot({
      ...base,
      state: {
        ...base.state,
        invites: [invite]
      }
    });
    window.__harness.setCommandResponse("accept_invite", () => {
      const snapshot = window.__harness.currentSnapshot();
      const joinedRoom = {
        room_id: "!joined-from-invite:example.invalid",
        display_name: "Seeded Invite",
        is_dm: false,
        unread_count: 0,
        parent_space_ids: []
      };
      const next = {
        ...snapshot,
        state: {
          ...snapshot.state,
          rooms: [...snapshot.state.rooms, joinedRoom],
          invites: [],
          navigation: {
            ...snapshot.state.navigation,
            active_room_id: joinedRoom.room_id
          },
          timeline: {
            ...snapshot.state.timeline,
            room_id: joinedRoom.room_id,
            is_subscribed: true
          }
        },
        sidebar: {
          ...snapshot.sidebar,
          space_rooms: [
            ...snapshot.sidebar.space_rooms,
            {
              room_id: joinedRoom.room_id,
              display_name: joinedRoom.display_name,
              unread_count: 0
            }
          ]
        }
      };
      window.__harness.setSnapshot(next);
      return next;
    });
    window.__harness.setCommandResponse("start_direct_message", ({ userId }) => {
      const snapshot = window.__harness.currentSnapshot();
      const dmRoom = {
        room_id: "!dm-started:example.invalid",
        display_name: String(userId),
        is_dm: true,
        unread_count: 0,
        parent_space_ids: []
      };
      const next = {
        ...snapshot,
        state: {
          ...snapshot.state,
          rooms: [...snapshot.state.rooms, dmRoom],
          navigation: {
            ...snapshot.state.navigation,
            active_room_id: dmRoom.room_id
          },
          timeline: {
            ...snapshot.state.timeline,
            room_id: dmRoom.room_id,
            is_subscribed: true
          }
        },
        sidebar: {
          ...snapshot.sidebar,
          global_dms: [
            ...snapshot.sidebar.global_dms,
            {
              room_id: dmRoom.room_id,
              display_name: dmRoom.display_name,
              unread_count: 0
            }
          ]
        }
      };
      window.__harness.setSnapshot(next);
      return next;
    });
    window.__harness.setCommandResponse("select_room", ({ roomId }) => {
      const snapshot = window.__harness.currentSnapshot();
      const next = {
        ...snapshot,
        state: {
          ...snapshot.state,
          navigation: {
            ...snapshot.state.navigation,
            active_room_id: String(roomId)
          },
          timeline: {
            ...snapshot.state.timeline,
            room_id: String(roomId),
            is_subscribed: true
          },
          thread: { kind: "closed" }
        },
        thread: null
      };
      window.__harness.setSnapshot(next);
      return next;
    });
    window.__harness.setCommandResponse("invite_user", () => window.__harness.currentSnapshot());
    window.__harness.pushStateChanged();
    window.__harness.clearInvocations();
  });

  await page.getByRole("button", { name: "Invites" }).click();
  await expect(page.getByRole("heading", { name: "Invites" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Seeded Invite" })).toBeVisible();
  await expect(page.getByText("Synthetic Inviter", { exact: true })).toBeVisible();

  await page.getByRole("button", { name: "Accept invite" }).click();

  await expect.poll(() => invocationCount(page, "accept_invite")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("accept_invite")[0]?.args)
    )
    .toEqual({ roomId: "!invite-seed:example.invalid" });
  await expect(page.getByRole("button", { name: "Seeded Invite" })).toBeVisible();
  await expect(
    page.getByRole("main", { name: "Invites" }).getByText("No pending invites").first()
  ).toBeVisible();

  await page.getByRole("button", { name: "Seeded Invite" }).click();
  await page.getByRole("button", { name: "Room info" }).click();
  await page.getByRole("button", { name: "Invite people" }).click();
  const inviteUserInput = page.getByRole("textbox", { name: "Matrix user ID" });
  await inviteUserInput.fill("@invitee:example.invalid");
  await page.getByRole("button", { name: "Send invite" }).click();

  await expect.poll(() => invocationCount(page, "invite_user")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("invite_user")[0]?.args)
    )
    .toEqual({
      roomId: "!joined-from-invite:example.invalid",
      userId: "@invitee:example.invalid"
    });

  await page.getByRole("button", { name: "Invites" }).click();
  await page.getByRole("main", { name: "Invites" }).getByRole("button", { name: "New DM" }).click();
  const userIdInput = page.getByRole("textbox", { name: "Matrix user ID" });
  await expect(userIdInput).toBeVisible();
  await userIdInput.fill("@target:example.invalid");
  await page.getByRole("button", { name: "Start DM" }).click();

  await expect.poll(() => invocationCount(page, "start_direct_message")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("start_direct_message")[0]?.args)
    )
    .toEqual({ userId: "@target:example.invalid" });
  await expect(page.getByRole("button", { name: "@target:example.invalid" })).toBeVisible();
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

test("attach control invokes upload_media and renders Rust-owned media progress", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    window.__harness.setCommandResponse("upload_media", () => window.__harness.currentSnapshot());
    window.__harness.setCommandResponse("download_media", () => window.__harness.currentSnapshot());
    window.__harness.clearInvocations();
  });

  const fixtureBytes = Buffer.from("browser-headless media fixture");
  await page.getByRole("button", { name: "Attach file", exact: true }).click();
  await page
    .locator('input[type="file"][aria-label="Attach file input"]')
    .setInputFiles({
      name: "media-fixture.txt",
      mimeType: "text/plain",
      buffer: fixtureBytes
    });

  await expect.poll(() => invocationCount(page, "upload_media")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => {
        const args = window.__harness.invocationsOf("upload_media")[0]?.args;
        return args
          ? {
              roomId: args.roomId,
              filename: args.filename,
              mimeType: args.mimeType,
              byteCount: Array.isArray(args.bytes) ? args.bytes.length : -1
            }
          : null;
      })
    )
    .toEqual({
      roomId: "!harness-room:example.invalid",
      filename: "media-fixture.txt",
      mimeType: "text/plain",
      byteCount: fixtureBytes.length
    });

  const key = roomTimelineKey("@harness-user:example.invalid", "!harness-room:example.invalid");
  await page.evaluate(({ key }) => {
    window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        ItemsUpdated: {
          key,
          generation: 1,
          batch_id: 4,
          diffs: [
            {
              PushBack: {
                item: {
                  id: { Transaction: { transaction_id: "desktop-media-1" } },
                  sender: "@harness-user:example.invalid",
                  body: null,
                  timestamp_ms: 1_800_000_000_300,
                  in_reply_to_event_id: null,
                  thread_root: null,
                  thread_summary: null,
                  media: {
                    kind: "File",
                    filename: "media-fixture.txt",
                    source: {
                      mxc_uri: "mxc://example.invalid/media-fixture",
                      encrypted: false,
                      encryption_version: null
                    },
                    mimetype: "text/plain",
                    size: 30,
                    width: null,
                    height: null,
                    thumbnail: null
                  },
                  reactions: [],
                  can_react: false,
                  is_redacted: false,
                  can_redact: false,
                  is_edited: false,
                  can_edit: false
                }
              }
            }
          ]
        }
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);
    window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        MediaUploadProgress: {
          request_id: null,
          key,
          transaction_id: "desktop-media-1",
          index: 0,
          progress: { current: 15, total: 30 },
          source: null
        }
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);
    window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        ItemsUpdated: {
          key,
          generation: 1,
          batch_id: 5,
          diffs: [
            {
              PushBack: {
                item: {
                  id: { Event: { event_id: "$media-event:example.invalid" } },
                  sender: "@harness-user:example.invalid",
                  body: null,
                  timestamp_ms: 1_800_000_000_400,
                  in_reply_to_event_id: null,
                  thread_root: null,
                  thread_summary: null,
                  media: {
                    kind: "File",
                    filename: "downloadable-fixture.txt",
                    source: {
                      mxc_uri: "mxc://example.invalid/downloadable-fixture",
                      encrypted: false,
                      encryption_version: null
                    },
                    mimetype: "text/plain",
                    size: 30,
                    width: null,
                    height: null,
                    thumbnail: null
                  },
                  reactions: [],
                  can_react: true,
                  is_redacted: false,
                  can_redact: false,
                  is_edited: false,
                  can_edit: false
                }
              }
            }
          ]
        }
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);
  }, { key });

  const mediaRow = page.locator('[data-item-id="txn:desktop-media-1"]');
  await expect(mediaRow.getByText("media-fixture.txt", { exact: true })).toBeVisible();
  await expect(mediaRow.getByText("50%", { exact: true })).toBeVisible();

  const downloadableRow = page.locator('[data-event-id="$media-event:example.invalid"]');
  await expect(downloadableRow.getByText("downloadable-fixture.txt", { exact: true })).toBeVisible();
  await downloadableRow.getByRole("button", { name: "Download downloadable-fixture.txt" }).click();
  await expect.poll(() => invocationCount(page, "download_media")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("download_media")[0]?.args)
    )
    .toEqual({
      roomId: "!harness-room:example.invalid",
      eventId: "$media-event:example.invalid"
    });
});

test("live signals render from Rust state and dispatch read/typing commands", async ({
  page
}) => {
  await gotoReadyShell(page);

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        live_signals: {
          rooms: {
            "!harness-room:example.invalid": {
              receipts_by_event: {
                "$seed-event:example.invalid": [
                  {
                    user_id: "@reader:example.invalid",
                    timestamp_ms: 1_800_000_000_500
                  }
                ]
              },
              fully_read_event_id: "$seed-event:example.invalid",
              typing_user_ids: ["@typing-user:example.invalid"]
            }
          },
          presence: {
            "@harness-user:example.invalid": "online"
          }
        }
      }
    });
    window.__harness.pushStateChanged();
  });

  const row = page.locator('[data-event-id="$seed-event:example.invalid"]');
  await expect(row.locator(".presence-dot[data-presence='online']")).toBeVisible();
  await expect(row.getByText("Read by 1", { exact: true })).toBeVisible();
  await expect(page.getByText("Read up to here", { exact: true })).toBeVisible();
  await expect(page.getByText("@typing-user:example.invalid is typing", { exact: true })).toBeVisible();
  await expect.poll(() => invocationCount(page, "send_read_receipt")).toBeGreaterThanOrEqual(1);
  await expect.poll(() => invocationCount(page, "set_fully_read")).toBeGreaterThanOrEqual(1);

  await page.evaluate(() => window.__harness.clearInvocations());
  await page.getByRole("textbox", { name: "Message composer" }).fill("Typing signal");

  await expect.poll(() => invocationCount(page, "set_typing")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("set_typing")[0]?.args))
    .toEqual({
      roomId: "!harness-room:example.invalid",
      isTyping: true
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
      page.evaluate(() => window.__harness.invocationsOf("resolve_composer_key_action")[0]?.args)
    )
    .toEqual({
      surface: "thread",
      keyEvent: {
        key: "enter",
        modifiers: { ctrl: false, meta: false, shift: false, alt: false },
        is_composing: false
      },
      autocompleteOpen: false,
      sendEnabled: true
    });
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

test("Rust-owned locale profile applies root lang and dir", async ({ page }) => {
  await gotoReadyShell(page);

  await page.evaluate(() => {
    const snapshot = window.__harness.replyModeSnapshot();
    snapshot.state.locale_profile = {
      lang: "ar-XB",
      dir: "rtl",
      catalog_locale: "pseudo",
      pseudo_locale: "bidi",
      platform: "linux",
      modifier_labels: { primary: "Ctrl" }
    };
    window.__harness.setSnapshot(snapshot);
    window.__harness.pushStateChanged();
  });

  await expect.poll(() => page.evaluate(() => document.documentElement.lang)).toBe("ar-XB");
  await expect.poll(() => page.evaluate(() => document.documentElement.dir)).toBe("rtl");
  await expect
    .poll(() => page.evaluate(() => document.documentElement.dataset.catalogLocale))
    .toBe("pseudo");
  await expect
    .poll(() => page.evaluate(() => document.documentElement.dataset.pseudoLocale))
    .toBe("bidi");
});

test("pseudo RTL profile with CJK and combining samples does not overflow shell", async ({
  page
}) => {
  await gotoReadyShell(page);

  const longRoomName = "Cafe\u0301 日本語 العربية Very Long Synthetic Room Label For Pseudo Locale";
  const sampleBody = "Cafe\u0301 日本語 العربية long pseudo locale sample";
  const roomKey = roomTimelineKey(
    "@harness-user:example.invalid",
    "!harness-room:example.invalid"
  );
  await page.evaluate((roomName) => {
    const snapshot = window.__harness.replyModeSnapshot();
    snapshot.state.locale_profile = {
      lang: "ar-XB",
      dir: "rtl",
      catalog_locale: "pseudo",
      pseudo_locale: "bidi",
      platform: "linux",
      modifier_labels: { primary: "Ctrl" }
    };
    snapshot.state.rooms[0].display_name = roomName;
    snapshot.sidebar.space_rooms[0].display_name = roomName;
    snapshot.state.spaces[0].display_name = "日本語 Space العربية";
    snapshot.sidebar.space_rail[0].display_name = "日本語 Space العربية";
    window.__harness.setSnapshot(snapshot);
    window.__harness.pushStateChanged();
  }, longRoomName);

  await expect(page.locator("main.main-pane").getByText(longRoomName)).toBeVisible();
  await expect(page.getByText("Seed message for reply target")).toBeVisible();

  await page.evaluate(async ({ key, body }) => {
    const item = {
      id: { Event: { event_id: "$seed-event:example.invalid" } },
      sender: "@rtl-user:example.invalid",
      body,
      timestamp_ms: 1_800_000_000_000,
      in_reply_to_event_id: null,
      thread_root: null,
      thread_summary: null,
      can_react: true,
      is_redacted: false,
      can_redact: false,
      is_edited: false,
      can_edit: true,
      reactions: [
        {
          key: "日本語",
          count: 1,
          reacted_by_me: false,
          my_reaction_event_id: null,
          sender_preview: ["@rtl-user:example.invalid"]
        }
      ]
    };
    const payload = {
      kind: "Timeline",
      event: {
        ItemsUpdated: {
          key,
          generation: 1,
          batch_id: 2,
          diffs: [{ Set: { index: 0, item } }]
        }
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any;

    for (let attempt = 0; attempt < 20; attempt += 1) {
      await window.__harness.pushCoreEvent(payload);
      await new Promise((resolve) => setTimeout(resolve, 25));
      if (document.body.textContent?.includes(body)) {
        break;
      }
    }
  }, { key: roomKey, body: sampleBody });

  await expect.poll(() => page.evaluate(() => document.documentElement.dir)).toBe("rtl");
  await expect(page.locator(".room-name").first()).toHaveAttribute("dir", "auto");
  await expect(page.locator(".message-body").first()).toHaveAttribute("dir", "auto");
  await expect(page.getByText(sampleBody)).toBeVisible();
  await expect(page.locator(".reaction-pill-key", { hasText: "日本語" })).toBeVisible();
  await expect
    .poll(() => page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 2))
    .toBe(true);
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

test("keyboard settings update composer send shortcut through Rust-owned commands", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.getByRole("button", { name: "Keyboard settings" }).click();
  await expect(page.getByText("Composer send shortcut")).toBeVisible();
  await page.getByRole("button", { name: /^(Ctrl|Cmd)\+Enter sends$/ }).click();

  await expect.poll(() => invocationCount(page, "update_settings")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("update_settings")[0]?.args)
    )
    .toEqual({
      patch: {
        keyboard: { composer_send_shortcut: "modEnter" }
      }
    });

  await page.evaluate(() => window.__harness.clearInvocations());
  const composer = page.getByRole("textbox", { name: "Message composer" });
  await composer.fill("Shortcut-controlled body");
  await composer.press("Enter");

  await expect.poll(() => invocationCount(page, "resolve_composer_key_action")).toBeGreaterThanOrEqual(1);
  expect(await invocationCount(page, "send_text")).toBe(0);

  await composer.press("Control+Enter");

  await expect.poll(() => invocationCount(page, "send_text")).toBeGreaterThanOrEqual(1);
});

test("E2EE trust controls dispatch Rust-owned commands and render snapshot updates", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    window.__harness.setSnapshot(window.__harness.e2eeTrustSnapshot());
    window.__harness.pushStateChanged();
  });

  await page.getByRole("button", { name: "User settings" }).click();
  await expect(page.getByRole("heading", { name: "Encryption" })).toBeVisible();
  await expect(page.getByText("Device verification")).toBeVisible();
  await expect(page.getByText("Device 1")).toBeVisible();
  await expect(page.getByText("redacted-trust-target")).toHaveCount(0);

  await page.evaluate(() => window.__harness.clearInvocations());
  await page.getByRole("button", { name: "Accept" }).click();

  await expect.poll(() => invocationCount(page, "accept_verification")).toBe(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("accept_verification")[0]?.args)
    )
    .toEqual({ flowId: 9001 });
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.currentSnapshot().state.e2ee_trust.verification.kind)
    )
    .toBe("accepted");
  await expect(page.getByText("Accepted")).toBeVisible();

  await page.getByRole("button", { name: "Enable", exact: true }).click();
  await expect.poll(() => invocationCount(page, "enable_key_backup")).toBe(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.currentSnapshot().state.e2ee_trust.key_backup.kind)
    )
    .toBe("enabled");
  await expect(page.getByText("Enabled")).toBeVisible();

  await page.getByRole("button", { name: "Set up", exact: true }).click();
  await expect.poll(() => invocationCount(page, "bootstrap_cross_signing")).toBe(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.currentSnapshot().state.e2ee_trust.cross_signing.kind)
    )
    .toBe("trusted");

  await page.getByRole("button", { name: "Reset", exact: true }).click();
  await expect.poll(() => invocationCount(page, "reset_identity")).toBe(1);
  await expect(page.getByLabel("Password")).toBeVisible();
  await page.getByLabel("Password").fill("identity reset smoke password");
  await page.getByRole("button", { name: "Continue" }).click();

  await expect.poll(() => invocationCount(page, "submit_identity_reset_password")).toBe(1);
  await expect
    .poll(async () =>
      page.evaluate(
        () => window.__harness.invocationsOf("submit_identity_reset_password")[0]?.args
      )
    )
    .toEqual({ flowId: 9100, password: "[REDACTED]" });
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.currentSnapshot().state.e2ee_trust.identity_reset.kind)
    )
    .toBe("idle");
});

test("edit composer respects the Rust-owned composer shortcut resolver", async ({
  page
}) => {
  await gotoReadyShell(page);

  await page.getByRole("button", { name: "Keyboard settings" }).click();
  await page.getByRole("button", { name: /^(Ctrl|Cmd)\+Enter sends$/ }).click();

  const row = page.locator('[data-event-id="$seed-event:example.invalid"]');
  await row.hover();
  await page.getByRole("button", { name: t("timeline.editMessage") }).first().click();
  const editBody = page.getByRole("textbox", { name: t("timeline.editBody") });
  await expect(editBody).toBeVisible();

  await page.evaluate(() => window.__harness.clearInvocations());
  await editBody.fill("Resolver edited body");
  await editBody.press("Enter");

  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("resolve_composer_key_action")[0]?.args)
    )
    .toEqual({
      surface: "edit",
      keyEvent: {
        key: "enter",
        modifiers: { ctrl: false, meta: false, shift: false, alt: false },
        is_composing: false
      },
      autocompleteOpen: false,
      sendEnabled: true
    });
  expect(await invocationCount(page, "edit_message")).toBe(0);

  await editBody.press("Control+Enter");

  await expect.poll(() => invocationCount(page, "edit_message")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("edit_message")[0]?.args)
    )
    .toEqual({
      roomId: "!harness-room:example.invalid",
      eventId: "$seed-event:example.invalid",
      body: "Resolver edited body"
    });
});
