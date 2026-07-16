import { describe, expect, it } from "vitest";

import {
  createLatestAsyncOperationQueue,
  createLatestAsyncResultGate
} from "./latestAsyncResult";

describe("latest async result gate", () => {
  it("accepts only the newest result for each logical field", () => {
    const gate = createLatestAsyncResultGate<string>();
    const captionA = gate.begin("caption");
    const alias = gate.begin("alias");
    const captionB = gate.begin("caption");

    expect(captionA.isCurrent()).toBe(false);
    expect(captionB.isCurrent()).toBe(true);
    expect(alias.isCurrent()).toBe(true);
  });

  it("invalidates a pending field result when its surface closes", () => {
    const gate = createLatestAsyncResultGate<string>();
    const request = gate.begin("caption");

    gate.invalidate("caption");

    expect(request.isCurrent()).toBe(false);
  });

  it("settles only its own generation without invalidating a newer request", () => {
    const gate = createLatestAsyncResultGate<string>();
    const older = gate.begin("caption");
    const newer = gate.begin("caption");

    older.settle();
    expect(newer.isCurrent()).toBe(true);

    newer.settle();
    expect(newer.isCurrent()).toBe(false);
  });
});

describe("latest async operation queue", () => {
  it("serializes one logical field and skips superseded pending writes", async () => {
    const queue = createLatestAsyncOperationQueue<string>();
    const calls: string[] = [];
    let resolveFirst!: (value: string) => void;
    let resolveLatest!: (value: string) => void;

    const first = queue.run("caption", async () => {
      calls.push("first");
      return new Promise<string>((resolve) => {
        resolveFirst = resolve;
      });
    });
    await Promise.resolve();
    const middle = queue.run("caption", async () => {
      calls.push("middle");
      return "middle";
    });
    const latest = queue.run("caption", async () => {
      calls.push("latest");
      return new Promise<string>((resolve) => {
        resolveLatest = resolve;
      });
    });

    expect(calls).toEqual(["first"]);
    resolveFirst("old");
    await expect(first).resolves.toEqual({ kind: "superseded" });
    await expect(middle).resolves.toEqual({ kind: "superseded" });
    expect(calls).toEqual(["first", "latest"]);

    resolveLatest("new");
    await expect(latest).resolves.toEqual({ kind: "applied", value: "new" });
  });

  it("runs independent logical fields concurrently", async () => {
    const queue = createLatestAsyncOperationQueue<string>();
    const calls: string[] = [];

    const caption = queue.run("caption", async () => {
      calls.push("caption");
      return "caption";
    });
    const alias = queue.run("alias", async () => {
      calls.push("alias");
      return "alias";
    });

    await expect(Promise.all([caption, alias])).resolves.toEqual([
      { kind: "applied", value: "caption" },
      { kind: "applied", value: "alias" }
    ]);
    expect(calls).toEqual(["caption", "alias"]);
  });

  it("invalidates an active operation result", async () => {
    const queue = createLatestAsyncOperationQueue<string>();
    let resolveOperation!: (value: string) => void;
    const operation = queue.run(
      "caption",
      () => new Promise<string>((resolve) => {
        resolveOperation = resolve;
      })
    );
    await Promise.resolve();

    queue.invalidate("caption");
    resolveOperation("stale");

    await expect(operation).resolves.toEqual({ kind: "superseded" });
  });
});
