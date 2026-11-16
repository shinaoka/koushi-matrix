// @vitest-environment jsdom

import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";

const drag = vi.hoisted(() => vi.fn());
vi.mock("../backend/appRuntime", () => ({
  api: {},
  startSessionVerificationWindowDrag: drag
}));

import { PersistentAccountShell, TopBar } from "./Shell";

afterEach(() => {
  cleanup();
  drag.mockClear();
});

const tabs = {
  selectedTabId: "account:@member-one:example.invalid",
  tabs: [
    {
      id: "account:@member-one:example.invalid",
      accountKey: "@member-one:example.invalid",
      homeserver: "https://matrix.org",
      displayName: "Member 1",
      avatarSourceRef: null,
      status: "ready" as const,
      unreadCount: 0
    }
  ],
  badgeCount: 0
};

function renderShell(accountScopedContentReady: boolean) {
  return render(
    <PersistentAccountShell
      accountTabs={tabs}
      selectedAccountTabId={tabs.selectedTabId}
      accountScopedContentReady={accountScopedContentReady}
      onSelectAccountTab={() => undefined}
      onAddAccountTab={() => undefined}
      onRemoveSignedOutAccountTab={() => undefined}
      onOpenAppSettings={() => undefined}
      onOpenDiagnostics={() => undefined}
    >
      <main data-testid="account-content" />
    </PersistentAccountShell>
  );
}

// #1920: the pre-ready shell draws its own title bar. Dragging empty space
// beside the account tabs must move the window there too -- without a signed-in
// tab selected the title bar is the only thing a user has to drag, and the
// header's own mousedown suppresses the drag region the platform would
// otherwise pick up.
test("the pre-ready title bar starts a window drag on empty space", () => {
  const { container } = renderShell(false);
  const titlebar = container.querySelector<HTMLElement>(".titlebar");
  expect(titlebar).not.toBeNull();
  expect(titlebar!.getAttribute("data-tauri-drag-region")).toBe("");
  expect(titlebar!.dataset["accountContentReady"]).toBe("false");

  fireEvent.mouseDown(titlebar!, { buttons: 1, clientX: 60, clientY: 12 });

  expect(drag).toHaveBeenCalledTimes(1);
});

test("the pre-ready title bar leaves a title-bar control to itself", () => {
  const { container } = renderShell(false);
  const titlebar = container.querySelector<HTMLElement>(".titlebar");
  const addAccount = titlebar!.querySelector<HTMLElement>('button[aria-label="Add account"]');
  expect(addAccount).not.toBeNull();

  fireEvent.mouseDown(addAccount!, { buttons: 1 });

  expect(drag).not.toHaveBeenCalled();
});

// The ready shell renders its own top bar inside the account content, without
// passing a drag handler; it relies on the same default.
test("the ready title bar keeps starting a window drag", () => {
  const { container } = render(
    <TopBar accountTabs={tabs} selectedAccountTabId={tabs.selectedTabId} />
  );
  const titlebar = container.querySelector<HTMLElement>(".titlebar");
  expect(titlebar!.dataset["accountContentReady"]).toBe("true");

  fireEvent.mouseDown(titlebar!, { buttons: 1, clientX: 60, clientY: 12 });

  expect(drag).toHaveBeenCalledTimes(1);
});
