// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";

import { createBrowserFakeApi, type DesktopApi } from "./backend/browserFakeApi";
import type { DesktopSnapshot } from "./domain/types";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((promiseResolve, promiseReject) => {
    resolve = promiseResolve;
    reject = promiseReject;
  });
  return { promise, resolve, reject };
}

async function renderAppWithApi(api: DesktopApi) {
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

describe("App search lifecycle", () => {
  test("opens the search panel while the submitted query is still pending", async () => {
    const api = createBrowserFakeApi();
    const pending = deferred<DesktopSnapshot>();
    const submitSearch = vi
      .spyOn(api, "submitSearch")
      .mockImplementation(() => pending.promise);

    await renderAppWithApi(api);

    const searchInput = await screen.findByRole("textbox", { name: "Search" });
    await act(async () => {
      fireEvent.change(searchInput, { target: { value: "Pending" } });
    });

    await waitFor(() => {
      expect(submitSearch).toHaveBeenCalledWith("Pending", "allRooms");
    });
    expect(await screen.findByText('Searching for "Pending"')).toBeTruthy();
  });

  test("replaces previous results with the current pending query", async () => {
    const api = createBrowserFakeApi();

    await renderAppWithApi(api);

    const searchInput = await screen.findByRole("textbox", { name: "Search" });
    await act(async () => {
      fireEvent.change(searchInput, { target: { value: "Alpha" } });
    });
    expect(await screen.findByText(/result[s]? for "Alpha"/)).toBeTruthy();

    const pending = deferred<DesktopSnapshot>();
    const originalSubmitSearch = api.submitSearch.bind(api);
    const submitSearch = vi
      .spyOn(api, "submitSearch")
      .mockImplementation((query, scope) =>
        query === "Beta" ? pending.promise : originalSubmitSearch(query, scope)
      );

    await act(async () => {
      fireEvent.change(searchInput, { target: { value: "Beta" } });
    });

    await waitFor(() => {
      expect(submitSearch).toHaveBeenCalledWith("Beta", "allRooms");
    });
    expect(await screen.findByText('Searching for "Beta"')).toBeTruthy();
    expect(screen.queryByText(/result[s]? for "Alpha"/)).toBeNull();
  });
});
