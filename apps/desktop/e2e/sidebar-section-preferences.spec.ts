import { expect, test, type Page } from "@playwright/test";
import type { StateUpdateEnvelope } from "../src/domain/coreEvents";
import type { DesktopSnapshot, RoomListItem, RoomListSort } from "../src/domain/types";
import { t } from "../src/i18n/messages";

// StateDelta is carried by the dedicated state-update lane in this checkout,
// not CoreEventPayload. pushStateUpdate emits to App's production consumer and
// applyAppStoreDelta; pushCoreEvent would never apply these preferences.
interface Harness {
  currentSnapshot(): DesktopSnapshot;
  pushStateUpdate(update: StateUpdateEnvelope): void;
  clearInvocations(): void;
  invocationsOf(command: string): unknown[];
  invocations(): Array<{ command: string }>;
  setCommandResponse(command: string, response: unknown): void;
}

const SPACE = "!harness-space:example.invalid";
const OTHER = "!other-space:example.invalid";
const labels = { rooms: t("roomList.categoryRooms"), dms: t("roomList.categoryDms") };
type Section = keyof typeof labels;

function row(room_id: string, display_name: string, unread: boolean): RoomListItem {
  return {
    room_id, display_name, avatar: null,
    tags: { favourite: null, low_priority: null },
    unread_count: unread ? 1 : 0, notification_count: unread ? 1 : 0,
    display_count: unread ? 1 : 0, highlight_count: 0,
    has_unread_content: unread, is_attention_highlighted: false,
    has_unread_mention: false, is_muted: false
  };
}

const roomA = row("!A-false:example.invalid", "A", false);
const roomB = row("!B-false:example.invalid", "B", false);
const roomC = row("!C-false:example.invalid", "C", true);
const dmA = row("!A-true:example.invalid", "A", false);
const dmB = row("!B-true:example.invalid", "B", false);
const dmC = row("!C-true:example.invalid", "C", true);

// Explicit expected Rust projections from state_delta/sidebar_preferences_tests.rs.
// No comparator, projection, reducer, or preference-command response lives here.
const orders: Array<{
  sort: RoomListSort;
  label: string;
  names: string[];
  rooms: RoomListItem[];
  dms: RoomListItem[];
}> = [
  { sort: { kind: "normalLocale" }, label: t("roomList.sortName"),
    names: ["A", "B", "C"], rooms: [roomA, roomB, roomC], dms: [dmA, dmB, dmC] },
  { sort: { kind: "recentFirst" }, label: t("roomList.sortRecent"),
    names: ["B", "C", "A"], rooms: [roomB, roomC, roomA], dms: [dmB, dmC, dmA] },
  { sort: { kind: "activity" }, label: t("roomList.sortAttention"),
    names: ["C", "B", "A"], rooms: [roomC, roomB, roomA], dms: [dmC, dmB, dmA] }
];

async function push(page: Page, update: StateUpdateEnvelope): Promise<void> {
  await page.evaluate((envelope) => {
    const harness = (window as unknown as { __harness: Harness }).__harness;
    if (envelope.kind === "delta" &&
        envelope.generation !== (harness.currentSnapshot().state_generation ?? 0) + 1) {
      throw new Error(`Noncontiguous sidebar fixture: next=${envelope.generation}, ` +
        `current=${harness.currentSnapshot().state_generation}; commands=` +
        harness.invocations().map(({ command }) => command).join(","));
    }
    harness.pushStateUpdate(envelope);
  }, update);
}

async function expectSection(page: Page, section: Section, collapsed: boolean, names: string[]) {
  const region = page.getByRole("region", { name: labels[section], exact: true });
  await expect(region.getByRole("button", { name: labels[section], exact: true }))
    .toHaveAttribute("aria-expanded", String(!collapsed));
  await expect(region.locator(".room-name")).toHaveText(collapsed ? [] : names);
}

async function expectSort(page: Page, section: Section, label: string) {
  const region = page.getByRole("region", { name: labels[section], exact: true });
  const trigger = region.getByRole("button", {
    name: t("roomList.sectionOptions", { section: labels[section] }), exact: true
  });
  await trigger.click();
  await expect(region.getByRole("menuitemradio", { name: label, exact: true }))
    .toHaveAttribute("aria-checked", "true");
  await expect(region.getByRole("menuitemradio", { checked: true })).toHaveCount(1);
  await trigger.click();
  await expect(trigger).toHaveAttribute("aria-expanded", "false");
}

for (const activeSpace of [null, SPACE]) {
  for (const target of ["rooms", "dms"] as const) {
    test(`${activeSpace === null ? "Home" : "Space"}: pushed ${target} preferences apply without navigation`, async ({ page }) => {
      await page.goto("/appHarness.html");
      await expect(page.getByRole("complementary", { name: t("workspace.rooms") })).toBeVisible();
      const base = await page.evaluate(() => {
        const harness = (window as unknown as { __harness: Harness }).__harness;
        // Mount/space selection schedules an empty-search close. The generic
        // mock invents a new snapshot generation even though search is closed.
        // Supply an explicit no-op Rust settlement receipt, so that unrelated
        // transport fixture cannot consume the sidebar stream's generations.
        harness.setCommandResponse("close_search", {
          protocolVersion: 1, publishedGeneration: 0
        });
        return harness.currentSnapshot();
      });
      const scope = activeSpace ?? "__home__";
      const untouched: Section = target === "rooms" ? "dms" : "rooms";
      let generation = (base.state_generation ?? 0) + 1;
      const initialPreference = { collapsed: false, sort: { kind: "activity" } as RoomListSort };
      let settings: DesktopSnapshot["state"]["domain"]["settings"] = {
        ...base.state.domain.settings,
        values: {
          ...base.state.domain.settings.values,
          sidebar: {
            ...base.state.domain.settings.values.sidebar,
            scope_preferences: { [scope]: { rooms: initialPreference, dms: initialPreference } }
          }
        }
      };
      let sidebar: DesktopSnapshot["sidebar"] = {
        ...base.sidebar,
        active_space_id: activeSpace,
        account_home: { ...base.sidebar.account_home, is_active: activeSpace === null },
        space_rail: base.sidebar.space_rail.map((space) => ({
          ...space, is_active: space.space_id === activeSpace
        })),
        rooms_collapsed: false, dms_collapsed: false,
        rooms_sort: { kind: "activity" }, dms_sort: { kind: "activity" },
        space_rooms: [roomC, roomB, roomA], global_dms: [dmC, dmB, dmA],
        space_unread_count: 1, dm_unread_count: 1,
        sections: { favourites: [], low_priority: [], not_joined: [],
          rooms: [roomC, roomB, roomA], people: [dmC, dmB, dmA] }
      };
      await push(page, {
        protocol_version: 1, kind: "snapshot", generation, reason: "settlement",
        snapshot: {
          ...base, state_generation: generation, sidebar,
          state: { ...base.state,
            domain: { ...base.state.domain, settings },
            ui: { ...base.state.ui, navigation: {
              ...base.state.ui.navigation, active_space_id: activeSpace
            } }
          }
        }
      });
      await expectSection(page, "rooms", false, ["C", "B", "A"]);
      await expectSection(page, "dms", false, ["C", "B", "A"]);
      await page.evaluate(() =>
        (window as unknown as { __harness: Harness }).__harness.clearInvocations());

      for (const order of orders) {
        // Collapse, change sort while collapsed, then expand. Every transition
        // supplies the authoritative sidebar AND settings slices, no snapshot.
        for (const [collapsed, sort, rows] of [
          [true, sidebar[`${target}_sort`]!, sidebar[target === "rooms" ? "space_rooms" : "global_dms"]],
          [true, order.sort, order[target]],
          [false, order.sort, order[target]]
        ] as const) {
          settings = { ...settings, values: { ...settings.values,
            sidebar: { ...settings.values.sidebar, scope_preferences: {
              ...settings.values.sidebar.scope_preferences,
              [scope]: { ...settings.values.sidebar.scope_preferences![scope],
                [target]: { collapsed, sort } }
            } }
          } };
          sidebar = { ...sidebar,
            [`${target}_collapsed`]: collapsed, [`${target}_sort`]: sort,
            [target === "rooms" ? "space_rooms" : "global_dms"]: rows,
            sections: { ...sidebar.sections, [target === "rooms" ? "rooms" : "people"]: rows }
          };
          await push(page, { protocol_version: 1, kind: "delta", generation: ++generation,
            changed: { sidebar, state: { domain: { settings } } } });
          await expectSection(page, target, collapsed, order.names);
          await expectSection(page, untouched, false, ["C", "B", "A"]);
          const selected = orders.find((candidate) => candidate.sort.kind === sort.kind)!;
          await expectSort(page, target, selected.label);
          await expectSort(page, untouched, t("roomList.sortAttention"));
        }
      }

      // An inactive scope emits settings only. It must not collapse or reorder
      // either active section (nor leak its selected sort into either menu).
      settings = { ...settings, values: { ...settings.values,
        sidebar: { ...settings.values.sidebar, scope_preferences: {
          ...settings.values.sidebar.scope_preferences,
          [OTHER]: {
            rooms: { collapsed: true, sort: { kind: "normalLocale" } },
            dms: { collapsed: true, sort: { kind: "recentFirst" } }
          }
        } }
      } };
      await push(page, { protocol_version: 1, kind: "delta", generation: ++generation,
        changed: { state: { domain: { settings } } } });
      for (const section of ["rooms", "dms"] as const) {
        await expectSection(page, section, false, ["C", "B", "A"]);
        await expectSort(page, section, t("roomList.sortAttention"));
      }
      // A snapshot recovery could mask a broken delta lane. No navigation or
      // preference command is needed to render any of the pushed transitions.
      const commands = await page.evaluate(() => {
        const harness = (window as unknown as { __harness: Harness }).__harness;
        return ["select_space", "select_room", "update_settings", "get_snapshot", "resync_snapshot"]
          .map((command) => [command, harness.invocationsOf(command).length]);
      });
      expect(commands).toEqual([
        ["select_space", 0], ["select_room", 0], ["update_settings", 0],
        ["get_snapshot", 0], ["resync_snapshot", 0]
      ]);
    });
  }
}
