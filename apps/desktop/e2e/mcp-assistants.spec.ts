import { expect, test, type Locator, type Page } from "@playwright/test";
import type { AccountTabsSnapshot, McpAssistantAccount, McpAssistantsState } from "../src/domain/types";
import { t } from "../src/i18n/messages";
import { gotoReadyShell } from "./support/basicOperations";

const ACCOUNT = "account:@harness-user:example.invalid";

const accountTabs: AccountTabsSnapshot = {
  selectedTabId: ACCOUNT,
  tabs: [{
    id: ACCOUNT,
    accountKey: "@harness-user:example.invalid",
    homeserver: "https://harness.example.invalid",
    displayName: "Harness",
    avatarSourceRef: null,
    status: "ready",
    unreadCount: 0
  }],
  badgeCount: 0
};

function mcpState(overrides: Partial<McpAssistantsState> = {}): McpAssistantsState {
  return {
    bridge: "running",
    store: "ok",
    launcher: { state: "available", path: "/opt/koushi/koushi-mcp" },
    pending: [],
    assistants: [],
    registrations: [{
      client: "claude_code",
      client_name: "claude-code",
      kind: "command",
      text: "claude mcp add --scope user koushi -- '/opt/koushi/koushi-mcp' --client-name claude-code",
      config_hint: null,
      action: "run_command"
    }],
    outdated_launchers: [],
    ...overrides
  };
}

test("an assistant's first read opens an approval that answers by request id", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(async (tabs) => {
    await window.__harness.pushAccountTabs(tabs);
    window.__harness.clearInvocations();
  }, accountTabs);
  await page.evaluate((state) => window.__harness.pushMcpAssistants(state), mcpState({
    pending: [{
      request_id: 4,
      client_name: "claude-code",
      client_version: "2.1.0",
      account_id: ACCOUNT,
      need: "read",
      room_label: null,
      message_preview: null,
      waiting: true,
      waiting_calls: 1
    }]
  }));

  const dialog = page.getByRole("dialog", { name: "Allow claude-code to read your messages?", exact: true });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText("wants to read Harness")).toBeVisible();
  const allow = dialog.getByRole("button", { name: "Allow reading Harness", exact: true });
  // Allow is never the initially focused control and arms after a short delay.
  await expect(allow).not.toBeFocused();
  await expect(allow).toBeEnabled();
  await allow.click();
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("answer_mcp_approval").at(-1)?.args))
    .toMatchObject({ requestId: 4, allow: true });

  // The broker settles the request; the dialog follows its state.
  await page.evaluate((state) => window.__harness.pushMcpAssistants(state), mcpState());
  await expect(dialog).toHaveCount(0);
});

test("App Settings lists assistants and changes one permission at a time", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(async (tabs) => {
    await window.__harness.pushAccountTabs(tabs);
    window.__harness.clearInvocations();
  }, accountTabs);
  await page.getByRole("button", { name: "App Settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "App Settings", exact: true });
  await settings.getByRole("tab", { name: "AI assistants", exact: true }).click();
  await page.evaluate((state) => window.__harness.pushMcpAssistants(state), mcpState({
    assistants: [{
      client_name: "codex",
      client_version: "0.147.0",
      last_used_ms: null,
      accounts: [{ account_id: ACCOUNT, read: true, send: false, attachments: false, message_actions: false, denied: [] }]
    }]
  }));

  const card = settings.getByRole("article", { name: "codex" });
  await expect(card).toBeVisible();
  await card.getByRole("switch", { name: "Send for @harness-user:example.invalid", exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("set_mcp_permission").at(-1)?.args))
    .toMatchObject({ clientName: "codex", accountId: ACCOUNT, need: "send", allowed: true });

  await expect(settings.getByText("claude mcp add --scope user koushi", { exact: false })).toBeVisible();

  await settings.getByRole("switch", { name: "Let AI assistants connect", exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("update_settings").at(-1)?.args))
    .toMatchObject({ patch: { scope: "app", mcp: { enabled: false } } });
});

test("a message-actions approval names the react, edit, and delete grant (#1628)", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(async (tabs) => {
    await window.__harness.pushAccountTabs(tabs);
    window.__harness.clearInvocations();
  }, accountTabs);
  await page.evaluate((state) => window.__harness.pushMcpAssistants(state), mcpState({
    pending: [{
      request_id: 7,
      client_name: "claude-code",
      client_version: "2.1.0",
      account_id: ACCOUNT,
      need: "message_actions",
      room_label: null,
      message_preview: null,
      waiting: true,
      waiting_calls: 1
    }]
  }));

  const dialog = page.getByRole("dialog", { name: "Allow claude-code to change your messages?", exact: true });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText("wants to react to, edit, and delete your messages in Harness")).toBeVisible();
  await expect(dialog.getByText("Add and remove its own reactions on any message it can see")).toBeVisible();
  await expect(dialog.getByText("Edit the text of messages you sent, keeping any image or file")).toBeVisible();
  await expect(dialog.getByText("Delete messages you sent")).toBeVisible();
  await expect(dialog.getByText(/It cannot change anyone else's message/)).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Allow changing messages in Harness", exact: true })).toBeVisible();
});

test("App Settings lists the message-actions permission and toggles it alone", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(async (tabs) => {
    await window.__harness.pushAccountTabs(tabs);
    window.__harness.clearInvocations();
  }, accountTabs);
  await page.getByRole("button", { name: "App Settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "App Settings", exact: true });
  await settings.getByRole("tab", { name: "AI assistants", exact: true }).click();
  await page.evaluate((state) => window.__harness.pushMcpAssistants(state), mcpState({
    assistants: [{
      client_name: "codex",
      client_version: "0.147.0",
      last_used_ms: null,
      accounts: [{ account_id: ACCOUNT, read: true, send: true, attachments: true, message_actions: false, denied: [] }]
    }]
  }));

  const card = settings.getByRole("article", { name: "codex" });
  await expect(card.getByRole("columnheader", { name: "Change messages", exact: true })).toBeVisible();
  const change = card.getByRole("switch", { name: "Change messages for @harness-user:example.invalid", exact: true });
  // Ungranted reads as off rather than denied.
  await expect(change).toHaveAttribute("aria-checked", "false");
  await expect(card.getByText("Denied", { exact: true })).toHaveCount(0);
  await change.click();
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("set_mcp_permission").at(-1)?.args))
    .toMatchObject({ clientName: "codex", accountId: ACCOUNT, need: "message_actions", allowed: true });
});

/** The permission table has six columns (#1786): the account and five switches. */
const PERMISSION_COLUMNS = 6;

type CellBox = {
  text: string;
  left: number;
  right: number;
  middle: number;
  height: number;
  /** Line boxes of a text-only cell; null when the cell holds a control. */
  lines: number | null;
  /** The cell's switch, relative to the cell's own box. */
  control: { top: number; bottom: number; left: number; right: number; middle: number } | null;
};

async function cellBoxes(row: Locator): Promise<CellBox[]> {
  return row.evaluate((element) => [...element.children].map((child) => {
    const box = child.getBoundingClientRect();
    // One distinct top per line box: a truncated label is still one line.
    const range = document.createRange();
    range.selectNodeContents(child);
    const tops = new Set([...range.getClientRects()].map(rect => Math.round(rect.top)));
    const control = child.querySelector('[role="switch"]')?.getBoundingClientRect();
    return {
      text: (child.textContent ?? "").trim(),
      left: box.left,
      right: box.right,
      middle: box.top + box.height / 2,
      height: box.height,
      lines: child.childElementCount === 0 ? tops.size : null,
      control: control
        ? {
            top: control.top - box.top,
            bottom: control.bottom - box.top,
            left: control.left - box.left,
            right: control.right - box.left,
            middle: control.top + control.height / 2
          }
        : null
    };
  }));
}

/** One grid row is one line: no cell in another track, no wrapped text, and every
 *  switch sits inside its own cell, on the same line as its neighbours. */
function expectOneLine(cells: CellBox[], what: string) {
  expect(cells, what).toHaveLength(PERMISSION_COLUMNS);
  const offLine = cells.filter(cell => Math.abs(cell.middle - cells[0].middle) > 1);
  expect(offLine.map(cell => cell.text), `${what} moved onto another line`).toEqual([]);
  const wrapped = cells.filter(cell => cell.lines !== null && cell.lines > 1);
  expect(wrapped.map(cell => cell.text), `${what} wrapped its text`).toEqual([]);
  const stacked = cells.filter(
    cell => cell.control !== null && Math.abs(cell.control.middle - cell.middle) > 3
  );
  expect(stacked.map(cell => cell.text), `${what} stacked a control off its cell`).toEqual([]);
  const escaped = cells.filter(cell => cell.control !== null && (
    cell.control.left < -0.5 || cell.control.top < -0.5 ||
    cell.control.right > cell.right - cell.left + 0.5 || cell.control.bottom > cell.height + 0.5
  ));
  expect(escaped.map(cell => cell.text), `${what} let a switch escape its column`).toEqual([]);
}

async function openAssistantsWith(page: Page, accounts: {
  account_id: string; read: boolean; send: boolean; attachments: boolean; message_actions: boolean; draft: boolean;
  denied?: McpAssistantAccount["denied"];
}[], locale: "en" | "ja" = "en") {
  await page.evaluate(async ({ tabs, next }) => {
    if (next !== "en") {
      const snapshot = structuredClone(window.__harness.currentSnapshot());
      snapshot.state.domain.locale_profile = {
        ...snapshot.state.domain.locale_profile, lang: next, catalog_locale: next
      };
      window.__harness.setSnapshot(snapshot);
      window.__harness.pushStateUpdate();
    }
    await window.__harness.pushAccountTabs(tabs);
    window.__harness.clearInvocations();
  }, { tabs: accountTabs, next: locale });
  await page.getByRole("button", { name: t("settings.appSettings", {}, locale), exact: true }).click();
  const settings = page.getByRole("dialog", { name: t("settings.appSettings", {}, locale), exact: true });
  await settings.getByRole("tab", { name: t("settings.categoryAssistants", {}, locale), exact: true }).click();
  await page.evaluate((state) => window.__harness.pushMcpAssistants(state), mcpState({
    assistants: [{ client_name: "codex", client_version: "0.147.0", last_used_ms: null,
      accounts: accounts.map(account => ({ denied: [], ...account })) }]
  }));
  return settings.getByRole("table", { name: "codex" });
}

test("a permission table is one row with its headings above their own switches (#1786)", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await gotoReadyShell(page);
  const table = await openAssistantsWith(page, [
    // One refused permission: the widest cell, a 42px track plus its "Denied" note.
    { account_id: ACCOUNT, read: true, send: true, attachments: true, message_actions: false, draft: true,
      denied: ["message_actions"] }
  ]);
  await expect(table).toBeVisible();

  const heading = table.locator('[role="row"].is-heading');
  const row = table.locator('[role="row"]:not(.is-heading)');
  // The wide dialog and a narrow window: the columns narrow, the row does not wrap.
  for (const width of [1280, 760]) {
    await page.setViewportSize({ width, height: 900 });
    const headingCells = await cellBoxes(heading);
    const rowCells = await cellBoxes(row);
    expectOneLine(headingCells, `the headings at ${width}px`);
    expectOneLine(rowCells, `the account row at ${width}px`);

    // Headings sit above the column they name: one shared grid definition.
    const misaligned = headingCells.flatMap((cell, index) =>
      Math.abs(cell.left - rowCells[index].left) > 1 || Math.abs(cell.right - rowCells[index].right) > 1
        ? [`${cell.text} is not above ${rowCells[index].text}`]
        : []
    );
    expect(misaligned, `columns at ${width}px`).toEqual([]);

    // Nothing is pushed sideways or clipped.
    const overflow = await row.evaluate((element) => element.scrollWidth - element.clientWidth);
    expect(overflow, `overflow at ${width}px`).toBeLessThanOrEqual(1);
  }
});

test("long account IDs stay on one row and keep their whole identity (#1786)", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await gotoReadyShell(page);
  const ids = [
    "@member:example.invalid",
    "@member:synthetic-homeserver-with-a-long-name.example.invalid"
  ];
  // Japanese headings are the widest catalog labels this table renders.
  const table = await openAssistantsWith(page, ids.map(id => ({
    account_id: `account:${id}`, read: true, send: false, attachments: false, message_actions: false, draft: false
  })), "ja");
  await expect(table).toBeVisible();

  expectOneLine(await cellBoxes(table.locator('[role="row"].is-heading')), "the Japanese headings");
  const rows = table.locator('[role="row"]:has([role="switch"])');
  await expect(rows).toHaveCount(ids.length);
  for (const [index, id] of ids.entries()) {
    const row = rows.nth(index);
    expectOneLine(await cellBoxes(row), `the row for ${id}`);
    const rowheader = row.getByRole("rowheader");
    // Truncation may shorten the painted text, never the identity itself.
    await expect(rowheader).toHaveText(id);
    await expect(rowheader).toHaveAttribute("title", id);
    for (const key of ["settings.assistantsRead", "settings.assistantsSend", "settings.assistantsAttachments",
      "settings.assistantsMessageActions", "settings.assistantsDraft"] as const) {
      const label = t("settings.assistantsPermissionLabel", { permission: t(key, {}, "ja"), account: id }, "ja");
      await expect(row.getByRole("switch", { name: label, exact: true })).toBeVisible();
    }
  }

  // The account Koushi knows but the assistant never used keeps its own row: the
  // account in column one, the explanation spanning the five permission columns.
  const unrequested = table.locator('[role="row"]:not(.is-heading):not(:has([role="switch"]))');
  await expect(unrequested).toHaveCount(1);
  const [accountCell, explanation] = await cellBoxes(unrequested);
  const [firstAccountCell, , , , , lastPermissionCell] = await cellBoxes(rows.first());
  expect(Math.abs(accountCell.left - firstAccountCell.left)).toBeLessThanOrEqual(1);
  expect(Math.abs(accountCell.right - firstAccountCell.right)).toBeLessThanOrEqual(1);
  expect(explanation.left).toBeGreaterThan(accountCell.right);
  expect(Math.abs(explanation.right - lastPermissionCell.right)).toBeLessThanOrEqual(1);
});

test("Register asks first, then runs the client's command through Rust", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());
  await page.getByRole("button", { name: "App Settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "App Settings", exact: true });
  await settings.getByRole("tab", { name: "AI assistants", exact: true }).click();
  await page.evaluate((state) => window.__harness.pushMcpAssistants(state), mcpState());

  await settings.getByRole("button", { name: "Register", exact: true }).click();
  const confirm = page.getByRole("dialog", { name: "Register Koushi with Claude Code?", exact: true });
  await expect(confirm).toBeVisible();
  expect(await page.evaluate(() => window.__harness.invocationsOf("register_mcp_client").length)).toBe(0);
  await confirm.getByRole("button", { name: "Run", exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("register_mcp_client").at(-1)?.args))
    .toMatchObject({ client: "claude_code" });
  await expect(settings.getByText("Registered with Claude Code.", { exact: false })).toBeVisible();
});

