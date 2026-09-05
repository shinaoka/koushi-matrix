import { expect, test, type Page } from "@playwright/test";
import path from "node:path";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const probe = window as Window & { viewportResizeLoopErrors?: number };
    probe.viewportResizeLoopErrors = 0;
    window.addEventListener("error", event => {
      if (event.message.includes("ResizeObserver loop")) probe.viewportResizeLoopErrors! += 1;
    });
  });
});
test.afterEach(async ({ page }) => {
  expect(await page.evaluate(() => (window as Window & { viewportResizeLoopErrors?: number }).viewportResizeLoopErrors ?? 0)).toBe(0);
});

const key = { account_key: "@harness-user:example.invalid", kind: { Room: { room_id: "!harness-room:example.invalid" } } };
function item(id: string) {
  return {
    id: { Event: { event_id: id } }, sender: "@sender:example.invalid", body: "Synthetic row",
    timestamp_ms: 1_800_000_000_000, in_reply_to_event_id: null, thread_root: null,
    thread_summary: null, can_react: false, is_redacted: false, is_hidden: false,
    can_redact: false, is_edited: false, can_edit: false, reactions: []
  };
}
async function setup(page: Page, count: number) {
  await page.goto("/harness.html?variableHeights=true");
  const view = page.getByTestId("timeline-view");
  await expect(view).toBeVisible();
  await page.addStyleTag({ path: path.resolve(import.meta.dirname, "../src/styles.css") });
  await page.evaluate(() => document.fonts.ready.then(() => undefined));
  await page.evaluate(({ key, items }) => {
    window.__harness.pushCoreEvent({ kind: "Timeline", event: {
      InitialItems: { request_id: null, key, generation: 1, items }
    // The fixture is a Rust-shaped transport payload, not a frontend reducer.
    } } as Parameters<typeof window.__harness.pushCoreEvent>[0]);
  }, { key, items: Array.from({ length: count }, (_, i) => item(`$row-${i}`)) });
  await expect(view).toHaveAttribute("data-total-items", String(count));
  await view.evaluate((node, top) => {
    node.scrollTop = top;
    node.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -2 }));
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
  }, count > 500 ? 20000 : 1800);
  await expect.poll(() => view.evaluate((node) => {
    const top = node.getBoundingClientRect().top;
    return [...node.querySelectorAll<HTMLElement>("[data-item-id]")].some(row => {
      const rect = row.getBoundingClientRect();
      return rect.bottom > top && rect.top < top + node.clientHeight;
    });
  })).toBe(true);
  await page.evaluate(() => document.fonts.ready.then(() => undefined));
  await view.hover();
  if (count > 500) {
    await expect.poll(() => page.evaluate(() => {
      const diagnostics = window.__harness.scrollDiagnostics();
      return diagnostics?.pendingMeasuredRows === 0;
    })).toBe(true);
  }
  return view;
}
async function anchor(page: Page) {
  return page.getByTestId("timeline-view").evaluate((node) => {
    const top = node.getBoundingClientRect().top;
    const row = [...node.querySelectorAll<HTMLElement>("[data-item-id]")].find(row => {
      const rect = row.getBoundingClientRect();
      return rect.bottom > top && rect.top < top + node.clientHeight;
    });
    if (!row) throw new Error("visible synthetic anchor missing");
    return { id: row.dataset.itemId!, offset: row.getBoundingClientRect().top - top };
  });
}
async function offset(page: Page, id: string) {
  return page.getByTestId("timeline-view").evaluate((node, id) => {
    const row = [...node.querySelectorAll<HTMLElement>("[data-item-id]")].find(row => row.dataset.itemId === id);
    return row ? row.getBoundingClientRect().top - node.getBoundingClientRect().top : null;
  }, id);
}

for (const count of [120, 800]) {
  test(`upward input across four prepend batches preserves the visible anchor (${count} rows)`, async ({ page }) => {
    const view = await setup(page, count);
    const before = await anchor(page);
    for (let frame = 0; frame < 12; frame++) {
      await page.mouse.wheel(0, -2);
      await view.evaluate((_node, { key, items, batch }) => {
        if (items.length) window.__harness.pushCoreEvent({ kind: "Timeline", event: {
          ItemsUpdated: { key, generation: 1, batch_id: batch, diffs: items.map(item => ({ PushFront: { item } })) }
        } } as Parameters<typeof window.__harness.pushCoreEvent>[0]);
      }, { key, batch: frame / 3 + 1, items: frame % 3 === 0 ? Array.from({ length: 10 }, (_, i) => item(`$old-${frame}-${i}`)) : [] });
      try {
        await expect.poll(async () => {
          const actual = await offset(page, before.id);
          return actual === null ? Infinity : Math.abs(actual - (before.offset + (frame + 1) * 2));
        }, { message: `input ${frame}` }).toBeLessThanOrEqual(1);
      } catch (error) {
        await test.info().attach("viewport-input-diagnostics", {
          contentType: "application/json",
          body: JSON.stringify({ before: before.offset, after: await offset(page, before.id), logs: await page.evaluate(() => window.__harness.diagnosticLogs().filter(e => e.source === "timeline.viewport_transaction").slice(-40)) })
        });
        throw error;
      }
    }
    await expect(view).toHaveAttribute("data-total-items", String(count + 40));
    await expect.poll(async () => {
      const actual = await offset(page, before.id);
      return actual === null ? Infinity : Math.abs(actual - before.offset - 24);
    }).toBeLessThanOrEqual(1);
  });

  test(`later independent row resize preserves the settled anchor (${count} rows)`, async ({ page }) => {
    const view = await setup(page, count);
    const before = await anchor(page);
    // Establish a committed prepend first; resize is a later DOM observation,
    // not an invented CoreEvent or a second renderer projection owner.
    await page.evaluate(({ key, items }) => {
      window.__harness.pushCoreEvent({ kind: "Timeline", event: {
        ItemsUpdated: { key, generation: 1, batch_id: 1, diffs: items.map(item => ({ PushFront: { item } })) }
      } } as Parameters<typeof window.__harness.pushCoreEvent>[0]);
    }, { key, items: [item("$older-a"), item("$older-b")] });
    await expect(view).toHaveAttribute("data-total-items", String(count + 2));
    await expect.poll(async () => {
      const actual = await offset(page, before.id);
      return actual === null ? Infinity : Math.abs(actual - before.offset);
    }).toBeLessThanOrEqual(1);
    await expect.poll(() => page.evaluate(() => window.__harness.diagnosticLogs().filter(e => e.source === "timeline.viewport_transaction").at(-1)?.message)).toContain("reason=settled");
    await page.evaluate(() => window.__harness.clearDiagnosticLogs());
    const resized = await view.evaluate((node, id) => {
      const rows = [...node.querySelectorAll<HTMLElement>(".timeline-item-frame")];
      const index = rows.findIndex(row => row.querySelector<HTMLElement>("[data-item-id]")?.dataset.itemId === id);
      if (index < 1) throw new Error("above-anchor mounted row missing");
      const preceding = rows[index - 1];
      const list = preceding.parentElement!;
      const beforeHeight = list.getBoundingClientRect().height;
      preceding.style.minHeight = `${preceding.getBoundingClientRect().height + 80}px`;
      return { beforeHeight, afterHeight: list.getBoundingClientRect().height, rowHeight: preceding.getBoundingClientRect().height };
    }, before.id);
    expect(resized.afterHeight - resized.beforeHeight).toBeCloseTo(80, 2);
    // Force a headless compositor frame; the assertion remains DOM geometry,
    // not a screenshot comparison or a wall-clock sleep.
    await page.screenshot({ path: test.info().outputPath("viewport-resize.png") });
    try {
      await expect.poll(async () => {
        const actual = await offset(page, before.id);
        return actual === null ? Infinity : Math.abs(actual - before.offset);
      }).toBeLessThanOrEqual(1);
    } catch (error) {
      await test.info().attach("viewport-resize-diagnostics", {
        contentType: "application/json",
        body: JSON.stringify(await page.evaluate(() => ({ logs: window.__harness.diagnosticLogs().filter(e => e.source === "timeline.viewport_transaction").slice(-40), scroll: window.__harness.scrollDiagnostics() })))
      });
      throw error;
    }
    const diagnostics = await page.evaluate(() => window.__harness.diagnosticLogs().filter(entry => entry.source === "timeline.viewport_transaction"));
    expect(diagnostics.length).toBeGreaterThan(0);
    expect(JSON.stringify(diagnostics)).not.toMatch(/example\.invalid|\$row-|\$older-/);
  });
}

test("input during a deferred prepend and native resize preserves the original visible anchor", async ({ page }) => {
  const view = await setup(page, 120);
  const before = await anchor(page);
  await view.evaluate((node, { key, items, anchorId }) => {
    const move = () => {
      node.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -2 }));
      node.scrollTop -= 2;
      node.dispatchEvent(new Event("scroll", { bubbles: true }));
    };
    move();
    window.__harness.pushCoreEvent({ kind: "Timeline", event: {
      ItemsUpdated: { key, generation: 1, batch_id: 1, diffs: items.map(item => ({ PushFront: { item } })) }
    } } as Parameters<typeof window.__harness.pushCoreEvent>[0]);
    const rows = [...node.querySelectorAll<HTMLElement>(".timeline-item-frame")];
    const index = rows.findIndex(row => row.querySelector<HTMLElement>("[data-item-id]")?.dataset.itemId === anchorId);
    if (index < 1) throw new Error("above-anchor row missing");
    const preceding = rows[index - 1];
    preceding.style.minHeight = `${preceding.getBoundingClientRect().height + 80}px`;
    move();
  }, { key, items: Array.from({ length: 8 }, (_, i) => item(`$coupled-${i}`)), anchorId: before.id });
  await page.screenshot({ path: test.info().outputPath("coupled-resize.png") });
  await expect(view).toHaveAttribute("data-total-items", "128");
  await expect.poll(async () => {
    const actual = await offset(page, before.id);
    return actual === null ? Infinity : Math.abs(actual - before.offset - 4);
  }).toBeLessThanOrEqual(1);
});
