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
import type { SearchCrawlerSettings, SearchCrawlerState } from "../../domain/types";
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
