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
    window.__harness.setSnapshot({ ...snapshot, state: { ...snapshot.state, domain: { ...snapshot.state.domain, session: { kind: "awaitingVerification", homeserver: "https://example.invalid", user_id: "@gate:example.invalid", device_id: "DEVICE", gate: { methods: ["recoveryKey", "bootstrap"], account_kind: "newIdentity", failureKind: null } } } } });
    window.__harness.pushStateChanged();
    window.__harness.clearInvocations();
  });
  const secret = "SYNTHETIC_RECOVERY_SECRET_8842";
  await page.getByLabel("Recovery secret").fill(secret);
  await page.getByRole("button", { name: "Recover", exact: true }).click();
  await expect(page.getByLabel("Recovery secret")).toHaveValue("");
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("submit_recovery")[0]?.args)).toEqual({ secret: "[REDACTED]" });
  await expect(page.locator("body")).not.toContainText(secret);
  expect(await page.evaluate((sentinel) => JSON.stringify(window.__harness.currentSnapshot()).includes(sentinel), secret)).toBe(false);

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({ ...snapshot, state: { ...snapshot.state, domain: { ...snapshot.state.domain, session: { ...snapshot.state.domain.session, kind: "awaitingVerification", gate: { methods: ["bootstrap"], account_kind: "newIdentity", failureKind: null }, flow_id: undefined } } } });
    window.__harness.pushStateChanged();
    window.__harness.clearInvocations();
  });
  const passphrase = "SYNTHETIC_PASSPHRASE_9911";
  const destination = "/synthetic/private/recovery-9911.txt";
  await page.getByLabel("Recovery key destination").fill(destination);
  await page.getByLabel("Backup passphrase").fill(passphrase);
  await page.getByRole("button", { name: "Create secure backup" }).click();
  await expect(page.getByLabel("Recovery key destination")).toHaveValue("");
  await expect(page.getByLabel("Backup passphrase")).toHaveValue("");
  const bootstrapArgs = await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("start_session_bootstrap")[0]?.args)).toBeTruthy().then(() => page.evaluate(() => window.__harness.invocationsOf("start_session_bootstrap")[0]!.args));
  expect(bootstrapArgs).toMatchObject({ passphrase: "[REDACTED]", recoveryKeyDestinationPath: "[REDACTED]" });
  expect(bootstrapArgs.flowId).toBeUndefined();
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
    window.__harness.clearInvocations();
    window.__harness.setSnapshot({ ...snapshot, state: { ...snapshot.state, domain: { ...snapshot.state.domain, native_attention: { summary: { unread_count: 1, highlight_count: 1, badge_count: 1, candidate: { room_display_name: "Attention", kind: "mention", unread_count: 1, highlight_count: 1 }, capabilities: { notifications: "available", badge: "available", overlay_icon: "unavailable", sound: "available", tray: "available", activation: "available" } }, dispatch: { kind: "idle" } } } } });
    window.__harness.pushStateChanged();
  });
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("play_native_attention_sound").length)).toBeGreaterThanOrEqual(1);
  const attentionCount = await page.evaluate(() => window.__harness.invocationsOf("play_native_attention_sound").length);
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({ ...snapshot, state: { ...snapshot.state, domain: { ...snapshot.state.domain, session: { kind: "locked", homeserver: "https://example.invalid", user_id: "@gate:example.invalid", device_id: "DEVICE" } } } });
    window.__harness.pushStateChanged();
  });
  await expect(page.getByRole("main", { name: "Verify this session" })).toBeVisible();
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toHaveCount(0);
  await expect(page.getByRole("textbox", { name: "Message composer" })).toHaveCount(0);
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({ ...snapshot, state: { ...snapshot.state, domain: { ...snapshot.state.domain, native_attention: { ...snapshot.state.domain.native_attention, summary: { ...snapshot.state.domain.native_attention.summary, unread_count: 2, highlight_count: 2, candidate: { room_display_name: "Locked attention", kind: "mention", unread_count: 2, highlight_count: 2 } }, dispatch: { kind: "idle" } } } } });
    window.__harness.pushStateChanged();
  });
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("play_native_attention_sound").length)).toBe(attentionCount);
});

test("SAS actions stay flow-correlated through retry and cancellation", async ({ page }) => {
  await page.goto("/appHarness.html");
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({ ...snapshot, state: { ...snapshot.state, domain: { ...snapshot.state.domain, session: { kind: "awaitingVerification", homeserver: "https://example.invalid", user_id: "@gate:example.invalid", device_id: "DEVICE", gate: { methods: ["existingDeviceSas"], account_kind: "existingIdentity", failureKind: null } } } } });
    window.__harness.pushStateChanged();
    window.__harness.clearInvocations();
  });
  await page.getByRole("button", { name: "Verify with another device" }).click();
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("start_own_user_sas")[0]?.args)).toEqual({});
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    const emojis = ["🐶", "🐱", "🦁", "🐎", "🦄", "🐷", "🐘"].map((symbol, index) => ({ symbol, description: `emoji-${index}` }));
    const flowId = snapshot.state.domain.session.flow_id!;
    window.__harness.setSnapshot({ ...snapshot, state: { ...snapshot.state, domain: { ...snapshot.state.domain, e2ee_trust: { ...snapshot.state.domain.e2ee_trust, verification: { kind: "sasPresented", request_id: flowId, target: { user_id: "opaque-current-user", device_id: "opaque-device" }, emojis } } } } });
    window.__harness.pushStateChanged();
  });
  await expect(page.locator(".session-verification-emojis span")).toHaveCount(7);
  await page.getByRole("button", { name: "They match" }).click();
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("confirm_sas_verification")[0]?.args.flowId)).toBeGreaterThan(0);

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    const emojis = ["🐶", "🐱", "🦁", "🐎", "🦄", "🐷", "🐘"].map((symbol, index) => ({ symbol, description: `emoji-${index}` }));
    window.__harness.setSnapshot({ ...snapshot, state: { ...snapshot.state, domain: { ...snapshot.state.domain, session: { ...snapshot.state.domain.session, kind: "verifying", flow_id: 81, method: "existingDeviceSas" }, e2ee_trust: { ...snapshot.state.domain.e2ee_trust, verification: { kind: "sasPresented", request_id: 81, target: { user_id: "opaque-current-user", device_id: "opaque-device" }, emojis } } } } });
    window.__harness.pushStateChanged();
  });
  await page.getByRole("button", { name: "They do not match" }).click();
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("mismatch_sas_verification")[0]?.args)).toEqual({ flowId: 81 });
  await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();
  await page.getByRole("button", { name: "Retry" }).click();
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("retry_current_device_trust_discovery").length)).toBe(1);

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({ ...snapshot, state: { ...snapshot.state, domain: { ...snapshot.state.domain, session: { ...snapshot.state.domain.session, kind: "verifying", flow_id: 82, method: "existingDeviceSas", gate: { methods: ["existingDeviceSas"], account_kind: "existingIdentity", failureKind: null } } } } });
    window.__harness.pushStateChanged();
  });
  await page.getByRole("button", { name: "Cancel" }).click();
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("cancel_verification").at(-1)?.args)).toEqual({ flowId: 82 });
  const recorder = await page.evaluate(() => JSON.stringify(window.__harness.invocationsOf("start_own_user_sas").concat(window.__harness.invocationsOf("confirm_sas_verification"), window.__harness.invocationsOf("mismatch_sas_verification"), window.__harness.invocationsOf("cancel_verification"))));
  expect(recorder).not.toMatch(/secret|passphrase|destination/i);
});

test("saved confirmation and sign out use matching gate commands", async ({ page }) => {
  await page.goto("/appHarness.html");
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({ ...snapshot, state: { ...snapshot.state, domain: { ...snapshot.state.domain, session: { kind: "awaitingBootstrapConfirmation", homeserver: "https://example.invalid", user_id: "@gate:example.invalid", device_id: "DEVICE", flow_id: 91, destination_written: true, gate: { methods: ["bootstrap"], account_kind: "newIdentity", failureKind: null } } } } });
    window.__harness.pushStateChanged();
    window.__harness.clearInvocations();
  });
  await page.getByRole("button", { name: "I saved the recovery key" }).click();
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("confirm_session_bootstrap_saved")[0]?.args)).toEqual({ flowId: 91 });
  await expect(page.getByText("Checking device trust…")).toBeVisible();
  await page.getByRole("button", { name: "Sign out" }).click();
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("logout").length)).toBe(1);
  await expect(page.getByTestId("auth-screen")).toBeVisible();
  await expect(page.getByRole("main", { name: "Verify this session" })).toHaveCount(0);
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toHaveCount(0);
});

test("start retries allocate distinct opaque flows and stale terminals are ignored", async ({ page }) => {
  await page.goto("/appHarness.html");
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({ ...snapshot, state: { ...snapshot.state, domain: { ...snapshot.state.domain, session: { kind: "awaitingVerification", homeserver: "https://example.invalid", user_id: "@gate:example.invalid", device_id: "DEVICE", gate: { methods: ["existingDeviceSas"], account_kind: "existingIdentity", failureKind: null } } } } });
    window.__harness.pushStateChanged();
  });
  await page.getByRole("button", { name: "Verify with another device" }).click();
  const first = await expect.poll(() => page.evaluate(() => window.__harness.currentSnapshot().state.domain.session.flow_id)).toBeTruthy().then(() => page.evaluate(() => window.__harness.currentSnapshot().state.domain.session.flow_id!));
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({ ...snapshot, state: { ...snapshot.state, domain: { ...snapshot.state.domain, session: { ...snapshot.state.domain.session, kind: "awaitingVerification", flow_id: undefined } } } });
    window.__harness.pushStateChanged();
  });
  await page.getByRole("button", { name: "Verify with another device" }).click();
  const second = await expect.poll(() => page.evaluate(() => window.__harness.currentSnapshot().state.domain.session.flow_id)).not.toBe(first).then(() => page.evaluate(() => window.__harness.currentSnapshot().state.domain.session.flow_id!));
  expect(second).not.toBe(first);
  const beforeStale = await page.evaluate(() => JSON.stringify(window.__harness.currentSnapshot().state.domain.session));
  await page.evaluate((flowId) => window.__harness.invoke("mismatch_sas_verification", { flowId }), first);
  expect(await page.evaluate(() => JSON.stringify(window.__harness.currentSnapshot().state.domain.session))).toBe(beforeStale);
  await page.evaluate((flowId) => window.__harness.invoke("mismatch_sas_verification", { flowId }), second);
  await expect.poll(() => page.evaluate(() => window.__harness.currentSnapshot().state.domain.session.kind)).toBe("awaitingVerification");
});
