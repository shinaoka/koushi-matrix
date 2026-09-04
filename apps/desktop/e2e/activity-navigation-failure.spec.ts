import { expect, test } from "@playwright/test";

import { t } from "../src/i18n/messages";
import { gotoReadyShell } from "./support/basicOperations";

function activitySnapshot(snapshot: any, threadRootEventId: string | null = null) {
  return {
    ...snapshot,
    state: {
      ...snapshot.state,
      domain: {
        ...snapshot.state.domain,
        activity: {
          kind: "open",
          active_tab: "recent",
          recent: {
            rows: [
              {
                kind: "event",
                room_id: "!activity-room:example.invalid",
                event_id: "$activity-event:example.invalid",
                thread_root_event_id: threadRootEventId,
                room_label: "Activity room",
                context_label: "Room · Activity room",
                sender_label: "Activity sender",
                sender_avatar: null,
                preview: "Activity preview",
                timestamp_ms: 1_800_000_000_000,
                unread: false,
                highlight: false
              }
            ],
            next_batch: null,
            resolution: { kind: "idle" }
          },
          unread: { rows: [], next_batch: null, resolution: { kind: "idle" } },
          mark_read: { kind: "idle" }
        }
      }
    }
  };
}

test("Activity event navigation surfaces a rejected focused open", async ({ page }) => {
  await gotoReadyShell(page);
  const snapshot = await page.evaluate(() => window.__harness.currentSnapshot());
  const nextActivitySnapshot = activitySnapshot(snapshot);
  await page.evaluate((next) => {
    window.__harness.setCommandResponse("open_activity", () => {
      window.__harness.setSnapshot(next);
      return next;
    });
    window.__harness.setCommandResponse("open_activity_event", () => {
      throw new Error("synthetic navigation rejection");
    });
  }, nextActivitySnapshot);

  await page.getByRole("navigation", { name: t("workspace.workspaces") })
    .getByRole("button", { name: /^Home/ })
    .click();
  await expect(page.getByRole("main", { name: "Activity" })).toBeVisible();
  await page.getByRole("button", { name: "Open activity item Activity room" }).click();

  await expect(page.getByRole("alert")).toContainText(t("navigation.failed"));
  await expect(page.getByRole("main", { name: "Activity" })).toBeVisible();
});

test("thread Activity navigation surfaces a rejected thread open", async ({ page }) => {
  await gotoReadyShell(page);
  const snapshot = await page.evaluate(() => window.__harness.currentSnapshot());
  const nextActivitySnapshot = activitySnapshot(snapshot, "$thread-root:example.invalid");
  await page.evaluate((next) => {
    window.__harness.setCommandResponse("open_activity", () => {
      window.__harness.setSnapshot(next);
      return next;
    });
    window.__harness.setCommandResponse("select_room", () => {
      const current = window.__harness.currentSnapshot();
      return {
        ...current,
        state: {
          ...current.state,
          ui: {
            ...current.state.ui,
            navigation: {
              ...current.state.ui.navigation,
              active_room_id: "!activity-room:example.invalid"
            },
            timeline: {
              ...current.state.ui.timeline,
              room_id: "!activity-room:example.invalid"
            }
          }
        }
      };
    });
    window.__harness.setCommandResponse("open_thread", () => {
      throw new Error("synthetic thread rejection");
    });
  }, nextActivitySnapshot);

  await page.getByRole("navigation", { name: t("workspace.workspaces") })
    .getByRole("button", { name: /^Home/ })
    .click();
  await expect(page.getByRole("main", { name: "Activity" })).toBeVisible();
  await page.getByRole("button", { name: "Open activity item Activity room" }).click();

  await expect(page.getByRole("alert")).toContainText(t("navigation.failed"));
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
});
