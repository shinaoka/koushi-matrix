// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import { SessionVerificationGate } from "./App";
import { createBrowserFakeApi } from "./backend/browserFakeApi";
import type { ProvisionalPhase } from "./domain/types";

const provisionalPhaseCases: Array<[ProvisionalPhase, string, boolean]> = [
  ["checkingTrust", "Checking device trust…", false],
  ["discoveringMethods", "Discovering verification methods…", true],
  [{ recheckingTrust: { failureKind: "timeout" } }, "Finishing sign-in…", true],
];

describe("SessionVerificationGate interactions", () => {
  afterEach(cleanup);

  test.each(provisionalPhaseCases)("renders provisional phase %j with phase-specific retry availability", async (phase, copy, retryVisible) => {
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
        operations={{ startOwnUserSas: async () => snapshot, submitRecovery: async () => snapshot, retryCurrentDeviceTrustDiscovery: async () => snapshot }}
      />
    );

    expect(screen.getByText(copy)).toBeTruthy();
    const retry = screen.queryByRole("button", { name: "Retry" });
    expect(Boolean(retry)).toBe(retryVisible);
  });

  test("blocks duplicate retry promise construction while discovery is pending", async () => {
    const snapshot = await createBrowserFakeApi({ session: "needsRecovery" }).getSnapshot();
    snapshot.state.domain.session = {
      kind: "provisional",
      user_id: "@u:example.invalid",
      homeserver: "https://example.invalid",
      device_id: "D",
      phase: "discoveringMethods",
    };
    let releaseRetry!: (value: typeof snapshot) => void;
    const retryPromise = new Promise<typeof snapshot>((resolve) => { releaseRetry = resolve; });
    const retryCurrentDeviceTrustDiscovery = vi.fn(() => retryPromise);
    render(
      <SessionVerificationGate
        snapshot={snapshot}
        onSnapshot={() => undefined}
        onSignOut={() => undefined}
        operations={{
          startOwnUserSas: async () => snapshot,
          submitRecovery: async () => snapshot,
          retryCurrentDeviceTrustDiscovery,
        }}
      />
    );

    const retry = screen.getByRole("button", { name: "Retry" });
    fireEvent.click(retry);
    fireEvent.click(retry);
    expect(retryCurrentDeviceTrustDiscovery).toHaveBeenCalledTimes(1);
    releaseRetry(snapshot);
  });

  test("admits SAS and recovery independently and blocks duplicate promise construction", async () => {
    const snapshot = await createBrowserFakeApi({ session: "needsRecovery" }).getSnapshot();
    snapshot.state.domain.session = { kind: "awaitingVerification", user_id: "@u:example.invalid", homeserver: "https://example.invalid", device_id: "D", gate: { methods: ["existingDeviceSas", "recoveryKey"], account_kind: "existingIdentity" } };
    let releaseSas!: (value: typeof snapshot) => void;
    const sasPromise = new Promise<typeof snapshot>((resolve) => { releaseSas = resolve; });
    const startOwnUserSas = vi.fn(() => sasPromise);
    const submitRecovery = vi.fn(async () => snapshot);
    render(<SessionVerificationGate snapshot={snapshot} onSnapshot={() => undefined} onSignOut={() => undefined} operations={{ startOwnUserSas, submitRecovery, retryCurrentDeviceTrustDiscovery: async () => snapshot }} />);

    const sas = screen.getByRole("button", { name: "Verify with another device" });
    fireEvent.click(sas);
    fireEvent.click(sas);
    expect(startOwnUserSas).toHaveBeenCalledTimes(1);

    fireEvent.change(screen.getByLabelText("Recovery secret"), { target: { value: "fixture-secret" } });
    fireEvent.click(screen.getByRole("button", { name: "Recover" }));
    expect(submitRecovery).toHaveBeenCalledTimes(1);
    releaseSas(snapshot);
  });

  test("rejected operation settles and permits a later attempt", async () => {
    const snapshot = await createBrowserFakeApi({ session: "needsRecovery" }).getSnapshot();
    snapshot.state.domain.session = { kind: "awaitingVerification", user_id: "@u:example.invalid", homeserver: "https://example.invalid", device_id: "D", gate: { methods: ["existingDeviceSas"], account_kind: "existingIdentity" } };
    const startOwnUserSas = vi.fn().mockRejectedValueOnce(new Error("rejected")).mockResolvedValue(snapshot);
    render(<SessionVerificationGate snapshot={snapshot} onSnapshot={() => undefined} onSignOut={() => undefined} operations={{ startOwnUserSas, submitRecovery: async () => snapshot, retryCurrentDeviceTrustDiscovery: async () => snapshot }} />);
    const button = screen.getByRole("button", { name: "Verify with another device" });
    fireEvent.click(button);
    await vi.waitFor(() => expect((button as HTMLButtonElement).disabled).toBe(false));
    expect(screen.getByRole("alert").textContent).toContain("Verification command failed");
    fireEvent.click(button);
    await vi.waitFor(() => expect(startOwnUserSas).toHaveBeenCalledTimes(2));
  });
});
