import { beforeEach, describe, expect, test, vi } from "vitest";

import {
  applyAppStoreDelta,
  applyDeltaToState,
  applySnapshotToState,
  clearAppStoreSnapshot,
  getAppStoreDeltaStats,
  getAppStoreSnapshot,
  selectForwardDestinations,
  selectMentionCandidates,
  selectSnapshot,
  setAppStoreSnapshot,
  useAppStore
} from "./appStore";
import type { DesktopSnapshot } from "./types";

describe("appStore projection cache", () => {
  beforeEach(() => {
    clearAppStoreSnapshot();
  });

  test("keeps identical-by-value snapshot references stable", () => {
    const previous = makeSnapshot();
    const next = structuredClone(previous);

    const projected = applySnapshotToState(previous, next);

    expect(projected).toBe(previous);
  });

  test("keeps unrelated slices stable when search crawler changes", () => {
    const previous = makeSnapshot();
    const next = structuredClone(previous);
    next.state.domain.search_crawler = {
      rooms: {
        "!room-crawler:example.invalid": { kind: "running", processed: 3, indexed: 2 }
      },
      last_active: {
        room_id: "!room-crawler:example.invalid",
        updated_at_ms: 1_800_000_000_000,
        status: "running",
        processed: 3,
        indexed: 2
      }
    };

    const projected = applySnapshotToState(previous, next);

    expect(projected).not.toBe(previous);
    expect(projected).not.toBeNull();
    if (!projected) {
      throw new Error("expected projected snapshot");
    }
    expect(projected.state).not.toBe(previous.state);
    expect(projected.state.domain).not.toBe(previous.state.domain);
    expect(projected.state.ui).toBe(previous.state.ui);
    expect(projected.sidebar).toBe(previous.sidebar);
    expect(projected.timeline).toBe(previous.timeline);
    expect(projected.thread).toBe(previous.thread);
    expect(projected.state.domain.rooms).toBe(previous.state.domain.rooms);
    expect(projected.state.domain.profile).toBe(previous.state.domain.profile);
  });

  test("keeps domain, sidebar, timeline, and thread stable when the composer draft changes", () => {
    const previous = makeSnapshot();
    const next = structuredClone(previous);
    next.state.ui.timeline.composer.draft = "updated draft";

    const projected = applySnapshotToState(previous, next);

    expect(projected).not.toBe(previous);
    expect(projected).not.toBeNull();
    if (!projected) {
      throw new Error("expected projected snapshot");
    }
    expect(projected.state).not.toBe(previous.state);
    expect(projected.state.ui).not.toBe(previous.state.ui);
    expect(projected.state.domain).toBe(previous.state.domain);
    expect(projected.sidebar).toBe(previous.sidebar);
    expect(projected.timeline).toBe(previous.timeline);
    expect(projected.thread).toBe(previous.thread);
  });

  test("resets the projected snapshot to null", () => {
    setAppStoreSnapshot(makeSnapshot());
    expect(getAppStoreSnapshot()).not.toBeNull();

    setAppStoreSnapshot(null);

    expect(getAppStoreSnapshot()).toBeNull();
    expect(useAppStore.getState().snapshot).toBeNull();
  });

  test("keeps selector output stable across unrelated updates", () => {
    const previous = makeSnapshot();
    setAppStoreSnapshot(previous);

    const firstForwardDestinations = selectForwardDestinations({ snapshot: previous });
    const firstMentionCandidates = selectMentionCandidates({ snapshot: previous });

    const next = structuredClone(previous);
    next.state.domain.search_crawler = {
      rooms: {
        "!room-crawler:example.invalid": { kind: "queued" }
      },
      last_active: {
        room_id: "!room-crawler:example.invalid",
        updated_at_ms: 1_800_000_000_000,
        status: "queued",
        processed: 0,
        indexed: 0
      }
    };
    setAppStoreSnapshot(next);

    const secondSnapshot = getAppStoreSnapshot();
    expect(secondSnapshot).not.toBeNull();
    if (!secondSnapshot) {
      return;
    }

    expect(selectForwardDestinations({ snapshot: secondSnapshot })).toBe(firstForwardDestinations);
    expect(selectMentionCandidates({ snapshot: secondSnapshot })).toBe(firstMentionCandidates);
  });

  test("updates selector outputs when rooms or profile users change", () => {
    const previous = makeSnapshot();
    setAppStoreSnapshot(previous);

    const firstForwardDestinations = selectForwardDestinations({ snapshot: previous });
    const firstMentionCandidates = selectMentionCandidates({ snapshot: previous });

    const roomsChanged = structuredClone(previous);
    roomsChanged.state.domain.rooms = [
      ...roomsChanged.state.domain.rooms,
      {
        room_id: "!room-delta:example.invalid",
        display_name: "Delta Room",
        display_label: "Delta Room",
        original_display_label: "Delta Room",
        avatar: null,
        is_dm: false,
        dm_user_ids: [],
        tags: { favourite: null, low_priority: null },
        unread_count: 0,
        parent_space_ids: [],
        dm_space_ids: [],
        is_encrypted: false
      }
    ];
    setAppStoreSnapshot(roomsChanged);
    const afterRoomsChange = getAppStoreSnapshot();
    expect(afterRoomsChange).not.toBeNull();
    if (!afterRoomsChange) {
      return;
    }
    expect(selectForwardDestinations({ snapshot: afterRoomsChange })).not.toBe(
      firstForwardDestinations
    );
    expect(selectMentionCandidates({ snapshot: afterRoomsChange })).toBe(firstMentionCandidates);

    const usersChanged = structuredClone(afterRoomsChange);
    usersChanged.state.domain.profile.users["@delta-user:example.invalid"] = {
      user_id: "@delta-user:example.invalid",
      display_name: "Delta User",
      display_label: "Delta User",
      original_display_label: "Delta User",
      mention_search_terms: ["delta", "@delta-user:example.invalid"],
      avatar: null
    };
    setAppStoreSnapshot(usersChanged);
    const afterUsersChange = getAppStoreSnapshot();
    expect(afterUsersChange).not.toBeNull();
    if (!afterUsersChange) {
      return;
    }
    expect(selectForwardDestinations({ snapshot: afterUsersChange })).toBe(
      selectForwardDestinations({ snapshot: afterRoomsChange })
    );
    expect(selectMentionCandidates({ snapshot: afterUsersChange })).not.toBe(
      firstMentionCandidates
    );
  });

  test("notifies selector subscribers only when the selected slice changes", () => {
    const previous = makeSnapshot();
    setAppStoreSnapshot(previous);

    const listener = vi.fn();
    const unsubscribe = useAppStore.subscribe(
      (state) => selectSnapshot(state)?.state.domain.profile.users,
      listener
    );
    listener.mockClear();

    const next = structuredClone(previous);
    next.state.domain.search_crawler = {
      rooms: {
        "!room-crawler:example.invalid": { kind: "running", processed: 1, indexed: 1 }
      },
      last_active: {
        room_id: "!room-crawler:example.invalid",
        updated_at_ms: 1_800_000_000_000,
        status: "running",
        processed: 1,
        indexed: 1
      }
    };
    setAppStoreSnapshot(next);
    expect(listener).not.toHaveBeenCalled();

    const usersChanged = structuredClone(next);
    usersChanged.state.domain.profile.users["@epsilon-user:example.invalid"] = {
      user_id: "@epsilon-user:example.invalid",
      display_name: "Epsilon User",
      display_label: "Epsilon User",
      original_display_label: "Epsilon User",
      mention_search_terms: ["epsilon", "@epsilon-user:example.invalid"],
      avatar: null
    };
    setAppStoreSnapshot(usersChanged);
    expect(listener).toHaveBeenCalledTimes(1);
    unsubscribe();
  });

  test("applies state deltas while preserving unchanged slice references", () => {
    const previous = makeSnapshot();
    const delta = {
      generation: 1,
      changed: {
        state: {
          domain: {
            search_crawler: {
              rooms: {
                "!room-crawler:example.invalid": {
                  kind: "running" as const,
                  processed: 4,
                  indexed: 3
                }
              },
              last_active: {
                room_id: "!room-crawler:example.invalid",
                updated_at_ms: 1_800_000_000_000,
                status: "running" as const,
                processed: 4,
                indexed: 3
              }
            }
          }
        }
      }
    };

    const projected = applyDeltaToState(previous, delta);

    expect(projected).not.toBe(previous);
    expect(projected).not.toBeNull();
    if (!projected) {
      throw new Error("expected projected snapshot");
    }
    expect(projected.state.domain.search_crawler).toEqual(delta.changed.state.domain.search_crawler);
    expect(projected.state.domain.rooms).toBe(previous.state.domain.rooms);
    expect(projected.state.ui).toBe(previous.state.ui);
    expect(projected.sidebar).toBe(previous.sidebar);
  });

  test("delta-applied snapshots match equivalent full snapshots", () => {
    const previous = makeSnapshot();
    const next = structuredClone(previous);
    next.state_generation = 1;
    next.state.domain.search_crawler = {
      rooms: {
        "!room-crawler:example.invalid": {
          kind: "running",
          processed: 8,
          indexed: 5
        }
      },
      last_active: {
        room_id: "!room-crawler:example.invalid",
        updated_at_ms: 1_800_000_000_000,
        status: "running",
        processed: 8,
        indexed: 5
      }
    };
    next.state.ui.navigation = {
      ...next.state.ui.navigation,
      active_room_id: "!room-b:example.invalid"
    };

    const delta = {
      generation: 1,
      changed: {
        state: {
          domain: {
            search_crawler: next.state.domain.search_crawler
          },
          ui: {
            navigation: next.state.ui.navigation
          }
        }
      }
    };

    expect(applyDeltaToState(previous, delta)).toEqual(applySnapshotToState(previous, next));
  });

  test("ignores stale deltas after a full snapshot reset without applying or refreshing", () => {
    const snapshot = makeSnapshot();
    snapshot.state_generation = 4;
    setAppStoreSnapshot(snapshot);

    expect(useAppStore.getState().stateGeneration).toBe(4);

    // A stale delta (same generation as the just-applied full snapshot) is
    // already reflected. It must be ignored as handled (no refresh) AND must
    // not clobber the reset state with its trailing payload.
    expect(
      applyAppStoreDelta({
        generation: 4,
        changed: {
          state: {
            ui: {
              navigation: {
                active_space_id: null,
                active_room_id: "!stale-room:example.invalid",
                last_room_by_space_id: {},
                space_order: []
              }
            }
          }
        }
      })
    ).toBe(true);
    expect(useAppStore.getState().stateGeneration).toBe(4);
    expect(getAppStoreSnapshot()?.state.ui.navigation.active_room_id).toBe(
      "!room-alpha:example.invalid"
    );

    // The next contiguous delta still applies.
    expect(
      applyAppStoreDelta({
        generation: 5,
        changed: { state: { domain: { search_crawler: { rooms: {}, last_active: null } } } }
      })
    ).toBe(true);
    expect(useAppStore.getState().stateGeneration).toBe(5);
  });

  test("rejects stale full snapshots after a newer state delta has applied", () => {
    const initial = makeSnapshot();
    initial.state_generation = 4;
    initial.state.ui.navigation.active_space_id = "!space-alpha:example.invalid";
    initial.state.ui.navigation.active_room_id = "!room-alpha:example.invalid";
    setAppStoreSnapshot(initial);

    expect(
      applyAppStoreDelta({
        generation: 5,
        changed: {
          state: {
            ui: {
              navigation: {
                ...initial.state.ui.navigation,
                active_space_id: "!space-beta:example.invalid",
                active_room_id: "!room-beta:example.invalid"
              }
            }
          }
        }
      })
    ).toBe(true);

    const stale = structuredClone(initial);
    stale.state_generation = 4;
    stale.state.ui.navigation.active_space_id = "!space-alpha:example.invalid";
    stale.state.ui.navigation.active_room_id = "!room-alpha:example.invalid";
    setAppStoreSnapshot(stale);

    expect(useAppStore.getState().stateGeneration).toBe(5);
    expect(getAppStoreSnapshot()?.state.ui.navigation.active_space_id).toBe(
      "!space-beta:example.invalid"
    );
    expect(getAppStoreSnapshot()?.state.ui.navigation.active_room_id).toBe(
      "!room-beta:example.invalid"
    );
  });

  test("rejects gapped state deltas so the caller can reset from a full snapshot", () => {
    setAppStoreSnapshot(makeSnapshot());

    expect(
      applyAppStoreDelta({
        generation: 1,
        changed: {
          state: {
            ui: {
              navigation: {
                active_space_id: null,
                active_room_id: "!room-a:example.invalid",
                last_room_by_space_id: {},
                space_order: []
              }
            }
          }
        }
      })
    ).toBe(true);
    expect(
      applyAppStoreDelta({
        generation: 3,
        changed: { state: { domain: { search_crawler: { rooms: {}, last_active: null } } } }
      })
    ).toBe(false);
  });

  test("ignores already-applied deltas without requesting a full refresh", () => {
    const snapshot = makeSnapshot();
    snapshot.state_generation = 10;
    setAppStoreSnapshot(snapshot);

    const staleChange = {
      changed: { state: { domain: { search_crawler: { rooms: {}, last_active: null } } } }
    };

    // A full snapshot (command response or refresh) just landed generation 10.
    // Background StateDeltas emitted at or before generation 10 are already
    // reflected, so they must be ignored as handled (return true) and must NOT
    // ask the caller to refresh. Returning false here is what makes App.tsx
    // call get_snapshot again, which lands a newer generation and turns every
    // trailing delta stale -> a self-amplifying refresh storm at scale.
    expect(applyAppStoreDelta({ generation: 8, ...staleChange })).toBe(true);
    expect(applyAppStoreDelta({ generation: 9, ...staleChange })).toBe(true);
    expect(applyAppStoreDelta({ generation: 10, ...staleChange })).toBe(true);

    // The next contiguous delta still applies normally.
    expect(applyAppStoreDelta({ generation: 11, ...staleChange })).toBe(true);
    expect(useAppStore.getState().stateGeneration).toBe(11);

    // A genuine forward gap still requests a refresh.
    expect(applyAppStoreDelta({ generation: 13, ...staleChange })).toBe(false);
  });

  test("does not storm refreshes when background deltas trail a full snapshot under load", () => {
    // The core has churned far ahead from large-account background sync.
    const CORE_LATEST = 200;
    const base = makeSnapshot();
    base.state_generation = 0;
    setAppStoreSnapshot(base);

    let refreshCount = 0;
    const refresh = () => {
      refreshCount += 1;
      const full = makeSnapshot();
      full.state_generation = CORE_LATEST;
      setAppStoreSnapshot(full);
    };

    // A selectRoom/refresh command response lands the latest generation first.
    refresh();

    // Then the in-flight background deltas (generations 1..CORE_LATEST) arrive,
    // every one now stale relative to the applied full snapshot. App.tsx calls
    // refresh() on each non-applied delta, so the bug produces one full
    // get_snapshot per trailing delta; the fix ignores them.
    for (let generation = 1; generation <= CORE_LATEST; generation += 1) {
      const applied = applyAppStoreDelta({
        generation,
        changed: { state: { domain: { search_crawler: { rooms: {}, last_active: null } } } }
      });
      if (!applied) {
        refresh();
      }
    }

    expect(refreshCount).toBe(1);
  });

  test("counts applied, stale-ignored, and forward-gap deltas for transport diagnostics", () => {
    const snapshot = makeSnapshot();
    snapshot.state_generation = 2;
    setAppStoreSnapshot(snapshot);
    const change = {
      changed: { state: { domain: { search_crawler: { rooms: {}, last_active: null } } } }
    };

    applyAppStoreDelta({ generation: 3, ...change }); // contiguous -> applied
    applyAppStoreDelta({ generation: 2, ...change }); // stale -> ignored, no refresh
    applyAppStoreDelta({ generation: 99, ...change }); // forward gap -> refresh requested

    expect(getAppStoreDeltaStats()).toEqual({
      applied: 1,
      staleIgnored: 1,
      gapRefreshRequested: 1
    });
  });
});

function makeSnapshot(): DesktopSnapshot {
  return {
    state: {
      schema_version: 2,
      domain: {
        session: { kind: "ready", homeserver: "https://example.invalid", user_id: "@user:example.invalid", device_id: "DEVICE" },
        auth: { kind: "unknown" },
        settings: {
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
              hide_redacted: false,
              url_previews_enabled: true,
              encrypted_url_previews_enabled: false
            },
            media: {
              image_upload_compression: "never",
              image_upload_compression_policy: {
                threshold_bytes: 1048576,
                threshold_long_edge: 2560,
                target_long_edge: 2048,
                quality_percent: 82
              }
            },
            timeline: {
              auto_load_older_messages: true,
              thread_root_order: { kind: "latestReply" }
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
          users: {
            "@alpha-user:example.invalid": {
              user_id: "@alpha-user:example.invalid",
              display_name: "Alpha User",
              display_label: "Alpha User",
              original_display_label: "Alpha User",
              mention_search_terms: ["alpha", "@alpha-user:example.invalid"],
              avatar: null
            }
          },
          local_aliases: {},
          local_alias_update: { kind: "idle" },
          ignored_user_ids: [],
          ignored_user_update: { kind: "idle" },
          update: { kind: "idle" }
        },
        sync: "running",
        sync_mode: { kind: "unsupported" },
        spaces: [
          {
            space_id: "!space-alpha:example.invalid",
            display_name: "Alpha Space",
            avatar: null,
            child_room_ids: ["!room-alpha:example.invalid"]
          }
        ],
        rooms: [
          {
            room_id: "!room-alpha:example.invalid",
            display_name: "Alpha Room",
            display_label: "Alpha Room",
            original_display_label: "Alpha Room",
            avatar: null,
            is_dm: false,
            dm_user_ids: [],
            tags: { favourite: null, low_priority: null },
            unread_count: 0,
            parent_space_ids: ["!space-alpha:example.invalid"],
            dm_space_ids: [],
            is_encrypted: false,
            joined_members: 2
          },
          {
            room_id: "!room-beta:example.invalid",
            display_name: "Beta DM",
            display_label: "Beta User",
            original_display_label: "Beta User",
            avatar: null,
            is_dm: true,
            dm_user_ids: ["@beta-user:example.invalid"],
            tags: { favourite: null, low_priority: null },
            unread_count: 1,
            parent_space_ids: [],
            dm_space_ids: ["!space-alpha:example.invalid"],
            is_encrypted: true
          }
        ],
        invites: [],
        room_interactions: {},
        room_notification_settings: {},
        device_sessions: { kind: "idle" },
        account_management: { kind: "idle" },
        account_management_capabilities: { change_password: { kind: "unknown" } },
        soft_logout_reauth: { kind: "idle" },
        qr_login: { kind: "idle" },
        directory: { query: { kind: "closed" }, join: { kind: "idle" } },
        room_management: { selected_room_id: null, settings: null, operation: { kind: "idle" } },
        activity: { kind: "closed" },
        thread_attention: { kind: "closed" },
        search: { kind: "closed" },
        search_crawler: { rooms: {}, last_active: null },
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
      },
      ui: {
        navigation: { active_space_id: "!space-alpha:example.invalid", active_room_id: "!room-alpha:example.invalid" },
        room_list: { active_filter: { kind: "rooms" }, sort: { kind: "activity" }, items: [] },
        timeline: {
          room_id: "!room-alpha:example.invalid",
          is_subscribed: true,
          is_paginating_backwards: false,
          composer: { pending_transaction_id: null, draft: "hello", mode: "Plain" },
          scheduled_send_capability: "unknown",
          scheduled_sends: [],
          staged_uploads: [],
          media_gallery: [],
          media_downloads: {}
        },
        thread: {
          kind: "open",
          room_id: "!room-alpha:example.invalid",
          root_event_id: "$thread-root:example.invalid",
          is_subscribed: true,
          composer: { pending_transaction_id: null, draft: "", mode: "Plain" }
        },
        focused_context: { kind: "closed" },
        files_view: { kind: "closed" },
        threads_list: { kind: "closed" },
        errors: [],
        basic_operation: { kind: "idle" }
      }
    },
    sidebar: {
      active_space_id: "!space-alpha:example.invalid",
      account_home: {
        display_name: "Home",
        unread_count: 0,
        highlight_count: 0,
        is_active: true
      },
      space_rail: [
        {
          space_id: "!space-alpha:example.invalid",
          display_name: "Alpha Space",
          avatar: null,
          unread_count: 0,
          highlight_count: 0,
          is_active: true
        }
      ],
      space_rooms: [
        {
          room_id: "!room-alpha:example.invalid",
          display_name: "Alpha Room",
          avatar: null,
          tags: { favourite: null, low_priority: null },
          unread_count: 0,
          highlight_count: 0
        }
      ],
      global_dms: [
        {
          room_id: "!room-beta:example.invalid",
          display_name: "Beta User",
          avatar: null,
          tags: { favourite: null, low_priority: null },
          unread_count: 1,
          highlight_count: 0
        }
      ],
      space_unread_count: 0,
      dm_unread_count: 1,
      space_highlight_count: 0,
      dm_highlight_count: 0
    },
    timeline: [
      {
        room_id: "!room-alpha:example.invalid",
        event_id: "$event-alpha:example.invalid",
        sender: "@alpha-user:example.invalid",
        timestamp_ms: 1710000000000,
        body: "Hello world",
        attachment_filename: null,
        reply_count: 0
      }
    ],
    thread: {
      room_id: "!room-alpha:example.invalid",
      root_event_id: "$thread-root:example.invalid",
      replies: [
        {
          room_id: "!room-alpha:example.invalid",
          root_event_id: "$thread-root:example.invalid",
          event_id: "$thread-reply:example.invalid",
          sender: "@alpha-user:example.invalid",
          timestamp_ms: 1710000001000,
          body: "Reply"
        }
      ]
    }
  } as DesktopSnapshot;
}
