/**
 * Headless spec: member-list entry points (#81).
 *
 * Proves that People/Profile entry points preserve their Room/Space context,
 * request the Rust-owned member snapshot, and dispatch typed member commands.
 *
 * Entry points under test:
 *  1. Room: room-header People action → People panel opens.
 *  2. Room: Room info "People" entry → People panel opens.
 *  3. Space: Space info "Members" entry → Space-scoped People panel opens.
 */

import { expect, test, type Page } from "@playwright/test";
import { t } from "../src/i18n/messages";

const HARNESS_ROOM_ID = "!harness-room:example.invalid";
const HARNESS_SPACE_ID = "!harness-space:example.invalid";
const HARNESS_MEMBERS = [
  { label: "Harness Ada", userId: "@harness-ada:example.invalid" },
  { label: "Harness Grace", userId: "@harness-grace:example.invalid" },
  { label: "Harness Linus", userId: "@harness-linus:example.invalid" }
] as const;

async function gotoReadyShell(page: Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: t("timeline.conversation") })).toBeVisible();
}

/** Count how many times a command has been invoked in the harness. */
async function invocationCount(
  page: Page,
  command: string
): Promise<number> {
  return page.evaluate(
    (cmd) => (window as unknown as { __harness: { invocationsOf(c: string): unknown[] } }).__harness.invocationsOf(cmd).length,
    command
  );
}

/** Make the active space the harness space so space-info controls are visible. */
async function activateSpace(page: Page): Promise<void> {
  await page
    .getByRole("navigation", { name: t("workspace.workspaces") })
    .getByRole("button", { name: "Harness Space", exact: true })
    .click();
  await expect
    .poll(() => invocationCount(page, "select_space"))
    .toBeGreaterThanOrEqual(1);
  await expect(
    page
      .getByRole("complementary", { name: t("workspace.rooms") })
      .getByText("Harness Space", { exact: true })
  ).toBeVisible();
}

function contextPanel(page: Page) {
  return page.getByRole("complementary", { name: t("panel.context") });
}

async function clearInvocations(page: Page): Promise<void> {
  await page.evaluate(() =>
    (window as unknown as { __harness: { clearInvocations(): void } }).__harness.clearInvocations()
  );
}

async function firstInvocationArgs<T>(page: Page, command: string): Promise<T> {
  return page.evaluate(
    (cmd) =>
      (window as unknown as {
        __harness: { invocationsOf(c: string): { args: unknown }[] };
      }).__harness.invocationsOf(cmd)[0]?.args as T,
    command
  );
}

async function expectPeoplePanelMembers(page: Page): Promise<void> {
  const panel = contextPanel(page);
  await expect(panel.getByRole("heading", { name: t("panel.people") })).toBeVisible();
  await expect(
    panel.getByText(t("people.memberCount", { count: String(HARNESS_MEMBERS.length) }), {
      exact: true
    })
  ).toBeVisible();

  const memberList = panel.getByRole("list", { name: t("room.members") });
  await expect(memberList).toBeVisible();
  for (const member of HARNESS_MEMBERS) {
    await expect(memberList).toContainText(member.label);
    await expect(
      memberList.getByRole("button", {
        name: t("people.openProfile", { name: member.label })
      })
    ).toBeVisible();
    await expect(
      memberList.getByRole("button", {
        name: t("room.messageMember", { name: member.label })
      })
    ).toBeVisible();
  }
}

async function openRoomPeopleFromHeader(page: Page): Promise<void> {
  await page
    .locator(".channel-actions")
    .getByRole("button", { name: t("panel.people") })
    .click();
  await expectPeoplePanelMembers(page);
}

async function openSpacePeopleFromSpaceInfo(page: Page): Promise<void> {
  await activateSpace(page);
  await page.getByRole("button", { name: t("workspace.spaceInfoSettings") }).click();
  const panel = contextPanel(page);
  await expect(panel.getByText(t("panel.spaceInfo"), { exact: true })).toBeVisible();
  await panel.getByRole("button", { name: t("room.members"), exact: true }).click();
  await expectPeoplePanelMembers(page);
}

// ─────────────────────────────────────────────────────────────
//  ROOM entry points
// ─────────────────────────────────────────────────────────────

test("room header People action opens People panel and loads active room members", async ({
  page
}) => {
  await gotoReadyShell(page);
  await clearInvocations(page);

  await openRoomPeopleFromHeader(page);

  await expect
    .poll(() => invocationCount(page, "load_room_settings"))
    .toBeGreaterThanOrEqual(1);
  const args = await firstInvocationArgs<{ roomId: string }>(page, "load_room_settings");
  expect(args.roomId).toBe(HARNESS_ROOM_ID);
});

test("Room info panel dispatches load_room_settings for the active room", async ({ page }) => {
  await gotoReadyShell(page);
  await clearInvocations(page);

  await page.getByRole("button", { name: t("room.roomInfo") }).click();
  await expect(page.getByText(t("panel.roomInfo"), { exact: true })).toBeVisible();

  // load_room_settings should have been dispatched for the harness room
  await expect
    .poll(() => invocationCount(page, "load_room_settings"))
    .toBeGreaterThanOrEqual(1);
  const args = await firstInvocationArgs<{ roomId: string }>(page, "load_room_settings");
  expect(args.roomId).toBe(HARNESS_ROOM_ID);
});

test("Room info People entry opens the standalone People panel", async ({
  page
}) => {
  await gotoReadyShell(page);
  await clearInvocations(page);

  await page.getByRole("button", { name: t("room.roomInfo") }).click();
  const panel = contextPanel(page);
  await expect(panel.getByText(t("panel.roomInfo"), { exact: true })).toBeVisible();
  await panel.getByRole("button", { name: t("room.people"), exact: true }).click();

  await expectPeoplePanelMembers(page);
  await expect
    .poll(() => invocationCount(page, "load_room_settings"))
    .toBeGreaterThanOrEqual(1);
  const args = await firstInvocationArgs<{ roomId: string }>(page, "load_room_settings");
  expect(args.roomId).toBe(HARNESS_ROOM_ID);
});

test("People list rows start DMs through typed commands", async ({ page }) => {
  await gotoReadyShell(page);

  await openRoomPeopleFromHeader(page);
  await clearInvocations(page);

  const memberList = contextPanel(page).getByRole("list", { name: t("room.members") });
  await memberList
    .getByRole("button", {
      name: t("room.messageMember", { name: HARNESS_MEMBERS[1].label })
    })
    .click();

  await expect.poll(() => invocationCount(page, "start_direct_message")).toBe(1);
  const args = await firstInvocationArgs<{ userId: string }>(page, "start_direct_message");
  expect(args.userId).toBe(HARNESS_MEMBERS[1].userId);
});

test("People rows open Profile while preserving Rust member identity", async ({ page }) => {
  await gotoReadyShell(page);

  await openRoomPeopleFromHeader(page);
  const panel = contextPanel(page);
  await panel
    .getByRole("button", {
      name: t("people.openProfile", { name: HARNESS_MEMBERS[0].label })
    })
    .click();

  await expect(panel.getByRole("heading", { name: t("panel.profile") })).toBeVisible();
  await expect(panel).toContainText(HARNESS_MEMBERS[0].label);
  await expect(panel).toContainText(HARNESS_MEMBERS[0].userId);
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

test("Space info 'Members' entry opens Space-scoped People panel", async ({
  page
}) => {
  await gotoReadyShell(page);
  await activateSpace(page);
  await clearInvocations(page);

  await page.getByRole("button", { name: t("workspace.spaceInfoSettings") }).click();
  const spaceInfoPanel = contextPanel(page);
  await expect(spaceInfoPanel.getByText(t("panel.spaceInfo"), { exact: true })).toBeVisible();

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
  const args = await firstInvocationArgs<{ roomId: string }>(page, "load_room_settings");
  expect(args.roomId).toBe(HARNESS_SPACE_ID);
  await expectPeoplePanelMembers(page);
});

test("Space-scoped People panel rows can start DMs for any listed member", async ({ page }) => {
  await gotoReadyShell(page);

  await openSpacePeopleFromSpaceInfo(page);
  await clearInvocations(page);

  const memberList = contextPanel(page).getByRole("list", { name: t("room.members") });
  await memberList
    .getByRole("button", {
      name: t("room.messageMember", { name: HARNESS_MEMBERS[2].label })
    })
    .click();

  await expect.poll(() => invocationCount(page, "start_direct_message")).toBe(1);
  const args = await firstInvocationArgs<{ userId: string }>(page, "start_direct_message");
  expect(args.userId).toBe(HARNESS_MEMBERS[2].userId);
});

test("Space People Profile preserves the Space member context", async ({ page }) => {
  await gotoReadyShell(page);

  await openSpacePeopleFromSpaceInfo(page);
  const panel = contextPanel(page);
  await panel
    .getByRole("button", {
      name: t("people.openProfile", { name: HARNESS_MEMBERS[2].label })
    })
    .click();

  await expect(panel.getByRole("heading", { name: t("panel.profile") })).toBeVisible();
  await expect(panel).toContainText(HARNESS_MEMBERS[2].label);
  await expect(panel).toContainText(HARNESS_MEMBERS[2].userId);
});
