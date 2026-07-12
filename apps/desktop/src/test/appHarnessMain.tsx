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
  ActivityTab,
  ComposerKeyEvent,
  ComposerResolverOptions,
  ComposerResolvedAction,
  ComposerMode,
  ComposerSurface,
  DesktopSnapshot,
  E2eeTrustState,
  LocaleDisplayProfile,
  LocaleSettings,
  RoomNotificationMode,
  RoomNotificationSettings,
  SettingsPatch,
  StagedUploadCompressionChoice,
  UploadStagingRequestItem
} from "../domain/types";
import { TauriIpcMock, type IpcInvocation } from "./tauriIpcMock";
import { computeBrowserRoomListProjection } from "../backend/roomListProjection";
import { composeSidebar } from "../domain/desktopModel";
import "../styles.css";

const CORE_EVENT_NAME = "koushi-desktop://event";
const STATE_EVENT_NAME = "koushi-desktop://state";

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
  invoke(command: string, args?: Record<string, unknown>): Promise<unknown>;
   
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
    basicOperation?: DesktopSnapshot["state"]["ui"]["basic_operation"];
    e2eeTrust?: E2eeTrustState;
    extraSpaces?: DesktopSnapshot["state"]["domain"]["spaces"];
    extraRailItems?: DesktopSnapshot["sidebar"]["space_rail"];
  } = {}
): DesktopSnapshot {
  const composerMode = overrides.composerMode ?? "Plain";
  const basicOperation = overrides.basicOperation ?? { kind: "idle" };
  const activeSpaceId = null;
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
  const rooms = [
    {
      room_id: ROOM_ID,
      display_name: ROOM_NAME,
      display_label: ROOM_NAME,
      original_display_label: ROOM_NAME,
      avatar: null,
      is_dm: false,
      dm_user_ids: [],
      tags: { favourite: null, low_priority: null },
      unread_count: 0,
      notification_count: 0,
      highlight_count: 0,
      parent_space_ids: [SPACE_ID],
      dm_space_ids: [],
      is_encrypted: false,
      joined_members: 8
    }
  ];
  const sidebar = {
    ...composeSidebar(activeSpaceId, spaces, rooms),
    account_home: {
      ...composeSidebar(activeSpaceId, spaces, rooms).account_home,
      is_active: false
    },
    space_rail: railItems
  };
      return {
      state: {
        schema_version: 2,
        domain: {
          session: { kind: "ready", homeserver: HOMESERVER, user_id: USER_ID, device_id: DEVICE_ID },
          auth: { kind: "unknown" },
          settings: defaultSettingsState(),
          link_preview_settings: { room_overrides: {} },
          room_preferences: { rooms: {} },
          locale_profile: defaultLocaleDisplayProfile(),
          typography_profile: defaultTypographyDisplayProfile(),
          profile: { own: { display_name: "Harness User", avatar: null }, users: {}, local_aliases: {}, local_alias_update: { kind: "idle" }, ignored_user_ids: [], ignored_user_update: { kind: "idle" }, update: { kind: "idle" } },
          sync: "running",
          sync_mode: { kind: "unsupported" },
          spaces, rooms, invites: [],
          invite_workflow: {
            query: { room_id: null, query: "", candidates: [], explicit_user_id: null },
            selected_targets: [],
            scope_plan: null,
            operation: { kind: "idle" }
          },
          room_notification_settings: {}, room_interactions: {},
          device_sessions: { kind: "idle" },
          account_management: { kind: "idle" },
          account_management_capabilities: { change_password: { kind: "unknown" } },
          soft_logout_reauth: { kind: "idle" }, qr_login: { kind: "idle" },
          directory: { query: { kind: "closed" }, join: { kind: "idle" } },
          room_management: { selected_room_id: null, settings: null, operation: { kind: "idle" } },
          activity: { kind: "closed" }, thread_attention: { kind: "closed" },
          search: { kind: "closed" }, search_crawler: { rooms: {}, last_active: null },
          live_signals: defaultLiveSignalsState(),
          e2ee_trust: overrides.e2eeTrust ?? defaultE2eeTrustState(),
          local_encryption: { kind: "unknown" },
          native_attention: defaultNativeAttentionState(),
          cjk_text_policy: defaultCjkTextPolicyState()
        },
        ui: {
          navigation: { active_space_id: activeSpaceId, active_room_id: ROOM_ID, space_order: spaces.map((space) => space.space_id), last_room_by_space_id: {} },
          room_list: computeBrowserRoomListProjection({ kind: "rooms" }, { kind: "activity" }, activeSpaceId, spaces, rooms, []),
          timeline: { room_id: ROOM_ID, is_subscribed: true, is_paginating_backwards: false, composer: { accepted_submission_ids: [], pending_transaction_id: null, draft: "", mode: composerMode }, submission_registry: { accepted_submission_ids: [], settled_submission_ids: [] }, scheduled_send_capability: "unknown", scheduled_sends: [], staged_uploads: [], media_gallery: [], media_downloads: {} },
          thread: { kind: "closed" }, threads_list: { kind: "closed" }, focused_context: { kind: "closed" },
          files_view: { kind: "closed" }, errors: [], basic_operation: basicOperation
        }
      },
      sidebar,
      timeline: [], thread: null
    };
}

function defaultSettingsState(): DesktopSnapshot["state"]["domain"]["settings"] {
  return {
    values: {
      locale: { language_tag: null, text_direction: "auto" },
      appearance: { theme: "system" },
      typography: { font: "system", emoji: "system" },
      keyboard: { composer_send_shortcut: "enter" },
      notifications: {
        desktop_notifications: true,
        sound: true,
        badges: true,
        send_read_receipts: true,
        send_typing_notifications: true
      },
      display: {
        code_block_wrap: true,
        hide_redacted: true,
        url_previews_enabled: true,
        encrypted_url_previews_enabled: true
      },
      media: {
        image_upload_compression: "ask",
        image_upload_compression_policy: {
          threshold_bytes: 1048576,
          threshold_long_edge: 2560,
          target_long_edge: 2048,
          quality_percent: 82
        }
      },
      timeline: {
        auto_load_older_messages: true,
        thread_root_order: { kind: "rootEvent" }
      },
      search_crawler: {
        speed: "standard",
        include_media_captions: true,
        include_filenames: true
      },
      thread_list_order: { kind: "latestReply" },
      room_list_sort: { kind: "activity" }
    },
    persistence: { kind: "idle" }
  };
}

function defaultE2eeTrustState(): DesktopSnapshot["state"]["domain"]["e2ee_trust"] {
  return {
    verification: { kind: "idle" },
    cross_signing: { kind: "unknown" },
    key_backup: { kind: "unknown" },
    identity_reset: { kind: "idle" },
    key_management: defaultE2eeKeyManagementState(),
    devices: []
  };
}

function defaultE2eeKeyManagementState(): DesktopSnapshot["state"]["domain"]["e2ee_trust"]["key_management"] {
  return {
    room_key_export: { kind: "idle" },
    room_key_import: { kind: "idle" },
    secure_backup_setup: { kind: "idle" },
    passphrase_change: { kind: "idle" }
  };
}

function defaultLiveSignalsState(): DesktopSnapshot["state"]["domain"]["live_signals"] {
  return {
    rooms: {},
    presence: {}
  };
}

function defaultNativeAttentionState(): DesktopSnapshot["state"]["domain"]["native_attention"] {
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

function defaultCjkTextPolicyState(): DesktopSnapshot["state"]["domain"]["cjk_text_policy"] {
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
  values: DesktopSnapshot["state"]["domain"]["settings"]["values"],
  patch: SettingsPatch
): DesktopSnapshot["state"]["domain"]["settings"]["values"] {
  return {
    locale: patch.locale ?? values.locale,
    appearance: patch.appearance ?? values.appearance,
    typography: patch.typography ?? values.typography,
    keyboard: patch.keyboard ?? values.keyboard,
    notifications: patch.notifications ?? values.notifications,
    display: patch.display ?? values.display,
    media: patch.media ?? values.media,
    timeline: patch.timeline ?? values.timeline,
    search_crawler: patch.search_crawler ?? values.search_crawler,
    thread_list_order: patch.thread_list_order ?? values.thread_list_order,
    room_list_sort: patch.room_list_sort ?? values.room_list_sort
  };
}

function defaultLocaleDisplayProfile(): LocaleDisplayProfile {
  return resolveLocaleDisplayProfile({ language_tag: null, text_direction: "auto" });
}

function defaultTypographyDisplayProfile(): DesktopSnapshot["state"]["domain"]["typography_profile"] {
  return resolveTypographyDisplayProfile({ font: "system", emoji: "system" });
}

function resolveTypographyDisplayProfile(
  typography: DesktopSnapshot["state"]["domain"]["settings"]["values"]["typography"]
): DesktopSnapshot["state"]["domain"]["typography_profile"] {
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
  sendShortcut: DesktopSnapshot["state"]["domain"]["settings"]["values"]["keyboard"]["composer_send_shortcut"],
  surface: ComposerSurface,
  keyEvent: ComposerKeyEvent,
  options: ComposerResolverOptions
): ComposerResolvedAction {
  void surface;
  if (keyEvent.is_composing) {
    return "commitImeCandidate";
  }
  if (keyEvent.key === "escape") {
    return options.autocomplete_open ? "closeAutocomplete" : "cancel";
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
// set_composer_reply_target response so the App sees Rust-shaped reply mode and
// routes to send_reply.
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
    key_management: defaultE2eeKeyManagementState(),
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
  snapshot.state.domain.rooms.push({
    room_id: newRoomId,
    display_name: "Created Room",
    display_label: "Created Room",
    original_display_label: "Created Room",
    avatar: null,
    is_dm: false,
    dm_user_ids: [],
    tags: { favourite: null, low_priority: null },
    unread_count: 0,
    notification_count: 0,
    highlight_count: 0,
    parent_space_ids: [],
    dm_space_ids: [],
    is_encrypted: false
  });
  snapshot.state.ui.navigation.active_room_id = newRoomId;
  snapshot.state.ui.timeline.room_id = newRoomId;
  snapshot.sidebar.space_rooms.push({
    room_id: newRoomId,
    display_name: "Created Room",
    avatar: null,
    tags: { favourite: null, low_priority: null },
    unread_count: 0,
    highlight_count: 0
  });
  snapshot.state.ui.room_list = computeBrowserRoomListProjection(
    snapshot.state.ui.room_list.active_filter,
    snapshot.state.ui.room_list.sort,
    snapshot.state.ui.navigation.active_space_id,
    snapshot.state.domain.spaces,
    snapshot.state.domain.rooms,
    snapshot.state.domain.invites
  );
  return snapshot;
}

function afterCreateSpaceSnapshot(): DesktopSnapshot {
  const snapshot = readySnapshot();
  const newSpaceId = "!created-space:example.invalid";
  snapshot.state.domain.spaces.push({
    space_id: newSpaceId,
    display_name: "Created Space",
    avatar: null,
    child_room_ids: []
  });
  snapshot.state.ui.navigation.active_space_id = newSpaceId;
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
let nextGateFlowId = 80;

function setCurrentSnapshot(next: DesktopSnapshot): DesktopSnapshot {
  const rooms = next.state.domain.rooms.map(normalizeHarnessRoomSummary);
  const spaces = next.state.domain.spaces.map((space) => ({
    ...space,
    child_room_ids: space.child_room_ids ?? []
  }));
  const invites = next.state.domain.invites ?? [];
  const roomList = next.state.ui.room_list ?? {
    active_filter: { kind: "rooms" as const },
    sort: { kind: "activity" as const },
    items: []
  };
  currentSnapshot = {
    ...next,
    state: {
      ...next.state,
      domain: {
        ...next.state.domain,
        rooms,
        spaces,
        invites
      },
      ui: {
        ...next.state.ui,
      room_list: computeBrowserRoomListProjection(
        roomList.active_filter,
        roomList.sort,
        next.state.ui.navigation.active_space_id,
        spaces,
        rooms,
        invites
      )
      },
    }
  };
  return currentSnapshot;
}

function normalizeHarnessRoomSummary(
  room: DesktopSnapshot["state"]["domain"]["rooms"][number]
): DesktopSnapshot["state"]["domain"]["rooms"][number] {
  const displayName = room.display_name ?? room.room_id;
  const displayLabel = room.display_label ?? displayName;
  return {
    ...room,
    display_name: displayName,
    display_label: displayLabel,
    original_display_label: room.original_display_label ?? displayLabel,
    avatar: room.avatar ?? null,
    is_dm: room.is_dm ?? false,
    dm_user_ids: room.dm_user_ids ?? [],
    tags: {
      favourite: room.tags?.favourite ?? null,
      low_priority: room.tags?.low_priority ?? null
    },
    unread_count: room.unread_count ?? 0,
    notification_count: room.notification_count ?? 0,
    highlight_count: room.highlight_count ?? 0,
    parent_space_ids: room.parent_space_ids ?? [],
    dm_space_ids: room.dm_space_ids ?? [],
    is_encrypted: room.is_encrypted ?? false
  };
}

function normalizeHarnessCommandResponse(value: unknown): unknown {
  if (isPromiseLike(value)) {
    return value.then(normalizeHarnessCommandResponse);
  }
  if (isDesktopSnapshotLike(value)) {
    return setCurrentSnapshot(value);
  }
  return value;
}

function isPromiseLike(value: unknown): value is Promise<unknown> {
  return (
    typeof value === "object" &&
    value !== null &&
    "then" in value &&
    typeof (value as { then?: unknown }).then === "function"
  );
}

function isDesktopSnapshotLike(value: unknown): value is DesktopSnapshot {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const candidate = value as Partial<DesktopSnapshot> & {
    state?: {
      schema_version?: unknown;
      domain?: {
        rooms?: unknown;
        spaces?: unknown;
        invites?: unknown;
      };
      ui?: unknown;
    };
  };
  return Boolean(
    candidate.state?.schema_version === 2 &&
      candidate.sidebar &&
      candidate.state?.ui &&
      Array.isArray(candidate.state.domain?.rooms) &&
      Array.isArray(candidate.state.domain?.spaces) &&
      Array.isArray(candidate.state.domain?.invites)
  );
}

// Snapshot-returning commands the App calls. Default snapshot stays ready so
// any unanticipated snapshot read still renders the shell.
mock.setCommandResponse("get_snapshot", () => currentSnapshot);
mock.setCommandResponse("list_saved_sessions", () => []);
mock.setCommandResponse("logout", () => {
  const next = structuredClone(currentSnapshot);
  next.state.domain.session = { kind: "signedOut" };
  return setCurrentSnapshot(next);
});
mock.setCommandResponse("submit_recovery", () => {
  const next = structuredClone(currentSnapshot);
  const session = next.state.domain.session;
  if (session.kind === "awaitingVerification" || session.kind === "verifying") next.state.domain.session = { ...session, kind: "verifying", method: "recoveryKey", flow_id: session.flow_id ?? 72 };
  return setCurrentSnapshot(next);
});
mock.setCommandResponse("select_space", ({ spaceId }: { spaceId: string | null }) => {
  const nextSpaceId =
    spaceId && currentSnapshot.state.domain.spaces.some((space) => space.space_id === spaceId)
      ? spaceId
      : null;
  const activeRoomId =
    nextSpaceId
      ? currentSnapshot.state.domain.spaces
          .find((space) => space.space_id === nextSpaceId)
          ?.child_room_ids.find((roomId) =>
            currentSnapshot.state.domain.rooms.some((room) => room.room_id === roomId)
          ) ?? currentSnapshot.state.ui.navigation.active_room_id
      : currentSnapshot.state.ui.navigation.active_room_id;
  return setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      domain: {
        ...currentSnapshot.state.domain,
        activity: { kind: "closed" }
      },
      ui: {
        ...currentSnapshot.state.ui,
        navigation: {
          ...currentSnapshot.state.ui.navigation,
          active_space_id: nextSpaceId,
          active_room_id: activeRoomId
        },
        timeline: {
          ...currentSnapshot.state.ui.timeline,
          room_id: activeRoomId,
          is_subscribed: Boolean(activeRoomId)
        }
      }
    },
    sidebar: composeSidebar(
      nextSpaceId,
      currentSnapshot.state.domain.spaces,
      currentSnapshot.state.domain.rooms
    )
  });
});
mock.setCommandResponse("select_room", ({ roomId }: { roomId: string }) => {
  const room = currentSnapshot.state.domain.rooms.find((candidate) => candidate.room_id === roomId);
  if (!room) {
    return currentSnapshot;
  }
  const activeSpaceId = room.is_dm
    ? null
    : (room.parent_space_ids[0] ?? currentSnapshot.state.ui.navigation.active_space_id);
  return setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      domain: {
        ...currentSnapshot.state.domain,
        activity: { kind: "closed" },
        thread_attention: { kind: "closed" }
      },
      ui: {
        ...currentSnapshot.state.ui,
        navigation: {
          ...currentSnapshot.state.ui.navigation,
          active_space_id: activeSpaceId,
          active_room_id: roomId
        },
        timeline: {
          ...currentSnapshot.state.ui.timeline,
          room_id: roomId,
          is_subscribed: true,
          composer: {
            accepted_submission_ids: [],
            pending_transaction_id: null,
            draft: "",
            mode: "Plain"
          }
        },
        thread: { kind: "closed" },
        threads_list: { kind: "closed" },
        focused_context: { kind: "closed" }
      }
    },
    sidebar: composeSidebar(
      activeSpaceId,
      currentSnapshot.state.domain.spaces,
      currentSnapshot.state.domain.rooms
    ),
    thread: null
  });
});
mock.setCommandResponse("open_activity", () =>
  setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      domain: {
        ...currentSnapshot.state.domain,
        activity: {
          kind: "open",
          active_tab: "recent",
          recent: { rows: [], next_batch: null },
          unread: { rows: [], next_batch: null },
          mark_read: { kind: "idle" }
        }
      }
    }
  })
);
mock.setCommandResponse("close_activity", () =>
  setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      domain: {
        ...currentSnapshot.state.domain,
        activity: { kind: "closed" }
      }
    }
  })
);
mock.setCommandResponse("set_activity_tab", ({ tab }: { tab: ActivityTab }) => {
  if (currentSnapshot.state.domain.activity.kind !== "open") {
    return currentSnapshot;
  }
  return setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      domain: {
        ...currentSnapshot.state.domain,
        activity: {
          ...currentSnapshot.state.domain.activity,
          active_tab: tab
        }
      }
    }
  });
});
mock.setCommandResponse("paginate_activity", () => currentSnapshot);
mock.setCommandResponse("mark_activity_read", () => currentSnapshot);
mock.setCommandResponse("reorder_spaces", ({ spaceIds }: { spaceIds: string[] }) => {
  const positionBySpaceId = new Map(spaceIds.map((spaceId, index) => [spaceId, index]));
  return setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      ui: {
        ...currentSnapshot.state.ui,
        navigation: {
          ...currentSnapshot.state.ui.navigation,
          space_order: [...spaceIds]
        }
      },
      domain: {
        ...currentSnapshot.state.domain,
        spaces: [...currentSnapshot.state.domain.spaces].sort(
          (left, right) =>
            (positionBySpaceId.get(left.space_id) ?? Number.MAX_SAFE_INTEGER) -
            (positionBySpaceId.get(right.space_id) ?? Number.MAX_SAFE_INTEGER)
        )
      }
    },
    sidebar: {
      ...currentSnapshot.sidebar,
      space_rail: [...currentSnapshot.sidebar.space_rail].sort(
        (left, right) =>
          (positionBySpaceId.get(left.space_id) ?? Number.MAX_SAFE_INTEGER) -
          (positionBySpaceId.get(right.space_id) ?? Number.MAX_SAFE_INTEGER)
      )
    }
  });
});
mock.setCommandResponse("update_settings", ({ patch }: { patch: SettingsPatch }) => {
  const values = applySettingsPatch(currentSnapshot.state.domain.settings.values, patch);
  return setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      domain: {
        ...currentSnapshot.state.domain,
      settings: {
        ...currentSnapshot.state.domain.settings,
        values,
        persistence: { kind: "idle" }
      },
      locale_profile: resolveLocaleDisplayProfile(values.locale),
      typography_profile: resolveTypographyDisplayProfile(values.typography)
      },
    }
  });
});
mock.setCommandResponse(
  "set_room_url_preview_override",
  ({ roomId, enabled }: { roomId: string; enabled: boolean }) => {
    const room = currentSnapshot.state.domain.rooms.find((candidate) => candidate.room_id === roomId);
    if (!room) {
      return currentSnapshot;
    }
    const roomOverrides = { ...currentSnapshot.state.domain.link_preview_settings.room_overrides };
    const defaultEnabled = room.is_encrypted
      ? currentSnapshot.state.domain.settings.values.display.encrypted_url_previews_enabled
      : currentSnapshot.state.domain.settings.values.display.url_previews_enabled;
    const roomPreferences = { ...currentSnapshot.state.domain.room_preferences.rooms };
    const preference = { ...roomPreferences[roomId] };
    if (enabled === defaultEnabled) {
      delete roomOverrides[roomId];
      delete preference.url_previews_enabled_override;
    } else {
      roomOverrides[roomId] = enabled;
      preference.url_previews_enabled_override = enabled;
    }
    if (
      preference.url_previews_enabled_override === undefined &&
      preference.notification_mode === undefined
    ) {
      delete roomPreferences[roomId];
    } else {
      roomPreferences[roomId] = preference;
    }
    return setCurrentSnapshot({
      ...currentSnapshot,
      state: {
        ...currentSnapshot.state,
        domain: {
          ...currentSnapshot.state.domain,
          link_preview_settings: {
            room_overrides: roomOverrides
          },
          room_preferences: {
            rooms: roomPreferences
          }
        },
      }
    });
  }
);
mock.setCommandResponse(
  "select_room_list_filter",
  ({ filter }: { filter: DesktopSnapshot["state"]["ui"]["room_list"]["active_filter"] }) =>
    setCurrentSnapshot({
      ...currentSnapshot,
      state: {
        ...currentSnapshot.state,
        ui: {
          ...currentSnapshot.state.ui,
        room_list: computeBrowserRoomListProjection(
          filter,
          currentSnapshot.state.ui.room_list.sort,
          currentSnapshot.state.ui.navigation.active_space_id,
          currentSnapshot.state.domain.spaces,
          currentSnapshot.state.domain.rooms,
          currentSnapshot.state.domain.invites
        )
        },
      }
    })
);
mock.setCommandResponse("mark_room_as_read", () => currentSnapshot);
mock.setCommandResponse("mark_room_as_unread", () => currentSnapshot);
mock.setCommandResponse("leave_room", ({ roomId }: { roomId: string }) => {
  const removedSpace = currentSnapshot.state.domain.spaces.find((space) => space.space_id === roomId);
  const nextSpaces = currentSnapshot.state.domain.spaces.filter((space) => space.space_id !== roomId);
  const nextRooms = removedSpace
    ? currentSnapshot.state.domain.rooms.map((room) => ({
        ...room,
        parent_space_ids: room.parent_space_ids.filter((spaceId) => spaceId !== roomId)
      }))
    : currentSnapshot.state.domain.rooms.filter((room) => room.room_id !== roomId);
  const nextActiveSpaceId =
    currentSnapshot.state.ui.navigation.active_space_id === roomId
      ? null
      : currentSnapshot.state.ui.navigation.active_space_id;
  const nextLastRoomBySpaceId = {
    ...(currentSnapshot.state.ui.navigation.last_room_by_space_id ?? {})
  };
  delete nextLastRoomBySpaceId[roomId];
  return setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      ui: {
        ...currentSnapshot.state.ui,
        navigation: {
          ...currentSnapshot.state.ui.navigation,
          active_space_id: nextActiveSpaceId,
          active_room_id:
            removedSpace || currentSnapshot.state.ui.navigation.active_room_id !== roomId
              ? currentSnapshot.state.ui.navigation.active_room_id
              : null,
          space_order:
            currentSnapshot.state.ui.navigation.space_order?.filter((spaceId) => spaceId !== roomId) ??
            [],
          last_room_by_space_id: nextLastRoomBySpaceId
        },
        room_list: computeBrowserRoomListProjection(
          currentSnapshot.state.ui.room_list.active_filter,
          currentSnapshot.state.ui.room_list.sort,
          nextActiveSpaceId,
          nextSpaces,
          nextRooms,
          currentSnapshot.state.domain.invites
        )
      },
      domain: {
        ...currentSnapshot.state.domain,
        spaces: nextSpaces,
        rooms: nextRooms
      }
    },
    sidebar: {
      ...currentSnapshot.sidebar,
      active_space_id: nextActiveSpaceId,
      space_rail: currentSnapshot.sidebar.space_rail.filter((space) => space.space_id !== roomId),
      space_rooms: removedSpace
        ? currentSnapshot.sidebar.space_rooms
        : currentSnapshot.sidebar.space_rooms.filter((room) => room.room_id !== roomId)
    }
  });
});
mock.setCommandResponse(
  "set_room_notification_mode",
  ({ roomId, mode }: { roomId: string; mode: RoomNotificationMode }) => {
    const known =
      currentSnapshot.state.domain.rooms.some((room) => room.room_id === roomId) ||
      currentSnapshot.state.domain.invites.some((invite) => invite.room_id === roomId);
    if (!known) {
      return currentSnapshot;
    }
    const next: Record<string, RoomNotificationSettings> = {
      ...currentSnapshot.state.domain.room_notification_settings,
      [roomId]: { mode, operation: { kind: "idle" } }
    };
    const roomPreferences = { ...currentSnapshot.state.domain.room_preferences.rooms };
    const preference = { ...roomPreferences[roomId] };
    if (mode.kind === "all") {
      delete preference.notification_mode;
    } else {
      preference.notification_mode = mode;
    }
    if (
      preference.url_previews_enabled_override === undefined &&
      preference.notification_mode === undefined
    ) {
      delete roomPreferences[roomId];
    } else {
      roomPreferences[roomId] = preference;
    }
    return setCurrentSnapshot({
      ...currentSnapshot,
      state: {
        ...currentSnapshot.state,
        domain: {
          ...currentSnapshot.state.domain,
          room_notification_settings: next,
          room_preferences: {
            rooms: roomPreferences
          }
        },
      }
    });
  }
);
mock.setCommandResponse("query_devices", () =>
  setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      domain: {
        ...currentSnapshot.state.domain,
      device_sessions: {
        kind: "loaded",
        devices: [
          {
            device_ordinal: 1,
            display_name: "Current session",
            current: true,
            verified: true,
            inactive: false
          },
          {
            device_ordinal: 2,
            display_name: "Other session",
            current: false,
            verified: false,
            inactive: true
          }
        ]
      }
      },
    }
  })
);
mock.setCommandResponse(
  "rename_device",
  ({ deviceOrdinal, displayName }: { deviceOrdinal: number; displayName: string }) => {
    if (currentSnapshot.state.domain.device_sessions.kind !== "loaded") {
      return currentSnapshot;
    }
    return setCurrentSnapshot({
      ...currentSnapshot,
      state: {
        ...currentSnapshot.state,
        domain: {
          ...currentSnapshot.state.domain,
        device_sessions: {
          ...currentSnapshot.state.domain.device_sessions,
          devices: currentSnapshot.state.domain.device_sessions.devices.map((device) =>
            device.device_ordinal === deviceOrdinal
              ? { ...device, display_name: displayName }
              : device
          )
        }
        },
      }
    });
  }
);
mock.setCommandResponse(
  "delete_devices",
  ({ deviceOrdinals }: { deviceOrdinals: number[] }) => {
    if (currentSnapshot.state.domain.device_sessions.kind !== "loaded") {
      return currentSnapshot;
    }
    return setCurrentSnapshot({
      ...currentSnapshot,
      state: {
        ...currentSnapshot.state,
        domain: {
          ...currentSnapshot.state.domain,
        device_sessions: {
          ...currentSnapshot.state.domain.device_sessions,
          devices: currentSnapshot.state.domain.device_sessions.devices.filter(
            (device) => !deviceOrdinals.includes(device.device_ordinal)
          )
        }
        },
      }
    });
  }
);
mock.setCommandResponse(
  "submit_account_management_uia",
  ({ flowId }: { flowId: number; password: string }) => {
    void flowId;
    return setCurrentSnapshot({
      ...currentSnapshot,
      state: {
        ...currentSnapshot.state,
        domain: {
          ...currentSnapshot.state.domain,
        account_management: { kind: "idle" }
        },
      }
    });
  }
);
mock.setCommandResponse("load_account_management_capabilities", () =>
  setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      domain: {
        ...currentSnapshot.state.domain,
      account_management_capabilities: {
        change_password: { kind: "enabled" }
      }
      },
    }
  })
);
mock.setCommandResponse(
  "change_password",
  ({ newPassword }: { newPassword: string }) => {
    void newPassword;
    return setCurrentSnapshot({
      ...currentSnapshot,
      state: {
        ...currentSnapshot.state,
        domain: {
          ...currentSnapshot.state.domain,
        account_management: {
          kind: "succeeded",
          request_id: 1,
          operation: "changePassword"
        }
        },
      }
    });
  }
);
mock.setCommandResponse(
  "deactivate_account",
  ({ eraseData }: { eraseData: boolean }) => {
    void eraseData;
    return setCurrentSnapshot({
      ...currentSnapshot,
      state: {
        ...currentSnapshot.state,
        domain: {
          ...currentSnapshot.state.domain,
        account_management: {
          kind: "succeeded",
          request_id: 2,
          operation: "deactivateAccount"
        }
        },
      }
    });
  }
);
mock.setCommandResponse("probe_local_encryption_health", () =>
  setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      domain: {
        ...currentSnapshot.state.domain,
      local_encryption: { kind: "healthy" }
      },
    }
  })
);
mock.setCommandResponse("reset_local_data", () =>
  setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      domain: {
        ...currentSnapshot.state.domain,
      session: { kind: "signedOut" },
      sync: "stopped",
      local_encryption: { kind: "unknown" }
      },
    }
  })
);
mock.setCommandResponse("bootstrap_cross_signing", () =>
  setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      domain: {
        ...currentSnapshot.state.domain,
      e2ee_trust: {
        ...currentSnapshot.state.domain.e2ee_trust,
        cross_signing: { kind: "trusted" }
      }
      },
    }
  })
);
mock.setCommandResponse("enable_key_backup", () =>
  setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      domain: {
        ...currentSnapshot.state.domain,
      e2ee_trust: {
        ...currentSnapshot.state.domain.e2ee_trust,
        key_backup: { kind: "enabled", version: "harness-backup" }
      }
      },
    }
  })
);
mock.setCommandResponse("export_room_keys", () =>
  setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      domain: {
        ...currentSnapshot.state.domain,
      e2ee_trust: {
        ...currentSnapshot.state.domain.e2ee_trust,
        key_management: {
          ...currentSnapshot.state.domain.e2ee_trust.key_management,
          room_key_export: {
            kind: "exported",
            request_id: 9_200,
            exported_sessions: null
          }
        }
      }
      },
    }
  })
);
mock.setCommandResponse("import_room_keys", () =>
  setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      domain: {
        ...currentSnapshot.state.domain,
      e2ee_trust: {
        ...currentSnapshot.state.domain.e2ee_trust,
        key_management: {
          ...currentSnapshot.state.domain.e2ee_trust.key_management,
          room_key_import: {
            kind: "imported",
            request_id: 9_201,
            imported_count: 1,
            total_count: 1
          }
        }
      }
      },
    }
  })
);
mock.setCommandResponse(
  "bootstrap_secure_backup",
  ({ recoveryKeyDestinationPath }: { recoveryKeyDestinationPath?: string | null }) =>
    setCurrentSnapshot({
      ...currentSnapshot,
      state: {
        ...currentSnapshot.state,
        domain: {
          ...currentSnapshot.state.domain,
        e2ee_trust: {
          ...currentSnapshot.state.domain.e2ee_trust,
          key_management: {
            ...currentSnapshot.state.domain.e2ee_trust.key_management,
            secure_backup_setup: {
              kind: "recoveryKeyReady",
              request_id: 9_202,
              delivery: recoveryKeyDestinationPath?.trim()
                ? { kind: "written" }
                : { kind: "notWritten" }
            }
          }
        }
        },
      }
    })
);
mock.setCommandResponse(
  "change_secure_backup_passphrase",
  ({ recoveryKeyDestinationPath }: { recoveryKeyDestinationPath?: string | null }) =>
    setCurrentSnapshot({
      ...currentSnapshot,
      state: {
        ...currentSnapshot.state,
        domain: {
          ...currentSnapshot.state.domain,
        e2ee_trust: {
          ...currentSnapshot.state.domain.e2ee_trust,
          key_management: {
            ...currentSnapshot.state.domain.e2ee_trust.key_management,
            passphrase_change: {
              kind: "changed",
              request_id: 9_203,
              delivery: recoveryKeyDestinationPath?.trim()
                ? { kind: "written" }
                : { kind: "notWritten" }
            }
          }
        }
        },
      }
    })
);
mock.setCommandResponse("accept_verification", ({ flowId }: { flowId: number }) => {
  const verification = currentSnapshot.state.domain.e2ee_trust.verification;
  if (verification.kind !== "requested" || verification.request_id !== flowId) {
    return currentSnapshot;
  }
  return setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      domain: {
        ...currentSnapshot.state.domain,
      e2ee_trust: {
        ...currentSnapshot.state.domain.e2ee_trust,
        verification: {
          kind: "accepted",
          request_id: flowId,
          target: verification.target
        }
      }
      },
    }
  });
});
mock.setCommandResponse("start_own_user_sas", () => {
  const flowId = nextGateFlowId++;
  const next = structuredClone(currentSnapshot);
  const session = next.state.domain.session;
  if (session.kind === "awaitingVerification") next.state.domain.session = { ...session, kind: "verifying", method: "existingDeviceSas", flow_id: flowId, sas_emojis: [] };
  return setCurrentSnapshot(next);
});
mock.setCommandResponse("retry_current_device_trust_discovery", () => {
  const next = structuredClone(currentSnapshot);
  const session = next.state.domain.session;
  if (session.kind === "awaitingVerification" || session.kind === "provisional") next.state.domain.session = { ...session, kind: "provisional", phase: { recheckingTrust: {} } };
  return setCurrentSnapshot(next);
});
mock.setCommandResponse("mismatch_sas_verification", ({ flowId }: { flowId: number }) => {
  const next = structuredClone(currentSnapshot);
  const session = next.state.domain.session;
  if (session.flow_id === flowId && session.gate) next.state.domain.session = { ...session, kind: "awaitingVerification", gate: { ...session.gate, failureKind: "mismatch" }, method: undefined, flow_id: undefined };
  return setCurrentSnapshot(next);
});
mock.setCommandResponse("start_session_bootstrap", ({ recoveryKeyDestinationPath }: { recoveryKeyDestinationPath: string }) => {
  const flowId = nextGateFlowId++;
  const next = structuredClone(currentSnapshot);
  const session = next.state.domain.session;
  if (session.gate && recoveryKeyDestinationPath.trim()) next.state.domain.session = { ...session, kind: "awaitingBootstrapConfirmation", flow_id: flowId, destination_written: true };
  return setCurrentSnapshot(next);
});
mock.setCommandResponse("confirm_session_bootstrap_saved", ({ flowId }: { flowId: number }) => {
  const next = structuredClone(currentSnapshot);
  const session = next.state.domain.session;
  if (session.kind === "awaitingBootstrapConfirmation" && session.flow_id === flowId) next.state.domain.session = { ...session, kind: "provisional", phase: { recheckingTrust: {} }, flow_id: undefined, destination_written: undefined };
  return setCurrentSnapshot(next);
});
mock.setCommandResponse("confirm_sas_verification", ({ flowId }: { flowId: number }) => {
  const session = currentSnapshot.state.domain.session;
  if (session.kind === "verifying" && session.method === "existingDeviceSas" && session.flow_id === flowId) {
    const next = structuredClone(currentSnapshot);
    next.state.domain.session = { ...session, kind: "provisional", phase: { recheckingTrust: { failureKind: null } }, method: undefined, flow_id: undefined, sas_emojis: undefined };
    next.state.domain.e2ee_trust.verification = { kind: "idle" };
    return setCurrentSnapshot(next);
  }
  const verification = currentSnapshot.state.domain.e2ee_trust.verification;
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
      domain: {
        ...currentSnapshot.state.domain,
      e2ee_trust: {
        ...currentSnapshot.state.domain.e2ee_trust,
        verification: {
          kind: "done",
          request_id: flowId,
          target: verification.target
        }
      }
      },
    }
  });
});
mock.setCommandResponse("cancel_verification", ({ flowId }: { flowId: number }) => {
  const verification = currentSnapshot.state.domain.e2ee_trust.verification;
  if (verification.kind === "idle" || verification.request_id !== flowId) {
    return currentSnapshot;
  }
  return setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      domain: {
        ...currentSnapshot.state.domain,
      e2ee_trust: {
        ...currentSnapshot.state.domain.e2ee_trust,
        verification: { kind: "idle" }
      }
      },
    }
  });
});
mock.setCommandResponse("reset_identity", () =>
  setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      domain: {
        ...currentSnapshot.state.domain,
      e2ee_trust: {
        ...currentSnapshot.state.domain.e2ee_trust,
        identity_reset: {
          kind: "awaitingAuth",
          request_id: 9_100,
          auth_type: "uiaa"
        }
      }
      },
    }
  })
);
mock.setCommandResponse("cancel_identity_reset", ({ flowId }: { flowId: number }) => {
  const identityReset = currentSnapshot.state.domain.e2ee_trust.identity_reset;
  if (identityReset.kind !== "awaitingAuth" || identityReset.request_id !== flowId) {
    return currentSnapshot;
  }
  return setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      domain: {
        ...currentSnapshot.state.domain,
        e2ee_trust: {
          ...currentSnapshot.state.domain.e2ee_trust,
          cross_signing: {
            kind: "failed",
            request_id: flowId,
            failureKind: "cancelled"
          },
          identity_reset: {
            kind: "failed",
            request_id: flowId,
            failureKind: "cancelled"
          }
        }
      },
    }
  });
});
mock.setCommandResponse(
  "submit_identity_reset_password",
  ({ flowId }: { flowId: number; password: string }) => {
    const identityReset = currentSnapshot.state.domain.e2ee_trust.identity_reset;
    if (identityReset.kind !== "awaitingAuth" || identityReset.request_id !== flowId) {
      return currentSnapshot;
    }
    return setCurrentSnapshot({
      ...currentSnapshot,
      state: {
        ...currentSnapshot.state,
        domain: {
          ...currentSnapshot.state.domain,
        e2ee_trust: {
          ...currentSnapshot.state.domain.e2ee_trust,
          cross_signing: { kind: "missing" },
          key_backup: { kind: "disabled" },
          identity_reset: { kind: "idle" },
          devices: currentSnapshot.state.domain.e2ee_trust.devices.map((device) => ({
            ...device,
            trust_level: "unverified"
          }))
        }
        },
      }
    });
  }
);
mock.setCommandResponse("submit_identity_reset_oauth", ({ flowId }: { flowId: number }) => {
  const identityReset = currentSnapshot.state.domain.e2ee_trust.identity_reset;
  if (identityReset.kind !== "awaitingAuth" || identityReset.request_id !== flowId) {
    return currentSnapshot;
  }
  return setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      domain: {
        ...currentSnapshot.state.domain,
      e2ee_trust: {
        ...currentSnapshot.state.domain.e2ee_trust,
        cross_signing: { kind: "missing" },
        key_backup: { kind: "disabled" },
        identity_reset: { kind: "idle" },
        devices: currentSnapshot.state.domain.e2ee_trust.devices.map((device) => ({
          ...device,
          trust_level: "unverified"
        }))
      }
      },
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
      currentSnapshot.state.domain.settings.values.keyboard.composer_send_shortcut,
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
mock.setCommandResponse("open_invite_workflow", () => currentSnapshot);
mock.setCommandResponse("close_invite_workflow", () => currentSnapshot);
mock.setCommandResponse("search_invite_targets", () => currentSnapshot);
mock.setCommandResponse("select_invite_target", () => currentSnapshot);
mock.setCommandResponse("remove_invite_target", () => currentSnapshot);
mock.setCommandResponse("invite_targets", () => currentSnapshot);
mock.setCommandResponse("set_composer_draft", ({ roomId, draft }: { roomId: string; draft: string }) => {
  if (currentSnapshot.state.ui.timeline.room_id !== roomId) {
    return currentSnapshot;
  }
  return setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      ui: {
        ...currentSnapshot.state.ui,
      timeline: {
        ...currentSnapshot.state.ui.timeline,
        composer: {
          ...currentSnapshot.state.ui.timeline.composer,
          draft
        }
      }
      },
    }
  });
});
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
        ui: {
          ...currentSnapshot.state.ui,
          thread: {
            kind: "open",
            room_id: roomId,
            root_event_id: rootEventId,
            is_subscribed: true,
          composer: {
            accepted_submission_ids: [],
            pending_transaction_id: null,
              draft: "",
              mode: "Plain"
            }
          }
        },
        domain: {
          ...currentSnapshot.state.domain,
          thread_attention: {
            kind: "tracking",
            room_id: roomId,
            root_event_id: rootEventId,
            notification_count: 0,
            highlight_count: 0,
            live_event_marker_count: 0
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
    const thread = currentSnapshot.state.ui.thread;
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
        ui: {
          ...currentSnapshot.state.ui,
        thread: {
          ...thread,
          composer: {
            ...thread.composer,
            draft
          }
        }
        },
      }
    };
    return setCurrentSnapshot(next);
  }
);
mock.setCommandResponse(
  "send_thread_reply",
  ({ roomId, rootEventId, body }: { roomId: string; rootEventId: string; body: string }) => {
    const thread = currentSnapshot.state.ui.thread;
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
        ui: {
          ...currentSnapshot.state.ui,
        thread: {
          ...thread,
          composer: {
            ...thread.composer,
            draft: "",
            pending_transaction_id: null
          }
        }
        },
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
      ui: {
        ...currentSnapshot.state.ui,
        thread: { kind: "closed" }
      },
      domain: {
        ...currentSnapshot.state.domain,
        thread_attention: { kind: "closed" }
      }
    },
    thread: null
  };
  return setCurrentSnapshot(next);
});
mock.setCommandResponse("submit_search", ({ query }: { query?: string }) => {
  const next = readySnapshot();
  next.state.domain.search = {
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
        ui: {
          ...currentSnapshot.state.ui,
          navigation: {
            ...currentSnapshot.state.ui.navigation,
            active_room_id: roomId,
            main_timeline_anchor: { event_id: eventId }
          },
          timeline: {
            ...currentSnapshot.state.ui.timeline,
            room_id: roomId,
            is_subscribed: true
          },
          thread: { kind: "closed" },
          focused_context: { kind: "closed" }
        },
        domain: {
          ...currentSnapshot.state.domain,
          thread_attention: { kind: "closed" }
        }
      }
    };
    return setCurrentSnapshot(next);
  }
);
mock.setCommandResponse("close_search", () => {
  const next: DesktopSnapshot = {
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      domain: {
        ...currentSnapshot.state.domain,
        search: { kind: "closed" }
      }
    }
  };
  return setCurrentSnapshot(next);
});
mock.setCommandResponse(
  "open_activity_event",
  ({ roomId, eventId }: { roomId: string; eventId: string }) => {
    const next: DesktopSnapshot = {
      ...currentSnapshot,
      state: {
        ...currentSnapshot.state,
        ui: {
          ...currentSnapshot.state.ui,
          navigation: {
            ...currentSnapshot.state.ui.navigation,
            active_room_id: roomId,
            room_scroll_anchors: {
              ...(currentSnapshot.state.ui.navigation.room_scroll_anchors ?? {}),
              [roomId]: {
                event_id: eventId,
                edge: "bottom",
                offset_px: 0,
                updated_at_ms: Date.now()
              }
            }
          },
          timeline: {
            ...currentSnapshot.state.ui.timeline,
            room_id: roomId,
            is_subscribed: true
          },
          thread: { kind: "closed" },
          focused_context: { kind: "closed" }
        },
        domain: {
          ...currentSnapshot.state.domain,
          thread_attention: { kind: "closed" }
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
      ui: {
        ...currentSnapshot.state.ui,
      focused_context: { kind: "closed" }
      },
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
  const room = currentSnapshot.state.domain.rooms.find((candidate) => candidate.room_id === roomId);
  const next: DesktopSnapshot = {
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      domain: {
        ...currentSnapshot.state.domain,
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
            can_edit_roles: true,
            can_kick: true,
            can_ban: true,
            can_unban: true
          },
          members: [
            {
              user_id: "@harness-ada:example.invalid",
              display_name: "Harness Ada",
              display_label: "Harness Ada",
              original_display_label: "Harness Ada",
              avatar_url: null,
              power_level: 100,
              role: "administrator"
            },
            {
              user_id: "@harness-grace:example.invalid",
              display_name: "Harness Grace",
              display_label: "Harness Grace",
              original_display_label: "Harness Grace",
              avatar_url: null,
              power_level: 50,
              role: "moderator"
            },
            {
              user_id: "@harness-linus:example.invalid",
              display_name: "Harness Linus",
              display_label: "Harness Linus",
              original_display_label: "Harness Linus",
              avatar_url: null,
              power_level: 0,
              role: "user"
            }
          ]
        },
        operation: { kind: "idle" }
      }
      },
    }
  };
  return setCurrentSnapshot(next);
});
mock.setCommandResponse("start_room_crawl", ({ roomId }: { roomId: string }) => {
  return setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      domain: {
        ...currentSnapshot.state.domain,
        search_crawler: {
          rooms: {
            ...currentSnapshot.state.domain.search_crawler.rooms,
            [roomId]: { kind: "queued" }
          },
          last_active: {
            room_id: roomId,
            updated_at_ms: Date.now(),
            status: "queued",
            processed: 0,
            indexed: 0
          }
        }
      },
    }
  });
});
mock.setCommandResponse("stop_room_crawl", ({ roomId }: { roomId: string }) => {
  // Transition to idle (matching Rust contract) so the status row stays visible
  // with a Start button instead of disappearing from the list.
  return setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      domain: {
        ...currentSnapshot.state.domain,
        search_crawler: {
          rooms: {
            ...currentSnapshot.state.domain.search_crawler.rooms,
            [roomId]: { kind: "idle" }
          },
          last_active: currentSnapshot.state.domain.search_crawler.last_active
        }
      },
    }
  });
});
mock.setCommandResponse("update_room_setting", () => currentSnapshot);
mock.setCommandResponse("moderate_room_member", () => currentSnapshot);
mock.setCommandResponse("update_room_member_role", () => currentSnapshot);
mock.setCommandResponse("pin_event", () => currentSnapshot);
mock.setCommandResponse("unpin_event", () => currentSnapshot);
mock.setCommandResponse(
  "stage_uploads",
  ({ roomId, items }: { roomId: string; items: UploadStagingRequestItem[] }) => {
  const stagedUploads = (items ?? []).map((item, index: number) => ({
    staged_id: item.stagedId,
    room_id: roomId,
    position: item.position ?? index + 1,
    filename: item.filename,
    mime_type: item.mimeType,
    byte_count: item.byteCount,
    kind: item.kind,
    caption: null,
    compression_choice: item.compressionChoice
  }));
  return setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      ui: {
        ...currentSnapshot.state.ui,
      timeline: {
        ...currentSnapshot.state.ui.timeline,
        staged_uploads: stagedUploads
      }
      },
    }
  });
});
mock.setCommandResponse("update_staged_upload_caption", ({ stagedId, caption }: {
  stagedId: string;
  caption: string | null;
}) => {
  const normalized = typeof caption === "string" && caption.trim() ? caption : null;
  return setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      ui: {
        ...currentSnapshot.state.ui,
      timeline: {
        ...currentSnapshot.state.ui.timeline,
        staged_uploads: currentSnapshot.state.ui.timeline.staged_uploads.map((item) =>
          item.staged_id === stagedId
            ? {
                ...item,
                caption: normalized
                  ? { plain_body: normalized, formatted_body: null, mentions: { targets: [] } }
                  : null
              }
            : item
        )
      }
      },
    }
  });
});
mock.setCommandResponse("update_staged_upload_compression", ({ stagedId, compressionChoice }: {
  stagedId: string;
  compressionChoice: StagedUploadCompressionChoice;
}) =>
  setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      ui: {
        ...currentSnapshot.state.ui,
      timeline: {
        ...currentSnapshot.state.ui.timeline,
        staged_uploads: currentSnapshot.state.ui.timeline.staged_uploads.map((item) =>
          item.staged_id === stagedId ? { ...item, compression_choice: compressionChoice } : item
        )
      }
      },
    }
  })
);
mock.setCommandResponse("clear_upload_staging", () =>
  setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      ui: {
        ...currentSnapshot.state.ui,
      timeline: {
        ...currentSnapshot.state.ui.timeline,
        staged_uploads: []
      }
      },
    }
  })
);
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
      domain: {
        ...currentSnapshot.state.domain,
      profile: {
        ...currentSnapshot.state.domain.profile,
        own: {
          ...currentSnapshot.state.domain.profile.own,
          display_name: normalized
        },
        update: { kind: "idle" }
      }
      },
    }
  });
});
mock.setCommandResponse(
  "set_local_user_alias",
  ({ userId, alias }: { userId: string; alias: string | null }) => {
    const normalizedUserId = userId.trim();
    const normalizedAlias = alias?.trim() ? alias.trim() : null;
    if (!normalizedUserId) {
      return currentSnapshot;
    }
    return setCurrentSnapshot(projectAliasSnapshot(currentSnapshot, normalizedUserId, normalizedAlias));
  }
);
mock.setCommandResponse("set_avatar", () =>
  setCurrentSnapshot({
    ...currentSnapshot,
    state: {
      ...currentSnapshot.state,
      domain: {
        ...currentSnapshot.state.domain,
      profile: {
        ...currentSnapshot.state.domain.profile,
        own: {
          ...currentSnapshot.state.domain.profile.own,
          avatar: {
            mxc_uri: "mxc://harness/avatar",
            thumbnail: { kind: "notRequested" }
          }
        },
        update: { kind: "idle" }
      }
      },
    }
  })
);
mock.setCommandResponse("paginate_timeline_backwards", () => currentSnapshot);
mock.setCommandResponse("paginate_thread_timeline_backwards", () => currentSnapshot);

function projectAliasSnapshot(
  snapshot: DesktopSnapshot,
  userId: string,
  alias: string | null
): DesktopSnapshot {
  const existingProfile = snapshot.state.domain.profile.users[userId];
  const dmRoom = snapshot.state.domain.rooms.find(
    (room) => room.is_dm && room.dm_user_ids.includes(userId)
  );
  const originalDisplayLabel =
    existingProfile?.original_display_label.trim() ||
    existingProfile?.display_name?.trim() ||
    dmRoom?.original_display_label.trim() ||
    dmRoom?.display_name.trim() ||
    userId;
  const displayLabel = alias ?? originalDisplayLabel;
  const localAliases = { ...snapshot.state.domain.profile.local_aliases };
  if (alias) {
    localAliases[userId] = alias;
  } else {
    delete localAliases[userId];
  }
  const profile = {
    user_id: userId,
    display_name:
      existingProfile?.display_name ??
      (originalDisplayLabel === userId ? null : originalDisplayLabel),
    display_label: displayLabel,
    original_display_label: originalDisplayLabel,
    mention_search_terms: uniqueNonBlank([displayLabel, originalDisplayLabel, userId]),
    avatar: existingProfile?.avatar ?? null
  };
  return {
    ...snapshot,
    state: {
      ...snapshot.state,
      domain: {
        ...snapshot.state.domain,
      profile: {
        ...snapshot.state.domain.profile,
        users: {
          ...snapshot.state.domain.profile.users,
          [userId]: profile
        },
        local_aliases: localAliases,
        local_alias_update: { kind: "idle" }
      },
      rooms: snapshot.state.domain.rooms.map((room) =>
        room.is_dm && room.dm_user_ids.includes(userId)
          ? {
              ...room,
              display_label: displayLabel,
              original_display_label: originalDisplayLabel
            }
          : room
      ),
      room_management:
        snapshot.state.domain.room_management.settings === null
          ? snapshot.state.domain.room_management
          : {
              ...snapshot.state.domain.room_management,
              settings: {
                ...snapshot.state.domain.room_management.settings,
                members: snapshot.state.domain.room_management.settings.members.map((member) =>
                  member.user_id === userId
                    ? {
                        ...member,
                        display_label: displayLabel,
                        original_display_label: originalDisplayLabel
                      }
                    : member
                )
              }
            }
      },
    },
    sidebar: {
      ...snapshot.sidebar,
      global_dms: snapshot.sidebar.global_dms.map((room) =>
        room.room_id === dmRoom?.room_id ? { ...room, display_name: displayLabel } : room
      ),
      space_rooms: snapshot.sidebar.space_rooms.map((room) =>
        room.room_id === dmRoom?.room_id ? { ...room, display_name: displayLabel } : room
      )
    }
  };
}

function uniqueNonBlank(values: Array<string | null | undefined>): string[] {
  const terms: string[] = [];
  for (const value of values) {
    const normalized = value?.trim();
    if (normalized && !terms.includes(normalized)) {
      terms.push(normalized);
    }
  }
  return terms;
}

// Route ALL Tauri IPC through the recording mock. Plugin commands
// (window setTitle/setBadge, notifications, etc.) must NOT throw: they are
// not snapshot commands, so return a harmless value. Event-plugin commands
// (plugin:event|*) are handled internally by mockIPC's shouldMockEvents.
mockIPC(
  (cmd, args) => {
    if (cmd.startsWith("plugin:dialog|")) {
      return mock.invoke(cmd, (args ?? {}) as Record<string, unknown>);
    }
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
  invoke: (command, args = {}) => mock.invoke(command, args),
  setCommandResponse: (command, response) =>
    mock.setCommandResponse(command, (args: Record<string, any>) => {
      const value = typeof response === "function" ? response(args) : response;
      return normalizeHarnessCommandResponse(value);
    }),
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

  // After the App mounts and TimelineView subscribes to koushi-desktop://event,
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
      latest_event_id: "$thread-panel-reply:example.invalid",
      latest_sender: "@thread-user:example.invalid",
      latest_body_preview: "Thread panel reply from keyed event stream",
      latest_timestamp_ms: 1_800_000_000_200
    },
    can_react: true,
    is_redacted: false,
    is_hidden: false,
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
