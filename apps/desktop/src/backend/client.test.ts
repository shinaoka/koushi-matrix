import { invoke } from "@tauri-apps/api/core";
import { afterEach, describe, expect, test, vi } from "vitest";

import { createDesktopApi } from "./client";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async () => ({ ok: true }))
}));

describe("TauriDesktopApi", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.clearAllMocks();
  });

  test("passes settings patches to the Rust update_settings command", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = createDesktopApi();
    await api.updateSettings({ appearance: { theme: "dark" } });

    expect(invoke).toHaveBeenCalledWith("update_settings", {
      patch: { appearance: { theme: "dark" } }
    });
  });

  test("passes composer resolver facts to the Rust resolver command", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = createDesktopApi();
    await api.resolveComposerKeyAction(
      "main",
      {
        key: "enter",
        modifiers: { ctrl: false, meta: true, shift: false, alt: false },
        is_composing: false
      },
      { autocomplete_open: false, send_enabled: true }
    );

    expect(invoke).toHaveBeenCalledWith("resolve_composer_key_action", {
      surface: "main",
      keyEvent: {
        key: "enter",
        modifiers: { ctrl: false, meta: true, shift: false, alt: false },
        is_composing: false
      },
      autocompleteOpen: false,
      sendEnabled: true
    });
  });

  test("passes E2EE trust actions to Rust-owned commands", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });

    const api = createDesktopApi();
    await api.bootstrapCrossSigning();
    await api.enableKeyBackup();
    await api.acceptVerification(41);
    await api.confirmSasVerification(42);
    await api.cancelVerification(43);
    await api.resetIdentity();
    await api.submitIdentityResetPassword(44, "synthetic-password");
    await api.submitIdentityResetOAuth(45);

    expect(invoke).toHaveBeenCalledWith("bootstrap_cross_signing");
    expect(invoke).toHaveBeenCalledWith("enable_key_backup");
    expect(invoke).toHaveBeenCalledWith("accept_verification", { flowId: 41 });
    expect(invoke).toHaveBeenCalledWith("confirm_sas_verification", { flowId: 42 });
    expect(invoke).toHaveBeenCalledWith("cancel_verification", { flowId: 43 });
    expect(invoke).toHaveBeenCalledWith("reset_identity");
    expect(invoke).toHaveBeenCalledWith("submit_identity_reset_password", {
      flowId: 44,
      password: "synthetic-password"
    });
    expect(invoke).toHaveBeenCalledWith("submit_identity_reset_oauth", { flowId: 45 });
  });
});
