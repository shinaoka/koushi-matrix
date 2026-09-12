/* @vitest-environment jsdom */

import { listen } from "@tauri-apps/api/event";
import { beforeEach, describe, expect, test, vi } from "vitest";

import type { CoreEventPayload } from "../../domain/coreEvents";
import { createTauriDesktopEventPort } from "./desktopEventPort";

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

const unlisten = vi.fn();

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(listen).mockResolvedValue(unlisten);
});

describe("Tauri desktop event port", () => {
  test("constructs without subscribing", () => {
    createTauriDesktopEventPort();
    expect(listen).not.toHaveBeenCalled();
  });

  test("unwraps Core and menu payloads on their exact channels", async () => {
    const coreListener = vi.fn();
    const menuListener = vi.fn();
    const port = createTauriDesktopEventPort();

    await expect(port.listenCoreEvents(coreListener)).resolves.toBe(unlisten);
    await expect(port.listenMenuActions(menuListener)).resolves.toBe(unlisten);

    expect(listen).toHaveBeenNthCalledWith(
      1,
      "koushi-desktop://event",
      expect.any(Function)
    );
    expect(listen).toHaveBeenNthCalledWith(
      2,
      "koushi-desktop://menu",
      expect.any(Function)
    );
    const corePayload = { kind: "ResyncMarker", generation: 7 } as CoreEventPayload;
    const coreEnvelopeListener = vi.mocked(listen).mock.calls[0]?.[1] as (
      event: { payload: CoreEventPayload }
    ) => void;
    const menuEnvelopeListener = vi.mocked(listen).mock.calls[1]?.[1] as (
      event: { payload: string }
    ) => void;
    coreEnvelopeListener({ payload: corePayload });
    menuEnvelopeListener({ payload: "toggleFullscreen" });

    expect(coreListener).toHaveBeenCalledWith(corePayload);
    expect(menuListener).toHaveBeenCalledWith("toggleFullscreen");
  });

  test("forwards v1 state updates on the single state-update channel", async () => {
    const stateListener = vi.fn();
    const port = createTauriDesktopEventPort();

    await expect(port.listenStateUpdates(stateListener)).resolves.toBe(unlisten);
    expect(listen).toHaveBeenCalledWith("koushi-desktop://state-update", expect.any(Function));
    const envelope = {
      protocol_version: 1 as const,
      kind: "delta" as const,
      generation: 7,
      changed: {}
    };
    const envelopeListener = vi.mocked(listen).mock.calls[0]?.[1] as (
      event: { payload: typeof envelope }
    ) => void;
    envelopeListener({ payload: envelope });

    expect(stateListener).toHaveBeenCalledOnce();
    expect(stateListener).toHaveBeenCalledWith(envelope);
  });

  test("forwards desktop update state on its adapter channel", async () => {
    const updateListener = vi.fn();
    const port = createTauriDesktopEventPort();

    await expect(port.listenDesktopUpdates(updateListener)).resolves.toBe(unlisten);
    expect(listen).toHaveBeenCalledWith("koushi-desktop://update", expect.any(Function));
    const envelopeListener = vi.mocked(listen).mock.calls[0]?.[1] as (
      event: { payload: { kind: "ready"; version: string } }
    ) => void;
    envelopeListener({ payload: { kind: "ready", version: "1.2.3" } });

    expect(updateListener).toHaveBeenCalledWith({ kind: "ready", version: "1.2.3" });
  });
});
