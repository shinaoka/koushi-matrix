import { expect, test, type Page } from "@playwright/test";

import { roomTimelineKey } from "../src/domain/coreEvents";
import { pseudoLocalize, t } from "../src/i18n/messages";
import {
  HARNESS_ROOM_ID,
  gotoReadyShell,
  invocationCount,
  seedTimelineItems
} from "./support/basicOperations";

async function gotoSignedOutAuth(page: Page): Promise<void> {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          session: { kind: "signedOut" },
          auth: { kind: "unknown" },
          sync: "stopped",
          rooms: [],
          spaces: [],
          invites: [],
          room_notification_settings: {},
          room_interactions: {}
        },
        ui: {
          ...snapshot.state.ui,
          navigation: { active_space_id: null, active_room_id: null },
          timeline: {
            room_id: null,
            is_subscribed: false,
            is_paginating_backwards: false,
            composer: {
              accepted_submission_ids: [],
              pending_transaction_id: null,
              draft: "",
              document: { version: 2, inlines: [] },
              draft_revision: "0",
              last_accepted_clear_revision: "0",
              mode: "Plain"
            },
            submission_registry: {
              accepted_submission_ids: [],
              settled_submission_ids: []
            },
            scheduled_send_capability: "unknown",
            scheduled_sends: [],
            staged_uploads: [],
            media_gallery: [],
            media_downloads: {},
            continuity: { kind: "unknown" }
          }
        }
      },
      sidebar: {
        active_space_id: null,
        account_home: {
          display_name: "Home",
          unread_count: 0,
          highlight_count: 0,
          invite_count: 0,
          attention_count: 0,
          is_active: true
        },
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
    window.__harness.pushStateUpdate();
    window.__harness.clearInvocations();
  });
  await expect(page.getByTestId("auth-screen")).toBeVisible();
}

test("SSO start reports authorization and native browser outcomes without a window.open fallback", async ({
  page
}) => {
  await gotoSignedOutAuth(page);
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          auth: {
            kind: "ready",
            homeserver: "https://matrix.org",
            flows: [
              {
                kind: "sso",
                delegated_oidc_compatibility: false,
                display_name: null
              }
            ],
            delegated: { registration_url: null }
          }
        }
      }
    });
    (window as typeof window & { __oidcWindowOpenCalls: number }).__oidcWindowOpenCalls = 0;
    window.open = () => {
      (window as typeof window & { __oidcWindowOpenCalls: number }).__oidcWindowOpenCalls += 1;
      return null;
    };
    window.__harness.setCommandResponse("start_oidc_login", {
      outcome: "invalid_authorization_url"
    });
    window.__harness.pushStateUpdate();
  });

  const startSso = page.getByRole("button", { name: t("auth.flowSso") });
  await startSso.click();
  await expect(page.getByRole("alert")).toHaveText(t("auth.ssoInvalidAuthorizationUrl"));

  await page.evaluate(() => {
    window.__harness.setCommandResponse("start_oidc_login", {
      outcome: "browser_launch_failed"
    });
  });
  await startSso.click();
  await expect(page.getByRole("alert")).toHaveText(t("auth.ssoBrowserLaunchFailed"));

  await page.evaluate(() => {
    window.__harness.setCommandResponse("start_oidc_login", () => {
      throw new Error("authorization creation failed");
    });
  });
  await startSso.click();
  await expect(page.getByRole("alert")).toHaveText(t("auth.ssoAuthorizationFailed"));

  await page.evaluate(() => {
    window.__harness.setCommandResponse("start_oidc_login", { outcome: "launched" });
  });
  await startSso.click();
  await expect(page.getByRole("alert")).toHaveCount(0);
  expect(
    await page.evaluate(
      () => (window as typeof window & { __oidcWindowOpenCalls: number }).__oidcWindowOpenCalls
    )
  ).toBe(0);
});

test("auth form defaults to matrix.org and submits custom ports in the homeserver URL field", async ({
  page
}) => {
  await gotoSignedOutAuth(page);

  const homeserverInput = page.locator('input[name="homeserver"]');
  await expect(homeserverInput).toHaveValue("https://matrix.org");

  await homeserverInput.fill("https://example.org:8448");
  await page.getByRole("textbox", { name: t("auth.username") }).fill("alice");
  await page.getByLabel(t("auth.password")).fill("synthetic-password");
  await page.getByRole("textbox", { name: t("auth.deviceName") }).fill("Koushi Test Device");
  await page.getByRole("button", { name: t("auth.continue") }).click();

  await expect.poll(() => invocationCount(page, "submit_login")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("submit_login")[0]?.args))
    .toEqual({
      homeserver: "https://example.org:8448",
      username: "alice",
      password: "[REDACTED]",
      deviceDisplayName: "Koushi Test Device",
      platform: "linux"
    });
  await expect(page.locator('input[name="port"]')).toHaveCount(0);
});

test("Rust-owned locale profile applies root lang and dir", async ({ page }) => {
  await gotoReadyShell(page);

  await page.evaluate(() => {
    const snapshot = window.__harness.replyModeSnapshot();
    snapshot.state.domain.locale_profile = {
      lang: "ar-XB",
      dir: "rtl",
      catalog_locale: "pseudo",
      pseudo_locale: "bidi",
      platform: "linux",
      modifier_labels: { primary: "Ctrl" }
    };
    window.__harness.setSnapshot(snapshot);
    window.__harness.pushStateUpdate();
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
      ...snapshot.state.domain.rooms[0],
	      room_id: index === roomNames.length - 1 ? snapshot.state.domain.rooms[0].room_id : `!cjk-order-${index}:example.invalid`,
	      display_name: displayName,
	      display_label: displayName,
	      original_display_label: displayName,
	      conversation_activity: {
	        timestamp_ms: 1_800_000_010_000 - index,
	        source: "message"
	      }
	    }));
    const sidebarRooms = cjkRooms.map((room) => ({
      room_id: room.room_id,
      display_name: room.display_name,
      avatar: room.avatar,
      tags: room.tags,
      unread_count: room.unread_count,
      highlight_count: room.highlight_count
    }));
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          locale_profile: {
            lang: "ja",
            dir: "ltr",
            catalog_locale: "ja",
            pseudo_locale: "none",
            platform: "linux",
            modifier_labels: { primary: "Ctrl" }
          },
          cjk_text_policy: {
            ...snapshot.state.domain.cjk_text_policy,
            japanese_catalog: {
              catalog_locale: "ja",
              complete: true,
              missing_message_ids: []
            }
          },
          rooms: cjkRooms
        },
        ui: {
          ...snapshot.state.ui,
          room_list: {
            ...snapshot.state.ui.room_list,
            active_filter: { kind: "rooms" },
            items: cjkRooms.map((room) => ({ kind: "room" as const, room_id: room.room_id }))
          }
        }
      },
      sidebar: {
        ...snapshot.sidebar,
        account_home: {
          ...snapshot.sidebar.account_home,
          display_name: workspaceName
        },
        space_rooms: sidebarRooms,
        sections: { ...snapshot.sidebar.sections, rooms: sidebarRooms }
      }
    });
    window.__harness.pushStateUpdate();
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
            domain: {
              ...next.state.domain,
              rooms: next.state.domain.rooms.map((room) =>
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
                scope: "currentRoom",
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
    page.locator(".channel-actions").getByRole("button", { name: "メンバー", exact: true })
  ).toBeVisible();
  await expect(
    page
      .locator(".channel-actions")
      .getByRole("button", { name: "メディアギャラリーを開く", exact: true })
  ).toBeVisible();
  await expect(
    page.locator(".channel-actions").getByRole("button", { name: "ルーム情報", exact: true })
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "送信", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Create room", exact: true })).toHaveCount(0);
  await expect(
    page.locator(".channel-actions").getByRole("button", { name: "Threads", exact: true })
  ).toHaveCount(0);
  await page.getByRole("button", { name: /^ルーム、/ }).click();
  await page.getByRole("button", { name: /^(アクティブ|Active)$/ }).click();
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

  const searchInput = page.getByRole("textbox", { name: /^(検索|Search)$/ });
  await searchInput.fill("ABC123");
  await searchInput.press("Enter");
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
    snapshot.state.domain.locale_profile = {
      lang: "ar-XB",
      dir: "rtl",
      catalog_locale: "pseudo",
      pseudo_locale: "bidi",
      platform: "linux",
      modifier_labels: { primary: "Ctrl" }
    };
    snapshot.state.domain.rooms[0].display_name = roomName;
    snapshot.state.domain.rooms[0].display_label = roomName;
    snapshot.state.domain.rooms[0].original_display_label = roomName;
    snapshot.sidebar.space_rooms[0].display_name = roomName;
    snapshot.state.domain.spaces[0].display_name = "日本語 Space العربية";
    snapshot.sidebar.space_rail[0].display_name = "日本語 Space العربية";
    window.__harness.setSnapshot(snapshot);
    window.__harness.pushStateUpdate();
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
      ...base.state.domain.settings.values,
      typography: { font: "inter" as const, emoji: "twemojiColr" as const }
    };
    window.__harness.setSnapshot({
      ...base,
      state: {
        ...base.state,
        domain: {
          ...base.state.domain,
          settings: {
            ...base.state.domain.settings,
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
      }
    });
    window.__harness.pushStateUpdate();
  });

  await expect
    .poll(() => page.evaluate(() => document.documentElement.dataset.uiFont))
    .toBe("inter");
  await expect
    .poll(() => page.evaluate(() => document.documentElement.dataset.emojiFont))
    .toBe("twemojiColr");

  const typography = await page.evaluate(async () => {
    await Promise.allSettled([
      document.fonts.load('14px "Inter"', "English"),
      document.fonts.load('14px "Twemoji"', "🐶👍"),
      document.fonts.ready
    ]);
    const rootStyle = getComputedStyle(document.documentElement);
    const messageBody = document.querySelector(".message-body");
    const reactionKey = document.querySelector(".reaction-pill-key");
    return {
      fontUi: rootStyle.getPropertyValue("--font-ui"),
      fontEmoji: rootStyle.getPropertyValue("--font-emoji"),
      messageFont: messageBody ? getComputedStyle(messageBody).fontFamily : "",
      reactionFont: reactionKey ? getComputedStyle(reactionKey).fontFamily : "",
      fontResources: performance.getEntriesByType("resource")
        .map((entry) => entry.name)
        .filter((name) => /woff|ttf/.test(name))
    };
  });

  expect(typography.fontUi).toContain("Inter");
  expect(typography.fontEmoji).toContain("Twemoji");
  expect(typography.fontResources.some((url) => url.includes("inter-latin-400-normal"))).toBe(true);
  expect(typography.fontResources.some((url) => url.includes("twemoji.woff2"))).toBe(true);
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

test("timeline auto-load setting dispatches a Rust-owned update_settings patch", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.getByRole("button", { name: "User settings" }).click();
  await expect(page.getByRole("heading", { name: t("settings.timeline") })).toBeVisible();

  const autoLoad = page.getByRole("switch", { name: t("settings.autoLoadOlderMessages") });
  await expect(autoLoad).toHaveAttribute("aria-checked", "true");
  await autoLoad.click();

  await expect.poll(() => invocationCount(page, "update_settings")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("update_settings")[0]?.args)
    )
    .toEqual({
      patch: {
        timeline: {
          auto_load_older_messages: false,
          thread_root_order: { kind: "rootEvent" }
        }
      }
    });
  await expect(autoLoad).toHaveAttribute("aria-checked", "false");
});

test("Compact density visually groups consecutive messages from the same sender", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("koushi.displayDensity.v1", "compact");
  });
  await gotoReadyShell(page);
  await seedTimelineItems(
    page,
    ["first", "second"].map((suffix, index) => ({
      id: { Event: { event_id: `$compact-${suffix}:example.invalid` } },
      sender: "@harness-user:example.invalid",
      body: `Compact ${suffix}`,
      timestamp_ms: 1_800_000_000_000 + index,
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
    }))
  );

  const first = page.locator('[data-event-id="$compact-first:example.invalid"]');
  const second = page.locator('[data-event-id="$compact-second:example.invalid"]');
  await expect(page.locator('.desktop[data-density="compact"]')).toBeVisible();
  await expect
    .poll(() => page.evaluate(() => localStorage.getItem("koushi.displayDensity.v1")))
    .toBeNull();
  await expect(first.locator(".avatar")).toBeVisible();
  await expect(second).toHaveClass(/is-continuation/);
  await expect(second.locator(".avatar")).toBeHidden();
  await expect(second.locator(".sender")).toHaveCount(1);
  await expect
    .poll(() => second.locator(".sender").evaluate((element) => getComputedStyle(element).position))
    .toBe("absolute");
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
          hide_redacted: true,
          url_previews_enabled: true,
          encrypted_url_previews_enabled: true
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
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          settings: {
            ...snapshot.state.domain.settings,
            values: {
              ...snapshot.state.domain.settings.values,
              display: {
                ...snapshot.state.domain.settings.values.display,
                hide_redacted: false
              }
            }
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
  });
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
          url_previews_enabled: true,
          encrypted_url_previews_enabled: true
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
        source_ref:
          "data:image/gif;base64,R0lGODlhAQABAPAAAP///wAAACH5BAAAAAAALAAAAAABAAEAAAICRAEAOw==",
        width: 1,
        height: 1,
        mime_type: "image/gif"
      }
    };
    const next = {
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          profile: {
            ...snapshot.state.domain.profile,
            users: {
              ...snapshot.state.domain.profile.users,
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
          rooms: snapshot.state.domain.rooms.map((room) =>
            room.room_id === "!harness-room:example.invalid" ? { ...room, avatar } : room
          )
        }
      },
      sidebar: {
        ...snapshot.sidebar,
        space_rooms: snapshot.sidebar.space_rooms.map((room) =>
          room.room_id === "!harness-room:example.invalid" ? { ...room, avatar } : room
        ),
        sections: {
          ...snapshot.sidebar.sections,
          rooms: snapshot.sidebar.sections.rooms.map((room) =>
            room.room_id === "!harness-room:example.invalid" ? { ...room, avatar } : room
          )
        }
      }
    };
    window.__harness.setSnapshot(next);
    window.__harness.pushStateUpdate();
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

test("unsafe account-management destination is hidden in User Settings", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          account_management_url: "javascript:alert(1)"
        }
      }
    });
    window.__harness.pushStateUpdate();
  });
  await page.getByRole("button", { name: "User settings", exact: true }).click();
  await page.getByRole("button", { name: "Session", exact: true }).click();

  await expect(page.getByRole("button", { name: "Manage account & devices" })).toHaveCount(0);
});

test("remote device management is delegated to the active server", async ({ page }) => {
  await gotoReadyShell(page);
  await page.getByRole("button", { name: "User settings", exact: true }).click();
  await page.getByRole("button", { name: "Session", exact: true }).click();

  await expect(page.getByRole("button", { name: "Manage account & devices" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Sessions", exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Rename", exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Sign out all other sessions" })).toHaveCount(0);
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

test("URL previews global toggle invokes update_settings", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    window.__harness.setCommandResponse("update_settings", () => window.__harness.currentSnapshot());
    window.__harness.clearInvocations();
  });

  await page.getByRole("button", { name: t("workspace.userSettings") }).click();

  const toggle = page.getByRole("switch", { name: t("settings.urlPreviewsUnencrypted") });
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
          hide_redacted: true,
          url_previews_enabled: false,
          encrypted_url_previews_enabled: true
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

test("a stale schema-v1 snapshot fails closed to the recovery screen instead of crashing", async ({
  page
}) => {
  await gotoReadyShell(page);
  // #87 Phase 4 IPC contract guard: a stale Rust build can return the prior schema version.
  // Keep the envelope structurally projectable so the test deterministically exercises the
  // App setSnapshot version gate rather than the transport's malformed-envelope recovery path.
  await page.evaluate(() => {
    const current = window.__harness.currentSnapshot();
    const generation = (current.state_generation ?? 0) + 10_000;
    const staleV1Snapshot = {
      ...current,
      state_generation: generation,
      state: { ...current.state, schema_version: 1 }
    };
    window.__harness.pushStateUpdate({
      protocol_version: 1,
      kind: "snapshot",
      generation,
      snapshot: staleV1Snapshot as never,
      reason: "settlement"
    });
  });

  await Promise.all([
    expect(page.getByRole("alert")).toContainText(t("app.versionMismatch.title")),
    expect(page.getByRole("alert")).toContainText(t("app.versionMismatch.detail")),
    expect(page.getByRole("main", { name: "Conversation timeline" })).toBeHidden()
  ]);
});

test("a future snapshot schema_version is also rejected to the recovery screen", async ({
  page
}) => {
  await gotoReadyShell(page);
  // Even when the shape still looks v2, a schema_version the renderer does not recognise (a
  // newer Rust build) must fail closed rather than render against an unverified contract.
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    const generation = (snapshot.state_generation ?? 0) + 1;
    window.__harness.pushStateUpdate({
      protocol_version: 1,
      kind: "snapshot",
      generation,
      snapshot: {
        ...snapshot,
        state_generation: generation,
        state: { ...snapshot.state, schema_version: 999 }
      },
      reason: "settlement"
    });
  });

  await Promise.all([
    expect(page.getByRole("alert")).toContainText(t("app.versionMismatch.title")),
    expect(page.getByRole("main", { name: "Conversation timeline" })).toBeHidden()
  ]);
});
