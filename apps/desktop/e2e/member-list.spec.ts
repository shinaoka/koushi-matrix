/**
 * Headless spec: member-list entry points (#81).
 *
 * Proves that every entry point that leads to the member list actually
 * opens the correct panel and, where a load_room_settings command is
 * needed, dispatches it — for both room and space contexts.
 *
 * Entry points under test:
 *  1. Room: room-header member-count pill → Room info panel opens.
 *  2. Room: Room info "People" entry → scrolls to / shows the members section.
 *  3. Room: Room info members section — shows Rust-owned member after load_room_settings.
 *  4. Space: workspace space-info-settings button → Space info panel opens.
 *  5. Space: Space info "Members" entry → calls load_room_settings and shows members.
 */

import { expect, test } from "@playwright/test";
import { t } from "../src/i18n/messages";

const HARNESS_ROOM_ID = "!harness-room:example.invalid";
const HARNESS_SPACE_ID = "!harness-space:example.invalid";
const HARNESS_MEMBERS = [
  { label: "Harness Ada", userId: "@harness-ada:example.invalid" },
  { label: "Harness Grace", userId: "@harness-grace:example.invalid" },
  { label: "Harness Linus", userId: "@harness-linus:example.invalid" }
] as const;

async function gotoReadyShell(page: import("@playwright/test").Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: t("timeline.conversation") })).toBeVisible();
}

/** Count how many times a command has been invoked in the harness. */
async function invocationCount(
  page: import("@playwright/test").Page,
  command: string
): Promise<number> {
  return page.evaluate(
    (cmd) => (window as unknown as { __harness: { invocationsOf(c: string): unknown[] } }).__harness.invocationsOf(cmd).length,
    command
  );
}

/** Make the active space the harness space so space-info controls are visible. */
async function activateSpace(page: import("@playwright/test").Page): Promise<void> {
  await page.evaluate(() => {
    const snap = (window as unknown as { __harness: { currentSnapshot(): unknown; setSnapshot(s: unknown): void } }).__harness.currentSnapshot() as Record<string, unknown>;
    const state = snap.state as Record<string, unknown>;
    const ui = (state.ui ?? {}) as Record<string, unknown>;
    const nav = (ui.navigation ?? {}) as Record<string, unknown>;
    const sidebar = snap.sidebar as Record<string, unknown>;
    (window as unknown as { __harness: { setSnapshot(s: unknown): void } }).__harness.setSnapshot({
      ...snap,
      state: {
        ...state,
        ui: {
          ...ui,
          navigation: {
            ...nav,
            active_space_id: "!harness-space:example.invalid"
          }
        }
      },
      sidebar: {
        ...sidebar,
        active_space_id: "!harness-space:example.invalid"
      }
    });
  });
}

// ─────────────────────────────────────────────────────────────
//  ROOM entry points
// ─────────────────────────────────────────────────────────────

test("room header member pill opens Room info panel", async ({ page }) => {
  await gotoReadyShell(page);

  const pill = page
    .locator(".channel-actions")
    .getByRole("button", { name: t("room.members") });
  await expect(pill).toBeVisible();

  await pill.click();

  await expect(page.getByText(t("panel.roomInfo"), { exact: true })).toBeVisible();
});

test("room header member pill shows the Rust-owned joined_members count", async ({ page }) => {
  await gotoReadyShell(page);

  // The harness seeds joined_members: 8 on the room
  const pill = page
    .locator(".channel-actions")
    .getByRole("button", { name: t("room.members") });
  await expect(pill).toContainText("8");
});

test("Room info panel dispatches load_room_settings for the active room", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() =>
    (window as unknown as { __harness: { clearInvocations(): void } }).__harness.clearInvocations()
  );

  await page.getByRole("button", { name: t("room.roomInfo") }).click();
  await expect(page.getByText(t("panel.roomInfo"), { exact: true })).toBeVisible();

  // load_room_settings should have been dispatched for the harness room
  await expect
    .poll(() => invocationCount(page, "load_room_settings"))
    .toBeGreaterThanOrEqual(1);

  const args = await page.evaluate(
    () =>
      (window as unknown as {
        __harness: { invocationsOf(c: string): { args: unknown }[] };
      }).__harness.invocationsOf("load_room_settings")[0]?.args
  );
  expect((args as { roomId: string }).roomId).toBe(HARNESS_ROOM_ID);
});

test("Room info members section shows the Rust-owned member after settings load", async ({
  page
}) => {
  await gotoReadyShell(page);

  await page.getByRole("button", { name: t("room.roomInfo") }).click();
  await expect(page.getByText(t("panel.roomInfo"), { exact: true })).toBeVisible();

  // The harness mock returns several members so the UI cannot pass by showing
  // only the first row or a summarized count.
  const membersSection = page.getByRole("region", { name: t("room.members") });
  await expect(membersSection).toBeVisible();
  for (const member of HARNESS_MEMBERS) {
    await expect(membersSection).toContainText(member.label);
    await expect(membersSection).toContainText(member.userId);
    await expect(
      membersSection.getByRole("button", {
        name: t("room.messageMember", { name: member.label })
      })
    ).toBeVisible();
  }
});

test("Room info member rows can start DMs for any listed member", async ({ page }) => {
  await gotoReadyShell(page);

  await page.getByRole("button", { name: t("room.roomInfo") }).click();
  await expect(page.getByText(t("panel.roomInfo"), { exact: true })).toBeVisible();
  await page.evaluate(() =>
    (window as unknown as { __harness: { clearInvocations(): void } }).__harness.clearInvocations()
  );

  const membersSection = page.getByRole("region", { name: t("room.members") });
  await membersSection
    .getByRole("button", {
      name: t("room.messageMember", { name: HARNESS_MEMBERS[1].label })
    })
    .click();

  await expect.poll(() => invocationCount(page, "start_direct_message")).toBe(1);
  const args = await page.evaluate(
    () =>
      (window as unknown as {
        __harness: { invocationsOf(c: string): { args: unknown }[] };
      }).__harness.invocationsOf("start_direct_message")[0]?.args
  );
  expect((args as { userId: string }).userId).toBe(HARNESS_MEMBERS[1].userId);
});

test("Room info 'People' entry scrolls to the members section", async ({ page }) => {
  await gotoReadyShell(page);

  await page.getByRole("button", { name: t("room.roomInfo") }).click();
  await expect(page.getByText(t("panel.roomInfo"), { exact: true })).toBeVisible();

  const peopleButton = page.getByRole("button", { name: t("room.people"), exact: true });
  await expect(peopleButton).toBeEnabled();
  await peopleButton.click();

  // After clicking People, the members heading should be in view
  await expect(page.getByRole("heading", { name: t("room.members") })).toBeVisible();
});

// ─────────────────────────────────────────────────────────────
//  SPACE entry points
// ─────────────────────────────────────────────────────────────

test("space info-settings button opens the Space info panel", async ({ page }) => {
  await gotoReadyShell(page);
  await activateSpace(page);

  const spaceInfoButton = page.getByRole("button", {
    name: t("workspace.spaceInfoSettings")
  });
  await expect(spaceInfoButton).toBeVisible();
  await spaceInfoButton.click();

  await expect(page.getByText(t("panel.spaceInfo"), { exact: true })).toBeVisible();
});

test("Space info 'Members' entry dispatches load_room_settings for the space", async ({
  page
}) => {
  await gotoReadyShell(page);
  await activateSpace(page);
  await page.evaluate(() =>
    (window as unknown as { __harness: { clearInvocations(): void } }).__harness.clearInvocations()
  );

  // Open space info
  await page.getByRole("button", { name: t("workspace.spaceInfoSettings") }).click();
  const spaceInfoPanel = page.getByRole("complementary", { name: t("panel.context") });
  await expect(page.getByText(t("panel.spaceInfo"), { exact: true })).toBeVisible();

  // Click the Members entry in the SettingsEntryList — scope to the panel to avoid
  // matching the room-header member pill with the same label
  const membersEntry = spaceInfoPanel.getByRole("button", {
    name: t("room.members"),
    exact: true
  });
  await expect(membersEntry).toBeEnabled();
  await membersEntry.click();

  // load_room_settings should have been dispatched for the harness space
  await expect
    .poll(() => invocationCount(page, "load_room_settings"))
    .toBeGreaterThanOrEqual(1);

  const args = await page.evaluate(
    () =>
      (window as unknown as {
        __harness: { invocationsOf(c: string): { args: unknown }[] };
      }).__harness.invocationsOf("load_room_settings")[0]?.args
  );
  expect((args as { roomId: string }).roomId).toBe(HARNESS_SPACE_ID);
});

test("Space info members section shows the Rust-owned member after settings load", async ({
  page
}) => {
  await gotoReadyShell(page);
  await activateSpace(page);

  // Open space info
  await page.getByRole("button", { name: t("workspace.spaceInfoSettings") }).click();
  const spaceInfoPanel = page.getByRole("complementary", { name: t("panel.context") });
  await expect(page.getByText(t("panel.spaceInfo"), { exact: true })).toBeVisible();

  // Trigger members load — scope to panel to avoid the room-header member pill
  const membersEntry = spaceInfoPanel.getByRole("button", {
    name: t("room.members"),
    exact: true
  });
  await membersEntry.click();

  // The harness mock returns several members so the UI cannot pass by showing
  // only the first row or a summarized count.
  const membersSection = page.getByRole("region", { name: t("room.members") });
  await expect(membersSection).toBeVisible();
  for (const member of HARNESS_MEMBERS) {
    await expect(membersSection).toContainText(member.label);
    await expect(membersSection).toContainText(member.userId);
    await expect(
      membersSection.getByRole("button", {
        name: t("room.messageMember", { name: member.label })
      })
    ).toBeVisible();
  }
});

test("Space info member rows can start DMs for any listed member", async ({ page }) => {
  await gotoReadyShell(page);
  await activateSpace(page);

  await page.getByRole("button", { name: t("workspace.spaceInfoSettings") }).click();
  const spaceInfoPanel = page.getByRole("complementary", { name: t("panel.context") });
  await expect(page.getByText(t("panel.spaceInfo"), { exact: true })).toBeVisible();
  await spaceInfoPanel.getByRole("button", { name: t("room.members"), exact: true }).click();
  await page.evaluate(() =>
    (window as unknown as { __harness: { clearInvocations(): void } }).__harness.clearInvocations()
  );

  const membersSection = page.getByRole("region", { name: t("room.members") });
  await membersSection
    .getByRole("button", {
      name: t("room.messageMember", { name: HARNESS_MEMBERS[2].label })
    })
    .click();

  await expect.poll(() => invocationCount(page, "start_direct_message")).toBe(1);
  const args = await page.evaluate(
    () =>
      (window as unknown as {
        __harness: { invocationsOf(c: string): { args: unknown }[] };
      }).__harness.invocationsOf("start_direct_message")[0]?.args
  );
  expect((args as { userId: string }).userId).toBe(HARNESS_MEMBERS[2].userId);
});

test("Space info members count tile reflects loaded member count", async ({ page }) => {
  await gotoReadyShell(page);
  await activateSpace(page);

  // Open space info — before load, member tile shows "-"
  await page.getByRole("button", { name: t("workspace.spaceInfoSettings") }).click();
  const spaceInfoPanel = page.getByRole("complementary", { name: t("panel.context") });
  await expect(page.getByText(t("panel.spaceInfo"), { exact: true })).toBeVisible();

  // Trigger load through Members entry — scope to panel to avoid the room-header member pill
  await spaceInfoPanel.getByRole("button", { name: t("room.members"), exact: true }).click();

  // After load, harness returns three members so tile should show "3".
  await expect
    .poll(async () => {
      // The tile containing the member count label
      const summaryGrid = page.locator(".settings-summary-grid");
      const membersValue = summaryGrid.locator("text=3");
      return membersValue.count();
    })
    .toBeGreaterThanOrEqual(1);
});
