import { expect, test, type Page } from "@playwright/test";

import { HARNESS_ROOM_ID, gotoReadyShell } from "./support/basicOperations";

// Issue #1008: each Room info property is one card holding its value, its
// change control and its result. At a narrow panel and in Japanese, the card
// must wrap its actions under the value instead of clipping or overflowing.

const LONG_TOPIC =
  "合成トピック：この部屋では週次の進捗、長い説明文、改行を含まない日本語の文章が表示幅に収まるかを確認します。" +
  "Synthetic topic text that keeps going without spaces_to_force_wrapping_behaviour_checks";

async function seedRoomManagement(page: Page, locale: "en" | "ja") {
  await page.evaluate(
    ({ roomId, locale, topic }) => {
      const snapshot = window.__harness.currentSnapshot();
      window.__harness.setSnapshot({
        ...snapshot,
        state: {
          ...snapshot.state,
          domain: {
            ...snapshot.state.domain,
            locale_profile: {
              ...snapshot.state.domain.locale_profile,
              lang: locale,
              catalog_locale: locale
            },
            room_management: {
              selected_room_id: roomId,
              settings: {
                room_id: roomId,
                name: "Harness Room",
                topic,
                avatar_url: "mxc://example.invalid/synthetic-avatar-with-a-long-media-identifier",
                join_rule: "knockRestricted",
                history_visibility: "worldReadable",
                permissions: {
                  can_edit_settings: true,
                  can_change_join_rule: true,
                  can_edit_roles: true,
                  can_invite: true,
                  can_kick: true,
                  can_ban: false,
                  can_unban: false
                },
                members: []
              },
              operation: { kind: "idle" }
            }
          }
        }
      });
      window.__harness.setCommandResponse("load_room_settings", () =>
        window.__harness.currentSnapshot()
      );
      window.__harness.pushStateUpdate();
    },
    { roomId: HARNESS_ROOM_ID, locale, topic: LONG_TOPIC }
  );
}

async function cardOverflow(page: Page) {
  return page.evaluate(() =>
    Array.from(document.querySelectorAll<HTMLElement>(".room-info-panel [data-setting-property]")).map(
      (card) => {
        const box = card.getBoundingClientRect();
        const escaped = Array.from(card.querySelectorAll<HTMLElement>("button, select, textarea, input"))
          .filter((control) => {
            const inner = control.getBoundingClientRect();
            return inner.left < box.left - 1 || inner.right > box.right + 1;
          })
          .map((control) => control.getAttribute("aria-label") ?? control.tagName);
        return {
          property: card.dataset.settingProperty,
          overflow: card.scrollWidth - card.clientWidth,
          escaped
        };
      }
    )
  );
}

for (const locale of ["en", "ja"] as const) {
  test(`Room info property cards fit a narrow panel in ${locale}`, async ({ page }) => {
    await page.setViewportSize({ width: 900, height: 900 });
    await gotoReadyShell(page);
    await seedRoomManagement(page, locale);
    await page.locator('button[aria-label="Room info"], button[aria-label="ルーム情報"]').first().click();

    const topic = page.locator('.room-info-panel [data-setting-property="topic"]');
    await expect(topic).toBeVisible();
    await expect(topic.locator(".settings-property-value")).toContainText("合成トピック");
    await expect(topic.locator("h4")).toHaveText(locale === "ja" ? "トピック" : "Topic");

    const cards = await cardOverflow(page);
    expect(cards.map((card) => card.property)).toEqual([
      "topic",
      "avatar",
      "join-rule",
      "history-visibility"
    ]);
    for (const card of cards) {
      expect(card.overflow, card.property).toBeLessThanOrEqual(1);
      expect(card.escaped, card.property).toEqual([]);
    }

    // The open editor fits the same card.
    await topic.locator("button").first().click();
    await expect(topic.locator("textarea")).toBeVisible();
    for (const card of await cardOverflow(page)) {
      expect(card.overflow, card.property).toBeLessThanOrEqual(1);
      expect(card.escaped, card.property).toEqual([]);
    }
  });
}
