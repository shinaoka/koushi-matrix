import { expect, test } from "@playwright/test";
import { t } from "../src/i18n/messages";

/**
 * #330: account-global actions belong to Home. A sidebar entry's location has to
 * explain its scope, so Explore and Invites are Home-only and a space sidebar is
 * the room list for that space.
 */

function rail(page: import("@playwright/test").Page) {
  return page.getByRole("navigation", { name: t("workspace.workspaces") });
}

async function selectHome(page: import("@playwright/test").Page) {
  await rail(page).getByRole("button", { name: "Home" }).click();
}

async function selectSpace(page: import("@playwright/test").Page) {
  await rail(page).getByRole("button", { name: "Harness Space" }).click();
}

test("Home owns Explore and Invites; a selected space shows neither", async ({ page }) => {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("complementary", { name: t("workspace.rooms") })).toBeVisible();

  await selectHome(page);
  await expect(page.getByRole("button", { name: t("workspace.explore"), exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: t("workspace.invites"), exact: true })).toBeVisible();

  await selectSpace(page);
  await expect(page.getByRole("button", { name: t("workspace.explore"), exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: t("workspace.invites"), exact: true })).toHaveCount(0);
});

test("Home drains the current room composer after same-view room navigation", async ({ page }) => {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: t("timeline.conversation") })).toBeVisible();

  const composer = page.getByRole("textbox", { name: t("composer.messageComposer") });
  await composer.fill("temporary draft");
  await composer.fill("");

  await page.evaluate(() => {
    const next = structuredClone(window.__harness.currentSnapshot());
    const currentRoom = next.state.domain.rooms[0];
    const currentSpace = next.state.domain.spaces[0];
    if (!currentRoom || !currentSpace) throw new Error("ready harness room/space missing");
    const roomId = "!home-navigation-room:example.invalid";
    next.state.domain.rooms.push({
      ...currentRoom,
      room_id: roomId,
      display_name: "Home Navigation Room",
      display_label: "Home Navigation Room",
      original_display_label: "Home Navigation Room"
    });
    currentSpace.child_room_ids.push(roomId);
    const currentSidebarRoom = next.sidebar.space_rooms[0];
    if (!currentSidebarRoom) throw new Error("ready harness sidebar room missing");
    const sidebarRoom = {
      ...currentSidebarRoom,
      room_id: roomId,
      display_name: "Home Navigation Room"
    };
    next.sidebar.space_rooms.push(sidebarRoom);
    next.sidebar.sections.rooms.push(sidebarRoom);
    window.__harness.setSnapshot(next);
    window.__harness.pushStateUpdate();
  });

  await page.getByRole("button", { name: "Home Navigation Room", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(() => window.__harness.currentSnapshot().state.ui.timeline.room_id)
    )
    .toBe("!home-navigation-room:example.invalid");

  await page.evaluate(() => window.__harness.clearInvocations());
  await selectHome(page);

  await expect
    .poll(() => page.evaluate(() => window.__harness.invocationsOf("select_space").length))
    .toBe(1);
  await expect
    .poll(() => page.evaluate(() => window.__harness.invocationsOf("select_space")[0]?.args))
    .toEqual({ spaceId: null });
});

test("a selected space exposes the aggregate Threads nav entry", async ({ page }) => {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("complementary", { name: t("workspace.rooms") })).toBeVisible();

  await selectSpace(page);

  const sidebar = page.getByRole("complementary", { name: t("workspace.rooms") });
  await expect(sidebar.getByRole("button", { name: t("workspace.threads") })).toBeVisible();
});

test("the room header exposes Threads with no unread thread activity", async ({ page }) => {
  // Previously the header button appeared only when a thread had unread
  // attention, so a quiet room's threads were unreachable once the sidebar
  // entry went away.
  await page.goto("/appHarness.html");
  const header = page.getByRole("main", { name: t("timeline.conversation") });
  await expect(header.getByRole("button", { name: t("workspace.threads") })).toBeVisible();
});

test("the Home rail badge totals unread messages and invites separately in its label", async ({
  page
}) => {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("complementary", { name: t("workspace.rooms") })).toBeVisible();

  const homeButton = rail(page).getByRole("button", { name: "Home" });
  const label = await homeButton.getAttribute("aria-label");
  const badge = await homeButton.getAttribute("data-count");

  expect(label).not.toBeNull();
  const counts = label!.match(/(\d+)\D+(\d+)/);
  expect(
    counts,
    `Home label should name unread messages and invites separately, got ${label}`
  ).not.toBeNull();

  const unread = Number(counts![1]);
  const invites = Number(counts![2]);
  expect(invites).toBeGreaterThan(0);
  expect(
    Number(badge),
    "the badge shows the Rust-owned total of unread messages plus invites"
  ).toBe(unread + invites);
});

test("Explore separates joining by address from searching a public directory", async ({
  page
}) => {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("complementary", { name: t("workspace.rooms") })).toBeVisible();
  await selectHome(page);
  await page.getByRole("button", { name: t("workspace.explore"), exact: true }).click();

  const explore = page.getByRole("main", { name: t("workspace.explore") });
  await expect(explore).toBeVisible();

  // Both actions are visible and separately labelled, instead of one field
  // labelled for search silently also accepting addresses.
  await expect(
    explore.getByRole("textbox", { name: t("directory.addressLabel") })
  ).toBeVisible();
  await expect(
    explore.getByRole("searchbox", { name: t("directory.searchTermLabel") })
  ).toBeVisible();
  await expect(
    explore.getByRole("textbox", { name: t("directory.searchServer") })
  ).toBeVisible();
  await expect(explore.getByText(t("directory.searchServerHelper"))).toBeVisible();
});

test("a user id in the address field is explained, not silently ignored", async ({ page }) => {
  await page.goto("/appHarness.html");
  await selectHome(page);
  await page.getByRole("button", { name: t("workspace.explore"), exact: true }).click();

  const explore = page.getByRole("main", { name: t("workspace.explore") });
  await explore
    .getByRole("textbox", { name: t("directory.addressLabel") })
    .fill("@someone:example.invalid");
  await explore.getByRole("button", { name: t("directory.preview") }).click();

  await expect(explore.getByText(t("directory.addressIsUser"))).toBeVisible();
});

test("ordinary words in the address field are reported as not an address", async ({ page }) => {
  await page.goto("/appHarness.html");
  await selectHome(page);
  await page.getByRole("button", { name: t("workspace.explore"), exact: true }).click();

  const explore = page.getByRole("main", { name: t("workspace.explore") });
  await explore
    .getByRole("textbox", { name: t("directory.addressLabel") })
    .fill("just some words");
  await explore.getByRole("button", { name: t("directory.preview") }).click();

  await expect(explore.getByText(t("directory.addressNotRecognized"))).toBeVisible();
});

test("directory results state whether each hit is a room or a space", async ({ page }) => {
  await page.goto("/appHarness.html");
  await selectHome(page);
  await page.getByRole("button", { name: t("workspace.explore"), exact: true }).click();

  const explore = page.getByRole("main", { name: t("workspace.explore") });
  await explore.getByRole("searchbox", { name: t("directory.searchTermLabel") }).fill("public");
  await explore.getByRole("button", { name: t("directory.search"), exact: true }).click();

  const results = explore.getByRole("region", { name: t("directory.results") });
  await expect(results.locator(".directory-result").first()).toBeVisible();
  const badges = results.locator(".directory-result-type");
  expect(await badges.count()).toBe(await results.locator(".directory-result").count());
});
