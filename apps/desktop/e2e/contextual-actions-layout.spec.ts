import { expect, test, type Locator } from "@playwright/test";
import { t } from "../src/i18n/messages";
import { gotoReadyShell, HARNESS_ROOM_ID } from "./support/basicOperations";

async function expectActionFits(button: Locator) {
  await expect(button).toBeVisible();
  const geometry = await button.evaluate((element) => {
    const box = element.getBoundingClientRect();
    const parent = element.parentElement!.getBoundingClientRect();
    return {
      overflow: element.scrollWidth - element.clientWidth,
      left: box.left - parent.left,
      right: parent.right - box.right,
      top: box.top,
      bottom: window.innerHeight - box.bottom
    };
  });
  expect(geometry.overflow).toBeLessThanOrEqual(1);
  for (const distance of [geometry.left, geometry.right, geometry.top, geometry.bottom]) {
    expect(distance).toBeGreaterThanOrEqual(-1);
  }
}

for (const locale of ["en", "ja"] as const) {
  test(`creation and scheduled-edit actions fit in ${locale}`, async ({ page }) => {
    await page.setViewportSize({ width: 900, height: 800 });
    await gotoReadyShell(page);
    await page.evaluate((locale) => {
      const snapshot = structuredClone(window.__harness.currentSnapshot());
      snapshot.state.domain.locale_profile = {
        ...snapshot.state.domain.locale_profile, lang: locale, catalog_locale: locale
      };
      window.__harness.setSnapshot(snapshot);
      window.__harness.pushStateUpdate();
    }, locale);

    for (const kind of ["room", "space"] as const) {
      const launchKey = kind === "room" ? "action.createRoom" : "action.createSpace";
      await page.getByRole("button", { name: t(launchKey, {}, locale), exact: true }).click();
      const submitKey = kind === "room" ? "dialog.submitCreateRoom" : "dialog.submitCreateSpace";
      const submit = page.getByRole("button", { name: t(submitKey, {}, locale) });
      await expect(submit).toHaveText(t("action.create", {}, locale));
      await expectActionFits(submit);
      await page.getByRole("button", { name: t("dialog.cancelCreate", {}, locale) }).click();
    }

    await page.evaluate((roomId) => {
      const snapshot = structuredClone(window.__harness.currentSnapshot());
      snapshot.state.ui.timeline.scheduled_send_capability = "localFallback";
      snapshot.state.ui.timeline.scheduled_sends = [{
        scheduled_id: "synthetic-layout", room_id: roomId, body: "Synthetic scheduled message",
        send_at_ms: new Date("2030-01-02T03:04:00").getTime(), handle: { kind: "local" }
      }];
      window.__harness.setSnapshot(snapshot);
      window.__harness.pushStateUpdate();
    }, HARNESS_ROOM_ID);
    await page.getByRole("button", { name: t("scheduled.edit", {}, locale) }).click();
    const save = page.getByRole("button", { name: t("scheduled.save", {}, locale) });
    await expect(save).toHaveText(t("settings.propertySave", {}, locale));
    await expectActionFits(save);
  });
}
