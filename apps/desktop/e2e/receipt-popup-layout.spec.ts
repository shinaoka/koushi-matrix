import { expect, test } from "@playwright/test";
import { gotoReadyShell } from "./support/basicOperations";

for (const count of [1, 2]) {
  test(`reader popup fits ${count} timestamped rows without wrapping or scrolling`, async ({ page }) => {
    await gotoReadyShell(page);
    await page.evaluate((count) => {
      const snapshot = window.__harness.currentSnapshot();
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
    await page.locator('[data-event-id="$seed-event:example.invalid"] .message-receipts').hover();
    const popup = page.locator("body > .receipt-tooltip");
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
