import { expect, test } from "@playwright/test";

import { t } from "../src/i18n/messages";
import {
  HARNESS_ACCOUNT_KEY,
  HARNESS_ROOM_ID,
  HARNESS_ROOM_KEY,
  gotoReadyShell,
  invocationCount,
  pushTimelineDiffs,
  seedTimelineItems
} from "./support/basicOperations";

const LIVE_SIGNALS_EVENT_ID = "$live-signals-latest:example.invalid";

test("timeline action tooltip remains available", async ({ page }) => {
  await gotoReadyShell(page);
  const replyButton = page.getByRole("button", { name: "Reply to message" }).first();

  await replyButton.hover();

  await expect(page.getByRole("tooltip", { name: "Reply to message" })).toBeVisible();
});

test("room people labels never promote raw Matrix ids in timeline metadata", async ({ page }) => {
  await gotoReadyShell(page);
  await pushTimelineDiffs(
    page,
    [
      {
        Set: {
          index: 0,
          item: {
            id: { Event: { event_id: "$people-labels:example.invalid" } },
            sender: "@raw-sender:example.invalid",
            sender_label: "Sender Alias",
            sender_avatar: null,
            body: "People label probe",
            timestamp_ms: 1_800_000_000_000,
            in_reply_to_event_id: "$quoted:example.invalid",
            reply_quote: {
              event_id: "$quoted:example.invalid",
              sender: "@raw-quoted:example.invalid",
              sender_label: null,
              body_preview: "Quoted body",
              state: "ready"
            },
            thread_root: null,
            thread_summary: {
              reply_count: 1,
              latest_event_id: "$latest:example.invalid",
              latest_sender: "@raw-latest:example.invalid",
              latest_sender_label: "Latest Alias",
              latest_body_preview: "Latest reply",
              latest_timestamp_ms: 1_800_000_000_100
            },
            can_react: true,
            is_redacted: false,
            is_hidden: false,
            can_redact: false,
            is_edited: false,
            can_edit: false,
            reactions: [
              {
                key: "✅",
                count: 2,
                reacted_by_me: false,
                my_reaction_event_id: null,
                sender_preview: [
                  {
                    user_id: "@raw-reactor:example.invalid",
                    display_label: "Known Reactor"
                  },
                  {
                    user_id: "@missing-reactor:example.invalid",
                    display_label: null
                  }
                ]
              }
            ]
          }
        }
      }
    ],
    1
  );

  const row = page.locator(
    'article[data-content-event-id="$people-labels:example.invalid"]'
  );
  await expect(row.getByText("Sender Alias", { exact: true })).toBeVisible();
  await expect(row.getByText("Unknown user", { exact: true })).toBeVisible();
  await expect(row.getByText(/Latest Alias: Latest reply/)).toBeVisible();
  await expect(
    row.getByText("Known Reactor and Unknown user reacted with ✅", { exact: true })
  ).toBeAttached();
  await expect(row).not.toContainText("@raw-");
  await expect(row).not.toContainText("@missing-reactor:example.invalid");
});

test("timeline sender avatars render after headless account thumbnail events", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  await seedTimelineItems(page, [
    {
      id: { Event: { event_id: "$headless-avatar-a:example.invalid" } },
      sender: "@avatar-a:example.invalid",
      sender_label: "Avatar Alpha",
      body: "Avatar headless row A",
      timestamp_ms: 1_800_000_010_000,
      in_reply_to_event_id: null,
      thread_root: null,
      thread_summary: null,
      sender_avatar: {
        mxc_uri: "mxc://example.invalid/headless-avatar-a",
        thumbnail: { kind: "notRequested" }
      },
      media: null,
      is_redacted: false,
      is_hidden: false,
      can_redact: false,
      is_edited: false,
      can_edit: false,
      reactions: []
    },
    {
      id: { Event: { event_id: "$headless-avatar-b:example.invalid" } },
      sender: "@avatar-b:example.invalid",
      sender_label: "Avatar Beta",
      body: "Avatar headless row B",
      timestamp_ms: 1_800_000_010_500,
      in_reply_to_event_id: null,
      thread_root: null,
      thread_summary: null,
      sender_avatar: {
        mxc_uri: "mxc://example.invalid/headless-avatar-b",
        thumbnail: { kind: "notRequested" }
      },
      media: null,
      is_redacted: false,
      is_hidden: false,
      can_redact: false,
      is_edited: false,
      can_edit: false,
      reactions: []
    }
  ]);

  const firstRow = page.locator('[data-event-id="$headless-avatar-a:example.invalid"]');
  const secondRow = page.locator('[data-event-id="$headless-avatar-b:example.invalid"]');

  await expect
    .poll(async () =>
      page.evaluate(() =>
        window.__harness
          .invocationsOf("download_avatar_thumbnail")
          .map((invocation) => invocation.args.mxcUri)
      )
    )
    .toEqual(
      expect.arrayContaining([
        "mxc://example.invalid/headless-avatar-a",
        "mxc://example.invalid/headless-avatar-b"
      ])
    );

  // The invocation recorder observes the mock call synchronously. Wait for the
  // corresponding timeline commit before injecting the account-owned
  // completion event so the relevance fence is deterministic under full-suite
  // load.
  await expect(firstRow.getByText("Avatar headless row A")).toBeVisible();
  await expect(secondRow.getByText("Avatar headless row B")).toBeVisible();

  // The harness promise acknowledges dispatch, not React commit. Settle each
  // completion in the DOM before publishing the next non-replaying event.
  for (const { mxcUri, sequence, row } of [
    {
      mxcUri: "mxc://example.invalid/headless-avatar-a",
      sequence: 31,
      row: firstRow
    },
    {
      mxcUri: "mxc://example.invalid/headless-avatar-b",
      sequence: 32,
      row: secondRow
    }
  ]) {
    await page.evaluate(
      async ({ completionMxcUri, completionSequence }) => {
        await window.__harness.pushCoreEvent({
          kind: "Account",
          event: {
            AvatarThumbnailDownloaded: {
              request_id: { connection_id: 1, sequence: completionSequence },
              mxc_uri: completionMxcUri,
              thumbnail: {
                kind: "ready",
                source_ref: "data:image/gif;base64,R0lGODlhAQABAAAAACw=",
                width: 1,
                height: 1,
                mime_type: "image/gif"
              }
            }
          }
          // eslint-disable-next-line @typescript-eslint/no-explicit-any
        } as any);
      },
      { completionMxcUri: mxcUri, completionSequence: sequence }
    );
    await expect(row.locator(".avatar img")).toHaveAttribute("src", /data:image\/gif;base64/);
  }
});

test("sent own messages show a visible timestamp and a sent check mark", async ({ page }) => {
  await gotoReadyShell(page);
  const timestampMs = 1_800_000_002_000;
  await seedTimelineItems(page, [
    {
      id: { Event: { event_id: "$sent-state-event:example.invalid" } },
      sender: HARNESS_ACCOUNT_KEY,
      sender_label: "Harness Sender",
      body: "Sent and delivered",
      timestamp_ms: timestampMs,
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
      send_state: { kind: "sent" }
    }
  ]);

  const row = page.locator('[data-event-id="$sent-state-event:example.invalid"]');
  await expect(row).toHaveAttribute("data-send-state", "sent");
  const timestamp = row.locator(".message-timestamp");
  await expect(timestamp).toHaveAttribute("datetime", new Date(timestampMs).toISOString());
  await expect(timestamp).not.toBeEmpty();
  const sentMark = row.locator('.message-send-state[data-send-state="sent"]');
  await expect(sentMark).toBeVisible();
  await expect(sentMark).toHaveAttribute("aria-label", t("timeline.sent"));
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
  await expect(page.getByRole("button", { name: "slightly smiling face" })).toBeVisible();
  await page.getByRole("button", { name: "slightly smiling face" }).click();

  await expect.poll(() => invocationCount(page, "send_reaction")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("send_reaction")[0]?.args)
    )
    .toEqual({
      roomId: "!harness-room:example.invalid",
      eventId: "$seed-event:example.invalid",
      reactionKey: "🙂"
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
        sender_label: "Quoted User",
        body_preview: "Quoted source from Rust state",
        formatted: {
          html: '<ul><li>First item</li><li>Second item</li></ul><p><a href="https://example.invalid/quote">quoted link</a></p><pre><code class="language-rust">fn main() {}</code></pre>',
          plain_text: "First itemSecond itemquoted linkfn main() {}",
          code_blocks: [{ language: "rust", body: "fn main() {}" }]
        },
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
  await expect(row.getByText("Quoted User", { exact: true })).toBeVisible();
  await expect(row).not.toContainText("@quoted-user:example.invalid");
  await expect(row.locator(".reply-quote ul li")).toHaveCount(2);
  await expect(row.locator('.reply-quote a[href="https://example.invalid/quote"]')).toBeVisible();
  await expect(row.locator(".reply-quote .message-code-block-pre code")).toHaveText(
    "fn main() {}"
  );
  await expect(row).not.toContainText("Quoted source from Rust state");
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
        domain: {
          ...snapshot.state.domain,
          room_interactions: {
            ...snapshot.state.domain.room_interactions,
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
      }
    });
    window.__harness.pushStateUpdate();
  }, HARNESS_ROOM_ID);

  await expect(page.getByRole("button", { name: "Pinned · 1", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Pinned · 1", exact: true }).click();
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
        domain: {
          ...snapshot.state.domain,
          room_interactions: {
            ...snapshot.state.domain.room_interactions,
            [roomId]: {
              pinned_events: [],
              pin_operation: { kind: "idle" }
            }
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
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
          domain: {
            ...snapshot.state.domain,
            room_interactions: {
              ...snapshot.state.domain.room_interactions,
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
        }
      };
    });
    window.__harness.setCommandResponse("unpin_event", () => {
      const snapshot = window.__harness.currentSnapshot();
      return {
        ...snapshot,
        state: {
          ...snapshot.state,
          domain: {
            ...snapshot.state.domain,
            room_interactions: {
              ...snapshot.state.domain.room_interactions,
              [roomId]: {
                pinned_events: [],
                pin_operation: { kind: "idle" }
              }
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
  await page.getByRole("button", { name: "Pinned · 1", exact: true }).click();
  await expect(pinnedRegion.getByText("Pinned from Tauri response", { exact: true })).toBeVisible();

  await pinnedRegion.getByRole("button", { name: "Unpin message" }).click({ force: true });
  await expect.poll(() => invocationCount(page, "unpin_event")).toBeGreaterThanOrEqual(1);
  await expect(pinnedRegion).toHaveCount(0);
  await expect(page.getByText("No pinned messages", { exact: true })).toBeVisible();
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
        can_reply: true,
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
        can_reply: true,
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
  await expect(sourceDialog.locator(".message-source-json")).toContainText(
    '"body": "Source body projected by Rust"'
  );
  await expect(sourceDialog.locator(".message-source-json")).toContainText('"edited": true');
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
        ui: {
          ...snapshot.state.ui,
          timeline: {
            ...snapshot.state.ui.timeline,
            media_gallery: mediaGallery
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
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

test("live signals render from Rust state and dispatch only viewport/typing commands", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  await seedTimelineItems(
    page,
    [
      {
        id: { Event: { event_id: LIVE_SIGNALS_EVENT_ID } },
        sender: HARNESS_ACCOUNT_KEY,
        body: "Fresh live signal message",
        timestamp_ms: 1_800_000_001_000,
        in_reply_to_event_id: null,
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
    ],
    2
  );

  await page.evaluate((eventId) => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          live_signals: {
            rooms: {
              "!harness-room:example.invalid": {
                receipts_by_event: {
                  [eventId]: {
                    readers: [
                      {
                        user_id: "@reader:example.invalid",
                        display_name: "Reader",
                        avatar: null,
                        timestamp_ms: 1_800_000_001_500
                      }
                    ],
                    total_count: 1,
                    overflow_count: 0
                  }
                },
                fully_read_event_id: eventId,
                typing_user_ids: ["@typing-user:example.invalid"],
                typing_users: [
                  {
                    user_id: "@typing-user:example.invalid",
                    display_label: "Typing User"
                  }
                ]
              }
            },
            presence: {
              "@harness-user:example.invalid": "online"
            }
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
  }, LIVE_SIGNALS_EVENT_ID);

  const row = page.locator(`[data-event-id="${LIVE_SIGNALS_EVENT_ID}"]`);
  await expect(row.locator(".presence-dot[data-presence='online']")).toBeVisible();
  await expect(row.locator(".message-receipts")).toHaveAttribute("aria-label", /Read by 1/);
  await expect(page.getByText("Read up to here", { exact: true })).toBeVisible();
  await expect(page.getByText("Typing User is typing", { exact: true })).toBeVisible();
  await expect(page.locator(".typing-indicator")).not.toContainText(
    "@typing-user:example.invalid"
  );
  await expect
    .poll(async () =>
      page.evaluate(
        () => window.__harness.invocationsOf("observe_timeline_viewport").at(-1)?.args
      )
    )
    .toMatchObject({
      roomId: HARNESS_ROOM_ID,
      lastVisibleEventId: LIVE_SIGNALS_EVENT_ID,
      atBottom: true
    });
  expect(await invocationCount(page, "send_read_receipt")).toBe(0);
  expect(await invocationCount(page, "set_fully_read")).toBe(0);

  await page.evaluate(() => window.__harness.clearInvocations());
  await page.getByRole("textbox", { name: "Message composer" }).fill("Typing signal");

  await expect.poll(() => invocationCount(page, "set_typing")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("set_typing")[0]?.args))
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      isTyping: true
    });
});

test("ready receipt thumbnails replace initials in place without changing marker geometry", async ({
  page
}) => {
  await gotoReadyShell(page);

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    const next = structuredClone(snapshot);
    next.state.domain.live_signals = {
      rooms: {
        "!harness-room:example.invalid": {
          receipts_by_event: {
            "$seed-event:example.invalid": {
              readers: [{
                user_id: "@alice:example.invalid",
                display_name: "Alice",
                avatar: {
                  mxc_uri: "mxc://example.invalid/alice",
                  thumbnail: { kind: "loading", request_id: 1 }
                },
                timestamp_ms: 1_800_000_000_500
              }],
              total_count: 1,
              overflow_count: 0
            }
          },
          fully_read_event_id: null,
          typing_user_ids: [],
          typing_users: []
        }
      },
      presence: {}
    };
    window.__harness.setSnapshot(next);
    window.__harness.pushStateUpdate();
  });

  const avatar = page.locator('[data-event-id="$seed-event:example.invalid"]')
    .locator(".receipt-reader-avatar");
  await expect(avatar).toHaveText("AL");
  await expect(avatar.locator("img")).toHaveCount(0);
  await avatar.evaluate((element) => {
    element.setAttribute("data-receipt-node-identity", "preserved");
  });

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    const next = structuredClone(snapshot);
    const reader = next.state.domain.live_signals.rooms["!harness-room:example.invalid"]
      .receipts_by_event["$seed-event:example.invalid"].readers[0];
    if (!reader.avatar) throw new Error("seeded reader avatar missing");
    reader.avatar.thumbnail = {
      kind: "ready",
      source_ref: "data:image/gif;base64,R0lGODlhAQABAAAAACw=",
      width: 1,
      height: 1,
      mime_type: "image/gif"
    };
    window.__harness.setSnapshot(next);
    window.__harness.pushStateUpdate();
  });

  await expect(avatar.locator("img")).toHaveCount(1);
  await expect(avatar).not.toHaveText("AL");
  await expect(avatar).toHaveAttribute("data-receipt-node-identity", "preserved");
  expect(await avatar.evaluate((element) => {
    const marker = getComputedStyle(element);
    const image = getComputedStyle(element.querySelector("img")!);
    return {
      width: marker.width,
      height: marker.height,
      borderRadius: marker.borderRadius,
      objectFit: image.objectFit
    };
  })).toEqual({ width: "18px", height: "18px", borderRadius: "50%", objectFit: "cover" });

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    const next = structuredClone(snapshot);
    const reader = next.state.domain.live_signals.rooms["!harness-room:example.invalid"]
      .receipts_by_event["$seed-event:example.invalid"].readers[0];
    if (!reader.avatar) throw new Error("seeded reader avatar missing");
    reader.avatar.thumbnail = { kind: "failed", request_id: 1, failureKind: "network" };
    window.__harness.setSnapshot(next);
    window.__harness.pushStateUpdate();
  });
  await expect(avatar.locator("img")).toHaveCount(0);
  await expect(avatar).toHaveText("AL");
  await expect(avatar).toHaveAttribute("data-receipt-node-identity", "preserved");
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
        domain: {
          ...snapshot.state.domain,
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
                          source_ref:
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
              typing_user_ids: [],
              typing_users: []
            }
          },
          presence: {}
        }
      }
    }
    });
    window.__harness.pushStateUpdate();
  });

  const row = page.locator('[data-event-id="$seed-event:example.invalid"]');
  const receipts = row.locator(".message-receipts");
  await expect(receipts).toHaveAttribute("aria-label", /Read by 4/);
  await expect(receipts).toHaveAttribute("aria-label", /Alice/);
  await expect(receipts.locator(".receipt-reader-avatar")).toHaveCount(3);
  await expect(receipts.locator(".receipt-reader-avatar img")).toHaveCount(1);
  await expect(receipts.locator(".receipt-reader-avatar").nth(1)).toHaveText("DA");
  await expect(receipts.locator(".receipt-overflow")).toHaveText("+1");

  // #314: the reader popup lives in the body-level floating layer so a clipped
  // pane cannot cut it off, so it is no longer a descendant of the row.
  await receipts.hover();
  const readerPopup = page.locator("body > .receipt-tooltip");
  await expect(readerPopup).toBeVisible();
  await expect(receipts.locator(".receipt-tooltip")).toHaveCount(0);
  for (const reader of ["Alice", "Dana", "Bob", "1 more"]) {
    await expect(readerPopup).toContainText(reader);
  }

  // Keyboard users reach the same popup, and it stays inside the viewport.
  await page.mouse.move(0, 0);
  await expect(readerPopup).toHaveCount(0);
  await receipts.focus();
  await expect(readerPopup).toBeVisible();
  const metrics = await readerPopup.evaluate((element) => {
    const rect = element.getBoundingClientRect();
    return {
      bottom: rect.bottom,
      left: rect.left,
      right: rect.right,
      top: rect.top,
      viewportHeight: window.innerHeight,
      viewportWidth: window.innerWidth
    };
  });
  expect(metrics.left).toBeGreaterThanOrEqual(0);
  expect(metrics.top).toBeGreaterThanOrEqual(0);
  expect(metrics.right).toBeLessThanOrEqual(metrics.viewportWidth);
  expect(metrics.bottom).toBeLessThanOrEqual(metrics.viewportHeight);
});

test("Seen popup keeps each reader on one compact line (#360)", async ({ page }) => {
  await gotoReadyShell(page);

  // Two readers, the first with a realistically long display name — the shape
  // from the report, where the first entry wrapped and left a large blank gap
  // before the second.
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          live_signals: {
            rooms: {
              "!harness-room:example.invalid": {
                receipts_by_event: {
                  "$seed-event:example.invalid": {
                    readers: [
                      {
                        user_id: "@long:example.invalid",
                        display_name: "Yoshito Azumagawa",
                        avatar: null,
                        timestamp_ms: 1_800_000_000_500
                      },
                      {
                        user_id: "@bob:example.invalid",
                        display_name: "Bob",
                        avatar: null,
                        timestamp_ms: 1_800_000_000_300
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
            },
            presence: {}
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
  });

  const receipts = page
    .locator('[data-event-id="$seed-event:example.invalid"]')
    .locator(".message-receipts");
  await receipts.focus();
  const popup = page.locator("body > .receipt-tooltip");
  await expect(popup).toBeVisible();

  const layout = await popup.evaluate((element) => {
    const styles = getComputedStyle(element);
    const fontSize = Number.parseFloat(styles.fontSize);
    const rows = Array.from(element.children).map((child) => {
      const rect = child.getBoundingClientRect();
      return { height: rect.height, width: rect.width };
    });
    const chromeHeight =
      Number.parseFloat(styles.paddingBlockStart) +
      Number.parseFloat(styles.paddingBlockEnd) +
      Number.parseFloat(styles.borderBlockStartWidth) +
      Number.parseFloat(styles.borderBlockEndWidth) +
      Number.parseFloat(styles.rowGap || "0") * Math.max(rows.length - 1, 0);
    return {
      chromeHeight,
      fontSize,
      lineHeight: Number.parseFloat(styles.lineHeight),
      popupHeight: element.getBoundingClientRect().height,
      contentWidth: element.clientWidth,
      rows
    };
  });

  expect(layout.rows).toHaveLength(2);

  // Each entry stays on ONE line: a wrapped row would be at least two
  // line-heights tall. This is the assertion the report's screenshot fails.
  for (const row of layout.rows) {
    expect(row.height).toBeLessThan(layout.lineHeight * 1.6);
    // A long name must not overflow the popup's content box.
    expect(row.width).toBeLessThanOrEqual(layout.contentWidth + 1);
  }

  // No large blank vertical gap: the popup is its two single-line rows plus its
  // own padding/border/row-gap, and nothing more. Before the fix the height was
  // a constant 132px regardless of reader count, and the grid stretched its
  // auto rows to fill that slack.
  const rowsHeight = layout.rows.reduce((total, row) => total + row.height, 0);
  const slack = layout.popupHeight - rowsHeight - layout.chromeHeight;
  expect(slack).toBeLessThan(layout.lineHeight);

  // Read-receipt rows use the smaller receipt scale, not body text.
  expect(layout.fontSize).toBeLessThanOrEqual(12);
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
      document: {
        version: 2,
        inlines: [{ kind: "text", text: "Edited seed message" }]
      }
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

  // Re-edit: the edited message remains editable while can_edit is true.
  await row.hover();
  await page.getByRole("button", { name: t("timeline.editMessage") }).first().click();
  await expect(editBody).toBeVisible();
  await editBody.fill("Re-edited seed message");
  await page.evaluate(() => window.__harness.clearInvocations());
  await page.getByRole("button", { name: t("timeline.saveEdit") }).click();
  await expect.poll(() => invocationCount(page, "edit_message")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("edit_message")[0]?.args)
    )
    .toEqual({
      roomId: "!harness-room:example.invalid",
      eventId: "$seed-event:example.invalid",
      document: {
        version: 2,
        inlines: [{ kind: "text", text: "Re-edited seed message" }]
      }
    });
});

test("pretty and minified formatted lists have equivalent compact layout", async ({ page }) => {
  await gotoReadyShell(page);
  const common = {
    sender: "@harness-user:example.invalid",
    body: "Hello world\nnext\n項目一\n項目二\n内側",
    timestamp_ms: 1_800_000_000_950,
    in_reply_to_event_id: null,
    thread_root: null,
    thread_summary: null,
    reactions: [],
    can_react: false,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false
  };
  const plainText = "Hello world\nnext\n項目一\n項目二\n内側";
  await seedTimelineItems(page, [
    {
      ...common,
      id: { Event: { event_id: "$formatted-minified:example.invalid" } },
      formatted: {
        html: "<p><strong>Hello</strong> <em>world</em><br>next</p><ul><li>項目一</li><li>項目二<ol><li>内側</li></ol></li></ul>",
        plain_text: plainText,
        code_blocks: []
      }
    },
    {
      ...common,
      id: { Event: { event_id: "$formatted-pretty:example.invalid" } },
      formatted: {
        html: `
          <p><strong>Hello</strong> <em>world</em><br>next</p>
          <ul>
            <li>項目一</li>
            <li>項目二
              <ol>
                <li>内側</li>
              </ol>
            </li>
          </ul>
        `,
        plain_text: plainText,
        code_blocks: []
      }
    }
  ]);

  const minified = page.locator('[data-event-id="$formatted-minified:example.invalid"] .message-formatted-body');
  const pretty = page.locator('[data-event-id="$formatted-pretty:example.invalid"] .message-formatted-body');
  await expect(minified.locator("br")).toHaveCount(1);
  await expect(pretty.locator("br")).toHaveCount(1);
  await expect(minified.locator("ul > br, ol > br")).toHaveCount(0);
  await expect(pretty.locator("ul > br, ol > br")).toHaveCount(0);
  expect(
    await pretty.locator("ul, ol").evaluateAll((lists) =>
      lists.every((list) => Array.from(list.children).every((child) => child.tagName === "LI"))
    )
  ).toBe(true);
  const heights = await Promise.all([
    minified.evaluate((element) => element.getBoundingClientRect().height),
    pretty.evaluate((element) => element.getBoundingClientRect().height)
  ]);
  expect(Math.abs(heights[0] - heights[1])).toBeLessThanOrEqual(2);
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
          domain: {
            ...snapshot.state.domain,
            profile: {
              ...snapshot.state.domain.profile,
              ignored_user_ids: [...snapshot.state.domain.profile.ignored_user_ids, String(incomingUserId)]
            }
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
          domain: {
            ...snapshot.state.domain,
            profile: {
              ...snapshot.state.domain.profile,
              ignored_user_ids: snapshot.state.domain.profile.ignored_user_ids.filter(
                (id) => id !== String(incomingUserId)
              )
            }
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
      page.evaluate(() => window.__harness.currentSnapshot().state.domain.profile.ignored_user_ids)
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
      page.evaluate(() => window.__harness.currentSnapshot().state.domain.profile.ignored_user_ids)
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
