// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";

import { createBrowserFakeApi, type DesktopApi } from "./backend/browserFakeApi";
import type { DiagnosticLogSnapshot } from "./domain/diagnostics";

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
  vi.restoreAllMocks();
  vi.resetModules();
});

function snapshot(entries: DiagnosticLogSnapshot["entries"], droppedEntries = 0) {
  return { entries, droppedEntries } satisfies DiagnosticLogSnapshot;
}

async function openDiagnostics() {
  const button = await screen.findByRole("button", { name: "Open diagnostics" });
  await act(async () => {
    fireEvent.click(button);
  });
  return screen.findByRole("dialog", { name: "Diagnostics" });
}

describe("App diagnostics lifecycle", () => {
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
});

describe("diagnostics runtime source contract", () => {
  test("contains no verbose diagnostics runtime gate or Vite variable", async () => {
    const { readFile } = await import("node:fs/promises");
    const files = ["./App.tsx", "./domain/diagnostics.ts", "./vite-env.d.ts"];
    const contents = await Promise.all(files.map((file) => readFile(new URL(file, import.meta.url), "utf8")));
    const runtimeSource = contents.join("\n");

    expect(runtimeSource).not.toContain("VITE_KOUSHI_VERBOSE_DIAGNOSTICS");
    expect(runtimeSource).not.toContain("verboseDiagnosticsEnabled");
  });
});
