// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { AccountTabSummary, McpAssistantsState } from "../../domain/types";
import { AiAssistantsSection } from "./AiAssistantsSection";

const fixture = vi.hoisted(() => ({
  listeners: new Set<(state: McpAssistantsState) => void>(),
  getState: vi.fn(),
  answer: vi.fn(),
  register: vi.fn(),
  createExtension: vi.fn(),
  setPermission: vi.fn(),
  remove: vi.fn()
}));
vi.mock("../../backend/runtimeEnvironment", () => ({ isTauriRuntime: () => true }));
vi.mock("../../backend/appRuntime", () => ({
  api: {
    getMcpAssistantsState: fixture.getState,
    answerMcpApproval: fixture.answer,
    registerMcpClient: fixture.register,
    createClaudeDesktopExtension: fixture.createExtension,
    setMcpPermission: fixture.setPermission,
    removeMcpAssistant: fixture.remove
  },
  startSessionVerificationWindowDrag: vi.fn()
}));
vi.mock("../../backend/desktopEventRuntime", () => ({ desktopEventPort: {
  listenMcpAssistants: async (listener: (state: McpAssistantsState) => void) => {
    fixture.listeners.add(listener);
    return () => fixture.listeners.delete(listener);
  }
} }));

const WORK = "account:@member1:example.invalid";
const HOME = "account:@member2:example.invalid";
const tab = (id: string, name: string) => ({
  id,
  accountKey: id.replace("account:", ""),
  homeserver: "example.invalid",
  displayName: name,
  avatarSourceRef: null,
  status: "ready",
  unreadCount: 0
}) as AccountTabSummary;
const tabs = [tab(WORK, "Synthetic Workspace"), tab(HOME, "Member Home")];

function state(overrides: Partial<McpAssistantsState> = {}): McpAssistantsState {
  return {
    bridge: "running",
    store: "ok",
    launcher: { state: "available", path: "/opt/koushi/koushi-mcp" },
    pending: [],
    assistants: [{
      client_name: "claude-code",
      client_version: "2.1.0",
      last_used_ms: null,
      accounts: [{ account_id: WORK, read: true, send: false, attachments: false, message_actions: false, draft: false, denied: ["send"] }]
    }],
    registrations: [
      { client: "claude_code", client_name: "claude-code", kind: "command", text: "claude mcp add --scope user koushi -- '/opt/koushi/koushi-mcp' --client-name claude-code", config_hint: null, action: "run_command" },
      { client: "claude_desktop", client_name: "claude-desktop", kind: "json", text: "{}", config_hint: "claude_desktop_config.json", action: "create_extension" },
      { client: "deepseek_harness", client_name: "deepseek-harness", kind: "yaml", text: "- id: mcp-koushi\n", config_hint: "~/.dsh/profiles/<profile>/cordis.patch.yml", action: null }
    ],
    outdated_launchers: [],
    ...overrides
  };
}

async function renderSection(initial: McpAssistantsState, onUpdateSettings = vi.fn()) {
  fixture.getState.mockResolvedValue(initial);
  render(<AiAssistantsSection settings={{ enabled: true }} isSaving={false} accountTabs={tabs} onUpdateSettings={onUpdateSettings} />);
  await waitFor(() => expect(screen.queryByText("claude-code") ?? screen.queryByText(/No assistant/)).toBeTruthy());
  return onUpdateSettings;
}

beforeEach(() => {
  fixture.setPermission.mockResolvedValue(undefined);
  fixture.remove.mockResolvedValue(undefined);
});
afterEach(() => { cleanup(); vi.clearAllMocks(); });

test("the switch turns the bridge off through an app-scope settings patch", async () => {
  const onUpdateSettings = await renderSection(state());
  fireEvent.click(screen.getByRole("switch", { name: "Let AI assistants connect" }));
  expect(onUpdateSettings).toHaveBeenCalledWith({ scope: "app", mcp: { enabled: false } });
});

test("permission switches change exactly one permission for one account", async () => {
  await renderSection(state());
  const send = screen.getByRole("switch", { name: "Send for @member1:example.invalid" });
  expect(send.getAttribute("aria-checked")).toBe("false");
  expect(screen.getByText("Denied")).toBeTruthy();
  fireEvent.click(send);
  expect(fixture.setPermission).toHaveBeenCalledWith("claude-code", WORK, "send", true);
  // Switches stay disabled while a change is saving.
  const read = screen.getByRole("switch", { name: "Read for @member1:example.invalid" }) as HTMLButtonElement;
  expect(read.disabled).toBe(true);
  await waitFor(() => expect(read.disabled).toBe(false));
  fireEvent.click(read);
  expect(fixture.setPermission).toHaveBeenCalledWith("claude-code", WORK, "read", false);
});

test("accounts the assistant has not used yet are listed as not requested", async () => {
  await renderSection(state());
  const card = screen.getByRole("article", { name: "claude-code" });
  expect(within(card).getByText("@member2:example.invalid")).toBeTruthy();
  expect(within(card).getByText(/Koushi will ask when claude-code first uses this account/)).toBeTruthy();
});

test("Remove access removes the whole assistant", async () => {
  await renderSection(state());
  fireEvent.click(screen.getByRole("button", { name: "Remove access for claude-code" }));
  expect(fixture.remove).toHaveBeenCalledWith("claude-code");
});

test("registration snippets switch per client and copy their exact text", async () => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
  await renderSection(state());
  expect(screen.getByText(/Run this in a terminal/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Copy" }));
  await waitFor(() => expect(writeText).toHaveBeenCalledWith(
    "claude mcp add --scope user koushi -- '/opt/koushi/koushi-mcp' --client-name claude-code"
  ));

  fireEvent.click(screen.getByRole("tab", { name: "DeepSeek Harness" }));
  expect(screen.getByText(/cordis\.patch\.yml/)).toBeTruthy();
});

test("problems are explained: launcher missing, unreadable store, start failure", async () => {
  await renderSection(state({
    bridge: "start_failed",
    store: "malformed",
    launcher: { state: "unavailable", reason: "not_bundled" },
    registrations: [],
    assistants: []
  }));
  expect(screen.getByText(/could not start the assistant connection/)).toBeTruthy();
  expect(screen.getByText(/cannot read its assistant permissions file/)).toBeTruthy();
  expect(screen.getByText(/does not include the koushi-mcp launcher/)).toBeTruthy();
  expect(screen.queryByRole("tablist")).toBeNull();
});

test("an approval whose dialog was closed can be answered here", async () => {
  fixture.answer.mockResolvedValue("applied");
  await renderSection(state({
    pending: [{
      request_id: 5,
      client_name: "pi",
      client_version: "",
      account_id: HOME,
      need: "read",
      room_label: null,
      message_preview: null,
      waiting: false,
      waiting_calls: 0
    }]
  }));
  const pending = screen.getByRole("region", { name: "Waiting for your answer" });
  expect(within(pending).getByText("wants to read Member Home")).toBeTruthy();
  const allow = within(pending).getByRole("button", { name: "Allow reading Member Home" }) as HTMLButtonElement;
  expect(allow.disabled).toBe(true);
  await waitFor(() => expect(allow.disabled).toBe(false), { timeout: 2000 });
  fireEvent.click(allow);
  expect(fixture.answer).toHaveBeenCalledWith(5, true);
});

test("live updates replace the shown state", async () => {
  await renderSection(state());
  act(() => fixture.listeners.forEach(listener => listener(state({ assistants: [] }))));
  expect(screen.getByText("No assistant has asked for access yet.")).toBeTruthy();
});

test("permissions of an account signed in this run match its tab", async () => {
  fixture.getState.mockResolvedValue(state());
  const signInTabs = [{ ...tabs[0], id: "add:1" } as AccountTabSummary, tabs[1]];
  render(<AiAssistantsSection settings={{ enabled: true }} isSaving={false} accountTabs={signInTabs} onUpdateSettings={vi.fn()} />);
  const card = await screen.findByRole("article", { name: "claude-code" });
  expect(within(card).getByRole("switch", { name: "Read for @member1:example.invalid" })).toBeTruthy();
  expect(within(card).queryByText("Account not open in Koushi")).toBeNull();
  // Only the other account is listed as not requested.
  expect(within(card).getAllByText(/Koushi will ask when claude-code first uses this account/)).toHaveLength(1);
});

test("Register runs the client's own command only after confirmation", async () => {
  fixture.register.mockResolvedValue({ outcome: "registered", output: "" });
  await renderSection(state());
  fireEvent.click(screen.getByRole("button", { name: "Register" }));
  const dialog = screen.getByRole("dialog", { name: "Register Koushi with Claude Code?" });
  expect(within(dialog).getByText(/claude mcp add --scope user koushi/)).toBeTruthy();
  expect(fixture.register).not.toHaveBeenCalled();
  fireEvent.click(within(dialog).getByRole("button", { name: "Run" }));
  expect(fixture.register).toHaveBeenCalledWith("claude_code");
  expect(await screen.findByText("Registered with Claude Code. Ask it something, and approve it in Koushi.")).toBeTruthy();
});

test("a failed registration shows the client's own message", async () => {
  fixture.register.mockResolvedValue({ outcome: "failed", exit_code: 1, output: "MCP server koushi already exists" });
  await renderSection(state());
  fireEvent.click(screen.getByRole("button", { name: "Register" }));
  fireEvent.click(screen.getByRole("button", { name: "Run" }));
  expect(await screen.findByText("Claude Code reported a problem (exit code 1):")).toBeTruthy();
  expect(screen.getByText("MCP server koushi already exists")).toBeTruthy();
});

test("a missing CLI points to installing it or copying the command", async () => {
  fixture.register.mockResolvedValue({ outcome: "not_found", program: "claude" });
  await renderSection(state());
  fireEvent.click(screen.getByRole("button", { name: "Register" }));
  fireEvent.click(screen.getByRole("button", { name: "Run" }));
  expect(await screen.findByText(/claude was not found on this computer/)).toBeTruthy();
});

test("Claude Desktop gets an extension instead of a command", async () => {
  fixture.createExtension.mockResolvedValue({ outcome: "saved", path: "/Users/member1/Downloads/Koushi.mcpb" });
  await renderSection(state());
  fireEvent.click(screen.getByRole("tab", { name: "Claude Desktop" }));
  expect(screen.queryByRole("button", { name: "Register" })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Create Claude Desktop extension" }));
  expect(fixture.createExtension).toHaveBeenCalledOnce();
  expect(await screen.findByText(/Created \/Users\/member1\/Downloads\/Koushi\.mcpb\. In Claude Desktop/)).toBeTruthy();
});

test("clients without a CLI keep copy only", async () => {
  await renderSection(state());
  fireEvent.click(screen.getByRole("tab", { name: "DeepSeek Harness" }));
  expect(screen.queryByRole("button", { name: "Register" })).toBeNull();
  expect(screen.getByRole("button", { name: "Copy" })).toBeTruthy();
});

test("an outdated launcher is reported", async () => {
  await renderSection(state({ outdated_launchers: ["claude-desktop"] }));
  expect(screen.getByText(/claude-desktop uses a launcher from an older Koushi/)).toBeTruthy();
});



test("the message-actions permission is listed and turned on from the settings row", async () => {
  await renderSection(state({ assistants: [{
    client_name: "claude-code", client_version: "2.1.0", last_used_ms: null,
    accounts: [{ account_id: WORK, read: true, send: true, attachments: true, message_actions: false, draft: false, denied: [] }]
  }] }));
  const card = screen.getByRole("article", { name: "claude-code" });
  expect(within(card).getByRole("columnheader", { name: "Change messages" })).toBeTruthy();
  const change = within(card).getByRole("switch", { name: "Change messages for @member1:example.invalid" }) as HTMLButtonElement;
  // Not granted reads as off, not as denied (#1628).
  expect(change.getAttribute("aria-checked")).toBe("false");
  expect(within(card).queryByText("Denied")).toBeNull();
  fireEvent.click(change);
  expect(fixture.setPermission).toHaveBeenCalledWith("claude-code", WORK, "message_actions", true);
});

test("a granted message-actions permission reads as on", async () => {
  await renderSection(state({ assistants: [{
    client_name: "claude-code", client_version: "2.1.0", last_used_ms: null,
    accounts: [{ account_id: WORK, read: true, send: true, attachments: true, message_actions: true, draft: false, denied: [] }]
  }] }));
  const card = screen.getByRole("article", { name: "claude-code" });
  expect(within(card).getByRole("switch", { name: "Change messages for @member1:example.invalid" }).getAttribute("aria-checked"))
    .toBe("true");
});

test("full Matrix IDs distinguish equal display names without changing permission targets", async () => {
  const ids = ["@member:example.invalid", "@member:other.example.invalid"];
  const sameNameTabs = ids.map(id => tab(`account:${id}`, "Member"));
  fixture.getState.mockResolvedValue(state({ assistants: [{
    client_name: "claude-code", client_version: "2.1.0", last_used_ms: null,
    accounts: ids.map(id => ({ account_id: `account:${id}`, read: true, send: false, attachments: false, message_actions: false, draft: false, denied: [] }))
  }] }));
  render(<AiAssistantsSection settings={{ enabled: true }} isSaving={false} accountTabs={sameNameTabs} onUpdateSettings={vi.fn()} />);
  const card = await screen.findByRole("article", { name: "claude-code" });
  for (const id of ids) {
    expect(within(card).getByRole("rowheader", { name: id })).toBeTruthy();
  }
  fireEvent.click(within(card).getByRole("switch", { name: `Send for ${ids[1]}` }));
  expect(fixture.setPermission).toHaveBeenCalledWith("claude-code", `account:${ids[1]}`, "send", true);
});
