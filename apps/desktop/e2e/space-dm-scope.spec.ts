import { expect, test } from "@playwright/test";
import { t } from "../src/i18n/messages";

test("active space sidebar renders the Rust-projected DM scope", async ({ page }) => {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("complementary", { name: t("workspace.rooms") })).toBeVisible();
  const dmsSection = page.locator('[data-room-section="dms"]');

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    const item = (room_id: string, display_name: string, unread_count: number) => ({
      room_id,
      display_name,
      avatar: null,
      tags: { favourite: null, low_priority: null },
      unread_count,
      highlight_count: 0,
      notification_count: unread_count,
      display_count: unread_count,
      has_unread_content: unread_count > 0,
      is_attention_highlighted: false,
      has_unread_mention: false,
      is_muted: false
    });
    const people = [
      item("!dm-one:example.invalid", "Member 1", 1),
      item("!dm-two:example.invalid", "Member 2", 0)
    ];
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        ui: {
          ...snapshot.state.ui,
          navigation: { ...snapshot.state.ui.navigation, active_space_id: null }
        }
      },
      sidebar: {
        ...snapshot.sidebar,
        active_space_id: null,
        global_dms: people,
        dm_unread_count: 1,
        sections: { ...snapshot.sidebar.sections, people }
      }
    });
    window.__harness.pushStateUpdate();
  });

  await expect(dmsSection.getByRole("button", { name: "Member 1" })).toBeVisible();
  await expect(dmsSection.getByRole("button", { name: "Member 2" })).toBeVisible();

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    const searchRoom = {
      room_id: "!search:example.invalid",
      display_name: "matrix-sdk-search",
      avatar: null,
      tags: { favourite: null, low_priority: null },
      unread_count: 0,
      highlight_count: 0,
      notification_count: 0,
      display_count: 0,
      has_unread_content: false,
      is_attention_highlighted: false,
      has_unread_mention: false,
      is_muted: false
    };
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
              sidebar: { ...snapshot.state.domain.settings.values.sidebar, category: "rooms" }
            }
          }
        },
        ui: {
          ...snapshot.state.ui,
          navigation: {
            ...snapshot.state.ui.navigation,
            active_space_id: "!harness-space:example.invalid"
          }
        }
      },
      sidebar: {
        ...snapshot.sidebar,
        active_space_id: "!harness-space:example.invalid",
        global_dms: [],
        space_rooms: [searchRoom],
        dm_unread_count: 0,
        sections: {
          favourites: [],
          rooms: [searchRoom],
          people: [],
          low_priority: [],
          not_joined: []
        }
      }
    });
    window.__harness.pushStateUpdate();
  });

  await expect(page.getByRole("button", { name: "matrix-sdk-search" })).toBeVisible();
  await expect(dmsSection.getByRole("button", { name: "Member 1" })).toHaveCount(0);
  await expect(dmsSection.getByRole("button", { name: "Member 2" })).toHaveCount(0);
});
