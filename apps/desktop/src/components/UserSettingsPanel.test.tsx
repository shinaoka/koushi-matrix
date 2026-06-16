import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, test } from "vitest";

import { UserSettingsPanel } from "./UserSettingsPanel";
import type { E2eeTrustState, ProfileState } from "../domain/types";

describe("UserSettingsPanel", () => {
  const settings = {
    values: {
      locale: { language_tag: null, text_direction: "auto" },
      appearance: { theme: "dark" },
      typography: { font: "system", emoji: "system" },
      keyboard: { composer_send_shortcut: "enter" },
      notifications: { desktop_notifications: true, sound: true, badges: true },
      display: { code_block_wrap: true, hide_redacted: false },
      media: {
        image_upload_compression: "never",
        image_upload_compression_policy: {
          threshold_bytes: 1048576,
          threshold_long_edge: 2560,
          target_long_edge: 2048,
          quality_percent: 82
        }
      }
    },
    persistence: { kind: "idle" }
  } as const;
  const keyManagement: E2eeTrustState["key_management"] = {
    room_key_export: { kind: "idle" },
    room_key_import: { kind: "idle" },
    secure_backup_setup: { kind: "idle" },
    passphrase_change: { kind: "idle" }
  };
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
    key_management: keyManagement,
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
    key_management: keyManagement,
    devices: []
  };
  const profile: ProfileState = {
    own: {
      display_name: "Demo User",
      avatar: {
        mxc_uri: "mxc://matrix.org/avatar",
        thumbnail: {
          kind: "ready",
          source_url: "asset://profile-avatar",
          width: 64,
          height: 64,
          mime_type: "image/png"
        }
      }
    },
    users: {},
    local_aliases: {},
    local_alias_update: { kind: "idle" },
    update: { kind: "idle" }
  };
  const handlers = {
    onAcceptVerification: () => undefined,
    onBootstrapCrossSigning: () => undefined,
    onCancelVerification: () => undefined,
    onConfirmSasVerification: () => undefined,
    onEnableKeyBackup: () => undefined,
    onOpenRecovery: () => undefined,
    onOpenKeyboardSettings: () => undefined,
    onProbeLocalEncryption: () => undefined,
    onResetLocalData: () => undefined,
    onResetIdentity: () => undefined,
    onSetAvatar: () => undefined,
    onSetDisplayName: () => undefined,
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
        localEncryption={{ kind: "healthy" }}
        platform="linux"
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
        profile={profile}
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
    expect(markup).toContain("Profile");
    expect(markup).toContain("Demo User");
    expect(markup).toContain("asset://profile-avatar");
    expect(markup).toContain("Upload");
    expect(markup).toContain("Homeserver");
    expect(markup).toContain("Device");
    expect(markup).toContain("Local store");
    expect(markup).toContain("Appearance");
    expect(markup).toContain("Dark");
    expect(markup).toContain("Typography");
    expect(markup).toContain("UI font");
    expect(markup).toContain("Emoji font");
    expect(markup).toContain("Media");
    expect(markup).toContain("Compress images");
    expect(markup).toContain("Always");
    expect(markup).toContain("Ask");
    expect(markup).toContain("Never");
    expect(markup).toContain("Notifications");
    expect(markup).toContain("Desktop notifications");
    expect(markup).toContain("Sound");
    expect(markup).toContain("Badges");
    expect(markup).toContain('role="switch"');
    expect(markup).toContain("Inter");
    expect(markup).toContain("Twemoji COLR");
    expect(markup).toContain('aria-pressed="true"');
    expect(markup).toContain("Separate encrypted namespace");
    expect(markup).toContain("Secret Service");
    expect(markup).toContain("Protected");
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

  test("renders the Rust-owned code block wrap display setting", () => {
    const markup = renderToStaticMarkup(
      <UserSettingsPanel
        currentSession={{
          homeserver: "https://matrix.org",
          user_id: "@demo-user:example.invalid",
          device_id: "FAKEDEVICE"
        }}
        e2eeTrust={idleE2eeTrust}
        localEncryption={{ kind: "healthy" }}
        platform="linux"
        savedSessions={[]}
        profile={profile}
        settings={{
          ...settings,
          values: {
            ...settings.values,
            display: { code_block_wrap: false, hide_redacted: true }
          }
        }}
        {...handlers}
      />
    );

    expect(markup).toContain("Display");
    expect(markup).toContain("Wrap long lines in code blocks");
    expect(markup).toContain("Hide deleted messages");
    expect(markup).toContain('role="switch"');
    expect(markup).toContain('aria-checked="false"');
  });

  test("renders saved sessions when the current session is unavailable", () => {
    const markup = renderToStaticMarkup(
      <UserSettingsPanel
        currentSession={null}
        e2eeTrust={idleE2eeTrust}
        localEncryption={{ kind: "unknown" }}
        platform="linux"
        savedSessions={[
          {
            homeserver: "https://matrix.org",
            user_id: "@second-user:example.invalid",
            device_id: "SECONDDEVICE"
          }
        ]}
        profile={{
          own: { display_name: null, avatar: null },
          users: {},
          local_aliases: {},
          local_alias_update: { kind: "idle" },
          update: { kind: "idle" }
        }}
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
