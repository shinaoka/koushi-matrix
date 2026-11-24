import { expect, test, type Page } from "@playwright/test";
import { gotoReadyShell } from "./support/basicOperations";

const RECEIPTS_ANCHOR = '[data-event-id="$seed-event:example.invalid"] .message-receipts';
const POPUP = "body > .receipt-tooltip";

for (const count of [1, 2]) {
  test(`reader popup fits ${count} timestamped rows without wrapping or scrolling`, async ({ page }) => {
    await gotoReadyShell(page);
    await page.evaluate((count) => {
      // A Rust snapshot is always a fresh object, never the live one edited in place.
      const snapshot = structuredClone(window.__harness.currentSnapshot());
      const room = "!harness-room:example.invalid";
      snapshot.state.domain.live_signals.rooms[room] = {
        fully_read_event_id: null, typing_user_ids: [], typing_users: [],
        receipts_by_event: { "$seed-event:example.invalid": {
          total_count: count, overflow_count: 0,
          readers: Array.from({ length: count }, (_, index) => ({
            user_id: `@reader${index}:example.invalid`,
            display_name: "Synthetic Reader With A Long Display Name",
            original_display_label: "Synthetic Reader With A Long Display Name",
            avatar: null, timestamp_ms: 1_800_000_000_000
          }))
        } }
      };
      window.__harness.setSnapshot(snapshot);
      window.__harness.pushStateUpdate();
    }, count);
    await page.locator(RECEIPTS_ANCHOR).hover();
    const popup = page.locator(POPUP);
    await expect(popup).toBeVisible();
    await expect(popup.getByRole("listitem")).toHaveCount(count);
    const metrics = await popup.evaluate((element) => ({
      client: element.clientHeight, scroll: element.scrollHeight,
      rows: [...element.querySelectorAll('[role="listitem"]')].map(row => ({
        height: row.getBoundingClientRect().height,
        bottom: row.getBoundingClientRect().bottom,
        popupBottom: element.getBoundingClientRect().bottom
      }))
    }));
    expect(metrics.scroll).toBeLessThanOrEqual(metrics.client + 1);
    for (const row of metrics.rows) {
      expect(row.height).toBeLessThanOrEqual(20);
      expect(row.bottom).toBeLessThan(metrics.rows[0].popupBottom);
    }
    await expect(popup.locator(".receipt-reader-time").first()).toBeVisible();
    await page.screenshot({ path: test.info().outputPath("reader-popup.png") });
  });
}

/** Publishes one synthetic receipt event whose readers are `names`. */
async function seedReceipts(page: Page, names: string[]) {
  await page.evaluate((names) => {
    const snapshot = structuredClone(window.__harness.currentSnapshot());
    const room = "!harness-room:example.invalid";
    snapshot.state.domain.live_signals.rooms[room] = {
      fully_read_event_id: null, typing_user_ids: [], typing_users: [],
      receipts_by_event: { "$seed-event:example.invalid": {
        total_count: names.length, overflow_count: 0,
        readers: names.map((display_name, index) => ({
          user_id: `@reader${index}:example.invalid`,
          display_name, original_display_label: display_name,
          avatar: null, timestamp_ms: 1_800_000_000_000
        }))
      } }
    };
    window.__harness.setSnapshot(snapshot);
    window.__harness.pushStateUpdate();
  }, names);
}

/** Opens the reader popup for one synthetic receipt event with the given names. */
async function openReaderPopup(page: Page, names: string[]) {
  await gotoReadyShell(page);
  await seedReceipts(page, names);
  await page.locator(RECEIPTS_ANCHOR).hover();
  const popup = page.locator(POPUP);
  await expect(popup).toBeVisible();
  await expect(popup.getByRole("listitem")).toHaveCount(names.length);
  return popup;
}

/**
 * #2098: the popup's width must be its content's width, not a pinned 420px.
 *
 * Every number here is font-independent: the name's own text width comes from a
 * Range over its text node, so no expectation depends on which fonts the runner
 * has. `trailing` is the empty space between the timestamp and the row's end —
 * the gap the report showed as a wide blank strip.
 */
async function measureReaderPopup(page: Page) {
  return page.locator(POPUP).evaluate((element) => {
    const style = getComputedStyle(element);
    const popup = element.getBoundingClientRect();
    const chrome =
      parseFloat(style.paddingInlineStart) +
      parseFloat(style.paddingInlineEnd) +
      parseFloat(style.borderInlineStartWidth) +
      parseFloat(style.borderInlineEndWidth);
    const rows = [...element.querySelectorAll<HTMLElement>('[role="listitem"]')].map((row) => {
      const name = row.querySelector<HTMLElement>(".receipt-reader-name")!;
      const time = row.querySelector<HTMLElement>(".receipt-reader-time");
      const nameRect = name.getBoundingClientRect();
      const timeRect = time?.getBoundingClientRect() ?? null;
      const text = document.createRange();
      text.selectNodeContents(name);
      const rowRect = row.getBoundingClientRect();
      const rowStyle = getComputedStyle(row);
      return {
        nameBox: nameRect.width,
        nameText: text.getBoundingClientRect().width,
        nameContent: name.scrollWidth,
        // Everything the row occupies from its first child (the avatar) to the
        // end of the timestamp. A row that hugs its content equals the popup's
        // content box.
        occupied: timeRect ? timeRect.right - rowRect.left : null,
        gap: timeRect ? timeRect.left - nameRect.right : null,
        trailing: timeRect ? rowRect.right - timeRect.right : null,
        rowHeight: rowRect.height,
        rowGap: parseFloat(rowStyle.columnGap)
      };
    });
    return {
      width: popup.width,
      height: popup.height,
      client: element.clientHeight,
      scroll: element.scrollHeight,
      chrome,
      maxInlineSize: parseFloat(style.maxInlineSize),
      rows
    };
  });
}

test("a short reader name and its timestamp hug the popup instead of filling it", async ({
  page
}) => {
  const popup = await openReaderPopup(page, ["Ann"]);
  const metrics = await measureReaderPopup(page);
  const [row] = metrics.rows;

  // The width is the content's width to the pixel: padding, the avatar, the
  // name's own text, the fixed gap, the timestamp, padding and border.
  expect(Math.abs(metrics.width - (metrics.chrome + row.occupied!))).toBeLessThanOrEqual(1);
  // The name is its own text, not the box it sits in.
  expect(Math.abs(row.nameBox - row.nameText)).toBeLessThanOrEqual(1);

  // The timestamp follows the name at the row's gap; nothing is pushed right.
  expect(row.gap).not.toBeNull();
  expect(Math.abs(row.gap! - row.rowGap)).toBeLessThanOrEqual(1);
  expect(row.trailing).not.toBeNull();
  expect(row.trailing!).toBeLessThanOrEqual(1);
});

test("a long reader name truncates instead of widening the popup", async ({ page }) => {
  const popup = await openReaderPopup(page, [
    "A Reader Whose Display Name Is Deliberately Long Enough To Overflow Any Reasonable Popup Width"
  ]);
  const metrics = await measureReaderPopup(page);
  const [row] = metrics.rows;

  // The cap applies to the popup, and the name gives way instead of the popup.
  expect(metrics.maxInlineSize).toBeGreaterThan(0);
  expect(metrics.maxInlineSize).toBeLessThan(420);
  expect(metrics.width).toBeLessThanOrEqual(metrics.maxInlineSize + 1);
  expect(row.nameContent).toBeGreaterThan(row.nameBox);
  // #360: one line, so the height stays proportional to the reader count.
  expect(row.rowHeight).toBeLessThanOrEqual(20);
  expect(metrics.height).toBeLessThanOrEqual(60);
  // The timestamp is still right beside the truncated name.
  expect(Math.abs(row.gap! - row.rowGap)).toBeLessThanOrEqual(1);
  expect(row.trailing!).toBeLessThanOrEqual(1);
});

test("the popup width follows its widest reader row", async ({ page }) => {
  const popup = await openReaderPopup(page, ["Ann", "Bartholomew Example"]);
  const metrics = await measureReaderPopup(page);
  const [short, wide] = metrics.rows;

  // Each row keeps its own natural width; the popup takes the widest of them.
  const widest = Math.max(...metrics.rows.map((row) => row.occupied!));
  expect(Math.abs(metrics.width - (metrics.chrome + widest))).toBeLessThanOrEqual(1);
  expect(short.nameBox).toBeLessThan(wide.nameBox);
  // Both rows are one line and the popup does not scroll.
  for (const row of metrics.rows) expect(row.rowHeight).toBeLessThanOrEqual(20);
  expect(metrics.scroll).toBeLessThanOrEqual(metrics.client + 1);
});

test("the popup grows with the rows that arrive after it opens", async ({ page }) => {
  // The compact publication carries at most three readers; the reader window
  // that follows carries the rest. A box pinned from the first measurement
  // would keep the one-row height and scroll the rows that arrived.
  const names = Array.from({ length: 12 }, (_, index) => `Reader ${index}`);
  const popup = await openReaderPopup(page, [names[0]!]);
  const before = await measureReaderPopup(page);

  await seedReceipts(page, names);
  await expect(popup.getByRole("listitem")).toHaveCount(names.length);
  const after = await measureReaderPopup(page);

  expect(after.height).toBeGreaterThan(before.height + 100);
  // Twelve 20px rows, eleven 3px gaps, 8px padding and a 1px border on each side.
  expect(Math.abs(after.height - (12 * 20 + 11 * 3 + 2 * (8 + 1)))).toBeLessThanOrEqual(1);
  expect(after.scroll).toBeLessThanOrEqual(after.client + 1);
  for (const row of after.rows) expect(row.rowHeight).toBeLessThanOrEqual(20);
});
