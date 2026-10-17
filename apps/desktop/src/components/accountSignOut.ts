import { t } from "../i18n/messages";
import type { AccountTabSummary, SessionState } from "../domain/types";
import type { WindowConfirmOptions } from "../backend/windowDialogPort";

/** The account a tab is bound to, by display name, then user ID localpart. */
export function accountTabLabel(tab: AccountTabSummary): string {
  return (
    tab.displayName?.trim() ||
    tab.accountKey?.replace(/^@/, "").split(":", 1)[0] ||
    t("accountTabs.addAccount")
  );
}

/**
 * The account a destructive action targets: the bound tab, or, when no tab
 * list is published (single-account runtime), the session's own account.
 */
export function targetAccountTab(
  tab: AccountTabSummary | null | undefined,
  session: SessionState | null | undefined
): AccountTabSummary | null {
  if (tab) return tab;
  if (!session || !("user_id" in session) || typeof session.user_id !== "string") return null;
  const homeserver = "homeserver" in session && typeof session.homeserver === "string"
    ? session.homeserver
    : null;
  return {
    id: "",
    accountKey: session.user_id,
    homeserver,
    displayName: null,
    avatarSourceRef: null,
    status: "ready",
    unreadCount: 0
  };
}

/** Tabs whose account is bound and not signed out (#1504). */
export function isSignedInAccountTab(tab: AccountTabSummary): boolean {
  return tab.accountKey !== null && tab.status !== "signedOut" && tab.status !== "addAccount";
}

/** The label account-scoped menu items name, or null when no account is bound. */
export function signedInAccountLabel(tab: AccountTabSummary | null | undefined): string | null {
  return tab && isSignedInAccountTab(tab) ? accountTabLabel(tab) : null;
}

/**
 * #1504: the sign-out confirmation names the target account (display name,
 * full user ID, homeserver) and says whether the other accounts stay signed in,
 * so signing out one tab can't be mistaken for signing out of every account.
 */
export function accountSignOutConfirmation(
  tab: AccountTabSummary,
  tabs: readonly AccountTabSummary[],
  { removeTab }: { removeTab: boolean }
): { message: string; options: WindowConfirmOptions } {
  const account = accountTabLabel(tab);
  const othersSignedIn = tabs.some((other) => other.id !== tab.id && isSignedInAccountTab(other));
  const message = [
    t("settings.signOutTarget", {
      userId: tab.accountKey ?? account,
      homeserver: tab.homeserver ?? ""
    }),
    removeTab ? t("settings.signOutAndRemoveBody") : t("settings.signOutBody"),
    othersSignedIn ? t("settings.signOutOthersStay") : null
  ]
    .filter((part): part is string => part !== null)
    .join("\n\n");
  return {
    message,
    options: {
      title: removeTab
        ? t("settings.signOutAndRemoveTitle", { account })
        : t("settings.signOutAccountTitle", { account }),
      kind: "warning",
      okLabel: removeTab ? t("settings.signOutAndRemoveAction") : t("settings.signOutConfirmAction")
    }
  };
}

/** #1504: Reset identity names the account whose cryptographic identity resets. */
export function accountResetIdentityConfirmation(tab: AccountTabSummary): {
  message: string;
  options: WindowConfirmOptions;
} {
  return {
    message: [
      t("settings.signOutTarget", {
        userId: tab.accountKey ?? accountTabLabel(tab),
        homeserver: tab.homeserver ?? ""
      }),
      t("trust.resetIdentityConfirmBody")
    ].join("\n\n"),
    options: {
      title: t("trust.resetIdentityConfirmTitle", { account: accountTabLabel(tab) }),
      kind: "warning",
      okLabel: t("trust.resetIdentityConfirmAction")
    }
  };
}
