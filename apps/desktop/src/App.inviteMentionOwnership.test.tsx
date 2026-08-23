// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";

import { createBrowserFakeApi, type DesktopApi } from "./backend/browserFakeApi";
import { setInlineMentionEditorSelection } from "./components/ImeTextControl";
import type { DesktopSnapshot, MentionSurface } from "./domain/types";

const ROOM_ID = "!room-alpha:example.invalid";
const THREAD_ROOT_ID = "$alpha-update";

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

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((promiseResolve, promiseReject) => {
    resolve = promiseResolve;
    reject = promiseReject;
  });
  return { promise, resolve, reject };
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

function snapshotWithGeneration(snapshot: DesktopSnapshot, stateGeneration: number): DesktopSnapshot {
  const next = structuredClone(snapshot);
  next.state_generation = stateGeneration;
  return next;
}

function inviteSnapshot(
  snapshot: DesktopSnapshot,
  stateGeneration: number,
  query: string,
  displayLabel: string
): DesktopSnapshot {
  const next = snapshotWithGeneration(snapshot, stateGeneration);
  const workflow = next.state.domain.invite_workflow!;
  next.state.domain.invite_workflow = {
    ...workflow,
    query: {
      ...workflow.query,
      room_id: ROOM_ID,
      query,
      candidates: [
        {
          user_id: `@${query.toLowerCase()}:example.invalid`,
          display_label: displayLabel,
          original_display_label: displayLabel,
          avatar: null,
          source: "profile",
          status: "selectable",
          status_message: null
        }
      ],
      explicit_user_id: null
    }
  };
  return next;
}

function mentionSnapshot(
  snapshot: DesktopSnapshot,
  stateGeneration: number,
  surface: MentionSurface,
  query: string,
  displayLabel: string,
  requestId: number,
  generation: number
): DesktopSnapshot {
  const next = snapshotWithGeneration(snapshot, stateGeneration);
  next.state.domain.mention_candidates.targets = [
    {
      room_id: ROOM_ID,
      generation,
      request_id: requestId,
      query,
      surface,
      completeness: "complete",
      candidates: [
        {
          user_id: `@${query}:example.invalid`,
          display_label: displayLabel,
          original_display_label: displayLabel,
          avatar: null,
          membership: "joined"
        }
      ],
      room_mention_allowed: "unknown",
      failure_kind: null
    }
  ];
  return next;
}

function changeEditorText(editor: HTMLElement, text: string): void {
  const control = editor as HTMLDivElement;
  setInlineMentionEditorSelection(control, 0, control.textContent?.length ?? 0);
  fireEvent(
    control,
    new InputEvent("beforeinput", {
      bubbles: true,
      cancelable: true,
      inputType: "insertText",
      data: text
    })
  );
}

async function openInviteDialog(api: DesktopApi): Promise<void> {
  await renderAppWithApi(api);
  await act(async () => {
    fireEvent.click(await screen.findByRole("button", { name: "Room info" }));
  });
  await act(async () => {
    fireEvent.click(await screen.findByRole("button", { name: "Invite people" }));
  });
  await screen.findByRole("dialog", { name: /Invite people to/ });
}

async function flushReact(): Promise<void> {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
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

describe("App query ownership leaf", () => {
  test("dispatches every deferred invite A/B/A query and renders the authoritative final A projection", async () => {
    const api = createBrowserFakeApi();
    const baseline = await api.getSnapshot();
    const results = [
      {
        query: "A",
        snapshot: inviteSnapshot(baseline, 10, "A", "Invite A old"),
        deferred: deferred<DesktopSnapshot>()
      },
      {
        query: "B",
        snapshot: inviteSnapshot(baseline, 20, "B", "Invite B"),
        deferred: deferred<DesktopSnapshot>()
      },
      {
        query: "A",
        snapshot: inviteSnapshot(baseline, 30, "A", "Invite A final"),
        deferred: deferred<DesktopSnapshot>()
      }
    ];
    const dispatched: string[] = [];
    vi.spyOn(api, "searchInviteTargets").mockImplementation((_roomId, query) => {
      const next = results[dispatched.length];
      if (!next || next.query !== query) {
        throw new Error(`unexpected invite query ${query}`);
      }
      dispatched.push(query);
      return next.deferred.promise;
    });

    await openInviteDialog(api);
    const input = screen.getByRole("textbox", { name: "Name, alias, or Matrix ID" });
    for (const query of ["A", "B", "A"]) {
      await act(async () => {
        fireEvent.change(input, { target: { value: query } });
      });
      await flushReact();
    }

    expect(dispatched).toEqual(["A", "B", "A"]);

    await act(async () => {
      results[1]!.deferred.resolve(results[1]!.snapshot);
      await results[1]!.deferred.promise;
    });
    await flushReact();
    await act(async () => {
      results[0]!.deferred.resolve(results[0]!.snapshot);
      await results[0]!.deferred.promise;
    });
    await flushReact();
    await act(async () => {
      results[2]!.deferred.resolve(results[2]!.snapshot);
      await results[2]!.deferred.promise;
    });

    await waitFor(() => {
      expect(screen.getByRole("button", { name: /Invite A final/ })).toBeTruthy();
      expect(screen.queryByRole("button", { name: /Invite A old/ })).toBeNull();
      expect(screen.queryByRole("button", { name: /Invite B/ })).toBeNull();
    });
  });

  test("does not restore a late invite projection after dialog replacement", async () => {
    const api = createBrowserFakeApi();
    const baseline = await api.getSnapshot();
    const pending = deferred<DesktopSnapshot>();
    const stale = inviteSnapshot(baseline, 10, "old", "Stale invite projection");
    vi.spyOn(api, "searchInviteTargets").mockImplementation(() => pending.promise);

    await openInviteDialog(api);
    const input = screen.getByRole("textbox", { name: "Name, alias, or Matrix ID" });
    await act(async () => {
      fireEvent.change(input, { target: { value: "old" } });
    });
    await waitFor(() => expect(api.searchInviteTargets).toHaveBeenCalledTimes(1));

    const replacement = snapshotWithGeneration(baseline, 40);
    vi.spyOn(api, "closeInviteWorkflow").mockResolvedValue(replacement);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: /Invite people to/ })).toBeNull());

    await act(async () => {
      pending.resolve(stale);
      await pending.promise;
    });
    await flushReact();

    const { getAppStoreSnapshot } = await import("./domain/appStore");
    expect(getAppStoreSnapshot()?.state_generation).toBe(40);
    expect(getAppStoreSnapshot()?.state.domain.invite_workflow?.query.query).toBe("");
    expect(screen.queryByText("Stale invite projection")).toBeNull();
  });

  test("dispatches every deferred main mention A/B/A query and renders its final Rust projection", async () => {
    const api = createBrowserFakeApi();
    const baseline = await api.getSnapshot();
    const results = [
      {
        query: "a",
        snapshot: mentionSnapshot(baseline, 10, "main", "a", "Mention A old", 11, 1),
        deferred: deferred<void>()
      },
      {
        query: "b",
        snapshot: mentionSnapshot(baseline, 20, "main", "b", "Mention B", 12, 2),
        deferred: deferred<void>()
      },
      {
        query: "a",
        snapshot: mentionSnapshot(baseline, 30, "main", "a", "Mention A final", 13, 3),
        deferred: deferred<void>()
      }
    ];
    let refreshSnapshot = baseline;
    const dispatched: string[] = [];
    vi.spyOn(api, "queryMentionCandidates").mockImplementation((_roomId, surface, query) => {
      const next = results[dispatched.length];
      if (!next || surface !== "main" || next.query !== query) {
        throw new Error(`unexpected main mention query ${query}`);
      }
      dispatched.push(query);
      return next.deferred.promise.then(() => {
        refreshSnapshot = next.snapshot;
      });
    });
    vi.spyOn(api, "getSnapshot").mockImplementation(async () => structuredClone(refreshSnapshot));

    await renderAppWithApi(api);
    const editor = await screen.findByRole("textbox", { name: "Message composer" });
    for (const query of ["@a", "@b", "@a"]) {
      await act(async () => changeEditorText(editor, query));
      await flushReact();
    }

    expect(dispatched).toEqual(["a", "b", "a"]);

    await act(async () => {
      results[1]!.deferred.resolve();
      await results[1]!.deferred.promise;
    });
    await flushReact();
    await act(async () => {
      results[0]!.deferred.resolve();
      await results[0]!.deferred.promise;
    });
    await flushReact();
    await act(async () => {
      results[2]!.deferred.resolve();
      await results[2]!.deferred.promise;
    });

    await waitFor(() => {
      expect(screen.getByRole("option", { name: /Mention A final/ })).toBeTruthy();
      expect(screen.queryByRole("option", { name: /Mention A old/ })).toBeNull();
      expect(screen.queryByRole("option", { name: /Mention B/ })).toBeNull();
    });
  });

  test("dispatches every deferred thread mention query", async () => {
    const api = createBrowserFakeApi();
    await api.openThread(ROOM_ID, THREAD_ROOT_ID, "existingThread");
    const baseline = await api.getSnapshot();
    const results = [
      {
        query: "a",
        snapshot: mentionSnapshot(baseline, 10, "thread", "a", "Thread A old", 21, 1),
        deferred: deferred<void>()
      },
      {
        query: "b",
        snapshot: mentionSnapshot(baseline, 20, "thread", "b", "Thread B", 22, 2),
        deferred: deferred<void>()
      },
      {
        query: "a",
        snapshot: mentionSnapshot(baseline, 30, "thread", "a", "Thread A final", 23, 3),
        deferred: deferred<void>()
      }
    ];
    let refreshSnapshot = baseline;
    const dispatched: string[] = [];
    vi.spyOn(api, "queryMentionCandidates").mockImplementation((_roomId, surface, query) => {
      const next = results[dispatched.length];
      if (!next || surface !== "thread" || next.query !== query) {
        throw new Error(`unexpected thread mention query ${query}`);
      }
      dispatched.push(query);
      return next.deferred.promise.then(() => {
        refreshSnapshot = next.snapshot;
      });
    });
    vi.spyOn(api, "getSnapshot").mockImplementation(async () => structuredClone(refreshSnapshot));

    await renderAppWithApi(api);
    await act(async () => {
      fireEvent.click(await screen.findByRole("button", { name: /View new replies · 2/ }));
    });
    const editor = await screen.findByRole("textbox", { name: "Thread composer" });
    for (const query of ["@a", "@b", "@a"]) {
      await act(async () => changeEditorText(editor, query));
      await flushReact();
    }

    expect(dispatched).toEqual(["a", "b", "a"]);

    await act(async () => {
      results[1]!.deferred.resolve();
      await results[1]!.deferred.promise;
    });
    await flushReact();
    await act(async () => {
      results[0]!.deferred.resolve();
      await results[0]!.deferred.promise;
    });
    await flushReact();
    await act(async () => {
      results[2]!.deferred.resolve();
      await results[2]!.deferred.promise;
    });

    await waitFor(() => {
      expect(screen.getByRole("option", { name: /Thread A final/ })).toBeTruthy();
      expect(screen.queryByRole("option", { name: /Thread A old/ })).toBeNull();
      expect(screen.queryByRole("option", { name: /Thread B/ })).toBeNull();
    });
  });

  test("does not restore a late main mention projection after room replacement", async () => {
    const api = createBrowserFakeApi();
    const baseline = await api.getSnapshot();
    const pending = deferred<void>();
    let refreshSnapshot = baseline;
    const stale = mentionSnapshot(baseline, 10, "main", "old", "Stale room mention", 31, 1);
    vi.spyOn(api, "queryMentionCandidates").mockImplementation(() =>
      pending.promise.then(() => {
        refreshSnapshot = stale;
      })
    );
    vi.spyOn(api, "getSnapshot").mockImplementation(async () => structuredClone(refreshSnapshot));

    await renderAppWithApi(api);
    const editor = await screen.findByRole("textbox", { name: "Message composer" });
    await act(async () => changeEditorText(editor, "@old"));
    await waitFor(() => expect(api.queryMentionCandidates).toHaveBeenCalledTimes(1));

    const replacement = snapshotWithGeneration(baseline, 40);
    replacement.state.ui.navigation.active_room_id = "!room-planning:example.invalid";
    replacement.state.ui.timeline.room_id = "!room-planning:example.invalid";
    replacement.state.domain.mention_candidates.targets = [];
    vi.spyOn(api, "selectRoom").mockResolvedValue(replacement);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "planning-room" }));
    });
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "planning-room" }).className).toContain(
        "is-active"
      )
    );

    await act(async () => {
      pending.resolve();
      await pending.promise;
    });
    await flushReact();

    const { getAppStoreSnapshot } = await import("./domain/appStore");
    expect(getAppStoreSnapshot()?.state_generation).toBe(40);
    expect(screen.queryByRole("option", { name: /Stale room mention/ })).toBeNull();
    expect(screen.getByRole("button", { name: "planning-room" }).className).toContain(
      "is-active"
    );
  });

  test("does not restore a late main mention projection after account replacement", async () => {
    const api = createBrowserFakeApi();
    const baseline = await api.getSnapshot();
    const pending = deferred<void>();
    let refreshSnapshot = baseline;
    const stale = mentionSnapshot(baseline, 10, "main", "old", "Stale account mention", 41, 1);
    vi.spyOn(api, "queryMentionCandidates").mockImplementation(() =>
      pending.promise.then(() => {
        refreshSnapshot = stale;
      })
    );
    vi.spyOn(api, "getSnapshot").mockImplementation(async () => structuredClone(refreshSnapshot));

    await renderAppWithApi(api);
    const editor = await screen.findByRole("textbox", { name: "Message composer" });
    await act(async () => changeEditorText(editor, "@old"));
    await waitFor(() => expect(api.queryMentionCandidates).toHaveBeenCalledTimes(1));

    const replacement = snapshotWithGeneration(baseline, 40);
    replacement.state.domain.session = {
      kind: "ready",
      homeserver: "https://matrix.example.invalid",
      user_id: "@second-user:example.invalid",
      device_id: "SECONDDEVICE"
    };
    replacement.state.domain.mention_candidates.targets = [];
    const { setAppStoreSnapshot } = await import("./domain/appStore");
    await act(async () => setAppStoreSnapshot(replacement));

    await act(async () => {
      pending.resolve();
      await pending.promise;
    });
    await flushReact();

    const { getAppStoreSnapshot } = await import("./domain/appStore");
    expect(getAppStoreSnapshot()?.state_generation).toBe(40);
    expect(getAppStoreSnapshot()?.state.domain.session.user_id).toBe(
      "@second-user:example.invalid"
    );
    expect(screen.queryByRole("option", { name: /Stale account mention/ })).toBeNull();
  });
});
