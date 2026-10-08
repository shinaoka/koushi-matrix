import { expect, test } from "@playwright/test";
import { gotoReadyShell, seedTimelineItems } from "./support/basicOperations";

const formulas = ["x", "x_i", "x^2", "\\frac{a}{b}", "\\min_x |Ax-b|^2"];
const longFormula = Array.from({ length: 80 }, () => "x").join("+");

test("inline math preserves its baseline and long formulas still scroll", async ({ page }) => {
  await gotoReadyShell(page);
  const html = formulas.map((formula) => `<p>Before <span data-mx-maths="${formula}">formula</span>, after.</p>`).join("") +
    `<p>Before <span data-mx-maths="${longFormula}">long inline</span>, after.</p>` +
    `<div data-mx-maths="${longFormula}">long block</div>`;
  await seedTimelineItems(page, [{
    id: { Event: { event_id: "$math-layout:example.invalid" } },
    sender: "@member:example.invalid", body: "Synthetic math layout", timestamp_ms: 1_800_000_000_000,
    formatted: { html, plain_text: "Synthetic math layout", code_blocks: [] },
    reactions: [], can_react: true, is_redacted: false, is_hidden: false, can_redact: false,
    is_edited: false, can_edit: false, thread_summary: null
  }]);
  const body = page.locator('[data-event-id="$math-layout:example.invalid"] .message-formatted-body');
  await expect(body.locator(".katex")).toHaveCount(7);
  await page.evaluate(() => document.fonts.ready);

  for (const fontSize of [14, 20]) {
    const result = await body.evaluate((element, size) => {
      const body = element as HTMLElement;
      body.style.fontSize = `${size}px`;
      body.style.inlineSize = "420px";
      body.style.maxInlineSize = "100%";
      const inline = Array.from(body.querySelectorAll<HTMLElement>(".message-math:not(.is-block)"));
      const deltas = inline.slice(0, 5).map((math) => {
        const inner = document.createElement("span");
        inner.style.cssText = "display:inline-block;width:0;height:0;padding:0;margin:0;vertical-align:baseline";
        const outer = inner.cloneNode() as HTMLElement;
        math.querySelector(".katex")!.append(inner);
        math.after(outer);
        const delta = inner.getBoundingClientRect().top - outer.getBoundingClientRect().top;
        inner.remove();
        outer.remove();
        return delta;
      });
      const wide = [inline[5]!, body.querySelector<HTMLElement>(".message-math.is-block")!].map((math) => {
        math.scrollLeft = 30;
        return {
          client: math.clientWidth, scroll: math.scrollWidth, moved: math.scrollLeft,
          body: body.clientWidth
        };
      });
      return { deltas, wide };
    }, fontSize);
    for (const delta of result.deltas) expect(Math.abs(delta)).toBeLessThan(0.5);
    for (const wide of result.wide) {
      expect(wide.client).toBeLessThanOrEqual(wide.body);
      expect(wide.scroll).toBeGreaterThan(wide.client);
      expect(wide.moved).toBeGreaterThan(0);
    }
  }
});

/// #1193: a formula that fits paints its own overflow instead of growing mini
/// scrollbars, and a formula wider than the message pane owns a horizontal scroll
/// container that never clips it vertically.
test("fitting inline math does not scroll and overwide math stays unclipped (#1193)", async ({ page }) => {
  await gotoReadyShell(page);
  const html = formulas.map((formula) => `<p>Before <span data-mx-maths="${formula}">formula</span>, after.</p>`).join("") +
    `<p>Before <span data-mx-maths="${longFormula}">long inline</span>, after.</p>` +
    `<div data-mx-maths="${longFormula}">long block</div>`;
  await seedTimelineItems(page, [{
    id: { Event: { event_id: "$math-overflow:example.invalid" } },
    sender: "@member:example.invalid", body: "Synthetic math overflow", timestamp_ms: 1_800_000_001_000,
    formatted: { html, plain_text: "Synthetic math overflow", code_blocks: [] },
    reactions: [], can_react: true, is_redacted: false, is_hidden: false, can_redact: false,
    is_edited: false, can_edit: false, thread_summary: null
  }]);
  const body = page.locator('[data-event-id="$math-overflow:example.invalid"] .message-formatted-body');
  await expect(body.locator(".katex")).toHaveCount(7);
  await page.evaluate(() => document.fonts.ready);

  // Short formulas at a narrow pane and a larger font: no scroll region on either
  // axis. An inline-level box reports no client/scroll metrics, so probe the scroll
  // positions instead.
  const fitting = await body.evaluate((element) => {
    const body = element as HTMLElement;
    body.style.inlineSize = "420px";
    body.style.maxInlineSize = "100%";
    body.style.fontFamily = "Arial, sans-serif";
    body.style.fontSize = "16px";
    return Array.from(
      body.querySelectorAll<HTMLElement>(".message-math:not(.is-block)")
    ).slice(0, 5).map((math) => {
      math.scrollLeft = 30;
      math.scrollTop = 30;
      return {
        latex: math.dataset.mxMaths,
        inline: math.scrollLeft,
        block: math.scrollTop,
        overwide: math.classList.contains("is-overwide")
      };
    });
  });
  for (const entry of fitting) {
    expect(entry, "a fitting inline formula must not scroll or opt into the scroll container").toEqual({
      latex: entry.latex,
      inline: 0,
      block: 0,
      overwide: false
    });
  }

  // The long formula opts into the scroll container only after the pane measurement.
  await expect(body.locator(".message-math:not(.is-block).is-overwide")).toHaveCount(1);
  const overwide = await body.evaluate((element) => {
    const body = element as HTMLElement;
    const metrics = (math: HTMLElement) => {
      math.scrollLeft = 30;
      return {
        moved: math.scrollLeft,
        horizontalOverflow: math.scrollWidth > math.clientWidth,
        verticalOverflow: math.scrollHeight > math.clientHeight
      };
    };
    const inline = body.querySelector<HTMLElement>(".message-math:not(.is-block).is-overwide")!;
    const block = body.querySelector<HTMLElement>(".message-math.is-block")!;
    const display = block.querySelector<HTMLElement>(".katex-display")!;
    return {
      inline: metrics(inline),
      block: metrics(block),
      blockMargin: getComputedStyle(block).marginBlockStart,
      displayMargin: getComputedStyle(display).marginBlockStart
    };
  });
  expect(overwide.inline).toEqual({ moved: 30, horizontalOverflow: true, verticalOverflow: false });
  expect(overwide.block.moved).toBeGreaterThan(0);
  expect(overwide.block.horizontalOverflow).toBe(true);
  expect(overwide.block.verticalOverflow).toBe(false);
  expect(overwide.blockMargin).toBe("6px");
  expect(overwide.displayMargin).toBe("0px");
});
