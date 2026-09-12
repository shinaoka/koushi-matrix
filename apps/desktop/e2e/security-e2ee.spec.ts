import { expect, test } from "@playwright/test";

import { t } from "../src/i18n/messages";
import { HARNESS_ROOM_ID, gotoReadyShell, invocationCount } from "./support/basicOperations";

test("eligible unverified peer devices do not gate ordinary sends", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          e2ee_trust: {
            ...snapshot.state.domain.e2ee_trust,
            verification: { kind: "idle" },
            devices: [
              {
                user_id: "@peer:example.invalid",
                device_id: "PEERDEVICE",
                trust_level: "unverified"
              }
            ]
          }
        }
      }
    });
    window.__harness.clearInvocations();
  });

  const composer = page.getByRole("textbox", { name: "Message composer" });
  await composer.fill("peer trust remains non-blocking");
  await page.getByRole("button", { name: "Send", exact: true }).click();

  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("send_text")[0]?.args))
    .toMatchObject({
      roomId: HARNESS_ROOM_ID,
      document: {
        version: 2,
        inlines: [{ kind: "text", text: "peer trust remains non-blocking" }]
      }
    });
  await expect(page.locator(".trust-verification-dialog")).toHaveCount(0);
  await expect(page.getByText(/send anyway/i)).toHaveCount(0);
});

test("Security settings render local encryption health and dispatch probe commands", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    window.__harness.setCommandResponse("probe_local_encryption_health", () => {
      const snapshot = window.__harness.currentSnapshot();
      const next = {
        ...snapshot,
        state: {
          ...snapshot.state,
          domain: {
            ...snapshot.state.domain,
            local_encryption: { kind: "healthy" as const }
          }
        }
      };
      window.__harness.setSnapshot(next);
      return next;
    });

    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          locale_profile: { ...snapshot.state.domain.locale_profile, platform: "linux" },
          typography_profile: { ...snapshot.state.domain.typography_profile, platform: "linux" },
          local_encryption: { kind: "healthy" }
        }
      }
    });
    window.__harness.pushStateUpdate();
    window.__harness.clearInvocations();
  });

  await page.getByRole("button", { name: "User settings" }).click();
  await page.getByRole("tab", { name: "Encryption", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Security" })).toBeVisible();
  await expect(page.getByText("Secret Service")).toBeVisible();
  await expect(page.getByText("Protected")).toBeVisible();

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          locale_profile: { ...snapshot.state.domain.locale_profile, platform: "macos" },
          typography_profile: { ...snapshot.state.domain.typography_profile, platform: "macos" },
          local_encryption: { kind: "lockedOrInaccessible" }
        }
      }
    });
    window.__harness.pushStateUpdate();
  });
  await expect(page.getByText("macOS Keychain")).toBeVisible();
  await expect(page.getByText("Credential store locked")).toBeVisible();

  await page.getByRole("button", { name: "Check local encryption" }).click();
  await expect.poll(() => invocationCount(page, "probe_local_encryption_health")).toBe(1);
  await expect(page.getByText("Protected")).toBeVisible();

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          locale_profile: { ...snapshot.state.domain.locale_profile, platform: "windows" },
          typography_profile: { ...snapshot.state.domain.typography_profile, platform: "windows" },
          local_encryption: { kind: "resetRequired" }
        }
      }
    });
    window.__harness.pushStateUpdate();
  });
  await expect(page.getByText("Windows Credential Manager")).toBeVisible();
  await expect(page.getByText("Reset local data required")).toBeVisible();
  await expect(page.getByRole("button", { name: "Open recovery" })).toBeVisible();

  await page.evaluate(() => {
    window.__harness.setCommandResponse("reset_local_data", () => {
      const snapshot = window.__harness.currentSnapshot();
      const next = {
        ...snapshot,
        state: {
          ...snapshot.state,
          domain: {
            ...snapshot.state.domain,
            local_encryption: { kind: "unknown" as const }
          }
        }
      };
      window.__harness.setSnapshot(next);
      return next;
    });
  });
  await page.getByRole("button", { name: "Reset local data" }).click();
  const resetDialog = page.getByRole("dialog", { name: "Reset local data" });
  await expect(resetDialog).toBeVisible();
  await expect(resetDialog.getByText("This cannot be undone.")).toBeVisible();
  await expect.poll(() => invocationCount(page, "reset_local_data")).toBe(0);
  await resetDialog.getByRole("button", { name: "Cancel" }).click();
  await expect(resetDialog).toHaveCount(0);

  await page.getByRole("button", { name: "Reset local data" }).click();
  const confirmResetDialog = page.getByRole("dialog", { name: "Reset local data" });
  await confirmResetDialog.getByRole("button", { name: "Reset local data" }).click();
  await expect.poll(() => invocationCount(page, "reset_local_data")).toBe(1);
  await expect(page.getByText("Not checked")).toBeVisible();
});

test("E2EE trust controls dispatch Rust-owned commands and render snapshot updates", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    window.__harness.setSnapshot(window.__harness.e2eeTrustSnapshot());
    window.__harness.setCommandResponse("refresh_current_session_status", () =>
      window.__harness.currentSnapshot()
    );
    window.__harness.pushStateUpdate();
  });

  await page.getByRole("button", { name: "User settings" }).click();
  await page.getByRole("tab", { name: "Encryption", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Encryption" })).toBeVisible();
  await expect(page.getByText("Device verification")).toBeVisible();
  await expect(page.getByText("Device 1")).toHaveCount(0);
  await expect(page.getByText("redacted-trust-target")).toHaveCount(0);

  await page.evaluate(() => window.__harness.clearInvocations());
  await page.getByRole("button", { name: "Accept" }).click();

  await expect.poll(() => invocationCount(page, "accept_verification")).toBe(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("accept_verification")[0]?.args)
    )
    .toEqual({ flowId: 9001 });
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.currentSnapshot().state.domain.e2ee_trust.verification.kind)
    )
    .toBe("accepted");
  await expect(page.getByText("Accepted")).toBeVisible();

  await page.getByRole("button", { name: "Enable", exact: true }).click();
  await expect.poll(() => invocationCount(page, "enable_key_backup")).toBe(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.currentSnapshot().state.domain.e2ee_trust.key_backup.kind)
    )
    .toBe("enabled");
  await expect(page.getByText("Enabled")).toBeVisible();

  await page.getByRole("button", { name: "Set up", exact: true }).click();
  await expect.poll(() => invocationCount(page, "bootstrap_cross_signing")).toBe(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.currentSnapshot().state.domain.e2ee_trust.cross_signing.kind)
    )
    .toBe("trusted");

  await page.getByRole("button", { name: "Reset", exact: true }).click();
  await expect.poll(() => invocationCount(page, "reset_identity")).toBe(1);
  await expect(page.getByLabel("Password")).toBeVisible();
  await page.getByRole("button", { name: "Cancel identity reset" }).click();
  await expect.poll(() => invocationCount(page, "cancel_identity_reset")).toBe(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("cancel_identity_reset")[0]?.args)
    )
    .toEqual({ flowId: 9100 });
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.currentSnapshot().state.domain.e2ee_trust.identity_reset.kind)
    )
    .toBe("failed");

  await page.getByRole("button", { name: "Reset", exact: true }).click();
  await expect.poll(() => invocationCount(page, "reset_identity")).toBe(2);
  await expect(page.getByLabel("Password")).toBeVisible();
  await page.getByLabel("Password").fill("identity reset smoke password");
  await page.getByRole("button", { name: "Continue" }).click();

  await expect.poll(() => invocationCount(page, "submit_identity_reset_password")).toBe(1);
  await expect
    .poll(async () =>
      page.evaluate(
        () => window.__harness.invocationsOf("submit_identity_reset_password")[0]?.args
      )
    )
    .toEqual({ flowId: 9100, password: "[REDACTED]" });
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.currentSnapshot().state.domain.e2ee_trust.identity_reset.kind)
    )
    .toBe("idle");
});

test("security settings drive Rust-owned room-key transfer and secure backup state", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    window.__harness.setSnapshot(window.__harness.e2eeTrustSnapshot());
    window.__harness.pushStateUpdate();
    window.__harness.clearInvocations();
  });

  await page.getByRole("button", { name: "User settings" }).click();
  await page.getByRole("tab", { name: "Encryption", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Key management" })).toBeVisible();
  await page.evaluate(() => {
    window.__harness.setCommandResponse("plugin:dialog|save", "/tmp/koushi-export.txt");
    window.__harness.setCommandResponse(
      "plugin:dialog|open",
      "/tmp/element-compatible-keys.txt"
    );
  });

  const exportForm = page.getByRole("form", { name: "Room key export", exact: true });
  await exportForm.getByRole("button", { name: "Export room keys" }).click();
  const exportDialog = page.getByRole("dialog", { name: "Room key passphrase" });
  await exportDialog.getByLabel("Room key passphrase").fill("synthetic-export-passphrase");
  await exportDialog.getByRole("button", { name: "Export room keys" }).click();
  await expect.poll(() => invocationCount(page, "export_room_keys")).toBe(1);
  await expect(page.getByTestId("room-key-export-state")).toHaveText("Exported");
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("export_room_keys")[0]?.args)
    )
    .toEqual({
      destinationPath: "[REDACTED]",
      passphrase: "[REDACTED]"
    });

  const importForm = page.getByRole("form", { name: "Room key import", exact: true });
  await importForm.getByRole("button", { name: "Import room keys" }).click();
  const importDialog = page.getByRole("dialog", { name: "Room key passphrase" });
  await importDialog.getByLabel("Room key passphrase").fill("synthetic-import-passphrase");
  await importDialog.getByRole("button", { name: "Import room keys" }).click();
  await expect.poll(() => invocationCount(page, "import_room_keys")).toBe(1);
  await expect(page.getByTestId("room-key-import-state")).toHaveText("1 of 1 imported");
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("import_room_keys")[0]?.args)
    )
    .toEqual({
      sourcePath: "[REDACTED]",
      passphrase: "[REDACTED]"
    });

  const secureBackupForm = page.getByRole("form", { name: "Secure backup", exact: true });
  await secureBackupForm
    .getByLabel("Secure backup passphrase", { exact: true })
    .fill("synthetic-secure-backup-passphrase");
  await page.evaluate(() =>
    window.__harness.setCommandResponse("plugin:dialog|save", "/tmp/recovery-key.txt")
  );
  await secureBackupForm.getByRole("button", { name: "Set up secure backup" }).click();
  await expect.poll(() => invocationCount(page, "bootstrap_secure_backup")).toBe(1);
  await expect(page.getByTestId("secure-backup-state")).toHaveText("Recovery key saved");
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("bootstrap_secure_backup")[0]?.args)
    )
    .toEqual({
      passphrase: "[REDACTED]",
      recoveryKeyDestinationPath: "[REDACTED]",
      intent: { kind: "initialSetup" }
    });

  const passphraseChangeForm = page.getByRole("form", {
    name: "Change secure backup passphrase",
    exact: true
  });
  await passphraseChangeForm
    .getByLabel("Current recovery secret")
    .fill("synthetic-current-recovery-secret");
  await passphraseChangeForm
    .getByLabel("New secure backup passphrase")
    .fill("synthetic-new-secure-backup-passphrase");
  await page.evaluate(() =>
    window.__harness.setCommandResponse("plugin:dialog|save", "/tmp/changed-recovery-key.txt")
  );
  await passphraseChangeForm
    .getByRole("button", { name: "Update secure backup passphrase" })
    .click();
  await expect.poll(() => invocationCount(page, "change_secure_backup_passphrase")).toBe(1);
  await expect(page.getByTestId("secure-backup-passphrase-change-state")).toHaveText(
    "Changed; recovery key saved"
  );
  await expect
    .poll(async () =>
      page.evaluate(
        () => window.__harness.invocationsOf("change_secure_backup_passphrase")[0]?.args
      )
    )
    .toEqual({
      oldSecret: "[REDACTED]",
      newPassphrase: "[REDACTED]",
      recoveryKeyDestinationPath: "[REDACTED]"
    });

  const serializedPrivateState = await page.evaluate(() =>
    JSON.stringify(window.__harness.currentSnapshot().state.domain.e2ee_trust.key_management)
  );
  expect(serializedPrivateState).not.toContain("synthetic-");
  expect(serializedPrivateState).not.toContain("/tmp/");

  const recordedIpc = await page.evaluate(() => JSON.stringify(window.__harness.invocations()));
  expect(recordedIpc).not.toContain("synthetic-");
  expect(recordedIpc).not.toContain("/tmp/");
  expect(recordedIpc).toContain("[REDACTED]");
});

test("sliding sync capability block exposes only Rust-owned recovery actions", async ({ page }) => {
  await gotoReadyShell(page);

  const showCapabilityBlock = async () => {
    await page.evaluate(() => {
      const snapshot = window.__harness.currentSnapshot();
      const blocked = {
        ...snapshot,
        state: {
          ...snapshot.state,
          domain: {
            ...snapshot.state.domain,
            session: {
              kind: "capabilityBlocked" as const,
              homeserver: "https://unsupported.example.invalid",
              user_id: "@blocked:example.invalid",
              device_id: "BLOCKED",
              failure: "unsupported" as const
            },
            sync: "stopped" as const,
            rooms: [],
            spaces: [],
            invites: []
          }
        },
        timeline: []
      };
      window.__harness.setSnapshot(blocked);
      window.__harness.setCommandResponse("retry_sliding_sync_capability", blocked);
      window.__harness.pushStateUpdate();
      window.__harness.clearInvocations();
    });
    await expect(page.getByTestId("sliding-sync-capability-blocked")).toBeVisible();
  };

  await showCapabilityBlock();
  await expect(page.getByText("This homeserver does not support Simplified Sliding Sync.")).toBeVisible();

  await page.getByRole("button", { name: "Retry" }).click();
  await expect.poll(() => invocationCount(page, "retry_sliding_sync_capability")).toBe(1);

  await page.getByRole("button", { name: "Change homeserver" }).click();
  await expect.poll(() => invocationCount(page, "change_homeserver")).toBe(1);
  expect(await invocationCount(page, "logout")).toBe(0);
  await expect
    .poll(() => page.evaluate(() => window.__harness.currentSnapshot().state.domain.session.kind))
    .toBe("signedOut");
  await expect(page.getByTestId("auth-screen")).toBeVisible();

  await showCapabilityBlock();
  await page.getByRole("button", { name: "Sign out" }).click();
  await expect.poll(() => invocationCount(page, "logout")).toBe(1);
  await expect(page.getByTestId("auth-screen")).toBeVisible();
});

test("encrypted room suppresses link previews and shows privacy notice", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(async () => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          rooms: snapshot.state.domain.rooms.map((room, index) =>
            index === 0 ? { ...room, is_encrypted: true } : room
          )
        }
      }
    });
    await window.__harness.pushStateUpdate();
  });

  await page.getByRole("button", { name: t("room.roomInfo") }).click();
  await expect(page.getByText(t("settings.urlPreviewsEncryptedNotice"))).toBeVisible();
});
