import { describe, expect, test, vi } from "vitest";

import { createOrderedEventBatcher } from "./orderedEventBatcher";

describe("createOrderedEventBatcher", () => {
  test("publishes a synchronous burst once without changing event order", async () => {
    const flush = vi.fn();
    const batcher = createOrderedEventBatcher<number>(flush);

    for (let value = 0; value < 100; value += 1) {
      batcher.enqueue(value);
    }
    expect(flush).not.toHaveBeenCalled();

    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(flush).toHaveBeenCalledTimes(1);
    expect(flush).toHaveBeenCalledWith(Array.from({ length: 100 }, (_, index) => index));
  });

  test("drops a queued publication after disposal", async () => {
    const flush = vi.fn();
    const batcher = createOrderedEventBatcher<number>(flush);
    batcher.enqueue(1);
    batcher.dispose();

    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(flush).not.toHaveBeenCalled();
  });
});
