/**
 * Headless spec: member-list entry points (#81).
 *
 * Proves that People/Profile entry points preserve their Room context and that
 * the dedicated Space Members panel requests and renders Rust-owned membership
 * classifications before dispatching typed audit actions.
 *
 * Entry points under test:
 *  1. Room: room-header People action → People panel opens.
 *  2. Room: Room info "People" entry → People panel opens.
 *  3. Space: Space info "Members" entry → dedicated Space Members panel opens.
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

async function expectSpaceMembersPanel(page: Page): Promise<void> {
  const panel = contextPanel(page);
  await expect(
    panel.getByRole("heading", { name: t("spaceMembers.title"), level: 2 })
  ).toBeVisible();
  await expect(panel.getByRole("list", { name: t("spaceMembers.sectionJoined") })).toContainText(
    HARNESS_MEMBERS[0].label
  );
  await expect(panel.getByRole("list", { name: t("spaceMembers.sectionInvited") })).toContainText(
    HARNESS_MEMBERS[1].label
  );
  await expect(
    panel.getByRole("list", { name: t("spaceMembers.sectionChildOnly") })
  ).toContainText(HARNESS_MEMBERS[2].label);
}

async function openSpaceMembersFromSpaceInfo(page: Page): Promise<void> {
  await activateSpace(page);
  await page.getByRole("button", { name: t("workspace.spaceInfoSettings") }).click();
  const panel = contextPanel(page);
  await expect(panel.getByText(t("panel.spaceInfo"), { exact: true })).toBeVisible();
  await panel.getByRole("button", { name: t("room.members"), exact: true }).click();
  await expectSpaceMembersPanel(page);
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

test("Space info 'Members' entry opens the dedicated Space Members panel", async ({
  page
}) => {
  await gotoReadyShell(page);
  await clearInvocations(page);
  await activateSpace(page);

  await page.getByRole("button", { name: t("workspace.spaceInfoSettings") }).click();
  const spaceInfoPanel = contextPanel(page);
  await expect(spaceInfoPanel.getByText(t("panel.spaceInfo"), { exact: true })).toBeVisible();

  const membersEntry = spaceInfoPanel.getByRole("button", {
    name: t("room.members"),
    exact: true
  });
  await expect(membersEntry).toBeEnabled();
  await membersEntry.click();

  // Both the permission snapshot and the classified membership projection are
  // Rust-owned and must be requested for this exact Space.
  await expect
    .poll(() => invocationCount(page, "load_room_settings"))
    .toBeGreaterThanOrEqual(1);
  const args = await firstInvocationArgs<{ roomId: string }>(page, "load_room_settings");
  expect(args.roomId).toBe(HARNESS_SPACE_ID);
  await expect.poll(() => invocationCount(page, "load_space_members")).toBeGreaterThanOrEqual(1);
  const membersArgs = await firstInvocationArgs<{ spaceId: string; generation: number }>(
    page,
    "load_space_members"
  );
  expect(membersArgs).toEqual({ spaceId: HARNESS_SPACE_ID, generation: 2 });
  await expectSpaceMembersPanel(page);
});

test("Space Members can invite a child-room-only user to the Space", async ({ page }) => {
  await gotoReadyShell(page);

  await openSpaceMembersFromSpaceInfo(page);
  await clearInvocations(page);

  const childOnlyList = contextPanel(page).getByRole("list", {
    name: t("spaceMembers.sectionChildOnly")
  });
  await childOnlyList.getByRole("button", { name: t("spaceMembers.invite") }).click();

  await expect.poll(() => invocationCount(page, "invite_user_to_space")).toBe(1);
  const args = await firstInvocationArgs<{
    spaceId: string;
    userId: string;
    generation: number;
  }>(page, "invite_user_to_space");
  expect(args).toEqual({
    spaceId: HARNESS_SPACE_ID,
    userId: HARNESS_MEMBERS[2].userId,
    generation: 2
  });
  await expect(
    contextPanel(page)
      .getByRole("list", { name: t("spaceMembers.sectionInvited") })
      .getByText(HARNESS_MEMBERS[2].label, { exact: true })
  ).toBeVisible();
  await expect(childOnlyList.getByText(HARNESS_MEMBERS[2].label, { exact: true })).toHaveCount(0);
});

test("Space Members can cancel a pending Space invitation", async ({ page }) => {
  await gotoReadyShell(page);

  await openSpaceMembersFromSpaceInfo(page);
  await clearInvocations(page);

  const invitedList = contextPanel(page).getByRole("list", {
    name: t("spaceMembers.sectionInvited")
  });
  await invitedList.getByRole("button", { name: t("spaceMembers.cancelInvite") }).click();

  await expect.poll(() => invocationCount(page, "cancel_space_invite")).toBe(1);
  const args = await firstInvocationArgs<{
    spaceId: string;
    userId: string;
    generation: number;
  }>(page, "cancel_space_invite");
  expect(args).toEqual({
    spaceId: HARNESS_SPACE_ID,
    userId: HARNESS_MEMBERS[1].userId,
    generation: 2
  });
  await expect(
    contextPanel(page).getByText(HARNESS_MEMBERS[1].label, { exact: true })
  ).toHaveCount(0);
});

test("Space Members Profile preserves the Space member context", async ({ page }) => {
  await gotoReadyShell(page);

  await openSpaceMembersFromSpaceInfo(page);
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

test("Space Members rows show each name once and keep one labelled role control", async ({
  page
}) => {
  await gotoReadyShell(page);

  await openSpaceMembersFromSpaceInfo(page);
  const joinedList = contextPanel(page).getByRole("list", {
    name: t("spaceMembers.sectionJoined")
  });

  // #880: the role label reserved its own cell while rendering at full size, so
  // every row printed its display name a second time as "<name>'s role".
  const rowLabel = "Harness Role Target";
  const row = joinedList.locator(".space-members-row").filter({ hasText: rowLabel });
  await expect(row).toHaveCount(1);

  // The name cell states the member once; the duplicate came from the label.
  await expect(row.locator(".space-members-name")).toHaveText(rowLabel);

  // The label stays in the document for assistive technology only: clipped to
  // a single pixel instead of laid out at full width beside the control.
  const labelBox = await row.locator(".space-members-role-control > span").boundingBox();
  expect(labelBox?.width ?? 0).toBeLessThanOrEqual(1);
  expect(labelBox?.height ?? 0).toBeLessThanOrEqual(1);
  await expect(
    row.getByRole("combobox", { name: t("spaceMembers.roleSelect", { name: rowLabel }) })
  ).toBeVisible();
});

test("Space Members can invite a brand-new user to the Space via the invite search", async ({
  page
}) => {
  await gotoReadyShell(page);
  await openSpaceMembersFromSpaceInfo(page);
  await clearInvocations(page);

  // The harness's room-invite search returns candidates; override it so the
  // Space members panel's invite search resolves a brand-new user.
  await page.evaluate(({ spaceId }) => {
    const candidates = [
      {
        user_id: "@brand-new:example.invalid",
        display_label: "Brand New Person",
        original_display_label: "Brand New Person",
        avatar: null,
        source: "profile",
        status: "selectable",
        status_message: null
      }
    ];
    window.__harness.setCommandResponse("search_invite_targets", ({ roomId }) => {
      const snapshot = window.__harness.currentSnapshot();
      const next = {
        ...snapshot,
        state: {
          ...snapshot.state,
          domain: {
            ...snapshot.state.domain,
            invite_workflow: {
              query: {
                room_id: roomId,
                query: "brand",
                candidates,
                explicit_user_id: null
              },
              selected_targets: [],
              scope_plan: null,
              selected_scope: null,
              history_policy: null,
              operation: { kind: "idle" }
            }
          }
        }
      };
      window.__harness.setSnapshot(next);
      return next;
    });
  }, { spaceId: HARNESS_SPACE_ID });

  const panel = contextPanel(page);
  await panel.getByRole("button", { name: t("room.invitePeople") }).click();
  const search = panel.getByRole("searchbox", { name: t("dialog.inviteSearch") });
  await search.fill("brand");
  await expect(panel.getByRole("button", { name: /Brand New Person/ })).toBeVisible();
  await panel.getByRole("button", { name: /Brand New Person/ }).click();

  await expect.poll(() => invocationCount(page, "invite_user_to_space")).toBe(1);
  const args = await firstInvocationArgs<{
    spaceId: string;
    userId: string;
    generation: number;
  }>(page, "invite_user_to_space");
  expect(args).toEqual({
    spaceId: HARNESS_SPACE_ID,
    userId: "@brand-new:example.invalid",
    generation: 2
  });

  // Leaving the invite search resets the shared invite workflow so a later
  // room invite dialog never inherits this space search.
  await clearInvocations(page);
  await panel.getByRole("button", { name: t("action.cancel") }).click();
  await expect.poll(() => invocationCount(page, "close_invite_workflow")).toBe(1);
});

test("Space Members rows keep a long Japanese name and one compact role control on one line", async ({
  page
}) => {
  await gotoReadyShell(page);
  await openSpaceMembersFromSpaceInfo(page);

  const longName = "山田太郎（スペース管理者・テスト用アカウント）";
  await page.evaluate((name) => {
    const harness = (
      window as unknown as {
        __harness: {
          currentSnapshot(): {
            state: { domain: { space_members: { space_joined: Record<string, unknown>[] } } };
          };
          setSnapshot(snapshot: unknown): void;
          pushStateUpdate(): void;
        };
      }
    ).__harness;
    const snapshot = harness.currentSnapshot();
    const members = snapshot.state.domain.space_members;
    harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          space_members: {
            ...members,
            space_joined: members.space_joined.map((entry) =>
              entry.user_id === "@harness-role-target:example.invalid"
                ? { ...entry, display_label: name, display_name: name }
                : entry
            )
          }
        }
      }
    });
    harness.pushStateUpdate();
  }, longName);

  // #880: the role control must not squeeze a long name into a collapsed or
  // wrapped row when the panel is narrow.
  await page.setViewportSize({ width: 900, height: 800 });
  const row = contextPanel(page).locator(".space-members-row").filter({ hasText: longName });
  await expect(row.locator(".space-members-name")).toHaveText(longName);

  const rowBox = await row.boundingBox();
  const nameBox = await row.locator(".space-members-name").boundingBox();
  const selectBox = await row.getByRole("combobox").boundingBox();
  expect(rowBox?.height ?? 0).toBeLessThan(48);
  expect(nameBox?.width ?? 0).toBeGreaterThan(80);
  expect(selectBox?.width ?? 0).toBeGreaterThan(0);
  await expect(row.getByRole("combobox")).toBeVisible();
});
