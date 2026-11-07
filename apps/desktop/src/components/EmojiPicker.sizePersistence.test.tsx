// @vitest-environment jsdom
/**
 * #1583 follow-up: the picker size lives in the Rust-owned window preference, so
 * the picker must open at the saved value and a resize must travel as an
 * app-scoped `UpdateSettings` whose settlement snapshot carries the new size.
 * The saved size is the *base* of a resize: a panel the layer narrowed to fit a
 * small window must never stand in for the size the user chose.
 *
 * The picker renders the whole emoji grid, so these mounting tests get a longer
 * deadline than the default under a full parallel suite; the inner `waitFor`s
 * keep their own 1s budget.
 */
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import { api } from "../backend/appRuntime";
import {
  clearAppStoreSnapshot,
  getAppStoreSnapshot,
  setAppStoreSnapshot,
} from "../domain/appStore";
import type { DesktopSnapshot, SettingsPatch } from "../domain/types";
import { readyDesktopSnapshotFixture } from "../test/desktopApiFixture";
import { EmojiPicker } from "./EmojiPicker";

vi.mock("../backend/appRuntime", () => ({
  api: { settlementSnapshot: vi.fn(), updateSettings: vi.fn() },
  startSessionVerificationWindowDrag: vi.fn(),
}));

const SAVED: [number, number] = [640, 420];

/** Patches the renderer sent, in order. */
const sent: SettingsPatch[] = [];
/** The size the core has published, and the generation of that snapshot. */
let published: [number, number] = SAVED;
let generation = 1;

function snapshotAt(size: [number, number], at: number): DesktopSnapshot {
  const snapshot = readyDesktopSnapshotFixture();
  snapshot.state_generation = at;
  snapshot.state.domain.settings.values.window.emoji_picker_size = size;
  return snapshot;
}

/** The marked corner decides which direction its arrow keys resize. */
function markedZone(): { horizontal: number } {
  const zone =
    screen.getByRole("button", { name: /resize emoji picker/i }).getAttribute(
      "data-emoji-picker-resize",
    ) ?? "";
  return { horizontal: zone.includes("left") ? -1 : 1 };
}

function pressArrow(): void {
  fireEvent.keyDown(screen.getByRole("button", { name: /resize emoji picker/i }), {
    key: "ArrowRight",
  });
}

function storedSize(): [number, number] | null | undefined {
  return getAppStoreSnapshot()?.state.domain.settings.values.window.emoji_picker_size;
}

beforeEach(() => {
  sent.length = 0;
  published = SAVED;
  generation = 1;
  // An accepted `update_settings` settles on a newer snapshot that carries it.
  vi.mocked(api.updateSettings).mockImplementation(async (patch: SettingsPatch) => {
    sent.push(patch);
    published = patch.window?.emoji_picker_size ?? published;
    generation += 1;
    return { admittedGeneration: generation, protocolVersion: 1 };
  });
  vi.mocked(api.settlementSnapshot).mockImplementation(async () =>
    snapshotAt(published, generation),
  );
  setAppStoreSnapshot(snapshotAt(SAVED, generation));
});

afterEach(() => {
  cleanup();
  clearAppStoreSnapshot();
  vi.resetAllMocks();
});

it(
  "opens at the saved size and commits the new size through the app-scoped window patch",
  { timeout: 15_000 },
  async () => {
    render(<EmojiPicker onSelect={vi.fn()} onClose={vi.fn()} />);
    const expected: [number, number] = [SAVED[0] + markedZone().horizontal * 20, SAVED[1]];

    pressArrow();

    await waitFor(() => expect(sent).toHaveLength(1));
    expect(sent[0]).toEqual({
      scope: "app",
      window: { close_to_tray: true, emoji_picker_size: expected },
    });
    await waitFor(() => expect(storedSize()).toEqual(expected));
  },
);

it(
  "resumes from the committed size after the picker is closed and opened again",
  { timeout: 15_000 },
  async () => {
    const picker = render(<EmojiPicker onSelect={vi.fn()} onClose={vi.fn()} />);
    const { horizontal } = markedZone();
    const first: [number, number] = [SAVED[0] + horizontal * 20, SAVED[1]];
    pressArrow();
    await waitFor(() => expect(storedSize()).toEqual(first));
    picker.unmount();

    // Reopening reads the same preference back; its arrow key continues from it.
    render(<EmojiPicker onSelect={vi.fn()} onClose={vi.fn()} />);
    pressArrow();
    await waitFor(() =>
      expect(sent[1]).toEqual({
        scope: "app",
        window: {
          close_to_tray: true,
          emoji_picker_size: [first[0] + horizontal * 20, first[1]],
        },
      }),
    );
  },
);
