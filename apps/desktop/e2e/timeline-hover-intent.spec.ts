/**
 * Headless regression for #1603.
 *
 * A trackpad scroll moves timeline rows under a stationary pointer. The
 * browser still delivers `:hover`/`mouseenter` to whatever row is under the
 * pointer, so the row's hover background, floating action bar, and tooltip
 * must wait for real hover intent: scrolling stopped AND the pointer moved
 * after that scroll began. Keyboard focus is independent and stays immediate.
 */

import { expect, test, type Page } from "@playwright/test";

const key = {
  account_key: "@harness-user:example.invalid",
  kind: { Room: { room_id: "!harness-room:example.invalid" } }
};

function item(id: string) {
  return {
    id: { Event: { event_id: id } },
    sender: "@sender:example.invalid",
    body: "Synthetic hover row",
    timestamp_ms: 1_800_000_000_000,
    in_reply_to_event_id: null,
    thread_root: null,
    thread_summary: null,
    can_react: true,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false,
    reactions: [],
    actions: {
      can_copy: true,
      can_forward: false,
      can_reply: true,
      can_permalink: false,
      can_view_source: false
    }
  };
}

async function openTimeline(page: Page) {
  // The full app harness bundles styles.css, so the computed opacity under
  // test is the same sheet production ships (and WebKit can run the spec).
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Reply to message" }).first()).toBeVisible();
  const view = page.getByTestId("timeline-view");
  await expect(view).toBeVisible();
  await page.evaluate(({ key, items }) => {
    window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: { InitialItems: { request_id: null, key, generation: 1, items } }
      // The fixture is a Rust-shaped transport payload, not a reducer.
    } as Parameters<typeof window.__harness.pushCoreEvent>[0]);
  }, { key, items: Array.from({ length: 80 }, (_, i) => item(`$hover-row-${i}`)) });
  await expect(view).toHaveAttribute("data-total-items", "80");
  // Sit in the middle of the history, then let layout settle.
  await view.evaluate((node) => {
    node.scrollTop = 1800;
    node.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -2 }));
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  // Let the initial programmatic placement settle, then arm the gate with a
  // real pointer move before the test parks the pointer.
  await expect(view).not.toHaveAttribute("data-scroll-active", "true");
  const box = await view.boundingBox();
  if (!box) throw new Error("timeline viewport missing");
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await expect(view).toHaveAttribute("data-hover-armed", "true");
  return view;
}

async function hoveredActionsOpacity(page: Page, x: number, y: number) {
  return page.evaluate(({ x, y }) => {
    const actions = document
      .elementFromPoint(x, y)
      ?.closest("article.message")
      ?.querySelector<HTMLElement>(".message-actions");
    return actions ? Number(getComputedStyle(actions).opacity) : null;
  }, { x, y });
}

test("scrolling under a parked pointer waits for movement before revealing hover chrome", async ({
  page
}) => {
  const view = await openTimeline(page);
  const box = await view.boundingBox();
  if (!box) throw new Error("timeline viewport missing");
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;

  // Park the pointer, then start a gesture that moves rows underneath it.
  await page.mouse.move(x, y);
  const before = await view.evaluate((node) => node.scrollTop);
  // The scroll-idle flush clears `data-scroll-active` after roughly 100 ms, so
  // polling for it can miss the window under a loaded runner. The gate writes
  // it synchronously inside the dispatched scroll event; read it in the same
  // task instead.
  const gesture = await view.evaluate((node) => {
    node.scrollTop -= 120;
    node.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -40 }));
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
    return {
      scrollTop: node.scrollTop,
      scrollActive: node.getAttribute("data-scroll-active"),
      hoverArmed: node.getAttribute("data-hover-armed")
    };
  });
  expect(gesture.scrollTop).toBeLessThan(before);
  expect(gesture.scrollActive).toBe("true");
  expect(gesture.hoverArmed).not.toBe("true");
  expect(await hoveredActionsOpacity(page, x, y)).toBe(0);

  // Scrolling has stopped, but a row came to rest under a pointer that never
  // moved, so the gate stays closed.
  await expect(view).not.toHaveAttribute("data-scroll-active", "true");
  await expect(view).not.toHaveAttribute("data-hover-armed", "true");
  expect(await hoveredActionsOpacity(page, x, y)).toBe(0);

  // Keyboard focus is not pointer chrome and must reveal immediately.
  const focusedOpacity = await view.evaluate((node) => {
    const row = [...node.querySelectorAll<HTMLElement>("article.message")].find((candidate) => {
      const rect = candidate.getBoundingClientRect();
      const top = node.getBoundingClientRect().top;
      return rect.bottom > top && rect.top < top + node.clientHeight;
    });
    const action = row?.querySelector<HTMLElement>(".message-action");
    if (!action) throw new Error("visible row action missing");
    action.focus({ preventScroll: true });
    const actions = row.querySelector<HTMLElement>(".message-actions");
    return actions ? Number(getComputedStyle(actions).opacity) : null;
  });
  expect(focusedOpacity).toBe(1);
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());

  // The reader's next real pointer movement re-arms the gate, and only then
  // does the action bar fade in.
  await page.mouse.move(x + 1, y);
  await expect(view).toHaveAttribute("data-hover-armed", "true");
  await expect.poll(() => hoveredActionsOpacity(page, x + 1, y)).toBe(1);
});
