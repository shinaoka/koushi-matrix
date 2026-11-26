import { describe, expect, it } from "vitest";

import { runPdfThumbnailJob } from "./pdfThumbnailQueue";

describe("PDF thumbnail queue (#2096)", () => {
  it("runs jobs one at a time and skips a cancelled one", async () => {
    let running = 0;
    let peak = 0;
    const order: string[] = [];
    const job = (name: string) => async () => {
      running += 1;
      peak = Math.max(peak, running);
      await new Promise((resolve) => setTimeout(resolve, 5));
      order.push(name);
      running -= 1;
      return name;
    };
    const results = await Promise.all([
      runPdfThumbnailJob(job("a"), () => false),
      runPdfThumbnailJob(job("b"), () => true),
      runPdfThumbnailJob(job("c"), () => false)
    ]);
    expect(results).toEqual(["a", null, "c"]);
    expect(order).toEqual(["a", "c"]);
    expect(peak).toBe(1);
  });

  it("keeps running after a job throws", async () => {
    await expect(
      runPdfThumbnailJob(async () => {
        throw new Error("boom");
      }, () => false)
    ).rejects.toThrow("boom");
    await expect(runPdfThumbnailJob(async () => "next", () => false)).resolves.toBe("next");
  });
});
