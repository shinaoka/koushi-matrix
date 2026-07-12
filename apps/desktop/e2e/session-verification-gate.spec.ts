import { expect, test } from "@playwright/test";

const nonReady = [
  "provisional",
  "awaitingVerification",
  "verifying",
  "awaitingBootstrapConfirmation",
  "rejecting",
  "locked"
] as const;

test("verification states replace the complete desktop shell", async ({ page }) => {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();

  for (const kind of nonReady) {
    await page.evaluate((nextKind) => {
      const snapshot = window.__harness.currentSnapshot();
      window.__harness.setSnapshot({
        ...snapshot,
        state: {
          ...snapshot.state,
          domain: {
            ...snapshot.state.domain,
            session: {
              kind: nextKind,
              homeserver: "https://example.invalid",
              user_id: "@gate:example.invalid",
              device_id: "DEVICE",
              flow_id: 71,
              gate: {
                methods: ["existingDeviceSas", "recoveryKey", "bootstrap"],
                account_kind: "existingIdentity",
                failureKind: null
              },
              ...(nextKind === "awaitingBootstrapConfirmation"
                ? { destination_written: true }
                : {})
            }
          }
        }
      });
      window.__harness.pushStateChanged();
    }, kind);
    await expect(page.getByRole("main", { name: "Verify this session" })).toBeVisible();
    await expect(page.getByRole("main", { name: "Conversation timeline" })).toHaveCount(0);
    await expect(page.getByRole("textbox", { name: "Message composer" })).toHaveCount(0);
    await expect(page.getByText(/skip|verify later|send anyway/i)).toHaveCount(0);
  }
});
