import { expect, test } from "@playwright/test";
import { focusedTimelineKey } from "../src/domain/coreEvents";
import { t } from "../src/i18n/messages";
import { gotoReadyShell, HARNESS_ACCOUNT_KEY, HARNESS_ROOM_ID } from "./support/basicOperations";

const TARGET = "$activity-shell-target:example.invalid";

test("Activity centers its target without scrolling the shell with a long room list", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await gotoReadyShell(page);
  await page.evaluate(({ roomId, eventId }) => {
    const initial = window.__harness.currentSnapshot();
    const rooms = Array.from({ length: 35 }, (_, index) => ({
      ...initial.sidebar.sections.rooms[0],
      room_id: `!synthetic-room-${index}:example.invalid`,
      display_name: `Synthetic room ${index}`
    }));
    window.__harness.setSnapshot({ ...initial,
      sidebar: { ...initial.sidebar, sections: { ...initial.sidebar.sections, rooms } }
    });
    window.__harness.pushStateUpdate();
    window.__harness.setCommandResponse("open_activity", () => {
      const current = window.__harness.currentSnapshot();
      const next = { ...current, state: { ...current.state, domain: { ...current.state.domain,
        activity: { kind: "open", active_tab: "recent",
          recent: { rows: [{ kind: "event", room_id: roomId, event_id: eventId,
            thread_root_event_id: null, room_label: "Synthetic target room", context_label: "Room",
            sender_label: "Member 1", sender_avatar: null, preview: "Synthetic target",
            timestamp_ms: 1_800_000_000_000, unread: false, highlight: false
          }], next_batch: null, resolution: { kind: "idle" } },
          unread: { rows: [], next_batch: null, resolution: { kind: "idle" } },
          mark_read: { kind: "idle" }
        }
      } } };
      window.__harness.setSnapshot(next);
      return next;
    });
    window.__harness.setCommandResponse("open_activity_event", () => {
      const current = window.__harness.currentSnapshot();
      const next = { ...current, state: { ...current.state, ui: { ...current.state.ui,
        navigation: { ...current.state.ui.navigation, active_room_id: roomId,
          main_timeline_anchor: { event_id: eventId },
          event_navigation: { kind: "anchored", generation: 1, source: "activity" }
        },
        timeline: { ...current.state.ui.timeline, room_id: roomId, is_subscribed: true },
        focused_context: { kind: "opening", room_id: roomId, event_id: eventId },
        thread: { kind: "closed" }
      } } };
      window.__harness.setSnapshot(next);
      return next;
    });
  }, { roomId: HARNESS_ROOM_ID, eventId: TARGET });

  await expect(page.locator(".room-item .sr-only")).toHaveCount(35);
  await page.getByRole("navigation", { name: t("workspace.workspaces") })
    .getByRole("button", { name: /^Home/ }).click();
  await page.getByRole("button", { name: "Open activity item Synthetic target room" }).click();
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  await page.evaluate(async ({ key, eventId }) => {
    await window.__harness.pushCoreEvent({ kind: "Timeline", event: { InitialItems: {
      request_id: null, key, generation: 1,
      items: Array.from({ length: 35 }, (_, index) => ({
        id: { Event: { event_id: index === 34 ? eventId : `$synthetic-context-${index}:example.invalid` } },
        sender: "@member-1:example.invalid", body: `Synthetic context ${index}\nSecond line\nThird line`,
        timestamp_ms: 1_800_000_000_000 + index,
        in_reply_to_event_id: null, thread_root: null, thread_summary: null,
        reactions: [], can_react: true, is_redacted: false, is_hidden: false,
        can_redact: false, is_edited: false, can_edit: false
      }))
    } }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);
  }, { key: focusedTimelineKey(HARNESS_ACCOUNT_KEY, HARNESS_ROOM_ID, TARGET), eventId: TARGET });

  await expect(page.locator(`[data-item-id="${TARGET}"]`)).toBeInViewport();
  await expect.poll(() => page.locator(".timeline-view").first()
    .evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
  // The sidebar overflows internally; its descriptions must not enlarge the shell.
  expect(await page.locator(".sidebar-scroll").evaluate((element) =>
    element.scrollHeight > element.clientHeight)).toBe(true);
  expect(await page.locator(".desktop").evaluate((element) => ({
    top: element.scrollTop, height: element.scrollHeight, viewport: element.clientHeight
  }))).toEqual({ top: 0, height: 800, viewport: 800 });
  expect(await page.locator(".titlebar").evaluate((element) =>
    element.getBoundingClientRect().top)).toBe(0);
});
