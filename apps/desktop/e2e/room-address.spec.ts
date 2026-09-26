import { expect, test } from "@playwright/test";
import { gotoReadyShell, invocationCount } from "./support/basicOperations";

test("public address preserves manual edits and collision drafts until successful retry", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const suggestions: Record<string, string> = { "Example Room": "example-room", "Changed Name": "changed-name" };
    window.__harness.setCommandResponse("preview_room_address", ({ name, aliasLocalpart }) => {
      const localpart = aliasLocalpart ?? suggestions[name] ?? "";
      return {
        localpart,
        full_alias: localpart ? `#${localpart}:example.invalid` : null,
        error: localpart ? null : "empty",
        server_name: "example.invalid"
      };
    });
    window.__harness.setCommandResponse("create_room", () => { throw { kind: "aliasInUse" }; });
    window.__harness.clearInvocations();
  });
  await page.getByRole("button", { name: "Create room", exact: true }).click();
  const name = page.getByRole("textbox", { name: "Room name" });
  await name.fill("Example Room");
  await page.getByRole("radio", { name: "Public room", exact: true }).check();
  const address = page.getByRole("textbox", { name: "Room address" });
  await expect(address).toHaveValue("example-room");
  await address.fill("manual");
  await name.fill("Changed Name");
  await expect(address).toHaveValue("manual");
  await page.getByRole("radio", { name: "Private room", exact: true }).check();
  await page.getByRole("radio", { name: "Public room", exact: true }).check();
  await expect(address).toHaveValue("manual");
  await expect(page.getByRole("status").filter({ hasText: "Full address:" })).toHaveText("Full address: #manual:example.invalid");
  await address.focus();
  await address.evaluate((input) => {
    input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", code: "Enter", keyCode: 229, isComposing: true, bubbles: true }));
    input.closest("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  });
  expect(await invocationCount(page, "create_room")).toBe(0);
  await address.dispatchEvent("compositionend", { data: "manual" });
  await address.dispatchEvent("keyup", { key: "Enter", code: "Enter" });
  await page.getByRole("button", { name: "Submit create room" }).click();
  await expect(page.getByRole("alert").filter({ hasText: "already in use" })).toBeVisible();
  await expect(name).toHaveValue("Changed Name");
  await expect(address).toHaveValue("manual");
  await address.fill("available");
  await page.evaluate(() => window.__harness.setCommandResponse("create_room", () => window.__harness.currentSnapshot()));
  await page.getByRole("button", { name: "Submit create room" }).click();
  await expect(name).toBeHidden();
  expect(await invocationCount(page, "create_room")).toBe(2);
});

test("private creation needs no alias even when a public address is invalid", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    window.__harness.setCommandResponse("preview_room_address", () => ({
      localpart: "#invalid", full_alias: null, error: "invalid", server_name: "example.invalid"
    }));
    window.__harness.setCommandResponse("create_room", () => window.__harness.currentSnapshot());
    window.__harness.clearInvocations();
  });
  await page.getByRole("button", { name: "Create room", exact: true }).click();
  const name = page.getByRole("textbox", { name: "Room name" });
  await name.fill("Private discussion");
  await page.getByRole("radio", { name: "Public room", exact: true }).check();
  await expect(page.getByRole("textbox", { name: "Room address" })).toHaveValue("#invalid");
  const submit = page.getByRole("button", { name: "Submit create room" });
  await expect(submit).toBeDisabled();
  await page.getByRole("radio", { name: "Private room", exact: true }).check();
  await expect(submit).toBeEnabled();
  await submit.click();
  await expect(name).toBeHidden();
  expect(await page.evaluate(() => window.__harness.invocationsOf("create_room").map(call => call.args.options)))
    .toEqual([expect.objectContaining({
      name: "Private discussion", visibility: "private", aliasLocalpart: null
    })]);
});

// #1006: from a Space, the dialog renders the Rust `<space>-<room>` suggestion,
// names the target Space and server-wide scope, and makes a conflict
// actionable at the address field without losing the draft or the Space.
test("a Space room conflict names the attempted address and keeps the draft in the Space", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const next = structuredClone(window.__harness.currentSnapshot());
    next.state_generation = (next.state_generation ?? 0) + 1;
    next.state.ui.navigation.active_space_id = "!research:example.invalid";
    next.state.domain.spaces = [
      ...next.state.domain.spaces,
      {
        space_id: "!research:example.invalid",
        raw_name: "research-group",
        display_name: "research-group",
        avatar: null,
        join_rule: null,
        child_room_ids: []
      }
    ];
    next.sidebar.active_space_id = "!research:example.invalid";
    next.sidebar.account_home.is_active = false;
    next.sidebar.space_rail = [
      ...next.sidebar.space_rail.map((space) => ({ ...space, is_active: false })),
      {
        space_id: "!research:example.invalid",
        display_name: "research-group",
        avatar: null,
        unread_count: 0,
        highlight_count: 0,
        is_active: true
      }
    ];
    window.__harness.setSnapshot(next);
    window.__harness.pushStateUpdate();
    window.__harness.setCommandResponse("preview_room_address", ({ name, aliasLocalpart }) => {
      const localpart = aliasLocalpart ?? (name ? `research-group-${name}` : "");
      return {
        localpart,
        full_alias: localpart ? `#${localpart}:example.invalid` : null,
        error: localpart ? null : "empty",
        server_name: "example.invalid"
      };
    });
    window.__harness.setCommandResponse("create_room", () => { throw { kind: "aliasInUse" }; });
    window.__harness.clearInvocations();
  });
  await page.getByRole("button", { name: "Create room", exact: true }).click();
  const name = page.getByRole("textbox", { name: "Room name" });
  await name.fill("papers");
  await page.getByRole("radio", { name: "Public room", exact: true }).check();
  const address = page.getByRole("textbox", { name: "Room address" });
  await expect(address).toHaveValue("research-group-papers");
  await expect(page.getByText(/^Public room in research-group:/)).toBeVisible();
  await expect(page.getByText(/must be unique on example\.invalid, across all Spaces/)).toBeVisible();
  await expect(page.getByRole("status").filter({ hasText: "Full address:" }))
    .toHaveText("Full address: #research-group-papers:example.invalid");

  await page.getByRole("button", { name: "Submit create room" }).click();
  await expect(page.getByRole("alert")).toHaveText(
    "The address #research-group-papers:example.invalid is already in use. Addresses are shared across all Spaces on example.invalid. You can keep the room name ‘papers’; change only the room address, for example by adding a project name or number."
  );
  await expect(address).toBeFocused();
  await expect(name).toHaveValue("papers");
  expect(await page.evaluate(() => window.__harness.invocationsOf("create_room").map((call) => call.args.options)))
    .toEqual([expect.objectContaining({
      name: "papers",
      aliasLocalpart: "research-group-papers",
      parentSpace: { spaceId: "!research:example.invalid" }
    })]);

  // Editing the address retires the conflict; the room name is still `papers`.
  await address.fill("research-group-papers-2026");
  await expect(page.getByRole("alert")).toHaveCount(0);
  await page.evaluate(() => window.__harness.setCommandResponse("create_room", () => window.__harness.currentSnapshot()));
  await page.getByRole("button", { name: "Submit create room" }).click();
  await expect(name).toBeHidden();
  expect(await page.evaluate(() => window.__harness.invocationsOf("create_room").map((call) => call.args.options)))
    .toEqual([
      expect.objectContaining({ aliasLocalpart: "research-group-papers" }),
      expect.objectContaining({
        name: "papers",
        aliasLocalpart: "research-group-papers-2026",
        parentSpace: { spaceId: "!research:example.invalid" }
      })
    ]);
});
