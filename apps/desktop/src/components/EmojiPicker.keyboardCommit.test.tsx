// @vitest-environment jsdom
/**
 * #1826: a keyboard resize is one gesture, not a command per key. Every
 * committed size is an app-scoped `UpdateSettings` round-trip plus a settings
 * write, so a burst of arrow keys — including OS auto-repeat — must reach the
 * preference once, when the keys stop, and a key the clamp cannot honour must
 * not write at all.
 *
 * The picker renders the whole emoji grid, so these mounting tests get a longer
 * deadline than the default under a full parallel suite.
 */
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import { api } from "../backend/appRuntime";
import { clearAppStoreSnapshot, setAppStoreSnapshot } from "../domain/appStore";
import type { DesktopSnapshot, SettingsPatch } from "../domain/types";
import { readyDesktopSnapshotFixture } from "../test/desktopApiFixture";
import { EMOJI_PICKER_KEYBOARD_COMMIT_MS, EmojiPicker } from "./EmojiPicker";

vi.mock("../backend/appRuntime", () => ({
  api: { settlementSnapshot: vi.fn(), updateSettings: vi.fn() },
  startSessionVerificationWindowDrag: vi.fn(),
}));

const SAVED: [number, number] = [640, 420];
const KEY_STEP = 20;

/** Patches the renderer sent, in order. */
const sent: SettingsPatch[] = [];

function snapshotAt(size: [number, number]): DesktopSnapshot {
  const snapshot = readyDesktopSnapshotFixture();
  snapshot.state_generation = 1;
  snapshot.state.domain.settings.values.window.emoji_picker_size = size;
  return snapshot;
}

function handle(): HTMLElement {
  return screen.getByRole("button", { name: /resize emoji picker/i });
}

/** The marked corner decides which direction its arrow keys resize. */
function markedHorizontal(): number {
  const zone = handle().getAttribute("data-emoji-picker-resize") ?? "";
  return zone.includes("left") ? -1 : 1;
}

/** Press the arrow key that grows the panel, `times` times. */
function pressGrowKey(times: number): void {
  const key = markedHorizontal() > 0 ? "ArrowRight" : "ArrowLeft";
  for (let i = 0; i < times; i += 1) {
    fireEvent.keyDown(handle(), { key });
  }
}

/** Let the commit debounce elapse: the only thing between the keys and a write. */
async function keysStop(): Promise<void> {
  await act(async () => {
    vi.advanceTimersByTime(EMOJI_PICKER_KEYBOARD_COMMIT_MS);
  });
}

beforeEach(() => {
  vi.useFakeTimers();
  // jsdom does not implement pointer capture; a drag only needs it to exist.
  Element.prototype.setPointerCapture ??= () => undefined;
  Element.prototype.releasePointerCapture ??= () => undefined;
  sent.length = 0;
  vi.mocked(api.updateSettings).mockImplementation(async (patch: SettingsPatch) => {
    sent.push(patch);
    return { admittedGeneration: 1, protocolVersion: 1 };
  });
  vi.mocked(api.settlementSnapshot).mockImplementation(async () => snapshotAt(SAVED));
  setAppStoreSnapshot(snapshotAt(SAVED));
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  clearAppStoreSnapshot();
  vi.resetAllMocks();
});

it("writes the preference once for a burst of arrow keys", { timeout: 15_000 }, async () => {
  render(<EmojiPicker onSelect={vi.fn()} onClose={vi.fn()} />);
  pressGrowKey(5);
  // Auto-repeat does not write per key; the gesture has not ended yet.
  expect(sent).toHaveLength(0);

  await keysStop();
  expect(sent).toEqual([
    {
      scope: "app",
      window: {
        close_to_tray: true,
        emoji_picker_size: [SAVED[0] + 5 * KEY_STEP * markedHorizontal(), SAVED[1]],
      },
    },
  ]);
});

it("writes nothing for arrow keys the clamp cannot honour", { timeout: 15_000 }, async () => {
  // Already at the maximum: every further key is a no-op, so none may be sent.
  render(<EmojiPicker onSelect={vi.fn()} onClose={vi.fn()} size={[1200, 400]} />);
  pressGrowKey(3);

  await keysStop();
  expect(sent).toHaveLength(0);
});

it("keeps a pointer drag's own commit when a key is pressed mid-drag", { timeout: 15_000 }, async () => {
  const onSizeChange = vi.fn();
  render(
    <EmojiPicker onSelect={vi.fn()} onClose={vi.fn()} size={[640, 420]} onSizeChange={onSizeChange} />,
  );
  const edge = document.querySelector<HTMLElement>('[data-emoji-picker-resize="right"]')!;
  const drag = { button: 0, clientX: 100, clientY: 100, pointerId: 1 };
  fireEvent.pointerDown(edge, drag);
  fireEvent.pointerMove(edge, { ...drag, clientX: 140 });
  // The drag owns the size until release; this key must not queue a write that
  // lands after the drag's own commit.
  fireEvent.keyDown(handle(), { key: "ArrowRight" });
  fireEvent.pointerUp(edge, { ...drag, clientX: 140 });

  await keysStop();
  expect(onSizeChange).toHaveBeenCalledTimes(1);
  expect(onSizeChange).toHaveBeenLastCalledWith([680, 420]);
});
