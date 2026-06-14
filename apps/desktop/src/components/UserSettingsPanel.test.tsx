import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, test } from "vitest";

import { UserSettingsPanel } from "./UserSettingsPanel";
import type { E2eeTrustState } from "../domain/types";

describe("UserSettingsPanel", () => {
  const settings = {
    values: {
      locale: { language_tag: null, text_direction: "auto" },
      appearance: { theme: "dark" },
      typography: { font: "system", emoji: "system" },
      keyboard: { composer_send_shortcut: "enter" }
    },
    persistence: { kind: "idle" }
  } as const;
  const e2eeTrust: E2eeTrustState = {
    verification: {
      kind: "sasPresented",
      request_id: 7,
      target: {
        user_id: "redacted-target-user",
        device_id: "TARGETDEVICE"
      },
      emojis: [
        { symbol: "🐶", description: "dog" },
        { symbol: "🐱", description: "cat" }
      ]
    },
    cross_signing: { kind: "trusted" },
    key_backup: { kind: "enabled", version: "backup-version" },
    identity_reset: { kind: "idle" },
    devices: [
      {
        user_id: "@demo-user:example.invalid",
        device_id: "FAKEDEVICE",
        trust_level: "verified"
      },
      {
        user_id: "redacted-target-user",
        device_id: "TARGETDEVICE",
        trust_level: "unverified"
      }
    ]
  };
  const idleE2eeTrust: E2eeTrustState = {
    verification: { kind: "idle" },
    cross_signing: { kind: "unknown" },
    key_backup: { kind: "unknown" },
    identity_reset: { kind: "idle" },
    devices: []
  };
  const handlers = {
    onAcceptVerification: () => undefined,
    onBootstrapCrossSigning: () => undefined,
    onCancelVerification: () => undefined,
    onConfirmSasVerification: () => undefined,
    onEnableKeyBackup: () => undefined,
    onOpenKeyboardSettings: () => undefined,
    onResetIdentity: () => undefined,
    onSubmitIdentityResetOAuth: () => undefined,
    onSubmitIdentityResetPassword: () => undefined,
    onSwitchAccount: () => undefined,
    onUpdateSettings: () => undefined
  };

  test("renders account switch entries and keyboard settings access", () => {
    const markup = renderToStaticMarkup(
      <UserSettingsPanel
        currentSession={{
          homeserver: "https://matrix.org",
          user_id: "@demo-user:example.invalid",
          device_id: "FAKEDEVICE"
        }}
        e2eeTrust={e2eeTrust}
        savedSessions={[
          {
            homeserver: "https://matrix.org",
            user_id: "@demo-user:example.invalid",
            device_id: "FAKEDEVICE"
          },
          {
            homeserver: "https://matrix.org",
            user_id: "@second-user:example.invalid",
            device_id: "SECONDDEVICE"
          }
        ]}
        settings={settings}
        {...handlers}
      />
    );

    expect(markup).toContain("User settings");
    expect(markup).toContain("@demo-user:example.invalid");
    expect(markup).toContain("@second-user:example.invalid");
    expect(markup).toContain("Current");
    expect(markup).toContain("Switch");
    expect(markup).toContain("Keyboard");
    expect(markup).toContain("Session");
    expect(markup).toContain("Homeserver");
    expect(markup).toContain("Device");
    expect(markup).toContain("Local store");
    expect(markup).toContain("Appearance");
    expect(markup).toContain("Dark");
    expect(markup).toContain("Typography");
    expect(markup).toContain("UI font");
    expect(markup).toContain("Emoji font");
    expect(markup).toContain("Inter");
    expect(markup).toContain("Twemoji COLR");
    expect(markup).toContain('aria-pressed="true"');
    expect(markup).toContain("Separate encrypted namespace");
    expect(markup).toContain("OS credential store");
    expect(markup).toContain("Encryption");
    expect(markup).toContain("Device verification");
    expect(markup).toContain("Compare emoji");
    expect(markup).toContain("🐶");
    expect(markup).toContain("Key backup");
    expect(markup).toContain("Device 1");
    expect(markup).toContain("Device 2");
    expect(markup).not.toContain("redacted-target-user");
    expect(markup).not.toContain("TARGETDEVICE");
  });

  test("renders saved sessions when the current session is unavailable", () => {
    const markup = renderToStaticMarkup(
      <UserSettingsPanel
        currentSession={null}
        e2eeTrust={idleE2eeTrust}
        savedSessions={[
          {
            homeserver: "https://matrix.org",
            user_id: "@second-user:example.invalid",
            device_id: "SECONDDEVICE"
          }
        ]}
        settings={settings}
        {...handlers}
      />
    );

    expect(markup).toContain("Matrix account");
    expect(markup).toContain("Not restored");
    expect(markup).toContain("@second-user:example.invalid");
    expect(markup).toContain("Switch");
  });
});
