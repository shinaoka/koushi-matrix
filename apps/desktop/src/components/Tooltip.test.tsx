import { readFileSync } from "node:fs";

import { describe, expect, test } from "vitest";

const tooltipSource = readFileSync(new URL("./Tooltip.tsx", import.meta.url), "utf8");

describe("Tooltip", () => {
  test("registers unmount cleanup for delayed hover timers", () => {
    expect(tooltipSource).toContain("return () => clearOpenTimer();");
    expect(tooltipSource).not.toContain("useEffect(() => clearOpenTimer, [])");
  });

  test("checks reduced-motion preference before scheduling hover delay", () => {
    expect(tooltipSource).toContain("(prefers-reduced-motion: reduce)");
    expect(tooltipSource).toContain("openNow();");
  });
});
