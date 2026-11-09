// @vitest-environment jsdom
/**
 * App Settings shows the device-local emoji picker size and clears it through
 * the picker's own write path (#1583 keeps the saved size; this is the only
 * place the saved value is visible). Clearing writes `null` — the Rust default —
 * so the button is disabled while the size is already unset.
 */
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import { api } from "../../backend/appRuntime";
import { clearAppStoreSnapshot, setAppStoreSnapshot } from "../../domain/appStore";
import type { DesktopSnapshot, SettingsPatch } from "../../domain/types";
import { t } from "../../i18n/messages";
import { readyDesktopSnapshotFixture } from "../../test/desktopApiFixture";
import { EmojiPickerSizeRow } from "./EmojiPickerSizeRow";

vi.mock("../../backend/appRuntime", () => ({
  api: { settlementSnapshot: vi.fn(), updateSettings: vi.fn() },
  startSessionVerificationWindowDrag: vi.fn(),
}));

const SAVED: [number, number] = [700, 900];

const sent: SettingsPatch[] = [];
let published: [number, number] | null = SAVED;

function snapshotAt(size: [number, number] | null, at: number): DesktopSnapshot {
  const snapshot = readyDesktopSnapshotFixture();
  snapshot.state_generation = at;
  snapshot.state.domain.settings.values.window.emoji_picker_size = size;
  return snapshot;
}

function resetButton(): HTMLButtonElement {
  return screen.getByRole("button", {
    name: t("settings.emojiPickerSizeReset"),
  }) as HTMLButtonElement;
}

beforeEach(() => {
  sent.length = 0;
  published = SAVED;
  vi.mocked(api.updateSettings).mockImplementation(async (patch: SettingsPatch) => {
    sent.push(patch);
    published = patch.window?.emoji_picker_size ?? null;
    return { admittedGeneration: 2, protocolVersion: 1 };
  });
  vi.mocked(api.settlementSnapshot).mockImplementation(async () => snapshotAt(published, 2));
  setAppStoreSnapshot(snapshotAt(SAVED, 1));
});

afterEach(() => {
  cleanup();
  clearAppStoreSnapshot();
  vi.resetAllMocks();
});

it("shows the saved size and clears it to the default through the app-scoped window patch", async () => {
  render(<EmojiPickerSizeRow />);

  expect(screen.getByText(t("settings.emojiPickerSize"))).toBeTruthy();
  expect(screen.getByText("700×900")).toBeTruthy();
  expect(resetButton().disabled).toBe(false);

  fireEvent.click(resetButton());

  await waitFor(() => expect(sent).toHaveLength(1));
  // The picker's own path: one app-scoped patch that keeps the rest of the
  // window preference and clears only the size.
  expect(sent[0]).toEqual({
    scope: "app",
    window: { close_to_tray: true, emoji_picker_size: null },
  });
  await waitFor(() => expect(screen.getByText(t("settings.emojiPickerSizeDefault"))).toBeTruthy());
  expect(resetButton().disabled).toBe(true);
});

it("starts at the default with nothing to reset", () => {
  setAppStoreSnapshot(snapshotAt(null, 3));
  render(<EmojiPickerSizeRow />);

  expect(screen.getByText(t("settings.emojiPickerSizeDefault"))).toBeTruthy();
  expect(resetButton().disabled).toBe(true);

  fireEvent.click(resetButton());
  expect(sent).toHaveLength(0);
});
