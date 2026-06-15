/**
 * Playwright harness entry: mounts the FULL <App /> over a recording mock
 * Tauri IPC transport, so the headless-Chromium spec
 * (e2e/basic-operations.spec.ts) can prove that the Task 5 UI drives the
 * right Tauri COMMAND NAMES (create_room, create_space,
 * accept_invite, invite_user, start_direct_message,
 * set_composer_reply_target, send_reply, edit_message).
 *
 * Loaded only by appHarness.html (never part of the production index.html
 * bundle). No Tauri process, no network: everything flows through
 * TauriIpcMock via Tauri's official `mockIPC` (which installs
 * window.__TAURI_INTERNALS__ so isTauriRuntime() is true and the App selects
 * TauriDesktopApi → invoke()).
 *
 * CRITICAL ORDERING: createDesktopApi() and the tauriTimelineTransport
 * constant both run at App MODULE-LOAD time and snapshot isTauriRuntime().
 * So the mock + window.__harness must be installed BEFORE the App module is
 * imported. We therefore set everything up first and dynamically import the
 * App only afterwards.
 */

import { createRoot } from "react-dom/client";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";

import type { CoreEventPayload, TimelineItem } from "../domain/coreEvents";
import { roomTimelineKey } from "../domain/coreEvents";
import type {
  ComposerKeyEvent,
  ComposerResolverOptions,
  ComposerResolvedAction,
  ComposerMode,
  ComposerSurface,
  DesktopSnapshot,
  E2eeTrustState,
  LocaleDisplayProfile,
  LocaleSettings,
  SettingsPatch
} from "../domain/types";
import { TauriIpcMock, type IpcInvocation } from "./tauriIpcMock";
import "../styles.css";

const CORE_EVENT_NAME = "matrix-desktop://event";
const STATE_EVENT_NAME = "matrix-desktop://state";

// Identity used across the ready snapshot, timeline key, and CoreEvents.
const HOMESERVER = "https://harness.example.invalid";
const USER_ID = "@harness-user:example.invalid";
const DEVICE_ID = "HARNESSDEVICE";
const SPACE_ID = "!harness-space:example.invalid";
const ROOM_ID = "!harness-room:example.invalid";
const ROOM_NAME = "Harness Room";
const SPACE_NAME = "Harness Space";
const SEED_EVENT_ID = "$seed-event:example.invalid";

// The control surface exposed on window for the Playwright spec. We do NOT
// augment the global Window interface here: src/test/harnessMain.tsx already
// declares a (narrower) window.__harness for the timeline harness, and two
// conflicting global augmentations would break `tsc`. Each harness HTML loads
// exactly one entry module, so a local interface + a single assignment cast is
// sufficient and keeps the two harnesses independent.
interface AppHarnessControl {
  invocations(): readonly IpcInvocation[];
  invocationsOf(command: string): IpcInvocation[];
  clearInvocations(): void;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  setCommandResponse(command: string, response: any): void;
  setSnapshot(snapshot: DesktopSnapshot): void;
  pushCoreEvent(event: CoreEventPayload): Promise<void>;
  pushStateChanged(): void;
  currentSnapshot(): DesktopSnapshot;
  e2eeTrustSnapshot(): DesktopSnapshot;
  replyModeSnapshot(): DesktopSnapshot;
}

// ---------------------------------------------------------------------------
// Ready snapshot factory: a signed-in shell with one space, one selected
// room, and a populated composer mode (Plain by default). The timeline ROWS
// do NOT live here — in Tauri-runtime mode the App renders TimelineView from
// the CoreEvent stream, so the reply target is seeded via pushCoreEvent.
// ---------------------------------------------------------------------------

function readySnapshot(
  overrides: {
    composerMode?: ComposerMode;
    basicOperation?: DesktopSnapshot["state"]["basic_operation"];
    e2eeTrust?: E2eeTrustState;
    extraSpaces?: DesktopSnapshot["state"]["spaces"];
    extraRailItems?: DesktopSnapshot["sidebar"]["space_rail"];
  } = {}
): DesktopSnapshot {
  const composerMode = overrides.composerMode ?? "Plain";
  const basicOperation = overrides.basicOperation ?? { kind: "idle" };
  const spaces = [
    {
      space_id: SPACE_ID,
      display_name: SPACE_NAME,
      avatar: null,
      child_room_ids: [ROOM_ID]
    },
    ...(overrides.extraSpaces ?? [])
  ];
  const railItems = [
    {
      space_id: SPACE_ID,
      display_name: SPACE_NAME,
      avatar: null,
      unread_count: 0,
      highlight_count: 0,
      is_active: false
    },
    ...(overrides.extraRailItems ?? [])
  ];
  return {
    state: {
      session: {
        kind: "ready",
        homeserver: HOMESERVER,
        user_id: USER_ID,
        device_id: DEVICE_ID
      },
      auth: { kind: "unknown" },
      settings: defaultSettingsState(),
      locale_profile: defaultLocaleDisplayProfile(),
      typography_profile: defaultTypographyDisplayProfile(),
      profile: {
        own: { display_name: "Harness User", avatar: null },
        users: {},
        update: { kind: "idle" }
      },
      sync: "running",
      navigation: { active_space_id: null, active_room_id: ROOM_ID },
      spaces,
      rooms: [
        {
          room_id: ROOM_ID,
          display_name: ROOM_NAME,
          avatar: null,
          is_dm: false,
          tags: { favourite: null, low_priority: null },
          unread_count: 0,
          notification_count: 0,
          highlight_count: 0,
          parent_space_ids: []
        }
      ],
      invites: [],
      room_interactions: {},
      directory: { query: { kind: "closed" }, join: { kind: "idle" } },
      room_management: { selected_room_id: null, settings: null, operation: { kind: "idle" } },
      activity: { kind: "closed" },
      timeline: {
        room_id: ROOM_ID,
        is_subscribed: true,
        is_paginating_backwards: false,
        composer: {
          pending_transaction_id: null,
          draft: "",
          mode: composerMode
        }
      },
      thread: { kind: "closed" },
      focused_context: { kind: "closed" },
      search: { kind: "closed" },
      errors: [],
      basic_operation: basicOperation,
      live_signals: defaultLiveSignalsState(),
      e2ee_trust: overrides.e2eeTrust ?? defaultE2eeTrustState(),
      local_encryption: { kind: "unknown" },
      native_attention: defaultNativeAttentionState(),
      cjk_text_policy: defaultCjkTextPolicyState()
    },
    sidebar: {
      active_space_id: null,
      account_home: { display_name: "Home", unread_count: 0, highlight_count: 0, is_active: true },
      space_rail: railItems,
      space_rooms: [
        {
          room_id: ROOM_ID,
          display_name: ROOM_NAME,
          avatar: null,
          tags: { favourite: null, low_priority: null },
          unread_count: 0,
          highlight_count: 0
        }
      ],
      global_dms: [],
      space_unread_count: 0,
      dm_unread_count: 0,
      space_highlight_count: 0,
      dm_highlight_count: 0
    },
    timeline: [],
    thread: null
  };
}

function defaultSettingsState(): DesktopSnapshot["state"]["settings"] {
  return {
    values: {
      locale: { language_tag: null, text_direction: "auto" },
      appearance: { theme: "system" },
      typography: { font: "system", emoji: "system" },
      keyboard: { composer_send_shortcut: "enter" }
    },
    persistence: { kind: "idle" }
  };
}

function defaultE2eeTrustState(): DesktopSnapshot["state"]["e2ee_trust"] {
  return {
    verification: { kind: "idle" },
    cross_signing: { kind: "unknown" },
    key_backup: { kind: "unknown" },
    identity_reset: { kind: "idle" },
    devices: []
  };
}

function defaultLiveSignalsState(): DesktopSnapshot["state"]["live_signals"] {
  return {
    rooms: {},
    presence: {}
  };
}

function defaultNativeAttentionState(): DesktopSnapshot["state"]["native_attention"] {
  return {
    summary: {
      unread_count: 0,
      highlight_count: 0,
      badge_count: 0,
      candidate: null,
      capabilities: {
        notifications: "unknown",
        badge: "unknown",
        sound: "unknown",
        tray: "unknown",
        activation: "unknown"
      }
    },
    dispatch: { kind: "idle" }
  };
}

function defaultCjkTextPolicyState(): DesktopSnapshot["state"]["cjk_text_policy"] {
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

function applySettingsPatch(
  values: DesktopSnapshot["state"]["settings"]["values"],
  patch: SettingsPatch
): DesktopSnapshot["state"]["settings"]["values"] {
  return {
    locale: patch.locale ?? values.locale,
    appearance: patch.appearance ?? values.appearance,
    typography: patch.typography ?? values.typography,
    keyboard: patch.keyboard ?? values.keyboard
  };
}

function defaultLocaleDisplayProfile(): LocaleDisplayProfile {
  return resolveLocaleDisplayProfile({ language_tag: null, text_direction: "auto" });
}

function defaultTypographyDisplayProfile(): DesktopSnapshot["state"]["typography_profile"] {
  return resolveTypographyDisplayProfile({ font: "system", emoji: "system" });
}

function resolveTypographyDisplayProfile(
  typography: DesktopSnapshot["state"]["settings"]["values"]["typography"]
): DesktopSnapshot["state"]["typography_profile"] {
  return {
    font: typography.font,
    emoji: typography.emoji,
    platform: "linux",
    font_asset: typography.font === "inter" ? "bundledPreferred" : "systemFallback",
    emoji_asset: typography.emoji === "twemojiColr" ? "bundledPreferred" : "systemFallback"
  };
}

function resolveLocaleDisplayProfile(locale: LocaleSettings): LocaleDisplayProfile {
  const parsed = parseLocale(locale.language_tag);
  const pseudoLocale = parsed?.pseudo_locale ?? "none";
  const catalogLocale =
    pseudoLocale === "accented" || pseudoLocale === "bidi"
      ? "pseudo"
      : parsed?.language === "ja"
        ? "ja"
        : "en";
  const lang =
    pseudoLocale === "accented"
      ? "en-XA"
      : pseudoLocale === "bidi"
        ? "ar-XB"
        : catalogLocale === "ja"
          ? "ja"
          : "en";
  const dir =
    locale.text_direction === "ltr" || locale.text_direction === "rtl"
      ? locale.text_direction
      : pseudoLocale === "bidi" || parsed?.direction === "rtl"
        ? "rtl"
        : "ltr";

  return {
    lang,
    dir,
    catalog_locale: catalogLocale,
    pseudo_locale: pseudoLocale,
    platform: "linux",
    modifier_labels: { primary: "Ctrl" }
  };
}

function parseLocale(
  rawTag: string | null
): {
  language: "en" | "ja" | "rtl";
  direction: "ltr" | "rtl";
  pseudo_locale: "none" | "accented" | "bidi";
} | null {
  const normalized = rawTag?.trim().replaceAll("_", "-");
  if (!normalized) {
    return null;
  }
  const [primaryRaw, ...rest] = normalized.split("-");
  const primary = primaryRaw.toLowerCase();
  if (
    !/^[a-z]{2,3}$/.test(primary) ||
    rest.some((subtag) => subtag.toLowerCase() === "x")
  ) {
    return null;
  }
  if (!rest.every((subtag) => /^[a-z0-9]{1,8}$/i.test(subtag))) {
    return null;
  }
  const pseudo_locale =
    normalized.toLowerCase() === "en-xa"
      ? "accented"
      : normalized.toLowerCase() === "ar-xb"
        ? "bidi"
        : "none";

  if (primary === "en") {
    return { language: "en", direction: "ltr", pseudo_locale };
  }
  if (primary === "ja") {
    return { language: "ja", direction: "ltr", pseudo_locale };
  }
  if (["ar", "dv", "fa", "he", "ps", "sd", "ug", "ur", "yi"].includes(primary)) {
    return { language: "rtl", direction: "rtl", pseudo_locale };
  }
  return null;
}

function resolveComposerKeyActionFromSettings(
  sendShortcut: DesktopSnapshot["state"]["settings"]["values"]["keyboard"]["composer_send_shortcut"],
  surface: ComposerSurface,
  keyEvent: ComposerKeyEvent,
  options: ComposerResolverOptions
): ComposerResolvedAction {
  void surface;
  if (keyEvent.is_composing) {
    return "commitImeCandidate";
  }
  if (keyEvent.key === "escape") {
    return "cancel";
  }
  if (keyEvent.key !== "enter") {
    return "noop";
  }
  if (keyEvent.modifiers.shift || keyEvent.modifiers.alt) {
    return "insertNewline";
  }
  if (options.autocomplete_open) {
    return "acceptAutocomplete";
  }
  const wantsSend =
    sendShortcut === "enter" ||
    (sendShortcut === "modEnter" && (keyEvent.modifiers.ctrl || keyEvent.modifiers.meta));
  if (!wantsSend) {
    return "insertNewline";
  }
  return options.send_enabled ? "send" : "noop";
}

// A reply-mode composer snapshot (composer.mode = Reply) used as the
// set_composer_reply_target response so the App's local `sendText` dispatch
// sees reply mode and routes to send_reply.
function replyModeSnapshot(): DesktopSnapshot {
  return readySnapshot({
    composerMode: { Reply: { in_reply_to_event_id: SEED_EVENT_ID } }
  });
}

function e2eeTrustFixture(): E2eeTrustState {
  return {
    verification: {
      kind: "requested",
      request_id: 9_001,
      target: {
        user_id: "redacted-trust-target",
        device_id: "TRUSTDEVICE"
      }
    },
    cross_signing: { kind: "missing" },
    key_backup: { kind: "disabled" },
    identity_reset: { kind: "idle" },
    devices: [
      {
        user_id: USER_ID,
        device_id: DEVICE_ID,
        trust_level: "verified"
      },
      {
        user_id: "redacted-trust-target",
        device_id: "TRUSTDEVICE",
        trust_level: "unverified"
      }
    ]
  };
}

function e2eeTrustSnapshot(): DesktopSnapshot {
  return readySnapshot({ e2eeTrust: e2eeTrustFixture() });
}

// A snapshot where a freshly-created room/space is present + active, so the
// create dialog's success path (which closes on success) is exercised.
function afterCreateRoomSnapshot(): DesktopSnapshot {
  const snapshot = readySnapshot();
  const newRoomId = "!created-room:example.invalid";
  snapshot.state.rooms.push({
    room_id: newRoomId,
    display_name: "Created Room",
    avatar: null,
    is_dm: false,
    tags: { favourite: null, low_priority: null },
    unread_count: 0,
    notification_count: 0,
    highlight_count: 0,
    parent_space_ids: []
  });
  snapshot.state.navigation.active_room_id = newRoomId;
  snapshot.state.timeline.room_id = newRoomId;
  snapshot.sidebar.space_rooms.push({
    room_id: newRoomId,
    display_name: "Created Room",
    avatar: null,
    tags: { favourite: null, low_priority: null },
    unread_count: 0,
    highlight_count: 0
  });
  return snapshot;
}

function afterCreateSpaceSnapshot(): DesktopSnapshot {
  const snapshot = readySnapshot();
  const newSpaceId = "!created-space:example.invalid";
  snapshot.state.spaces.push({
    space_id: newSpaceId,
    display_name: "Created Space",
    avatar: null,
    child_room_ids: []
  });
  snapshot.state.navigation.active_space_id = newSpaceId;
  snapshot.sidebar.active_space_id = newSpaceId;
  snapshot.sidebar.space_rail.push({
    space_id: newSpaceId,
    display_name: "Created Space",
    avatar: null,
    unread_count: 0,
    highlight_count: 0,
    is_active: true
  });
  return snapshot;
}

// ---------------------------------------------------------------------------
// Mock setup (BEFORE importing App)
// ---------------------------------------------------------------------------

const mock = new TauriIpcMock();
let currentSnapshot = readySnapshot();

function setCurrentSnapshot(next: DesktopSnapshot): DesktopSnapshot {
  currentSnapshot = next;
  return currentSnapshot;
}

// Snapshot-returning commands the App calls. Default snapshot stays ready so
// any unanticipated snapshot read still renders the shell.
mock.setCommandResponse("get_snapshot", () => currentSnapshot);
mock.setCommandResponse("list_saved_sessions", () => []);
mock.setCommandResponse("update_settings", ({ patch }: { patch: SettingsPatch }) => {
  const values = applySettingsPatch(currentSnapshot.state.settings.values, patch);
  return setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      settings: {
        ...currentSnapshot.state.settings,
        values,
        persistence: { kind: "idle" }
      },
      locale_profile: resolveLocaleDisplayProfile(values.locale),
      typography_profile: resolveTypographyDisplayProfile(values.typography)
    }
  });
});
mock.setCommandResponse("bootstrap_cross_signing", () =>
  setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      e2ee_trust: {
        ...currentSnapshot.state.e2ee_trust,
        cross_signing: { kind: "trusted" }
      }
    }
  })
);
mock.setCommandResponse("enable_key_backup", () =>
  setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      e2ee_trust: {
        ...currentSnapshot.state.e2ee_trust,
        key_backup: { kind: "enabled", version: "harness-backup" }
      }
    }
  })
);
mock.setCommandResponse("accept_verification", ({ flowId }: { flowId: number }) => {
  const verification = currentSnapshot.state.e2ee_trust.verification;
  if (verification.kind !== "requested" || verification.request_id !== flowId) {
    return currentSnapshot;
  }
  return setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      e2ee_trust: {
        ...currentSnapshot.state.e2ee_trust,
        verification: {
          kind: "accepted",
          request_id: flowId,
          target: verification.target
        }
      }
    }
  });
});
mock.setCommandResponse("confirm_sas_verification", ({ flowId }: { flowId: number }) => {
  const verification = currentSnapshot.state.e2ee_trust.verification;
  if (
    (verification.kind !== "sasPresented" && verification.kind !== "confirming") ||
    verification.request_id !== flowId
  ) {
    return currentSnapshot;
  }
  return setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      e2ee_trust: {
        ...currentSnapshot.state.e2ee_trust,
        verification: {
          kind: "done",
          request_id: flowId,
          target: verification.target
        }
      }
    }
  });
});
mock.setCommandResponse("cancel_verification", ({ flowId }: { flowId: number }) => {
  const verification = currentSnapshot.state.e2ee_trust.verification;
  if (verification.kind === "idle" || verification.request_id !== flowId) {
    return currentSnapshot;
  }
  return setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      e2ee_trust: {
        ...currentSnapshot.state.e2ee_trust,
        verification: { kind: "idle" }
      }
    }
  });
});
mock.setCommandResponse("reset_identity", () =>
  setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      e2ee_trust: {
        ...currentSnapshot.state.e2ee_trust,
        identity_reset: {
          kind: "awaitingAuth",
          request_id: 9_100,
          auth_type: "uiaa"
        }
      }
    }
  })
);
mock.setCommandResponse(
  "submit_identity_reset_password",
  ({ flowId }: { flowId: number; password: string }) => {
    const identityReset = currentSnapshot.state.e2ee_trust.identity_reset;
    if (identityReset.kind !== "awaitingAuth" || identityReset.request_id !== flowId) {
      return currentSnapshot;
    }
    return setCurrentSnapshot({
      ...currentSnapshot,
      state: {
        ...currentSnapshot.state,
        e2ee_trust: {
          ...currentSnapshot.state.e2ee_trust,
          cross_signing: { kind: "missing" },
          key_backup: { kind: "disabled" },
          identity_reset: { kind: "idle" },
          devices: currentSnapshot.state.e2ee_trust.devices.map((device) => ({
            ...device,
            trust_level: "unverified"
          }))
        }
      }
    });
  }
);
mock.setCommandResponse("submit_identity_reset_oauth", ({ flowId }: { flowId: number }) => {
  const identityReset = currentSnapshot.state.e2ee_trust.identity_reset;
  if (identityReset.kind !== "awaitingAuth" || identityReset.request_id !== flowId) {
    return currentSnapshot;
  }
  return setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      e2ee_trust: {
        ...currentSnapshot.state.e2ee_trust,
        cross_signing: { kind: "missing" },
        key_backup: { kind: "disabled" },
        identity_reset: { kind: "idle" },
        devices: currentSnapshot.state.e2ee_trust.devices.map((device) => ({
          ...device,
          trust_level: "unverified"
        }))
      }
    }
  });
});
mock.setCommandResponse(
  "resolve_composer_key_action",
  ({
    surface,
    keyEvent,
    autocompleteOpen,
    sendEnabled
  }: {
    surface: ComposerSurface;
    keyEvent: ComposerKeyEvent;
    autocompleteOpen: boolean;
    sendEnabled: boolean;
  }) =>
    resolveComposerKeyActionFromSettings(
      currentSnapshot.state.settings.values.keyboard.composer_send_shortcut,
      surface,
      keyEvent,
      {
        autocomplete_open: autocompleteOpen,
        send_enabled: sendEnabled
      }
    )
);
mock.setCommandResponse("create_room", () => setCurrentSnapshot(afterCreateRoomSnapshot()));
mock.setCommandResponse("create_space", () => setCurrentSnapshot(afterCreateSpaceSnapshot()));
mock.setCommandResponse("accept_invite", () => currentSnapshot);
mock.setCommandResponse("decline_invite", () => currentSnapshot);
mock.setCommandResponse("start_direct_message", () => currentSnapshot);
mock.setCommandResponse("invite_user", () => currentSnapshot);
// Clicking reply records set_composer_reply_target AND returns a reply-mode
// snapshot, so the subsequent composer submit dispatches send_reply.
mock.setCommandResponse("set_composer_reply_target", () =>
  setCurrentSnapshot(replyModeSnapshot())
);
mock.setCommandResponse("cancel_composer_reply", () => setCurrentSnapshot(readySnapshot()));
// send_reply / send_text return to a Plain composer snapshot.
mock.setCommandResponse("send_reply", () => setCurrentSnapshot(readySnapshot()));
mock.setCommandResponse("send_text", () => setCurrentSnapshot(readySnapshot()));
mock.setCommandResponse(
  "open_thread",
  ({ roomId, rootEventId }: { roomId: string; rootEventId: string }) => {
    const next: DesktopSnapshot = {
      ...currentSnapshot,
      state: {
        ...currentSnapshot.state,
        thread: {
          kind: "open",
          room_id: roomId,
          root_event_id: rootEventId,
          is_subscribed: true,
          composer: {
            pending_transaction_id: null,
            draft: "",
            mode: "Plain"
          }
        }
      },
      thread: null
    };
    return setCurrentSnapshot(next);
  }
);
mock.setCommandResponse(
  "set_thread_composer_draft",
  ({ roomId, rootEventId, draft }: { roomId: string; rootEventId: string; draft: string }) => {
    const thread = currentSnapshot.state.thread;
    if (
      thread.kind !== "open" ||
      thread.room_id !== roomId ||
      thread.root_event_id !== rootEventId ||
      !thread.composer
    ) {
      return currentSnapshot;
    }
    const next: DesktopSnapshot = {
      ...currentSnapshot,
      state: {
        ...currentSnapshot.state,
        thread: {
          ...thread,
          composer: {
            ...thread.composer,
            draft
          }
        }
      }
    };
    return setCurrentSnapshot(next);
  }
);
mock.setCommandResponse(
  "send_thread_reply",
  ({ roomId, rootEventId, body }: { roomId: string; rootEventId: string; body: string }) => {
    const thread = currentSnapshot.state.thread;
    if (
      thread.kind !== "open" ||
      thread.room_id !== roomId ||
      thread.root_event_id !== rootEventId ||
      !thread.composer ||
      thread.composer.pending_transaction_id ||
      body.trim().length === 0
    ) {
      return currentSnapshot;
    }
    const next: DesktopSnapshot = {
      ...currentSnapshot,
      state: {
        ...currentSnapshot.state,
        thread: {
          ...thread,
          composer: {
            ...thread.composer,
            draft: "",
            pending_transaction_id: null
          }
        }
      }
    };
    return setCurrentSnapshot(next);
  }
);
mock.setCommandResponse("close_thread", () => {
  const next: DesktopSnapshot = {
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      thread: { kind: "closed" }
    },
    thread: null
  };
  return setCurrentSnapshot(next);
});
mock.setCommandResponse("submit_search", ({ query }: { query?: string }) => {
  const next = readySnapshot();
  next.state.search = {
    kind: "results",
    request_id: 1,
    query: String(query ?? "Alpha"),
    scope: "allRooms",
    results: [
      {
        room_id: ROOM_ID,
        event_id: SEED_EVENT_ID,
        sender: USER_ID,
        timestamp_ms: 1_800_000_000_000,
        score_millis: 950,
        snippet: "Alpha keyword update from demo coordinator.",
        match_field: "messageBody",
        highlights: [{ start_utf16: 0, end_utf16: 5 }],
        match_kind: "exact"
      }
    ]
  };
  return setCurrentSnapshot(next);
});
mock.setCommandResponse(
  "select_search_result",
  ({ roomId, eventId }: { roomId: string; eventId: string }) => {
    const next: DesktopSnapshot = {
      ...currentSnapshot,
      state: {
        ...currentSnapshot.state,
        navigation: {
          ...currentSnapshot.state.navigation,
          active_room_id: roomId
        },
        timeline: {
          ...currentSnapshot.state.timeline,
          room_id: roomId,
          is_subscribed: true
        },
        thread: { kind: "closed" },
        focused_context: {
          kind: "opening",
          room_id: roomId,
          event_id: eventId
        }
      }
    };
    return setCurrentSnapshot(next);
  }
);
mock.setCommandResponse("close_focused_context", () => {
  const next: DesktopSnapshot = {
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      focused_context: { kind: "closed" }
    }
  };
  return setCurrentSnapshot(next);
});
mock.setCommandResponse("send_reaction", () => currentSnapshot);
mock.setCommandResponse("redact_reaction", () => currentSnapshot);
mock.setCommandResponse("edit_message", () => currentSnapshot);
mock.setCommandResponse("redact_message", () => currentSnapshot);
mock.setCommandResponse("set_room_tag", () => currentSnapshot);
mock.setCommandResponse("remove_room_tag", () => currentSnapshot);
mock.setCommandResponse("load_room_settings", ({ roomId }: { roomId: string }) => {
  const room = currentSnapshot.state.rooms.find((candidate) => candidate.room_id === roomId);
  const next: DesktopSnapshot = {
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      room_management: {
        selected_room_id: roomId,
        settings: {
          room_id: roomId,
          name: room?.display_name ?? null,
          topic: null,
          avatar_url: null,
          join_rule: "invite",
          history_visibility: "shared",
          permissions: {
            can_edit_settings: true,
            can_kick: true,
            can_ban: true,
            can_unban: true
          },
          members: []
        },
        operation: { kind: "idle" }
      }
    }
  };
  return setCurrentSnapshot(next);
});
mock.setCommandResponse("update_room_setting", () => currentSnapshot);
mock.setCommandResponse("moderate_room_member", () => currentSnapshot);
mock.setCommandResponse("pin_event", () => currentSnapshot);
mock.setCommandResponse("unpin_event", () => currentSnapshot);
mock.setCommandResponse("upload_media", () => currentSnapshot);
mock.setCommandResponse("download_media", () => currentSnapshot);
mock.setCommandResponse("load_message_source", () => currentSnapshot);
mock.setCommandResponse("forward_message", () => currentSnapshot);
mock.setCommandResponse("send_read_receipt", () => currentSnapshot);
mock.setCommandResponse("set_fully_read", () => currentSnapshot);
mock.setCommandResponse("set_typing", () => currentSnapshot);
mock.setCommandResponse("set_presence", () => currentSnapshot);
mock.setCommandResponse("set_display_name", ({ displayName }: { displayName: string | null }) => {
  const normalized = displayName?.trim() ? displayName.trim() : null;
  return setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      profile: {
        ...currentSnapshot.state.profile,
        own: {
          ...currentSnapshot.state.profile.own,
          display_name: normalized
        },
        update: { kind: "idle" }
      }
    }
  });
});
mock.setCommandResponse("set_avatar", () =>
  setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      profile: {
        ...currentSnapshot.state.profile,
        own: {
          ...currentSnapshot.state.profile.own,
          avatar: {
            mxc_uri: "mxc://harness/avatar",
            thumbnail: { kind: "notRequested" }
          }
        },
        update: { kind: "idle" }
      }
    }
  })
);
mock.setCommandResponse("paginate_timeline_backwards", () => currentSnapshot);
mock.setCommandResponse("paginate_thread_timeline_backwards", () => currentSnapshot);

// Route ALL Tauri IPC through the recording mock. Plugin commands
// (window setTitle/setBadge, notifications, etc.) must NOT throw: they are
// not snapshot commands, so return a harmless value. Event-plugin commands
// (plugin:event|*) are handled internally by mockIPC's shouldMockEvents.
mockIPC(
  (cmd, args) => {
    if (cmd.startsWith("plugin:")) {
      // Window/notification/other plugin calls: return a benign value.
      return null;
    }
    return mock.invoke(cmd, (args ?? {}) as Record<string, unknown>);
  },
  { shouldMockEvents: true }
);

// getCurrentWindow() (used by the title effect) reads __TAURI_INTERNALS__.metadata.
mockWindows("main");

const harnessControl: AppHarnessControl = {
  invocations: () => mock.recordedInvocations(),
  invocationsOf: (command) => mock.invocationsOf(command),
  clearInvocations: () => mock.clearInvocations(),
  setCommandResponse: (command, response) =>
    mock.setCommandResponse(command, response),
  setSnapshot: (snapshot) => {
    setCurrentSnapshot(snapshot);
    mock.setCommandResponse("get_snapshot", () => currentSnapshot);
  },
  pushCoreEvent: (event) => emit(CORE_EVENT_NAME, event),
  pushStateChanged: () => {
    void emit(STATE_EVENT_NAME, "stateChanged");
  },
  currentSnapshot: () => currentSnapshot,
  e2eeTrustSnapshot,
  replyModeSnapshot
};
(window as unknown as { __harness: AppHarnessControl }).__harness =
  harnessControl;

// ---------------------------------------------------------------------------
// Boot: dynamically import App AFTER the mock is installed, then seed the
// timeline (one real event row so a reply target exists) and render.
// ---------------------------------------------------------------------------

async function boot() {
  const { App } = await import("../App");

  const root = document.getElementById("root");
  if (!root) {
    throw new Error("appHarness root element missing");
  }
  createRoot(root).render(<App />);

  // After the App mounts and TimelineView subscribes to matrix-desktop://event,
  // push one InitialItems batch carrying a single event-backed row so the
  // "Reply to message" action renders.
  const seedItem: TimelineItem = {
    id: { Event: { event_id: SEED_EVENT_ID } },
    sender: USER_ID,
    body: "Seed message for reply target",
    timestamp_ms: 1_800_000_000_000,
    in_reply_to_event_id: null,
    thread_root: null,
    thread_summary: {
      reply_count: 2,
      latest_sender: "@thread-user:example.invalid",
      latest_body_preview: "Thread panel reply from keyed event stream",
      latest_timestamp_ms: 1_800_000_000_200
    },
    can_react: true,
    is_redacted: false,
    can_redact: true,
    is_edited: false,
    can_edit: true,
    reactions: [
      {
        key: "👍",
        count: 1,
        reacted_by_me: false,
        my_reaction_event_id: null,
        sender_preview: ["@other-user:example.invalid"]
      }
    ]
  };
  const payload: CoreEventPayload = {
    kind: "Timeline",
    event: {
      InitialItems: {
        request_id: null,
        key: roomTimelineKey(USER_ID, ROOM_ID),
        generation: 1,
        items: [seedItem]
      }
    }
  };
  // Retry-emit until the App's listener is registered (listen() is async).
  for (let attempt = 0; attempt < 40; attempt += 1) {
    await emit(CORE_EVENT_NAME, payload);
    await new Promise((resolve) => setTimeout(resolve, 25));
    if (document.querySelector(`[data-event-id="${SEED_EVENT_ID}"]`)) {
      break;
    }
  }
}

void boot();
