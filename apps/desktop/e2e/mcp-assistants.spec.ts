import { expect, test } from "@playwright/test";
import type { AccountTabsSnapshot, McpAssistantsState } from "../src/domain/types";
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

