import { describe, expect, it } from "vitest";

import {
  ComposerDraftRendererRetiredError,
  createComposerDraftLifecycleRegistry,
  type ComposerDraftLeaseSnapshot,
  type ComposerDraftLifecycleBackend,
  type ComposerDraftScope
} from "./composerDraftLifecycle";
import { parseComposerDraftRevision } from "./composerDraftRevision";

function account(name: string) {
  return {
    homeserver: `https://${name}.invalid`,
    user_id: `@${name}:invalid`,
    device_id: `${name}-device`
  };
}

function main(owner: ReturnType<typeof account>, roomId: string): ComposerDraftScope {
  return { account: owner, target: { kind: "main", room_id: roomId } };
}

function thread(
  owner: ReturnType<typeof account>,
  roomId: string,
  rootId: string
): ComposerDraftScope {
  return {
    account: owner,
    target: { kind: "thread", room_id: roomId, root_event_id: rootId }
  };
}

function revision(value: number | string) {
  return parseComposerDraftRevision(String(value));
}

function immediateBackend(): ComposerDraftLifecycleBackend {
  let nextLease = 1;
  return {
    acquire: async (_scope, rendererGeneration) => ({
      rendererGeneration,
      leaseId: String(nextLease++),
      revision: revision(0),
      lastAcceptedClearRevision: revision(0),
      hasAuthoritativeContent: false
    }),
    release: async () => {}
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

describe("composer draft lifecycle registry", () => {
  it("bounds room and thread quiescent entries by lifecycle order", async () => {
    const owner = account("bounds");
    const registry = createComposerDraftLifecycleRegistry(immediateBackend());
    const oldestMain = main(owner, "z-oldest");
    const oldestThread = thread(owner, "z-room", "z-oldest-root");
    const touchedMain = main(owner, "middle-main");
    const touchedThread = thread(owner, "middle-room", "middle-root");

    registry.observe(oldestMain, revision(0), revision(0), false);
    for (let index = 1; index < 128; index += 1) {
      registry.observe(main(owner, `main-${index}`), revision(0), revision(0), false);
    }
    registry.observe(oldestThread, revision(0), revision(0), false);
    for (let index = 1; index < 256; index += 1) {
      registry.observe(
        thread(owner, `thread-room-${index}`, `root-${index}`),
        revision(0),
        revision(0),
        false
      );
    }
    registry.observe(touchedMain, revision(0), revision(0), false);
    registry.observe(touchedThread, revision(0), revision(0), false);
    await registry.activate(touchedMain);
    await registry.deactivate(touchedMain);
    await registry.activate(touchedThread);
    await registry.deactivate(touchedThread);

    const newestMain = main(owner, "a-newest-main");
    const newestThread = thread(owner, "a-newest-room", "a-newest-root");
    registry.observe(newestMain, revision(0), revision(0), false);
    registry.observe(newestThread, revision(0), revision(0), false);

    expect(registry.counts()).toEqual({
      quiescentMain: 128,
      quiescentThread: 256,
      protected: 0
    });
    expect(registry.has(oldestMain)).toBe(false);
    expect(registry.has(oldestThread)).toBe(false);
    expect(registry.has(touchedMain)).toBe(true);
    expect(registry.has(touchedThread)).toBe(true);
    expect(registry.has(newestMain)).toBe(true);
    expect(registry.has(newestThread)).toBe(true);
  });

  it("protects authoritative content active timers and pending operations", async () => {
    const owner = account("protected");
    const registry = createComposerDraftLifecycleRegistry(immediateBackend());
    const protectedScopes = [
      main(owner, "content"),
      main(owner, "active"),
      main(owner, "timer"),
      thread(owner, "operation-room", "operation-root"),
      thread(owner, "overlay-room", "overlay-root")
    ];
    registry.observe(protectedScopes[0], revision(1), revision(0), true);
    await registry.activate(protectedScopes[1]);
    registry.observe(protectedScopes[2], revision(1), revision(0), false);
    registry.setDebounce(protectedScopes[2], 7);
    registry.observe(protectedScopes[3], revision(1), revision(0), false);
    registry.beginOperation(protectedScopes[3]);
    registry.observe(protectedScopes[4], revision(1), revision(0), false);
    registry.setActiveOverlay(protectedScopes[4], "typed");

    for (let index = 0; index < 140; index += 1) {
      registry.observe(main(owner, `churn-main-${index}`), revision(0), revision(0), false);
    }
    for (let index = 0; index < 270; index += 1) {
      registry.observe(
        thread(owner, `churn-room-${index}`, `churn-root-${index}`),
        revision(0),
        revision(0),
        false
      );
    }

    expect(registry.counts()).toEqual({
      quiescentMain: 128,
      quiescentThread: 256,
      protected: 5
    });
    for (const scope of protectedScopes) {
      expect(registry.has(scope)).toBe(true);
    }
  });

  it("retires protected entries after flush and settlement", () => {
    const owner = account("settlement");
    const registry = createComposerDraftLifecycleRegistry(immediateBackend());
    const scope = main(owner, "settle");
    registry.observe(scope, revision(1), revision(1), false);
    registry.setDebounce(scope, 3);
    const capture = registry.beginOperation(scope);

    registry.clearDebounce(scope);
    expect(registry.counts().protected).toBe(1);
    expect(registry.settleOperation(capture)).toBe(true);

    expect(registry.counts()).toEqual({
      quiescentMain: 1,
      quiescentThread: 0,
      protected: 0
    });
  });

  it("rejects late completion from a retired renderer generation", () => {
    const owner = account("retired");
    const registry = createComposerDraftLifecycleRegistry(immediateBackend());
    const scope = main(owner, "retired-room");
    registry.observe(scope, revision(4), revision(3), false);
    registry.setActiveOverlay(scope, "new input");
    const capture = registry.beginOperation(scope);

    registry.revokeRendererGeneration();

    expect(registry.settleOperation(capture)).toBe(false);
    expect(registry.has(scope)).toBe(true);
    expect(registry.snapshot(scope)).toMatchObject({
      revision: "4",
      lastAcceptedClearRevision: "3"
    });
  });

  it("isolates complete account main and thread scopes", () => {
    const ownerA = account("account-a");
    const ownerB = account("account-b");
    const registry = createComposerDraftLifecycleRegistry(immediateBackend());
    const mainA = main(ownerA, "same-room");
    const mainB = main(ownerB, "same-room");
    const threadA = thread(ownerA, "same-room", "same-root");
    const threadB = thread(ownerB, "same-room", "same-root");
    registry.observe(mainA, revision(7), revision(6), true);
    registry.observe(threadA, revision(9), revision(8), false);
    registry.observe(mainB, revision(2), revision(1), false);
    registry.observe(threadB, revision(3), revision(2), true);

    registry.nextDraft(mainA);
    registry.nextDraft(threadA);

    expect(registry.snapshot(mainB)).toMatchObject({
      revision: "2",
      lastAcceptedClearRevision: "1"
    });
    expect(registry.snapshot(threadB)).toMatchObject({
      revision: "3",
      lastAcceptedClearRevision: "2"
    });
  });

  it("rehydrates rather than max-merging a fresh lease generation", async () => {
    const owner = account("rehydrate");
    const scope = main(owner, "rehydrate-room");
    const acquire = deferred<ComposerDraftLeaseSnapshot>();
    const backend: ComposerDraftLifecycleBackend = {
      acquire: () => acquire.promise,
      release: async () => {}
    };
    const registry = createComposerDraftLifecycleRegistry(backend);
    registry.observe(scope, revision(12), revision(11), false);

    const activation = registry.activate(scope);
    registry.setActiveOverlay(scope, "typed during acquire");
    acquire.resolve({
      rendererGeneration: "1",
      leaseId: "lease",
      revision: revision(3),
      lastAcceptedClearRevision: revision(2),
      hasAuthoritativeContent: false
    });
    await activation;

    expect(registry.snapshot(scope)).toMatchObject({
      revision: "3",
      lastAcceptedClearRevision: "2"
    });
  });

  it("rejects an acquire that resolves after renderer revocation", async () => {
    const owner = account("late-acquire");
    const scope = main(owner, "late-room");
    const acquire = deferred<ComposerDraftLeaseSnapshot>();
    const backend: ComposerDraftLifecycleBackend = {
      acquire: () => acquire.promise,
      release: async () => {}
    };
    const registry = createComposerDraftLifecycleRegistry(backend);
    const activation = registry.activate(scope);
    registry.revokeRendererGeneration();
    acquire.resolve({
      rendererGeneration: "1",
      leaseId: "late",
      revision: revision(1),
      lastAcceptedClearRevision: revision(0),
      hasAuthoritativeContent: false
    });

    await expect(activation).rejects.toBeInstanceOf(ComposerDraftRendererRetiredError);
    expect(registry.snapshot(scope)).toMatchObject({ revision: "0" });
  });
});
