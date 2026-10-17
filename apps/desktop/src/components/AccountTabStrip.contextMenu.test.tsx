// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AccountTabStrip } from "./Shell";
import type { AccountTabSummary } from "../domain/types";
import { t } from "../i18n/messages";

function tab(overrides: Partial<AccountTabSummary> & Pick<AccountTabSummary, "id">): AccountTabSummary {
  return {
    accountKey: `@${overrides.id}:example.invalid`,
    homeserver: "https://example.invalid",
    displayName: null,
    avatarSourceRef: null,
    status: "ready",
    unreadCount: 0,
    inviteCount: 0,
    ...overrides
  };
}

const tabs = [
  tab({ id: "alice", displayName: "Alice" }),
  tab({ id: "bob", displayName: "Bob" }),
  tab({ id: "carol", displayName: "Carol", status: "signedOut" })
];

function renderStrip(onAccountTabAction = vi.fn()) {
  render(
    <AccountTabStrip
      tabs={tabs}
      selectedTabId="alice"
      onSelect={vi.fn()}
      onAdd={vi.fn()}
      onRemove={vi.fn()}
      onAccountTabAction={onAccountTabAction}
    />
  );
  return onAccountTabAction;
}

function tabButton(name: string): HTMLElement {
  return screen.getByRole("button", { name: new RegExp(`^${name}`) });
}

afterEach(cleanup);

describe("account tab context menu (#1504)", () => {
  it("acts on the right-clicked tab, not the selected one", () => {
    const onAction = renderStrip();
    fireEvent.contextMenu(tabButton("Bob"));
    expect(screen.getAllByRole("menuitem").map((item) => item.textContent)).toEqual([
      t("accountTabs.menuAccountSettings"),
      t("accountTabs.menuSignOut"),
      t("accountTabs.menuSignOutAndRemove")
    ]);
    fireEvent.click(screen.getByRole("menuitem", { name: t("accountTabs.menuSignOutAndRemove") }));
    expect(onAction).toHaveBeenCalledExactlyOnceWith("bob", "accountTabSignOutAndRemove");
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("offers sign in again and remove for a signed-out tab", () => {
    const onAction = renderStrip();
    fireEvent.contextMenu(tabButton("Carol"));
    expect(screen.getAllByRole("menuitem").map((item) => item.textContent)).toEqual([
      t("accountTabs.menuSignInAgain"),
      t("accountTabs.menuRemoveTab")
    ]);
    fireEvent.click(screen.getByRole("menuitem", { name: t("accountTabs.menuRemoveTab") }));
    expect(onAction).toHaveBeenCalledExactlyOnceWith("carol", "accountTabRemove");
  });

  it("opens from the keyboard with Shift+F10", () => {
    renderStrip();
    fireEvent.keyDown(tabButton("Bob"), { key: "F10", shiftKey: true });
    expect(screen.getByRole("menu")).toBeTruthy();
  });

  it("opens no menu without an action handler", () => {
    render(
      <AccountTabStrip
        tabs={tabs}
        selectedTabId="alice"
        onSelect={vi.fn()}
        onAdd={vi.fn()}
        onRemove={vi.fn()}
      />
    );
    fireEvent.contextMenu(tabButton("Bob"));
    expect(screen.queryByRole("menu")).toBeNull();
  });
});
