import { expect, test, type Page } from "@playwright/test";

async function gotoSessions(page: Page): Promise<void> {
  await page.goto("/appHarness.html");
  const settingsButton = page.getByRole("button", { name: "Account Settings", exact: true });
  await expect(settingsButton).toBeVisible();
  await page.evaluate(() => {
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
          sync: "running",
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
  });
  await settingsButton.click();
  const dialog = page.getByRole("dialog", { name: "Account Settings", exact: true });
  await dialog.getByRole("tab", { name: "Sessions", exact: true }).click();
  await expect(dialog.locator("#settings-session")).toBeVisible();
}

test("session health and sync recovery live in Account Settings", async ({ page }) => {
  await gotoSessions(page);
  const dialog = page.getByRole("dialog", { name: "Account Settings", exact: true });
  await expect(dialog).toContainText("Harness Desktop");
  await expect(dialog).toContainText("HARNESSDEVICE");
  await expect(dialog).toContainText("OAuth");
  await expect(dialog).toContainText("Cross-signed");
  await expect(dialog).toContainText("Identity verified");

  await page.evaluate(() => window.__harness.clearInvocations());
  await dialog.getByRole("button", { name: "Recheck", exact: true }).click();
  await expect(dialog.getByRole("button", { name: "Checking", exact: true })).toBeDisabled();
  await expect
    .poll(() => page.evaluate(
      () => window.__harness.invocationsOf("refresh_current_session_status").at(-1)?.args
    ))
    .toEqual({ trigger: "manual" });

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          sync: { failed: "transport error" },
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
  await expect(dialog.getByRole("button", { name: "Restart sync", exact: true })).toBeVisible();
  await dialog.getByRole("button", { name: "Restart sync", exact: true }).click();
  await expect.poll(() => page.evaluate(
    () => window.__harness.invocationsOf("restart_sync").length
  )).toBe(1);
});

test("Account Settings closes accessibly and returns focus", async ({ page }) => {
  await gotoSessions(page);
  const dialog = page.getByRole("dialog", { name: "Account Settings", exact: true });
  const settingsButton = page.getByRole("button", { name: "Account Settings", exact: true });

  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(settingsButton).toBeFocused();
});
