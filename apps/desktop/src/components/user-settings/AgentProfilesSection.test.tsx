// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { AccountTabSummary, McpAssistantsState, McpProfile } from "../../domain/types";
import { AiAssistantsSection } from "./AiAssistantsSection";

const fixture = vi.hoisted(() => ({
  listeners: new Set<(state: McpAssistantsState) => void>(),
  getState: vi.fn(),
  create: vi.fn(),
  revoke: vi.fn()
}));
vi.mock("../../backend/runtimeEnvironment", () => ({ isTauriRuntime: () => true }));
vi.mock("../../backend/appRuntime", () => ({
  api: {
    getMcpAssistantsState: fixture.getState,
    createAgentProfile: fixture.create,
    revokeAgentProfile: fixture.revoke
  },
  startSessionVerificationWindowDrag: vi.fn()
}));
vi.mock("../../backend/desktopEventRuntime", () => ({ desktopEventPort: {
  listenMcpAssistants: async (listener: (state: McpAssistantsState) => void) => {
    fixture.listeners.add(listener);
    return () => fixture.listeners.delete(listener);
  }
} }));

const OWNER = "account:@owner:example.invalid";
const AGENT = "account:@agent-a:example.invalid";
const tab = (id: string, name: string, isAgent: boolean) => ({
  id,
  accountKey: id.replace("account:", ""),
  homeserver: "example.invalid",
  displayName: name,
  avatarSourceRef: null,
  avatarMxcUri: null,
  status: "ready",
  unreadCount: 0,
  isAgent
}) as AccountTabSummary;
const tabs = [tab(OWNER, "Owner Person", false), tab(AGENT, "Agent Alpha", true)];

const ID = "prof_00112233445566778899aabbccddeeff";
// Text only Rust produces: a re-derivation in React could not match it.
const RUST_SNIPPET = `claude mcp add --scope user koushi-prof_00112233 -- /opt/koushi-mcp --profile ${ID} --profile-secret-file /rust/built/path.secret # from-rust`;

function profile(overrides: Partial<McpProfile> = {}): McpProfile {
  return {
    profile_id: ID,
    label: "Reviewer",
    account_id: AGENT,
    read: true,
    send: false,
    attachments: false,
    message_actions: false,
    draft: false,
    status: "ready",
    connected: false,
    subscribed: false,
    last_seen_ms: null,
    last_rejected_ms: null,
    snippets: [
      { client: "claude_code", client_name: "claude-code", kind: "command", text: RUST_SNIPPET, config_hint: null, action: null },
      { client: "other", client_name: "my-client", kind: "json", text: "{\"from\":\"rust\"}", config_hint: null, action: null }
    ],
    ...overrides
  };
}

function state(overrides: Partial<McpAssistantsState> = {}): McpAssistantsState {
  return {
    bridge: "running",
    store: "ok",
    launcher: { state: "available", path: "/opt/koushi-mcp" },
    pending: [],
    assistants: [],
    registrations: [],
    outdated_launchers: [],
    profiles: [],
    agent_accounts: [AGENT],
    ...overrides
  };
}

async function renderPage(initial: McpAssistantsState) {
  fixture.getState.mockResolvedValue(initial);
  render(<AiAssistantsSection settings={{ enabled: true, allowed_senders: { added: [], removed: [] } }} isSaving={false} accountTabs={tabs} onUpdateSettings={vi.fn()} />);
  await screen.findByRole("region", { name: "Agent profiles" });
}

function publish(next: McpAssistantsState) {
  act(() => { for (const listener of fixture.listeners) listener(next); });
}

const section = () => screen.getByRole("region", { name: "Agent profiles" });

beforeEach(() => {
  fixture.listeners.clear();
  Object.defineProperty(navigator, "clipboard", { value: { writeText: vi.fn().mockResolvedValue(undefined) }, configurable: true });
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

test("only the agent accounts Rust lists are offered, never a non-agent account", async () => {
  await renderPage(state({ agent_accounts: [AGENT] }));
  const options = within(within(section()).getByLabelText("Agent account")).getAllByRole("option").map(option => option.textContent);
  expect(options).toEqual(["Agent Alpha"]);
  expect(within(section()).queryByText("Owner Person")).toBeNull();
});

test("with no agent account the form says so and cannot create", async () => {
  await renderPage(state({ agent_accounts: [] }));
  expect(within(section()).getByText(/Mark an account as an agent account above/)).toBeTruthy();
  expect((within(section()).getByRole("button", { name: "Create profile" }) as HTMLButtonElement).disabled).toBe(true);
});

test("create sends the label, the account and the chosen grants, and shows no secret", async () => {
  fixture.create.mockResolvedValue({ kind: "created", profile_id: ID, secret_path: "/rust/built/path.secret", secret: "S3CR3T-MARKER" });
  await renderPage(state());
  fireEvent.change(within(section()).getByLabelText("Label"), { target: { value: "Reviewer" } });
  fireEvent.click(within(section()).getByRole("checkbox", { name: "Read" }));
  fireEvent.click(within(section()).getByRole("checkbox", { name: "Send" }));
  fireEvent.click(within(section()).getByRole("button", { name: "Create profile" }));
  await waitFor(() => expect(fixture.create).toHaveBeenCalledWith("Reviewer", AGENT, {
    read: true, send: true, attachments: false, message_actions: false, draft: false
  }));
  expect(await within(section()).findByText(/Profile created/)).toBeTruthy();
  expect(document.body.textContent).not.toContain("S3CR3T-MARKER");
});

test("a failed create names the reason from Rust", async () => {
  fixture.create.mockResolvedValue({ kind: "failed", reason: "not_agent_account" });
  await renderPage(state());
  fireEvent.change(within(section()).getByLabelText("Label"), { target: { value: "Reviewer" } });
  fireEvent.click(within(section()).getByRole("button", { name: "Create profile" }));
  expect((await within(section()).findByRole("alert")).textContent).toMatch(/cannot be used for a profile/);
});

test("the row follows Rust's status: ready, connected, subscribed, secret missing, last wrong secret", async () => {
  await renderPage(state({ profiles: [profile()] }));
  const row = () => within(section()).getByRole("article", { name: "Reviewer" });
  expect(within(row()).getByText("Ready")).toBeTruthy();
  expect(within(row()).getByText("Not connected")).toBeTruthy();
  expect(within(row()).getByText("Not seen yet")).toBeTruthy();

  publish(state({ profiles: [profile({ connected: true, last_seen_ms: Date.UTC(2026, 10, 27, 9, 30) })] }));
  expect(within(row()).getByText("Connected")).toBeTruthy();
  expect(within(row()).getByText("Not subscribed")).toBeTruthy();
  expect(within(row()).getByText(/Last seen/)).toBeTruthy();

  publish(state({ profiles: [profile({ connected: true, subscribed: true })] }));
  expect(within(row()).getByText("Subscribed")).toBeTruthy();

  publish(state({ profiles: [profile({ status: "secret_missing", last_rejected_ms: Date.UTC(2026, 10, 27, 10, 0) })] }));
  expect(within(row()).getByText("Secret file is missing")).toBeTruthy();
  expect(within(row()).getByText(/Last wrong secret/)).toBeTruthy();
});

test("copy puts exactly the text Rust built on the clipboard, for the chosen client", async () => {
  await renderPage(state({ profiles: [profile()] }));
  const row = within(section()).getByRole("article", { name: "Reviewer" });
  expect(within(row).getByText(RUST_SNIPPET)).toBeTruthy();
  fireEvent.click(within(row).getByRole("button", { name: "Copy" }));
  await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenCalledWith(RUST_SNIPPET));

  fireEvent.change(within(row).getByLabelText("Client"), { target: { value: "other" } });
  fireEvent.click(within(row).getByRole("button", { name: "Copy" }));
  await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenLastCalledWith("{\"from\":\"rust\"}"));
});

test("revoke asks first, then the row leaves when Rust publishes it gone", async () => {
  fixture.revoke.mockResolvedValue("revoked");
  await renderPage(state({ profiles: [profile()] }));
  fireEvent.click(within(section()).getByRole("button", { name: "Revoke Reviewer" }));
  const dialog = await screen.findByRole("dialog");
  expect(within(dialog).getByText(/cannot be undone/)).toBeTruthy();
  fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
  expect(fixture.revoke).not.toHaveBeenCalled();
  expect(within(section()).getByRole("article", { name: "Reviewer" })).toBeTruthy();

  fireEvent.click(within(section()).getByRole("button", { name: "Revoke Reviewer" }));
  fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Revoke" }));
  await waitFor(() => expect(fixture.revoke).toHaveBeenCalledWith(ID));
  // The click alone removes nothing: the row is Rust's until it says otherwise.
  expect(within(section()).getByRole("article", { name: "Reviewer" })).toBeTruthy();
  publish(state({ profiles: [] }));
  await waitFor(() => expect(within(section()).queryByRole("article", { name: "Reviewer" })).toBeNull());
});
