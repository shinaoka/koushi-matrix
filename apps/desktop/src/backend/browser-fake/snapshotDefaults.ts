import type { DesktopSnapshot } from "../../domain/types";

export function defaultDirectoryState(): DesktopSnapshot["state"]["domain"]["directory"] {
  return {
    query: { kind: "closed" },
    preview: { kind: "closed" },
    join: { kind: "idle" }
  };
}

export function defaultE2eeTrustState(): DesktopSnapshot["state"]["domain"]["e2ee_trust"] {
  return {
    verification: { kind: "idle" },
    cross_signing: { kind: "unknown" },
    key_backup: { kind: "unknown" },
    identity_reset: { kind: "idle" },
    key_management: defaultE2eeKeyManagementState(),
    devices: []
  };
}

export function defaultDelegatedAuthLinks(): Extract<
  DesktopSnapshot["state"]["domain"]["auth"],
  { kind: "ready" }
>["delegated"] {
  return { registration_url: null };
}

function defaultE2eeKeyManagementState(): DesktopSnapshot["state"]["domain"]["e2ee_trust"]["key_management"] {
  return {
    room_key_export: { kind: "idle" },
    room_key_import: { kind: "idle" },
    secure_backup_setup: { kind: "idle" },
    passphrase_change: { kind: "idle" }
  };
}

export function defaultLiveSignalsState(): DesktopSnapshot["state"]["domain"]["live_signals"] {
  return {
    rooms: {},
    presence: {}
  };
}

export function defaultNativeAttentionState(): DesktopSnapshot["state"]["domain"]["native_attention"] {
  return {
    summary: {
      unread_count: 0,
      highlight_count: 0,
      badge_count: 0,
      candidate: null,
      capabilities: {
        notifications: "unknown",
        badge: "unknown",
        overlay_icon: "unknown",
        sound: "unknown",
        tray: "unknown",
        activation: "unknown"
      }
    },
    dispatch: { kind: "idle" }
  };
}

export function defaultCjkTextPolicyState(): DesktopSnapshot["state"]["domain"]["cjk_text_policy"] {
  return {
    japanese_catalog: {
      catalog_locale: "en",
      complete: true,
      missing_message_ids: []
    },
    normalization: {
      form: "nfkc",
      width_fold: true,
      kana_fold: true
    },
    collation: {
      locale: "ja",
      numeric: true,
      case_first: null
    }
  };
}

export function defaultProfileState(userId: string | null | undefined): DesktopSnapshot["state"]["domain"]["profile"] {
  return {
    own: {
      display_name: userId ? "Demo User" : null,
      avatar: null
    },
    users: {},
    room_users: {},
    local_aliases: {},
    local_alias_update: { kind: "idle" },
    ignored_user_ids: [],
    ignored_user_update: { kind: "idle" },
    update: { kind: "idle" }
  };
}
