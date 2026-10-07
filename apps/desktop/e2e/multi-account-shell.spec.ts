import { expect, test } from "@playwright/test";
import type { AccountTabsSnapshot } from "../src/domain/types";
import { gotoReadyShell } from "./support/basicOperations";

const accountId = "harness-account-tab";
const addId = "new-account-tab";
const bobId = "bob-account-tab";

function accountTabs(
  includeAdd: boolean,
  includeBob = false,
  bobStatus: "needsVerification" | "ready" = "needsVerification"
): AccountTabsSnapshot {
  return {
    selectedTabId: accountId,
    tabs: [
      {
        id: accountId,
        accountKey: "@harness-user:example.invalid",
        homeserver: "https://harness.example.invalid",
        displayName: "Harness",
        avatarSourceRef: null,
        status: "ready",
        unreadCount: 0
      },
      ...(includeBob
        ? [{
            id: bobId,
            accountKey: "@bob:example.invalid",
            homeserver: "https://bob.example.invalid",
            displayName: "Bob",
            avatarSourceRef: null,
            status: bobStatus,
            unreadCount: 0
          }]
        : []),
      ...(includeAdd
        ? [{
            id: addId,
            accountKey: null,
            homeserver: "https://harness.example.invalid",
            displayName: "New account",
            avatarSourceRef: null,
            status: "addAccount" as const,
            unreadCount: 0
          }]
        : [])
    ],
    badgeCount: 0
  };
}

test("adding an account and a verification gate leave other accounts available", async ({ page }) => {
  await gotoReadyShell(page);
  const withAdd = accountTabs(true);

  await page.evaluate((tabs) => {
    const harness = window.__harness as any;
    const setSession = (session: unknown) => {
      const snapshot = harness.currentSnapshot();
      harness.setSnapshot({
        ...snapshot,
        state: {
          ...snapshot.state,
          domain: { ...snapshot.state.domain, session }
        }
      });
    };

    harness.setCommandResponse("add_account_tab", () => {
      setSession({ kind: "signedOut", homeserver: "https://harness.example.invalid" });
      return { ...tabs, selectedTabId: "new-account-tab" };
    });
    harness.setCommandResponse("select_account_tab", ({ tabId }: { tabId: string }) => {
      setSession({ kind: "ready", homeserver: "https://harness.example.invalid" });
      return { ...tabs, selectedTabId: tabId };
    });
  }, withAdd);

  await page.getByRole("button", { name: "Add account", exact: true }).click();
  await expect(page.getByTestId("auth-screen")).toBeVisible();
  await expect(page.getByRole("button", { name: "Harness: Ready", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "New account: Add account", exact: true }))
    .toHaveAttribute("aria-current", "page");
  await expect(page.getByRole("button", { name: "App Settings", exact: true })).toBeVisible();

  await page.getByRole("button", { name: "Harness: Ready", exact: true }).click();
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();

  const withBob = accountTabs(true, true);
  await page.evaluate((tabs) => {
    const harness = window.__harness as any;
    const setSession = (session: unknown) => {
      const snapshot = harness.currentSnapshot();
      harness.setSnapshot({
        ...snapshot,
        state: {
          ...snapshot.state,
          domain: { ...snapshot.state.domain, session }
        }
      });
    };

    harness.setCommandResponse("select_account_tab", ({ tabId }: { tabId: string }) => {
      setSession(tabId === "bob-account-tab"
        ? {
            kind: "awaitingVerification",
            homeserver: "https://bob.example.invalid",
            user_id: "@bob:example.invalid",
            device_id: "BOBDEVICE",
            gate: {
              methods: ["existingDeviceSas", "recoveryKey", "bootstrap"],
              account_kind: "existingIdentity",
              failureKind: null
            }
          }
        : { kind: "ready", homeserver: "https://harness.example.invalid" });
      return { ...tabs, selectedTabId: tabId };
    });
    void harness.pushAccountTabs(tabs);
  }, withBob);

  const bobTab = page.getByRole("button", { name: "Bob: Needs verification", exact: true });
  await expect(bobTab).toBeVisible();
  await bobTab.click();
  await expect(page.locator("main.session-verification-gate")).toBeVisible();
  await expect(page.getByRole("button", { name: "Harness: Ready", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "App Settings", exact: true })).toBeVisible();

  await page.getByRole("button", { name: "Harness: Ready", exact: true }).click();
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  await expect(page.locator("main.session-verification-gate")).toHaveCount(0);
});

for (const control of ["auth Cancel button", "tab close button"] as const) {
  test(`cancelling an unfinished add-account tab via the ${control} returns to the previous account`, async ({
    page
  }) => {
    await gotoReadyShell(page);
    const withAdd = accountTabs(true);
    const withoutAdd = accountTabs(false);

    await page.evaluate(({ tabs, previousTabs }) => {
      const harness = window.__harness as any;
      const setSession = (session: unknown) => {
        const snapshot = harness.currentSnapshot();
        harness.setSnapshot({
          ...snapshot,
          state: {
            ...snapshot.state,
            domain: { ...snapshot.state.domain, session, auth: { kind: "unknown" } }
          }
        });
      };
      harness.setCommandResponse("add_account_tab", () => {
        setSession({ kind: "signedOut" });
        return { ...tabs, selectedTabId: "new-account-tab" };
      });
      harness.setCommandResponse("cancel_add_account_tab", () => {
        setSession({ kind: "ready", homeserver: "https://harness.example.invalid" });
        return previousTabs;
      });
    }, { tabs: withAdd, previousTabs: withoutAdd });

    await page.getByRole("button", { name: "Add account", exact: true }).click();
    await expect(page.getByTestId("auth-screen")).toBeVisible();
    await page.evaluate(() => window.__harness.clearInvocations());

    if (control === "auth Cancel button") {
      await page.getByTestId("auth-screen").getByRole("button", { name: "Cancel", exact: true }).click();
    } else {
      await page.getByRole("button", { name: "Cancel adding account", exact: true }).click();
    }

    await expect
      .poll(() => page.evaluate(() => window.__harness.invocationsOf("cancel_add_account_tab").map((call) => call.args)))
      .toEqual([{ tabId: "new-account-tab" }]);
    expect(await page.evaluate(() => window.__harness.invocationsOf("remove_signed_out_account_tab").length)).toBe(0);
    expect(await page.evaluate(() => window.__harness.invocationsOf("logout").length)).toBe(0);
    await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Harness: Ready", exact: true }))
      .toHaveAttribute("aria-current", "page");
    await expect(page.getByRole("button", { name: "New account: Add account", exact: true })).toHaveCount(0);
  });
}

test("switching accounts flushes the active composer draft on its owning tab", async ({ page }) => {
  await gotoReadyShell(page);
  const tabs = accountTabs(false, true, "ready");
  await page.evaluate(async (nextTabs) => {
    const harness = window.__harness as any;
    harness.setCommandResponse("select_account_tab", ({ tabId }: { tabId: string }) => ({
      ...nextTabs,
      selectedTabId: tabId
    }));
    await harness.pushAccountTabs(nextTabs);
    harness.clearInvocations();
  }, tabs);

  const composer = page.getByRole("textbox", { name: "Message composer" });
  await composer.fill("draft before switching accounts");
  await expect
    .poll(() => page.evaluate(() => window.__harness.invocationsOf("set_composer_draft").length))
    .toBe(1);
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.getByRole("button", { name: "Bob: Ready", exact: true }).click();
  await expect(page.getByRole("button", { name: "Bob: Ready", exact: true }))
    .toHaveAttribute("aria-current", "page");

  const invocations = await page.evaluate(() => window.__harness.invocations());
  const flushIndex = invocations.findIndex((call) => call.command === "set_composer_draft");
  const switchIndex = invocations.findIndex((call) => call.command === "select_account_tab");
  expect(flushIndex).toBeGreaterThanOrEqual(0);
  expect(switchIndex).toBeGreaterThan(flushIndex);
  expect(invocations[flushIndex]?.args).toMatchObject({
    accountTabId: accountId,
    document: {
      inlines: [{ kind: "text", text: "draft before switching accounts" }]
    }
  });
});

test("notification activation navigates and settles in the target account context", async ({ page }) => {
  await gotoReadyShell(page);
  const tabs = accountTabs(false, true, "ready");
  await page.evaluate(async (nextTabs) => {
    const harness = window.__harness as any;
    harness.setCommandResponse("open_notification_event", () => harness.currentSnapshot());
    harness.setCommandResponse("settlement_snapshot", () => harness.currentSnapshot());
    harness.setCommandResponse("select_account_tab", ({ tabId }: { tabId: string }) => {
      const snapshot = harness.currentSnapshot();
      harness.setSnapshot({
        ...snapshot,
        account_tab_id: tabId,
        state: {
          ...snapshot.state,
          domain: {
            ...snapshot.state.domain,
            session: {
              ...snapshot.state.domain.session,
              kind: "ready",
              homeserver: "https://bob.example.invalid",
              user_id: "@bob:example.invalid",
              device_id: "BOBDEVICE"
            }
          }
        }
      });
      return { ...nextTabs, selectedTabId: tabId };
    });
    await harness.pushAccountTabs(nextTabs);
    harness.clearInvocations();
  }, tabs);

  await page.evaluate(async () => {
    await window.__harness.pushNotificationActivation({
      account_tab_id: "bob-account-tab",
      room_id: "!harness-room:example.invalid",
      event_id: "$notification-event:example.invalid",
      thread_root_event_id: null
    });
  });

  await expect
    .poll(() => page.evaluate(() => window.__harness.invocationsOf("open_notification_event").length))
    .toBe(1);
  const invocations = await page.evaluate(() => window.__harness.invocations());
  expect(invocations.find((call) => call.command === "open_notification_event")?.args.accountTabId)
    .toBe(bobId);
  // Every settlement that follows the activation must reconcile in the target
  // account's context. Asserting only the first `settlement_snapshot`
  // invocation was order-dependent: settling the preceding `select_account_tab`
  // command legitimately uses the previously selected tab, so whether it landed
  // before or after the activation decided the outcome (#119 flake).
  const activationIndex = invocations.findIndex(
    (call) => call.command === "open_notification_event"
  );
  expect(activationIndex).toBeGreaterThanOrEqual(0);
  const settlementsAfterActivation = invocations
    .slice(activationIndex)
    .filter((call) => call.command === "settlement_snapshot");
  expect(settlementsAfterActivation.length).toBeGreaterThan(0);
  expect(
    settlementsAfterActivation.every((call) => call.args.accountTabId === bobId)
  ).toBe(true);
});
