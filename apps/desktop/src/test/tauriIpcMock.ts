/**
 * Mock Tauri IPC transport for headless UI tests.
 *
 * Provides:
 *   - A fake `invoke` that records command invocations and can return
 *     controlled responses.
 *   - Utilities to push fake CoreEvent / AppStateSnapshot payloads to
 *     registered listeners (simulating what the real Tauri backend emits on
 *     `koushi-desktop://event` and `koushi-desktop://state`).
 *
 * Used by two test tiers (plan changelog 2026-06-13):
 *   - Vitest node-mode logic tests (timelineStore.test.ts);
 *   - the Playwright headless-Chromium harness (src/test/harnessMain.tsx),
 *     which mounts the real TimelineView against this mock and exercises
 *     real-DOM scroll anchoring.
 */

import type { CoreEventPayload } from "../domain/coreEvents";
import type { ViewportSyncReceipt } from "../backend/browserFakeApi";

// ---------------------------------------------------------------------------
// Invocation record
// ---------------------------------------------------------------------------

export interface IpcInvocation {
  command: string;
   
  args: Record<string, any>;
}

type CommandResponse =
  | unknown
  | ((args: Record<string, any>) => unknown | Promise<unknown>);

// ---------------------------------------------------------------------------
// Mock IPC state
// ---------------------------------------------------------------------------

type EventListener = (payload: unknown) => void;

export class TauriIpcMock {
  private invocations: IpcInvocation[] = [];
  private listeners: Map<string, EventListener[]> = new Map();
  private commandResponses: Map<string, CommandResponse> = new Map();

  // ---- Recording ----

  recordedInvocations(): readonly IpcInvocation[] {
    return this.invocations;
  }

  invocationsOf(command: string): IpcInvocation[] {
    return this.invocations.filter((inv) => inv.command === command);
  }

  clearInvocations(): void {
    this.invocations = [];
  }

  // ---- Controlled responses ----

  setCommandResponse(command: string, response: CommandResponse): void {
    this.commandResponses.set(command, response);
  }

  // ---- The fake invoke function ----

   
  invoke<T>(command: string, args: Record<string, any> = {}): Promise<T> {
    // Strip password/secret fields from log (security: do not trace secrets).
    const safeArgs = sanitiseArgs(args);
    this.invocations.push({ command, args: safeArgs });

    if (this.commandResponses.has(command)) {
      const response = this.commandResponses.get(command);
      const resolved =
        typeof response === "function" ? response(args) : response;
      return Promise.resolve(resolved as T);
    }

    if (command === "observe_viewport_sync") {
      const observation = args.observation as {
        trigger?: ViewportSyncReceipt["trigger"];
        density?: NonNullable<ViewportSyncReceipt["density"]>;
      } | undefined;
      return Promise.resolve({
        generation: 1,
        trigger: observation?.trigger ?? "browser_resize",
        density: observation?.density ?? "default",
        nativeSupport: "unsupported",
        decision: "unsupported",
        nativeAligned: false,
        nativeOriginAligned: false,
        nativeSizeAligned: false,
        domAligned: true,
        domJsAligned: true,
        domRootAligned: true,
        parent: null,
        webview: null
      } satisfies ViewportSyncReceipt as T);
    }

    // Default: return a minimal empty snapshot.
    return Promise.resolve(defaultSnapshotResponse() as unknown as T);
  }

  // ---- Event emission (simulates core backend) ----

  /** Push a CoreEvent as if the Tauri backend emitted koushi-desktop://event */
  emitCoreEvent(event: CoreEventPayload): void {
    const listeners = this.listeners.get("koushi-desktop://event") ?? [];
    for (const listener of listeners) {
      listener({ payload: event });
    }
  }

  /** Push a state-changed notification as if the backend emitted koushi-desktop://state */
  emitStateChanged(): void {
    const listeners = this.listeners.get("koushi-desktop://state") ?? [];
    for (const listener of listeners) {
      listener({ payload: "stateChanged" });
    }
  }

  // ---- Listener registration (mirrors @tauri-apps/api/event listen) ----

  listen(eventName: string, listener: EventListener): () => void {
    const existing = this.listeners.get(eventName) ?? [];
    this.listeners.set(eventName, [...existing, listener]);
    return () => {
      const current = this.listeners.get(eventName) ?? [];
      this.listeners.set(
        eventName,
        current.filter((l) => l !== listener)
      );
    };
  }
}

// ---------------------------------------------------------------------------
// Security: strip secret-bearing fields from recorded args
// ---------------------------------------------------------------------------

 
function sanitiseArgs(args: Record<string, any>): Record<string, any> {
  const REDACTED = "[REDACTED]";
  const SECRET_KEYS = new Set([
    "password",
    "passphrase",
    "oldSecret",
    "newPassphrase",
    "secret",
    "recovery_secret",
    "destinationPath",
    "sourcePath",
    "recoveryKeyDestinationPath",
    "access_token",
    "store_key",
    "search_index_key"
  ]);
   
  const result: Record<string, any> = {};
  for (const [key, value] of Object.entries(args)) {
    result[key] = SECRET_KEYS.has(key) ? REDACTED : value;
  }
  return result;
}

// ---------------------------------------------------------------------------
// Minimal default snapshot (matches FrontendDesktopSnapshot serialisation)
// ---------------------------------------------------------------------------

function defaultSnapshotResponse() {
  // #87 Phase 4: this fixture is returned via an `as unknown as T` cast (so typecheck
  // cannot see its shape). Keep the field values flat here, then partition into the
  // domain/ui sections at runtime so the mock matches the nested IPC contract.
  const flatState: Record<string, unknown> = {
      session: { kind: "signedOut" },
      session_lock_reason: null,
      secure_backup_gate: { kind: "inactive" },
      current_session_status: { status: "idle" },
      device_cleanup: { kind: "idle" },
      auth: { kind: "unknown" },
      settings: {
        values: {
          locale: { language_tag: null, text_direction: "auto" },
          appearance: { theme: "system" },
          typography: { font: "system", emoji: "system" },
          keyboard: { composer_send_shortcut: "enter" },
          composer: { math_mode: true },
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
            image_upload_compression_policy: {
              threshold_bytes: 1048576,
              threshold_long_edge: 2560,
              target_long_edge: 2048,
              quality_percent: 82
            }
          },
          timeline: {
            auto_load_older_messages: true,
            // Deliberately NOT the product default (latestReply since #366):
            // settings-toggle tests drive the rootEvent -> latestReply
            // transition from this seeded user choice.
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
      },
      link_preview_settings: { room_overrides: {} },
      room_preferences: { rooms: {} },
      locale_profile: {
        lang: "en",
        dir: "ltr",
        catalog_locale: "en",
        pseudo_locale: "none",
        platform: "linux",
        modifier_labels: { primary: "Ctrl" }
      },
      typography_profile: {
        font: "system",
        emoji: "system",
        platform: "linux",
        font_asset: "systemFallback",
        emoji_asset: "systemFallback"
      },
      profile: {
        own: { display_name: null, avatar: null },
        users: {},
        room_users: {},
        local_aliases: {},
        local_alias_update: { kind: "idle" },
        ignored_user_ids: [],
        ignored_user_update: { kind: "idle" },
        update: { kind: "idle" }
      },
      space_members: {
        selected_space_id: null,
        generation: 0,
        power_levels_revision: null,
        can_edit_roles: false,
        space_joined: [],
        space_invited: [],
        child_room_only: [],
        child_room_count: 0,
        complete_child_room_count: 0,
        incomplete_child_room_count: 0,
        operation: { kind: "idle" }
      },
      sync: "stopped",
      navigation: { active_space_id: null, active_room_id: null },
      spaces: [],
      rooms: [],
      invites: [],
      invite_workflow: {
        query: { room_id: null, query: "", candidates: [], explicit_user_id: null },
        selected_targets: [],
        scope_plan: null,
        selected_scope: null,
        history_policy: null,
        operation: { kind: "idle" }
      },
      room_list: {
        readiness: { kind: "ready", source: "cache", generation: 0 },
        active_filter: { kind: "rooms" },
        sort: { kind: "activity" },
        items: [
          { room_id: "!room-alpha:example.invalid", kind: "room" },
          { room_id: "!room-beta:example.invalid", kind: "room" },
          { room_id: "!dm-alpha:example.invalid", kind: "room" }
        ]
      },
      room_notification_settings: {},
      room_interactions: {},
      account_management_url: "https://account.example.test/devices",
      account_management: { kind: "idle" },
      account_management_capabilities: { change_password: { kind: "unknown" } },
      soft_logout_reauth: { kind: "idle" },
      qr_login: { kind: "idle" },
      directory: {
        query: { kind: "closed" },
        preview: { kind: "closed" },
        join: { kind: "idle" },
      },
      room_management: { selected_room_id: null, settings: null, operation: { kind: "idle" } },
      mention_candidates: { targets: [] },
      activity: { kind: "closed" },
      search_crawler: { rooms: {}, last_active: null },
      timeline: {
        room_id: null,
        is_subscribed: false,
        is_paginating_backwards: false,
        composer: {
          accepted_submission_ids: [],
          pending_submission_id: null,
          pending_transaction_id: null,
          draft_revision: "0",
          last_accepted_clear_revision: "0",
          draft: "",
          document: { version: 2, inlines: [] },
          mode: "Plain"
        },
        submission_registry: { accepted_submission_ids: [], settled_submission_ids: [] },
        scheduled_send_capability: "unknown",
        scheduled_sends: [],
        staged_uploads: [],
        media_gallery: []
      },
      thread: { kind: "closed" },
      thread_attention: { kind: "closed" },
      threads_list: { kind: "closed" },
      focused_context: { kind: "closed" },
      search: { kind: "closed" },
      files_view: { kind: "closed" },
      errors: [],
      basic_operation: { kind: "idle" },
      live_signals: { rooms: {}, presence: {} },
      e2ee_trust: {
        verification: { kind: "idle" },
        cross_signing: { kind: "unknown" },
        key_backup: { kind: "unknown" },
        identity_reset: { kind: "idle" },
        key_management: {
          room_key_export: { kind: "idle" },
          room_key_import: { kind: "idle" },
          secure_backup_setup: { kind: "idle" },
          passphrase_change: { kind: "idle" }
        },
        devices: []
      },
      local_encryption: { kind: "unknown" },
      native_attention: {
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
      },
      cjk_text_policy: {
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
      }
  };
  const DOMAIN_KEYS = new Set([
    "session", "session_lock_reason", "secure_backup_gate", "current_session_status", "device_cleanup",
    "auth", "account_management_url", "account_management", "account_management_capabilities",
    "soft_logout_reauth", "qr_login", "settings", "link_preview_settings", "room_preferences",
    "locale_profile", "typography_profile", "profile", "space_members", "sync", "spaces", "rooms",
    "invites", "invite_workflow", "room_notification_settings", "room_interactions", "directory",
    "room_management", "mention_candidates", "activity", "thread_attention", "search", "search_crawler",
    "live_signals", "e2ee_trust", "local_encryption", "native_attention", "cjk_text_policy"
  ]);
  const domain: Record<string, unknown> = {};
  const ui: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(flatState)) {
    if (DOMAIN_KEYS.has(key)) domain[key] = value;
    else ui[key] = value;
  }
  return {
    state: { schema_version: 5, domain, ui },
    sidebar: {
      active_space_id: null,
      account_home: {
        display_name: "Home",
        unread_count: 0,
        highlight_count: 0,
        invite_count: 0,
        attention_count: 0,
        is_active: true
      },
      space_rail: [],
      space_rooms: [],
      not_joined_space_rooms: [],
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
