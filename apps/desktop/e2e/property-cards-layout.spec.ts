import { expect, test, type Page } from "@playwright/test";

import { pseudoLocalize, t } from "../src/i18n/messages";

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

    const nameSave = page.getByRole("button", { name: t("room.saveName", {}, locale) });
    await expect(nameSave).toHaveText(t("settings.propertySave", {}, locale));
    const nameGeometry = await nameSave.evaluate((button) => {
      const box = button.getBoundingClientRect();
      const form = button.closest("form")!.getBoundingClientRect();
      return { left: box.left - form.left, right: form.right - box.right };
    });
    expect(nameGeometry.left).toBeGreaterThanOrEqual(-1);
    expect(nameGeometry.right).toBeGreaterThanOrEqual(-1);

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

/*
 * #1177: the shared access/history editor is a wide choice/detail split that
 * stacks in a narrow or short pane. Every choice and both actions must stay
 * hit-testable and keyboard reachable across locales, not merely present in
 * document order. Pseudo-accented and bidi locales stand in for expanded and
 * RTL product text.
 */
const EDITOR_PROFILES = {
  en: { lang: "en", catalog_locale: "en", pseudo_locale: "none" },
  ja: { lang: "ja", catalog_locale: "ja", pseudo_locale: "none" },
  accented: { lang: "en-XA", catalog_locale: "pseudo", pseudo_locale: "accented" },
  bidi: { lang: "ar-XB", catalog_locale: "pseudo", pseudo_locale: "bidi" }
} as const;

type EditorLocale = keyof typeof EDITOR_PROFILES;

function roomInfoAriaLabel(locale: EditorLocale): string {
  if (locale === "en") return t("room.roomInfo");
  if (locale === "ja") return t("room.roomInfo", {}, "ja");
  return pseudoLocalize("Room info", EDITOR_PROFILES[locale].pseudo_locale);
}

async function seedAccessEditor(page: Page, locale: EditorLocale) {
  await page.evaluate(
    ({ roomId, profile }) => {
      const outcome = {
        join: { messageId: "room.accessOutcomeJoinSpaceMembers", substitutions: ["Alpha"] },
        history: { messageId: "room.accessOutcomeHistoryShared" },
        encryption: { messageId: "room.accessOutcomeNotEncrypted" },
        directory: { messageId: "room.accessOutcomeDirectoryPrivate" },
        nonRetroactive: { messageId: "room.historyNonRetroactive" }
      };
      window.__harness.setCommandResponse(
        "preview_room_access",
        ({ scope, context }: { scope: unknown; context: unknown }) => ({
          scope,
          context,
          confirmed: false,
          outcome
        })
      );
      const snapshot = window.__harness.currentSnapshot();
      const withAccess = <T extends { room_id: string }>(rows: T[]): T[] =>
        rows.map((row) =>
          row.room_id === roomId
            ? {
                ...row,
                access_join_rule: "restricted" as const,
                access_restricted_conditions: "membershipOnly" as const
              }
            : row
        );
      window.__harness.setSnapshot({
        ...snapshot,
        sidebar: {
          ...snapshot.sidebar,
          space_rooms: withAccess(snapshot.sidebar.space_rooms),
          sections: {
            ...snapshot.sidebar.sections,
            rooms: withAccess(snapshot.sidebar.sections.rooms)
          }
        },
        state: {
          ...snapshot.state,
          domain: {
            ...snapshot.state.domain,
            locale_profile: { ...snapshot.state.domain.locale_profile, ...profile },
            spaces: [
              {
                space_id: "!access-space:example.invalid",
                raw_name: "Accessibility Space With A Long Synthetic Name",
                display_name: "Accessibility Space With A Long Synthetic Name",
                avatar: null,
                join_rule: "invite",
                child_room_ids: [],
                parent_side_child_room_ids: []
              }
            ],
            room_management: {
              selected_room_id: roomId,
              settings: {
                room_id: roomId,
                name: "Harness Room",
                topic: null,
                avatar_url: null,
                join_rule: "restricted",
                history_visibility: "shared",
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
              draft: {
                scope: { kind: "room", roomId },
                revision: 1,
                rule: "restricted",
                allowTargets: []
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
    { roomId: HARNESS_ROOM_ID, profile: EDITOR_PROFILES[locale] }
  );
}

const EDITOR_CASES: Array<{
  locale: EditorLocale;
  name: string;
  width: number;
  height: number;
  split: boolean;
}> = [
  { locale: "en", name: "wide", width: 1200, height: 900, split: true },
  { locale: "ja", name: "narrow", width: 560, height: 900, split: false },
  { locale: "accented", name: "wide", width: 1200, height: 900, split: true },
  { locale: "bidi", name: "short", width: 1200, height: 430, split: false }
];

for (const testCase of EDITOR_CASES) {
  test(`the access editor keeps every choice and action reachable (${testCase.locale}, ${testCase.name})`, async ({
    page
  }) => {
    await page.setViewportSize({ width: testCase.width, height: testCase.height });
    await gotoReadyShell(page);
    await seedAccessEditor(page, testCase.locale);
    await page.getByRole("button", { name: roomInfoAriaLabel(testCase.locale) }).click();

    const card = page.locator('.room-info-panel [data-setting-property="join-rule"]');
    await expect(card).toBeVisible();
    const editor = card.locator(".access-choice-detail");
    await expect(editor).toBeVisible();

    // Every choice and both actions are fully inside the viewport and hit-
    // testable at their measured centre — real geometry, not document order.
    const controls = editor.locator('input[type="radio"], input[type="checkbox"], button');
    const count = await controls.count();
    expect(count).toBeGreaterThanOrEqual(6);
    for (let index = 0; index < count; index += 1) {
      const control = controls.nth(index);
      await control.scrollIntoViewIfNeeded();
      await expect(control).toBeInViewport({ ratio: 1 });
      const hit = await control.evaluate((element) => {
        const box = element.getBoundingClientRect();
        const target = document.elementFromPoint(
          box.left + box.width / 2,
          box.top + box.height / 2
        );
        if (!target) return false;
        return (
          target === element ||
          element.contains(target) ||
          Boolean(target.closest("label")?.contains(element))
        );
      });
      expect(hit, `control ${index} hit-testable`).toBe(true);
    }

    // The agreed layout: a side-by-side split when wide, stacked when narrow or
    // short.
    const listBox = await editor.locator(".access-choice-list").boundingBox();
    const detailsBox = await editor.locator(".access-detail-panel").boundingBox();
    expect(listBox).not.toBeNull();
    expect(detailsBox).not.toBeNull();
    if (testCase.split) {
      expect(listBox!.x + listBox!.width, "list left of details").toBeLessThanOrEqual(
        detailsBox!.x + 2
      );
    } else {
      expect(listBox!.y + listBox!.height, "list above details").toBeLessThanOrEqual(
        detailsBox!.y + 2
      );
    }

    // Keyboard navigation moves within the choice group and changes selection.
    const radios = editor.locator('input[type="radio"]');
    await radios.first().scrollIntoViewIfNeeded();
    await radios.first().focus();
    await page.keyboard.press("ArrowDown");
    await expect(radios.nth(1)).toBeFocused();
    expect(await editor.locator('input[type="radio"]:checked').count()).toBe(1);
  });
}
