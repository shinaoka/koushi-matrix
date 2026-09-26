import { expect, test, type Page } from "@playwright/test";
import type { DesktopSnapshot, SpaceAddRoomStatus } from "../src/domain/types";
import { gotoReadyShell } from "./support/basicOperations";

// #1007: Add existing room is a thin view of the Rust-projected
// `sidebar.space_add_rooms`; these tests drive it with explicit Rust-shaped
// snapshots and assert the submitted command, never a fake state machine.

const SPACE_ID = "!space-add:example.invalid";
/** A room version 12 room ID has no server name. */
const DOMAINLESS = "!31hneApxJ_1o-63DmFrpeqnkFfWppnzWso1JvH3ogLM";

async function showSpaceWithCandidates(page: Page, domainless: SpaceAddRoomStatus) {
  await page.evaluate(
    ({ spaceId, domainlessId, status }) => {
      const next: DesktopSnapshot = structuredClone(window.__harness.currentSnapshot());
      next.state_generation = (next.state_generation ?? 0) + 1;
      next.state.ui.navigation.active_space_id = spaceId;
      next.sidebar.active_space_id = spaceId;
      next.sidebar.account_home.is_active = false;
      next.sidebar.space_rail = [
        ...next.sidebar.space_rail.filter((space) => space.space_id !== spaceId).map((space) => ({ ...space, is_active: false })),
        {
          space_id: spaceId,
          display_name: "Synthetic Workspace",
          avatar: null,
          unread_count: 0,
          highlight_count: 0,
          is_active: true
        }
      ];
      next.sidebar.space_add_rooms = {
        space_id: spaceId,
        candidates: [
          { room_id: domainlessId, display_name: "設計レビュー", avatar: null, status },
          { room_id: "!linked:example.invalid", display_name: "Linked room", avatar: null, status: { kind: "added" } }
        ]
      };
      next.state.ui.basic_operation =
        status.kind === "adding"
          ? { kind: "linkingSpaceChild", request_id: 7, space_id: spaceId, child_room_id: domainlessId }
          : { kind: "idle" };
      window.__harness.setSnapshot(next);
      window.__harness.pushStateUpdate();
    },
    { spaceId: SPACE_ID, domainlessId: DOMAINLESS, status: domainless }
  );
}

async function openAddExistingRoom(page: Page) {
  await page.getByRole("button", { name: "Options for Rooms" }).click();
  await page.getByRole("menuitem", { name: "Add existing room" }).click();
  return page.getByRole("dialog", { name: "Add existing rooms to Synthetic Workspace" });
}

test("adds a domainless room and renders only Rust-settled pending, failure, retry, and success", async ({ page }) => {
  await gotoReadyShell(page);
  await showSpaceWithCandidates(page, { kind: "available" });
  await page.evaluate(() => {
    window.__harness.setCommandResponse("set_space_child", () => window.__harness.currentSnapshot());
    window.__harness.clearInvocations();
  });

  const dialog = await openAddExistingRoom(page);
  await expect(dialog).toBeVisible();
  const search = dialog.getByRole("searchbox", { name: "Search rooms" });
  // Candidate confirmation Enter belongs to the IME, not to a product action.
  await search.focus();
  await search.evaluate((input) => {
    input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", code: "Enter", keyCode: 229, isComposing: true, bubbles: true }));
  });
  await search.dispatchEvent("compositionend", { data: "設計" });
  await search.fill("設計");
  await expect(dialog.getByRole("listitem")).toHaveCount(1);
  expect(await page.evaluate(() => window.__harness.invocationsOf("set_space_child").length)).toBe(0);

  await dialog.getByRole("button", { name: "Add 設計レビュー to Synthetic Workspace" }).click();
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("set_space_child").map((call) => call.args)))
    .toEqual([{ spaceId: SPACE_ID, childRoomId: DOMAINLESS }]);
  // The receipt alone changes nothing visible.
  await expect(dialog.getByText("Adding…")).toHaveCount(0);

  await showSpaceWithCandidates(page, { kind: "adding" });
  await expect(dialog.getByText("Adding…")).toBeVisible();

  await showSpaceWithCandidates(page, { kind: "failed", reason: "network" });
  await expect(dialog.getByRole("alert")).toContainText("Couldn't add this room");
  await dialog.getByRole("button", { name: "Retry adding 設計レビュー" }).click();
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("set_space_child").length)).toBe(2);

  await showSpaceWithCandidates(page, { kind: "added" });
  await expect(dialog.getByText("Added")).toBeVisible();
  await expect(dialog.getByRole("button", { name: /Add 設計レビュー/ })).toHaveCount(0);
});

test("a permission failure is explained at the row", async ({ page }) => {
  await gotoReadyShell(page);
  await showSpaceWithCandidates(page, { kind: "failed", reason: "forbidden" });
  const dialog = await openAddExistingRoom(page);
  await expect(dialog.getByRole("alert")).toHaveText("You don't have permission to add rooms to this space.");
});

test("a room created in a Space whose link failed offers Add existing room", async ({ page }) => {
  await gotoReadyShell(page);
  await showSpaceWithCandidates(page, { kind: "failed", reason: "forbidden" });
  await page.evaluate(() => {
    window.__harness.setCommandResponse("create_room", () => ({
      protocolVersion: 1,
      publishedGeneration: window.__harness.currentSnapshot().state_generation ?? 0,
      spaceLinkFailure: "forbidden"
    }));
  });
  await page.getByRole("button", { name: "Create room", exact: true }).click();
  await page.getByRole("textbox", { name: "Room name" }).fill("papers");
  await page.getByRole("button", { name: "Submit create room" }).click();
  const notice = page.getByRole("dialog", { name: "Room created" });
  await expect(notice.getByRole("alert")).toHaveText(
    "“papers” was created, but you don't have permission to add rooms to Synthetic Workspace."
  );
  expect(await page.evaluate(() => window.__harness.invocationsOf("create_room").map((call) => call.args.options)))
    .toEqual([expect.objectContaining({ parentSpace: { spaceId: SPACE_ID } })]);
  await notice.getByRole("button", { name: "Add existing room" }).click();
  await expect(page.getByRole("dialog", { name: "Add existing rooms to Synthetic Workspace" })).toBeVisible();
});

test("leaving the Space closes the dialog instead of showing another Space's rows", async ({ page }) => {
  await gotoReadyShell(page);
  await showSpaceWithCandidates(page, { kind: "available" });
  const dialog = await openAddExistingRoom(page);
  await expect(dialog).toBeVisible();
  await page.evaluate(() => {
    const next = structuredClone(window.__harness.currentSnapshot());
    next.state_generation = (next.state_generation ?? 0) + 1;
    next.state.ui.navigation.active_space_id = null;
    next.sidebar.active_space_id = null;
    next.sidebar.account_home.is_active = true;
    next.sidebar.space_rail = next.sidebar.space_rail.map((space) => ({ ...space, is_active: false }));
    next.sidebar.space_add_rooms = null;
    window.__harness.setSnapshot(next);
    window.__harness.pushStateUpdate();
  });
  await expect(dialog).toBeHidden();
});
