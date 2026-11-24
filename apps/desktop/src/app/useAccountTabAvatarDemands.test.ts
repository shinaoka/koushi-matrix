// @vitest-environment jsdom
import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, test, vi } from "vitest";

import type { AccountTabsSnapshot, AccountTabSummary } from "../domain/types";
import { useAccountTabAvatarDemands } from "./useAccountTabAvatarDemands";

const harness = vi.hoisted(() => {
  const downloadThumbnail = vi.fn(async () => undefined);
  return {
    downloadThumbnail,
    createPort: vi.fn(() => ({ downloadThumbnail }) as { downloadThumbnail: typeof downloadThumbnail } | null)
  };
});

vi.mock("../backend/tauri/desktopAccountTabAvatarPort", () => ({
  createTauriAccountTabAvatarPort: () => harness.createPort()
}));

beforeEach(() => {
  harness.downloadThumbnail.mockClear();
  harness.createPort.mockReset();
  harness.createPort.mockImplementation(() => ({ downloadThumbnail: harness.downloadThumbnail }));
});

function tab(overrides: Partial<AccountTabSummary> & Pick<AccountTabSummary, "id">): AccountTabSummary {
  return {
    accountKey: `@${overrides.id}:example.invalid`,
    homeserver: "https://example.invalid",
    displayName: overrides.id,
    avatarMxcUri: `mxc://example.invalid/${overrides.id}`,
    avatarSourceRef: null,
    status: "ready",
    unreadCount: 0,
    ...overrides
  };
}

/** The signed-in account, always present and always the selected one. */
const SELECTED = tab({ id: "selected" });

function snapshot(otherTabs: AccountTabSummary[] = []): AccountTabsSnapshot {
  return { selectedTabId: SELECTED.id, tabs: [SELECTED, ...otherTabs], badgeCount: 0 };
}

describe("useAccountTabAvatarDemands", () => {
  // #2059: the selected account gets its avatar from its timeline, on that
  // account's own transport. This hook owns the other tabs only.
  test("demands a background account's avatar through its own tab", () => {
    const { rerender } = renderHook(
      ({ tabs }: { tabs: AccountTabsSnapshot | null }) => useAccountTabAvatarDemands(tabs),
      { initialProps: { tabs: null as AccountTabsSnapshot | null } }
    );

    rerender({ tabs: snapshot([tab({ id: "background" })]) });

    expect(harness.downloadThumbnail.mock.calls).toEqual([
      ["background", "mxc://example.invalid/background"]
    ]);
  });

  test("leaves a tab without a session alone until it is ready", () => {
    const { rerender } = renderHook(
      ({ tabs }: { tabs: AccountTabsSnapshot }) => useAccountTabAvatarDemands(tabs),
      { initialProps: { tabs: snapshot([tab({ id: "background", status: "restoring" })]) } }
    );

    // No client exists yet and Core would drop the update, so the one demand the
    // tab gets must not be spent on it.
    expect(harness.downloadThumbnail).not.toHaveBeenCalled();

    rerender({ tabs: snapshot([tab({ id: "background" })]) });

    expect(harness.downloadThumbnail.mock.calls).toEqual([
      ["background", "mxc://example.invalid/background"]
    ]);
  });

  test("does not demand again for a tab that already has a ready thumbnail", () => {
    const tabs = snapshot([tab({ id: "background", avatarSourceRef: "avatar/0123456789abcdef" })]);
    const { rerender } = renderHook(() => useAccountTabAvatarDemands(tabs));

    rerender();
    rerender();

    expect(harness.downloadThumbnail).not.toHaveBeenCalled();
  });

  test("asks once per account and resource, and again when the avatar changes", () => {
    const { rerender } = renderHook(
      ({ tabs }: { tabs: AccountTabsSnapshot }) => useAccountTabAvatarDemands(tabs),
      { initialProps: { tabs: snapshot([tab({ id: "background" })]) } }
    );
    rerender({ tabs: snapshot([tab({ id: "background" })]) });
    expect(harness.downloadThumbnail).toHaveBeenCalledTimes(1);

    rerender({ tabs: snapshot([tab({ id: "background", avatarMxcUri: "mxc://example.invalid/new" })]) });
    expect(harness.downloadThumbnail).toHaveBeenCalledTimes(2);
    expect(harness.downloadThumbnail).toHaveBeenLastCalledWith("background", "mxc://example.invalid/new");
  });

  test("leaves an account without an avatar alone", () => {
    renderHook(() => useAccountTabAvatarDemands(snapshot([tab({ id: "plain", avatarMxcUri: null })])));

    expect(harness.downloadThumbnail).not.toHaveBeenCalled();
  });

  test("retries when the adapter refused before Core owned the request", async () => {
    harness.downloadThumbnail.mockRejectedValueOnce(new Error("account tab does not exist"));
    const { rerender } = renderHook(
      ({ tabs }: { tabs: AccountTabsSnapshot }) => useAccountTabAvatarDemands(tabs),
      { initialProps: { tabs: snapshot([tab({ id: "background" })]) } }
    );
    await Promise.resolve();

    rerender({ tabs: snapshot([tab({ id: "background" })]) });

    expect(harness.downloadThumbnail).toHaveBeenCalledTimes(2);
  });

  test("does nothing when the app is not running in Tauri", () => {
    harness.createPort.mockImplementation(() => null);

    renderHook(() => useAccountTabAvatarDemands(snapshot([tab({ id: "background" })])));

    expect(harness.downloadThumbnail).not.toHaveBeenCalled();
  });
});
