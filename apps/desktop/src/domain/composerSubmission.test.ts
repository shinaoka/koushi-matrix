import { describe, expect, it } from "vitest";
import {
  classifySubmissionFailure,
  createComposerSubmissionController,
  createComposerSubmissionControllerRegistry,
  mainSubmissionTarget,
  threadSubmissionTarget
} from "./composerSubmission";

describe("composer submission controller", () => {
  it("admits one submission synchronously", () => {
    const controller = createComposerSubmissionController(() => "submission-1");
    expect(controller.begin()).toBe("submission-1");
    expect(controller.begin()).toBeNull();
  });

  it("preserves the guard after acceptance until matching terminal release", () => {
    const controller = createComposerSubmissionController(() => "submission-1");
    const id = controller.begin()!;
    controller.accept(id);
    expect(controller.begin()).toBeNull();
    controller.releaseTerminal("other");
    expect(controller.begin()).toBeNull();
    controller.releaseTerminal(id);
    expect(controller.begin()).toBe("submission-1");
  });

  it("releases immediately on matching rejection", () => {
    const controller = createComposerSubmissionController(() => "submission-1");
    controller.reject(controller.begin()!);
    expect(controller.begin()).toBe("submission-1");
  });

  it("retries an unknown outcome with the same id and original captured payload", () => {
    const controller = createComposerSubmissionController(() => "submission-1");
    const id = controller.begin()!;
    controller.capture(id, { body: "original", target: "room-a" });
    controller.markUnknown(id, "timeout");

    expect(controller.begin()).toBe(id);
    controller.capture(id, { body: "edited", target: "room-b" });
    expect(controller.payload(id)).toEqual({ body: "original", target: "room-a" });
    expect(controller.unknownReason(id)).toBe("timeout");
  });

  it("keeps unknown submissions guarded until explicit rejection or observed terminal", () => {
    const controller = createComposerSubmissionController(() => "submission-1");
    const id = controller.begin()!;
    controller.markUnknown(id, "disconnected");
    controller.releaseTerminal(id);
    expect(controller.active()).toBe(id);
    controller.accept(id);
    controller.releaseTerminal(id);
    expect(controller.active()).toBeNull();
  });

  it.each(["timeout", "disconnected"] as const)("classifies %s as an unknown outcome", (failure) => {
    expect(classifySubmissionFailure(failure)).toEqual({ kind: "unknown", reason: failure });
  });

  it("classifies invalid as an explicit safe rejection", () => {
    expect(classifySubmissionFailure("invalid")).toEqual({ kind: "rejected", reason: "invalid" });
  });

  it("retries the same id exactly once when timeout happens after core admission", () => {
    const controller = createComposerSubmissionController(() => "submission-1");
    const admitted = new Set<string>();
    let sends = 0;
    const submit = (id: string) => {
      if (!admitted.has(id)) {
        admitted.add(id);
        sends += 1;
      }
    };
    const id = controller.begin()!;
    controller.capture(id, { body: "original" });
    submit(id);
    controller.markUnknown(id, "timeout");
    const retryId = controller.begin()!;
    submit(retryId);
    expect(retryId).toBe(id);
    expect(sends).toBe(1);
  });

  it("retries the same id exactly once when disconnect happens before admission", () => {
    const controller = createComposerSubmissionController(() => "submission-1");
    const id = controller.begin()!;
    controller.capture(id, { body: "original" });
    controller.markUnknown(id, "disconnected");
    const retryId = controller.begin()!;
    const admitted = new Set([retryId]);
    expect(retryId).toBe(id);
    expect(admitted.size).toBe(1);
    expect(controller.payload<{ body: string }>(retryId)?.body).toBe("original");
  });

  it("settles an unknown send from an accepted terminal snapshot and allocates a new id", () => {
    const ids = ["submission-1", "submission-2"];
    const controller = createComposerSubmissionController(() => ids.shift()!);
    const first = controller.begin()!;
    controller.capture(first, { body: "original" });
    controller.markUnknown(first, "timeout");

    controller.observeSnapshot(first, [first], null);

    const next = controller.begin()!;
    controller.capture(next, { body: "current draft" });
    expect(next).toBe("submission-2");
    expect(controller.payload<{ body: string }>(next)?.body).toBe("current draft");
  });

  it("scopes unknown main submissions across room navigation", () => {
    const ids = ["room-a-1", "room-b-1", "room-a-2"];
    const registry = createComposerSubmissionControllerRegistry(
      () => createComposerSubmissionController(() => ids.shift()!)
    );
    const roomA = registry.forTarget(mainSubmissionTarget("room-a"));
    const a1 = roomA.begin()!;
    roomA.capture(a1, { roomId: "room-a", body: "A original" });
    roomA.markUnknown(a1, "timeout");
    const roomB = registry.forTarget(mainSubmissionTarget("room-b"));
    const b1 = roomB.begin()!;
    roomB.capture(b1, { roomId: "room-b", body: "B current" });
    expect(b1).toBe("room-b-1");
    expect(roomB.payload<{ roomId: string }>(b1)?.roomId).toBe("room-b");
    registry.reconcile([a1], [a1]);
    const returnedA = registry.forTarget(mainSubmissionTarget("room-a"));
    expect(returnedA.begin()).toBe("room-a-2");
  });

  it("scopes unknown thread submissions by room and root", () => {
    const ids = ["thread-a", "thread-b"];
    const registry = createComposerSubmissionControllerRegistry(
      () => createComposerSubmissionController(() => ids.shift()!)
    );
    const a = registry.forTarget(threadSubmissionTarget("room", "root-a"));
    const aId = a.begin()!;
    a.markUnknown(aId, "timeout");
    const b = registry.forTarget(threadSubmissionTarget("room", "root-b"));
    expect(b.begin()).toBe("thread-b");
    expect(a.begin()).toBe(aId);
  });

  it("does not release an active unknown controller when unrelated tombstones evict", () => {
    const registry = createComposerSubmissionControllerRegistry(
      () => createComposerSubmissionController(() => "active-unknown")
    );
    const controller = registry.forTarget(mainSubmissionTarget("room-a"));
    const id = controller.begin()!;
    controller.markUnknown(id, "timeout");
    registry.reconcile(
      Array.from({ length: 128 }, (_, index) => `accepted-${index}`),
      Array.from({ length: 128 }, (_, index) => `settled-${index}`)
    );
    expect(controller.begin()).toBe(id);
  });

  it("preserves same-account unknown controllers across lock and releases after terminal", () => {
    const registry = createComposerSubmissionControllerRegistry(
      () => createComposerSubmissionController(() => "locked-id")
    );
    const controller = registry.forTarget(mainSubmissionTarget("room-a"));
    const id = controller.begin()!;
    controller.markUnknown(id, "timeout");
    expect(registry.forTarget(mainSubmissionTarget("room-a")).begin()).toBe(id);
    registry.reconcile([id], [id]);
    expect(registry.forTarget(mainSubmissionTarget("room-a")).begin()).toBe("locked-id");
  });

  it("clears all controllers on true account reset", () => {
    let sequence = 0;
    const registry = createComposerSubmissionControllerRegistry(
      () => createComposerSubmissionController(() => `id-${++sequence}`)
    );
    const old = registry.forTarget(mainSubmissionTarget("room-a"));
    const oldId = old.begin()!;
    old.markUnknown(oldId, "timeout");
    registry.reset();
    expect(registry.forTarget(mainSubmissionTarget("room-a")).begin()).toBe("id-2");
  });
});
