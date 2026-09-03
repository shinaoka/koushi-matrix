import { expect, test, type Locator, type Page } from "@playwright/test";
import { roomTimelineKey, threadTimelineKey } from "../src/domain/coreEvents";
import { t } from "../src/i18n/messages";
import {
  HARNESS_ROOM_ID,
  gotoReadyShell,
  invocationCount,
  pushTimelineDiffs,
  seedTimelineItems
} from "./support/basicOperations";


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

function makeSendQueueItem(
  transactionId: string,
  body: string,
  sendState:
    | { kind: "sending" }
    | { kind: "notSent"; reason: "recoverable" | "unrecoverable" }
    | { kind: "cancelled" }
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

test("main composer Tab focuses Send before auxiliary controls", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());
  const composer = page.getByRole("textbox", { name: t("composer.messageComposer") });
  const send = page.getByRole("button", { name: t("action.send"), exact: true });

  await composer.fill("keyboard traversal");
  await composer.press("Tab");
  await expect(send).toBeFocused();
  await page.keyboard.press("Tab");
  const attach = page.getByRole("button", { name: t("composer.attachFile"), exact: true });
  await expect(attach).toBeFocused();
  const [attachBox, sendBox] = await Promise.all([attach.boundingBox(), send.boundingBox()]);
  expect(attachBox).not.toBeNull();
  expect(sendBox).not.toBeNull();
  expect(attachBox!.x).toBeLessThan(sendBox!.x);
  await page.keyboard.press("Tab");
  await expect(page.getByRole("button", { name: t("composer.mention"), exact: true })).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(page.getByRole("button", { name: t("composer.emoji"), exact: true })).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(page.getByRole("button", { name: t("scheduled.sendLater"), exact: true })).toBeFocused();

  await composer.fill("");
  await composer.focus();
  await composer.press("Tab");
  await expect(page.getByRole("button", { name: t("composer.attachFile"), exact: true })).toBeFocused();

  await composer.fill("keyboard send");
  await composer.press("Tab");
  await expect(send).toBeFocused();
  await send.press("Enter");
  await expect.poll(() => invocationCount(page, "send_text")).toBe(1);
});

test("main composer focused Send activates once with Space", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());
  const composer = page.getByRole("textbox", { name: t("composer.messageComposer") });
  const send = page.getByRole("button", { name: t("action.send"), exact: true });
  await composer.fill("keyboard space send");
  await composer.press("Tab");
  await expect(send).toBeFocused();
  await send.press("Space");
  await expect.poll(() => invocationCount(page, "send_text")).toBe(1);
});

test("accepted send stays visible without a local echo through event convergence", async ({
  page
}) => {
  await gotoReadyShell(page);
  const body = "Visible before local echo";
  await page.evaluate((pendingBody) => {
    (
      window.__harness as typeof window.__harness & {
        setNextTextSendPendingBody(body: string): void;
      }
    ).setNextTextSendPendingBody(pendingBody);
    window.__harness.clearInvocations();
  }, body);

  const composer = page.getByRole("textbox", { name: t("composer.messageComposer") });
  await composer.fill(body);
  await page.getByRole("button", { name: t("action.send"), exact: true }).click();
  await expect(composer).toHaveText("");

  const { submissionId, transactionId: clientTransactionId } = await page.evaluate(() => {
    const args = window.__harness.invocationsOf("send_text")[0]?.args;
    return {
      submissionId: args.submissionId as string,
      transactionId: args.transactionId as string
    };
  });
  const sdkTransactionId = `harness-${submissionId}`;
  const pendingRow = page.locator(`[data-item-id="txn:${clientTransactionId}"]`);
  await expect(pendingRow).toHaveCount(1);
  await expect(pendingRow.getByText(t("timeline.sending"))).toBeVisible();

  const sdkPending = makeSendQueueItem(sdkTransactionId, body, { kind: "sending" });
  await pushTimelineDiffs(page, [{ Set: { index: 1, item: sdkPending } }], 1, 9_001);
  const sdkRow = page.locator(`[data-item-id="txn:${sdkTransactionId}"]`);
  await expect(pendingRow).toHaveCount(0);
  await expect(sdkRow).toHaveCount(1);
  await expect(sdkRow.getByText(t("timeline.sending"))).toBeVisible();

  const eventId = "$accepted-without-local-echo:example.invalid";
  const sentFallback = {
    ...sdkPending,
    id: { Event: { event_id: eventId } },
    send_state: { kind: "sent" as const }
  };
  await pushTimelineDiffs(page, [{ Set: { index: 1, item: sentFallback } }], 1, 9_002);
  const eventRow = page.locator(`[data-item-id="${eventId}"]`);
  await expect(sdkRow).toHaveCount(0);
  await expect(eventRow).toHaveCount(1);
  await expect(eventRow.getByText(body)).toBeVisible();

  await pushTimelineDiffs(
    page,
    [
      {
        Set: {
          index: 1,
          item: { ...sentFallback, body: "Canonical remote echo" }
        }
      }
    ],
    1,
    9_003
  );
  await expect(eventRow).toHaveCount(1);
  await expect(eventRow.getByText("Canonical remote echo")).toBeVisible();
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

test("room mention candidates stay Rust-owned and send typed mention intent", async ({
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
          profile: {
            ...snapshot.state.domain.profile,
            users: {
              ...snapshot.state.domain.profile.users,
              "@account-global-only:example.invalid": {
                user_id: "@account-global-only:example.invalid",
                display_name: "Account Global Only",
                display_label: "Account Global Only",
                original_display_label: "Account Global Only",
                mention_search_terms: ["account", "global"],
                avatar: null
              }
            }
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
    window.__harness.clearInvocations();
  });
  const composer = page.getByRole("textbox", { name: "Message composer" });
  await composer.fill("@a");
  await expect(page.getByRole("listbox", { name: "Mention suggestions" })).toBeVisible();
  await expect(page.getByRole("option", { name: "Alice @alice:example.invalid" })).toBeVisible();
  await expect(page.getByRole("option", { name: /Account Global Only/ })).toHaveCount(0);
  await page.getByRole("option", { name: "Alice @alice:example.invalid" }).click();
  await expect(page.getByRole("link", { name: "Mention: Alice" })).toHaveText("@Alice");
  await expect(page.locator(".composer-mention-pills")).toHaveCount(0);

  await composer.press("Backspace");
  await composer.press("Backspace");
  await expect(page.getByRole("link", { name: "Mention: Alice" })).toHaveCount(0);
  await composer.evaluate((element) =>
    element.dispatchEvent(
      new InputEvent("beforeinput", {
        bubbles: true,
        cancelable: true,
        inputType: "historyUndo"
      })
    )
  );
  await expect(page.getByRole("link", { name: "Mention: Alice" })).toHaveText("@Alice");

  await page.getByRole("button", { name: "Send", exact: true }).click();

  await expect.poll(() => invocationCount(page, "send_text")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("send_text")[0]?.args))
    .toMatchObject({
      accountHomeserver: "https://harness.example.invalid",
      accountUserId: "@harness-user:example.invalid",
      accountDeviceId: "HARNESSDEVICE",
      roomId: HARNESS_ROOM_ID,
      document: {
        version: 2,
        inlines: [
          {
            kind: "mention",
            target: {
              kind: "user",
              user_id: "@alice:example.invalid",
              display_label: "Alice"
            },
            display_label: "Alice"
          }
        ]
      }
  });
});

test("room mention candidates keep main and thread composer targets independent", async ({
  page
}) => {
  await gotoReadyShell(page);
  const mainComposer = page.getByRole("textbox", { name: "Message composer" });
  await mainComposer.fill("@a");
  await expect(
    page.getByRole("option", { name: "Alice @alice:example.invalid" })
  ).toBeVisible();

  await page.getByRole("button", { name: /2 replies/ }).click();
  const threadComposer = page.getByRole("textbox", { name: "Thread composer" });
  await threadComposer.fill("@b");

  const mainSuggestions = page
    .getByRole("region", { name: "Message composer" })
    .getByRole("listbox", { name: "Mention suggestions" });
  const threadSuggestions = page
    .getByRole("region", { name: "Thread composer" })
    .getByRole("listbox", { name: "Mention suggestions" });
  await expect(
    threadSuggestions.getByRole("option", { name: "Bob @bob:example.invalid" })
  ).toBeVisible();
  await expect(
    threadSuggestions.getByRole("option", { name: "Alice @alice:example.invalid" })
  ).toHaveCount(0);
  await expect(
    mainSuggestions.getByRole("option", { name: "Alice @alice:example.invalid" })
  ).toBeVisible();
  await threadSuggestions
    .getByRole("option", { name: "Bob @bob:example.invalid" })
    .click();
  await expect(threadComposer.getByRole("link", { name: "Mention: Bob" })).toHaveText("@Bob");
  await expect
    .poll(async () =>
      page.evaluate(() => {
        const invocations = window.__harness.invocationsOf("query_mention_candidates");
        return invocations[invocations.length - 1]?.args;
      })
    )
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      surface: "thread",
      query: "b"
    });
});

test("markdown toolbar and slash composer input dispatch Rust-owned send bodies", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    window.__harness.setCommandResponse("open_threads_list", () =>
      window.__harness.currentSnapshot()
    );
    window.__harness.clearInvocations();
  });

  const composer = page.getByRole("textbox", { name: "Message composer" });
  await composer.fill("world");
  await composer.selectText();
  await page.getByRole("button", { name: "Bold" }).click();
  await expect(composer).toHaveText("**world**");
  await page.getByRole("button", { name: "Send", exact: true }).click();

  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("send_text")[0]?.args))
    .toMatchObject({
      accountHomeserver: "https://harness.example.invalid",
      accountUserId: "@harness-user:example.invalid",
      accountDeviceId: "HARNESSDEVICE",
      roomId: HARNESS_ROOM_ID,
      document: {
        version: 2,
        inlines: [{ kind: "text", text: "**world**" }]
      }
    });

  await page.evaluate(() => window.__harness.clearInvocations());
  await composer.fill("/me waves");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("send_text")[0]?.args))
    .toMatchObject({
      accountHomeserver: "https://harness.example.invalid",
      accountUserId: "@harness-user:example.invalid",
      accountDeviceId: "HARNESSDEVICE",
      roomId: HARNESS_ROOM_ID,
      document: {
        version: 2,
        inlines: [{ kind: "text", text: "/me waves" }]
      }
  });
});

test("composer string revision stays exact above Number.MAX_SAFE_INTEGER", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const current = window.__harness.currentSnapshot();
    const exactRevision = "9007199254740993";
    window.__harness.setSnapshot({
      ...current,
      state: {
        ...current.state,
        ui: {
          ...current.state.ui,
          timeline: {
            ...current.state.ui.timeline,
            composer: {
              ...current.state.ui.timeline.composer,
              draft: "exact Rust baseline",
              document: {
                version: 2,
                inlines: [{ kind: "text", text: "exact Rust baseline" }]
              },
              draft_revision: exactRevision,
              last_accepted_clear_revision: exactRevision
            }
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
    window.__harness.clearInvocations();
  });

  const composer = page.getByRole("textbox", { name: "Message composer" });
  await expect(composer).toHaveText("exact Rust baseline");
  await composer.fill("exact revision");
  await expect
    .poll(() =>
      page.evaluate(() => window.__harness.invocationsOf("set_composer_draft").at(-1)?.args)
    )
    .toMatchObject({
      document: {
        version: 2,
        inlines: [{ kind: "text", text: "exact revision" }]
      },
      draftRevision: "9007199254740994"
    });
});

test("main composer delayed write survives churn then rejects stale completion", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const primaryRoomId = "!harness-room:example.invalid";
    const secondaryRoomId = "!draft-room-b:example.invalid";
    const draftByRoom: Record<string, string> = {};
    const revisionByRoom: Record<string, string> = {};
    const base = window.__harness.currentSnapshot();
    const rooms = [
      base.state.domain.rooms[0],
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
          domain: {
            ...current.state.domain,
            rooms
          },
          ui: {
            ...current.state.ui,
            navigation: {
              ...current.state.ui.navigation,
              active_room_id: roomId
            },
            timeline: {
              ...current.state.ui.timeline,
              room_id: roomId,
              is_subscribed: true,
              composer: {
                accepted_submission_ids: [],
                pending_transaction_id: null,
                draft: draftByRoom[roomId] ?? "",
                document: {
                  version: 2,
                  inlines: draftByRoom[roomId]
                    ? [{ kind: "text", text: draftByRoom[roomId] }]
                    : []
                },
                draft_revision: revisionByRoom[roomId] ?? "0",
                last_accepted_clear_revision: "0",
                mode: "Plain"
              }
            },
            thread: { kind: "closed" },
            focused_context: { kind: "closed" }
          }
        },
        sidebar: {
          ...current.sidebar,
          space_rooms: roomListItems,
          sections: { ...current.sidebar.sections, rooms: roomListItems }
        },
        thread: null
      };
    };

    window.__harness.setSnapshot(projectRoom(primaryRoomId));
    window.__harness.setCommandResponse(
      "set_composer_draft",
      ({
        roomId,
        document,
        draftRevision
      }: {
        roomId: string;
        document: { inlines: Array<{ kind: string; text?: string; display_label?: string }> };
        draftRevision: string;
      }) => {
        const normalizedRoomId = String(roomId);
        const draft = document.inlines
          .map((inline) => inline.kind === "text" ? inline.text ?? "" : `@${inline.display_label ?? ""}`)
          .join("");
        if (BigInt(draftRevision) <= BigInt(revisionByRoom[normalizedRoomId] ?? "0")) {
          return projectRoom(
            window.__harness.currentSnapshot().state.ui.timeline.room_id ?? primaryRoomId
          );
        }
        revisionByRoom[normalizedRoomId] = draftRevision;
        if (draft.length === 0) {
          delete draftByRoom[normalizedRoomId];
        } else {
          draftByRoom[normalizedRoomId] = draft;
        }
        const next = projectRoom(window.__harness.currentSnapshot().state.ui.timeline.room_id ?? primaryRoomId);
        window.__harness.setSnapshot(next);
        return next;
      }
    );
    window.__harness.setCommandResponse("select_room", ({ roomId }: { roomId: string }) => {
      const next = projectRoom(String(roomId));
      window.__harness.setSnapshot(next);
      return next;
    });
    window.__harness.pushStateUpdate();
    window.__harness.clearInvocations();
  });

  const composer = page.getByRole("textbox", { name: "Message composer" });
  await composer.fill("Room A draft");
  await expect(composer).toHaveText("Room A draft");
  // Switch before the debounce expires: persistence timers are target-owned,
  // so editing room B must not cancel room A's pending encrypted-store write.
  await page
    .getByRole("button", { name: "Draft Room B" })
    .evaluate((button: HTMLButtonElement) => button.click());
  await expect(composer).toHaveText("");
  await composer.fill("Room B draft");
  await expect(composer).toHaveText("Room B draft");
  await expect
    .poll(async () =>
      page.evaluate(() =>
        window.__harness
          .invocationsOf("set_composer_draft")
          .map(({ args }) => args)
      )
    )
    .toMatchObject([
      {
        accountHomeserver: "https://harness.example.invalid",
        accountUserId: "@harness-user:example.invalid",
        accountDeviceId: "HARNESSDEVICE",
        roomId: HARNESS_ROOM_ID,
        document: { version: 2, inlines: [{ kind: "text", text: "Room A draft" }] },
        draftRevision: "1"
      },
      {
        accountHomeserver: "https://harness.example.invalid",
        accountUserId: "@harness-user:example.invalid",
        accountDeviceId: "HARNESSDEVICE",
        roomId: "!draft-room-b:example.invalid",
        document: { version: 2, inlines: [{ kind: "text", text: "Room B draft" }] },
        draftRevision: "1"
      }
    ]);

  await page.getByRole("button", { name: "Harness Room" }).click();
  await expect(composer).toHaveText("Room A draft");
  await page.getByRole("button", { name: "Draft Room B" }).click();
  await expect(composer).toHaveText("Room B draft");
});

test("main composer keeps an emptied local draft across stale snapshot refresh", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const roomId = "!harness-room:example.invalid";
    const current = window.__harness.currentSnapshot();
    const staleDraftSnapshot = {
      ...current,
      state: {
        ...current.state,
        ui: {
          ...current.state.ui,
          timeline: {
            ...current.state.ui.timeline,
            room_id: roomId,
            composer: {
              ...current.state.ui.timeline.composer,
              draft: "stale draft from Rust",
              document: {
                version: 2,
                inlines: [{ kind: "text", text: "stale draft from Rust" }]
              }
            }
          }
        }
      }
    };
    window.__harness.setSnapshot(staleDraftSnapshot);
    window.__harness.setCommandResponse(
      "set_composer_draft",
      ({ document, draftRevision }: {
        document: { version: 2; inlines: Array<{ kind: string; text?: string; display_label?: string }> };
        draftRevision: string;
      }) => {
        const draft = document.inlines
          .map((inline) => inline.kind === "text" ? inline.text ?? "" : `@${inline.display_label ?? ""}`)
          .join("");
        const snapshot = window.__harness.currentSnapshot();
        return {
          ...snapshot,
          state: {
            ...snapshot.state,
            ui: {
              ...snapshot.state.ui,
              timeline: {
                ...snapshot.state.ui.timeline,
                composer: {
                  ...snapshot.state.ui.timeline.composer,
                  draft,
                  document,
                  draft_revision: draftRevision
                }
              }
            }
          }
        };
      }
    );
    window.__harness.pushStateUpdate();
    window.__harness.clearInvocations();
  });

  const composer = page.getByRole("textbox", { name: "Message composer" });
  await expect(composer).toHaveText("stale draft from Rust");
  await composer.fill("typed then removed");
  await expect(composer).toHaveText("typed then removed");

  await composer.fill("");
  await expect(composer).toHaveText("");
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("set_composer_draft").at(-1)?.args)
    )
    .toMatchObject({
      accountHomeserver: "https://harness.example.invalid",
      accountUserId: "@harness-user:example.invalid",
      accountDeviceId: "HARNESSDEVICE",
      roomId: HARNESS_ROOM_ID,
      document: { version: 2, inlines: [] },
      draftRevision: "2"
    });

  await page.evaluate(() => {
    window.__harness.pushStateUpdate();
  });
  await expect(composer).toHaveText("");
});

for (const completionOrder of ["accepted-first", "persist-first"] as const) {
  test(`accepted clear changes IME sync while newer input does not (${completionOrder})`, async ({
    page
  }) => {
    await gotoReadyShell(page);
    await page.evaluate(() => {
      type RaceControls = Window & {
        __resolveDraftPersistRace?: () => void;
        __resolveDraftSendRace?: () => void;
      };
      const raceWindow = window as RaceControls;
      let resolvePersist!: () => void;
      let resolveSend!: () => void;
      const persistGate = new Promise<void>((resolve) => {
        resolvePersist = resolve;
      });
      const sendGate = new Promise<void>((resolve) => {
        resolveSend = resolve;
      });
      raceWindow.__resolveDraftPersistRace = resolvePersist;
      raceWindow.__resolveDraftSendRace = resolveSend;

      window.__harness.setCommandResponse(
        "set_composer_draft",
        async ({
          document,
          draftRevision
        }: {
          roomId: string;
          document: { version: 2; inlines: Array<{ kind: string; text?: string; display_label?: string }> };
          draftRevision: string;
        }) => {
          const draft = document.inlines
            .map((inline) => inline.kind === "text" ? inline.text ?? "" : `@${inline.display_label ?? ""}`)
            .join("");
          if (draftRevision === "1") {
            await persistGate;
          }
          const current = window.__harness.currentSnapshot();
          if (
            BigInt(draftRevision) <= BigInt(current.state.ui.timeline.composer.draft_revision)
          ) {
            return current;
          }
          const next = {
            ...current,
            state: {
              ...current.state,
              ui: {
                ...current.state.ui,
                timeline: {
                  ...current.state.ui.timeline,
                  composer: {
                    ...current.state.ui.timeline.composer,
                    draft,
                    document,
                    draft_revision: draftRevision
                  }
                }
              }
            }
          };
          window.__harness.setSnapshot(next);
          return next;
        }
      );
      window.__harness.setCommandResponse(
        "send_text",
        async ({
          submissionId,
          draftRevision
        }: {
          submissionId: string;
          draftRevision: string;
        }) => {
          await sendGate;
          const current = window.__harness.currentSnapshot();
          const currentComposer = current.state.ui.timeline.composer;
          const acceptedRevision = (
            (BigInt(currentComposer.draft_revision) > BigInt(draftRevision)
              ? BigInt(currentComposer.draft_revision)
              : BigInt(draftRevision)) + 1n
          ).toString();
          const next = {
            outcome: "accepted",
            submissionId,
            transactionId: "synthetic-transaction",
            snapshot: {
              ...current,
              state: {
                ...current.state,
                ui: {
                  ...current.state.ui,
                  timeline: {
                    ...current.state.ui.timeline,
                    composer: {
                      ...currentComposer,
                      draft:
                        BigInt(currentComposer.draft_revision) > BigInt(draftRevision)
                          ? currentComposer.draft
                          : "",
                      document:
                        BigInt(currentComposer.draft_revision) > BigInt(draftRevision)
                          ? currentComposer.document
                          : { version: 2, inlines: [] },
                      draft_revision:
                        acceptedRevision,
                      last_accepted_clear_revision: acceptedRevision
                    }
                  }
                }
              }
            }
          };
          window.__harness.setSnapshot(next.snapshot);
          return next;
        }
      );
      window.__harness.clearInvocations();
    });

    const composer = page.getByRole("textbox", { name: "Message composer" });
    await composer.fill("accepted message");
    await expect
      .poll(() => invocationCount(page, "set_composer_draft"))
      .toBe(1);

    await page.getByRole("button", { name: "Send", exact: true }).click();
    await expect.poll(() => invocationCount(page, "send_text")).toBe(1);
    await composer.fill("immediate next input");

    if (completionOrder === "accepted-first") {
      await page.evaluate(() =>
        (window as Window & { __resolveDraftSendRace?: () => void }).__resolveDraftSendRace?.()
      );
      await page.evaluate(() =>
        (window as Window & { __resolveDraftPersistRace?: () => void })
          .__resolveDraftPersistRace?.()
      );
    } else {
      await page.evaluate(() =>
        (window as Window & { __resolveDraftPersistRace?: () => void })
          .__resolveDraftPersistRace?.()
      );
      await expect.poll(() => invocationCount(page, "set_composer_draft")).toBe(2);
      await page.evaluate(() =>
        (window as Window & { __resolveDraftSendRace?: () => void }).__resolveDraftSendRace?.()
      );
    }

    await expect(composer).toHaveText("immediate next input");
    await expect
      .poll(async () =>
        page.evaluate(
          () => window.__harness.invocationsOf("set_composer_draft").at(-1)?.args.draftRevision
        )
      )
      .toBe("3");
    await expect(composer).toHaveText("immediate next input");
  });
}

test("account switch revokes unresolved composer lifecycle", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    type AccountRaceControls = Window & {
      __resolvePreviousAccountSend?: () => void;
    };
    let resolveSend!: () => void;
    const sendGate = new Promise<void>((resolve) => {
      resolveSend = resolve;
    });
    (window as AccountRaceControls).__resolvePreviousAccountSend = resolveSend;
    window.__harness.setCommandResponse(
      "send_text",
      async ({ submissionId }: { submissionId: string }) => {
        await sendGate;
        const currentSnapshot = window.__harness.currentSnapshot();
        return {
          outcome: "accepted",
          submissionId,
          transactionId: "previous-account-transaction",
          snapshot: currentSnapshot
        };
      }
    );
    window.__harness.clearInvocations();
  });

  const composer = page.getByRole("textbox", { name: "Message composer" });
  await composer.fill("previous account draft");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect.poll(() => invocationCount(page, "send_text")).toBe(1);

  await page.evaluate(() => {
    const current = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...current,
      state: {
        ...current.state,
        domain: {
          ...current.state.domain,
          session: {
            ...current.state.domain.session,
            kind: "ready",
            homeserver: "https://next-account.example.invalid",
            user_id: "@next-account:example.invalid",
            device_id: "NEXTDEVICE"
          }
        },
        ui: {
          ...current.state.ui,
          timeline: {
            ...current.state.ui.timeline,
            composer: {
              ...current.state.ui.timeline.composer,
              draft: "next account draft",
              document: {
                version: 2,
                inlines: [{ kind: "text", text: "next account draft" }]
              },
              draft_revision: "1"
            }
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
  });
  await expect(composer).toHaveText("next account draft");

  await page.evaluate(() =>
    (window as Window & { __resolvePreviousAccountSend?: () => void })
      .__resolvePreviousAccountSend?.()
  );
  await expect(composer).toHaveText("next account draft");
});

for (const failure of ["rejected", "timeout"] as const) {
  test(`main composer retains its draft when submission is ${failure}`, async ({ page }) => {
    await gotoReadyShell(page);
    await page.evaluate((failureKind) => {
      window.__harness.setCommandResponse(
        "send_text",
        ({ submissionId }: { submissionId: string }) => {
          if (failureKind === "timeout") {
            throw "timeout";
          }
          return {
            outcome: "rejected",
            submissionId,
            transactionId: null,
            snapshot: window.__harness.currentSnapshot()
          };
        }
      );
      window.__harness.clearInvocations();
    }, failure);

    const composer = page.getByRole("textbox", { name: "Message composer" });
    await composer.fill("draft retained after failure");
    await page.getByRole("button", { name: "Send", exact: true }).click();

    await expect.poll(() => invocationCount(page, "send_text")).toBe(1);
    await expect(composer).toHaveText("draft retained after failure");
    await expect
      .poll(async () =>
        page.evaluate(() => window.__harness.invocationsOf("send_text")[0]?.args.draftRevision)
      )
      .toBe("1");
  });
}

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
        draft = window.__harness.currentSnapshot().state.ui.timeline.composer.draft,
        draftRevision =
          window.__harness.currentSnapshot().state.ui.timeline.composer.draft_revision,
        lastAcceptedClearRevision =
          window.__harness.currentSnapshot().state.ui.timeline.composer
            .last_accepted_clear_revision
      ) => {
        const current = window.__harness.currentSnapshot();
        return {
          ...current,
          state: {
            ...current.state,
            ui: {
              ...current.state.ui,
              timeline: {
                ...current.state.ui.timeline,
                scheduled_send_capability: "localFallback",
                scheduled_sends: items,
                composer: {
                  ...current.state.ui.timeline.composer,
                  draft,
                  document: {
                    version: 2,
                    inlines: draft ? [{ kind: "text", text: draft }] : []
                  },
                  draft_revision: draftRevision,
                  last_accepted_clear_revision: lastAcceptedClearRevision
                }
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
        "set_composer_draft",
        ({ document, draftRevision }: {
          document: { inlines: Array<{ kind: string; text?: string; display_label?: string }> };
          draftRevision: string;
        }) => {
          const draft = document.inlines
            .map((inline) => inline.kind === "text" ? inline.text ?? "" : `@${inline.display_label ?? ""}`)
            .join("");
          const next = projectScheduled([], draft, draftRevision);
          window.__harness.setSnapshot(next);
          return next;
        }
      );
      window.__harness.setCommandResponse(
        "schedule_send",
        ({
          body,
          draftRevision
        }: {
          body: string;
          sendAtMs: number;
          draftRevision: string;
        }) => {
          const acceptedRevision = (BigInt(draftRevision) + 1n).toString();
          const next = projectScheduled(
            [{ ...scheduledItem, body: String(body) }],
            "",
            acceptedRevision,
            acceptedRevision
          );
          window.__harness.setSnapshot(next);
          return {
            acceptedRevision: next.state.ui.timeline.composer.draft_revision,
            snapshot: next
          };
        }
      );
      window.__harness.setCommandResponse("reschedule_scheduled_send", () =>
        window.__harness.currentSnapshot()
      );
      window.__harness.setCommandResponse("cancel_scheduled_send", () =>
        window.__harness.currentSnapshot()
      );
      window.__harness.pushStateUpdate();
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
    .toMatchObject({
      accountHomeserver: "https://harness.example.invalid",
      accountUserId: "@harness-user:example.invalid",
      accountDeviceId: "HARNESSDEVICE",
      target: { kind: "main", room_id: HARNESS_ROOM_ID },
      body: "Phase B scheduled body",
      sendAtMs: initialSendAt,
      draftRevision: "1"
    });
  await expect(page.getByRole("region", { name: "Scheduled messages" })).toContainText(
    "Phase B scheduled body"
  );
  await expect
    .poll(() =>
      page.evaluate(() => window.__harness.currentSnapshot().state.ui.timeline.composer)
    )
    .toMatchObject({ draft: "", draft_revision: "2" });
  await expect(page.getByRole("textbox", { name: "Message composer" })).toHaveText("");

  await page.getByRole("button", { name: "Edit scheduled send" }).click();
  const scheduledBody = page.getByRole("textbox", { name: "Scheduled message" });
  await expect(scheduledBody).toHaveText("Phase B scheduled body");
  await scheduledBody.fill("Phase B edited scheduled body");
  await page.getByLabel("Scheduled send time").fill("2030-01-03T04:05");
  await page.getByRole("button", { name: "Save scheduled send" }).click();
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("reschedule_scheduled_send")[0]?.args)
    )
    .toEqual({
      scheduledId: "scheduled-harness-1",
      body: "Phase B edited scheduled body",
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
        ui: {
          ...current.state.ui,
          timeline: {
            ...current.state.ui.timeline,
            scheduled_sends: current.state.ui.timeline.scheduled_sends.map((item) =>
              item.scheduled_id === "scheduled-harness-1"
                ? { ...item, body: "Phase B edited scheduled body", send_at_ms: editedSendAt }
                : item
            )
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
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
    "Phase B edited scheduled body"
  );
  await page.evaluate(() => {
    const current = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...current,
      state: {
        ...current.state,
        ui: {
          ...current.state.ui,
          timeline: {
            ...current.state.ui.timeline,
            scheduled_sends: []
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
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
        domain: {
          ...snapshot.state.domain,
          profile: {
            ...snapshot.state.domain.profile,
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
      }
    });
    window.__harness.pushStateUpdate();
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

  expect(await invocationCount(page, "resolve_composer_key_action")).toBe(0);
  expect(await invocationCount(page, "send_text")).toBe(0);
  await expect(page.getByRole("listbox", { name: "Mention suggestions" })).toBeVisible();
  await expect(composer).toHaveText("@a");
});

test("thread and edit composers composing Enter never send through GUI", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.getByRole("button", { name: /2 replies/ }).click();
  const threadComposer = page.getByRole("textbox", { name: t("timeline.threadComposer") });
  await expect(threadComposer).toBeVisible();
  const contextPanel = page.locator('aside[aria-label="Context panel"]');
  await expect(contextPanel.getByRole("button", { name: "Bold" })).toBeVisible();
  await expect(contextPanel.getByRole("button", { name: "Italic" })).toBeVisible();
  await expect(
    contextPanel.getByRole("button", { name: "Attach file", exact: true })
  ).toBeVisible();
  await threadComposer.fill("スレッド変換中");
  await page.evaluate(() => window.__harness.clearInvocations());

  await expect(await dispatchComposingEnter(threadComposer)).toBe(false);

  expect(await invocationCount(page, "resolve_composer_key_action")).toBe(0);
  expect(await invocationCount(page, "send_thread_reply")).toBe(0);
  await expect(threadComposer).toHaveText("スレッド変換中");

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

  expect(await invocationCount(page, "resolve_composer_key_action")).toBe(0);
  expect(await invocationCount(page, "edit_message")).toBe(0);
  await expect(editBody).toHaveText("編集変換中");
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

test("cancelled send queue row renders the cancelled state label", async ({ page }) => {
  await gotoReadyShell(page);
  const cancelled = makeSendQueueItem("txn-cancelled", "Synthetic cancelled send", {
    kind: "cancelled"
  });
  await seedTimelineItems(page, [cancelled]);

  const row = page.locator('[data-item-id="txn:txn-cancelled"]');
  await expect(row).toHaveAttribute("data-send-state", "cancelled");
  await expect(row.locator('[data-send-state="cancelled"]')).toHaveText("Cancelled");
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

test("attach control stages media caption and renders Rust-owned media progress", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
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
  // Attachments are sent from the staging panel; the composer send is
  // for the message text only.
  await page.getByRole("button", { name: "Send attachments" }).click();

  await expect.poll(() => invocationCount(page, "send_prepared_uploads")).toBe(1);
  await expect.poll(() => invocationCount(page, "send_text")).toBe(0);
  await expect
    .poll(async () =>
      page.evaluate(() => {
        const args = window.__harness.invocationsOf("send_prepared_uploads")[0]?.args;
        return args
          ? {
              target: args.target,
              draftRevision: args.draftRevision
            }
          : null;
      })
    )
    .toEqual({
      target: { kind: "main", room_id: "!harness-room:example.invalid" },
      draftRevision: "1"
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

test("paste/drop upload UX stages ordinary files for the captured main composer target", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.evaluate(() => {
    const file = new File(["pdf fixture bytes"], "dropped-fixture.pdf", {
      type: "application/pdf"
    });
    const data = new DataTransfer();
    data.items.add(file);
    const composer = document.querySelector("section.composer");
    if (!composer) {
      throw new Error("composer not found");
    }
    composer.dispatchEvent(
      new DragEvent("drop", {
        bubbles: true,
        cancelable: true,
        dataTransfer: data
      })
    );
  });

  await page.evaluate(() => {
    const file = new File(["zip fixture bytes"], "pasted-fixture.zip", {
      type: "application/zip"
    });
    const data = new DataTransfer();
    data.items.add(file);
    const editor = document.querySelector('[contenteditable="true"][aria-label="Message composer"]');
    if (!editor) throw new Error("composer editor not found");
    editor.dispatchEvent(
      new ClipboardEvent("paste", { bubbles: true, cancelable: true, clipboardData: data })
    );
  });

  await expect.poll(() => invocationCount(page, "stage_upload_bytes")).toBe(2);
  await expect.poll(() => invocationCount(page, "send_prepared_uploads")).toBe(0);
  await expect
    .poll(async () =>
      page.evaluate(() =>
        window.__harness.invocationsOf("stage_upload_bytes").map((invocation) => ({
          target: invocation.args.target,
          filename: invocation.args.items?.[0]?.filename,
          mimeType: invocation.args.items?.[0]?.mimeType,
          byteCount: invocation.args.items?.[0]?.bytes?.length
        }))
      )
    )
    .toEqual([
      {
        target: { kind: "main", room_id: "!harness-room:example.invalid" },
        filename: "dropped-fixture.pdf",
        mimeType: "application/pdf",
        byteCount: "pdf fixture bytes".length
      },
      {
        target: { kind: "main", room_id: "!harness-room:example.invalid" },
        filename: "pasted-fixture.zip",
        mimeType: "application/zip",
        byteCount: "zip fixture bytes".length
      }
    ]);
  await expect(page.getByRole("dialog", { name: "Upload attachments" })).toBeVisible();
  await expect(page.getByText("dropped-fixture.pdf", { exact: true })).toBeVisible();
  await expect(page.getByText("pasted-fixture.zip", { exact: true })).toBeVisible();

  await page.getByRole("textbox", { name: "Caption for dropped-fixture.pdf" }).fill("caption from staging");
  await expect.poll(() => invocationCount(page, "update_staged_upload_caption")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("update_staged_upload_caption").at(-1)?.args)
    )
    .toMatchObject({
      target: { kind: "main", room_id: "!harness-room:example.invalid" },
      document: {
        version: 2,
        inlines: [{ kind: "text", text: "caption from staging" }]
      }
    });

  // Attachments are sent from the staging panel; the composer send is
  // for the message text only.
  await page.getByRole("button", { name: "Send attachments" }).click();
  await expect.poll(() => invocationCount(page, "send_prepared_uploads")).toBe(1);
  await expect.poll(() => invocationCount(page, "send_text")).toBe(0);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("send_prepared_uploads")[0]?.args)
    )
    .toMatchObject({
      accountHomeserver: "https://harness.example.invalid",
      accountUserId: "@harness-user:example.invalid",
      accountDeviceId: "HARNESSDEVICE",
      target: { kind: "main", room_id: "!harness-room:example.invalid" },
      draftRevision: "0"
    });
  await expect(page.getByRole("dialog", { name: "Upload attachments" })).toHaveCount(0);
});

test("resize and format are chosen independently before the send action", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  const fixture = await canvasPngBuffer(page, 4, 2);
  await attachFile(page, {
    name: "screen.png",
    mimeType: "image/png",
    buffer: fixture
  });

  const dialog = page.getByRole("dialog", { name: "Upload attachments" });
  await expect(dialog).toBeVisible();
  await expect.poll(() => invocationCount(page, "stage_upload_bytes")).toBe(1);

  // Two independent compact controls, not per-variant cards, and no MIME text.
  const resize = dialog.getByRole("radiogroup", { name: "Resize" });
  const format = dialog.getByRole("radiogroup", { name: "Format" });
  await expect(resize.getByRole("radio")).toHaveCount(4);
  await expect(format.getByRole("radio")).toHaveCount(4);
  await expect(dialog.locator(".upload-variant-button")).toHaveCount(0);
  await expect(dialog).not.toContainText("image/webp");

  // Staging always starts untouched.
  await expect(resize.getByRole("radio", { name: "Original" })).toHaveAttribute(
    "aria-checked",
    "true"
  );
  await expect(format.getByRole("radio", { name: "Keep" })).toHaveAttribute(
    "aria-checked",
    "true"
  );
  await expect.poll(() => invocationCount(page, "prepared_upload_preview")).toBeGreaterThanOrEqual(1);

  // Each axis dispatches the whole pair, keeping the other axis intact.
  await resize.getByRole("radio", { name: "1/2" }).click();
  await expect
    .poll(async () =>
      page.evaluate(() =>
        window.__harness.invocationsOf("select_staged_upload_output").at(-1)?.args
      )
    )
    .toMatchObject({
      target: { kind: "main", room_id: "!harness-room:example.invalid" },
      selection: { resize: "half", format: "keep" }
    });
  await format.getByRole("radio", { name: "WebP" }).click();
  await expect
    .poll(async () =>
      page.evaluate(() =>
        window.__harness.invocationsOf("select_staged_upload_output").at(-1)?.args
      )
    )
    .toMatchObject({ selection: { resize: "half", format: "webp" } });

  // The preview viewport stays mounted throughout, and the result is summarized
  // exactly once.
  await expect(dialog.locator(".upload-preview-viewport")).toHaveCount(1);
  await expect(dialog.getByRole("status", { name: "Upload result" })).toHaveCount(1);

  const selectionCountBeforeSend = await invocationCount(page, "select_staged_upload_output");
  // Attachments are sent from the staging panel; the composer send is
  // for the message text only.
  await page.getByRole("button", { name: "Send attachments" }).click();
  await expect.poll(() => invocationCount(page, "send_prepared_uploads")).toBe(1);
  await expect.poll(() => invocationCount(page, "select_staged_upload_output")).toBe(
    selectionCountBeforeSend
  );
  await expect(dialog).toHaveCount(0);
});

test("thread pane rows expose no reply composition while room rows still do", async ({
  page
}) => {
  await gotoReadyShell(page);

  // The room timeline keeps both reply affordances.
  const roomRow = page
    .getByRole("main", { name: t("timeline.conversation") })
    .locator('[data-event-id="$seed-event:example.invalid"]')
    .first();
  await roomRow.hover();
  await expect(
    roomRow.getByRole("button", { name: t("timeline.replyToMessage") })
  ).toBeVisible();
  await expect(
    roomRow.getByRole("button", { name: t("timeline.replyInThread") })
  ).toBeVisible();

  await page.getByRole("button", { name: /2 replies/ }).click();
  await expect(page.getByText(t("panel.thread"), { exact: true })).toBeVisible();

  const threadEventId = "$thread-reply-actions:example.invalid";
  const threadKey = threadTimelineKey(
    "@harness-user:example.invalid",
    "!harness-room:example.invalid",
    "$seed-event:example.invalid"
  );
  await page.evaluate(({ key, eventId }) => {
    window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key,
          generation: 1,
          items: [
            {
              id: { Event: { event_id: eventId } },
              sender: "@thread-user:example.invalid",
              body: "Thread event without reply affordances",
              timestamp_ms: 1_800_000_000_300,
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
  }, { key: threadKey, eventId: threadEventId });

  const threadRow = page
    .locator(`aside[aria-label="Context panel"] [data-event-id="${threadEventId}"]`)
    .first();
  await expect(threadRow).toBeVisible();
  await threadRow.hover();

  // Reactions stay available in the thread pane; reply composition does not.
  await expect(threadRow.getByRole("button", { name: t("timeline.addReaction") })).toBeVisible();
  await expect(
    threadRow.getByRole("button", { name: t("timeline.replyToMessage") })
  ).toHaveCount(0);
  await expect(
    threadRow.getByRole("button", { name: t("timeline.replyInThread") })
  ).toHaveCount(0);

  // A context menu opened on a thread row must never expose the hidden action
  // through another path. The thread pane currently wires no message context
  // menu at all, so this asserts the absent menu items rather than the menu.
  await threadRow.click({ button: "right" });
  const contextMenu = page.locator(".context-menu");
  await expect(
    contextMenu.getByRole("menuitem", { name: t("timeline.replyToMessage") })
  ).toHaveCount(0);
  await expect(
    contextMenu.getByRole("menuitem", { name: t("context.openThread") })
  ).toHaveCount(0);

  // The thread composer stays in its ordinary thread-send mode.
  await expect(
    page.locator('aside[aria-label="Context panel"] .composer-reply-banner')
  ).toHaveCount(0);
});

test("thread composer delayed write is root isolated across churn", async ({
  page
}) => {
  await gotoReadyShell(page);

  await expect(page.getByRole("button", { name: /2 replies/ })).toBeVisible();
  await page.getByRole("button", { name: /2 replies/ }).click();
  await expect(page.getByText(t("panel.thread"), { exact: true })).toBeVisible();

  await page.evaluate(() => window.__harness.clearInvocations());

  const threadComposer = page.getByRole("textbox", { name: t("timeline.threadComposer") });
  const contextPanel = page.locator('aside[aria-label="Context panel"]');
  await expect(threadComposer).toBeVisible();
  const threadReplyBody = "Thread composer reply body";
  await threadComposer.fill(threadReplyBody);

  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("set_thread_composer_draft")[0]?.args)
    )
    .toMatchObject({
      accountHomeserver: "https://harness.example.invalid",
      accountUserId: "@harness-user:example.invalid",
      accountDeviceId: "HARNESSDEVICE",
      roomId: "!harness-room:example.invalid",
      rootEventId: "$seed-event:example.invalid",
      document: {
        version: 2,
        inlines: [{ kind: "text", text: threadReplyBody }]
      },
      draftRevision: "1"
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
    .toMatchObject({
      accountHomeserver: "https://harness.example.invalid",
      accountUserId: "@harness-user:example.invalid",
      accountDeviceId: "HARNESSDEVICE",
      roomId: "!harness-room:example.invalid",
      rootEventId: "$seed-event:example.invalid",
      document: {
        version: 2,
        inlines: [{ kind: "text", text: threadReplyBody }]
      },
      draftRevision: "1"
    });
  expect(await invocationCount(page, "send_text")).toBe(0);
  expect(await invocationCount(page, "send_reply")).toBe(0);

  await page.evaluate(() => window.__harness.clearInvocations());
  await page.evaluate(() => {
    const file = new File(["thread pdf bytes"], "thread-fixture.pdf", {
      type: "application/pdf"
    });
    const data = new DataTransfer();
    data.items.add(file);
    const composer = document.querySelector('aside[aria-label="Context panel"] section.composer');
    if (!composer) throw new Error("thread composer not found");
    composer.dispatchEvent(
      new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: data })
    );
  });

  await expect.poll(() => invocationCount(page, "stage_upload_bytes")).toBe(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("stage_upload_bytes")[0]?.args)
    )
    .toMatchObject({
      target: {
        kind: "thread",
        room_id: "!harness-room:example.invalid",
        root_event_id: "$seed-event:example.invalid"
      },
      items: [{ filename: "thread-fixture.pdf", mimeType: "application/pdf" }]
    });
  await expect(page.getByText("thread-fixture.pdf", { exact: true })).toBeVisible();
  // Thread attachments are sent from the thread's staging panel too.
  await contextPanel.getByRole("button", { name: "Send attachments" }).click();
  await expect.poll(() => invocationCount(page, "send_prepared_uploads")).toBe(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("send_prepared_uploads")[0]?.args)
    )
    .toMatchObject({
      accountHomeserver: "https://harness.example.invalid",
      accountUserId: "@harness-user:example.invalid",
      accountDeviceId: "HARNESSDEVICE",
      target: {
        kind: "thread",
        room_id: "!harness-room:example.invalid",
        root_event_id: "$seed-event:example.invalid"
      },
      draftRevision: "2"
    });
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
      document: {
        version: 2,
        inlines: [{ kind: "text", text: editedBody }]
      }
  });
});
