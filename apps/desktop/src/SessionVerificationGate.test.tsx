// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import { SessionVerificationGate } from "./App";
import { createBrowserFakeApi } from "./backend/browserFakeApi";

describe("SessionVerificationGate interactions", () => {
  afterEach(cleanup);
  test("admits SAS and recovery independently and blocks duplicate promise construction", async () => {
    const snapshot = await createBrowserFakeApi({ session: "needsRecovery" }).getSnapshot();
    snapshot.state.domain.session = { kind: "awaitingVerification", user_id: "@u:example.invalid", homeserver: "https://example.invalid", device_id: "D", gate: { methods: ["existingDeviceSas", "recoveryKey"], account_kind: "existingIdentity" } };
    let releaseSas!: (value: typeof snapshot) => void;
    const sasPromise = new Promise<typeof snapshot>((resolve) => { releaseSas = resolve; });
    const startOwnUserSas = vi.fn(() => sasPromise);
    const submitRecovery = vi.fn(async () => snapshot);
    render(<SessionVerificationGate snapshot={snapshot} onSnapshot={() => undefined} onSignOut={() => undefined} operations={{ startOwnUserSas, submitRecovery }} />);

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
    render(<SessionVerificationGate snapshot={snapshot} onSnapshot={() => undefined} onSignOut={() => undefined} operations={{ startOwnUserSas, submitRecovery: async () => snapshot }} />);
    const button = screen.getByRole("button", { name: "Verify with another device" });
    fireEvent.click(button);
    await vi.waitFor(() => expect((button as HTMLButtonElement).disabled).toBe(false));
    expect(screen.getByRole("alert").textContent).toContain("Verification command failed");
    fireEvent.click(button);
    await vi.waitFor(() => expect(startOwnUserSas).toHaveBeenCalledTimes(2));
  });
});
