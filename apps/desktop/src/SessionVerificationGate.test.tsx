// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, test, vi } from "vitest";
import { SessionVerificationGate } from "./components/SessionVerificationGate";
import { createBrowserFakeApi } from "./backend/browserFakeApi";
import type {
  DesktopSnapshot,
  ProvisionalPhase,
  SecureBackupGateState
} from "./domain/types";

const provisionalPhaseCases: Array<[ProvisionalPhase, string]> = [
  ["checkingTrust", "Checking device trust…"],
  [{ kind: "checkingTrust" }, "Checking device trust…"],
  ["discoveringMethods", "Discovering verification methods…"],
  [{ kind: "discoveringMethods" }, "Discovering verification methods…"],
  [{ recheckingTrust: { failureKind: "timeout" } }, "Finishing sign-in…"],
  [{ kind: "recheckingTrust", failureKind: "timeout" }, "Finishing sign-in…"],
];

describe("SessionVerificationGate interactions", () => {
  function secureBackupSnapshot(
    snapshot: DesktopSnapshot,
    secureBackupGate: SecureBackupGateState
  ): DesktopSnapshot {
    const currentSession = snapshot.state.domain.session;
    snapshot.state.domain.session = {
      kind: "ready",
      homeserver: currentSession.homeserver ?? "https://example.invalid",
      user_id: currentSession.user_id ?? "@user:example.invalid",
      device_id: currentSession.device_id ?? "DEVICE"
    };
    snapshot.state.domain.secure_backup_gate = secureBackupGate;
    return snapshot;
  }

  function secureBackupOperations(
    snapshot: DesktopSnapshot,
    overrides: Partial<{
      recoverSecureBackup: (secret: string) => Promise<DesktopSnapshot>;
      setupSecureBackup: (
        passphrase: string | null,
        recoveryKeyDestinationPath: string | null
      ) => Promise<DesktopSnapshot>;
      reenableSecureBackup: (
        passphrase: string | null,
        recoveryKeyDestinationPath: string | null
      ) => Promise<DesktopSnapshot>;
      chooseSecureBackupDestination: () => Promise<string | null>;
      retrySecureBackupInspection: () => Promise<DesktopSnapshot>;
      openSecureBackupDiagnostics: () => Promise<void>;
    }> = {}
  ) {
    return {
      startOwnUserSas: async () => snapshot,
      submitRecovery: async () => snapshot,
      recoverSecureBackup: async () => snapshot,
      setupSecureBackup: async () => snapshot,
      reenableSecureBackup: async () => snapshot,
      chooseSecureBackupDestination: async () => "/tmp/recovery-key.txt",
      retrySecureBackupInspection: async () => snapshot,
      openSecureBackupDiagnostics: async () => undefined,
      ...overrides
    };
  }

  function setCleanupSurfaceSession(snapshot: DesktopSnapshot): void {
    snapshot.state.domain.session = {
      kind: "awaitingVerification",
      user_id: "@u:example.invalid",
      homeserver: "https://example.invalid",
      device_id: "D",
      gate: {
        methods: ["recoveryKey"],
        account_kind: "existingIdentity",
        failureKind: "sdk"
      }
    };
  }

  afterEach(cleanup);

  test.each([true, false])(
    "renders authentication-specific locked copy and sign-out-only controls for soft_logout=%s",
    async (soft_logout) => {
      const snapshot = await createBrowserFakeApi({ session: "locked" }).getSnapshot();
      snapshot.state.domain.session_lock_reason = { kind: "unknownToken", soft_logout };

      render(
        <SessionVerificationGate
          snapshot={snapshot}
          onSnapshot={() => undefined}
          onSignOut={() => undefined}
        />
      );

      expect(screen.getByRole("heading", { name: "Session expired" })).toBeTruthy();
      expect(
        screen.getByText(
          "This session has expired or was revoked. Sign in again to continue."
        )
      ).toBeTruthy();
      expect(screen.getAllByRole("button")).toHaveLength(1);
      expect(screen.getByRole("button", { name: "Sign out" })).toBeTruthy();
      expect(screen.queryByText("This session must be verified again.")).toBeNull();
      expect(screen.queryByRole("button", { name: /verify|recovery|remove|backup/i })).toBeNull();
    }
  );

  test("keeps unknown trust retryable without offering verification or cleanup", async () => {
    const snapshot = await createBrowserFakeApi({ session: "needsRecovery" }).getSnapshot();
    snapshot.state.domain.session = {
      kind: "provisional",
      user_id: "@u:example.invalid",
      homeserver: "https://example.invalid",
      device_id: "D",
      phase: { kind: "recheckingTrust" }
    };
    snapshot.state.domain.device_cleanup = { kind: "idle" };

    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onSnapshot={() => undefined}
        onSignOut={() => undefined}
      />
    );

    expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /verify|recovery|remove/i })).toBeNull();
    expect(screen.getByRole("button", { name: "Sign out" })).toBeTruthy();
  });

  test("production requires warning confirmation before starting device verification", async () => {
    const snapshot = await createBrowserFakeApi({ session: "needsRecovery" }).getSnapshot();
    snapshot.state.domain.session = {
      kind: "awaitingVerification",
      user_id: "@u:example.invalid",
      homeserver: "https://example.invalid",
      device_id: "D",
      gate: { methods: ["existingDeviceSas", "recoveryKey"], account_kind: "existingIdentity" }
    };
    const startOwnUserSas = vi.fn(async () => snapshot);
    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onSnapshot={() => undefined}
        onSignOut={() => undefined}
        operations={{ startOwnUserSas, submitRecovery: async () => snapshot }}
      />
    );

    expect(screen.queryByRole("dialog", { name: "Try device verification?" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Verify with another device" }));
    expect(startOwnUserSas).not.toHaveBeenCalled();
    const dialog = screen.getByRole("dialog", { name: "Try device verification?" });
    expect(within(dialog).getByText(/can be unreliable/)).toBeTruthy();
    expect(within(dialog).getByRole("button", { name: "Use recovery key" })).toBeTruthy();
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog", { name: "Try device verification?" })).toBeNull();
    expect(startOwnUserSas).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Verify with another device" }));
    fireEvent.click(
      screen.getByRole("button", { name: "Try device verification anyway" })
    );
    await vi.waitFor(() => expect(startOwnUserSas).toHaveBeenCalledTimes(1));
  });

  test("production renders the Rust-owned seven-emoji SAS comparison", async () => {
    const snapshot = await createBrowserFakeApi({ session: "needsRecovery" }).getSnapshot();
    snapshot.state.domain.session = {
      kind: "verifying",
      user_id: "@u:example.invalid",
      homeserver: "https://example.invalid",
      device_id: "D",
      method: "existingDeviceSas",
      flow_id: 370,
      gate: {
        methods: ["existingDeviceSas"],
        account_kind: "existingIdentity",
        failureKind: null
      },
      sas_emojis: Array.from({ length: 7 }, (_, index) => ({
        symbol: "🐶",
        description: `emoji-${index}`
      }))
    };
    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onSnapshot={() => undefined}
        onSignOut={() => undefined}
        operations={{
          startOwnUserSas: async () => snapshot,
          submitRecovery: async () => snapshot
        }}
      />
    );

    expect(document.querySelectorAll(".session-verification-emojis span")).toHaveLength(7);
    expect(screen.getByRole("button", { name: "They match" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "They do not match" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeTruthy();
  });

  test("SAS-only availability is actionable instead of a no-recovery dead end", async () => {
    const snapshot = await createBrowserFakeApi({ session: "needsRecovery" }).getSnapshot();
    snapshot.state.domain.session = {
      kind: "awaitingVerification",
      user_id: "@u:example.invalid",
      homeserver: "https://example.invalid",
      device_id: "D",
      gate: { methods: ["existingDeviceSas"], account_kind: "existingIdentity" }
    };
    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onSnapshot={() => undefined}
        onSignOut={() => undefined}
        operations={{
          startOwnUserSas: async () => snapshot,
          submitRecovery: async () => snapshot
        }}
      />
    );

    expect(screen.getByRole("button", { name: "Verify with another device" })).toBeTruthy();
    expect(
      screen.queryByRole("heading", { name: "No recovery key available" })
    ).toBeNull();
    expect(screen.queryByLabelText("Recovery secret")).toBeNull();
  });

  test("explains the dead end when no verification method is available", async () => {
    const snapshot = await createBrowserFakeApi({ session: "needsRecovery" }).getSnapshot();
    snapshot.state.domain.session = {
      kind: "awaitingVerification",
      user_id: "@u:example.invalid",
      homeserver: "https://example.invalid",
      device_id: "D",
      gate: { methods: [], account_kind: "existingIdentity" }
    };
    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onSnapshot={() => undefined}
        onSignOut={() => undefined}
        operations={{
          startOwnUserSas: async () => snapshot,
          submitRecovery: async () => snapshot
        }}
      />
    );

    expect(
      screen.getByRole("heading", { name: "No recovery key available" })
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Verify with another device" })).toBeNull();
  });

  test.each(provisionalPhaseCases)("renders provisional phase %j with retry only while rechecking", async (phase, copy) => {
    const snapshot = await createBrowserFakeApi({ session: "needsRecovery" }).getSnapshot();
    snapshot.state.domain.session = {
      kind: "provisional",
      user_id: "@u:example.invalid",
      homeserver: "https://example.invalid",
      device_id: "D",
      phase,
    };
    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onSnapshot={() => undefined}
        onSignOut={() => undefined}
        operations={{ startOwnUserSas: async () => snapshot, submitRecovery: async () => snapshot }}
      />
    );

    expect(screen.getByText(copy)).toBeTruthy();
    if (copy === "Finishing sign-in…") {
      expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy();
    } else {
      expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
    }
  });

  test("uses checking-trust copy for both the landmark and heading", async () => {
    const snapshot = await createBrowserFakeApi({ session: "needsRecovery" }).getSnapshot();
    snapshot.state.domain.session = {
      kind: "provisional",
      user_id: "@u:example.invalid",
      homeserver: "https://example.invalid",
      device_id: "D",
      phase: "checkingTrust",
    };
    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onSnapshot={() => undefined}
        onSignOut={() => undefined}
      />
    );

    expect(screen.getByRole("main", { name: "Checking device trust…" })).toBeTruthy();
    expect(
      screen.getByRole("heading", { level: 1, name: "Checking device trust…" })
    ).toBeTruthy();
    expect(screen.queryByText("Verify this session")).toBeNull();
  });

  test("admits SAS and recovery independently and blocks duplicate promise construction", async () => {
    const snapshot = await createBrowserFakeApi({ session: "needsRecovery" }).getSnapshot();
    snapshot.state.domain.session = { kind: "awaitingVerification", user_id: "@u:example.invalid", homeserver: "https://example.invalid", device_id: "D", gate: { methods: ["existingDeviceSas", "recoveryKey"], account_kind: "existingIdentity" } };
    let releaseSas!: (value: typeof snapshot) => void;
    const sasPromise = new Promise<typeof snapshot>((resolve) => { releaseSas = resolve; });
    const startOwnUserSas = vi.fn(() => sasPromise);
    const submitRecovery = vi.fn(async () => snapshot);
    render(<SessionVerificationGate snapshot={snapshot} onSnapshot={() => undefined} onSignOut={() => undefined} operations={{ startOwnUserSas, submitRecovery }} />);

    const sas = screen.getByRole("button", { name: "Verify with another device" });
    const recovery = screen.getByRole("button", { name: "Verify with recovery key" });
    expect(
      recovery.compareDocumentPosition(sas) & Node.DOCUMENT_POSITION_FOLLOWING
    ).toBeTruthy();
    fireEvent.click(sas);
    expect(startOwnUserSas).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog", { name: "Try device verification?" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Use recovery key" }));
    expect(screen.queryByRole("dialog", { name: "Try device verification?" })).toBeNull();
    expect(startOwnUserSas).not.toHaveBeenCalled();
    fireEvent.click(sas);
    fireEvent.click(screen.getByRole("button", { name: "Try device verification anyway" }));
    expect(startOwnUserSas).toHaveBeenCalledTimes(1);

    fireEvent.change(screen.getByLabelText("Recovery secret"), { target: { value: "fixture-secret" } });
    fireEvent.click(screen.getByRole("button", { name: "Verify with recovery key" }));
    expect(submitRecovery).toHaveBeenCalledTimes(1);
    releaseSas(snapshot);
  });

  test("rejected operation settles and permits a later attempt", async () => {
    const snapshot = await createBrowserFakeApi({ session: "needsRecovery" }).getSnapshot();
    snapshot.state.domain.session = { kind: "awaitingVerification", user_id: "@u:example.invalid", homeserver: "https://example.invalid", device_id: "D", gate: { methods: ["existingDeviceSas"], account_kind: "existingIdentity" } };
    const startOwnUserSas = vi.fn().mockRejectedValueOnce(new Error("rejected")).mockResolvedValue(snapshot);
    render(<SessionVerificationGate snapshot={snapshot} onSnapshot={() => undefined} onSignOut={() => undefined} operations={{ startOwnUserSas, submitRecovery: async () => snapshot }} />);
    const button = screen.getByRole("button", { name: "Verify with another device" });
    fireEvent.click(button);
    fireEvent.click(screen.getByRole("button", { name: "Try device verification anyway" }));
    await vi.waitFor(() => expect((button as HTMLButtonElement).disabled).toBe(false));
    expect(screen.getByRole("alert").textContent).toContain("Verification command failed");
    fireEvent.click(button);
    fireEvent.click(screen.getByRole("button", { name: "Try device verification anyway" }));
    await vi.waitFor(() => expect(startOwnUserSas).toHaveBeenCalledTimes(2));
  });

  test("does not offer recovery-key fallback when only SAS is available", async () => {
    const snapshot = await createBrowserFakeApi({ session: "needsRecovery" }).getSnapshot();
    snapshot.state.domain.session = { kind: "awaitingVerification", user_id: "@u:example.invalid", homeserver: "https://example.invalid", device_id: "D", gate: { methods: ["existingDeviceSas"], account_kind: "existingIdentity" } };
    const startOwnUserSas = vi.fn(async () => snapshot);
    render(<SessionVerificationGate snapshot={snapshot} onSnapshot={() => undefined} onSignOut={() => undefined} operations={{ startOwnUserSas, submitRecovery: async () => snapshot }} />);

    fireEvent.click(screen.getByRole("button", { name: "Verify with another device" }));

    expect(screen.getByRole("dialog", { name: "Try device verification?" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Use recovery key" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Try device verification anyway" }));
    expect(startOwnUserSas).toHaveBeenCalledTimes(1);
  });

  test("requires consequence confirmation before starting remote-first device cleanup", async () => {
    const snapshot = await createBrowserFakeApi({ session: "needsRecovery" }).getSnapshot();
    snapshot.state.domain.session = {
      kind: "awaitingVerification",
      user_id: "@u:example.invalid",
      homeserver: "https://example.invalid",
      device_id: "POISONED",
      gate: {
        methods: ["recoveryKey"],
        account_kind: "existingIdentity",
        failureKind: "sdk",
      },
    };
    snapshot.state.domain.device_cleanup = {
      kind: "offered",
      reason: "recoveryFailed"
    };
    const resolvingSnapshot = structuredClone(snapshot);
    resolvingSnapshot.state.domain.device_cleanup = {
      kind: "resolvingRemote",
      request_id: 370
    };
    const startDeviceCleanup = vi.fn(async () => resolvingSnapshot);
    const onSnapshot = vi.fn();

    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onSnapshot={onSnapshot}
        onSignOut={() => undefined}
        operations={{
          startOwnUserSas: async () => snapshot,
          submitRecovery: async () => snapshot,
          startDeviceCleanup
        }}
      />
    );

    expect(startDeviceCleanup).not.toHaveBeenCalled();
    fireEvent.click(
      screen.getByRole("button", {
        name: "Cancel sign-in and remove this device…",
      })
    );
    const dialog = screen.getByRole("dialog", {
      name: "Cancel sign-in and remove this device",
    });
    expect(dialog).toBeTruthy();
    expect(within(dialog).getByText(/remove this device from your Matrix account first/i)).toBeTruthy();
    expect(within(dialog).getByText(/local messages.*encryption keys/i)).toBeTruthy();
    expect(within(dialog).getByText(/messages on your homeserver are preserved/i)).toBeTruthy();
    expect(within(dialog).getByText(/next sign-in creates a new Device ID/i)).toBeTruthy();
    fireEvent.click(
      within(dialog).getByRole("button", {
        name: "Remove device and erase local data",
      })
    );

    await vi.waitFor(() => expect(startDeviceCleanup).toHaveBeenCalledTimes(1));
    expect(onSnapshot).toHaveBeenCalledWith(resolvingSnapshot);
  });

  test("submits legacy UIA password through the IME-safe cleanup form", async () => {
    const snapshot = await createBrowserFakeApi({ session: "needsRecovery" }).getSnapshot();
    setCleanupSurfaceSession(snapshot);
    snapshot.state.domain.device_cleanup = {
      kind: "awaitingUia",
      request_id: 371,
      flow_id: 41
    };
    const submitDeviceCleanupUia = vi.fn(async () => snapshot);
    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onSnapshot={() => undefined}
        onSignOut={() => undefined}
        operations={{
          startOwnUserSas: async () => snapshot,
          submitRecovery: async () => snapshot,
          submitDeviceCleanupUia
        }}
      />
    );

    const password = screen.getByLabelText("Account password") as HTMLInputElement;
    fireEvent.change(password, { target: { value: "synthetic-password" } });
    fireEvent.click(screen.getByRole("button", { name: "Continue device removal" }));

    await vi.waitFor(() =>
      expect(submitDeviceCleanupUia).toHaveBeenCalledWith(41, "synthetic-password")
    );
    expect(password.value).toBe("");
  });

  test("offers retry and separately confirms local erasure after remote cleanup fails", async () => {
    const snapshot = await createBrowserFakeApi({ session: "needsRecovery" }).getSnapshot();
    setCleanupSurfaceSession(snapshot);
    snapshot.state.domain.device_cleanup = {
      kind: "remoteFailed",
      request_id: 372,
      auth_mode: "legacy",
      failureKind: "network"
    };
    const startDeviceCleanup = vi.fn(async () => snapshot);
    const eraseLocalDataAnyway = vi.fn(async () => snapshot);
    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onSnapshot={() => undefined}
        onSignOut={() => undefined}
        operations={{
          startOwnUserSas: async () => snapshot,
          submitRecovery: async () => snapshot,
          startDeviceCleanup,
          eraseLocalDataAnyway
        }}
      />
    );

    expect(
      screen.getByText(/Your credentials and local data are still preserved/)
    ).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Retry removing device" }));
    await vi.waitFor(() => expect(startDeviceCleanup).toHaveBeenCalledTimes(1));

    const eraseAnywayOffer = screen.getByRole("button", {
      name: "Erase local data anyway…"
    }) as HTMLButtonElement;
    await vi.waitFor(() => expect(eraseAnywayOffer.disabled).toBe(false));
    fireEvent.click(eraseAnywayOffer);
    const dialog = screen.getByRole("dialog", { name: "Erase local data anyway" });
    expect(within(dialog).getByText(/device may remain active on your Matrix account/i)).toBeTruthy();
    expect(eraseLocalDataAnyway).not.toHaveBeenCalled();
    fireEvent.click(within(dialog).getByRole("button", { name: "Erase local data anyway" }));
    await vi.waitFor(() => expect(eraseLocalDataAnyway).toHaveBeenCalledTimes(1));
  });

  test("never asks for a password on the OAuth cleanup failure path", async () => {
    const snapshot = await createBrowserFakeApi({ session: "needsRecovery" }).getSnapshot();
    setCleanupSurfaceSession(snapshot);
    snapshot.state.domain.device_cleanup = {
      kind: "remoteFailed",
      request_id: 373,
      auth_mode: "oAuth",
      failureKind: "forbidden"
    };
    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onSnapshot={() => undefined}
        onSignOut={() => undefined}
      />
    );

    expect(screen.queryByLabelText("Account password")).toBeNull();
    expect(screen.getByRole("button", { name: "Retry removing device" })).toBeTruthy();
  });

  test("shows progress without duplicate cleanup actions while remote removal is pending", async () => {
    const snapshot = await createBrowserFakeApi({ session: "needsRecovery" }).getSnapshot();
    setCleanupSurfaceSession(snapshot);
    snapshot.state.domain.device_cleanup = {
      kind: "removingRemote",
      request_id: 374,
      auth_mode: "legacy"
    };
    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onSnapshot={() => undefined}
        onSignOut={() => undefined}
      />
    );

    expect(screen.getByText("Removing this device from your Matrix account…")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Retry removing device" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Erase local data anyway…" })).toBeNull();
  });

  test("does not offer destructive cleanup while a recovery retry is verifying", async () => {
    const snapshot = await createBrowserFakeApi({ session: "needsRecovery" }).getSnapshot();
    snapshot.state.domain.session = {
      kind: "verifying",
      user_id: "@u:example.invalid",
      homeserver: "https://example.invalid",
      device_id: "D",
      gate: {
        methods: ["recoveryKey"],
        account_kind: "existingIdentity",
        failureKind: "sdk"
      },
      method: "recoveryKey",
      flow_id: 375,
      sas_emojis: []
    };
    snapshot.state.domain.device_cleanup = {
      kind: "offered",
      reason: "recoveryFailed"
    };

    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onSnapshot={() => undefined}
        onSignOut={() => undefined}
      />
    );

    expect(
      screen.queryByRole("button", {
        name: "Cancel sign-in and remove this device…"
      })
    ).toBeNull();
  });

  test("provides a primary-button-only verification window drag region", async () => {
    const snapshot = await createBrowserFakeApi({ session: "needsRecovery" }).getSnapshot();
    snapshot.state.domain.session = {
      kind: "awaitingVerification",
      user_id: "@u:example.invalid",
      homeserver: "https://example.invalid",
      device_id: "D",
      gate: { methods: ["existingDeviceSas"], account_kind: "existingIdentity" },
    };
    const onStartWindowDrag = vi.fn();
    const { container } = render(
      <SessionVerificationGate
        snapshot={snapshot}
        onSnapshot={() => undefined}
        onSignOut={() => undefined}
        onStartWindowDrag={onStartWindowDrag}
        operations={{
          startOwnUserSas: async () => snapshot,
          submitRecovery: async () => snapshot,
        }}
      />
    );

    const dragRegion = container.querySelector(".session-verification-drag-region");
    expect(dragRegion?.getAttribute("data-tauri-drag-region")).toBe("");
    fireEvent.mouseDown(dragRegion!, { button: 2, buttons: 2 });
    expect(onStartWindowDrag).not.toHaveBeenCalled();
    fireEvent.mouseDown(dragRegion!, { button: 0, buttons: 1 });
    expect(onStartWindowDrag).toHaveBeenCalledTimes(1);
  });

  test("renders a mandatory secure-backup checking gate for an otherwise ready session", async () => {
    const snapshot = secureBackupSnapshot(
      await createBrowserFakeApi().getSnapshot(),
      { kind: "checking" }
    );

    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onSnapshot={() => undefined}
        onSignOut={() => undefined}
        operations={secureBackupOperations(snapshot)}
      />
    );

    expect(screen.getByRole("main", { name: "Secure backup required" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Checking secure backup…" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Create room" })).toBeNull();
  });

  test("masks and clears the secure-backup recovery key after submission", async () => {
    const snapshot = secureBackupSnapshot(
      await createBrowserFakeApi().getSnapshot(),
      { kind: "existingBackupNeedsRecovery", failure: "invalidRecoveryKey" }
    );
    const recoverSecureBackup = vi.fn(async () => snapshot);

    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onSnapshot={() => undefined}
        onSignOut={() => undefined}
        operations={secureBackupOperations(snapshot, { recoverSecureBackup })}
      />
    );

    const recoveryKey = screen.getByLabelText("Secure backup recovery key") as HTMLInputElement;
    expect(recoveryKey.type).toBe("password");
    fireEvent.change(recoveryKey, { target: { value: "synthetic-recovery-key" } });
    fireEvent.click(screen.getByRole("button", { name: "Recover secure backup" }));

    await vi.waitFor(() =>
      expect(recoverSecureBackup).toHaveBeenCalledWith("synthetic-recovery-key")
    );
    expect(recoveryKey.value).toBe("");
    expect(screen.getByRole("alert").textContent).toContain("recovery key");
  });

  test("recovers incomplete secure storage instead of offering destructive setup", async () => {
    const snapshot = secureBackupSnapshot(
      await createBrowserFakeApi().getSnapshot(),
      { kind: "secureStorageIncomplete" }
    );

    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onSnapshot={() => undefined}
        onSignOut={() => undefined}
        operations={secureBackupOperations(snapshot)}
      />
    );

    expect(screen.getByLabelText("Secure backup recovery key")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Recover secure backup" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Set up secure backup" })).toBeNull();
  });

  test("submits setup passphrase and native destination selection without retaining either value", async () => {
    const snapshot = secureBackupSnapshot(
      await createBrowserFakeApi().getSnapshot(),
      { kind: "setupRequired" }
    );
    const setupSecureBackup = vi.fn(async () => snapshot);
    const chooseSecureBackupDestination = vi.fn(async () => "/tmp/recovery-key.txt");

    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onSnapshot={() => undefined}
        onSignOut={() => undefined}
        operations={secureBackupOperations(snapshot, {
          setupSecureBackup,
          chooseSecureBackupDestination
        })}
      />
    );

    const passphrase = screen.getByLabelText("Secure backup passphrase") as HTMLInputElement;
    expect(screen.queryByLabelText("Recovery key destination")).toBeNull();
    expect(screen.getByText("No recovery key destination selected.")).toBeTruthy();
    fireEvent.change(passphrase, { target: { value: "synthetic-passphrase" } });
    fireEvent.click(screen.getByRole("button", { name: "Choose recovery key destination" }));

    await vi.waitFor(() => expect(chooseSecureBackupDestination).toHaveBeenCalledTimes(1));
    await vi.waitFor(() =>
      expect(screen.getByText("Recovery key destination selected.")).toBeTruthy()
    );
    expect(screen.queryByText("/tmp/recovery-key.txt")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Set up secure backup" }));

    await vi.waitFor(() =>
      expect(setupSecureBackup).toHaveBeenCalledWith(
        "synthetic-passphrase",
        "/tmp/recovery-key.txt"
      )
    );
    expect(passphrase.value).toBe("");
  });

  test("requires explicit confirmation before re-enabling an account-wide disabled backup", async () => {
    const snapshot = secureBackupSnapshot(
      await createBrowserFakeApi().getSnapshot(),
      { kind: "explicitlyDisabledRequiresSetup" }
    );
    const reenableSecureBackup = vi.fn(async () => snapshot);
    const chooseSecureBackupDestination = vi.fn(async () => "/tmp/reenable-recovery-key.txt");

    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onSnapshot={() => undefined}
        onSignOut={() => undefined}
        operations={secureBackupOperations(snapshot, {
          reenableSecureBackup,
          chooseSecureBackupDestination
        })}
      />
    );

    expect(screen.getByText(/other Matrix clients/i)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Re-enable secure backup" }));
    const dialog = screen.getByRole("dialog", { name: "Re-enable secure backup" });
    expect(dialog).toBeTruthy();
    expect(reenableSecureBackup).not.toHaveBeenCalled();
    const passphrase = within(dialog).getByLabelText(
      "Secure backup passphrase"
    ) as HTMLInputElement;
    fireEvent.change(passphrase, { target: { value: "reenable-passphrase" } });
    expect(within(dialog).queryByLabelText("Recovery key destination")).toBeNull();
    fireEvent.click(
      within(dialog).getByRole("button", { name: "Choose recovery key destination" })
    );
    await vi.waitFor(() => expect(chooseSecureBackupDestination).toHaveBeenCalledTimes(1));
    await vi.waitFor(() =>
      expect(within(dialog).getByText("Recovery key destination selected.")).toBeTruthy()
    );
    expect(within(dialog).queryByText("/tmp/reenable-recovery-key.txt")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Confirm re-enable" }));

    await vi.waitFor(() =>
      expect(reenableSecureBackup).toHaveBeenCalledWith(
        "reenable-passphrase",
        "/tmp/reenable-recovery-key.txt"
      )
    );
    expect(passphrase.value).toBe("");
  });

  test("renders typed upload progress without exposing a raw count or error", async () => {
    const snapshot = secureBackupSnapshot(
      await createBrowserFakeApi().getSnapshot(),
      { kind: "uploadingExistingKeys", pending: "eleven_to_one_hundred" }
    );

    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onSnapshot={() => undefined}
        onSignOut={() => undefined}
        operations={secureBackupOperations(snapshot)}
      />
    );

    expect(screen.getByText("Uploading existing encrypted keys: 11–100 remaining.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Retry secure backup" })).toBeNull();
  });

  test("shows typed failure, supports retry, and exposes diagnostics without raw errors", async () => {
    const snapshot = secureBackupSnapshot(
      await createBrowserFakeApi().getSnapshot(),
      { kind: "blockedFailed", failure: "rateLimited" }
    );
    const retrySecureBackupInspection = vi.fn(async () => snapshot);
    const openSecureBackupDiagnostics = vi.fn(async () => undefined);

    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onSnapshot={() => undefined}
        onSignOut={() => undefined}
        operations={secureBackupOperations(snapshot, {
          retrySecureBackupInspection,
          openSecureBackupDiagnostics
        })}
      />
    );

    expect(screen.getByRole("alert").textContent).toContain("limited");
    expect(screen.getByRole("alert").textContent).not.toContain("raw sdk");
    fireEvent.click(screen.getByRole("button", { name: "Retry secure backup" }));
    fireEvent.click(screen.getByRole("button", { name: "Open secure backup diagnostics" }));

    await vi.waitFor(() => expect(retrySecureBackupInspection).toHaveBeenCalledTimes(1));
    await vi.waitFor(() => expect(openSecureBackupDiagnostics).toHaveBeenCalledTimes(1));
  });

  test("renders verification admission phases and an actionable preparation failure", async () => {
    const base = await createBrowserFakeApi({ session: "needsRecovery" }).getSnapshot();
    const renderGate = (snapshot: DesktopSnapshot) => renderToStaticMarkup(
      <SessionVerificationGate snapshot={snapshot} onSnapshot={() => undefined} onSignOut={() => undefined} />
    );
    expect(renderGate(base)).toContain("Verify this session");

    const verifying = structuredClone(base);
    verifying.state.domain.session = { ...base.state.domain.session, kind: "verifying", method: "recoveryKey", flow_id: 7 } as typeof base.state.domain.session;
    expect(renderGate(verifying)).toContain("Verifying this session…");

    const failed = structuredClone(base);
    failed.state.domain.session = { ...base.state.domain.session, kind: "provisional", phase: { recheckingTrust: { failureKind: "sdk" } } } as typeof base.state.domain.session;
    const failedMarkup = renderGate(failed);
    expect(failedMarkup).toContain("Finishing sign-in…");
    expect(failedMarkup).toContain('role="alert"');
    expect(failedMarkup).toContain("Retry");
  });
});
