// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";

import { createBrowserFakeApi, type DesktopApi } from "./backend/browserFakeApi";
import { parseComposerDraftRevision } from "./domain/composerDraftRevision";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

async function renderAppWithApi(api: DesktopApi) {
  vi.resetModules();
  vi.doMock("./backend/client", () => ({ createDesktopApi: () => api }));
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

describe("App composer draft lifecycle", () => {
  test.each(["response-first", "accepted-clear-first"] as const)(
    "preserves a newer exact draft after an accepted send when %s",
    async (order) => {
      const api = createBrowserFakeApi();
      const roomId = "!room-alpha:example.invalid";
      await api.selectRoom(roomId);
      const mutable = api as unknown as {
        composerDraftRevisions: Map<string, ReturnType<typeof parseComposerDraftRevision>>;
        snapshot: Awaited<ReturnType<typeof api.getSnapshot>>;
      };
      const baseline = parseComposerDraftRevision("9007199254740992");
      mutable.composerDraftRevisions.set(roomId, baseline);
      mutable.snapshot.state.ui.timeline.composer.draft_revision = baseline;
      const originalSend = api.sendText.bind(api);
      const pending = deferred<Awaited<ReturnType<typeof api.sendText>>>();
      let capturedArgs: Parameters<typeof api.sendText> | null = null;
      vi.spyOn(api, "sendText").mockImplementation((...args) => {
        capturedArgs = args;
        return pending.promise;
      });

      await renderAppWithApi(api);
      const composer = await screen.findByRole("textbox", { name: "Message composer" });
      await act(async () => {
        fireEvent.change(composer, { target: { value: "9007199254740993" } });
      });
      const send = await screen.findByRole("button", { name: "Send" });
      await act(async () => {
        fireEvent.click(send);
      });
      await waitFor(() => expect(capturedArgs).not.toBeNull());
      const accepted = await originalSend(...capturedArgs!);
      expect(accepted.snapshot.state.ui.timeline.composer.last_accepted_clear_revision).toBe(
        "9007199254740994"
      );

      if (order === "accepted-clear-first") {
        const { setAppStoreSnapshot } = await import("./domain/appStore");
        await act(async () => setAppStoreSnapshot(accepted.snapshot));
      }
      await act(async () => {
        fireEvent.change(composer, { target: { value: "9007199254740994" } });
      });
      await act(async () => pending.resolve(accepted));

      await waitFor(() =>
        expect((composer as HTMLTextAreaElement).value).toBe("9007199254740994")
      );
    }
  );
});
