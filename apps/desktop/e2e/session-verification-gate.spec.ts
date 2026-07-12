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

test("recovery and bootstrap actions preserve secrets outside observable state", async ({ page }) => {
  await page.goto("/appHarness.html");
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({ ...snapshot, state: { ...snapshot.state, domain: { ...snapshot.state.domain, session: { kind: "awaitingVerification", homeserver: "https://example.invalid", user_id: "@gate:example.invalid", device_id: "DEVICE", flow_id: 72, gate: { methods: ["recoveryKey", "bootstrap"], account_kind: "newIdentity", failureKind: null } } } } });
    window.__harness.pushStateChanged();
    window.__harness.clearInvocations();
  });
  const secret = "SYNTHETIC_RECOVERY_SECRET_8842";
  await page.getByLabel("Recovery secret").fill(secret);
  await page.getByRole("button", { name: "Recover", exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("submit_recovery")[0]?.args)).toEqual({ secret: "[REDACTED]" });
  await expect(page.locator("body")).not.toContainText(secret);
  expect(await page.evaluate((sentinel) => JSON.stringify(window.__harness.currentSnapshot()).includes(sentinel), secret)).toBe(false);

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({ ...snapshot, state: { ...snapshot.state, domain: { ...snapshot.state.domain, session: { ...snapshot.state.domain.session, kind: "awaitingVerification", gate: { methods: ["bootstrap"], account_kind: "newIdentity", failureKind: null }, flow_id: 73 } } } });
    window.__harness.pushStateChanged();
    window.__harness.clearInvocations();
  });
  const passphrase = "SYNTHETIC_PASSPHRASE_9911";
  const destination = "/synthetic/private/recovery-9911.txt";
  await page.getByLabel("Recovery key destination").fill(destination);
  await page.getByLabel("Backup passphrase").fill(passphrase);
  await page.getByRole("button", { name: "Create secure backup" }).click();
  const bootstrapArgs = await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("start_session_bootstrap")[0]?.args)).toBeTruthy().then(() => page.evaluate(() => window.__harness.invocationsOf("start_session_bootstrap")[0]!.args));
  expect(bootstrapArgs).toMatchObject({ passphrase: "[REDACTED]", recoveryKeyDestinationPath: "[REDACTED]" });
  expect([72, 73]).toContain(bootstrapArgs.flowId);
  await expect(page.getByRole("button", { name: "I saved the recovery key" })).toBeVisible();
  const observable = await page.evaluate(() => `${JSON.stringify(window.__harness.currentSnapshot())}\n${document.body.textContent ?? ""}`);
  expect(observable).not.toContain(passphrase);
  expect(observable).not.toContain(destination);
});

test("Ready to Locked replaces the shell with the gate", async ({ page }) => {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  await expect(page.getByRole("main", { name: "Verify this session" })).toHaveCount(0);
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({ ...snapshot, state: { ...snapshot.state, domain: { ...snapshot.state.domain, session: { kind: "locked", homeserver: "https://example.invalid", user_id: "@gate:example.invalid", device_id: "DEVICE" } } } });
    window.__harness.pushStateChanged();
  });
  await expect(page.getByRole("main", { name: "Verify this session" })).toBeVisible();
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toHaveCount(0);
  await expect(page.getByRole("textbox", { name: "Message composer" })).toHaveCount(0);
});
