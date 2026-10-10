// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, test, vi } from "vitest";
import { SessionVerificationGate } from "./components/SessionVerificationGate";
import { setRendererSelectedAccountTabId } from "./backend/client";
import { createDesktopApiFixture } from "./test/desktopApiFixture";
import { defaultSnapshotResponse } from "./test/tauriIpcMock";
import type {
  CommandReceipt,
  DesktopSnapshot,
  ProvisionalPhase,
  RecoveryKeyDeliveryState,
  SecureBackupFailureDetail,
  SecureBackupGateFailureKind,
  SecureBackupGateState,
  SecureBackupSetupIntent
} from "./domain/types";

const commandReceipt: CommandReceipt = { protocolVersion: 1, admittedGeneration: 1 };
// Synthetic, non-secret fixture; deliberately not shaped like a real key.
const SYNTHETIC_RECOVERY_KEY = "synthetic-revealed-recovery-key-927";

function sessionSnapshot(kind: "locked" | "needsRecovery"): DesktopSnapshot {
  const snapshot = structuredClone(defaultSnapshotResponse()) as unknown as DesktopSnapshot;
  snapshot.state.domain.session =
    kind === "locked"
      ? {
          kind: "locked",
          user_id: "@u:example.invalid",
          homeserver: "https://example.invalid",
          device_id: "D"
        }
      : {
          kind: "awaitingVerification",
          user_id: "@u:example.invalid",
          homeserver: "https://example.invalid",
          device_id: "D",
          gate: { methods: ["existingDeviceSas", "recoveryKey"], account_kind: "existingIdentity" }
        };
  return snapshot;
}

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
    _snapshot: DesktopSnapshot,
    overrides: Partial<{
      recoverSecureBackup: (secret: string) => Promise<CommandReceipt>;
      bootstrapSecureBackup: (
        passphrase: string | null,
        intent: SecureBackupSetupIntent
      ) => Promise<CommandReceipt>;
      copyRecoveryKey: (recoveryKey: string) => Promise<void>;
      saveSecureBackupRecoveryKey: (revealRequestId: number) => Promise<CommandReceipt | null>;
      confirmSecureBackupRecoveryKeySaved: (revealRequestId: number) => Promise<CommandReceipt>;
      retrySecureBackupInspection: () => Promise<CommandReceipt>;
      openSecureBackupDiagnostics: () => Promise<void>;
    }> = {}
  ) {
    return {
      startOwnUserSas: async () => commandReceipt,
      submitRecovery: async () => commandReceipt,
      recoverSecureBackup: async () => commandReceipt,
      bootstrapSecureBackup: async () => commandReceipt,
      copyRecoveryKey: async () => undefined,
      saveSecureBackupRecoveryKey: async () => commandReceipt,
      confirmSecureBackupRecoveryKeySaved: async () => commandReceipt,
      retrySecureBackupInspection: async () => commandReceipt,
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

  afterEach(() => {
    cleanup();
    setRendererSelectedAccountTabId(null);
  });

  test.each([true, false])(
    "renders authentication-specific locked copy and sign-out-only controls for soft_logout=%s",
    async (soft_logout) => {
      const snapshot = sessionSnapshot("locked");
      snapshot.state.domain.session_lock_reason = { kind: "unknownToken", soft_logout };

      render(
        <SessionVerificationGate
          snapshot={snapshot}
          onReceipt={async () => undefined}
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
    const snapshot = sessionSnapshot("needsRecovery");
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
        onReceipt={async () => undefined}
        onSignOut={() => undefined}
      />
    );

    expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /verify|recovery|remove/i })).toBeNull();
    expect(screen.getByRole("button", { name: "Sign out" })).toBeTruthy();
  });

  test("routes fallback trust commands through the supplied account API", async () => {
    const snapshot = sessionSnapshot("needsRecovery");
    snapshot.state.domain.session = {
      kind: "provisional",
      user_id: "@u:example.invalid",
      homeserver: "https://example.invalid",
      device_id: "D",
      phase: { kind: "recheckingTrust" }
    };
    const fixture = createDesktopApiFixture(snapshot);
    const accountApi = fixture.forAccountTab?.("account:@u:example.invalid");
    expect(accountApi).toBeDefined();
    setRendererSelectedAccountTabId("account:@u:example.invalid");

    render(
      <SessionVerificationGate
        desktopApi={accountApi}
        snapshot={snapshot}
        onReceipt={async () => undefined}
        onSignOut={() => undefined}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    await vi.waitFor(() =>
      expect(fixture.ipc.invocationsOf("retry_current_device_trust_discovery")).toHaveLength(1)
    );
    expect(
      fixture.ipc.invocationsOf("retry_current_device_trust_discovery")[0]?.args
    ).toEqual({ accountTabId: "account:@u:example.invalid" });
  });

  test("production requires warning confirmation before starting device verification", async () => {
    const snapshot = sessionSnapshot("needsRecovery");
    snapshot.state.domain.session = {
      kind: "awaitingVerification",
      user_id: "@u:example.invalid",
      homeserver: "https://example.invalid",
      device_id: "D",
      gate: { methods: ["existingDeviceSas", "recoveryKey"], account_kind: "existingIdentity" }
    };
    const startOwnUserSas = vi.fn(async () => commandReceipt);
    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onReceipt={async () => undefined}
        onSignOut={() => undefined}
        operations={{ startOwnUserSas, submitRecovery: async () => commandReceipt }}
      />
    );

    expect(screen.queryByRole("region", { name: "Try device verification?" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Verify with another device" }));
    expect(startOwnUserSas).not.toHaveBeenCalled();
    const dialog = screen.getByRole("region", { name: "Try device verification?" });
    expect(within(dialog).getByText(/can be unreliable/)).toBeTruthy();
    expect(within(dialog).getByRole("button", { name: "Use recovery key" })).toBeTruthy();
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("region", { name: "Try device verification?" })).toBeNull();
    expect(startOwnUserSas).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Verify with another device" }));
    fireEvent.click(
      screen.getByRole("button", { name: "Try device verification anyway" })
    );
    await vi.waitFor(() => expect(startOwnUserSas).toHaveBeenCalledTimes(1));
  });

  test("production renders the Rust-owned seven-emoji SAS comparison", async () => {
    const snapshot = sessionSnapshot("needsRecovery");
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
        onReceipt={async () => undefined}
        onSignOut={() => undefined}
        operations={{
          startOwnUserSas: async () => commandReceipt,
          submitRecovery: async () => commandReceipt
        }}
      />
    );

    expect(document.querySelectorAll(".session-verification-emojis span")).toHaveLength(7);
    expect(screen.getByRole("button", { name: "They match" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "They do not match" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeTruthy();
  });

  test("SAS-only availability is actionable instead of a no-recovery dead end", async () => {
    const snapshot = sessionSnapshot("needsRecovery");
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
        onReceipt={async () => undefined}
        onSignOut={() => undefined}
        operations={{
          startOwnUserSas: async () => commandReceipt,
          submitRecovery: async () => commandReceipt
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
    const snapshot = sessionSnapshot("needsRecovery");
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
        onReceipt={async () => undefined}
        onSignOut={() => undefined}
        operations={{
          startOwnUserSas: async () => commandReceipt,
          submitRecovery: async () => commandReceipt
        }}
      />
    );

    expect(
      screen.getByRole("heading", { name: "No recovery key available" })
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Verify with another device" })).toBeNull();
  });

  test.each(provisionalPhaseCases)("renders provisional phase %j with retry once discovery begins", async (phase, copy) => {
    const snapshot = sessionSnapshot("needsRecovery");
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
        onReceipt={async () => undefined}
        onSignOut={() => undefined}
        operations={{ startOwnUserSas: async () => commandReceipt, submitRecovery: async () => commandReceipt }}
      />
    );

    expect(screen.getByText(copy)).toBeTruthy();
    if (copy !== "Checking device trust…") {
      expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy();
    } else {
      expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
    }
  });

  test("uses checking-trust copy for both the landmark and heading", async () => {
    const snapshot = sessionSnapshot("needsRecovery");
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
        onReceipt={async () => undefined}
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
    const snapshot = sessionSnapshot("needsRecovery");
    snapshot.state.domain.session = { kind: "awaitingVerification", user_id: "@u:example.invalid", homeserver: "https://example.invalid", device_id: "D", gate: { methods: ["existingDeviceSas", "recoveryKey"], account_kind: "existingIdentity" } };
    let releaseSas!: (value: CommandReceipt) => void;
    const sasPromise = new Promise<CommandReceipt>((resolve) => { releaseSas = resolve; });
    const startOwnUserSas = vi.fn(() => sasPromise);
    const submitRecovery = vi.fn(async () => commandReceipt);
    render(<SessionVerificationGate snapshot={snapshot} onReceipt={async () => undefined} onSignOut={() => undefined} operations={{ startOwnUserSas, submitRecovery }} />);

    const sas = screen.getByRole("button", { name: "Verify with another device" });
    const recovery = screen.getByRole("button", { name: "Verify with recovery key" });
    expect(
      recovery.compareDocumentPosition(sas) & Node.DOCUMENT_POSITION_FOLLOWING
    ).toBeTruthy();
    fireEvent.click(sas);
    expect(startOwnUserSas).not.toHaveBeenCalled();
    expect(screen.getByRole("region", { name: "Try device verification?" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Use recovery key" }));
    expect(screen.queryByRole("region", { name: "Try device verification?" })).toBeNull();
    expect(startOwnUserSas).not.toHaveBeenCalled();
    fireEvent.click(sas);
    fireEvent.click(screen.getByRole("button", { name: "Try device verification anyway" }));
    expect(startOwnUserSas).toHaveBeenCalledTimes(1);

    fireEvent.change(screen.getByLabelText("Recovery secret"), { target: { value: "fixture-secret" } });
    fireEvent.click(screen.getByRole("button", { name: "Verify with recovery key" }));
    await vi.waitFor(() => expect(submitRecovery).toHaveBeenCalledTimes(1));
    expect((sas as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(sas);
    expect(startOwnUserSas).toHaveBeenCalledTimes(1);
    releaseSas(commandReceipt);
  });

  test("holds the same-kind gate until its command receipt settles", async () => {
    const snapshot = sessionSnapshot("needsRecovery");
    snapshot.state.domain.session = { kind: "awaitingVerification", user_id: "@u:example.invalid", homeserver: "https://example.invalid", device_id: "D", gate: { methods: ["existingDeviceSas"], account_kind: "existingIdentity" } };
    let releaseReceipt!: () => void;
    const receiptSettled = new Promise<void>((resolve) => { releaseReceipt = resolve; });
    const onReceipt = vi.fn(() => receiptSettled);
    const startOwnUserSas = vi.fn(async () => commandReceipt);
    render(<SessionVerificationGate snapshot={snapshot} onReceipt={onReceipt} onSignOut={() => undefined} operations={{ startOwnUserSas, submitRecovery: async () => commandReceipt }} />);

    const button = screen.getByRole("button", { name: "Verify with another device" });
    fireEvent.click(button);
    fireEvent.click(screen.getByRole("button", { name: "Try device verification anyway" }));
    await vi.waitFor(() => expect(onReceipt).toHaveBeenCalledOnce());
    expect((button as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(button);
    expect(startOwnUserSas).toHaveBeenCalledOnce();

    releaseReceipt();
    await vi.waitFor(() => expect((button as HTMLButtonElement).disabled).toBe(false));
  });

  test("rejected operation settles and permits a later attempt", async () => {
    const snapshot = sessionSnapshot("needsRecovery");
    snapshot.state.domain.session = { kind: "awaitingVerification", user_id: "@u:example.invalid", homeserver: "https://example.invalid", device_id: "D", gate: { methods: ["existingDeviceSas"], account_kind: "existingIdentity" } };
    const startOwnUserSas = vi.fn().mockRejectedValueOnce(new Error("rejected")).mockResolvedValue(commandReceipt);
    render(<SessionVerificationGate snapshot={snapshot} onReceipt={async () => undefined} onSignOut={() => undefined} operations={{ startOwnUserSas, submitRecovery: async () => commandReceipt }} />);
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
    const snapshot = sessionSnapshot("needsRecovery");
    snapshot.state.domain.session = { kind: "awaitingVerification", user_id: "@u:example.invalid", homeserver: "https://example.invalid", device_id: "D", gate: { methods: ["existingDeviceSas"], account_kind: "existingIdentity" } };
    const startOwnUserSas = vi.fn(async () => commandReceipt);
    render(<SessionVerificationGate snapshot={snapshot} onReceipt={async () => undefined} onSignOut={() => undefined} operations={{ startOwnUserSas, submitRecovery: async () => commandReceipt }} />);

    fireEvent.click(screen.getByRole("button", { name: "Verify with another device" }));

    expect(screen.getByRole("region", { name: "Try device verification?" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Use recovery key" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Try device verification anyway" }));
    expect(startOwnUserSas).toHaveBeenCalledTimes(1);
  });

  test("requires consequence confirmation before starting remote-first device cleanup", async () => {
    const snapshot = sessionSnapshot("needsRecovery");
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
    const startDeviceCleanup = vi.fn(async () => commandReceipt);
    const onReceipt = vi.fn(async () => undefined);

    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onReceipt={onReceipt}
        onSignOut={() => undefined}
        operations={{
          startOwnUserSas: async () => commandReceipt,
          submitRecovery: async () => commandReceipt,
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
    expect(onReceipt).toHaveBeenCalledWith(commandReceipt);
  });

  test("submits legacy UIA password through the IME-safe cleanup form", async () => {
    const snapshot = sessionSnapshot("needsRecovery");
    setCleanupSurfaceSession(snapshot);
    snapshot.state.domain.device_cleanup = {
      kind: "awaitingUia",
      request_id: 371,
      flow_id: 41
    };
    const submitDeviceCleanupUia = vi.fn(async () => commandReceipt);
    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onReceipt={async () => undefined}
        onSignOut={() => undefined}
        operations={{
          startOwnUserSas: async () => commandReceipt,
          submitRecovery: async () => commandReceipt,
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
    const snapshot = sessionSnapshot("needsRecovery");
    setCleanupSurfaceSession(snapshot);
    snapshot.state.domain.device_cleanup = {
      kind: "remoteFailed",
      request_id: 372,
      auth_mode: "legacy",
      failureKind: "network"
    };
    const startDeviceCleanup = vi.fn(async () => commandReceipt);
    const eraseLocalDataAnyway = vi.fn(async () => commandReceipt);
    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onReceipt={async () => undefined}
        onSignOut={() => undefined}
        operations={{
          startOwnUserSas: async () => commandReceipt,
          submitRecovery: async () => commandReceipt,
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
    const snapshot = sessionSnapshot("needsRecovery");
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
        onReceipt={async () => undefined}
        onSignOut={() => undefined}
      />
    );

    expect(screen.queryByLabelText("Account password")).toBeNull();
    expect(screen.getByRole("button", { name: "Retry removing device" })).toBeTruthy();
  });

  test("shows progress without duplicate cleanup actions while remote removal is pending", async () => {
    const snapshot = sessionSnapshot("needsRecovery");
    setCleanupSurfaceSession(snapshot);
    snapshot.state.domain.device_cleanup = {
      kind: "removingRemote",
      request_id: 374,
      auth_mode: "legacy"
    };
    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onReceipt={async () => undefined}
        onSignOut={() => undefined}
      />
    );

    expect(screen.getByText("Removing this device from your Matrix account…")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Retry removing device" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Erase local data anyway…" })).toBeNull();
  });

  test("does not offer destructive cleanup while a recovery retry is verifying", async () => {
    const snapshot = sessionSnapshot("needsRecovery");
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
        onReceipt={async () => undefined}
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
    const snapshot = sessionSnapshot("needsRecovery");
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
        onReceipt={async () => undefined}
        onSignOut={() => undefined}
        onStartWindowDrag={onStartWindowDrag}
        operations={{
          startOwnUserSas: async () => commandReceipt,
          submitRecovery: async () => commandReceipt,
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
      await createDesktopApiFixture().getSnapshot(),
      { kind: "checking" }
    );

    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onReceipt={async () => undefined}
        onSignOut={() => undefined}
        operations={secureBackupOperations(snapshot)}
      />
    );

    expect(screen.getByRole("main", { name: "Secure backup required" })).toBeTruthy();
    expect(
      screen.getByRole("heading", {
        name: "Checking the backup of your decryption keys on the homeserver…"
      })
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Create room" })).toBeNull();
  });

  test("masks and clears the secure-backup recovery key after submission", async () => {
    const snapshot = secureBackupSnapshot(
      await createDesktopApiFixture().getSnapshot(),
      { kind: "existingBackupNeedsRecovery", failure: "invalidRecoveryKey" }
    );
    const recoverSecureBackup = vi.fn(async () => commandReceipt);

    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onReceipt={async () => undefined}
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
      await createDesktopApiFixture().getSnapshot(),
      { kind: "secureStorageIncomplete" }
    );

    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onReceipt={async () => undefined}
        onSignOut={() => undefined}
        operations={secureBackupOperations(snapshot)}
      />
    );

    expect(screen.getByLabelText("Secure backup recovery key")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Recover secure backup" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Set up secure backup" })).toBeNull();
  });

  test("creates the secure backup without choosing a file destination and clears the passphrase", async () => {
    const snapshot = secureBackupSnapshot(
      await createDesktopApiFixture().getSnapshot(),
      { kind: "setupRequired" }
    );
    const bootstrapSecureBackup = vi.fn(async () => commandReceipt);

    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onReceipt={async () => undefined}
        onSignOut={() => undefined}
        operations={secureBackupOperations(snapshot, { bootstrapSecureBackup })}
      />
    );

    expect(screen.getByText(/shown on screen/i)).toBeTruthy();
    expect(screen.queryByRole("button", { name: /destination/i })).toBeNull();
    expect(screen.queryByText(/destination/i)).toBeNull();
    const passphrase = screen.getByLabelText("Secure backup passphrase") as HTMLInputElement;
    fireEvent.change(passphrase, { target: { value: "synthetic-passphrase" } });
    fireEvent.click(screen.getByRole("button", { name: "Set up secure backup" }));

    await vi.waitFor(() =>
      expect(bootstrapSecureBackup).toHaveBeenCalledWith("synthetic-passphrase", {
        kind: "initialSetup"
      })
    );
    expect(passphrase.value).toBe("");
  });

  test("requires explicit confirmation before re-enabling an account-wide disabled backup", async () => {
    const snapshot = secureBackupSnapshot(
      await createDesktopApiFixture().getSnapshot(),
      { kind: "explicitlyDisabledRequiresSetup" }
    );
    const bootstrapSecureBackup = vi.fn(async () => commandReceipt);

    const renderGate = (nextSnapshot = snapshot) => (
      <SessionVerificationGate
        snapshot={nextSnapshot}
        onReceipt={async () => undefined}
        onSignOut={() => undefined}
        operations={secureBackupOperations(nextSnapshot, { bootstrapSecureBackup })}
      />
    );
    const { rerender } = render(renderGate());

    expect(screen.getByText(/other Matrix clients/i)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Re-enable secure backup" }));
    let dialog = screen.getByRole("region", { name: "Re-enable secure backup" });
    expect(dialog).toBeTruthy();
    expect(bootstrapSecureBackup).not.toHaveBeenCalled();
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("region", { name: "Re-enable secure backup" })).toBeNull();
    expect(bootstrapSecureBackup).not.toHaveBeenCalled();

    const changedGate = secureBackupSnapshot(structuredClone(snapshot), { kind: "setupRequired" });
    rerender(renderGate(changedGate));
    rerender(renderGate(snapshot));
    expect(screen.queryByRole("region", { name: "Re-enable secure backup" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Re-enable secure backup" }));
    dialog = screen.getByRole("region", { name: "Re-enable secure backup" });
    expect(within(dialog).queryByRole("button", { name: /destination/i })).toBeNull();
    const passphrase = within(dialog).getByLabelText(
      "Secure backup passphrase"
    ) as HTMLInputElement;
    fireEvent.change(passphrase, { target: { value: "reenable-passphrase" } });
    fireEvent.click(screen.getByRole("button", { name: "Confirm re-enable" }));

    await vi.waitFor(() =>
      expect(bootstrapSecureBackup).toHaveBeenCalledWith("reenable-passphrase", {
        kind: "reenable",
        confirmed: true
      })
    );
    expect(passphrase.value).toBe("");
  });

  function revealSnapshot(
    snapshot: DesktopSnapshot,
    delivery: RecoveryKeyDeliveryState = { kind: "notWritten" },
    confirmationFailed = false
  ): DesktopSnapshot {
    const revealed = secureBackupSnapshot(snapshot, { kind: "recoveryKeyDeliveryRequired" });
    revealed.state.domain.e2ee_trust.key_management.secure_backup_setup = {
      kind: "recoveryKeyReady",
      request_id: 41,
      recovery_key: SYNTHETIC_RECOVERY_KEY,
      delivery,
      confirmation_failed: confirmationFailed
    };
    return revealed;
  }

  test("shows the recovery key on screen; copy and save never advance the gate", async () => {
    const snapshot = revealSnapshot(await createDesktopApiFixture().getSnapshot());
    const bootstrapSecureBackup = vi.fn(async () => commandReceipt);
    const copyRecoveryKey = vi.fn(async () => undefined);
    const saveSecureBackupRecoveryKey = vi.fn(async () => commandReceipt);
    const confirmSecureBackupRecoveryKeySaved = vi.fn(async () => commandReceipt);
    const renderGate = (nextSnapshot: DesktopSnapshot) => (
      <SessionVerificationGate
        snapshot={nextSnapshot}
        onReceipt={async () => undefined}
        onSignOut={() => undefined}
        operations={secureBackupOperations(nextSnapshot, {
          bootstrapSecureBackup,
          copyRecoveryKey,
          saveSecureBackupRecoveryKey,
          confirmSecureBackupRecoveryKeySaved
        })}
      />
    );
    const { rerender } = render(renderGate(snapshot));

    const reveal = screen.getByRole("region", { name: "Your recovery key" });
    const keyText = within(reveal).getByText(SYNTHETIC_RECOVERY_KEY);
    expect(keyText.tagName).toBe("CODE");
    expect(keyText.className).toContain("recovery-key-value");
    // The setup form is replaced while a key is revealed.
    expect(screen.queryByRole("button", { name: "Set up secure backup" })).toBeNull();
    expect(screen.queryByLabelText("Secure backup passphrase")).toBeNull();

    fireEvent.click(within(reveal).getByRole("button", { name: "Copy" }));
    await vi.waitFor(() => expect(copyRecoveryKey).toHaveBeenCalledWith(SYNTHETIC_RECOVERY_KEY));
    await vi.waitFor(() => expect(within(reveal).getByText("Copied")).toBeTruthy());

    fireEvent.click(within(reveal).getByRole("button", { name: "Save to file…" }));
    await vi.waitFor(() => expect(saveSecureBackupRecoveryKey).toHaveBeenCalledWith(41));

    expect(confirmSecureBackupRecoveryKeySaved).not.toHaveBeenCalled();
    expect(bootstrapSecureBackup).not.toHaveBeenCalled();
    expect(screen.getByText(SYNTHETIC_RECOVERY_KEY)).toBeTruthy();

    // A saved file is reported, but the key stays revealed until confirmation.
    rerender(renderGate(revealSnapshot(structuredClone(snapshot), { kind: "written" })));
    expect(screen.getByText("Recovery key saved to file.")).toBeTruthy();
    expect(screen.getByText(SYNTHETIC_RECOVERY_KEY)).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "I saved the recovery key" }));
    await vi.waitFor(() => expect(confirmSecureBackupRecoveryKeySaved).toHaveBeenCalledWith(41));

    // Rust drops the key on confirmation; the gate renders nothing of it.
    const confirmed = secureBackupSnapshot(structuredClone(snapshot), { kind: "checking" });
    confirmed.state.domain.e2ee_trust.key_management.secure_backup_setup = {
      kind: "enabled",
      request_id: 41
    };
    rerender(renderGate(confirmed));
    expect(screen.queryByText(SYNTHETIC_RECOVERY_KEY)).toBeNull();
    expect(screen.queryByRole("region", { name: "Your recovery key" })).toBeNull();
  });

  test("explains a failed saved confirmation when Rust restores the reveal", async () => {
    const base = await createDesktopApiFixture().getSnapshot();
    const renderGate = (nextSnapshot: DesktopSnapshot) => (
      <SessionVerificationGate
        snapshot={nextSnapshot}
        onReceipt={async () => undefined}
        onSignOut={() => undefined}
        operations={secureBackupOperations(nextSnapshot, {})}
      />
    );
    const { rerender } = render(renderGate(revealSnapshot(structuredClone(base))));
    expect(screen.queryByText(/confirmation could not be saved/)).toBeNull();

    rerender(
      renderGate(revealSnapshot(structuredClone(base), { kind: "written" }, true))
    );
    const reveal = screen.getByRole("region", { name: "Your recovery key" });
    expect(within(reveal).getByRole("alert").textContent).toContain(
      "Your confirmation could not be saved"
    );
    expect(within(reveal).getByText(SYNTHETIC_RECOVERY_KEY)).toBeTruthy();
    expect(screen.getByRole("button", { name: "I saved the recovery key" })).toBeTruthy();
  });

  test("keeps the reveal while the gate briefly leaves delivery-required", async () => {
    const snapshot = revealSnapshot(await createDesktopApiFixture().getSnapshot());
    snapshot.state.domain.secure_backup_gate = { kind: "checking" };

    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onReceipt={async () => undefined}
        onSignOut={() => undefined}
        operations={secureBackupOperations(snapshot, {})}
      />
    );

    const reveal = screen.getByRole("region", { name: "Your recovery key" });
    expect(within(reveal).getByText(SYNTHETIC_RECOVERY_KEY)).toBeTruthy();
    expect(screen.getByRole("button", { name: "I saved the recovery key" })).toBeTruthy();
    // The lost-key reset is never offered while the key is still held.
    expect(screen.queryByRole("button", { name: "Create new recovery key" })).toBeNull();
  });

  test("reports copy and save-to-file failures without leaving the reveal", async () => {
    const snapshot = revealSnapshot(await createDesktopApiFixture().getSnapshot());
    const copyRecoveryKey = vi.fn(async () => {
      throw new Error("clipboard unavailable");
    });
    const confirmSecureBackupRecoveryKeySaved = vi.fn(async () => commandReceipt);
    const renderGate = (nextSnapshot: DesktopSnapshot) => (
      <SessionVerificationGate
        snapshot={nextSnapshot}
        onReceipt={async () => undefined}
        onSignOut={() => undefined}
        operations={secureBackupOperations(nextSnapshot, {
          copyRecoveryKey,
          confirmSecureBackupRecoveryKeySaved
        })}
      />
    );
    const { rerender } = render(renderGate(snapshot));

    fireEvent.click(screen.getByRole("button", { name: "Copy" }));
    await vi.waitFor(() =>
      expect(screen.getByRole("alert").textContent).toContain("Could not copy")
    );

    rerender(renderGate(revealSnapshot(structuredClone(snapshot), { kind: "writeFailed" })));
    expect(
      screen.getAllByRole("alert").some((alert) =>
        alert.textContent?.includes("Could not save the recovery key to a file")
      )
    ).toBe(true);
    expect(screen.getByText(SYNTHETIC_RECOVERY_KEY)).toBeTruthy();
    expect(confirmSecureBackupRecoveryKeySaved).not.toHaveBeenCalled();
  });

  test("replaces a lost, unconfirmed recovery key only after confirming a NEW key", async () => {
    const snapshot = secureBackupSnapshot(
      await createDesktopApiFixture().getSnapshot(),
      { kind: "recoveryKeyDeliveryRequired" }
    );
    const bootstrapSecureBackup = vi.fn(async () => commandReceipt);

    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onReceipt={async () => undefined}
        onSignOut={() => undefined}
        operations={secureBackupOperations(snapshot, { bootstrapSecureBackup })}
      />
    );

    // The previous key is never re-shown or re-exported.
    expect(screen.getByText(/cannot be shown again/i)).toBeTruthy();
    expect(screen.queryByLabelText("Secure backup passphrase")).toBeNull();
    expect(screen.queryByRole("button", { name: /show recovery key/i })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Create new recovery key" }));
    let dialog = screen.getByRole("region", { name: "Create a new recovery key?" });
    expect(within(dialog).getByText(/previous recovery key .*stop working/i)).toBeTruthy();
    expect(bootstrapSecureBackup).not.toHaveBeenCalled();

    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("region", { name: "Create a new recovery key?" })).toBeNull();
    expect(bootstrapSecureBackup).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Create new recovery key" }));
    dialog = screen.getByRole("region", { name: "Create a new recovery key?" });
    fireEvent.click(
      within(dialog).getByRole("button", { name: "Yes, create a new recovery key" })
    );
    await vi.waitFor(() =>
      expect(bootstrapSecureBackup).toHaveBeenCalledWith(null, {
        kind: "resetRecoveryKey",
        confirmed: true
      })
    );
    expect(bootstrapSecureBackup).toHaveBeenCalledTimes(1);
  });

  function bootstrapSnapshot(
    snapshot: DesktopSnapshot,
    phase: "awaiting" | "revealed",
    confirmationFailed = false
  ): DesktopSnapshot {
    const identity = {
      homeserver: "https://example.invalid",
      user_id: "@new:example.invalid",
      device_id: "NEWDEVICE",
      gate: { methods: ["bootstrap" as const], account_kind: "newIdentity" as const, failureKind: null }
    };
    if (phase === "awaiting") {
      snapshot.state.domain.session = { kind: "awaitingVerification", ...identity };
      return snapshot;
    }
    snapshot.state.domain.session = { kind: "awaitingBootstrapConfirmation", ...identity, flow_id: 41 };
    snapshot.state.domain.e2ee_trust.key_management.secure_backup_setup = {
      kind: "recoveryKeyReady",
      request_id: 41,
      recovery_key: SYNTHETIC_RECOVERY_KEY,
      delivery: { kind: "notWritten" },
      confirmation_failed: confirmationFailed
    };
    return snapshot;
  }

  test("identity bootstrap needs no file destination and reveals the key until confirmed", async () => {
    const fixture = createDesktopApiFixture();
    const base = await fixture.getSnapshot();
    const copyRecoveryKey = vi.fn(async () => undefined);
    const saveSecureBackupRecoveryKey = vi.fn(async () => commandReceipt);
    const renderGate = (nextSnapshot: DesktopSnapshot) => (
      <SessionVerificationGate
        desktopApi={fixture}
        snapshot={nextSnapshot}
        onReceipt={async () => undefined}
        onSignOut={() => undefined}
        operations={secureBackupOperations(nextSnapshot, {
          copyRecoveryKey,
          saveSecureBackupRecoveryKey
        })}
      />
    );
    const { rerender } = render(renderGate(bootstrapSnapshot(structuredClone(base), "awaiting")));

    expect(screen.queryByLabelText(/destination/i)).toBeNull();
    const passphrase = screen.getByLabelText("Backup passphrase") as HTMLInputElement;
    fireEvent.change(passphrase, { target: { value: "synthetic-passphrase" } });
    fireEvent.click(screen.getByRole("button", { name: "Create secure backup" }));
    await vi.waitFor(() =>
      expect(fixture.ipc.invocationsOf("start_session_bootstrap")[0]?.args).toEqual({
        passphrase: "[REDACTED]"
      })
    );
    expect(passphrase.value).toBe("");

    rerender(renderGate(bootstrapSnapshot(structuredClone(base), "revealed")));
    expect(screen.getByRole("heading", { name: "Save your recovery key" })).toBeTruthy();
    const reveal = screen.getByRole("region", { name: "Your recovery key" });
    expect(within(reveal).getByText(SYNTHETIC_RECOVERY_KEY).tagName).toBe("CODE");
    expect(screen.queryByRole("button", { name: "Create secure backup" })).toBeNull();

    fireEvent.click(within(reveal).getByRole("button", { name: "Copy" }));
    await vi.waitFor(() => expect(copyRecoveryKey).toHaveBeenCalledWith(SYNTHETIC_RECOVERY_KEY));
    fireEvent.click(within(reveal).getByRole("button", { name: "Save to file…" }));
    await vi.waitFor(() => expect(saveSecureBackupRecoveryKey).toHaveBeenCalledWith(41));
    expect(fixture.ipc.invocationsOf("confirm_session_bootstrap_saved")).toHaveLength(0);
    expect(fixture.ipc.invocationsOf("confirm_secure_backup_recovery_key_saved")).toHaveLength(0);

    const confirm = within(reveal).getByRole("button", {
      name: "I saved the recovery key"
    }) as HTMLButtonElement;
    await vi.waitFor(() => expect(confirm.disabled).toBe(false));
    fireEvent.click(confirm);
    await vi.waitFor(() =>
      expect(fixture.ipc.invocationsOf("confirm_session_bootstrap_saved")[0]?.args).toEqual({
        flowId: 41
      })
    );
    expect(fixture.ipc.invocationsOf("confirm_secure_backup_recovery_key_saved")).toHaveLength(0);
  });

  test("explains a failed bootstrap confirmation while keeping the key revealed", async () => {
    const base = await createDesktopApiFixture().getSnapshot();
    render(
      <SessionVerificationGate
        snapshot={bootstrapSnapshot(base, "revealed", true)}
        onReceipt={async () => undefined}
        onSignOut={() => undefined}
        operations={secureBackupOperations(base)}
      />
    );
    expect(screen.getByText(SYNTHETIC_RECOVERY_KEY)).toBeTruthy();
    expect(
      screen.getByText(/Your confirmation could not be saved/)
    ).toBeTruthy();
  });

  test("renders typed upload progress without exposing a raw count or error", async () => {
    const snapshot = secureBackupSnapshot(
      await createDesktopApiFixture().getSnapshot(),
      { kind: "uploadingExistingKeys", pending: "eleven_to_one_hundred" }
    );

    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onReceipt={async () => undefined}
        onSignOut={() => undefined}
        operations={secureBackupOperations(snapshot)}
      />
    );

    expect(screen.getByText("Uploading existing encrypted keys: 11–100 remaining.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Retry secure backup" })).toBeNull();
  });

  test("explains a structured secure backup failure without raw DTO or SDK text", async () => {
    const cases: Array<{
      label: string;
      failure: SecureBackupGateFailureKind;
      detail: SecureBackupFailureDetail;
      explanation: string;
      action: string;
    }> = [
      {
        label: "no response",
        failure: "network",
        detail: { stage: "inspectServerTrust", transport: "noResponse", retryable: true },
        explanation:
          "The homeserver could not be reached, so this secure backup request never received a response.",
        action: "Check your connection, then retry."
      },
      {
        label: "timeout",
        failure: "timeout",
        detail: { stage: "inspectionDeadline", transport: "timeout", retryable: true },
        explanation: "The homeserver did not answer this secure backup request in time.",
        action: "Check your connection, then retry."
      },
      {
        label: "unauthorized 401",
        failure: "unauthorized",
        detail: {
          stage: "inspectServerTrust",
          transport: "httpResponse",
          httpStatus: 401,
          matrixErrorKind: "unknownToken",
          retryable: false
        },
        explanation: "The homeserver rejected the sign-in for secure backup.",
        action: "Sign out and sign in again."
      },
      {
        label: "forbidden 403",
        failure: "forbidden",
        detail: {
          stage: "inspectServerTrust",
          transport: "httpResponse",
          httpStatus: 403,
          matrixErrorKind: "forbidden",
          retryable: false
        },
        explanation: "This account is not allowed to use secure backup.",
        action: "Ask your homeserver administrator to check secure backup."
      },
      {
        label: "not found 404",
        failure: "serverResponse",
        detail: {
          stage: "inspectServerTrust",
          transport: "httpResponse",
          httpStatus: 404,
          matrixErrorKind: "notFound",
          retryable: false
        },
        explanation: "The homeserver answered this secure backup request with an error (HTTP 404).",
        action: "Ask your homeserver administrator to check secure backup."
      },
      {
        label: "rate limited 429",
        failure: "rateLimited",
        detail: {
          stage: "inspectServerTrust",
          transport: "httpResponse",
          httpStatus: 429,
          matrixErrorKind: "limitExceeded",
          retryable: true
        },
        explanation: "Secure backup requests are being limited. Try again later.",
        action: "Wait a few minutes, then retry."
      },
      {
        label: "server error 503",
        failure: "serverResponse",
        detail: {
          stage: "inspectServerTrust",
          transport: "httpResponse",
          httpStatus: 503,
          matrixErrorKind: "unknown",
          retryable: true
        },
        explanation: "The homeserver answered this secure backup request with an error (HTTP 503).",
        action: "Try again in a few minutes."
      },
      {
        label: "local preparation",
        failure: "sdk",
        detail: { stage: "crossSigningStatus", transport: "local", retryable: false },
        explanation: "Secure backup could not be prepared on this device.",
        action: "Retry; if it keeps failing, sign out and sign in again."
      }
    ];

    for (const { label, failure, detail, explanation, action } of cases) {
      const snapshot = secureBackupSnapshot(await createDesktopApiFixture().getSnapshot(), {
        kind: "blockedFailed",
        failure,
        detail
      });
      const { unmount } = render(
        <SessionVerificationGate
          snapshot={snapshot}
          onReceipt={async () => undefined}
          onSignOut={() => undefined}
        />
      );
      expect(screen.getByRole("alert").textContent, label).toBe(explanation);
      expect(screen.getByText(action), label).toBeTruthy();
      const rendered = document.body.textContent ?? "";
      for (const raw of [
        "synthetic",
        "inspectServerTrust",
        "httpResponse",
        "unknownToken",
        "matrixErrorKind",
        "httpStatus",
        "retryable",
        "blockedFailed"
      ]) {
        expect(rendered, `${label} leaked ${raw}`).not.toContain(raw);
      }
      unmount();
    }
  });

  test("falls back to the coarse failure copy when Rust records no detail", async () => {
    const snapshot = secureBackupSnapshot(await createDesktopApiFixture().getSnapshot(), {
      kind: "blockedFailed",
      failure: "rateLimited"
    });
    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onReceipt={async () => undefined}
        onSignOut={() => undefined}
      />
    );
    expect(screen.getByRole("alert").textContent).toContain("limited");
  });

  test("shows typed failure, supports retry, and exposes diagnostics without raw errors", async () => {
    const snapshot = secureBackupSnapshot(
      await createDesktopApiFixture().getSnapshot(),
      { kind: "blockedFailed", failure: "rateLimited" }
    );
    const retrySecureBackupInspection = vi.fn(async () => commandReceipt);
    const openSecureBackupDiagnostics = vi.fn(async () => undefined);

    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onReceipt={async () => undefined}
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
    const base = sessionSnapshot("needsRecovery");
    const renderGate = (snapshot: DesktopSnapshot) => renderToStaticMarkup(
      <SessionVerificationGate snapshot={snapshot} onReceipt={async () => undefined} onSignOut={() => undefined} />
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
