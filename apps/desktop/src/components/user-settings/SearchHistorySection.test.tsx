// @vitest-environment jsdom

/**
 * #1100: the search-database rebuild guard is an in-app confirmation.
 *
 * `window.confirm` never renders in the Tauri macOS webview, so the destructive
 * rebuild looked like a dead button. The guard must open the shared in-app
 * confirmation, and only its confirm action may start the rebuild.
 */
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";

import { t } from "../../i18n/messages";
import type { RoomSummary, SearchCrawlerSettings, SearchCrawlerState } from "../../domain/types";
import { SearchHistorySection } from "./SearchHistorySection";

afterEach(cleanup);

const settings: SearchCrawlerSettings = {
  speed: "standard",
  include_media_captions: true,
  include_filenames: true
};

const crawlerState: SearchCrawlerState = { rooms: {}, last_active: null };

function renderSection(onRebuildSearchIndex: () => void) {
  return render(
    <SearchHistorySection
      crawlerSettings={settings}
      crawlerState={crawlerState}
      rooms={[]}
      isSaving={false}
      onUpdateSettings={() => undefined}
      onRebuildSearchIndex={onRebuildSearchIndex}
      onStartCrawlRoom={() => undefined}
      onStopCrawlRoom={() => undefined}
    />
  );
}

const rooms = [
  { room_id: "!alpha:example.invalid", display_label: "Alpha room" } as RoomSummary,
  { room_id: "!beta:example.invalid", display_label: "Beta room" } as RoomSummary
];

const populatedState: SearchCrawlerState = {
  rooms: {
    "!alpha:example.invalid": { kind: "completed", indexed: 633 },
    "!beta:example.invalid": { kind: "running", processed: 12, indexed: 6 }
  },
  last_active: {
    room_id: "!alpha:example.invalid",
    status: "completed",
    updated_at_ms: Date.now(),
    processed: 700,
    indexed: 633
  }
};

function renderScoped({
  settingsScope,
  onOpenAccountSettings
}: {
  settingsScope: "account" | "app";
  onOpenAccountSettings?: () => void;
}) {
  return render(
    <SearchHistorySection
      crawlerSettings={settings}
      crawlerState={populatedState}
      settingsScope={settingsScope}
      rooms={rooms}
      isSaving={false}
      onUpdateSettings={() => undefined}
      onRebuildSearchIndex={() => undefined}
      onStartCrawlRoom={() => undefined}
      onStopCrawlRoom={() => undefined}
      onOpenAccountSettings={onOpenAccountSettings}
    />
  );
}

// App Settings owns the crawl speed and pause only, so the crawl status has to
// stay visible there as a read-only summary with one way into the account scope
// that owns the room-level controls.
describe("SearchHistorySection settings scopes", () => {
  test("app settings show the crawl status read-only and a way into account settings", () => {
    const onOpenAccountSettings = vi.fn();
    renderScoped({ settingsScope: "app", onOpenAccountSettings });

    expect(
      screen.getByText(
        t("settings.searchHistoryActivitySummary", { running: 1, queued: 0, completed: 1, failed: 0 })
      )
    ).toBeTruthy();
    expect(
      screen.getByText(
        t("settings.searchHistoryActivityLastIndexed", {
          room: "Alpha room",
          age: t("settings.searchHistoryActivityJustNow")
        })
      )
    ).toBeTruthy();
    // Crawl operations stay in account settings.
    expect(screen.queryByRole("button", { name: t("settings.searchHistoryRebuild") })).toBeNull();
    expect(screen.queryByRole("button", { name: t("settings.searchHistoryReindexRoom") })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: t("settings.accountSettings") }));
    expect(onOpenAccountSettings).toHaveBeenCalledTimes(1);
  });

  test("app settings omit the account-settings shortcut when scope cannot change", () => {
    renderScoped({ settingsScope: "app" });

    expect(screen.getByText(t("settings.searchHistoryActivitySummary", {
      running: 1,
      queued: 0,
      completed: 1,
      failed: 0
    }))).toBeTruthy();
    expect(screen.queryByRole("button", { name: t("settings.accountSettings") })).toBeNull();
  });

  test("account settings keep the crawl controls and room index status", () => {
    renderScoped({ settingsScope: "account" });

    expect(screen.getByRole("button", { name: t("settings.searchHistoryRebuild") })).toBeTruthy();
    expect(
      screen.getByRole("region", { name: t("settings.searchHistoryRoomStatus") })
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: t("settings.accountSettings") })).toBeNull();
  });

  test("keeps each room index status on one row with its action", () => {
    renderScoped({ settingsScope: "account" });

    const list = screen.getByRole("region", { name: t("settings.searchHistoryRoomStatus") });
    const rows = Array.from(list.querySelectorAll<HTMLElement>(".crawler-room-status-row"));
    expect(rows).toHaveLength(2);

    for (const row of rows) {
      // Name, status, and action share the row element, so the row is one line.
      expect(row.classList.contains("settings-detail-row")).toBe(true);
      expect(row.children).toHaveLength(3);
      expect(row.children[1]?.tagName).toBe("SMALL");
    }

    const completedRow = rows.find((row) => row.textContent?.includes("Alpha room"));
    expect(completedRow).toBeTruthy();
    expect(completedRow?.textContent).toContain(
      t("settings.searchHistoryRoomCompleted", { indexed: 633 })
    );
    expect(
      within(completedRow as HTMLElement).getByRole("button", {
        name: t("settings.searchHistoryReindexRoom")
      })
    ).toBeTruthy();
  });
});

describe("SearchHistorySection rebuild guard", () => {
  test("cancelling the in-app confirmation leaves the search index alone", () => {
    const onRebuildSearchIndex = vi.fn();
    renderSection(onRebuildSearchIndex);

    fireEvent.click(screen.getByRole("button", { name: t("settings.searchHistoryRebuild") }));

    // Nothing runs until the guard is confirmed.
    expect(onRebuildSearchIndex).not.toHaveBeenCalled();
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByText(t("settings.searchHistoryRebuildConfirm"))).toBeTruthy();

    fireEvent.click(within(dialog).getByRole("button", { name: t("action.cancel") }));

    expect(onRebuildSearchIndex).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  test("confirming the in-app guard starts the rebuild", () => {
    const onRebuildSearchIndex = vi.fn();
    renderSection(onRebuildSearchIndex);

    fireEvent.click(screen.getByRole("button", { name: t("settings.searchHistoryRebuild") }));

    const dialog = screen.getByRole("dialog");
    fireEvent.click(
      within(dialog).getByRole("button", { name: t("settings.searchHistoryRebuild") })
    );

    expect(onRebuildSearchIndex).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
