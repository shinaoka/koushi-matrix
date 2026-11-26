// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, expect, test, vi } from "vitest";
import type { AccountTabSummary, AgentAccounts, MentionAllowedSenders, SettingsPatch } from "../../domain/types";
import { AgentAccountsSection, AllowedSendersSection } from "./AgentAccountsSection";

afterEach(cleanup);

const tab = (user: string, name: string) => ({
  id: `account:${user}`, accountKey: user, homeserver: "example.invalid", displayName: name,
  avatarSourceRef: null, avatarMxcUri: null, status: "ready", unreadCount: 0
}) as AccountTabSummary;
const tabs = [tab("@alice:example.invalid", "Alice"), tab("@bot:example.invalid", "Bot")];

/** Applies each patch the way Rust does, so the panel is exercised through its own output. */
function Harness({ patches }: { patches: SettingsPatch[] }) {
  const [allowed, setAllowed] = useState<MentionAllowedSenders>({ added: [], removed: [] });
  const [agents, setAgents] = useState<AgentAccounts>({ accounts: {} });
  const update = (patch: SettingsPatch) => {
    patches.push(patch);
    if (patch.mcp_allowed_senders) setAllowed(patch.mcp_allowed_senders);
    if (patch.agent_account) {
      const accounts = { ...agents.accounts };
      if (patch.agent_account.role) accounts[patch.agent_account.user_id] = patch.agent_account.role;
      else delete accounts[patch.agent_account.user_id];
      setAgents({ accounts });
    }
  };
  return (
    <>
      <AgentAccountsSection agentAccounts={agents} accountTabs={tabs} isSaving={false} onUpdateSettings={update} />
      <AllowedSendersSection allowed={allowed} agentAccounts={agents} accountTabs={tabs} isSaving={false} onUpdateSettings={update} />
    </>
  );
}

test("the derived default is shown as implicit and the panel says what the list does not guarantee", () => {
  render(<Harness patches={[]} />);
  expect(screen.getAllByText("Signed-in account, included by default")).toHaveLength(2);
  expect(screen.getByText(/does not stop those senders' messages from being readable/)).toBeTruthy();
  expect(screen.getByText(/keeps mentions from other senders out of wait_for_mentions/)).toBeTruthy();
});

test("adding, excluding and restoring a sender round-trips through mcp_allowed_senders", () => {
  const patches: SettingsPatch[] = [];
  render(<Harness patches={patches} />);

  fireEvent.change(screen.getByTestId("allowed-sender-input"), { target: { value: "@carol:example.org" } });
  fireEvent.click(screen.getByTestId("allowed-sender-add"));
  expect(patches.at(-1)).toEqual({ scope: "app", mcp_allowed_senders: { added: ["@carol:example.org"], removed: [] } });
  expect(screen.getByText("Added by you")).toBeTruthy();

  // A default can be excluded (removal is expressible), and brought back.
  fireEvent.click(screen.getByRole("button", { name: "Exclude @bot:example.invalid" }));
  expect(patches.at(-1)).toEqual({
    scope: "app",
    mcp_allowed_senders: { added: ["@carol:example.org"], removed: ["@bot:example.invalid"] }
  });
  expect(screen.getByText("Excluded")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Include @bot:example.invalid again" }));
  expect(patches.at(-1)).toEqual({ scope: "app", mcp_allowed_senders: { added: ["@carol:example.org"], removed: [] } });

  fireEvent.click(screen.getByRole("button", { name: "Remove @carol:example.org" }));
  expect(patches.at(-1)).toEqual({ scope: "app", mcp_allowed_senders: { added: [], removed: [] } });
});

test("something that is not a Matrix user ID is refused without a patch", () => {
  const patches: SettingsPatch[] = [];
  render(<Harness patches={patches} />);
  fireEvent.change(screen.getByTestId("allowed-sender-input"), { target: { value: "carol" } });
  fireEvent.click(screen.getByTestId("allowed-sender-add"));
  expect(patches).toEqual([]);
  expect(screen.getByRole("alert").textContent).toMatch(/Matrix user ID/);
});

test("the agent role is marked per user id, offers no hiding, and leaves the agent out of the default list", () => {
  const patches: SettingsPatch[] = [];
  render(<Harness patches={patches} />);
  expect(screen.getAllByText("Signed-in account, included by default")).toHaveLength(2);
  fireEvent.click(screen.getByRole("switch", { name: "Bot is an agent account" }));
  expect(patches.at(-1)).toEqual({ scope: "app", agent_account: { user_id: "@bot:example.invalid", role: {} } });
  // No hide option, and the panel says why the agent is left out.
  expect(screen.queryByRole("switch", { name: /Hide the tab/ })).toBeNull();
  expect(screen.getAllByRole("switch")).toHaveLength(2);
  expect(screen.getAllByText("Signed-in account, included by default")).toHaveLength(1);
  expect(screen.getByText(/Agent account, left out by default/)).toBeTruthy();
  // An owner can still add the agent explicitly, and take it out again.
  fireEvent.click(screen.getByRole("button", { name: "Include @bot:example.invalid" }));
  expect(patches.at(-1)).toEqual({ scope: "app", mcp_allowed_senders: { added: ["@bot:example.invalid"], removed: [] } });
  expect(screen.getByText("Added by you")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Remove @bot:example.invalid" }));
  expect(patches.at(-1)).toEqual({ scope: "app", mcp_allowed_senders: { added: [], removed: [] } });
  fireEvent.click(screen.getByRole("switch", { name: "Bot is an agent account" }));
  expect(patches.at(-1)).toEqual({ scope: "app", agent_account: { user_id: "@bot:example.invalid", role: null } });
});

test("each agent-account row opens that account's settings", () => {
  const open = vi.fn();
  render(<AgentAccountsSection agentAccounts={{ accounts: {} }} accountTabs={tabs} isSaving={false} onUpdateSettings={vi.fn()} onOpenAccountSettings={open} />);
  fireEvent.click(screen.getByRole("button", { name: "Open settings for Bot" }));
  expect(open).toHaveBeenCalledWith("account:@bot:example.invalid");
});

test("a mark no tab covers is listed as not signed in and can be removed", () => {
  const patches: SettingsPatch[] = [];
  render(
    <AgentAccountsSection
      agentAccounts={{ accounts: { "@gone:example.invalid": {} } }}
      accountTabs={tabs}
      isSaving={false}
      onUpdateSettings={patch => patches.push(patch)}
    />
  );
  expect(screen.getByText("(not signed in)")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Remove the agent mark for @gone:example.invalid" }));
  expect(patches).toEqual([{ scope: "app", agent_account: { user_id: "@gone:example.invalid", role: null } }]);
});

test("the derived default lists only ready accounts, but a failed one can still be unmarked", () => {
  const failed = { ...tab("@locked:example.invalid", "Locked"), status: "error" } as AccountTabSummary;
  const all = [...tabs, failed];
  render(
    <>
      <AgentAccountsSection agentAccounts={{ accounts: {} }} accountTabs={all} isSaving={false} onUpdateSettings={vi.fn()} />
      <AllowedSendersSection allowed={{ added: [], removed: [] }} accountTabs={all} isSaving={false} onUpdateSettings={vi.fn()} />
    </>
  );
  expect(screen.getAllByText("Signed-in account, included by default")).toHaveLength(2);
  expect(screen.getByRole("switch", { name: "Locked is an agent account" })).toBeTruthy();
});
