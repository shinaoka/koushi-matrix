// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { AccountTabSummary, McpAssistantsState, McpPendingApproval } from "../domain/types";
import { McpApprovalDialog, McpApprovalPrompt } from "./McpApprovalDialog";

const fixture = vi.hoisted(() => ({
  listeners: new Set<(state: McpAssistantsState) => void>(),
  getState: vi.fn(),
  answer: vi.fn()
}));
vi.mock("../backend/runtimeEnvironment", () => ({ isTauriRuntime: () => true }));
vi.mock("../backend/appRuntime", () => ({
  api: { getMcpAssistantsState: fixture.getState, answerMcpApproval: fixture.answer },
  startSessionVerificationWindowDrag: vi.fn()
}));
vi.mock("../backend/desktopEventRuntime", () => ({ desktopEventPort: {
  listenMcpAssistants: async (listener: (state: McpAssistantsState) => void) => {
    fixture.listeners.add(listener);
    return () => fixture.listeners.delete(listener);
  }
} }));

const WORK = "account:@member1:example.invalid";
const tabs: AccountTabSummary[] = [{
  id: WORK,
  accountKey: "@member1:example.invalid",
  homeserver: "example.invalid",
  displayName: "Synthetic Workspace",
  avatarSourceRef: null,
  status: "ready",
  unreadCount: 0
} as AccountTabSummary];

function request(overrides: Partial<McpPendingApproval> = {}): McpPendingApproval {
  return {
    request_id: 1,
    client_name: "claude-code",
    client_version: "2.1.0",
    account_id: WORK,
    need: "read",
    room_label: null,
    message_preview: null,
    waiting: true,
    waiting_calls: 1,
    ...overrides
  };
}

function state(pending: McpPendingApproval[]): McpAssistantsState {
  return {
    bridge: "running",
    store: "ok",
    launcher: { state: "available", path: "/Applications/Koushi.app/Contents/MacOS/koushi-mcp" },
    pending,
    assistants: [],
    registrations: [],
    outdated_launchers: []
  };
}

beforeEach(() => {
  fixture.getState.mockResolvedValue(state([]));
  fixture.answer.mockResolvedValue("applied");
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  vi.useRealTimers();
});

test("shows the oldest pending approval with the account named by Koushi", async () => {
  render(<McpApprovalDialog accountTabs={tabs} />);
  await waitFor(() => expect(fixture.getState).toHaveBeenCalledOnce());
  act(() => fixture.listeners.forEach(listener => listener(state([request(), request({ request_id: 2, need: "send" })]))));

  const dialog = screen.getByRole("dialog", { name: "Allow claude-code to read your messages?" });
  expect(dialog.textContent).toContain("wants to read Synthetic Workspace");
  expect(dialog.textContent).toContain("@member1:example.invalid");
  expect(dialog.textContent).toContain("is the name the assistant gave itself");
  expect(dialog.textContent).toContain("More requests waiting: 1");
});

test("Allow is not focused and ignores activation until armed", () => {
  vi.useFakeTimers();
  render(<McpApprovalPrompt request={request()} accountTabs={tabs} moreWaiting={0} onClose={() => undefined} />);
  const allow = screen.getByRole("button", { name: "Allow reading Synthetic Workspace" }) as HTMLButtonElement;
  expect(document.activeElement).not.toBe(allow);
  expect(allow.disabled).toBe(true);
  fireEvent.click(allow);
  expect(fixture.answer).not.toHaveBeenCalled();

  act(() => vi.advanceTimersByTime(1000));
  expect(allow.disabled).toBe(false);
  fireEvent.click(allow);
  expect(fixture.answer).toHaveBeenCalledWith(1, true);
});

test("Deny answers the request by id", () => {
  render(<McpApprovalPrompt request={request({ request_id: 9 })} accountTabs={tabs} moreWaiting={0} onClose={() => undefined} />);
  fireEvent.click(screen.getByRole("button", { name: "Deny" }));
  expect(fixture.answer).toHaveBeenCalledWith(9, false);
});

test("a send request shows the waiting message as context and names the standing permission", () => {
  render(
    <McpApprovalPrompt
      request={request({ need: "send", room_label: "Synthetic Room", message_preview: "Fixture reply text" })}
      accountTabs={tabs}
      moreWaiting={0}
      armingDelayMs={0}
      onClose={() => undefined}
    />
  );
  expect(screen.getByText("The message it is waiting to send, in Synthetic Room:")).toBeTruthy();
  expect(screen.getByText("Fixture reply text")).toBeTruthy();
  expect(screen.getByText(/without asking again/)).toBeTruthy();
  expect(screen.getByRole("button", { name: "Allow sending from Synthetic Workspace" })).toBeTruthy();
});

test("a send whose caller went away says the message will not be sent", () => {
  render(
    <McpApprovalPrompt
      request={request({ need: "send", message_preview: null, waiting: false, waiting_calls: 0 })}
      accountTabs={tabs}
      moreWaiting={0}
      onClose={() => undefined}
    />
  );
  expect(screen.getByText(/that message will not be sent/)).toBeTruthy();
});

test("several calls waiting on one approval are counted", () => {
  render(
    <McpApprovalPrompt
      request={request({ need: "send", message_preview: "Newest text", waiting_calls: 3 })}
      accountTabs={tabs}
      moreWaiting={0}
      onClose={() => undefined}
    />
  );
  expect(screen.getByText("Newest text")).toBeTruthy();
  expect(screen.getByText("Other calls from this assistant waiting for this answer: 2")).toBeTruthy();
});

test("a message-actions approval states what it may and may not change (#1628)", () => {
  render(
    <McpApprovalPrompt
      request={request({ need: "message_actions" })}
      accountTabs={tabs}
      moreWaiting={0}
      armingDelayMs={0}
      onClose={() => undefined}
    />
  );
  const dialog = screen.getByRole("dialog", { name: "Allow claude-code to change your messages?" });
  expect(dialog.textContent).toContain("wants to react to, edit, and delete your messages in Synthetic Workspace");
  expect(dialog.textContent).toContain("If you allow, claude-code can:");
  expect(dialog.textContent).toContain("Add and remove its own reactions on any message it can see");
  expect(dialog.textContent).toContain("Edit the text of messages you sent, keeping any image or file");
  expect(dialog.textContent).toContain("Delete messages you sent");
  expect(dialog.textContent).toContain("It cannot change anyone else's message");
  expect(dialog.textContent).toContain("looks like your own action in Synthetic Workspace");
  expect(screen.getByRole("button", { name: "Allow changing messages in Synthetic Workspace" })).toBeTruthy();
});

test("the send approval names thread replies too (#1628)", () => {
  render(
    <McpApprovalPrompt
      request={request({ need: "send", message_preview: "Fixture reply text" })}
      accountTabs={tabs}
      moreWaiting={0}
      onClose={() => undefined}
    />
  );
  expect(screen.getByText(/messages, replies, and thread replies/)).toBeTruthy();
});

test("an approval that could not be saved says so", async () => {
  fixture.answer.mockResolvedValue("store_failed");
  render(<McpApprovalPrompt request={request()} accountTabs={tabs} moreWaiting={0} armingDelayMs={0} onClose={() => undefined} />);
  fireEvent.click(screen.getByRole("button", { name: "Allow reading Synthetic Workspace" }));
  expect(await screen.findByRole("alert")).toBeTruthy();
});

test("closing hides that request until a new one arrives; listeners are released", async () => {
  const view = render(<McpApprovalDialog accountTabs={tabs} />);
  await waitFor(() => expect(fixture.getState).toHaveBeenCalledOnce());
  act(() => fixture.listeners.forEach(listener => listener(state([request()]))));
  fireEvent.click(screen.getByRole("button", { name: /^Close / }));
  expect(screen.queryByRole("dialog")).toBeNull();

  act(() => fixture.listeners.forEach(listener => listener(state([request(), request({ request_id: 2 })]))));
  expect(screen.getByRole("dialog")).toBeTruthy();
  view.unmount();
  expect(fixture.listeners.size).toBe(0);
});

test("an account signed in this run is named although its tab keeps a sign-in id", () => {
  const signInTab = { ...tabs[0], id: "add:1" } as AccountTabSummary;
  render(<McpApprovalPrompt request={request()} accountTabs={[signInTab]} moreWaiting={0} onClose={() => undefined} />);
  expect(screen.getByText("wants to read Synthetic Workspace")).toBeTruthy();
  expect(screen.queryByText("Account not open in Koushi")).toBeNull();
});

test("a draft approval previews the draft and says it cannot send (#1644)", () => {
  render(
    <McpApprovalPrompt
      request={request({
        need: "draft",
        room_label: "Synthetic Room",
        message_preview: "Fixture draft text"
      })}
      accountTabs={tabs}
      moreWaiting={0}
      armingDelayMs={0}
      onClose={() => undefined}
    />
  );
  const dialog = screen.getByRole("dialog", { name: "Allow claude-code to write drafts?" });
  expect(dialog.textContent).toContain("wants to put message drafts in your composers in Synthetic Workspace");
  expect(dialog.textContent).toContain("The draft it is waiting to add, in Synthetic Room:");
  expect(screen.getByText("Fixture draft text")).toBeTruthy();
  expect(dialog.textContent).toContain("It cannot send them");
  expect(dialog.textContent).toContain("never replaces text you are writing");
  expect(screen.getByRole("button", { name: "Allow drafts in Synthetic Workspace" })).toBeTruthy();
});
