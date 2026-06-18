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
 *   4. Click reaction affordances → invokes typed `send_reaction` /
 *      `redact_reaction`.
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
 *  10. Render Rust-owned outbound send states and dispatch retry/cancel send
 *      commands without React repairing send-queue state.
 *  11. Drive message action menus from Rust-owned `TimelineItem.actions`, copy
 *      DTO values only, and dispatch source/forward typed commands.
 *  12. Drive Explore public-directory search/join commands and wait for
 *      Rust-shaped snapshots before rendering results or joined rooms.
 *  13. Drive room-management topic/avatar/moderation/role commands and wait for
 *      Rust-shaped snapshots before rendering settings or membership changes.
 *  14. Drive Activity Recent/Unread streams from Rust-owned snapshots, dispatch
 *      focused-context and mark-read commands, and wait for Rust to remove rows.
 *  15. Render Settings/Security local-encryption health from Rust-owned
 *      snapshots and dispatch credential health probes only through Tauri IPC.
 *  16. Render Rust-owned formatted timeline DTOs and drive the display
 *      code-block wrap setting through `update_settings`.
 *  17. Hide redacted timeline rows only after the Rust-owned display policy
 *      event marks redacted item DTOs hidden.
 *  18. Drive image-compression settings and assert upload_media receives the
 *      Rust-shaped selected image variant contract.
 *  19. Drive main composer drafts through set_composer_draft and hydrate them
 *      from Rust-shaped per-room snapshots.
 *  20. Drive scheduled-send composer/list controls through Rust-owned snapshot
 *      state and typed schedule/cancel/reschedule commands.
 *  21. Drive Security key-management controls through Rust-owned room-key and
 *      secure-backup commands, with secret/path redaction in recorded IPC.
 */

import { expect, test, type Locator, type Page } from "@playwright/test";

import { focusedTimelineKey, roomTimelineKey, threadTimelineKey } from "../src/domain/coreEvents";
import { pseudoLocalize, t } from "../src/i18n/messages";

const HARNESS_ACCOUNT_KEY = "@harness-user:example.invalid";
const HARNESS_ROOM_ID = "!harness-room:example.invalid";
const HARNESS_ROOM_KEY = roomTimelineKey(HARNESS_ACCOUNT_KEY, HARNESS_ROOM_ID);

async function canvasPngBuffer(page: Page, width: number, height: number): Promise<Buffer> {
  const base64 = await page.evaluate(
    ({ width, height }) => {
      const canvas = document.createElement("canvas");
      canvas.width = width;
      canvas.height = height;
      const context = canvas.getContext("2d");
      if (!context) {
        throw new Error("2d canvas unavailable");
      }
      context.fillStyle = "#2d6fef";
      context.fillRect(0, 0, width, height);
      context.fillStyle = "#ffffff";
      context.fillRect(0, 0, Math.max(1, Math.floor(width / 2)), height);
      return canvas.toDataURL("image/png").split(",")[1];
    },
    { width, height }
  );
  return Buffer.from(base64, "base64");
}

async function attachFile(
  page: Page,
  file: { name: string; mimeType: string; buffer: Buffer }
): Promise<void> {
  await page.getByRole("button", { name: "Attach file", exact: true }).click();
  await page
    .locator('input[type="file"][aria-label="Attach file input"]')
    .setInputFiles(file);
}

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
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false
  };
}

function makeSendQueueItem(
  transactionId: string,
  body: string,
  sendState: { kind: "sending" } | { kind: "notSent"; reason: "recoverable" | "unrecoverable" }
) {
  return {
    id: { Transaction: { transaction_id: transactionId } },
    sender: "Harness Sender",
    body,
    timestamp_ms: 1_800_000_002_000,
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
    send_state: sendState
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

async function gotoSignedOutAuth(page: Page): Promise<void> {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        session: { kind: "signedOut" },
        auth: { kind: "unknown" },
        sync: "stopped",
        navigation: { active_space_id: null, active_room_id: null },
        rooms: [],
        spaces: [],
        invites: [],
        room_notification_settings: {},
        room_interactions: {},
        timeline: {
          room_id: null,
          is_subscribed: false,
          is_paginating_backwards: false,
          composer: { pending_transaction_id: null, draft: "", mode: "Plain" },
          scheduled_send_capability: "unknown",
          scheduled_sends: [],
          staged_uploads: [],
          media_gallery: []
        }
      },
      sidebar: {
        active_space_id: null,
        account_home: { display_name: "Home", unread_count: 0, highlight_count: 0, is_active: true },
        space_rail: [],
        space_rooms: [],
        global_dms: [],
        space_unread_count: 0,
        dm_unread_count: 0,
        space_highlight_count: 0,
        dm_highlight_count: 0
      },
      timeline: []
    });
    window.__harness.pushStateChanged();
    window.__harness.clearInvocations();
  });
  await expect(page.getByTestId("auth-screen")).toBeVisible();
}

async function invocationCount(page: Page, command: string): Promise<number> {
  return page.evaluate((cmd) => window.__harness.invocationsOf(cmd).length, command);
}

async function dispatchComposingEnter(locator: Locator): Promise<boolean> {
  return locator.evaluate((element) => {
    const event = new KeyboardEvent("keydown", {
      bubbles: true,
      cancelable: true,
      key: "Enter"
    });
    Object.defineProperty(event, "isComposing", { value: true });
    element.dispatchEvent(event);
    return event.defaultPrevented;
  });
}

async function seedTimelineItems(page: Page, items: unknown[], generation = 2): Promise<void> {
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
            return itemDomIds.every((id) =>
              document.querySelector(`[data-item-id="${CSS.escape(id)}"]`)
            );
          },
          { key: HARNESS_ROOM_KEY, nextItems: items, nextGeneration: generation }
        ),
      { timeout: 10_000, intervals: [25, 50, 100, 250] }
    )
    .toBe(true);
}

async function pushTimelineDiffs(
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

test("auth form defaults to matrix.org and submits custom ports in the homeserver URL field", async ({
  page
}) => {
  await gotoSignedOutAuth(page);

  const homeserverInput = page.locator('input[name="homeserver"]');
  await expect(homeserverInput).toHaveValue("https://matrix.org");

  await homeserverInput.fill("https://example.org:8448");
  await page.getByRole("textbox", { name: t("auth.usernameOrMatrixId") }).fill("@alice:example.org");
  await page.getByLabel(t("auth.password")).fill("synthetic-password");
  await page.getByRole("textbox", { name: t("auth.deviceName") }).fill("Kagome Test Device");
  await page.getByRole("button", { name: t("auth.continue") }).click();

  await expect.poll(() => invocationCount(page, "submit_login")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("submit_login")[0]?.args))
    .toEqual({
      homeserver: "https://example.org:8448",
      username: "@alice:example.org",
      password: "[REDACTED]",
      deviceDisplayName: "Kagome Test Device"
    });
  await expect(page.locator('input[name="port"]')).toHaveCount(0);
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
      avatar: null,
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
        avatar: null,
        is_dm: false,
        tags: { favourite: null, low_priority: null },
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
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
              avatar: null,
              tags: { favourite: null, low_priority: null },
              unread_count: 0,
              highlight_count: 0
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
        avatar: null,
        is_dm: true,
        tags: { favourite: null, low_priority: null },
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
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
              avatar: null,
              tags: { favourite: null, low_priority: null },
              unread_count: 0,
              highlight_count: 0
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
          thread: { kind: "closed" },
          thread_attention: { kind: "closed" }
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

test("Explore searches public rooms and joins only after Rust snapshot updates", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    window.__harness.setCommandResponse("query_directory", () =>
      window.__harness.currentSnapshot()
    );
    window.__harness.setCommandResponse("join_directory_room", () =>
      window.__harness.currentSnapshot()
    );
    window.__harness.clearInvocations();
  });

  await page.getByRole("button", { name: "Explore" }).click();
  await expect(page.getByRole("main", { name: "Explore" })).toBeVisible();

  const searchInput = page.getByRole("searchbox", { name: "Search public rooms" });
  await searchInput.fill("public");
  await page.getByRole("button", { name: "Search public rooms" }).click();

  await expect.poll(() => invocationCount(page, "query_directory")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("query_directory")[0]?.args))
    .toEqual({
      term: "public",
      serverName: null,
      limit: 20,
      since: null
    });
  await expect(page.getByRole("heading", { name: "Public Search Result" })).toHaveCount(0);

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    const query = {
      term: "public",
      server_name: null,
      limit: 20,
      since: null
    };
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        directory: {
          ...snapshot.state.directory,
          query: {
            kind: "results",
            request_id: 44,
            query,
            rooms: [
              {
                room_id: "!public-result:example.invalid",
                canonical_alias: "#public-result:example.invalid",
                name: "Public Search Result",
                topic: "Rust-owned public directory result",
                avatar_url: null,
                joined_members: 12,
                world_readable: true,
                guest_can_join: false
              }
            ],
            next_batch: null
          },
          join: { kind: "idle" }
        }
      }
    });
    window.__harness.pushStateChanged();
  });

  await expect(page.getByRole("heading", { name: "Public Search Result" })).toBeVisible();
  await page.getByRole("button", { name: "Join Public Search Result" }).click();

  await expect.poll(() => invocationCount(page, "join_directory_room")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("join_directory_room")[0]?.args)
    )
    .toEqual({
      alias: "#public-result:example.invalid",
      viaServer: "example.invalid"
    });

  const roomsSection = page.locator('[data-room-section="rooms"]');
  await expect(
    roomsSection.getByRole("button", { name: "Public Search Result" })
  ).toHaveCount(0);

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    const joinedRoom = {
      room_id: "!joined-public-result:example.invalid",
      display_name: "Public Search Result",
      avatar: null,
      is_dm: false,
      tags: { favourite: null, low_priority: null },
      unread_count: 0,
      notification_count: 0,
      highlight_count: 0,
      parent_space_ids: []
    };
    const roomListItem = {
      room_id: joinedRoom.room_id,
      display_name: joinedRoom.display_name,
      avatar: joinedRoom.avatar,
      tags: joinedRoom.tags,
      unread_count: joinedRoom.unread_count,
      notification_count: joinedRoom.notification_count,
      highlight_count: joinedRoom.highlight_count
    };
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        rooms: [...snapshot.state.rooms, joinedRoom],
        directory: {
          ...snapshot.state.directory,
          join: { kind: "idle" }
        }
      },
      sidebar: {
        ...snapshot.sidebar,
        space_rooms: [...snapshot.sidebar.space_rooms, roomListItem]
      }
    });
    window.__harness.pushStateChanged();
  });

  await expect(
    roomsSection.getByRole("button", { name: "Public Search Result" })
  ).toBeVisible();
});

test("Activity renders Rust-owned streams and waits for mark-read snapshots", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const recentRows = [
      {
        room_id: "!room-beta:example.invalid",
        event_id: "$activity-beta-newest:example.invalid",
        room_label: "Project Beta",
        sender_label: "Beta Sender",
        preview: "Newest recent update",
        timestamp_ms: 1_800_000_010_000,
        unread: false,
        highlight: false
      },
      {
        room_id: "!room-alpha:example.invalid",
        event_id: "$activity-alpha-middle:example.invalid",
        room_label: "Project Alpha",
        sender_label: "Alpha Sender",
        preview: "Middle recent update",
        timestamp_ms: 1_800_000_009_000,
        unread: true,
        highlight: true
      },
      {
        room_id: "!room-gamma:example.invalid",
        event_id: "$activity-gamma-oldest:example.invalid",
        room_label: "Project Gamma",
        sender_label: null,
        preview: "Oldest recent update",
        timestamp_ms: 1_800_000_008_000,
        unread: false,
        highlight: false
      }
    ];
    const unreadRows = [
      {
        room_id: "!room-alpha:example.invalid",
        event_id: "$activity-alpha-unread:example.invalid",
        room_label: "Project Alpha",
        sender_label: "Alpha Sender",
        preview: "Stale unread update",
        timestamp_ms: 1_800_000_001_000,
        unread: true,
        highlight: true
      },
      {
        room_id: "!room-beta:example.invalid",
        event_id: "$activity-beta-unread:example.invalid",
        room_label: "Project Beta",
        sender_label: "Beta Sender",
        preview: "Fresh unread update",
        timestamp_ms: 1_800_000_011_000,
        unread: true,
        highlight: false
      }
    ];
    const activitySnapshot = (activeTab: "recent" | "unread", nextUnreadRows = unreadRows) => {
      const snapshot = window.__harness.currentSnapshot();
      return {
        ...snapshot,
        state: {
          ...snapshot.state,
          activity: {
            kind: "open",
            active_tab: activeTab,
            recent: { rows: recentRows, next_batch: "activity-page-2" },
            unread: { rows: nextUnreadRows, next_batch: null },
            mark_read: { kind: "idle" }
          }
        }
      };
    };

    window.__harness.setCommandResponse("open_activity", () => {
      const next = activitySnapshot("recent");
      window.__harness.setSnapshot(next);
      return next;
    });
    window.__harness.setCommandResponse("set_activity_tab", ({ tab }) => {
      const snapshot = window.__harness.currentSnapshot();
      const next = {
        ...snapshot,
        state: {
          ...snapshot.state,
          activity:
            snapshot.state.activity.kind === "open"
              ? { ...snapshot.state.activity, active_tab: tab }
              : snapshot.state.activity
        }
      };
      window.__harness.setSnapshot(next);
      return next;
    });
    window.__harness.setCommandResponse("paginate_activity", () =>
      window.__harness.currentSnapshot()
    );
    window.__harness.setCommandResponse("mark_activity_read", () =>
      window.__harness.currentSnapshot()
    );
    window.__harness.setCommandResponse("select_search_result", ({ roomId, eventId }) => {
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
          thread: { kind: "closed" },
          thread_attention: { kind: "closed" },
          focused_context: {
            kind: "opening",
            room_id: String(roomId),
            event_id: String(eventId)
          }
        }
      };
      window.__harness.setSnapshot(next);
      return next;
    });
    window.__harness.clearInvocations();
  });

  await page.getByRole("button", { name: "Activity" }).click();

  await expect.poll(() => invocationCount(page, "open_activity")).toBeGreaterThanOrEqual(1);
  await expect(page.getByRole("main", { name: "Activity" })).toBeVisible();
  const recentRows = page.locator(".activity-row");
  await expect(recentRows).toHaveCount(3);
  await expect(recentRows.nth(0)).toContainText("Newest recent update");
  await expect(recentRows.nth(1)).toContainText("Middle recent update");
  await expect(recentRows.nth(2)).toContainText("Oldest recent update");

  await page.getByRole("button", { name: "Load more activity" }).click();
  await expect.poll(() => invocationCount(page, "paginate_activity")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("paginate_activity")[0]?.args)
    )
    .toEqual({
      tab: "recent",
      cursor: "activity-page-2"
    });

  await page.getByRole("button", { name: "Open activity item Project Beta" }).click();
  await expect.poll(() => invocationCount(page, "select_search_result")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("select_search_result")[0]?.args)
    )
    .toEqual({
      roomId: "!room-beta:example.invalid",
      eventId: "$activity-beta-newest:example.invalid"
    });
  await expect(page.getByText("Focused context").first()).toBeVisible();

  await page.getByRole("button", { name: "Activity" }).click();
  await page.getByLabel("Activity views").getByRole("tab", { name: "Unread" }).click();

  await expect.poll(() => invocationCount(page, "set_activity_tab")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("set_activity_tab")[0]?.args))
    .toEqual({ tab: "unread" });
  expect(await invocationCount(page, "mark_activity_read")).toBe(0);

  const alphaUnreadRow = page.locator(".activity-row").filter({
    hasText: "Stale unread update"
  });
  await expect(alphaUnreadRow).toBeVisible();
  await alphaUnreadRow.getByRole("button", { name: "Mark room read" }).click();

  await expect.poll(() => invocationCount(page, "mark_activity_read")).toBe(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("mark_activity_read")[0]?.args)
    )
    .toEqual({
      target: {
        kind: "room",
        room_id: "!room-alpha:example.invalid",
        up_to_event_id: "$activity-alpha-unread:example.invalid"
      }
    });
  await expect(alphaUnreadRow).toBeVisible();

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    if (snapshot.state.activity.kind !== "open") {
      throw new Error("expected open Activity snapshot");
    }
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        activity: {
          ...snapshot.state.activity,
          unread: {
            ...snapshot.state.activity.unread,
            rows: snapshot.state.activity.unread.rows.filter(
              (row) => row.room_id !== "!room-alpha:example.invalid"
            )
          }
        }
      }
    });
    window.__harness.pushStateChanged();
  });

  await expect(alphaUnreadRow).toHaveCount(0);
  await page.getByRole("button", { name: "Mark all read" }).click();
  await expect.poll(() => invocationCount(page, "mark_activity_read")).toBe(2);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("mark_activity_read")[1]?.args)
    )
    .toEqual({ target: { kind: "all" } });
  await expect(page.locator(".activity-row").filter({ hasText: "Fresh unread update" })).toBeVisible();

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    if (snapshot.state.activity.kind !== "open") {
      throw new Error("expected open Activity snapshot");
    }
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        activity: {
          ...snapshot.state.activity,
          unread: { rows: [], next_batch: null }
        }
      }
    });
    window.__harness.pushStateChanged();
  });

  await expect(page.getByText("No unread activity")).toBeVisible();
});

test("room management panel updates settings, roles, and members from Rust state", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate((roomId) => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        room_management: {
          selected_room_id: roomId,
          settings: {
            room_id: roomId,
            name: "Harness Room",
            topic: "Original managed topic",
            avatar_url: null,
            join_rule: "invite",
            history_visibility: "shared",
            permissions: {
              can_edit_settings: true,
              can_edit_roles: true,
              can_kick: true,
              can_ban: false,
              can_unban: false
            },
            members: [
              {
                user_id: "@target-member:example.invalid",
                display_name: "Target Member",
                display_label: "Target Member",
                original_display_label: "Target Member",
                avatar_url: null,
                power_level: 0,
                role: "user"
              }
            ]
          },
          operation: { kind: "idle" }
        }
      }
    });
    window.__harness.setCommandResponse("update_room_setting", () =>
      window.__harness.currentSnapshot()
    );
    window.__harness.setCommandResponse("moderate_room_member", () =>
      window.__harness.currentSnapshot()
    );
    window.__harness.setCommandResponse("update_room_member_role", () =>
      window.__harness.currentSnapshot()
    );
    window.__harness.pushStateChanged();
    window.__harness.clearInvocations();
  }, HARNESS_ROOM_ID);

  await page.getByRole("button", { name: "Room info" }).click();
  await expect(page.getByRole("heading", { name: "Harness Room" })).toBeVisible();
  const currentTopicRow = page.locator(".settings-detail-row").filter({
    hasText: "Current topic"
  });
  await expect(currentTopicRow.getByText("Original managed topic")).toBeVisible();

  const topicInput = page.getByRole("textbox", { name: "Room topic" });
  await topicInput.fill("Updated managed topic");
  await page.getByRole("button", { name: "Save topic" }).click();

  await expect.poll(() => invocationCount(page, "update_room_setting")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("update_room_setting")[0]?.args)
    )
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      change: { topic: "Updated managed topic" }
    });
  await expect(currentTopicRow.getByText("Original managed topic")).toBeVisible();

  await page.evaluate((roomId) => {
    const snapshot = window.__harness.currentSnapshot();
    const settings = snapshot.state.room_management.settings;
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        room_management: {
          selected_room_id: roomId,
          settings: settings
            ? { ...settings, topic: "Updated managed topic" }
            : settings,
          operation: { kind: "idle" }
        }
      }
    });
    window.__harness.pushStateChanged();
  }, HARNESS_ROOM_ID);

  await expect(currentTopicRow.getByText("Updated managed topic")).toBeVisible();
  await expect(currentTopicRow.getByText("Original managed topic")).toHaveCount(0);

  const currentAvatarRow = page.locator(".settings-detail-row").filter({
    hasText: "Current avatar"
  });
  await expect(currentAvatarRow.getByText("No avatar")).toBeVisible();
  const avatarInput = page.getByRole("textbox", { name: "Room avatar URL" });
  await avatarInput.fill("mxc://example.invalid/managed-avatar");
  await page.getByRole("button", { name: "Save avatar" }).click();

  await expect.poll(() => invocationCount(page, "update_room_setting")).toBeGreaterThanOrEqual(2);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("update_room_setting")[1]?.args)
    )
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      change: { avatarUrl: "mxc://example.invalid/managed-avatar" }
    });
  await expect(currentAvatarRow.getByText("No avatar")).toBeVisible();

  await page.evaluate((roomId) => {
    const snapshot = window.__harness.currentSnapshot();
    const settings = snapshot.state.room_management.settings;
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        room_management: {
          selected_room_id: roomId,
          settings: settings
            ? { ...settings, avatar_url: "mxc://example.invalid/managed-avatar" }
            : settings,
          operation: { kind: "idle" }
        }
      }
    });
    window.__harness.pushStateChanged();
  }, HARNESS_ROOM_ID);

  await expect(currentAvatarRow.getByText("mxc://example.invalid/managed-avatar")).toBeVisible();
  await expect(currentAvatarRow.getByText("No avatar")).toHaveCount(0);

  const targetMemberRow = page.locator(".room-member-row").filter({
    hasText: "Target Member"
  });
  const targetMemberRoleLabel = targetMemberRow.locator(".room-member-main small").filter({
    hasText: /^(User|Moderator)$/
  });
  await expect(targetMemberRoleLabel).toHaveText("User");
  await targetMemberRow
    .getByRole("combobox", { name: "Member role for Target Member" })
    .selectOption("50");

  await expect.poll(() => invocationCount(page, "update_room_member_role")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("update_room_member_role")[0]?.args)
    )
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      targetUserId: "@target-member:example.invalid",
      powerLevel: 50
    });
  await expect(targetMemberRoleLabel).toHaveText("User");

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        room_management: {
          selected_room_id: snapshot.state.room_management.selected_room_id,
          settings: snapshot.state.room_management.settings
            ? {
                ...snapshot.state.room_management.settings,
                members: snapshot.state.room_management.settings.members.map((member) =>
                  member.user_id === "@target-member:example.invalid"
                    ? { ...member, power_level: 50, role: "moderator" }
                    : member
                )
              }
            : null,
          operation: { kind: "idle" }
        }
      }
    });
    window.__harness.pushStateChanged();
  });

  await expect(targetMemberRoleLabel).toHaveText("Moderator");

  await targetMemberRow.getByRole("button", { name: "Kick Target Member" }).click();

  await expect.poll(() => invocationCount(page, "moderate_room_member")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("moderate_room_member")[0]?.args)
    )
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      targetUserId: "@target-member:example.invalid",
      action: "kick",
      reason: null
    });
  await expect(targetMemberRow).toBeVisible();

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        room_management: {
          selected_room_id: snapshot.state.room_management.selected_room_id,
          settings: snapshot.state.room_management.settings
            ? {
                ...snapshot.state.room_management.settings,
                members: []
              }
            : null,
          operation: { kind: "idle" }
        }
      }
    });
    window.__harness.pushStateChanged();
  });

  await expect(targetMemberRow).toHaveCount(0);
});

test("local aliases dispatch typed account command and render Rust-projected labels", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate((roomId) => {
    const targetUserId = "@target-member:example.invalid";
    const snapshot = window.__harness.currentSnapshot();
    const dmRoom = {
      room_id: "!dm-target-member:example.invalid",
      display_name: "Target Member",
      display_label: "Target Member",
      original_display_label: "Target Member",
      avatar: null,
      is_dm: true,
      dm_user_ids: [targetUserId],
      tags: { favourite: null, low_priority: null },
      unread_count: 0,
      notification_count: 0,
      highlight_count: 0,
      parent_space_ids: []
    };
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        profile: {
          ...snapshot.state.profile,
          users: {
            ...snapshot.state.profile.users,
            [targetUserId]: {
              user_id: targetUserId,
              display_name: "Target Member",
              display_label: "Target Member",
              original_display_label: "Target Member",
              mention_search_terms: ["Target Member", targetUserId],
              avatar: null
            }
          }
        },
        rooms: [...snapshot.state.rooms, dmRoom],
        room_management: {
          selected_room_id: roomId,
          settings: {
            room_id: roomId,
            name: "Harness Room",
            topic: null,
            avatar_url: null,
            join_rule: "invite",
            history_visibility: "shared",
            permissions: {
              can_edit_settings: true,
              can_edit_roles: true,
              can_kick: true,
              can_ban: false,
              can_unban: false
            },
            members: [
              {
                user_id: targetUserId,
                display_name: "Target Member",
                display_label: "Target Member",
                original_display_label: "Target Member",
                avatar_url: null,
                power_level: 0,
                role: "user"
              }
            ]
          },
          operation: { kind: "idle" }
        }
      },
      sidebar: {
        ...snapshot.sidebar,
        global_dms: [
          ...snapshot.sidebar.global_dms,
          {
            room_id: dmRoom.room_id,
            display_name: "Target Member",
            avatar: null,
            tags: { favourite: null, low_priority: null },
            unread_count: 0,
            highlight_count: 0
          }
        ]
      }
    });
    window.__harness.pushStateChanged();
    window.__harness.clearInvocations();
  }, HARNESS_ROOM_ID);

  await page.getByRole("button", { name: "Room info" }).click();
  const targetMemberRow = page.locator(".room-member-row").filter({
    hasText: "Target Member"
  });
  await expect(targetMemberRow).toBeVisible();
  await targetMemberRow.getByRole("button", { name: "Set alias for Target Member" }).click();
  const aliasInput = page.getByRole("textbox", { name: "Alias" });
  await aliasInput.fill("Desk Alias");
  await page.getByRole("button", { name: "Save alias" }).click();

  await expect.poll(() => invocationCount(page, "set_local_user_alias")).toBe(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("set_local_user_alias")[0]?.args)
    )
    .toEqual({
      userId: "@target-member:example.invalid",
      alias: "Desk Alias"
    });
  const aliasedMemberRow = page.locator(".room-member-row").filter({
    hasText: "Desk Alias"
  });
  await expect(aliasedMemberRow).toBeVisible();
  await expect(aliasedMemberRow.getByText("Original: Target Member")).toBeVisible();
  await expect(page.locator('[data-room-section="people"]').getByText("Desk Alias")).toBeVisible();

  await seedTimelineItems(
    page,
    [
      {
        id: { Event: { event_id: "$alias-menu-target:example.invalid" } },
        sender: "@target-member:example.invalid",
        sender_label: "Desk Alias",
        body: "Alias menu target",
        timestamp_ms: 1_800_000_003_000,
        in_reply_to_event_id: null,
        thread_root: null,
        thread_summary: null,
        reactions: [],
        can_react: true,
        is_redacted: false,
        is_hidden: false,
        can_redact: false,
        is_edited: false,
        can_edit: false
      }
    ],
    63
  );
  const timelineAliasRow = page.locator(".message").filter({ hasText: "Alias menu target" });
  await timelineAliasRow.hover();
  await timelineAliasRow.getByRole("button", { name: "Message actions" }).click();
  await timelineAliasRow.getByRole("menuitem", { name: "Edit alias for Desk Alias" }).click();
  const timelineAliasInput = page.getByRole("textbox", { name: "Alias" });
  await timelineAliasInput.fill("Timeline Alias");
  await page.getByRole("button", { name: "Save alias" }).click();
  await expect.poll(() => invocationCount(page, "set_local_user_alias")).toBe(2);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("set_local_user_alias")[1]?.args)
    )
    .toEqual({
      userId: "@target-member:example.invalid",
      alias: "Timeline Alias"
    });
  await page.evaluate(async () => {
    await window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        DisplayLabelsUpdated: {
          labels: [
            {
              user_id: "@target-member:example.invalid",
              display_label: "Timeline Alias"
            }
          ]
        }
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);
  });
  await expect(timelineAliasRow.locator(".sender")).toHaveText("Timeline Alias");
  const timelineAliasedMemberRow = page.locator(".room-member-row").filter({
    hasText: "Timeline Alias"
  });
  await expect(timelineAliasedMemberRow).toBeVisible();
  await expect(
    page.locator('[data-room-section="people"]').getByText("Timeline Alias")
  ).toBeVisible();

  await timelineAliasedMemberRow
    .getByRole("button", { name: "Clear alias for Timeline Alias" })
    .click();
  await expect.poll(() => invocationCount(page, "set_local_user_alias")).toBe(3);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("set_local_user_alias")[2]?.args)
    )
    .toEqual({
      userId: "@target-member:example.invalid",
      alias: null
    });
  await expect(page.locator(".room-member-row").filter({ hasText: "Target Member" })).toBeVisible();
  await expect(
    page.locator('[data-room-section="people"]').getByText("Target Member")
  ).toBeVisible();
  await expect(page.locator('[data-room-section="people"]').getByText("Desk Alias")).toHaveCount(0);
});

test("room tag context menu dispatches typed commands and waits for Rust section state", async ({
  page
}) => {
  await gotoReadyShell(page);
  const roomsSection = page.locator('[data-room-section="rooms"]');
  const favouritesSection = page.locator('[data-room-section="favourites"]');

  await expect(roomsSection.getByRole("button", { name: "Harness Room" })).toBeVisible();
  await expect(favouritesSection).toHaveCount(0);
  await page.evaluate(() => {
    window.__harness.setCommandResponse("set_room_tag", () =>
      window.__harness.currentSnapshot()
    );
    window.__harness.setCommandResponse("remove_room_tag", () =>
      window.__harness.currentSnapshot()
    );
    window.__harness.clearInvocations();
  });

  await page.getByRole("button", { name: "Harness Room" }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Add to Favourites" }).click();

  await expect.poll(() => invocationCount(page, "set_room_tag")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("set_room_tag")[0]?.args))
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      tag: "favourite",
      order: null
    });
  await expect(favouritesSection).toHaveCount(0);
  await expect(roomsSection.getByRole("button", { name: "Harness Room" })).toBeVisible();

  await page.evaluate((roomId) => {
    const snapshot = window.__harness.currentSnapshot();
    const tags = { favourite: { order: null }, low_priority: null };
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        rooms: snapshot.state.rooms.map((room) =>
          room.room_id === roomId ? { ...room, tags } : room
        )
      },
      sidebar: {
        ...snapshot.sidebar,
        space_rooms: snapshot.sidebar.space_rooms.map((room) =>
          room.room_id === roomId ? { ...room, tags } : room
        )
      }
    });
    window.__harness.pushStateChanged();
  }, HARNESS_ROOM_ID);

  await expect(favouritesSection.getByRole("button", { name: "Harness Room" })).toBeVisible();
  await expect(roomsSection.getByRole("button", { name: "Harness Room" })).toHaveCount(0);

  await favouritesSection.getByRole("button", { name: "Harness Room" }).click({
    button: "right"
  });
  await page.getByRole("menuitem", { name: "Remove from Favourites" }).click();

  await expect.poll(() => invocationCount(page, "remove_room_tag")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("remove_room_tag")[0]?.args)
    )
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      tag: "favourite"
    });
  await expect(favouritesSection.getByRole("button", { name: "Harness Room" })).toBeVisible();

  await page.evaluate((roomId) => {
    const snapshot = window.__harness.currentSnapshot();
    const tags = { favourite: null, low_priority: null };
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        rooms: snapshot.state.rooms.map((room) =>
          room.room_id === roomId ? { ...room, tags } : room
        )
      },
      sidebar: {
        ...snapshot.sidebar,
        space_rooms: snapshot.sidebar.space_rooms.map((room) =>
          room.room_id === roomId ? { ...room, tags } : room
        )
      }
    });
    window.__harness.pushStateChanged();
  }, HARNESS_ROOM_ID);

  await expect(roomsSection.getByRole("button", { name: "Harness Room" })).toBeVisible();
  await expect(favouritesSection).toHaveCount(0);
});

test("room sections follow Element-aligned order and render Rust-owned counts", async ({
  page
}) => {
  await gotoReadyShell(page);

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    const favouriteTags = { favourite: { order: null }, low_priority: null };
    const plainTags = { favourite: null, low_priority: null };
    const lowPriorityTags = { favourite: null, low_priority: { order: null } };
    const rooms = [
      {
        room_id: "!favourite-room:example.invalid",
        display_name: "Favourite Room",
        avatar: null,
        is_dm: false,
        tags: favouriteTags,
        unread_count: 1,
        notification_count: 1,
        highlight_count: 1,
        parent_space_ids: []
      },
      {
        room_id: "!plain-room:example.invalid",
        display_name: "Plain Room",
        avatar: null,
        is_dm: false,
        tags: plainTags,
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        parent_space_ids: []
      },
      {
        room_id: "!low-room:example.invalid",
        display_name: "Low Priority Room",
        avatar: null,
        is_dm: false,
        tags: lowPriorityTags,
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        parent_space_ids: []
      },
      {
        room_id: "!dm-room:example.invalid",
        display_name: "Direct Person",
        avatar: null,
        is_dm: true,
        tags: plainTags,
        unread_count: 2,
        notification_count: 2,
        highlight_count: 0,
        parent_space_ids: []
      }
    ];
    const toRoomListItem = (room: (typeof rooms)[number]) => ({
      room_id: room.room_id,
      display_name: room.display_name,
      avatar: room.avatar,
      tags: room.tags,
      unread_count: room.unread_count,
      notification_count: room.notification_count,
      highlight_count: room.highlight_count
    });

    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        rooms,
        navigation: {
          ...snapshot.state.navigation,
          active_room_id: "!plain-room:example.invalid"
        },
        timeline: {
          ...snapshot.state.timeline,
          room_id: "!plain-room:example.invalid"
        }
      },
      sidebar: {
        ...snapshot.sidebar,
        account_home: {
          ...snapshot.sidebar.account_home,
          unread_count: 1,
          highlight_count: 1
        },
        space_rail: snapshot.sidebar.space_rail.map((space) => ({
          ...space,
          unread_count: 1,
          highlight_count: 1
        })),
        space_rooms: rooms.filter((room) => !room.is_dm).map(toRoomListItem),
        global_dms: rooms.filter((room) => room.is_dm).map(toRoomListItem),
        space_unread_count: 1,
        dm_unread_count: 2,
        space_highlight_count: 1,
        dm_highlight_count: 0
      }
    });
    window.__harness.pushStateChanged();
  });

  await expect(page.locator('[data-room-section="favourites"]')).toBeVisible();
  await expect(page.locator('[data-room-section="people"]')).toBeVisible();
  await expect(page.locator('[data-room-section="rooms"]')).toBeVisible();
  await expect(page.locator('[data-room-section="low-priority"]')).toBeVisible();

  await expect
    .poll(() =>
      page.locator(".sidebar .room-section").evaluateAll((sections) =>
        sections.map((section) => section.getAttribute("data-room-section"))
      )
    )
    .toEqual(["favourites", "people", "rooms", "low-priority"]);

  for (const id of ["favourites", "people", "rooms", "low-priority"]) {
    await expect(page.locator(`[data-room-section="${id}"] .section-count`)).toHaveText("1");
  }

  const favouriteRoom = page
    .locator('[data-room-section="favourites"]')
    .getByRole("button", { name: "Favourite Room" });
  await expect(favouriteRoom).toHaveAttribute("data-mention-count", "1");
  await expect(favouriteRoom.locator(".room-mention-dot")).toBeVisible();
  await expect(favouriteRoom.locator(".room-count")).toHaveText("1");
  await expect(page.locator(".workspace-rail .workspace-button").first()).toHaveAttribute(
    "data-mention-count",
    "1"
  );
});

test("notification attention snapshot drives room, space, thread, and click routing headlessly", async ({
  page
}) => {
  await gotoReadyShell(page);

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    const plainTags = { favourite: null, low_priority: null };
    const lowPriorityTags = { favourite: null, low_priority: { order: null } };
    const rooms = [
      {
        room_id: "!attention-room:example.invalid",
        display_name: "Attention Room",
        avatar: null,
        is_dm: false,
        tags: plainTags,
        unread_count: 4,
        notification_count: 4,
        highlight_count: 1,
        parent_space_ids: ["!attention-space:example.invalid"]
      },
      {
        room_id: "!quiet-low:example.invalid",
        display_name: "Quiet Low Priority",
        avatar: null,
        is_dm: false,
        tags: lowPriorityTags,
        unread_count: 8,
        notification_count: 8,
        highlight_count: 0,
        parent_space_ids: ["!attention-space:example.invalid"]
      }
    ];
    const toRoomListItem = (room: (typeof rooms)[number]) => ({
      room_id: room.room_id,
      display_name: room.display_name,
      avatar: room.avatar,
      tags: room.tags,
      unread_count: room.unread_count,
      notification_count: room.notification_count,
      highlight_count: room.highlight_count
    });
    const next = {
      ...snapshot,
      state: {
        ...snapshot.state,
        navigation: {
          ...snapshot.state.navigation,
          active_room_id: "!quiet-low:example.invalid",
          active_space_id: "!attention-space:example.invalid"
        },
        rooms,
        spaces: [
          {
            space_id: "!attention-space:example.invalid",
            display_name: "Attention Space",
            avatar: null,
            child_room_ids: rooms.map((room) => room.room_id)
          }
        ],
        timeline: {
          ...snapshot.state.timeline,
          room_id: "!quiet-low:example.invalid",
          is_subscribed: true
        },
        thread_attention: {
          kind: "tracking",
          room_id: "!attention-room:example.invalid",
          root_event_id: "$attention-thread:example.invalid",
          notification_count: 2,
          highlight_count: 1,
          live_event_marker_count: 3
        },
        native_attention: {
          summary: {
            unread_count: 4,
            highlight_count: 1,
            badge_count: 4,
            candidate: {
              room_display_name: "Attention Room",
              kind: "mention",
              unread_count: 4,
              highlight_count: 1
            },
            capabilities: {
              notifications: "available",
              badge: "available",
              overlay_icon: "unavailable",
              sound: "available",
              tray: "available",
              activation: "available"
            }
          },
          dispatch: { kind: "idle" }
        }
      },
      sidebar: {
        ...snapshot.sidebar,
        active_space_id: "!attention-space:example.invalid",
        account_home: {
          ...snapshot.sidebar.account_home,
          is_active: false
        },
        space_rail: [
          {
            space_id: "!attention-space:example.invalid",
            display_name: "Attention Space",
            avatar: null,
            unread_count: 4,
            highlight_count: 1,
            is_active: true
          }
        ],
        space_rooms: rooms.map(toRoomListItem),
        global_dms: [],
        space_unread_count: 4,
        dm_unread_count: 0,
        space_highlight_count: 1,
        dm_highlight_count: 0
      }
    };
    window.__harness.setSnapshot(next);
    window.__harness.setCommandResponse("select_room", ({ roomId }) => {
      const current = window.__harness.currentSnapshot();
      const updated = {
        ...current,
        state: {
          ...current.state,
          navigation: {
            ...current.state.navigation,
            active_room_id: String(roomId)
          },
          timeline: {
            ...current.state.timeline,
            room_id: String(roomId),
            is_subscribed: true
          }
        }
      };
      window.__harness.setSnapshot(updated);
      return updated;
    });
    window.__harness.pushStateChanged();
    window.__harness.clearInvocations();
  });

  await expect(page.locator('[data-room-section="rooms"]')).toBeVisible();
  await expect(page.locator('[data-room-section="low-priority"]')).toBeVisible();
  const attentionRoom = page.getByRole("button", { name: "Attention Room" });
  const lowPriorityRoom = page.getByRole("button", { name: "Quiet Low Priority" });
  await expect(attentionRoom.locator(".room-count")).toHaveText("4");
  await expect(attentionRoom.locator(".room-mention-dot")).toBeVisible();
  await expect(lowPriorityRoom.locator(".room-count")).toHaveText("8");
  const attentionSpace = page.getByRole("button", { name: "Attention Space" });
  await expect(attentionSpace).toHaveAttribute("data-count", "4");
  await expect(attentionSpace).toHaveAttribute(
    "data-mention-count",
    "1"
  );
  const sidebarThreadsButton = page
    .getByRole("complementary", { name: "Rooms" })
    .getByRole("button", { name: "Threads" });
  await expect(sidebarThreadsButton).toHaveAttribute("data-count", "2");
  await expect(sidebarThreadsButton).toHaveAttribute("data-mention-count", "1");
  await expect(sidebarThreadsButton).toHaveAttribute("data-live-count", "3");
  await expect(page).toHaveTitle("matrix-desktop · 4 unread");

  await attentionRoom.click();
  await expect.poll(() => invocationCount(page, "select_room")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("select_room")[0]?.args)
    )
    .toEqual({ roomId: "!attention-room:example.invalid" });
  await expect(attentionRoom).toHaveClass(/is-active/);
});

test("workspace rail space and account buttons show reusable tooltips on hover and focus", async ({
  page
}) => {
  await gotoReadyShell(page);

  const rail = page.getByRole("navigation", { name: "Workspaces" });
  const spaceButton = rail.getByRole("button", { name: "Harness Space" });
  const accountButton = rail.getByRole("button", { name: "Home" });

  await spaceButton.hover();
  const spaceTooltip = page.getByRole("tooltip", { name: "Harness Space" });
  await expect(spaceTooltip).toBeVisible();
  const tooltipId = await spaceTooltip.getAttribute("id");
  if (!tooltipId) {
    throw new Error("workspace tooltip id missing");
  }
  await expect(spaceButton).toHaveAttribute("aria-describedby", tooltipId);

  await page.keyboard.press("Escape");
  await expect(spaceTooltip).toBeHidden();
  await expect(spaceButton).not.toHaveAttribute("aria-describedby", /.+/);

  await accountButton.hover();
  const accountTooltip = page.getByRole("tooltip", { name: "Home" });
  await expect(accountTooltip).toBeVisible();

  await page.getByRole("main", { name: "Conversation timeline" }).hover();
  await expect(accountTooltip).toBeHidden();

  await spaceButton.focus();
  await expect(spaceTooltip).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(spaceTooltip).toBeHidden();
  await expect(spaceButton).not.toHaveAttribute("aria-describedby", /.+/);
});

test("mention autocomplete inserts a pill and sends typed mention intent", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        profile: {
          ...snapshot.state.profile,
          users: {
            ...snapshot.state.profile.users,
            "@alice:example.invalid": {
              user_id: "@alice:example.invalid",
              display_name: "Alice",
              display_label: "Alice",
              original_display_label: "Alice",
              mention_search_terms: ["Alice", "@alice:example.invalid"],
              avatar: null
            }
          }
        }
      }
    });
    window.__harness.pushStateChanged();
    window.__harness.clearInvocations();
  });
  await pushTimelineDiffs(
    page,
    [
      {
        Set: {
          index: 0,
          item: {
            id: { Event: { event_id: "$seed-event:example.invalid" } },
            sender: "@harness-user:example.invalid",
            body: "Timeline mentions @Alice from Rust profile data",
            timestamp_ms: 1_800_000_000_000,
            in_reply_to_event_id: null,
            thread_root: null,
            thread_summary: null,
            can_react: true,
            is_redacted: false,
            is_hidden: false,
            can_redact: true,
            is_edited: false,
            can_edit: true,
            reactions: []
          }
        }
      }
    ],
    1
  );
  const timelineMention = page.locator(".message-body .message-mention-pill", {
    hasText: "@Alice"
  });
  await expect(timelineMention).toBeVisible();
  await expect(timelineMention).toHaveAttribute(
    "data-mention-user-id",
    "@alice:example.invalid"
  );

  const composer = page.getByRole("textbox", { name: "Message composer" });
  await composer.fill("@a");
  await expect(page.getByRole("listbox", { name: "Mention suggestions" })).toBeVisible();
  await page.getByRole("option", { name: "Alice" }).click();
  await expect(
    page.getByLabel("Selected mentions").getByText("@Alice", { exact: true })
  ).toBeVisible();

  await page.getByRole("button", { name: "Send", exact: true }).click();

  await expect.poll(() => invocationCount(page, "send_text")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("send_text")[0]?.args))
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      body: "@Alice ",
      mentions: {
        targets: [
          {
            kind: "user",
            user_id: "@alice:example.invalid",
            display_label: "Alice"
          }
        ]
      }
    });
});

test("markdown toolbar and slash composer input dispatch Rust-owned send bodies", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  const composer = page.getByRole("textbox", { name: "Message composer" });
  await composer.fill("world");
  await composer.selectText();
  await page.getByRole("button", { name: "Bold" }).click();
  await expect(composer).toHaveValue("**world**");
  await page.getByRole("button", { name: "Send", exact: true }).click();

  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("send_text")[0]?.args))
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      body: "**world**",
      mentions: { targets: [] }
    });

  await page.evaluate(() => window.__harness.clearInvocations());
  await composer.fill("/me waves");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("send_text")[0]?.args))
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      body: "/me waves",
      mentions: { targets: [] }
  });
});

test("main composer draft is Rust snapshot scoped per room", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const primaryRoomId = "!harness-room:example.invalid";
    const secondaryRoomId = "!draft-room-b:example.invalid";
    const draftByRoom: Record<string, string> = {};
    const base = window.__harness.currentSnapshot();
    const rooms = [
      base.state.rooms[0],
      {
        room_id: secondaryRoomId,
        display_name: "Draft Room B",
        display_label: "Draft Room B",
        original_display_label: "Draft Room B",
        avatar: null,
        is_dm: false,
        dm_user_ids: [],
        tags: { favourite: null, low_priority: null },
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        parent_space_ids: []
      }
    ];
    const roomListItems = rooms.map((room) => ({
      room_id: room.room_id,
      display_name: room.display_label,
      avatar: room.avatar,
      tags: room.tags,
      unread_count: room.unread_count,
      highlight_count: room.highlight_count
    }));
    const projectRoom = (roomId: string) => {
      const current = window.__harness.currentSnapshot();
      return {
        ...current,
        state: {
          ...current.state,
          navigation: {
            ...current.state.navigation,
            active_room_id: roomId
          },
          rooms,
          timeline: {
            ...current.state.timeline,
            room_id: roomId,
            is_subscribed: true,
            composer: {
              pending_transaction_id: null,
              draft: draftByRoom[roomId] ?? "",
              mode: "Plain"
            }
          },
          thread: { kind: "closed" },
          focused_context: { kind: "closed" }
        },
        sidebar: {
          ...current.sidebar,
          space_rooms: roomListItems
        },
        thread: null
      };
    };

    window.__harness.setSnapshot(projectRoom(primaryRoomId));
    window.__harness.setCommandResponse(
      "set_composer_draft",
      ({ roomId, draft }: { roomId: string; draft: string }) => {
        const normalizedRoomId = String(roomId);
        if (draft.length === 0) {
          delete draftByRoom[normalizedRoomId];
        } else {
          draftByRoom[normalizedRoomId] = draft;
        }
        const next = projectRoom(window.__harness.currentSnapshot().state.timeline.room_id ?? primaryRoomId);
        window.__harness.setSnapshot(next);
        return next;
      }
    );
    window.__harness.setCommandResponse("select_room", ({ roomId }: { roomId: string }) => {
      const next = projectRoom(String(roomId));
      window.__harness.setSnapshot(next);
      return next;
    });
    window.__harness.pushStateChanged();
    window.__harness.clearInvocations();
  });

  const composer = page.getByRole("textbox", { name: "Message composer" });
  await composer.fill("Room A draft");
  await expect(composer).toHaveValue("Room A draft");
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("set_composer_draft").at(-1)?.args)
    )
    .toEqual({ roomId: HARNESS_ROOM_ID, draft: "Room A draft" });

  await page.getByRole("button", { name: "Draft Room B" }).click();
  await expect(composer).toHaveValue("");
  await composer.fill("Room B draft");
  await expect(composer).toHaveValue("Room B draft");

  await page.getByRole("button", { name: "Harness Room" }).click();
  await expect(composer).toHaveValue("Room A draft");
  await page.getByRole("button", { name: "Draft Room B" }).click();
  await expect(composer).toHaveValue("Room B draft");
});

test("scheduled send UI dispatches typed commands and waits for Rust snapshot changes", async ({
  page
}) => {
  await gotoReadyShell(page);
  const initialSendAt = await page.evaluate(() => new Date("2030-01-02T03:04:00").getTime());
  const editedSendAt = await page.evaluate(() => new Date("2030-01-03T04:05:00").getTime());

  await page.evaluate(
    ({ initialSendAt, editedSendAt }) => {
      const scheduledId = "scheduled-harness-1";
      const projectScheduled = (
        items: Array<{
          scheduled_id: string;
          room_id: string;
          body: string;
          send_at_ms: number;
          handle: { kind: "local" } | { kind: "server"; delay_id: string };
        }>,
        draft = window.__harness.currentSnapshot().state.timeline.composer.draft
      ) => {
        const current = window.__harness.currentSnapshot();
        return {
          ...current,
          state: {
            ...current.state,
            timeline: {
              ...current.state.timeline,
              scheduled_send_capability: "localFallback",
              scheduled_sends: items,
              composer: {
                ...current.state.timeline.composer,
                draft
              }
            }
          }
        };
      };
      const scheduledItem = {
        scheduled_id: scheduledId,
        room_id: "!harness-room:example.invalid",
        body: "Phase B scheduled body",
        send_at_ms: initialSendAt,
        handle: { kind: "local" } as const
      };
      const editedItem = {
        ...scheduledItem,
        send_at_ms: editedSendAt
      };

      window.__harness.setSnapshot(projectScheduled([], ""));
      window.__harness.setCommandResponse(
        "schedule_send",
        ({ body }: { body: string; sendAtMs: number }) => {
          const next = projectScheduled([{ ...scheduledItem, body: String(body) }], "");
          window.__harness.setSnapshot(next);
          return next;
        }
      );
      window.__harness.setCommandResponse("reschedule_scheduled_send", () =>
        window.__harness.currentSnapshot()
      );
      window.__harness.setCommandResponse("cancel_scheduled_send", () =>
        window.__harness.currentSnapshot()
      );
      window.__harness.pushStateChanged();
      window.__harness.clearInvocations();
    },
    { initialSendAt, editedSendAt }
  );

  const composer = page.getByRole("textbox", { name: "Message composer" });
  await composer.fill("Phase B scheduled body");
  await page.getByRole("button", { name: "Send later" }).click();
  const scheduleInput = page.getByLabel("Scheduled send time");
  await expect(scheduleInput).toHaveAttribute("aria-label", "Scheduled send time");
  await scheduleInput.fill("2030-01-02T03:04");
  await page.getByRole("button", { name: "Schedule send" }).click();

  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("schedule_send")[0]?.args))
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      body: "Phase B scheduled body",
      sendAtMs: initialSendAt
    });
  await expect(page.getByRole("region", { name: "Scheduled messages" })).toContainText(
    "Phase B scheduled body"
  );
  await expect(page.getByRole("textbox", { name: "Message composer" })).toHaveValue("");

  await page.getByRole("button", { name: "Edit scheduled send" }).click();
  await page.getByLabel("Scheduled send time").fill("2030-01-03T04:05");
  await page.getByRole("button", { name: "Save scheduled send" }).click();
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("reschedule_scheduled_send")[0]?.args)
    )
    .toEqual({
      scheduledId: "scheduled-harness-1",
      sendAtMs: editedSendAt
    });
  await expect(page.getByRole("region", { name: "Scheduled messages" })).not.toContainText(
    "Jan 3"
  );
  await page.evaluate(({ editedSendAt }) => {
    const current = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...current,
      state: {
        ...current.state,
        timeline: {
          ...current.state.timeline,
          scheduled_sends: current.state.timeline.scheduled_sends.map((item) =>
            item.scheduled_id === "scheduled-harness-1"
              ? { ...item, send_at_ms: editedSendAt }
              : item
          )
        }
      }
    });
    window.__harness.pushStateChanged();
  }, { editedSendAt });
  await expect(page.getByRole("region", { name: "Scheduled messages" })).toContainText(
    "Jan 3"
  );

  await page.getByRole("button", { name: "Cancel scheduled send" }).click();
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("cancel_scheduled_send")[0]?.args)
    )
    .toEqual({ scheduledId: "scheduled-harness-1" });
  await expect(page.getByRole("region", { name: "Scheduled messages" })).toContainText(
    "Phase B scheduled body"
  );
  await page.evaluate(() => {
    const current = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...current,
      state: {
        ...current.state,
        timeline: {
          ...current.state.timeline,
          scheduled_sends: []
        }
      }
    });
    window.__harness.pushStateChanged();
  });
  await expect(page.getByRole("region", { name: "Scheduled messages" })).toBeHidden();
});

test("main composer composing Enter never sends or accepts mention autocomplete", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        profile: {
          ...snapshot.state.profile,
          users: {
            "@alice:example.invalid": {
              user_id: "@alice:example.invalid",
              display_name: "Alice",
              display_label: "Alice",
              original_display_label: "Alice",
              mention_search_terms: ["Alice", "@alice:example.invalid"],
              avatar: null
            }
          }
        }
      }
    });
    window.__harness.pushStateChanged();
    window.__harness.clearInvocations();
  });

  const composer = page.getByRole("textbox", { name: "Message composer" });
  await composer.fill("@a");
  await expect(page.getByRole("listbox", { name: "Mention suggestions" })).toBeVisible();
  await composer.evaluate((element) => {
    const event = new KeyboardEvent("keydown", {
      bubbles: true,
      cancelable: true,
      key: "Enter"
    });
    Object.defineProperty(event, "isComposing", { value: true });
    element.dispatchEvent(event);
  });

  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("resolve_composer_key_action")[0]?.args)
    )
    .toMatchObject({
      surface: "main",
      keyEvent: { key: "enter", is_composing: true },
      autocompleteOpen: true,
      sendEnabled: true
    });
  expect(await invocationCount(page, "send_text")).toBe(0);
  await expect(page.getByRole("listbox", { name: "Mention suggestions" })).toBeVisible();
  await expect(composer).toHaveValue("@a");
});

test("thread and edit composers composing Enter never send through GUI", async ({
  page
}) => {
  await gotoReadyShell(page);

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        thread: {
          kind: "open",
          room_id: "!harness-room:example.invalid",
          root_event_id: "$seed-event:example.invalid",
          is_subscribed: true,
          composer: {
            pending_transaction_id: null,
            draft: "",
            mode: "Plain"
          }
        },
        thread_attention: {
          kind: "tracking",
          room_id: "!harness-room:example.invalid",
          root_event_id: "$seed-event:example.invalid",
          notification_count: 0,
          highlight_count: 0,
          live_event_marker_count: 0
        }
      },
      thread: null
    });
    window.__harness.pushStateChanged();
  });
  const threadComposer = page.getByRole("textbox", { name: t("timeline.threadComposer") });
  await expect(threadComposer).toBeVisible();
  await threadComposer.fill("スレッド変換中");
  await page.evaluate(() => window.__harness.clearInvocations());

  await expect(await dispatchComposingEnter(threadComposer)).toBe(false);

  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("resolve_composer_key_action")[0]?.args)
    )
    .toMatchObject({
      surface: "thread",
      keyEvent: { key: "enter", is_composing: true },
      autocompleteOpen: false,
      sendEnabled: true
    });
  expect(await invocationCount(page, "send_thread_reply")).toBe(0);
  await expect(threadComposer).toHaveValue("スレッド変換中");

  const row = page.locator('[data-event-id="$seed-event:example.invalid"]');
  await row
    .locator(`button[aria-label="${t("timeline.editMessage")}"]`)
    .first()
    .evaluate((button) => (button as HTMLButtonElement).click());
  const editBody = page.getByRole("textbox", { name: t("timeline.editBody") });
  await expect(editBody).toBeVisible();
  await editBody.fill("編集変換中");
  await page.evaluate(() => window.__harness.clearInvocations());

  await expect(await dispatchComposingEnter(editBody)).toBe(false);

  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("resolve_composer_key_action")[0]?.args)
    )
    .toMatchObject({
      surface: "edit",
      keyEvent: { key: "enter", is_composing: true },
      autocompleteOpen: false,
      sendEnabled: true
    });
  expect(await invocationCount(page, "edit_message")).toBe(0);
  await expect(editBody).toHaveValue("編集変換中");
});

test("send queue rows dispatch retry and cancel commands from Rust-owned send state", async ({
  page
}) => {
  await gotoReadyShell(page);
  const firstFailed = makeSendQueueItem(
    "txn-failed-first",
    "Synthetic failed send one",
    { kind: "notSent", reason: "recoverable" }
  );
  const secondFailed = makeSendQueueItem(
    "txn-failed-second",
    "Synthetic failed send two",
    { kind: "notSent", reason: "recoverable" }
  );
  const sending = makeSendQueueItem("txn-sending", "Synthetic pending send", {
    kind: "sending"
  });
  await seedTimelineItems(page, [firstFailed, secondFailed, sending]);

  const firstRow = page.locator('[data-item-id="txn:txn-failed-first"]');
  const secondRow = page.locator('[data-item-id="txn:txn-failed-second"]');
  const sendingRow = page.locator('[data-item-id="txn:txn-sending"]');
  await expect(firstRow).toHaveAttribute("data-send-state", "notSent");
  await expect(firstRow.getByText("Not sent")).toBeVisible();
  await expect(firstRow.getByRole("button", { name: "Resend" })).toBeVisible();
  await expect(firstRow.getByRole("button", { name: "Delete" })).toBeVisible();
  await expect(page.getByText("Some messages haven't been sent")).toBeVisible();
  await expect(sendingRow).toHaveAttribute("data-send-state", "sending");
  await expect(sendingRow.getByText("Sending")).toBeVisible();
  await expect(sendingRow.getByRole("button", { name: "Cancel send" })).toBeVisible();

  await page.evaluate(() => window.__harness.clearInvocations());
  await firstRow.getByRole("button", { name: "Resend" }).click();
  await expect.poll(() => invocationCount(page, "retry_send")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("retry_send")[0]?.args))
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      transactionId: "txn-failed-first"
    });

  await pushTimelineDiffs(page, [
    {
      Set: {
        index: 0,
        item: makeSendQueueItem("txn-failed-first", "Synthetic failed send one", {
          kind: "sending"
        })
      }
    }
  ], 2, 3);
  await expect(firstRow).toHaveAttribute("data-send-state", "sending");
  await expect(firstRow.getByText("Sending")).toBeVisible();
  await expect(firstRow.getByRole("button", { name: "Resend" })).toHaveCount(0);

  await page.evaluate(() => window.__harness.clearInvocations());
  await secondRow.getByRole("button", { name: "Delete" }).click();
  await expect.poll(() => invocationCount(page, "cancel_send")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("cancel_send")[0]?.args))
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      transactionId: "txn-failed-second"
    });
  await pushTimelineDiffs(page, [{ Remove: { index: 1 } }], 2, 4);
  await expect(secondRow).toHaveCount(0);

  await page.evaluate(() => window.__harness.clearInvocations());
  await sendingRow.getByRole("button", { name: "Cancel send" }).click();
  await expect.poll(() => invocationCount(page, "cancel_send")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("cancel_send")[0]?.args))
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      transactionId: "txn-sending"
    });
});

test("send queue room bar resends failed transactions in timeline order", async ({ page }) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [
    makeSendQueueItem("txn-fifo-first", "Synthetic FIFO send one", {
      kind: "notSent",
      reason: "recoverable"
    }),
    makeSendQueueItem("txn-fifo-second", "Synthetic FIFO send two", {
      kind: "notSent",
      reason: "recoverable"
    })
  ]);

  await page.evaluate(() => window.__harness.clearInvocations());
  await page.getByRole("button", { name: "Resend all" }).click();

  await expect.poll(() => invocationCount(page, "retry_send")).toBe(2);
  await expect
    .poll(async () =>
      page.evaluate(() =>
        window.__harness
          .invocationsOf("retry_send")
          .map((invocation) => invocation.args.transactionId)
      )
    )
    .toEqual(["txn-fifo-first", "txn-fifo-second"]);
});

test("clicking an unselected reaction pill invokes send_reaction", async ({ page }) => {
  await gotoReadyShell(page);
  await expect(page.getByRole("button", { name: "Reaction 👍, count 1" }).first()).toBeVisible();
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.getByRole("button", { name: "Reaction 👍, count 1" }).first().click();

  await expect.poll(() => invocationCount(page, "send_reaction")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("send_reaction")[0]?.args)
    )
    .toEqual({
      roomId: "!harness-room:example.invalid",
      eventId: "$seed-event:example.invalid",
      reactionKey: "👍"
    });
  expect(await invocationCount(page, "redact_reaction")).toBe(0);
});

test("clicking an own reaction pill invokes redact_reaction", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        ItemsUpdated: {
          key: {
            account_key: "@harness-user:example.invalid",
            kind: { Room: { room_id: "!harness-room:example.invalid" } }
          },
          generation: 1,
          batch_id: 2,
          diffs: [
            {
              Set: {
                index: 0,
                item: {
                  id: { Event: { event_id: "$seed-event:example.invalid" } },
                  sender: "@harness-user:example.invalid",
                  body: "Seed message for reply target",
                  timestamp_ms: 1_800_000_000_000,
                  in_reply_to_event_id: null,
                  thread_root: null,
                  thread_summary: null,
                  can_react: true,
                  is_redacted: false,
                  is_hidden: false,
                  can_redact: true,
                  is_edited: false,
                  can_edit: true,
                  reactions: [
                    {
                      key: "👍",
                      count: 2,
                      reacted_by_me: true,
                      my_reaction_event_id: "$reaction-own:example.invalid",
                      sender_preview: [
                        "@harness-user:example.invalid",
                        "@other-user:example.invalid"
                      ]
                    }
                  ]
                }
              }
            }
          ]
        }
      }
    });
  });
  const pill = page.getByRole("button", { name: "Reaction 👍, count 2" }).first();
  await expect(pill).toBeVisible();
  await expect(pill).toHaveAttribute("aria-pressed", "true");
  await page.evaluate(() => window.__harness.clearInvocations());

  await pill.click();

  await expect.poll(() => invocationCount(page, "redact_reaction")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("redact_reaction")[0]?.args)
    )
    .toEqual({
      roomId: "!harness-room:example.invalid",
      eventId: "$seed-event:example.invalid",
      reactionKey: "👍",
      reactionEventId: "$reaction-own:example.invalid"
    });
  expect(await invocationCount(page, "send_reaction")).toBe(0);
});

test("add reaction picker invokes send_reaction with the selected emoji", async ({ page }) => {
  await gotoReadyShell(page);
  await page.locator('[data-event-id="$seed-event:example.invalid"]').hover();
  await expect(page.getByRole("button", { name: "Add reaction" }).first()).toBeVisible();
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.getByRole("button", { name: "Add reaction" }).first().click();
  await expect(page.getByRole("button", { name: "React with 👀" })).toBeVisible();
  await page.getByRole("button", { name: "React with 👀" }).click();

  await expect.poll(() => invocationCount(page, "send_reaction")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("send_reaction")[0]?.args)
    )
    .toEqual({
      roomId: "!harness-room:example.invalid",
      eventId: "$seed-event:example.invalid",
      reactionKey: "👀"
    });
  expect(await invocationCount(page, "redact_reaction")).toBe(0);
});

test("reply quote block renders from Rust-owned timeline item data", async ({ page }) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [
    {
      id: { Event: { event_id: "$reply:example.invalid" } },
      sender: "@harness-user:example.invalid",
      body: "Reply from harness",
      timestamp_ms: 1_800_000_000_100,
      in_reply_to_event_id: "$root:example.invalid",
      reply_quote: {
        event_id: "$root:example.invalid",
        sender: "@quoted-user:example.invalid",
        body_preview: "Quoted source from Rust state",
        state: "ready"
      },
      thread_root: null,
      thread_summary: null,
      reactions: [],
      can_react: true,
      is_redacted: false,
      is_hidden: false,
      can_redact: false,
      is_edited: false,
      can_edit: false
    }
  ]);

  const row = page.locator('[data-event-id="$reply:example.invalid"]');
  await expect(row.locator(".reply-quote")).toBeVisible();
  await expect(row.getByText("@quoted-user:example.invalid", { exact: true })).toBeVisible();
  await expect(row.getByText("Quoted source from Rust state", { exact: true })).toBeVisible();
  await expect(row).not.toContainText("$root:example.invalid");
});

test("pin and unpin actions dispatch typed commands and pinned banner waits for Rust state", async ({
  page
}) => {
  await gotoReadyShell(page);
  const row = page.locator('[data-event-id="$seed-event:example.invalid"]');
  const pinnedRegion = page.getByRole("region", { name: "Pinned messages" });

  await row.hover();
  await expect(row.getByRole("button", { name: "Pin message" })).toBeVisible();
  await page.evaluate(() => window.__harness.clearInvocations());

  await row.getByRole("button", { name: "Pin message" }).click();

  await expect.poll(() => invocationCount(page, "pin_event")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("pin_event")[0]?.args))
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      eventId: "$seed-event:example.invalid"
    });
  await expect(pinnedRegion).toHaveCount(0);

  await page.evaluate((roomId) => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        room_interactions: {
          ...snapshot.state.room_interactions,
          [roomId]: {
            pinned_events: [
              {
                event_id: "$seed-event:example.invalid",
                sender: "@harness-user:example.invalid",
                body_preview: "Pinned preview from Rust state",
                redacted: false
              }
            ],
            pin_operation: { kind: "idle" }
          }
        }
      }
    });
    window.__harness.pushStateChanged();
  }, HARNESS_ROOM_ID);

  await expect(pinnedRegion).toBeVisible();
  await expect(pinnedRegion.getByText("Pinned preview from Rust state", { exact: true })).toBeVisible();

  await row.hover();
  await expect(row.getByRole("button", { name: "Unpin message" })).toBeVisible();
  await page.evaluate(() => window.__harness.clearInvocations());
  await row.getByRole("button", { name: "Unpin message" }).click();

  await expect.poll(() => invocationCount(page, "unpin_event")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("unpin_event")[0]?.args))
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      eventId: "$seed-event:example.invalid"
    });
  await expect(pinnedRegion).toBeVisible();

  await page.evaluate((roomId) => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        room_interactions: {
          ...snapshot.state.room_interactions,
          [roomId]: {
            pinned_events: [],
            pin_operation: { kind: "idle" }
          }
        }
      }
    });
    window.__harness.pushStateChanged();
  }, HARNESS_ROOM_ID);

  await expect(pinnedRegion).toHaveCount(0);
});

test("pin and unpin actions render the Tauri snapshot response without a manual state event", async ({
  page
}) => {
  await gotoReadyShell(page);
  const row = page.locator('[data-event-id="$seed-event:example.invalid"]');
  const pinnedRegion = page.getByRole("region", { name: "Pinned messages" });

  await page.evaluate((roomId) => {
    window.__harness.setCommandResponse("pin_event", () => {
      const snapshot = window.__harness.currentSnapshot();
      return {
        ...snapshot,
        state: {
          ...snapshot.state,
          room_interactions: {
            ...snapshot.state.room_interactions,
            [roomId]: {
              pinned_events: [
                {
                  event_id: "$seed-event:example.invalid",
                  sender: "@harness-user:example.invalid",
                  body_preview: "Pinned from Tauri response",
                  redacted: false
                }
              ],
              pin_operation: { kind: "idle" }
            }
          }
        }
      };
    });
    window.__harness.setCommandResponse("unpin_event", () => {
      const snapshot = window.__harness.currentSnapshot();
      return {
        ...snapshot,
        state: {
          ...snapshot.state,
          room_interactions: {
            ...snapshot.state.room_interactions,
            [roomId]: {
              pinned_events: [],
              pin_operation: { kind: "idle" }
            }
          }
        }
      };
    });
    window.__harness.clearInvocations();
  }, HARNESS_ROOM_ID);

  await row.hover();
  await row.getByRole("button", { name: "Pin message" }).click();
  await expect.poll(() => invocationCount(page, "pin_event")).toBeGreaterThanOrEqual(1);
  await expect(pinnedRegion.getByText("Pinned from Tauri response", { exact: true })).toBeVisible();

  await row.hover();
  await row.getByRole("button", { name: "Unpin message" }).click();
  await expect.poll(() => invocationCount(page, "unpin_event")).toBeGreaterThanOrEqual(1);
  await expect(pinnedRegion).toHaveCount(0);
});

test("message action menu copies Rust-owned body and permalink values", async ({ page }) => {
  await page.addInitScript(() => {
    let clipboardText = "";
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: async (value: string) => {
          clipboardText = value;
        },
        readText: async () => clipboardText
      }
    });
  });
  await gotoReadyShell(page);
  await seedTimelineItems(page, [
    {
      id: { Event: { event_id: "$actions-copy:example.invalid" } },
      sender: "@harness-user:example.invalid",
      body: "Copy body from Rust timeline item",
      timestamp_ms: 1_800_000_000_300,
      in_reply_to_event_id: null,
      reply_quote: null,
      thread_root: null,
      thread_summary: null,
      reactions: [],
      can_react: false,
      is_redacted: false,
      is_hidden: false,
      can_redact: false,
      is_edited: false,
      can_edit: false,
      actions: {
        can_copy: true,
        can_forward: true,
        can_permalink: true,
        can_view_source: true,
        permalink: "https://matrix.to/#/!harness-room%3Aexample.invalid/%24actions-copy%3Aexample.invalid"
      }
    }
  ]);

  const row = page.locator('[data-event-id="$actions-copy:example.invalid"]');
  await row.hover();
  await row.getByRole("button", { name: "Message actions" }).click();
  await row.getByRole("menuitem", { name: "Copy message" }).click();
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(
    "Copy body from Rust timeline item"
  );

  await row.hover();
  await row.getByRole("button", { name: "Message actions" }).click();
  await row.getByRole("menuitem", { name: "Copy permalink" }).click();
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(
    "https://matrix.to/#/!harness-room%3Aexample.invalid/%24actions-copy%3Aexample.invalid"
  );
});

test("message action menu dispatches source and forward through typed Rust contracts", async ({
  page
}) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [
    {
      id: { Event: { event_id: "$actions-source:example.invalid" } },
      sender: "@harness-user:example.invalid",
      body: "Forward body stays in Rust",
      timestamp_ms: 1_800_000_000_400,
      in_reply_to_event_id: null,
      reply_quote: null,
      thread_root: null,
      thread_summary: null,
      reactions: [],
      can_react: false,
      is_redacted: false,
      is_hidden: false,
      can_redact: false,
      is_edited: false,
      can_edit: false,
      actions: {
        can_copy: true,
        can_forward: true,
        can_permalink: true,
        can_view_source: true,
        permalink: "https://matrix.to/#/!harness-room%3Aexample.invalid/%24actions-source%3Aexample.invalid"
      }
    }
  ]);

  const row = page.locator('[data-event-id="$actions-source:example.invalid"]');
  await page.evaluate(() => window.__harness.clearInvocations());
  await row.hover();
  await row.getByRole("button", { name: "Message actions" }).click();
  await row.getByRole("menuitem", { name: "View source" }).click();

  await expect.poll(() => invocationCount(page, "load_message_source")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("load_message_source")[0]?.args)
    )
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      eventId: "$actions-source:example.invalid"
    });
  await expect(page.getByRole("dialog", { name: "Message source" })).toHaveCount(0);

  await page.evaluate((key) => {
    void window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        MessageSourceLoaded: {
          request_id: { connection_id: 1, sequence: 41 },
          key,
          source: {
            event_id: "$actions-source:example.invalid",
            sender: "@harness-user:example.invalid",
            timestamp_ms: 1_800_000_000_400,
            body: "Source body projected by Rust",
            in_reply_to_event_id: null,
            thread_root: null,
            is_redacted: false,
            is_hidden: false,
            is_edited: true,
            has_media: false
          }
        }
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);
  }, HARNESS_ROOM_KEY);

  const sourceDialog = page.getByRole("dialog", { name: "Message source" });
  await expect(sourceDialog).toBeVisible();
  await expect(sourceDialog.getByText("Source body projected by Rust", { exact: true })).toBeVisible();
  await expect(sourceDialog).toContainText("Edited");
  await sourceDialog.getByRole("button", { name: "Close message source" }).click();
  await expect(sourceDialog).toHaveCount(0);

  await row.hover();
  await row.getByRole("button", { name: "Message actions" }).click();
  await row.getByRole("menuitem", { name: "Forward" }).click();
  await row.getByRole("menuitem", { name: "Harness Room" }).click();

  await expect.poll(() => invocationCount(page, "forward_message")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("forward_message")[0]?.args)
    )
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      sourceEventId: "$actions-source:example.invalid",
      destinationRoomId: HARNESS_ROOM_ID
    });
});

test("attach control stages media caption and renders Rust-owned media progress", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    window.__harness.setCommandResponse("upload_media", () => window.__harness.currentSnapshot());
    window.__harness.setCommandResponse("download_media", () => window.__harness.currentSnapshot());
    window.__harness.clearInvocations();
  });

  const fixtureBytes = Buffer.from("browser-headless media fixture");
  await page.getByRole("textbox", { name: "Message composer" }).fill("single **event** caption");
  await page.getByRole("button", { name: "Attach file", exact: true }).click();
  await page
    .locator('input[type="file"][aria-label="Attach file input"]')
    .setInputFiles({
      name: "media-fixture.txt",
      mimeType: "text/plain",
      buffer: fixtureBytes
    });

  await expect(page.getByText("media-fixture.txt", { exact: true })).toBeVisible();
  await page
    .getByRole("textbox", { name: "Caption for media-fixture.txt" })
    .fill("single **event** caption");
  await expect.poll(() => invocationCount(page, "upload_media")).toBe(0);
  await page.getByRole("button", { name: "Send", exact: true }).click();

  await expect.poll(() => invocationCount(page, "upload_media")).toBeGreaterThanOrEqual(1);
  await expect.poll(() => invocationCount(page, "send_text")).toBe(0);
  await expect
    .poll(async () =>
      page.evaluate(() => {
        const args = window.__harness.invocationsOf("upload_media")[0]?.args;
        return args
          ? {
              roomId: args.roomId,
              filename: args.filename,
              mimeType: args.mimeType,
              caption: args.caption,
              byteCount: Array.isArray(args.bytes) ? args.bytes.length : -1
            }
          : null;
      })
    )
    .toEqual({
      roomId: "!harness-room:example.invalid",
      filename: "media-fixture.txt",
      mimeType: "text/plain",
      caption: "single **event** caption",
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
                  body: "single **event** caption",
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
                  is_hidden: false,
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
                  is_hidden: false,
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
  await expect(mediaRow.locator(".message-media + .message-body")).toContainText(
    "single **event** caption"
  );
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

test("paste/drop upload UX stages through Rust snapshot and sends dialog captions", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    window.__harness.setCommandResponse("stage_uploads", ({ roomId, items }) => {
      const snapshot = window.__harness.currentSnapshot();
      const stagedUploads = items.map((item: any, index: number) => ({
        staged_id: item.stagedId,
        room_id: roomId,
        position: index + 1,
        filename: item.filename,
        mime_type: item.mimeType,
        byte_count: item.byteCount,
        kind: item.kind,
        caption: null,
        compression_choice: item.compressionChoice
      }));
      const nextSnapshot = {
        ...snapshot,
        state: {
          ...snapshot.state,
          timeline: {
            ...snapshot.state.timeline,
            staged_uploads: stagedUploads
          }
        }
      };
      window.__harness.setSnapshot(nextSnapshot);
      return nextSnapshot;
    });
    window.__harness.setCommandResponse(
      "update_staged_upload_caption",
      ({ stagedId, caption }) => {
        const snapshot = window.__harness.currentSnapshot();
        const nextSnapshot = {
          ...snapshot,
          state: {
            ...snapshot.state,
            timeline: {
              ...snapshot.state.timeline,
              staged_uploads: snapshot.state.timeline.staged_uploads.map((item: any) =>
                item.staged_id === stagedId
                  ? {
                      ...item,
                      caption: caption
                        ? { plain_body: caption, formatted_body: null, mentions: { targets: [] } }
                        : null
                    }
                  : item
              )
            }
          }
        };
        window.__harness.setSnapshot(nextSnapshot);
        return nextSnapshot;
      }
    );
    window.__harness.setCommandResponse(
      "update_staged_upload_compression",
      ({ stagedId, compressionChoice }) => {
        const snapshot = window.__harness.currentSnapshot();
        const nextSnapshot = {
          ...snapshot,
          state: {
            ...snapshot.state,
            timeline: {
              ...snapshot.state.timeline,
              staged_uploads: snapshot.state.timeline.staged_uploads.map((item: any) =>
                item.staged_id === stagedId
                  ? { ...item, compression_choice: compressionChoice }
                  : item
              )
            }
          }
        };
        window.__harness.setSnapshot(nextSnapshot);
        return nextSnapshot;
      }
    );
    window.__harness.setCommandResponse("clear_upload_staging", () => {
      const snapshot = window.__harness.currentSnapshot();
      const nextSnapshot = {
        ...snapshot,
        state: {
          ...snapshot.state,
          timeline: {
            ...snapshot.state.timeline,
            staged_uploads: []
          }
        }
      };
      window.__harness.setSnapshot(nextSnapshot);
      return nextSnapshot;
    });
    window.__harness.setCommandResponse("upload_media", () => window.__harness.currentSnapshot());
    window.__harness.clearInvocations();
  });

  await page.evaluate(() => {
    const file = new File(["paste fixture bytes"], "pasted-fixture.txt", {
      type: "text/plain"
    });
    const data = new DataTransfer();
    data.items.add(file);
    const composer = document.querySelector(
      'textarea[aria-label="Message composer"]'
    );
    if (!composer) {
      throw new Error("composer not found");
    }
    composer.dispatchEvent(
      new ClipboardEvent("paste", {
        bubbles: true,
        cancelable: true,
        clipboardData: data
      })
    );
  });

  await expect.poll(() => invocationCount(page, "stage_uploads")).toBe(1);
  await expect.poll(() => invocationCount(page, "upload_media")).toBe(0);
  await expect(page.getByRole("dialog", { name: "Upload attachments" })).toBeVisible();
  await expect(page.getByText("pasted-fixture.txt", { exact: true })).toBeVisible();

  await page.getByRole("textbox", { name: "Caption for pasted-fixture.txt" }).fill("caption from staging");
  await expect.poll(() => invocationCount(page, "update_staged_upload_caption")).toBeGreaterThanOrEqual(1);

  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect.poll(() => invocationCount(page, "upload_media")).toBeGreaterThanOrEqual(1);
  await expect.poll(() => invocationCount(page, "send_text")).toBe(0);
  await expect
    .poll(async () =>
      page.evaluate(() => {
        const args = window.__harness.invocationsOf("upload_media")[0]?.args;
        return args
          ? {
              roomId: args.roomId,
              filename: args.filename,
              mimeType: args.mimeType,
              caption: args.caption,
              byteCount: Array.isArray(args.bytes) ? args.bytes.length : -1
            }
          : null;
      })
    )
    .toEqual({
      roomId: "!harness-room:example.invalid",
      filename: "pasted-fixture.txt",
      mimeType: "text/plain",
      caption: "caption from staging",
      byteCount: "paste fixture bytes".length
    });
  await expect.poll(() => invocationCount(page, "clear_upload_staging")).toBeGreaterThanOrEqual(1);
});

test("room media gallery opens a viewer from Rust-owned gallery projection", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    const mediaGallery = [
      {
        event_id: "$gallery-new:example.invalid",
        room_id: "!harness-room:example.invalid",
        sender: "@harness-user:example.invalid",
        sender_label: "Harness User",
        timestamp_ms: 1_900_000_060_000,
        media: {
          kind: "Image",
          filename: "new-image.png",
          source: {
            mxc_uri: "mxc://example.invalid/new-image",
            encrypted: false,
            encryption_version: null
          },
          mimetype: "image/png",
          size: 4096,
          width: 800,
          height: 600,
          thumbnail: null
        }
      },
      {
        event_id: "$gallery-old:example.invalid",
        room_id: "!harness-room:example.invalid",
        sender: "@harness-user:example.invalid",
        sender_label: "Harness User",
        timestamp_ms: 1_900_000_000_000,
        media: {
          kind: "File",
          filename: "old-file.pdf",
          source: {
            mxc_uri: "mxc://example.invalid/old-file",
            encrypted: true,
            encryption_version: "v2"
          },
          mimetype: "application/pdf",
          size: 8192,
          width: null,
          height: null,
          thumbnail: null
        }
      }
    ];
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        timeline: {
          ...snapshot.state.timeline,
          media_gallery: mediaGallery
        }
      }
    });
    window.__harness.pushStateChanged();
  });

  await page.getByRole("button", { name: "Open media gallery" }).click();
  const gallery = page.getByRole("region", { name: "Room media gallery" });
  await expect(gallery.getByRole("button", { name: "Open new-image.png" })).toBeVisible();
  await gallery.getByRole("button", { name: "Open new-image.png" }).click();

  const viewer = page.getByRole("dialog", { name: "Media viewer" });
  await expect(viewer).toBeVisible();
  await expect(viewer.getByText("new-image.png", { exact: true })).toBeVisible();
  await viewer.getByRole("button", { name: "Next media" }).click();
  await expect(viewer.getByText("old-file.pdf", { exact: true })).toBeVisible();
  await viewer.getByRole("button", { name: "Previous media" }).click();
  await expect(viewer.getByText("new-image.png", { exact: true })).toBeVisible();
  await viewer.getByRole("button", { name: "Close media viewer" }).click();
  await expect(viewer).toHaveCount(0);
});

test("image compression setting and dialog send selected Rust-owned variant metadata", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    window.__harness.setCommandResponse("upload_media", () => window.__harness.currentSnapshot());
    window.__harness.clearInvocations();
  });

  await page.getByRole("button", { name: "User settings" }).click();
  await page.getByRole("group", { name: "Compress images" }).getByRole("button", { name: "Ask" }).click();
  await expect.poll(() => invocationCount(page, "update_settings")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("update_settings")[0]?.args)
    )
    .toEqual({
      patch: {
        media: {
          image_upload_compression: "ask",
          image_upload_compression_policy: {
            threshold_bytes: 1048576,
            threshold_long_edge: 2560,
            target_long_edge: 2048,
            quality_percent: 82
          }
        }
      }
    });

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        settings: {
          ...snapshot.state.settings,
          values: {
            ...snapshot.state.settings.values,
            media: {
              ...snapshot.state.settings.values.media,
              image_upload_compression: "ask",
              image_upload_compression_policy: {
                threshold_bytes: 1,
                threshold_long_edge: 1,
                target_long_edge: 1,
                quality_percent: 82
              }
            }
          }
        }
      }
    });
    window.__harness.pushStateChanged();
    window.__harness.clearInvocations();
  });

  const fixture = await canvasPngBuffer(page, 4, 2);
  await attachFile(page, {
    name: "screen.png",
    mimeType: "image/png",
    buffer: fixture
  });
  await expect(page.getByRole("dialog", { name: "Upload attachments" })).toBeVisible();
  await page.getByRole("button", { name: "Compressed" }).click();
  await expect
    .poll(() => invocationCount(page, "update_staged_upload_compression"))
    .toBeGreaterThanOrEqual(1);
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "Compress image" })).toHaveCount(0);

  await expect.poll(() => invocationCount(page, "upload_media")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => {
        const args = window.__harness.invocationsOf("upload_media")[0]?.args;
        return args
          ? {
              filename: args.filename,
              mimeType: args.mimeType,
              byteCount: Array.isArray(args.bytes) ? args.bytes.length : -1,
              imageDimensions: args.imageDimensions,
              selectedVariant: args.imageCompression?.selected_variant,
              selected: args.imageCompression?.selected,
              metadataStripped: args.imageCompression?.metadata_stripped,
              thumbnailRefreshed: args.imageCompression?.thumbnail_refreshed,
              thumbnail: args.thumbnail
                ? {
                    width: args.thumbnail.width,
                    height: args.thumbnail.height,
                    mimeType: args.thumbnail.mime_type,
                    byteCount: Array.isArray(args.thumbnail.bytes)
                      ? args.thumbnail.bytes.length
                      : -1
                  }
                : null
            }
          : null;
      })
    )
    .toMatchObject({
      filename: "screen.jpg",
      mimeType: "image/jpeg",
      imageDimensions: { width: 1, height: 1 },
      selectedVariant: "Compressed",
      selected: {
        mime_type: "image/jpeg",
        dimensions: { width: 1, height: 1 }
      },
      metadataStripped: true,
      thumbnailRefreshed: true,
      thumbnail: {
        width: 1,
        height: 1,
        mimeType: "image/jpeg"
      }
    });
  await expect
    .poll(async () =>
      page.evaluate(() => {
        const args = window.__harness.invocationsOf("upload_media")[0]?.args;
        return args?.imageCompression?.selected?.byte_count === args?.bytes?.length;
      })
    )
    .toBe(true);

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        settings: {
          ...snapshot.state.settings,
          values: {
            ...snapshot.state.settings.values,
            media: {
              ...snapshot.state.settings.values.media,
              image_upload_compression: "always",
              image_upload_compression_policy: {
                threshold_bytes: 1,
                threshold_long_edge: 1,
                target_long_edge: 1,
                quality_percent: 82
              }
            }
          }
        }
      }
    });
    window.__harness.pushStateChanged();
    window.__harness.clearInvocations();
  });
  await attachFile(page, {
    name: "auto.png",
    mimeType: "image/png",
    buffer: fixture
  });
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "Compress image" })).toHaveCount(0);
  await expect.poll(() => invocationCount(page, "upload_media")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(
        () => window.__harness.invocationsOf("upload_media")[0]?.args?.imageCompression?.selected_variant
      )
    )
    .toBe("Compressed");

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        settings: {
          ...snapshot.state.settings,
          values: {
            ...snapshot.state.settings.values,
            media: {
              ...snapshot.state.settings.values.media,
              image_upload_compression: "ask",
              image_upload_compression_policy: {
                threshold_bytes: 10_000_000,
                threshold_long_edge: 5000,
                target_long_edge: 2048,
                quality_percent: 82
              }
            }
          }
        }
      }
    });
    window.__harness.pushStateChanged();
    window.__harness.clearInvocations();
  });
  await attachFile(page, {
    name: "small.png",
    mimeType: "image/png",
    buffer: fixture
  });
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "Compress image" })).toHaveCount(0);
  await expect.poll(() => invocationCount(page, "upload_media")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => {
        const args = window.__harness.invocationsOf("upload_media")[0]?.args;
        return args
          ? {
              selectedVariant: args.imageCompression?.selected_variant,
              skippedSmallImage: args.imageCompression?.skipped_small_image,
              metadataStripped: args.imageCompression?.metadata_stripped,
              imageDimensions: args.imageDimensions
            }
          : null;
      })
    )
    .toEqual({
      selectedVariant: "Original",
      skippedSmallImage: true,
      metadataStripped: false,
      imageDimensions: { width: 4, height: 2 }
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
                "$seed-event:example.invalid": {
                  readers: [
                    {
                      user_id: "@reader:example.invalid",
                      display_name: "Reader",
                      avatar: null,
                      timestamp_ms: 1_800_000_000_500
                    }
                  ],
                  total_count: 1,
                  overflow_count: 0
                }
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

test("read receipt avatars render from Rust projection with overflow and tooltip", async ({
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
                "$seed-event:example.invalid": {
                  readers: [
                    {
                      user_id: "@alice:example.invalid",
                      display_name: "Alice",
                      avatar: {
                        mxc_uri: "mxc://example.invalid/alice",
                        thumbnail: {
                          kind: "ready",
                          source_url:
                            "data:image/gif;base64,R0lGODlhAQABAAAAACw=",
                          width: 1,
                          height: 1,
                          mime_type: "image/gif"
                        }
                      },
                      timestamp_ms: 1_800_000_000_500
                    },
                    {
                      user_id: "@dana:example.invalid",
                      display_name: "Dana",
                      avatar: null,
                      timestamp_ms: 1_800_000_000_400
                    },
                    {
                      user_id: "@bob:example.invalid",
                      display_name: "Bob",
                      avatar: null,
                      timestamp_ms: 1_800_000_000_300
                    }
                  ],
                  total_count: 4,
                  overflow_count: 1
                }
              },
              fully_read_event_id: null,
              typing_user_ids: []
            }
          },
          presence: {}
        }
      }
    });
    window.__harness.pushStateChanged();
  });

  const row = page.locator('[data-event-id="$seed-event:example.invalid"]');
  const receipts = row.locator(".message-receipts");
  await expect(receipts).toHaveAttribute("aria-label", /Read by 4/);
  await expect(receipts).toHaveAttribute("aria-label", /Alice/);
  await expect(receipts.locator(".receipt-reader-avatar")).toHaveCount(3);
  await expect(receipts.locator(".receipt-reader-avatar img")).toHaveCount(1);
  await expect(receipts.locator(".receipt-reader-avatar").nth(1)).toHaveText("DA");
  await expect(receipts.locator(".receipt-overflow")).toHaveText("+1");

  await receipts.hover();
  await expect(receipts.locator(".receipt-tooltip")).toBeVisible();
  await expect(receipts.locator(".receipt-tooltip")).toContainText("Alice");
  await expect(receipts.locator(".receipt-tooltip")).toContainText("Dana");
  await expect(receipts.locator(".receipt-tooltip")).toContainText("Bob");
  await expect(receipts.locator(".receipt-tooltip")).toContainText("1 more");
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
                  is_hidden: false,
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
                  is_hidden: false,
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
              is_hidden: false,
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
              is_hidden: false,
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
  const threadReplyBody = "Thread composer reply body";
  await threadComposer.fill(threadReplyBody);

  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("set_thread_composer_draft")[0]?.args)
    )
    .toEqual({
      roomId: "!harness-room:example.invalid",
      rootEventId: "$seed-event:example.invalid",
      draft: threadReplyBody
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
        is_composing: false,
        selection: { start: threadReplyBody.length, end: threadReplyBody.length }
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
      body: threadReplyBody
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
  await page.getByRole("button", { name: "Send", exact: true }).click();

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

test("Japanese locale renders shell labels and CJK text without clipping", async ({
  page
}) => {
  await gotoReadyShell(page);

  const longWorkspaceName = "ホーム日本語検証".repeat(10);
  const longRoomName = "幅制約付き日本語ルーム名".repeat(24);
  const rustOrderedRoomNames = ["会議2", "会議10", longRoomName];
  const longSenderName = "長い日本語送信者名".repeat(12);
  const cjkMessageBody = "日本語の長文メッセージと検索確認テキスト".repeat(18);
  const fullWidthSnippet = "ＡＢＣ１２３を含む日本語検索結果";

  await page.evaluate(({ workspaceName, roomNames }) => {
    const snapshot = window.__harness.currentSnapshot();
    const cjkRooms = roomNames.map((displayName, index) => ({
      ...snapshot.state.rooms[0],
      room_id: index === roomNames.length - 1 ? snapshot.state.rooms[0].room_id : `!cjk-order-${index}:example.invalid`,
      display_name: displayName,
      display_label: displayName,
      original_display_label: displayName
    }));
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        locale_profile: {
          lang: "ja",
          dir: "ltr",
          catalog_locale: "ja",
          pseudo_locale: "none",
          platform: "linux",
          modifier_labels: { primary: "Ctrl" }
        },
        cjk_text_policy: {
          ...snapshot.state.cjk_text_policy,
          japanese_catalog: {
            catalog_locale: "ja",
            complete: true,
            missing_message_ids: []
          }
        },
        rooms: cjkRooms
      },
      sidebar: {
        ...snapshot.sidebar,
        account_home: {
          ...snapshot.sidebar.account_home,
          display_name: workspaceName
        },
        space_rooms: cjkRooms.map((room) => ({
          room_id: room.room_id,
          display_name: room.display_name,
          avatar: room.avatar,
          tags: room.tags,
          unread_count: room.unread_count,
          highlight_count: room.highlight_count
        }))
      }
    });
    window.__harness.pushStateChanged();
    window.__harness.clearInvocations();
  }, { workspaceName: longWorkspaceName, roomNames: rustOrderedRoomNames });

  await seedTimelineItems(page, [
    {
      id: { Event: { event_id: "$cjk-gui-linebreak:example.invalid" } },
      sender: longSenderName,
      body: cjkMessageBody,
      timestamp_ms: 1_800_000_003_000,
      in_reply_to_event_id: null,
      thread_root: null,
      thread_summary: null,
      reactions: [],
      can_react: true,
      is_redacted: false,
      is_hidden: false,
      can_redact: false,
      is_edited: false,
      can_edit: false
    }
  ]);

  await page.evaluate(
    ({ snippet, roomName }) => {
      const snapshot = window.__harness.currentSnapshot();
      window.__harness.setCommandResponse("submit_search", ({ query }: { query?: string }) => {
        const next = window.__harness.currentSnapshot();
        return {
          ...next,
          state: {
            ...next.state,
            rooms: next.state.rooms.map((room) =>
              room.room_id === "!harness-room:example.invalid"
                ? {
                    ...room,
                    display_name: "かな先頭",
                    display_label: "かな先頭",
                    original_display_label: "かな先頭"
                  }
                : room
            ),
            search: {
              kind: "results",
              request_id: 32,
              query: String(query ?? "ABC123"),
              scope: "allRooms",
              results: [
                {
                  room_id: "!harness-room:example.invalid",
                  event_id: "$cjk-gui-linebreak:example.invalid",
                  sender: "@cjk-user:example.invalid",
                  timestamp_ms: 1_800_000_003_000,
                  score_millis: 990,
                  snippet,
                  match_field: "messageBody",
                  highlights: [{ start_utf16: 0, end_utf16: 6 }],
                  match_kind: "exact"
                }
              ]
            }
          }
        };
      });
    },
    { snippet: fullWidthSnippet, roomName: longRoomName }
  );

  await expect.poll(() => page.evaluate(() => document.documentElement.lang)).toBe("ja");
  await expect
    .poll(() => page.evaluate(() => document.documentElement.dataset.catalogLocale))
    .toBe("ja");
  await expect(page.getByRole("button", { name: "ルームを作成", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "ユーザー設定", exact: true })).toBeVisible();
  await expect(
    page.locator(".channel-actions").getByRole("button", { name: "スレッド", exact: true })
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "送信", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Create room", exact: true })).toHaveCount(0);
  await expect(
    page.locator(".channel-actions").getByRole("button", { name: "Threads", exact: true })
  ).toHaveCount(0);
  await expect
    .poll(async () =>
      page
        .locator('section[data-room-section="rooms"] .room-name')
        .evaluateAll((elements) => elements.map((element) => element.textContent ?? ""))
    )
    .toEqual(rustOrderedRoomNames);

  const roomNameMetrics = await page
    .locator('section[data-room-section="rooms"] .room-name')
    .nth(2)
    .evaluate((element) => {
      const style = getComputedStyle(element);
      return {
        clientWidth: element.clientWidth,
        hyphens: style.hyphens,
        lineBreak: style.lineBreak,
        overflow: style.overflow,
        textContentLength: element.textContent?.length ?? 0,
        textOverflow: style.textOverflow,
        whiteSpace: style.whiteSpace,
        wordBreak: style.wordBreak
      };
    });
  expect(roomNameMetrics.textContentLength).toBeGreaterThan(roomNameMetrics.clientWidth);
  expect(roomNameMetrics).toMatchObject({
    hyphens: "none",
    lineBreak: "strict",
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
    wordBreak: "normal"
  });

  await page.getByRole("textbox", { name: "検索" }).fill("ABC123");
  await page.getByRole("textbox", { name: "検索" }).press("Enter");
  await expect(page.locator("mark").filter({ hasText: "ＡＢＣ１２３" })).toBeVisible();
  await expect(page.locator(".result-meta").first()).toContainText("かな先頭");

  const senderMetrics = await page
    .locator(".sender")
    .first()
    .evaluate((element) => {
      const style = getComputedStyle(element);
      return {
        clientWidth: element.clientWidth,
        hyphens: style.hyphens,
        lineBreak: style.lineBreak,
        overflow: style.overflow,
        textContentLength: element.textContent?.length ?? 0,
        textOverflow: style.textOverflow,
        whiteSpace: style.whiteSpace,
        wordBreak: style.wordBreak
      };
    });
  expect(senderMetrics).toMatchObject({
    hyphens: "none",
    lineBreak: "strict",
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
    wordBreak: "normal"
  });

  const bodyMetrics = await page
    .locator(".message-body")
    .first()
    .evaluate((element) => {
      const style = getComputedStyle(element);
      return {
        clientWidth: element.clientWidth,
        hyphens: style.hyphens,
        lineBreak: style.lineBreak,
        scrollWidth: element.scrollWidth,
        wordBreak: style.wordBreak
      };
    });
  expect(bodyMetrics.scrollWidth).toBeLessThanOrEqual(bodyMetrics.clientWidth + 1);
  expect(bodyMetrics).toMatchObject({
    hyphens: "none",
    lineBreak: "strict",
    wordBreak: "normal"
  });

  await expect
    .poll(() => page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 2))
    .toBe(true);
});

test("pseudo RTL profile with CJK and combining samples does not overflow shell", async ({
  page
}) => {
  await gotoReadyShell(page);

  const longRoomName = "Cafe\u0301 日本語 العربية Very Long Synthetic Room Label For Pseudo Locale";
  const sampleBody = "Cafe\u0301 日本語 العربية long pseudo locale sample";
  const longReactionKey =
    "very-long-reaction-key-without-breaks-arabic-العربية-0123456789";
  const expectedAddReactionLabel = pseudoLocalize("Add reaction", "bidi");
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
    snapshot.state.rooms[0].display_label = roomName;
    snapshot.state.rooms[0].original_display_label = roomName;
    snapshot.sidebar.space_rooms[0].display_name = roomName;
    snapshot.state.spaces[0].display_name = "日本語 Space العربية";
    snapshot.sidebar.space_rail[0].display_name = "日本語 Space العربية";
    window.__harness.setSnapshot(snapshot);
    window.__harness.pushStateChanged();
  }, longRoomName);

  await expect(page.locator("main.main-pane").getByText(longRoomName)).toBeVisible();
  await expect(page.getByText("Seed message for reply target")).toBeVisible();

  await page.evaluate(async ({ key, body, reactionKey }) => {
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
      is_hidden: false,
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
        },
        {
          key: reactionKey,
          count: 12,
          reacted_by_me: false,
          my_reaction_event_id: null,
          sender_preview: ["@rtl-user:example.invalid", "@second-user:example.invalid"]
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
  }, { key: roomKey, body: sampleBody, reactionKey: longReactionKey });

  await expect.poll(() => page.evaluate(() => document.documentElement.dir)).toBe("rtl");
  await expect(page.locator(".room-name").first()).toHaveAttribute("dir", "auto");
  await expect(page.locator(".sender").first()).toHaveAttribute("dir", "auto");
  await expect(page.locator(".message-body").first()).toHaveAttribute("dir", "auto");
  await expect(page.getByText(sampleBody)).toBeVisible();
  await expect(page.locator(".reaction-pill-key", { hasText: "日本語" })).toBeVisible();
  await expect(page.locator(".reaction-pill-key", { hasText: "日本語" })).toHaveAttribute(
    "dir",
    "auto"
  );
  const longReaction = page.locator(".reaction-pill-key", { hasText: longReactionKey });
  await expect(longReaction).toBeVisible();
  await expect(longReaction).toHaveAttribute("dir", "auto");
  await expect
    .poll(() =>
      longReaction.evaluate((element) => element.scrollWidth > element.clientWidth)
    )
    .toBe(true);
  await page.locator('[data-event-id="$seed-event:example.invalid"]').hover();
  await expect(page.getByRole("button", { name: expectedAddReactionLabel }).first()).toBeVisible();
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
  await page.getByRole("button", { name: "Send", exact: true }).click();

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

test("typography profile applies bundled font and emoji tokens from Rust snapshot", async ({
  page
}) => {
  await gotoReadyShell(page);

  await page.evaluate(() => {
    const base = window.__harness.currentSnapshot();
    const values = {
      ...base.state.settings.values,
      typography: { font: "inter" as const, emoji: "twemojiColr" as const }
    };
    window.__harness.setSnapshot({
      ...base,
      state: {
        ...base.state,
        settings: {
          ...base.state.settings,
          values
        },
        typography_profile: {
          font: "inter",
          emoji: "twemojiColr",
          platform: "linux",
          font_asset: "bundledPreferred",
          emoji_asset: "bundledPreferred"
        }
      }
    });
    window.__harness.pushStateChanged();
  });

  await expect
    .poll(() => page.evaluate(() => document.documentElement.dataset.uiFont))
    .toBe("inter");
  await expect
    .poll(() => page.evaluate(() => document.documentElement.dataset.emojiFont))
    .toBe("twemojiColr");

  const typography = await page.evaluate(async () => {
    await document.fonts.load('14px "Inter"', "English 日本語");
    await document.fonts.load('14px "Twemoji"', "🐶👍");
    await document.fonts.ready;
    const rootStyle = getComputedStyle(document.documentElement);
    const messageBody = document.querySelector(".message-body");
    const reactionKey = document.querySelector(".reaction-pill-key");
    return {
      fontUi: rootStyle.getPropertyValue("--font-ui"),
      fontEmoji: rootStyle.getPropertyValue("--font-emoji"),
      interLoaded: document.fonts.check('14px "Inter"', "English 日本語"),
      twemojiLoaded: document.fonts.check('14px "Twemoji"', "🐶👍"),
      messageFont: messageBody ? getComputedStyle(messageBody).fontFamily : "",
      reactionFont: reactionKey ? getComputedStyle(reactionKey).fontFamily : ""
    };
  });

  expect(typography.fontUi).toContain("Inter");
  expect(typography.fontEmoji).toContain("Twemoji");
  expect(typography.interLoaded).toBe(true);
  expect(typography.twemojiLoaded).toBe(true);
  expect(typography.messageFont).toContain("Inter");
  expect(typography.reactionFont).toContain("Twemoji");
});

test("typography settings dispatch Rust-owned update_settings patches", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.getByRole("button", { name: "User settings" }).click();
  await expect(page.getByText("Typography")).toBeVisible();

  await page.getByRole("button", { name: "Inter" }).click();
  await expect.poll(() => invocationCount(page, "update_settings")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("update_settings")[0]?.args)
    )
    .toEqual({
      patch: {
        typography: { font: "inter", emoji: "system" }
      }
    });
  await expect
    .poll(() => page.evaluate(() => document.documentElement.dataset.uiFont))
    .toBe("inter");

  await page.evaluate(() => window.__harness.clearInvocations());
  await page.getByRole("button", { name: "Twemoji COLR" }).click();
  await expect.poll(() => invocationCount(page, "update_settings")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("update_settings")[0]?.args)
    )
    .toEqual({
      patch: {
        typography: { font: "inter", emoji: "twemojiColr" }
      }
    });
  await expect
    .poll(() => page.evaluate(() => document.documentElement.dataset.emojiFont))
    .toBe("twemojiColr");
});

test("notification settings dispatch Rust-owned update_settings patches", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.getByRole("button", { name: "User settings" }).click();
  await expect(page.getByRole("heading", { name: "Notifications" })).toBeVisible();

  const desktopNotifications = page.getByRole("switch", { name: "Desktop notifications" });
  await expect(desktopNotifications).toHaveAttribute("aria-checked", "true");
  await desktopNotifications.click();

  await expect.poll(() => invocationCount(page, "update_settings")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("update_settings")[0]?.args)
    )
    .toEqual({
      patch: {
        notifications: {
          desktop_notifications: false,
          sound: true,
          badges: true,
          send_read_receipts: true,
          send_typing_notifications: true
        }
      }
    });
  await expect(desktopNotifications).toHaveAttribute("aria-checked", "false");

  await page.evaluate(() => window.__harness.clearInvocations());
  const sound = page.getByRole("switch", { name: "Sound" });
  await expect(sound).toHaveAttribute("aria-checked", "true");
  await sound.click();

  await expect.poll(() => invocationCount(page, "update_settings")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("update_settings")[0]?.args)
    )
    .toEqual({
      patch: {
        notifications: {
          desktop_notifications: false,
          sound: false,
          badges: true,
          send_read_receipts: true,
          send_typing_notifications: true
        }
      }
    });
  await expect(sound).toHaveAttribute("aria-checked", "false");
});

test("rich formatted timeline rows render Rust-owned DTOs and code-wrap setting", async ({
  page
}) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [
    {
      id: { Event: { event_id: "$formatted-rich:example.invalid" } },
      sender: "@harness-user:example.invalid",
      body: "plain fallback should not render",
      timestamp_ms: 1_800_000_000_900,
      in_reply_to_event_id: null,
      formatted: {
        html:
          '<strong>Formatted keyword</strong><blockquote>Quoted body</blockquote><ul><li>List item</li></ul><a href="https://example.invalid/path">safe link</a><pre><code class="language-rust">const veryLongToken = "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz";</code></pre>',
        plain_text:
          'Formatted keywordQuoted bodyList itemsafe linkconst veryLongToken = "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz";',
        code_blocks: [
          {
            language: "rust",
            body:
              'const veryLongToken = "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz";'
          }
        ]
      },
      thread_root: null,
      thread_summary: null,
      reactions: [],
      can_react: false,
      is_redacted: false,
      is_hidden: false,
      can_redact: false,
      is_edited: false,
      can_edit: false
    }
  ]);

  const row = page.locator('[data-event-id="$formatted-rich:example.invalid"]');
  await expect(row.locator("strong")).toHaveText("Formatted keyword");
  await expect(row.locator("blockquote")).toContainText("Quoted body");
  await expect(row.locator("li")).toHaveText("List item");
  await expect(row.locator('a[href="https://example.invalid/path"]')).toHaveText("safe link");
  await expect(row.locator("pre code.language-rust")).toContainText("veryLongToken");
  await expect(row.getByRole("button", { name: "Copy code" })).toBeVisible();
  await expect(row.getByText("plain fallback should not render")).toHaveCount(0);

  const pre = row.locator("pre").first();
  await expect.poll(() => pre.evaluate((element) => getComputedStyle(element).whiteSpace)).toBe(
    "pre-wrap"
  );

  await page.evaluate(() => window.__harness.clearInvocations());
  await page.getByRole("button", { name: "User settings" }).click();
  const wrapToggle = page.getByRole("switch", { name: "Wrap long lines in code blocks" });
  await expect(wrapToggle).toHaveAttribute("aria-checked", "true");
  await wrapToggle.click();

  await expect.poll(() => invocationCount(page, "update_settings")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("update_settings")[0]?.args)
    )
    .toEqual({
      patch: {
        display: {
          code_block_wrap: false,
          hide_redacted: false,
          url_previews_enabled: true
        }
      }
    });
  await expect(wrapToggle).toHaveAttribute("aria-checked", "false");
  await expect.poll(() => pre.evaluate((element) => getComputedStyle(element).whiteSpace)).toBe(
    "pre"
  );
});

test("hide deleted messages setting hides only Rust-marked redacted timeline rows", async ({
  page
}) => {
  await gotoReadyShell(page);
  const redactedEventId = "$redacted-hidden:example.invalid";
  const replyEventId = "$reply-to-hidden-redacted:example.invalid";
  await seedTimelineItems(page, [
    {
      id: { Event: { event_id: redactedEventId } },
      sender: "@harness-user:example.invalid",
      body: null,
      timestamp_ms: 1_800_000_000_950,
      in_reply_to_event_id: null,
      thread_root: null,
      thread_summary: null,
      reactions: [],
      can_react: false,
      is_redacted: true,
      is_hidden: false,
      can_redact: false,
      is_edited: false,
      can_edit: false
    },
    {
      id: { Event: { event_id: replyEventId } },
      sender: "@harness-user:example.invalid",
      body: "Visible reply to a deleted event",
      timestamp_ms: 1_800_000_000_960,
      in_reply_to_event_id: redactedEventId,
      reply_quote: {
        event_id: redactedEventId,
        sender: "@sender:example.invalid",
        sender_label: "Sender",
        body_preview: null,
        state: "redacted"
      },
      thread_root: null,
      thread_summary: null,
      reactions: [],
      can_react: false,
      is_redacted: false,
      is_hidden: false,
      can_redact: false,
      is_edited: false,
      can_edit: false
    }
  ]);

  const redactedRow = page.locator(`[data-event-id="${redactedEventId}"]`);
  const replyRow = page.locator(`[data-event-id="${replyEventId}"]`);
  await expect(redactedRow.getByText(t("timeline.redactedMessage"))).toBeVisible();
  await expect(replyRow.getByText("Visible reply to a deleted event")).toBeVisible();

  await page.evaluate(() => window.__harness.clearInvocations());
  await page.getByRole("button", { name: "User settings" }).click();
  const hideDeleted = page.getByRole("switch", { name: "Hide deleted messages" });
  await expect(hideDeleted).toHaveAttribute("aria-checked", "false");
  await hideDeleted.click();

  await expect.poll(() => invocationCount(page, "update_settings")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("update_settings")[0]?.args)
    )
    .toEqual({
      patch: {
        display: {
          code_block_wrap: true,
          hide_redacted: true,
          url_previews_enabled: true
        }
      }
    });
  await expect(redactedRow.getByText(t("timeline.redactedMessage"))).toBeVisible();

  await page.evaluate(async () => {
    await window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        DisplayPolicyUpdated: {
          hide_redacted: true
        }
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);
  });
  await expect(redactedRow).toHaveCount(0);
  await expect(replyRow.getByText(t("timeline.redactedMessage"))).toBeVisible();

  await hideDeleted.click();
  await page.evaluate(async () => {
    await window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        DisplayPolicyUpdated: {
          hide_redacted: false
        }
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);
  });
  await expect(redactedRow.getByText(t("timeline.redactedMessage"))).toBeVisible();
});

test("profile settings dispatch Rust-owned commands and avatars render from profile state", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    const avatar = {
      mxc_uri: "mxc://example.invalid/avatar-user",
      thumbnail: {
        kind: "ready",
        source_url:
          "data:image/gif;base64,R0lGODlhAQABAAAAACw=",
        width: 1,
        height: 1,
        mime_type: "image/gif"
      }
    };
    const next = {
      ...snapshot,
      state: {
        ...snapshot.state,
        profile: {
          ...snapshot.state.profile,
          users: {
            ...snapshot.state.profile.users,
            "@avatar-user:example.invalid": {
              user_id: "@avatar-user:example.invalid",
              display_name: "Avatar User",
              display_label: "Avatar User",
              original_display_label: "Avatar User",
              mention_search_terms: ["Avatar User", "@avatar-user:example.invalid"],
              avatar
            }
          }
        },
        rooms: snapshot.state.rooms.map((room) =>
          room.room_id === "!harness-room:example.invalid" ? { ...room, avatar } : room
        )
      },
      sidebar: {
        ...snapshot.sidebar,
        space_rooms: snapshot.sidebar.space_rooms.map((room) =>
          room.room_id === "!harness-room:example.invalid" ? { ...room, avatar } : room
        )
      }
    };
    window.__harness.setSnapshot(next);
    window.__harness.pushStateChanged();
  });

  const key = roomTimelineKey("@harness-user:example.invalid", "!harness-room:example.invalid");
  await page.evaluate(({ key }) => {
    window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        ItemsUpdated: {
          key,
          generation: 1,
          batch_id: 22,
          diffs: [
            {
              PushBack: {
                item: {
                  id: { Event: { event_id: "$avatar-event:example.invalid" } },
                  sender: "@avatar-user:example.invalid",
                  body: "Avatar-backed message",
                  timestamp_ms: 1_800_000_000_900,
                  in_reply_to_event_id: null,
                  thread_root: null,
                  thread_summary: null,
                  media: null,
                  is_redacted: false,
                  is_hidden: false,
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
    });
  }, { key });

  const avatarRow = page.locator('[data-event-id="$avatar-event:example.invalid"]');
  await expect(avatarRow.getByText("Avatar-backed message")).toBeVisible();
  await expect(avatarRow.locator(".avatar img")).toHaveAttribute(
    "src",
    /data:image\/gif;base64/
  );
  await expect(page.locator('[data-testid="room-item"] img').first()).toHaveAttribute(
    "src",
    /data:image\/gif;base64/
  );

  await page.evaluate(() => window.__harness.clearInvocations());
  await page.getByRole("button", { name: "User settings" }).click();
  await page.getByLabel("Display name").fill("Alice Profile");
  await page.getByRole("button", { name: "Update", exact: true }).click();
  await expect.poll(() => invocationCount(page, "set_display_name")).toBe(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("set_display_name")[0]?.args)
    )
    .toEqual({ displayName: "Alice Profile" });
  await expect(page.getByLabel("Display name")).toHaveValue("Alice Profile");

  await page.evaluate(() => window.__harness.clearInvocations());
  const fileChooserPromise = page.waitForEvent("filechooser");
  await page.getByRole("button", { name: "Upload" }).click();
  const fileChooser = await fileChooserPromise;
  await fileChooser.setFiles({
    name: "avatar.png",
    mimeType: "image/png",
    buffer: Buffer.from([137, 80, 78, 71])
  });
  await expect.poll(() => invocationCount(page, "set_avatar")).toBe(1);
  await expect
    .poll(async () =>
      page.evaluate(() => {
        const args = window.__harness.invocationsOf("set_avatar")[0]?.args;
        return args
          ? {
              mimeType: args.mimeType,
              byteCount: Array.isArray(args.bytes) ? args.bytes.length : -1
            }
          : null;
      })
    )
    .toEqual({ mimeType: "image/png", byteCount: 4 });
});

test("Security settings render local encryption health and dispatch probe commands", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    window.__harness.setCommandResponse("probe_local_encryption_health", () => {
      const snapshot = window.__harness.currentSnapshot();
      const next = {
        ...snapshot,
        state: {
          ...snapshot.state,
          local_encryption: { kind: "healthy" as const }
        }
      };
      window.__harness.setSnapshot(next);
      return next;
    });

    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        locale_profile: { ...snapshot.state.locale_profile, platform: "linux" },
        typography_profile: { ...snapshot.state.typography_profile, platform: "linux" },
        local_encryption: { kind: "healthy" }
      }
    });
    window.__harness.pushStateChanged();
    window.__harness.clearInvocations();
  });

  await page.getByRole("button", { name: "User settings" }).click();
  await expect(page.getByRole("heading", { name: "Security" })).toBeVisible();
  await expect(page.getByText("Secret Service")).toBeVisible();
  await expect(page.getByText("Protected")).toBeVisible();

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        locale_profile: { ...snapshot.state.locale_profile, platform: "macos" },
        typography_profile: { ...snapshot.state.typography_profile, platform: "macos" },
        local_encryption: { kind: "lockedOrInaccessible" }
      }
    });
    window.__harness.pushStateChanged();
  });
  await expect(page.getByText("macOS Keychain")).toBeVisible();
  await expect(page.getByText("Credential store locked")).toBeVisible();

  await page.getByRole("button", { name: "Check local encryption" }).click();
  await expect.poll(() => invocationCount(page, "probe_local_encryption_health")).toBe(1);
  await expect(page.getByText("Protected")).toBeVisible();

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        locale_profile: { ...snapshot.state.locale_profile, platform: "windows" },
        typography_profile: { ...snapshot.state.typography_profile, platform: "windows" },
        local_encryption: { kind: "resetRequired" }
      }
    });
    window.__harness.pushStateChanged();
  });
  await expect(page.getByText("Windows Credential Manager")).toBeVisible();
  await expect(page.getByText("Reset local data required")).toBeVisible();
  await expect(page.getByRole("button", { name: "Open recovery" })).toBeVisible();

  await page.evaluate(() => {
    window.__harness.setCommandResponse("reset_local_data", () => {
      const snapshot = window.__harness.currentSnapshot();
      const next = {
        ...snapshot,
        state: {
          ...snapshot.state,
          local_encryption: { kind: "unknown" as const }
        }
      };
      window.__harness.setSnapshot(next);
      return next;
    });
  });
  await page.getByRole("button", { name: "Reset local data" }).click();
  await expect.poll(() => invocationCount(page, "reset_local_data")).toBe(1);
  await expect(page.getByText("Not checked")).toBeVisible();
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

test("security settings drive Rust-owned room-key transfer and secure backup state", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    window.__harness.setSnapshot(window.__harness.e2eeTrustSnapshot());
    window.__harness.pushStateChanged();
    window.__harness.clearInvocations();
  });

  await page.getByRole("button", { name: "User settings" }).click();
  await expect(page.getByRole("heading", { name: "Key management" })).toBeVisible();

  const exportForm = page.getByRole("form", { name: "Room key export", exact: true });
  await exportForm.getByLabel("Key export destination").fill("/tmp/kagome-export.txt");
  await exportForm.getByLabel("Room key passphrase").fill("synthetic-export-passphrase");
  await exportForm.getByRole("button", { name: "Export room keys" }).click();
  await expect.poll(() => invocationCount(page, "export_room_keys")).toBe(1);
  await expect(page.getByTestId("room-key-export-state")).toHaveText("Exported");
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("export_room_keys")[0]?.args)
    )
    .toEqual({
      destinationPath: "[REDACTED]",
      passphrase: "[REDACTED]"
    });

  const importForm = page.getByRole("form", { name: "Room key import", exact: true });
  await importForm.getByLabel("Key import source").fill("/tmp/element-compatible-keys.txt");
  await importForm.getByLabel("Room key passphrase").fill("synthetic-import-passphrase");
  await importForm.getByRole("button", { name: "Import room keys" }).click();
  await expect.poll(() => invocationCount(page, "import_room_keys")).toBe(1);
  await expect(page.getByTestId("room-key-import-state")).toHaveText("1 of 1 imported");
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("import_room_keys")[0]?.args)
    )
    .toEqual({
      sourcePath: "[REDACTED]",
      passphrase: "[REDACTED]"
    });

  const secureBackupForm = page.getByRole("form", { name: "Secure backup", exact: true });
  await secureBackupForm
    .getByLabel("Secure backup passphrase", { exact: true })
    .fill("synthetic-secure-backup-passphrase");
  await secureBackupForm.getByLabel("Recovery key destination").fill("/tmp/recovery-key.txt");
  await secureBackupForm.getByRole("button", { name: "Set up secure backup" }).click();
  await expect.poll(() => invocationCount(page, "bootstrap_secure_backup")).toBe(1);
  await expect(page.getByTestId("secure-backup-state")).toHaveText("Recovery key saved");
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("bootstrap_secure_backup")[0]?.args)
    )
    .toEqual({
      passphrase: "[REDACTED]",
      recoveryKeyDestinationPath: "[REDACTED]"
    });

  const passphraseChangeForm = page.getByRole("form", {
    name: "Change secure backup passphrase",
    exact: true
  });
  await passphraseChangeForm
    .getByLabel("Current recovery secret")
    .fill("synthetic-current-recovery-secret");
  await passphraseChangeForm
    .getByLabel("New secure backup passphrase")
    .fill("synthetic-new-secure-backup-passphrase");
  await passphraseChangeForm
    .getByLabel("Recovery key destination")
    .fill("/tmp/changed-recovery-key.txt");
  await passphraseChangeForm
    .getByRole("button", { name: "Update secure backup passphrase" })
    .click();
  await expect.poll(() => invocationCount(page, "change_secure_backup_passphrase")).toBe(1);
  await expect(page.getByTestId("secure-backup-passphrase-change-state")).toHaveText(
    "Changed; recovery key saved"
  );
  await expect
    .poll(async () =>
      page.evaluate(
        () => window.__harness.invocationsOf("change_secure_backup_passphrase")[0]?.args
      )
    )
    .toEqual({
      oldSecret: "[REDACTED]",
      newPassphrase: "[REDACTED]",
      recoveryKeyDestinationPath: "[REDACTED]"
    });

  const serializedPrivateState = await page.evaluate(() =>
    JSON.stringify(window.__harness.currentSnapshot().state.e2ee_trust.key_management)
  );
  expect(serializedPrivateState).not.toContain("synthetic-");
  expect(serializedPrivateState).not.toContain("/tmp/");

  const recordedIpc = await page.evaluate(() => JSON.stringify(window.__harness.invocations()));
  expect(recordedIpc).not.toContain("synthetic-");
  expect(recordedIpc).not.toContain("/tmp/");
  expect(recordedIpc).toContain("[REDACTED]");
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
  const editedBody = "Resolver edited body";
  await editBody.fill(editedBody);
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
        is_composing: false,
        selection: { start: editedBody.length, end: editedBody.length }
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
      body: editedBody
    });
});

test("device session manager renames and signs out from Rust-owned snapshot", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        device_sessions: {
          kind: "loaded",
          devices: [
            {
              device_ordinal: 1,
              display_name: "Current session",
              current: true,
              verified: true,
              inactive: false
            },
            {
              device_ordinal: 2,
              display_name: "Other session",
              current: false,
              verified: false,
              inactive: true
            }
          ]
        }
      }
    });
    window.__harness.pushStateChanged();
    window.__harness.clearInvocations();
  });

  await page.getByRole("button", { name: "User settings", exact: true }).click();
  await page.getByRole("button", { name: "Sessions", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Sessions", exact: true })).toBeVisible();

  const secondSessionRow = page.locator(".session-row").filter({
    hasText: "Other session"
  });
  await expect(secondSessionRow).toBeVisible();
  await secondSessionRow.getByRole("button", { name: "Rename", exact: true }).click();

  const renameField = page.getByRole("textbox", { name: "Device name", exact: true });
  await expect(renameField).toBeVisible();
  await renameField.fill("Renamed other session");
  await page.getByRole("button", { name: "Rename", exact: true }).click();

  await expect.poll(() => invocationCount(page, "rename_device")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("rename_device")[0]?.args))
    .toEqual({
      deviceOrdinal: 2,
      displayName: "Renamed other session"
    });

  const signedOutRow = page.locator(".session-row").filter({ hasText: "Other session" });
  await signedOutRow.getByRole("button", { name: "Sign out", exact: true }).click();

  await expect.poll(() => invocationCount(page, "delete_devices")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("delete_devices")[0]?.args)
    )
    .toEqual({
      deviceOrdinals: [2]
    });
});

test("room list filter and mark unread dispatch Rust-owned commands", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        rooms: [
          ...snapshot.state.rooms,
          {
            room_id: "!room-alpha:example.invalid",
            display_name: "Room Alpha",
            display_label: "Room Alpha",
            original_display_label: "Room Alpha",
            avatar: null,
            is_dm: false,
            dm_user_ids: [],
            tags: { favourite: null, low_priority: null },
            unread_count: 3,
            notification_count: 0,
            highlight_count: 0,
            parent_space_ids: []
          },
          {
            room_id: "!room-beta:example.invalid",
            display_name: "Room Beta",
            display_label: "Room Beta",
            original_display_label: "Room Beta",
            avatar: null,
            is_dm: false,
            dm_user_ids: [],
            tags: { favourite: null, low_priority: null },
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            parent_space_ids: []
          },
          {
            room_id: "!dm-alpha:example.invalid",
            display_name: "DM Alpha",
            display_label: "DM Alpha",
            original_display_label: "DM Alpha",
            avatar: null,
            is_dm: true,
            dm_user_ids: ["@dm-alpha:example.invalid"],
            tags: { favourite: null, low_priority: null },
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            parent_space_ids: []
          }
        ],
        room_list: {
          active_filter: { kind: "rooms" },
          sort: { kind: "activity" },
          items: [
            { room_id: "!room-alpha:example.invalid", kind: "room" },
            { room_id: "!room-beta:example.invalid", kind: "room" },
            { room_id: "!dm-alpha:example.invalid", kind: "room" }
          ]
        }
      }
    });
    window.__harness.pushStateChanged();
    window.__harness.clearInvocations();
  });

  const alphaRow = page.getByTestId("room-item").filter({ hasText: "Room Alpha" }).first();
  await expect(alphaRow).toBeVisible();

  await page.getByRole("tab", { name: t("roomList.filterUnread") }).click();
  await expect.poll(() => invocationCount(page, "select_room_list_filter")).toBe(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("select_room_list_filter")[0]?.args)
    )
    .toEqual({ filter: { kind: "unread" } });

  await alphaRow.click({ button: "right" });
  await page.getByRole("menuitem", { name: t("room.markAsUnread") }).click();
  await expect.poll(() => invocationCount(page, "mark_room_as_unread")).toBe(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("mark_room_as_unread")[0]?.args)
    )
    .toEqual({ roomId: "!room-alpha:example.invalid", unread: true });
});


test("per-room notification mode dispatches set_room_notification_mode from room info", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.getByRole("button", { name: "Room info" }).click();

  const notificationSelect = page.getByRole("combobox", { name: "Notifications" });
  await expect(notificationSelect).toBeVisible();
  await expect(notificationSelect).toHaveValue("all");
  await notificationSelect.selectOption("mute");

  await expect.poll(() => invocationCount(page, "set_room_notification_mode")).toBe(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("set_room_notification_mode")[0]?.args)
    )
    .toEqual({
      roomId: "!harness-room:example.invalid",
      mode: { kind: "mute" }
    });
  await expect(notificationSelect).toHaveValue("mute");
});

test("privacy toggles dispatch Rust-owned update_settings patches for read receipts and typing", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.getByRole("button", { name: "User settings" }).click();
  await expect(page.getByRole("heading", { name: "Notifications" })).toBeVisible();

  const readReceipts = page.getByRole("switch", { name: "Send read receipts" });
  await expect(readReceipts).toHaveAttribute("aria-checked", "true");
  await readReceipts.click();

  await expect.poll(() => invocationCount(page, "update_settings")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("update_settings")[0]?.args)
    )
    .toEqual({
      patch: {
        notifications: {
          desktop_notifications: true,
          sound: true,
          badges: true,
          send_read_receipts: false,
          send_typing_notifications: true
        }
      }
    });
  await expect(readReceipts).toHaveAttribute("aria-checked", "false");

  await page.evaluate(() => window.__harness.clearInvocations());
  const typing = page.getByRole("switch", { name: "Send typing notifications" });
  await expect(typing).toHaveAttribute("aria-checked", "true");
  await typing.click();

  await expect.poll(() => invocationCount(page, "update_settings")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("update_settings")[0]?.args)
    )
    .toEqual({
      patch: {
        notifications: {
          desktop_notifications: true,
          sound: true,
          badges: true,
          send_read_receipts: false,
          send_typing_notifications: false
        }
      }
    });
  await expect(typing).toHaveAttribute("aria-checked", "false");
});


test("message context menu ignores and unignores a user via typed commands", async ({ page }) => {
  await gotoReadyShell(page);
  const targetUserId = "@other-user:example.invalid";
  const targetEventId = "$ignore-target:example.invalid";

  await seedTimelineItems(
    page,
    [
      {
        id: { Event: { event_id: targetEventId } },
        sender: targetUserId,
        body: "Ignore context menu target",
        timestamp_ms: 1_800_000_004_000,
        in_reply_to_event_id: null,
        thread_root: null,
        thread_summary: null,
        reactions: [],
        can_react: true,
        is_redacted: false,
        is_hidden: false,
        can_redact: false,
        is_edited: false,
        can_edit: false
      }
    ],
    71
  );

  await page.evaluate((userId) => {
    window.__harness.setCommandResponse("ignore_user", ({ userId: incomingUserId }) => {
      const snapshot = window.__harness.currentSnapshot();
      const next = {
        ...snapshot,
        state: {
          ...snapshot.state,
          profile: {
            ...snapshot.state.profile,
            ignored_user_ids: [...snapshot.state.profile.ignored_user_ids, String(incomingUserId)]
          }
        }
      };
      window.__harness.setSnapshot(next);
      return next;
    });
    window.__harness.setCommandResponse("unignore_user", ({ userId: incomingUserId }) => {
      const snapshot = window.__harness.currentSnapshot();
      const next = {
        ...snapshot,
        state: {
          ...snapshot.state,
          profile: {
            ...snapshot.state.profile,
            ignored_user_ids: snapshot.state.profile.ignored_user_ids.filter(
              (id) => id !== String(incomingUserId)
            )
          }
        }
      };
      window.__harness.setSnapshot(next);
      return next;
    });
    window.__harness.clearInvocations();
  }, targetUserId);

  const row = page.locator(".message").filter({ hasText: "Ignore context menu target" });
  await row.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Ignore" }).click();

  await expect.poll(() => invocationCount(page, "ignore_user")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("ignore_user")[0]?.args))
    .toEqual({ userId: targetUserId });
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.currentSnapshot().state.profile.ignored_user_ids)
    )
    .toContain(targetUserId);

  await row.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Unignore" }).click();

  await expect.poll(() => invocationCount(page, "unignore_user")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("unignore_user")[0]?.args))
    .toEqual({ userId: targetUserId });
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.currentSnapshot().state.profile.ignored_user_ids)
    )
    .not.toContain(targetUserId);
});

test("message context menu reports content with a reason", async ({ page }) => {
  await gotoReadyShell(page);
  const targetUserId = "@reported-user:example.invalid";
  const targetEventId = "$report-content-target:example.invalid";

  await seedTimelineItems(
    page,
    [
      {
        id: { Event: { event_id: targetEventId } },
        sender: targetUserId,
        body: "Report content target",
        timestamp_ms: 1_800_000_005_000,
        in_reply_to_event_id: null,
        thread_root: null,
        thread_summary: null,
        reactions: [],
        can_react: true,
        is_redacted: false,
        is_hidden: false,
        can_redact: false,
        is_edited: false,
        can_edit: false
      }
    ],
    72
  );

  await page.evaluate(() => {
    window.__harness.setCommandResponse("report_content", () => window.__harness.currentSnapshot());
    window.__harness.clearInvocations();
  });

  const row = page.locator(".message").filter({ hasText: "Report content target" });
  await row.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Report content" }).click();

  const reasonInput = page.getByRole("textbox", { name: "Reason" });
  await expect(reasonInput).toBeVisible();
  await reasonInput.fill("Spam content");
  await page.getByRole("button", { name: "Report", exact: true }).click();

  await expect.poll(() => invocationCount(page, "report_content")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("report_content")[0]?.args))
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      eventId: targetEventId,
      reason: "Spam content"
    });
});

test("room context menu reports room with a reason", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    window.__harness.setCommandResponse("report_room", () => window.__harness.currentSnapshot());
    window.__harness.clearInvocations();
  });

  await page.getByRole("button", { name: "Harness Room" }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Report room" }).click();

  const reasonInput = page.getByRole("textbox", { name: "Reason" });
  await expect(reasonInput).toBeVisible();
  await reasonInput.fill("Toxic room");
  await page.getByRole("button", { name: "Report", exact: true }).click();

  await expect.poll(() => invocationCount(page, "report_room")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("report_room")[0]?.args))
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      reason: "Toxic room"
    });
});

test("room member panel ignores, unignores, and reports a user", async ({ page }) => {
  await gotoReadyShell(page);
  const targetUserId = "@target-member:example.invalid";

  await page.evaluate(
    ({ roomId, targetUserId: userId }) => {
      window.__harness.setCommandResponse("load_room_settings", ({ roomId: incomingRoomId }) => {
        const current = window.__harness.currentSnapshot();
        const next = {
          ...current,
          state: {
            ...current.state,
            room_management: {
              selected_room_id: String(incomingRoomId),
              settings: {
                room_id: String(incomingRoomId),
                name: "Harness Room",
                topic: null,
                avatar_url: null,
                join_rule: "invite",
                history_visibility: "shared",
                permissions: {
                  can_edit_settings: true,
                  can_edit_roles: true,
                  can_kick: true,
                  can_ban: false,
                  can_unban: false
                },
                members: [
                  {
                    user_id: userId,
                    display_name: "Target Member",
                    display_label: "Target Member",
                    original_display_label: "Target Member",
                    avatar_url: null,
                    power_level: 0,
                    role: "user"
                  }
                ]
              },
              operation: { kind: "idle" }
            }
          }
        };
        window.__harness.setSnapshot(next);
        return next;
      });
      window.__harness.setCommandResponse("ignore_user", ({ userId: incomingUserId }) => {
        const current = window.__harness.currentSnapshot();
        const next = {
          ...current,
          state: {
            ...current.state,
            profile: {
              ...current.state.profile,
              ignored_user_ids: [...current.state.profile.ignored_user_ids, String(incomingUserId)]
            }
          }
        };
        window.__harness.setSnapshot(next);
        return next;
      });
      window.__harness.setCommandResponse("unignore_user", ({ userId: incomingUserId }) => {
        const current = window.__harness.currentSnapshot();
        const next = {
          ...current,
          state: {
            ...current.state,
            profile: {
              ...current.state.profile,
              ignored_user_ids: current.state.profile.ignored_user_ids.filter(
                (id) => id !== String(incomingUserId)
              )
            }
          }
        };
        window.__harness.setSnapshot(next);
        return next;
      });
      window.__harness.setCommandResponse("report_user", () => window.__harness.currentSnapshot());
      window.__harness.clearInvocations();
    },
    { roomId: HARNESS_ROOM_ID, targetUserId }
  );

  await page.getByRole("button", { name: "Room info" }).click();
  const targetMemberRow = page.locator(".room-member-row").filter({ hasText: "Target Member" });
  await expect(targetMemberRow).toBeVisible();

  await targetMemberRow.getByRole("button", { name: "Ignore" }).click();
  await expect.poll(() => invocationCount(page, "ignore_user")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("ignore_user")[0]?.args))
    .toEqual({ userId: targetUserId });
  await expect(targetMemberRow.getByRole("button", { name: "Unignore" })).toBeVisible();

  await targetMemberRow.getByRole("button", { name: "Unignore" }).click();
  await expect.poll(() => invocationCount(page, "unignore_user")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("unignore_user")[0]?.args))
    .toEqual({ userId: targetUserId });
  await expect(targetMemberRow.getByRole("button", { name: "Ignore" })).toBeVisible();

  await targetMemberRow.getByRole("button", { name: "Report user" }).click();
  const reasonInput = page.getByRole("textbox", { name: "Reason" });
  await expect(reasonInput).toBeVisible();
  await reasonInput.fill("Harassment");
  await page.getByRole("button", { name: "Report", exact: true }).click();

  await expect.poll(() => invocationCount(page, "report_user")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("report_user")[0]?.args))
    .toEqual({
      userId: targetUserId,
      reason: "Harassment"
    });
});

test("room info Files entry opens the file browser with room scope", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setCommandResponse("open_files_view", () => ({
      ...snapshot,
      state: {
        ...snapshot.state,
        files_view: {
          kind: "open",
          request_id: 1,
          scope: { kind: "room", room_id: "!harness-room:example.invalid" },
          filter: { kinds: ["image", "video", "audio", "file"], filename_query: null },
          sort: "newestFirst",
          items: [
            {
              room_id: "!harness-room:example.invalid",
              event_id: "$file-event:example.invalid",
              sender: "@file-sender:example.invalid",
              timestamp_ms: 1_800_000_000_000,
              kind: "file",
              filename: "quarterly_report.pdf",
              mimetype: "application/pdf",
              size: 12_345,
              source_mxc: "mxc://example.invalid/source",
              thumbnail_mxc: null,
              thread_root: null,
              encrypted: false,
              encryption_version: null,
              width: null,
              height: null,
              is_edited: false
            }
          ],
          selected_event_id: null
        }
      }
    }));
  });

  await page.getByRole("button", { name: t("room.roomInfo") }).click();
  await page.getByRole("button", { name: t("room.files") }).click();

  await expect.poll(() => invocationCount(page, "open_files_view")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("open_files_view")[0]?.args))
    .toEqual({
      scope: { kind: "room", room_id: "!harness-room:example.invalid" },
      filter: { kinds: ["image", "video", "audio", "file"], filename_query: null },
      sort: "newestFirst"
    });

  await expect(page.getByText(t("files.title"), { exact: true })).toBeVisible();
  await expect(page.getByText("quarterly_report.pdf")).toBeVisible();
});

test("timeline header Threads button opens the threads list and row opens a thread", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  await page
    .locator(".channel-actions")
    .getByRole("button", { name: t("threads.title") })
    .click();

  await expect.poll(() => invocationCount(page, "open_threads_list")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("open_threads_list")[0]?.args)
    )
    .toEqual({ roomId: "!harness-room:example.invalid" });

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        threads_list: {
          kind: "open",
          room_id: "!harness-room:example.invalid",
          request_id: 1,
          items: [
            {
              root_event_id: "$thread-root:example.invalid",
              root_sender: "@thread-root-sender:example.invalid",
              root_sender_label: null,
              root_body_preview: "Thread root preview",
              root_timestamp_ms: 1_800_000_000_000,
              latest_event_id: "$thread-latest:example.invalid",
              latest_sender: "@thread-latest-sender:example.invalid",
              latest_sender_label: null,
              latest_body_preview: "Latest reply preview",
              latest_timestamp_ms: 1_800_000_000_100,
              reply_count: 3
            }
          ],
          is_paginating: false,
          end_reached: true
        }
      }
    });
    window.__harness.pushStateChanged();
  });

  const contextPanel = page.locator('aside[aria-label="Context panel"]');
  await expect(contextPanel.getByText(t("threads.title"), { exact: true })).toBeVisible();
  await expect(contextPanel.getByText("Thread root preview")).toBeVisible();

  await page.getByRole("button", { name: /3 replies/ }).click({ force: true });

  await expect.poll(() => invocationCount(page, "open_thread")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("open_thread")[0]?.args))
    .toEqual({
      roomId: "!harness-room:example.invalid",
      rootEventId: "$thread-root:example.invalid"
    });
});

test("URL previews global toggle invokes update_settings", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    window.__harness.setCommandResponse("update_settings", () => window.__harness.currentSnapshot());
    window.__harness.clearInvocations();
  });

  await page.getByRole("button", { name: t("workspace.userSettings") }).click();

  const toggle = page.getByRole("switch", { name: t("settings.urlPreviews") });
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await toggle.click();

  await expect.poll(() => invocationCount(page, "update_settings")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("update_settings")[0]?.args)
    )
    .toEqual({
      patch: {
        display: {
          code_block_wrap: true,
          hide_redacted: false,
          url_previews_enabled: false
        }
      }
    });
});

test("room URL preview toggle invokes the per-room command instead of update_settings", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.getByRole("button", { name: t("room.roomInfo") }).click();
  const toggle = page.getByRole("switch", { name: t("settings.urlPreviewsEnabledForRoom") });
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await toggle.click();

  await expect
    .poll(() => invocationCount(page, "set_room_url_preview_override"))
    .toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("set_room_url_preview_override")[0]?.args)
    )
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      enabled: false
    });
  await expect.poll(() => invocationCount(page, "update_settings")).toBe(0);
  await expect(toggle).toHaveAttribute("aria-checked", "false");
});

test("link preview card renders from Rust-owned DTO and hides on close", async ({ page }) => {
  await gotoReadyShell(page);
  const eventId = "$link-preview:example.invalid";
  const linkPreviewItem = {
    id: { Event: { event_id: eventId } },
    sender: "@harness-user:example.invalid",
    body: "See https://example.invalid/page",
    timestamp_ms: 1_800_000_001_000,
    in_reply_to_event_id: null,
    thread_root: null,
    thread_summary: null,
    reactions: [],
    can_react: true,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false,
    link_previews: [
      {
        url: "https://example.invalid/page",
        title: "Example Preview",
        description: "A synthetic preview for testing.",
        image: null,
        state: "ready"
      }
    ]
  };
  await seedTimelineItems(page, [linkPreviewItem]);

  const row = page.locator(`[data-event-id="${eventId}"]`);
  await expect(row.locator(".link-preview-card")).toBeVisible();
  await expect(row.getByText("Example Preview")).toBeVisible();

  await page.evaluate(() => {
    window.__harness.setCommandResponse("hide_link_preview", () =>
      window.__harness.currentSnapshot()
    );
    window.__harness.clearInvocations();
  });
  await row.getByRole("button", { name: t("timeline.linkPreviewHide") }).click();

  await expect.poll(() => invocationCount(page, "hide_link_preview")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("hide_link_preview")[0]?.args)
    )
    .toEqual({ roomId: "!harness-room:example.invalid", eventId });

  // Simulate Rust removing the preview cards after the viewer-local hide command.
  await pushTimelineDiffs(
    page,
    [{ Set: { index: 0, item: { ...linkPreviewItem, link_previews: [] } } }],
    2,
    3
  );
  await expect(row.locator(".link-preview-card")).toHaveCount(0);
});

test("encrypted room suppresses link previews and shows privacy notice", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    snapshot.state.rooms[0] = {
      ...snapshot.state.rooms[0],
      is_encrypted: true
    };
    window.__harness.setSnapshot(snapshot);
    window.__harness.setCommandResponse("load_room_settings", () =>
      window.__harness.currentSnapshot()
    );
  });

  await page.getByRole("button", { name: t("room.roomInfo") }).click();
  await expect(page.getByText(t("settings.urlPreviewsEncryptedNotice"))).toBeVisible();
});
