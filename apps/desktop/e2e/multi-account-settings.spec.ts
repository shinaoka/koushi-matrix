import { expect, test } from "@playwright/test";
import type { AccountTabsSnapshot, DesktopSnapshot } from "../src/domain/types";

const accountTabs: AccountTabsSnapshot = {
  selectedTabId: "account:@harness-user:example.invalid",
  tabs: [
    {
      id: "account:@harness-user:example.invalid",
      accountKey: "@harness-user:example.invalid",
      homeserver: "https://harness.example.invalid",
      displayName: "Harness",
      avatarSourceRef: null,
      status: "ready",
      unreadCount: 0
    },
    {
      id: "account:@bob:example.invalid",
      accountKey: "@bob:example.invalid",
      homeserver: "https://bob.example.invalid",
      displayName: "Bob",
      avatarSourceRef: null,
      status: "ready",
      unreadCount: 0
    }
  ],
  badgeCount: 0
};

test("switching tabs during logout keeps Account Settings bound to the new account", async ({
  page
}) => {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  await page.evaluate(async (tabs) => {
    const harness = (window as unknown as {
      __harness: {
        clearInvocations(): void;
        setCommandResponse(command: string, response: unknown): void;
        deferCommand(command: string): void;
        pushAccountTabs(snapshot: AccountTabsSnapshot): Promise<void>;
        currentSnapshot(): DesktopSnapshot;
        setSnapshot(snapshot: DesktopSnapshot): void;
        pushStateUpdate(): void;
      };
    }).__harness;
    harness.setCommandResponse("list_account_tabs", tabs);
    harness.setCommandResponse("select_account_tab", ({ tabId }: { tabId: string }) => {
      const selected = { ...tabs, selectedTabId: tabId };
      const current = harness.currentSnapshot();
      harness.setSnapshot({
        ...current,
        state: {
          ...current.state,
          domain: {
            ...current.state.domain,
            session: {
              ...current.state.domain.session,
              homeserver: "https://bob.example.invalid",
              user_id: "@bob:example.invalid",
              device_id: "BOBDEVICE"
            }
          }
        }
      });
      harness.pushStateUpdate();
      return selected;
    });
    harness.deferCommand("logout");
    harness.clearInvocations();
    await harness.pushAccountTabs(tabs);
  }, accountTabs);
  await expect(page.getByRole("button", { name: "Bob: Ready", exact: true })).toBeVisible();

  const settingsButton = page.getByRole("button", { name: "Account Settings", exact: true });
  await settingsButton.click();
  const dialog = page.getByRole("dialog", { name: "Account Settings", exact: true });
  await dialog.getByRole("tab", { name: "Sessions", exact: true }).click();
  await dialog.getByRole("button", { name: "Sign out", exact: true }).click();
  await expect.poll(() => page.evaluate(() =>
    (window as unknown as { __harness: { invocationsOf(command: string): unknown[] } })
      .__harness.invocationsOf("logout").length
  )).toBe(1);
  const logoutAccountTabId = await page.evaluate(() =>
    (window as unknown as {
      __harness: {
        invocations(): readonly { command: string; args: Record<string, unknown> }[];
      };
    }).__harness.invocations().find(({ command }) => command === "logout")?.args.accountTabId
  );
  expect(logoutAccountTabId).toBe(accountTabs.selectedTabId);

  await dialog.getByRole("button", { name: "Bob: Ready", exact: true }).click();
  await page.evaluate(() => {
    (window as unknown as {
      __harness: {
        resolveDeferredCommand(command: string, index: number, value: unknown): void;
      };
    }).__harness.resolveDeferredCommand("logout", 0, {
      protocolVersion: 1,
      publishedGeneration: 1
    });
  });

  await expect(dialog.locator(".settings-account-owner")).toContainText("@bob:example.invalid");
  await dialog.getByRole("tab", { name: "Sessions", exact: true }).click();
  await expect(dialog.getByRole("button", { name: "Sign out", exact: true })).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Bob: Ready", exact: true }))
    .toHaveAttribute("aria-current", "page");
});
