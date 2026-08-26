// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";

import { createBrowserFakeApi, type DesktopApi } from "./backend/browserFakeApi";
import type { DiagnosticLogSnapshot } from "./domain/diagnostics";

const tauriEventListeners = vi.hoisted(
  () => new Map<string, (event: { payload: unknown }) => void>()
);

vi.mock("@tauri-apps/api/event", () => ({
  listen: async (eventName: string, listener: (event: { payload: unknown }) => void) => {
    tauriEventListeners.set(eventName, listener);
    return () => tauriEventListeners.delete(eventName);
  }
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    isFullscreen: async () => false,
    setFullscreen: async () => undefined,
    setTitle: async () => undefined,
    setBadgeCount: async () => undefined,
    startDragging: async () => undefined
  })
}));

async function renderAppWithApi(api: DesktopApi) {
  Object.defineProperty(document.body, "innerText", {
    configurable: true,
    get: () => document.body.textContent ?? ""
  });
  vi.resetModules();
  vi.doMock("./backend/client", () => ({
    createDesktopApi: () => api
  }));
  const { App } = await import("./App");
  return render(<App />);
}

async function clearProjectedSnapshot() {
  const { clearAppStoreSnapshot } = await import("./domain/appStore");
  clearAppStoreSnapshot();
}

afterEach(async () => {
  cleanup();
  await clearProjectedSnapshot();
  vi.doUnmock("./backend/client");
  tauriEventListeners.clear();
  Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
  vi.restoreAllMocks();
  vi.resetModules();
});

function snapshot(entries: DiagnosticLogSnapshot["entries"], droppedEntries = 0) {
  return { entries, droppedEntries } satisfies DiagnosticLogSnapshot;
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((promiseResolve, promiseReject) => {
    resolve = promiseResolve;
    reject = promiseReject;
  });
  return { promise, resolve, reject };
}

async function openDiagnostics() {
  const button = await screen.findByRole("button", { name: "Open diagnostics" });
  await act(async () => {
    fireEvent.click(button);
  });
  return screen.findByRole("dialog", { name: "Diagnostics" });
}

describe("App diagnostics lifecycle", () => {
  test("does not use login discovery to repair active-session account management", async () => {
    const api = createBrowserFakeApi();
    await api.discoverLoginMethods("https://matrix.org");
    const discoverLoginMethods = vi.spyOn(api, "discoverLoginMethods");

    await renderAppWithApi(api);

    await act(async () => {
      await Promise.resolve();
    });
    expect(discoverLoginMethods).not.toHaveBeenCalled();
  });

  test("keeps the normal shell hidden until a ready session has a ready secure backup gate", async () => {
    const api = createBrowserFakeApi({ secureBackupGate: { kind: "setupRequired" } });

    await renderAppWithApi(api);

    expect(
      await screen.findByRole("heading", { name: "Secure backup required" })
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Create room" })).toBeNull();
    expect(screen.queryByRole("textbox", { name: "Message" })).toBeNull();
  });

  test("exposes the normal shell while an authoritative backup uploads existing keys", async () => {
    const api = createBrowserFakeApi({
      secureBackupGate: { kind: "uploadingExistingKeys", pending: "over_one_hundred" }
    });

    await renderAppWithApi(api);

    expect(await screen.findByRole("button", { name: "Create room" })).toBeTruthy();
    expect(screen.queryByRole("heading", { name: "Uploading existing keys…" })).toBeNull();
  });

  test("keeps the operational shell visible when a ready session later degrades", async () => {
    const api = createBrowserFakeApi();
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText }
    });
    const getDiagnosticSnapshot = vi
      .spyOn(api, "getDiagnosticSnapshot")
      .mockResolvedValue(snapshot([{ timestampMs: 1, source: "core.runtime", message: "stage=copy" }]));
    const readySnapshot = await api.getSnapshot();
    const mutable = api as unknown as { snapshot: typeof readySnapshot };
    const activeRoomId = readySnapshot.state.ui.navigation.active_room_id;
    mutable.snapshot.state.domain.rooms = mutable.snapshot.state.domain.rooms.map((room) =>
      room.room_id === activeRoomId ? { ...room, is_encrypted: true } : room
    );
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true,
      value: {}
    });

    await renderAppWithApi(api);

    expect(await screen.findByRole("button", { name: "Create room" })).toBeTruthy();
    await waitFor(() =>
      expect(tauriEventListeners.get("koushi-desktop://state")).toBeDefined()
    );

    mutable.snapshot.state.domain.secure_backup_gate = {
      kind: "degradedRetrying",
      failure: "network"
    };

    await act(async () => {
      tauriEventListeners.get("koushi-desktop://state")?.({ payload: "stateChanged" });
    });

    const statusTrigger = await screen.findByRole("button", {
      name: "Open session status, 1 runtime warning"
    });
    expect(await screen.findByRole("button", { name: "Create room" })).toBeTruthy();
    expect(screen.getByRole("textbox", { name: "Message composer" }).getAttribute("aria-disabled"))
      .not.toBe("true");
    expect(screen.queryByRole("heading", { name: "Secure backup required" })).toBeNull();
    expect(document.querySelector(".secure-backup-runtime-banner")).toBeNull();

    fireEvent.click(statusTrigger);
    const status = screen.getByRole("dialog", { name: "Current session" });
    expect(status.textContent).toContain("Runtime warnings");
    expect(status.textContent).toContain("Secure Backup unavailable");
    expect(status.textContent).toContain("Secure backup could not reach the server.");
    fireEvent.click(screen.getByRole("button", { name: "Copy diagnostics" }));

    await waitFor(() => expect(getDiagnosticSnapshot).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    expect(writeText.mock.calls[0]?.[0]).toContain("Koushi diagnostics");
    expect(writeText.mock.calls[0]?.[0]).toContain("core.runtime stage=copy");
  });

  test("projects sync and current-session failures into the status alerts", async () => {
    const api = createBrowserFakeApi();
    const initialSnapshot = await api.getSnapshot();
    const mutable = api as unknown as { snapshot: typeof initialSnapshot };
    mutable.snapshot.state.domain.sync = { reconnecting: "network_offline" };
    mutable.snapshot.state.domain.current_session_status = {
      status: "failed",
      request_id: 370,
      kind: "timed_out",
      checked_at_ms: Date.UTC(2026, 7, 9, 9, 0, 0)
    };

    await renderAppWithApi(api);

    fireEvent.click(
      await screen.findByRole("button", {
        name: "Open session status, 2 runtime warnings"
      })
    );
    const status = screen.getByRole("dialog", { name: "Current session" });
    expect(status.textContent).toContain("Sync");
    expect(status.textContent).toContain("Sync reconnecting");
    expect(status.textContent).toContain("Session check timed out");
  });

  test("records a schema mismatch without console output and retains the fixed entry after refresh", async () => {
    const api = createBrowserFakeApi();
    const compatibleSnapshot = await api.getSnapshot();
    const incompatibleSchemaVersion = 987654321;
    const incompatibleSnapshot = {
      ...compatibleSnapshot,
      state: {
        ...compatibleSnapshot.state,
        schema_version: incompatibleSchemaVersion
      }
    };
    const getSnapshot = vi
      .spyOn(api, "getSnapshot")
      .mockResolvedValueOnce(incompatibleSnapshot)
      .mockResolvedValueOnce(compatibleSnapshot);
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => undefined);
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true,
      value: {}
    });

    await renderAppWithApi(api);

    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(consoleError).not.toHaveBeenCalled();
    await waitFor(() =>
      expect(tauriEventListeners.get("koushi-desktop://state")).toBeDefined()
    );

    await act(async () => {
      tauriEventListeners.get("koushi-desktop://state")?.({ payload: "stateChanged" });
    });
    await waitFor(() => {
      expect(getSnapshot.mock.calls.length).toBeGreaterThanOrEqual(2);
      expect(screen.queryByRole("alert")).toBeNull();
    });
    expect(screen.getByRole("button", { name: "Open diagnostics" })).toBeTruthy();

    const dialog = await openDiagnostics();
    expect(dialog.textContent).toContain("snapshot schema_mismatch");
    expect(dialog.textContent).not.toContain(String(incompatibleSchemaVersion));
    expect(consoleError).not.toHaveBeenCalled();
  });

  test("fetches runtime records on open and displays them with frontend diagnostics", async () => {
    const api = createBrowserFakeApi();
    const getDiagnosticSnapshot = vi
      .spyOn(api, "getDiagnosticSnapshot")
      .mockResolvedValue(
        snapshot([
          { timestampMs: 1, source: "core.timeline", message: "stage=runtime" }
        ], 3)
      );

    await renderAppWithApi(api);

    const searchInput = await screen.findByRole("textbox", { name: "Search" });
    await act(async () => {
      fireEvent.change(searchInput, { target: { value: "frontend" } });
    });
    await screen.findByText(/result[s]? for "frontend"|Searching for "frontend"/);

    const dialog = await openDiagnostics();
    expect(getDiagnosticSnapshot).toHaveBeenCalledTimes(1);
    expect(dialog.textContent).toContain("core.timeline stage=runtime");
    expect(dialog.textContent).toContain("panel mode=search");
    expect(dialog.textContent).toContain("Diagnostic records dropped: 3");
  });

  test("exports decrypt-retry diagnostics without private values", async () => {
    const api = createBrowserFakeApi();
    vi.spyOn(api, "getDiagnosticSnapshot").mockResolvedValue(
      snapshot([
        {
          timestampMs: 1,
          source: "core.decrypt_retry",
          message:
            "stage=request operation=correlation:17 reason=missing_room_key attempt=1 backup_state=available elapsed_bucket=under_1s"
        },
        {
          timestampMs: 2,
          source: "core.decrypt_retry",
          message: "stage=settled operation=correlation:17 result=timeout elapsed_bucket=over_30s"
        }
      ])
    );

    await renderAppWithApi(api);
    const dialog = await openDiagnostics();
    const exported = dialog.textContent ?? "";

    expect(exported).toContain("core.decrypt_retry");
    expect(exported).toContain("reason=missing_room_key");
    expect(exported).toContain("result=timeout");
    for (const privateValue of [
      "!room:example.test",
      "$event:example.test",
      "MEGOLMSESSION",
      "message body",
      "https://private.example.test",
      "raw SDK error"
    ]) {
      expect(exported).not.toContain(privateValue);
    }
  });

  test("refetches the runtime snapshot when Diagnostics is opened again", async () => {
    const api = createBrowserFakeApi();
    const snapshots = [
      snapshot([{ timestampMs: 1, source: "core.first", message: "first-runtime" }]),
      snapshot([{ timestampMs: 2, source: "core.second", message: "second-runtime" }])
    ];
    const getDiagnosticSnapshot = vi
      .spyOn(api, "getDiagnosticSnapshot")
      .mockImplementation(async () => snapshots.shift()!);

    await renderAppWithApi(api);

    let dialog = await openDiagnostics();
    expect(dialog.textContent).toContain("first-runtime");

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Close {title}" }));
    });
    dialog = await openDiagnostics();

    expect(getDiagnosticSnapshot).toHaveBeenCalledTimes(2);
    expect(dialog.textContent).toContain("second-runtime");
    expect(dialog.textContent).not.toContain("first-runtime");
  });

  test("only the newest overlapping snapshot success can open and update Diagnostics", async () => {
    const api = createBrowserFakeApi();
    const first = deferred<DiagnosticLogSnapshot>();
    const second = deferred<DiagnosticLogSnapshot>();
    const getDiagnosticSnapshot = vi
      .spyOn(api, "getDiagnosticSnapshot")
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);

    await renderAppWithApi(api);
    const button = await screen.findByRole("button", { name: "Open diagnostics" });

    await act(async () => {
      fireEvent.click(button);
      fireEvent.click(button);
    });
    expect(getDiagnosticSnapshot).toHaveBeenCalledTimes(2);
    expect(screen.queryByRole("dialog", { name: "Diagnostics" })).toBeNull();

    await act(async () => {
      second.resolve(snapshot([{ timestampMs: 2, source: "core.newest", message: "newest-runtime" }]));
      await second.promise;
    });
    const dialog = await screen.findByRole("dialog", { name: "Diagnostics" });
    expect(dialog.textContent).toContain("newest-runtime");

    await act(async () => {
      first.resolve(snapshot([{ timestampMs: 1, source: "core.stale", message: "stale-runtime" }]));
      await first.promise;
    });
    expect(dialog.textContent).toContain("newest-runtime");
    expect(dialog.textContent).not.toContain("stale-runtime");
  });

  test("only the newest overlapping snapshot success can survive a stale failure", async () => {
    const api = createBrowserFakeApi();
    const first = deferred<DiagnosticLogSnapshot>();
    const second = deferred<DiagnosticLogSnapshot>();
    const getDiagnosticSnapshot = vi
      .spyOn(api, "getDiagnosticSnapshot")
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);

    await renderAppWithApi(api);
    const button = await screen.findByRole("button", { name: "Open diagnostics" });

    await act(async () => {
      fireEvent.click(button);
      fireEvent.click(button);
    });
    expect(getDiagnosticSnapshot).toHaveBeenCalledTimes(2);

    await act(async () => {
      second.resolve(snapshot([{ timestampMs: 2, source: "core.newest", message: "newest-runtime" }]));
      await second.promise;
    });
    const dialog = await screen.findByRole("dialog", { name: "Diagnostics" });

    await act(async () => {
      first.reject(new Error("stale failure"));
      await first.promise.catch(() => undefined);
    });
    expect(dialog.textContent).toContain("newest-runtime");
    expect(dialog.textContent).not.toContain("kind=unavailable");
    expect(dialog.textContent).not.toContain("stale failure");
  });

  test("records manual room-key reshare failure with fixed private-data-free tokens", async () => {
    const api = createBrowserFakeApi();
    const baseSnapshot = await api.getSnapshot();
    const roomId = baseSnapshot.state.ui.navigation.active_room_id;
    expect(roomId).not.toBeNull();
    const privateRoomId = roomId!;
    const encryptedSnapshot = {
      ...baseSnapshot,
      state: {
        ...baseSnapshot.state,
        domain: {
          ...baseSnapshot.state.domain,
          rooms: baseSnapshot.state.domain.rooms.map((room) =>
            room.room_id === privateRoomId ? { ...room, is_encrypted: true } : room
          )
        }
      }
    };
    const rawError = [
      "raw SDK error",
      "secret message body",
      "$private-event:example.invalid",
      "/Users/member/private/store",
      "https://private.example.invalid/room",
      "access_token=private-token"
    ].join(" ");
    vi.spyOn(api, "getSnapshot").mockResolvedValue(encryptedSnapshot);
    vi.spyOn(api, "loadRoomSettings").mockResolvedValue(encryptedSnapshot);
    const reshareRoomKey = vi
      .spyOn(api, "reshareRoomKey")
      .mockRejectedValue(new Error(rawError));

    await renderAppWithApi(api);
    fireEvent.click(await screen.findByRole("button", { name: "Room info" }));
    fireEvent.click(await screen.findByRole("button", { name: "Reshare room keys" }));

    await waitFor(() => {
      expect(reshareRoomKey).toHaveBeenCalledWith(privateRoomId);
      expect(screen.getByText("Could not reshare room keys.")).toBeTruthy();
    });
    const dialog = await openDiagnostics();

    expect(dialog.textContent).toContain(
      "e2ee.room_key operation=manual_reshare stage=request"
    );
    expect(dialog.textContent).toContain(
      "e2ee.room_key operation=manual_reshare stage=failed kind=transport"
    );
    for (const privateValue of [
      privateRoomId,
      rawError,
      "raw SDK error",
      "secret message body",
      "$private-event:example.invalid",
      "/Users/member/private/store",
      "private.example.invalid",
      "private-token"
    ]) {
      expect(dialog.textContent).not.toContain(privateValue);
    }
  });

  test("opens with only a safe synthetic fetch record when the snapshot rejects", async () => {
    const api = createBrowserFakeApi();
    const secretError = "secret error stack /Users/private/path room=!secret:example.invalid";
    vi.spyOn(api, "getDiagnosticSnapshot").mockRejectedValue(new Error(secretError));

    await renderAppWithApi(api);
    const dialog = await openDiagnostics();

    await waitFor(() => {
      expect(dialog.textContent).toContain("diagnostics.fetch kind=unavailable");
    });
    expect(dialog.textContent).not.toContain(secretError);
    expect(dialog.textContent).not.toContain("secret error stack");
    expect(dialog.textContent).not.toContain("/Users/private/path");
  });

  test("keeps the latest successful runtime snapshot and dropped count when refresh fails", async () => {
    const api = createBrowserFakeApi();
    const rawError = [
      "raw SDK error",
      "secret message body",
      "/Users/member/private/store",
      "https://private.example.invalid/room",
      "access_token=private-token"
    ].join(" ");
    vi.spyOn(api, "getDiagnosticSnapshot")
      .mockResolvedValueOnce(
        snapshot(
          [{ timestampMs: 1, source: "core.retained", message: "stage=retained" }],
          7
        )
      )
      .mockRejectedValueOnce(new Error(rawError));

    await renderAppWithApi(api);

    let dialog = await openDiagnostics();
    expect(dialog.textContent).toContain("core.retained stage=retained");
    expect(dialog.textContent).toContain("Diagnostic records dropped: 7");

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Close {title}" }));
    });
    dialog = await openDiagnostics();

    expect(dialog.textContent).toContain("core.retained stage=retained");
    expect(dialog.textContent).toContain("Diagnostic records dropped: 7");
    expect(dialog.textContent).toContain("diagnostics.fetch kind=unavailable");
    for (const privateValue of [
      rawError,
      "raw SDK error",
      "secret message body",
      "/Users/member/private/store",
      "private.example.invalid",
      "private-token"
    ]) {
      expect(dialog.textContent).not.toContain(privateValue);
    }
  });

  test("newest overlapping failure preserves the prior success and ignores an older late success", async () => {
    const api = createBrowserFakeApi();
    const older = deferred<DiagnosticLogSnapshot>();
    const newest = deferred<DiagnosticLogSnapshot>();
    const rawError = "raw SDK error /Users/member/private/store access_token=private-token";
    vi.spyOn(api, "getDiagnosticSnapshot")
      .mockResolvedValueOnce(
        snapshot(
          [{ timestampMs: 1, source: "core.baseline", message: "stage=baseline" }],
          9
        )
      )
      .mockReturnValueOnce(older.promise)
      .mockReturnValueOnce(newest.promise);

    await renderAppWithApi(api);
    await openDiagnostics();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Close {title}" }));
    });

    const button = screen.getByRole("button", { name: "Open diagnostics" });
    await act(async () => {
      fireEvent.click(button);
      fireEvent.click(button);
    });

    await act(async () => {
      newest.reject(new Error(rawError));
      await newest.promise.catch(() => undefined);
    });
    const dialog = await screen.findByRole("dialog", { name: "Diagnostics" });
    expect(dialog.textContent).toContain("core.baseline stage=baseline");
    expect(dialog.textContent).toContain("Diagnostic records dropped: 9");
    expect(dialog.textContent).toContain("diagnostics.fetch kind=unavailable");
    expect(dialog.textContent).not.toContain(rawError);

    await act(async () => {
      older.resolve(
        snapshot([{ timestampMs: 2, source: "core.older", message: "stage=stale" }], 2)
      );
      await older.promise;
    });

    expect(dialog.textContent).toContain("core.baseline stage=baseline");
    expect(dialog.textContent).toContain("Diagnostic records dropped: 9");
    expect(dialog.textContent).not.toContain("core.older");
    expect(dialog.textContent).not.toContain("stage=stale");
  });
});

describe("diagnostics runtime source contract", () => {
  test("contains no verbose diagnostics runtime gate or Vite variable", async () => {
    const { readFile } = await import("node:fs/promises");
    const files = ["./App.tsx", "./domain/diagnostics.ts", "./vite-env.d.ts", "./app/qaDiagnostics.ts"];
    const contents = await Promise.all(files.map(async (file) => [file, await readFile(new URL(file, import.meta.url), "utf8")] as const));

    for (const [file, source] of contents) {
      expect(source, file).not.toContain("VITE_KOUSHI_VERBOSE_DIAGNOSTICS");
      expect(source, file).not.toContain("verboseDiagnosticsEnabled");
    }
  });
});
