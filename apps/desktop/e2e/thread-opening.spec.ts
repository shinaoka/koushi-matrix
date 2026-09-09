import { expect, test } from "@playwright/test";
import { t } from "../src/i18n/messages";
import { gotoReadyShell, invocationCount } from "./support/basicOperations";

for (const outcome of ["open", "closed"] as const) {
  test(`one pill click shows Opening before subscription becomes ${outcome}`, async ({ page }) => {
    await gotoReadyShell(page);
    await page.evaluate(() => {
      const snapshot = window.__harness.currentSnapshot();
      window.__harness.setCommandResponse("open_thread", {
        ...snapshot,
        state: { ...snapshot.state, ui: { ...snapshot.state.ui, thread: {
          kind: "opening", room_id: "!harness-room:example.invalid",
          root_event_id: "$seed-event:example.invalid", intent: "existingThread"
        } } }
      });
    });
    await page.getByRole("button", { name: /2 replies/ }).click();
    await expect.poll(() => invocationCount(page, "open_thread")).toBe(1);
    const panel = page.getByRole("complementary", { name: t("panel.context") });
    await expect(panel.getByText(t("panel.thread"), { exact: true })).toBeVisible();
    await expect(panel.getByRole("status")).toHaveText(t("timeline.openingThread"));
    await expect(panel.getByRole("textbox", { name: t("timeline.threadComposer") })).toHaveAttribute("contenteditable", "false");

    await page.evaluate((kind) => {
      const snapshot = window.__harness.currentSnapshot();
      window.__harness.setSnapshot({
        ...snapshot,
        state: { ...snapshot.state, ui: { ...snapshot.state.ui, thread: kind === "closed"
          ? { kind: "closed" }
          : {
              kind: "open", room_id: "!harness-room:example.invalid",
              root_event_id: "$seed-event:example.invalid", intent: "existingThread",
              is_subscribed: true, staged_uploads: [],
              composer: { ...snapshot.state.ui.timeline.composer }
            }
        } }
      });
      window.__harness.pushStateUpdate();
    }, outcome);
    if (outcome === "open") {
      await expect(panel.getByRole("status")).toHaveCount(0);
      await expect(panel.getByRole("textbox", { name: t("timeline.threadComposer") })).toHaveAttribute("contenteditable", "true");
    } else {
      await expect(panel.getByText(t("panel.thread"), { exact: true })).toHaveCount(0);
    }
    await expect.poll(() => invocationCount(page, "open_thread")).toBe(1);
  });
}
