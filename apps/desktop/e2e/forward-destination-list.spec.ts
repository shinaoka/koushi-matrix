import { expect, test, type Page } from "@playwright/test";

import { gotoReadyShell, seedTimelineItems } from "./support/basicOperations";

// #2078: the "…" › Forward destination list used to grow as one unbroken
// column inside the message row, so the timeline clipped it at the pane's
// bottom edge and every room past that edge was unreachable: the wheel did
// nothing, Arrow Down did nothing, and Tab walked into the cut-off part where
// Enter forwarded to a room that had never been on screen.
//
// The account in the report was in 42 rooms; the window was 1280×800. Both
// reproduction sizes run here, in both engines, because a viewport-placed panel
// is exactly what differs between them.

const ROOM_COUNT = 42;
/** The account's own room comes first, so the list holds one more row. */
const DESTINATIONS = ROOM_COUNT + 1;
/** Destination rows are 30px tall; the count is what the report's window fits. */
const VISIBLE_ROWS_IN_REPORT = 5;
const VIEWPORTS = [
  { height: 700, width: 1200 },
  { height: 800, width: 1280 }
];

const LIST = ".message-forward-menu";
const ROW = `${LIST} [role="menuitem"]`;

async function seedRooms(page: Page, count = ROOM_COUNT): Promise<void> {
  await page.evaluate((count) => {
    const snapshot = window.__harness.currentSnapshot();
    const base = snapshot.state.domain.rooms[0];
    const rooms = Array.from({ length: count }, (_, index) => ({
      ...base,
      room_id: `!forward-destination-${index}:example.invalid`,
      display_label: `Forward destination ${String(index + 1).padStart(2, "0")}`,
      is_dm: false,
      parent_space_ids: []
    }));
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: { ...snapshot.state.domain, rooms: [...snapshot.state.domain.rooms, ...rooms] }
      }
    });
    window.__harness.pushStateUpdate();
  }, count);
}

async function openForwardList(
  page: Page,
  eventId = "$seed-event:example.invalid"
): Promise<void> {
  const row = page.locator(`[data-event-id="${eventId}"]`);
  await expect(row).toBeVisible();
  const actionsMenu = row.getByRole("button", { name: "Message actions" });
  await actionsMenu.click();
  await expect(actionsMenu).toHaveAttribute("aria-expanded", "true");
  await page.getByRole("menuitem", { name: "Forward", exact: true }).click();
  await expect(page.locator(ROW).first()).toBeVisible();
}

interface Box {
  bottom: number;
  clientHeight: number;
  height: number;
  overflowY: string;
  scrollHeight: number;
  scrollTop: number;
  top: number;
}

async function listBox(page: Page): Promise<Box> {
  return page.locator(LIST).evaluate((element) => {
    const rect = element.getBoundingClientRect();
    const list = element as HTMLElement;
    return {
      bottom: rect.bottom,
      clientHeight: list.clientHeight,
      height: rect.height,
      overflowY: getComputedStyle(list).overflowY,
      scrollHeight: list.scrollHeight,
      scrollTop: list.scrollTop,
      top: rect.top
    };
  });
}

/** Whether the focused destination row is fully inside the list's visible box. */
async function focusedRowIsVisible(page: Page): Promise<{
  label: string;
  visible: boolean;
}> {
  return page.evaluate((listSelector) => {
    const list = document.querySelector<HTMLElement>(listSelector);
    const active = document.activeElement as HTMLElement | null;
    if (!list || !active || !list.contains(active)) {
      return { label: active?.textContent?.trim() ?? "", visible: false };
    }
    const rowRect = active.getBoundingClientRect();
    const listRect = list.getBoundingClientRect();
    return {
      label: active.textContent?.trim() ?? "",
      visible: rowRect.top >= listRect.top - 1 && rowRect.bottom <= listRect.bottom + 1
    };
  }, LIST);
}

for (const viewport of VIEWPORTS) {
  const size = `${viewport.width}×${viewport.height}`;

  test(`the Forward list is a bounded scroll box that reaches the last room (${size})`, async ({
    page
  }) => {
    await page.setViewportSize(viewport);
    await gotoReadyShell(page);
    await seedRooms(page);
    await openForwardList(page);

    const fresh = await listBox(page);
    // Bounded by the space the anchor leaves, not by the number of rooms, and
    // never past the window edge the report saw it cut at.
    expect(fresh.bottom, "the list runs past the bottom of the window").toBeLessThanOrEqual(
      viewport.height - 15
    );
    expect(fresh.top).toBeGreaterThanOrEqual(0);
    expect(fresh.overflowY, "the list does not scroll its own rows").toBe("auto");
    expect(
      fresh.scrollHeight,
      "the list grew to fit every room instead of scrolling"
    ).toBeGreaterThan(fresh.clientHeight);
    expect(await page.locator(ROW).count()).toBe(DESTINATIONS);

    // The wheel over the list now moves the list, which it did not before.
    await page.locator(ROW).first().hover();
    await page.mouse.wheel(0, 400);
    await expect.poll(async () => (await listBox(page)).scrollTop).toBeGreaterThan(0);

    const scrolled = await listBox(page);
    expect(scrolled.scrollTop).toBeLessThanOrEqual(scrolled.scrollHeight - scrolled.clientHeight);

    // Scrolled to the end, the last room is on screen.
    await page.locator(ROW).last().scrollIntoViewIfNeeded();
    await expect(page.locator(ROW).last()).toBeInViewport({ ratio: 1 });
    const last = await page.locator(ROW).last().boundingBox();
    const end = await listBox(page);
    expect(last!.y).toBeGreaterThanOrEqual(end.top);
    expect(last!.y + last!.height).toBeLessThanOrEqual(end.bottom + 1);

    // A click on the row still reaches it: the list is in the body-level layer,
    // so the row's outside-press dismissal has to recognise its own panel.
    await page.locator(ROW).last().click();
    await expect
      .poll(() => page.evaluate(() => window.__harness.invocationsOf("forward_message").length))
      .toBe(1);
    expect(
      await page.evaluate(() =>
        window.__harness
          .invocationsOf("forward_message")
          .map((invocation) => invocation.args?.destinationRoomId)
      )
    ).toEqual([`!forward-destination-${ROOM_COUNT - 1}:example.invalid`]);
  });

  test(`Arrow Down walks every room and keeps the focused row visible (${size})`, async ({
    page
  }) => {
    await page.setViewportSize(viewport);
    await gotoReadyShell(page);
    await seedRooms(page);
    await openForwardList(page);

    // Opening the submenu takes the first row, so the arrows have somewhere to start.
    const first = await focusedRowIsVisible(page);
    expect(first.visible).toBe(true);
    expect(first.label).toContain("Harness Room");

    // The end key reaches the last room in one press and brings it into view.
    await page.keyboard.press("End");
    const atEnd = await focusedRowIsVisible(page);
    expect(atEnd.label).toContain(`Forward destination ${String(ROOM_COUNT).padStart(2, "0")}`);
    expect(atEnd.visible, "the focused row is outside the visible part of the list").toBe(true);

    // Walking down one row at a time keeps it in view across the whole list.
    await page.keyboard.press("Home");
    expect((await focusedRowIsVisible(page)).visible).toBe(true);
    for (let index = 0; index < DESTINATIONS - 1; index += 1) {
      await page.keyboard.press("ArrowDown");
      const step = await focusedRowIsVisible(page);
      expect(step.visible, `Arrow Down ${index + 1} left the focused row out of view`).toBe(true);
    }
    const again = await focusedRowIsVisible(page);
    expect(again.label).toContain(`Forward destination ${String(ROOM_COUNT).padStart(2, "0")}`);
  });

  test(`Tab follows into rows the pointer cannot reach (${size})`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await gotoReadyShell(page);
    await seedRooms(page);
    await openForwardList(page);

    // The report's "twelve Tab presses and Enter": the visible band is a few
    // rows, so walking past it has to scroll the list or Enter acts blind.
    let presses = 0;
    while ((await listBox(page)).scrollTop === 0 && presses <= DESTINATIONS) {
      await page.keyboard.press("Tab");
      presses += 1;
      const step = await focusedRowIsVisible(page);
      expect(step.label, "focus left the destination list").toContain("Forward destination");
      expect(step.visible, `Tab ${presses} reached a row outside the visible part`).toBe(true);
    }
    expect(presses, "the whole list fit in the window, so this proves nothing").toBeGreaterThan(
      VISIBLE_ROWS_IN_REPORT
    );
    expect((await listBox(page)).scrollTop, "the list did not follow the focus").toBeGreaterThan(0);
  });

  test(`the keyboard alone forwards to the last room (${size})`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await gotoReadyShell(page);
    await seedRooms(page);
    await openForwardList(page);

    const lastRoomId = `!forward-destination-${ROOM_COUNT - 1}:example.invalid`;
    await page.keyboard.press("End");
    await page.keyboard.press("Enter");

    await expect
      .poll(() =>
        page.evaluate(() => window.__harness.invocationsOf("forward_message").length)
      )
      .toBe(1);
    const forwarded = await page.evaluate(() =>
      window.__harness
        .invocationsOf("forward_message")
        .map((invocation) => invocation.args?.destinationRoomId)
    );
    expect(forwarded).toEqual([lastRoomId]);
    // Sending closes the menu and returns focus to the row's own trigger.
    await expect(page.locator(LIST)).toHaveCount(0);
  });
}

test("a row the window shrank away from is brought into view, never forwarded", async ({
  page
}) => {
  await page.setViewportSize({ height: 800, width: 1280 });
  await gotoReadyShell(page);
  await seedRooms(page);
  await openForwardList(page);
  await page.keyboard.press("End");
  await expect.poll(() => focusedRowIsVisible(page)).toEqual({
    label: expect.stringContaining("Forward destination 42"),
    visible: true
  });

  // Shrinking the window clamps the list; the focused row can end up below the
  // new bottom edge without the focus itself moving (the report's danger).
  await page.setViewportSize({ height: 500, width: 1280 });
  await expect.poll(async () => (await focusedRowIsVisible(page)).visible).toBe(false);

  await page.keyboard.press("Enter");
  // Nothing was sent, and the row is now on screen to confirm.
  expect(await page.evaluate(() => window.__harness.invocationsOf("forward_message").length)).toBe(0);
  await expect.poll(() => focusedRowIsVisible(page)).toEqual({
    label: expect.stringContaining("Forward destination 42"),
    visible: true
  });

  // A second Enter acts on the row the user can now see.
  await page.keyboard.press("Enter");
  await expect
    .poll(() => page.evaluate(() => window.__harness.invocationsOf("forward_message").length))
    .toBe(1);
});

test("the list still closes and gives its focus back the way the menu does", async ({ page }) => {
  await page.setViewportSize({ height: 800, width: 1280 });
  await gotoReadyShell(page);
  await seedRooms(page);
  await openForwardList(page);

  // Escape in the list closes only the list and returns to the item that opened
  // it, so the menu behind it stays usable.
  await page.keyboard.press("Escape");
  await expect(page.locator(LIST)).toHaveCount(0);
  const forwardItem = page.getByRole("menuitem", { name: "Forward", exact: true });
  await expect(forwardItem).toBeVisible();
  await expect(forwardItem).toBeFocused();

  // A second Escape closes the whole menu.
  await page.keyboard.press("Escape");
  await expect(forwardItem).toHaveCount(0);
  await expect(page.getByRole("menuitem", { name: "Copy message" })).toHaveCount(0);

  // A press on the timeline closes the open list, because the list now lives in
  // the body-level layer rather than inside the row the press is measured against.
  const row = page.locator('[data-event-id="$seed-event:example.invalid"]');
  await row.getByRole("button", { name: "Message actions" }).click();
  await forwardItem.click();
  await expect(page.locator(ROW).first()).toBeVisible();
  await page.locator(".timeline-scroll").click({ position: { x: 40, y: 40 } });
  await expect(page.locator(LIST)).toHaveCount(0);
  await expect(forwardItem).toHaveCount(0);
});

/** Long enough that the row it targets sits near the bottom of the timeline. */
async function seedTallTimeline(page: Page, count: number): Promise<void> {
  const base = {
      sender: "@harness-user:example.invalid",
      sender_label: "Harness User",
      in_reply_to_event_id: null,
      thread_root: null,
      thread_summary: null,
      reactions: [],
      can_react: true,
      is_redacted: false,
      is_hidden: false,
      can_redact: true,
      is_edited: false,
      can_edit: true,
      actions: {
        can_copy: true,
        can_forward: true,
        can_reply: true,
        can_pin: true,
        can_permalink: true,
        can_view_source: true
      }
    };
  const items = Array.from({ length: count }, (_, index) => ({
    ...base,
    id: { Event: { event_id: `$filler-${index}:example.invalid` } },
    body: `Filler message ${index + 1}`,
    timestamp_ms: 1_800_000_100_000 + index * 60_000
  }));
  await seedTimelineItems(page, items);
}

test("a short list sits against its item instead of below the full cap", async ({ page }) => {
  // A destination list with a handful of rooms is shorter than the space the
  // layer reserves for it. It must still open against the item: the layer's
  // `top` assumes a box of the full cap, which is why the box is anchored by the
  // edge that must not move (pinned by the unit test for both sides).
  await page.setViewportSize({ height: 640, width: 1280 });
  await gotoReadyShell(page);
  await seedRooms(page, 4);
  await seedTallTimeline(page, 18);
  const lastRow = '[data-event-id="$filler-17:example.invalid"]';
  await expect(page.locator(lastRow)).toBeVisible();
  await openForwardList(page, "$filler-17:example.invalid");

  const geometry = await page.evaluate((listSelector) => {
    const list = document.querySelector<HTMLElement>(listSelector)!;
    const control = list
      .ownerDocument!.querySelector<HTMLElement>(".message-forward-menu-control")!;
    const listRect = list.getBoundingClientRect();
    const controlRect = control.getBoundingClientRect();
    return {
      controlBottom: controlRect.bottom,
      controlTop: controlRect.top,
      listBottom: listRect.bottom,
      listHeight: listRect.height,
      listTop: listRect.top
    };
  }, LIST);

  // Five rows do not fill the cap, so the box is shorter than the space reserved.
  expect(geometry.listHeight).toBeLessThan(300);
  // Its edge is the item's, on whichever side it opened.
  const gapBelow = geometry.listTop - geometry.controlBottom;
  const gapAbove = geometry.controlTop - geometry.listBottom;
  const gap = gapBelow > 0 ? gapBelow : gapAbove;
  expect(gap).toBeGreaterThanOrEqual(5);
  expect(gap).toBeLessThanOrEqual(7);
});

test("the message actions bar stays drawn while its portaled list is used", async ({ page }) => {
  await page.setViewportSize({ height: 800, width: 1280 });
  await gotoReadyShell(page);
  await seedRooms(page);
  await openForwardList(page);

  // `.message-actions` carries the opacity; the menu inside it inherits the
  // drawn result, so a passing assertion on the menu alone would prove nothing.
  const barOpacity = () =>
    page
      .locator(".message-actions")
      .first()
      .evaluate((element) => getComputedStyle(element).opacity);
  const transition = "opacity 120ms with a 180ms hover-intent delay";

  // Focus is in the list, so the row has neither hover nor focus-within.
  await page.mouse.move(0, 0);
  await expect
    .poll(barOpacity, { message: `the bar faded while its list held the focus (${transition})` })
    .toBe("1");

  // The pointer over the portaled list is not "hovering the row" either.
  await page.locator(ROW).first().hover();
  await expect
    .poll(barOpacity, { message: `the bar faded while the pointer was on its list (${transition})` })
    .toBe("1");
});

test("Shift+Tab from the first row goes back to the item that opened the list", async ({ page }) => {
  await page.setViewportSize({ height: 800, width: 1280 });
  await gotoReadyShell(page);
  await seedRooms(page);
  await openForwardList(page);

  await expect(page.locator(ROW).first()).toBeFocused();

  // Backwards from the first row leaves the list instead of Tab-walking the
  // whole app, and returns to the item the list was opened from.
  await page.keyboard.press("Shift+Tab");
  await expect(page.getByRole("menuitem", { name: "Forward", exact: true })).toBeFocused();
  await expect(page.locator(LIST)).toHaveCount(0);

  // A row in the middle walks backwards within the list.
  await page.keyboard.press("Enter");
  await expect(page.locator(ROW).first()).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Shift+Tab");
  await expect(page.locator(ROW).first()).toBeFocused();

  // Tab past the last row leaves the list instead of wrapping out of the
  // document with the panel still open behind it.
  await page.keyboard.press("End");
  await page.keyboard.press("Tab");
  await expect(page.locator(LIST)).toHaveCount(0);
  await expect(page.getByRole("menuitem", { name: "Forward", exact: true })).toBeFocused();
});

test("a row the list cut in half still takes the press", async ({ page }) => {
  // The guard refuses a row with no drawn part, not a row the list cut in half:
  // the reader pressed something they can see. Chromium scrolls the row into
  // view on mousedown and WebKit does not, so demanding a whole row would also
  // make the two engines disagree about the press.
  await page.setViewportSize({ height: 800, width: 1280 });
  await gotoReadyShell(page);
  await seedRooms(page);
  await openForwardList(page);

  const target = await page.evaluate((listSelector) => {
    const list = document.querySelector<HTMLElement>(listSelector)!;
    const rows = Array.from(list.querySelectorAll<HTMLElement>('[role="menuitem"]'));
    // Half a row down, so the row at the bottom edge is cut in half.
    const rowHeight = rows[0].getBoundingClientRect().height;
    list.scrollTop = rowHeight * 3 + rowHeight / 2;
    const listRect = list.getBoundingClientRect();
    const straddler = rows.find((row) => {
      const rect = row.getBoundingClientRect();
      return rect.top < listRect.bottom && rect.bottom > listRect.bottom;
    })!;
    const rect = straddler.getBoundingClientRect();
    const visibleTop = Math.max(rect.top, listRect.top);
    return {
      cut: rect.bottom > listRect.bottom + 5,
      label: straddler.textContent?.trim() ?? "",
      // The middle of the part of the row the list draws.
      x: rect.left + rect.width / 2,
      y: (visibleTop + listRect.bottom) / 2
    };
  }, LIST);
  expect(target.cut, "no row straddles the bottom edge of the list").toBe(true);

  await page.mouse.click(target.x, target.y);
  await expect
    .poll(() => page.evaluate(() => window.__harness.invocationsOf("forward_message").length))
    .toBe(1);
  const forwarded = await page.evaluate(() =>
    window.__harness
      .invocationsOf("forward_message")
      .map((invocation) => invocation.args?.destinationRoomId)
  );
  expect(forwarded).toHaveLength(1);
});
