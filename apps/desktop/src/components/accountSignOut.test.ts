import { describe, expect, it } from "vitest";
import {
  accountResetIdentityConfirmation,
  accountSignOutConfirmation,
  signedInAccountLabel,
  targetAccountTab
} from "./accountSignOut";
import type { AccountTabSummary } from "../domain/types";
import { t } from "../i18n/messages";

function tab(overrides: Partial<AccountTabSummary> & Pick<AccountTabSummary, "id">): AccountTabSummary {
  return {
    accountKey: `@${overrides.id}:example.invalid`,
    homeserver: "https://matrix.example.invalid",
    displayName: null,
    avatarSourceRef: null,
    status: "ready",
    unreadCount: 0,
    inviteCount: 0,
    ...overrides
  };
}

describe("account-scoped confirmations name their account (#1504)", () => {
  const alice = tab({ id: "alice", displayName: "Alice" });
  const bob = tab({ id: "bob", displayName: "Bob" });

  it("names the account, its user ID and homeserver when signing out", () => {
    const { message, options } = accountSignOutConfirmation(alice, [alice, bob], {
      removeTab: false
    });
    expect(options.title).toBe(t("settings.signOutAccountTitle", { account: "Alice" }));
    expect(options.okLabel).toBe(t("settings.signOutConfirmAction"));
    expect(message).toContain("@alice:example.invalid");
    expect(message).toContain("https://matrix.example.invalid");
    expect(message).toContain(t("settings.signOutOthersStay"));
  });

  it("says the tab is removed, and omits the others line for the only account", () => {
    const { message, options } = accountSignOutConfirmation(alice, [alice], { removeTab: true });
    expect(options.title).toBe(t("settings.signOutAndRemoveTitle", { account: "Alice" }));
    expect(options.okLabel).toBe(t("settings.signOutAndRemoveAction"));
    expect(message).toContain(t("settings.signOutAndRemoveBody"));
    expect(message).not.toContain(t("settings.signOutOthersStay"));
  });

  it("does not count signed-out tabs as accounts that stay signed in", () => {
    const carol = tab({ id: "carol", status: "signedOut" });
    const { message } = accountSignOutConfirmation(alice, [alice, carol], { removeTab: false });
    expect(message).not.toContain(t("settings.signOutOthersStay"));
  });

  it("names the account whose identity is reset", () => {
    const { message, options } = accountResetIdentityConfirmation(bob);
    expect(options.title).toBe(t("trust.resetIdentityConfirmTitle", { account: "Bob" }));
    expect(message).toContain("@bob:example.invalid");
  });

  it("names menu items only for a signed-in account", () => {
    expect(signedInAccountLabel(tab({ id: "dave" }))).toBe("dave");
    expect(signedInAccountLabel(tab({ id: "erin", status: "signedOut" }))).toBeNull();
    expect(signedInAccountLabel(null)).toBeNull();
  });
});

describe("targetAccountTab", () => {
  it("prefers the bound tab and otherwise names the session's own account", () => {
    const bound = tab({ id: "a", accountKey: "@alice:example.invalid" });
    expect(targetAccountTab(bound, null)).toBe(bound);
    expect(
      targetAccountTab(undefined, {
        kind: "ready",
        homeserver: "https://matrix.example.invalid",
        user_id: "@solo:example.invalid",
        device_id: "DEVICE"
      } as never)
    ).toMatchObject({
      accountKey: "@solo:example.invalid",
      homeserver: "https://matrix.example.invalid"
    });
    expect(targetAccountTab(null, { kind: "signedOut" } as never)).toBeNull();
  });
});
