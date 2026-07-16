import { describe, expect, it } from "vitest";

import { createLatestAsyncResultGate } from "./latestAsyncResult";

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
