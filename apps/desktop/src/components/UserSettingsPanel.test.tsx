import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, test, vi } from "vitest";

import { UserSettingsPanel } from "./UserSettingsPanel";
import type { E2eeTrustState, ProfileState } from "../domain/types";

describe("UserSettingsPanel", () => {
  const settings = {
    values: {
      locale: { language_tag: null, text_direction: "auto" },
      appearance: { theme: "dark" },
      typography: { font: "system", emoji: "system" },
      keyboard: { composer_send_shortcut: "enter" },
      notifications: {
        desktop_notifications: true,
        sound: true,
        badges: true,
        send_read_receipts: true,
        send_typing_notifications: true
      },
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
    ignored_user_ids: [],
    ignored_user_update: { kind: "idle" },
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
    onExportRoomKeys: () => undefined,
    onImportRoomKeys: () => undefined,
    onBootstrapSecureBackup: () => undefined,
    onChangeSecureBackupPassphrase: () => undefined,
    onSwitchAccount: () => undefined,
    onUpdateSettings: () => undefined,
    onQueryDevices: () => undefined,
    onRenameDevice: () => undefined,
    onDeleteDevices: () => undefined,
    onLoadAccountManagementCapabilities: () => undefined,
    onChangePassword: () => undefined,
    onDeactivateAccount: () => undefined,
    onSubmitAccountManagementUia: () => undefined
  };
  const idleDeviceSessions: import("../domain/types").DeviceSessionListState = { kind: "idle" };
  const idleAccountManagement: import("../domain/types").AccountManagementState = { kind: "idle" };
  const idleAccountManagementCapabilities: import("../domain/types").AccountManagementCapabilities =
    { change_password: { kind: "unknown" } };

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
        deviceSessions={idleDeviceSessions}
        accountManagement={idleAccountManagement}
        accountManagementCapabilities={idleAccountManagementCapabilities}
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
    expect(markup).toContain("Send read receipts");
    expect(markup).toContain("Send typing notifications");
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
        deviceSessions={idleDeviceSessions}
        accountManagement={idleAccountManagement}
        accountManagementCapabilities={idleAccountManagementCapabilities}
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
        deviceSessions={idleDeviceSessions}
        accountManagement={idleAccountManagement}
        accountManagementCapabilities={idleAccountManagementCapabilities}
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
          ignored_user_ids: [],
          ignored_user_update: { kind: "idle" },
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

  test("renders Rust-owned E2EE key-management controls and status", () => {
    const markup = renderToStaticMarkup(
      <UserSettingsPanel
        currentSession={{
          homeserver: "https://matrix.org",
          user_id: "@demo-user:example.invalid",
          device_id: "FAKEDEVICE"
        }}
        e2eeTrust={{
          ...idleE2eeTrust,
          key_management: {
            room_key_export: {
              kind: "exported",
              request_id: 10,
              exported_sessions: null
            },
            room_key_import: {
              kind: "imported",
              request_id: 11,
              imported_count: 1,
              total_count: 1
            },
            secure_backup_setup: {
              kind: "recoveryKeyReady",
              request_id: 12,
              delivery: { kind: "written" }
            },
            passphrase_change: {
              kind: "changed",
              request_id: 13,
              delivery: { kind: "notWritten" }
            }
          }
        }}
        localEncryption={{ kind: "healthy" }}
        platform="linux"
        deviceSessions={idleDeviceSessions}
        accountManagement={idleAccountManagement}
        accountManagementCapabilities={idleAccountManagementCapabilities}
        savedSessions={[]}
        profile={profile}
        settings={settings}
        {...handlers}
      />
    );

    expect(markup).toContain("Room key export");
    expect(markup).toContain("Room key import");
    expect(markup).toContain("Secure backup");
    expect(markup).toContain("Change secure backup passphrase");
    expect(markup).toContain("Export room keys");
    expect(markup).toContain("Import room keys");
    expect(markup).toContain("Set up secure backup");
    expect(markup).toContain("Recovery key saved");
    expect(markup).toContain("1 of 1 imported");
    expect(markup).not.toContain("private-room-key-passphrase");
    expect(markup).not.toContain("private-secure-backup-passphrase");
    expect(markup).not.toContain("/tmp/");
  });

  test("renders loaded device sessions and dispatches rename", () => {
    const onRenameDevice = vi.fn();
    const loadedDeviceSessions: import("../domain/types").DeviceSessionListState = {
      kind: "loaded",
      devices: [
        {
          device_ordinal: 1,
          display_name: "Current Browser",
          current: true,
          verified: true,
          inactive: false
        },
        {
          device_ordinal: 2,
          display_name: "Other Phone",
          current: false,
          verified: false,
          inactive: true
        }
      ]
    };
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
        deviceSessions={loadedDeviceSessions}
        accountManagement={idleAccountManagement}
        accountManagementCapabilities={idleAccountManagementCapabilities}
        savedSessions={[]}
        profile={profile}
        settings={settings}
        {...handlers}
        onRenameDevice={onRenameDevice}
      />
    );

    expect(markup).toContain("Sessions");
    expect(markup).toContain("Current Browser");
    expect(markup).toContain("Other Phone");
    expect(markup).toContain("Current session");
    expect(markup).toContain("Other sessions");
    expect(markup).toContain("Sign out all other sessions");
  });

  test("computes non-current ordinals for sign out all other sessions", () => {
    const loadedDeviceSessions: import("../domain/types").DeviceSessionListState = {
      kind: "loaded",
      devices: [
        {
          device_ordinal: 1,
          display_name: "Current Browser",
          current: true,
          verified: true,
          inactive: false
        },
        {
          device_ordinal: 2,
          display_name: "Other Phone",
          current: false,
          verified: false,
          inactive: true
        },
        {
          device_ordinal: 3,
          display_name: "Other Tablet",
          current: false,
          verified: true,
          inactive: false
        }
      ]
    };
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
        deviceSessions={loadedDeviceSessions}
        accountManagement={idleAccountManagement}
        accountManagementCapabilities={idleAccountManagementCapabilities}
        savedSessions={[]}
        profile={profile}
        settings={settings}
        {...handlers}
      />
    );

    expect(markup).toContain("Sign out all other sessions");
  });

  test("renders UIA password prompt when account management awaits UIA", () => {
    const onSubmitAccountManagementUia = vi.fn();
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
        deviceSessions={idleDeviceSessions}
        accountManagement={{
          kind: "awaitingUia",
          request_id: 42,
          flow_id: 7,
          operation: "deleteOtherDevices"
        }}
        accountManagementCapabilities={idleAccountManagementCapabilities}
        savedSessions={[]}
        profile={profile}
        settings={settings}
        {...handlers}
        onSubmitAccountManagementUia={onSubmitAccountManagementUia}
      />
    );

    expect(markup).toContain("Password");
    expect(markup).toContain("Continue");
  });

  test("renders account management section with change password and deactivate buttons", () => {
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
        deviceSessions={idleDeviceSessions}
        accountManagement={idleAccountManagement}
        accountManagementCapabilities={{
          change_password: { kind: "enabled" }
        }}
        savedSessions={[]}
        profile={profile}
        settings={settings}
        {...handlers}
      />
    );

    expect(markup).toContain("Account management");
    expect(markup).toContain("Change password");
    expect(markup).toContain("Deactivate account");
  });

  test("disables change password button when the server capability is disabled", () => {
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
        deviceSessions={idleDeviceSessions}
        accountManagement={idleAccountManagement}
        accountManagementCapabilities={{
          change_password: { kind: "disabled" }
        }}
        savedSessions={[]}
        profile={profile}
        settings={settings}
        {...handlers}
      />
    );

    expect(markup).toContain('data-testid="change-password-button"');
    expect(markup).toContain("disabled");
  });

  test("renders password-changed success state", () => {
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
        deviceSessions={idleDeviceSessions}
        accountManagement={{
          kind: "succeeded",
          request_id: 50,
          operation: "changePassword"
        }}
        accountManagementCapabilities={{
          change_password: { kind: "enabled" }
        }}
        savedSessions={[]}
        profile={profile}
        settings={settings}
        {...handlers}
      />
    );

    expect(markup).toContain("Password changed");
  });

  test("renders account-deactivated success state", () => {
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
        deviceSessions={idleDeviceSessions}
        accountManagement={{
          kind: "succeeded",
          request_id: 51,
          operation: "deactivateAccount"
        }}
        accountManagementCapabilities={{
          change_password: { kind: "enabled" }
        }}
        savedSessions={[]}
        profile={profile}
        settings={settings}
        {...handlers}
      />
    );

    expect(markup).toContain("Account deactivated");
  });

  test("renders UIA password prompt for change password awaiting UIA", () => {
    const onSubmitAccountManagementUia = vi.fn();
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
        deviceSessions={idleDeviceSessions}
        accountManagement={{
          kind: "awaitingUia",
          request_id: 52,
          flow_id: 8,
          operation: "changePassword"
        }}
        accountManagementCapabilities={{
          change_password: { kind: "enabled" }
        }}
        savedSessions={[]}
        profile={profile}
        settings={settings}
        {...handlers}
        onSubmitAccountManagementUia={onSubmitAccountManagementUia}
      />
    );

    expect(markup).toContain("Password");
    expect(markup).toContain("Continue");
  });
});
