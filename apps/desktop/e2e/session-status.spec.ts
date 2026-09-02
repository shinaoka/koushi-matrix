import { expect, test, type Page } from "@playwright/test";

async function gotoReadyShell(page: Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("button", { name: "Open session status" })).toBeVisible();
}

async function seedReadyStatus(page: Page, accountManagementUrl: string | null): Promise<void> {
  await page.evaluate((managementUrl) => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          auth: {
            kind: "ready",
            homeserver: snapshot.state.domain.session.homeserver ?? "",
            flows: [],
            delegated: { registration_url: null }
          },
          account_management_url: managementUrl,
          current_session_status: {
            status: "ready",
            request_id: 369,
            details: {
              device_display_name: "Harness Desktop",
              device_id: "HARNESSDEVICE",
              authentication_method: "oauth",
              sync_state: "running",
              is_cross_signed_by_owner: true,
              own_identity_verification: "verified",
              key_backup: "ready",
              verification: "verified",
              checked_at_ms: Date.UTC(2026, 6, 30, 12, 0, 0)
            }
          }
        }
      }
    });
    window.__harness.setCommandResponse("refresh_current_session_status", ({ trigger }) => {
      const current = window.__harness.currentSnapshot();
      const checking = {
        ...current,
        state: {
          ...current.state,
          domain: {
            ...current.state.domain,
            current_session_status: {
              status: "checking" as const,
              request_id: trigger === "open" ? 370 : 371,
              trigger,
              last_known_details:
                current.state.domain.current_session_status.status === "ready"
                  ? current.state.domain.current_session_status.details
                  : null
            }
          }
        }
      };
      window.__harness.setSnapshot(checking);
      return checking;
    });
    window.__harness.pushStateUpdate();
  }, accountManagementUrl);
}

test("session popover preserves stale facts and reflects core-owned network recovery", async ({
  page
}) => {
  await gotoReadyShell(page);
  await seedReadyStatus(page, "https://account.example.invalid/manage");
  await page.evaluate(() => window.__harness.clearInvocations());

  const trigger = page.getByRole("button", { name: "Open session status" });
  await trigger.click();
  const dialog = page.getByRole("dialog", { name: "Current session" });
  await expect(dialog).toBeVisible();
  await expect(dialog).toBeFocused();
  await expect(dialog).toContainText("Harness Desktop");
  await expect
    .poll(() =>
      page.evaluate(
        () => window.__harness.invocationsOf("refresh_current_session_status")[0]?.args
      )
    )
    .toEqual({ trigger: "open" });

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          current_session_status: {
            status: "ready",
            request_id: 370,
            details: {
              device_display_name: "Harness Desktop",
              device_id: "HARNESSDEVICE",
              authentication_method: "oauth",
              sync_state: "running",
              is_cross_signed_by_owner: true,
              own_identity_verification: "verified",
              key_backup: "ready",
              verification: "verified",
              checked_at_ms: Date.UTC(2026, 6, 30, 12, 0, 0)
            }
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
  });
  await expect(dialog).toContainText("Harness Desktop");
  await expect(dialog).toContainText("HARNESSDEVICE");
  await expect(dialog).toContainText("OAuth");
  await expect(dialog).toContainText("Cross-signed");
  await expect(dialog).toContainText("Identity verified");

  await page.evaluate(() => {
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText(value: string) {
          (window as unknown as { __copiedSessionDeviceId: string }).__copiedSessionDeviceId =
            value;
          return Promise.resolve();
        }
      }
    });
    window.__harness.setCommandResponse("plugin:opener|open_url", null);
  });
  await dialog.getByRole("button", { name: "Copy Device ID" }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as { __copiedSessionDeviceId?: string }).__copiedSessionDeviceId
      )
    )
    .toBe("HARNESSDEVICE");
  await dialog.getByRole("button", { name: "Manage account and devices" }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () => window.__harness.invocationsOf("plugin:opener|open_url").at(-1)?.args.url
      )
    )
    .toBe("https://account.example.invalid/manage");

  await dialog.getByRole("button", { name: "Recheck" }).click();
  await expect(dialog.getByRole("button", { name: "Checking" })).toBeDisabled();
  await expect
    .poll(() =>
      page.evaluate(
        () => window.__harness.invocationsOf("refresh_current_session_status").at(-1)?.args
      )
    )
    .toEqual({ trigger: "manual" });

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          current_session_status: {
            status: "failed",
            request_id: 371,
            kind: "timed_out",
            checked_at_ms: Date.UTC(2026, 6, 30, 12, 1, 0),
            last_known_details: {
              device_display_name: "Harness Desktop",
              device_id: "HARNESSDEVICE",
              authentication_method: "oauth",
              sync_state: "running",
              is_cross_signed_by_owner: true,
              own_identity_verification: "verified",
              key_backup: "ready",
              verification: "verified",
              checked_at_ms: Date.UTC(2026, 6, 30, 12, 0, 0)
            }
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
  });
  await expect(dialog).toContainText(
    "Could not check this session before the connection timed out"
  );
  await expect(dialog).toContainText("Harness Desktop");

  const refreshCountBeforeRecovery = await page.evaluate(
    () => window.__harness.invocationsOf("refresh_current_session_status").length
  );
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    const status = snapshot.state.domain.current_session_status;
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          current_session_status: {
            status: "checking",
            request_id: 372,
            trigger: "recovery",
            last_known_details: status.status === "failed" ? status.last_known_details : null
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
  });
  await expect(dialog.getByRole("button", { name: "Checking" })).toBeDisabled();
  await expect(dialog).toContainText("Harness Desktop");
  expect(
    await page.evaluate(
      () => window.__harness.invocationsOf("refresh_current_session_status").length
    )
  ).toBe(refreshCountBeforeRecovery);

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    const status = snapshot.state.domain.current_session_status;
    if (status.status !== "checking" || !status.last_known_details) {
      throw new Error("expected recovery checking with retained details");
    }
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          current_session_status: {
            status: "ready",
            request_id: 372,
            details: {
              ...status.last_known_details,
              checked_at_ms: Date.UTC(2026, 6, 30, 12, 1, 2)
            }
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
  });
  await expect(dialog.getByRole("button", { name: "Recheck" })).toBeEnabled();
  await expect(dialog).toContainText("Harness Desktop");

  await dialog.getByRole("button", { name: "Open diagnostics" }).click();
  await expect
    .poll(() => page.evaluate(() => window.__harness.invocationsOf("get_diagnostic_snapshot").length))
    .toBe(1);
});

test("session popover dismisses accessibly and hides an unsafe account destination", async ({
  page
}) => {
  await gotoReadyShell(page);
  await seedReadyStatus(page, "javascript:alert(1)");

  const trigger = page.getByRole("button", { name: "Open session status" });
  await trigger.focus();
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog", { name: "Current session" });
  await expect(dialog.getByRole("button", { name: "Manage account and devices" })).toHaveCount(0);

  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(trigger).toBeFocused();

  await trigger.click();
  await page.locator(".top-search").click();
  await expect(dialog).toBeHidden();
});

test("session popover has an opaque theme-aware surface and elevation", async ({ page }) => {
  await gotoReadyShell(page);
  await seedReadyStatus(page, null);

  const trigger = page.getByRole("button", { name: "Open session status" });
  await trigger.click();
  const dialog = page.getByRole("dialog", { name: "Current session" });
  await expect(dialog).toBeVisible();

  for (const theme of ["light", "dark"] as const) {
    await page.evaluate((nextTheme) => {
      document.documentElement.dataset.theme = nextTheme;
    }, theme);

    const styles = await dialog.evaluate((element) => {
      const computed = getComputedStyle(element);
      return {
        backgroundColor: computed.backgroundColor,
        boxShadow: computed.boxShadow
      };
    });

    expect(styles.backgroundColor, `${theme} popover should be opaque`).not.toMatch(
      /transparent|rgba\([^)]*,\s*0\)/
    );
    expect(styles.boxShadow, `${theme} popover should have elevation`).not.toBe("none");
  }
});
