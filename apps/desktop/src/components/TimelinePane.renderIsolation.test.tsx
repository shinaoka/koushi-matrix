// @vitest-environment jsdom

import { act, cleanup, render, screen } from "@testing-library/react";
import { createElement } from "react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

import { clearAppStoreSnapshot, getAppStoreSnapshot, setAppStoreSnapshot } from "../domain/appStore";
import { COMPOSER_DRAFT_REVISION_ZERO } from "../domain/composerDraftRevision";
import type { DesktopSnapshot, MentionIntent } from "../domain/types";
import { TimelinePane } from "./panes";
import type { TimelineTransport } from "./TimelineView";

const renderCounts = vi.hoisted(() => ({
  composer: 0,
  timelineView: 0
}));

vi.mock("./composer", async () => {
  const actual = await vi.importActual<typeof import("./composer")>("./composer");
  const { memo } = await import("react");

  const ComposerProbe = memo(function ComposerProbe(props: Parameters<typeof actual.Composer>[0]) {
    renderCounts.composer += 1;
    return createElement(actual.Composer, props);
  });

  return {
    ...actual,
    Composer: ComposerProbe
  };
});

vi.mock("./TimelineView", async () => {
  const actual = await vi.importActual<typeof import("./TimelineView")>("./TimelineView");
  const { memo } = await import("react");

  const TimelineViewProbe = memo(
    function TimelineViewProbe(props: Parameters<typeof actual.TimelineView>[0]) {
      renderCounts.timelineView += 1;
      return createElement(actual.TimelineView, props);
    }
  );

  return {
    ...actual,
    TimelineView: TimelineViewProbe
  };
});

describe("TimelinePane render isolation", () => {
  beforeEach(() => {
    clearAppStoreSnapshot();
    renderCounts.composer = 0;
    renderCounts.timelineView = 0;
  });

  afterEach(() => {
    cleanup();
    clearAppStoreSnapshot();
  });

  test("keeps hot timeline and composer consumers from rerendering on search crawler updates", async () => {
    const snapshot = makeSnapshot();
    const resolveComposerKeyAction = async (): Promise<"noop"> => "noop";
    const noop = () => undefined;
    const mentionIntent: MentionIntent = { targets: [] };
    const emptySearchResults: never[] = [];
    const timelineTransport = noopTimelineTransport();
    setAppStoreSnapshot(snapshot);

    const renderPane = (currentSnapshot: DesktopSnapshot) =>
      createElement(TimelinePane, {
        activeRoomName: "Alpha Room",
        composerDraft: currentSnapshot.state.ui.timeline.composer.draft,
        composerMode: { kind: "plain" },
        mentionIntent,
        resolveComposerKeyAction,
        searchQuery: "",
        searchResults: emptySearchResults,
        showSearchResults: false,
        snapshot: currentSnapshot,
        timelineTransport,
        onCancelReply: noop,
        onCancelScheduledSend: noop,
        onAttachFiles: noop,
        onClearUploadStaging: noop,
        onUpdateStagedUploadCaption: noop,
        onSelectStagedUploadVariant: noop,
        onLoadStagedUploadPreview: async () => [],
        onComposerDraftChange: noop,
        onMentionIntentChange: noop,
        onEditMessage: noop,
        onOpenContextMenu: noop,
        onOpenThread: noop,
        onRedactMessage: noop,
        onReply: noop,
        onRescheduleScheduledSend: noop,
        onResultSelect: noop,
        onScheduleSend: noop,
        onSendText: noop,
        onSetLocalUserAlias: noop,
        onUnpinPinnedEvent: noop,
        onOpenPeople: noop,
        onOpenThreads: noop,
        onToggleRoomInfo: noop
      });

    const { rerender } = render(renderPane(snapshot));

    expect(renderCounts.composer).toBe(1);
    expect(renderCounts.timelineView).toBe(1);

    const searchCrawlerOnly = structuredClone(snapshot);
    searchCrawlerOnly.state.domain.search_crawler = {
      rooms: {
        "!crawler:example.invalid": { kind: "running", processed: 7, indexed: 4 }
      },
      last_active: {
        room_id: "!crawler:example.invalid",
        updated_at_ms: 1_800_000_000_000,
        status: "running",
        processed: 7,
        indexed: 4
      }
    };

    await act(async () => {
      setAppStoreSnapshot(searchCrawlerOnly);
    });

    const projectedSearchCrawlerOnly = getAppStoreSnapshot();
    expect(projectedSearchCrawlerOnly).not.toBeNull();
    if (!projectedSearchCrawlerOnly) {
      return;
    }

    rerender(renderPane(projectedSearchCrawlerOnly));

    expect(renderCounts.composer).toBe(1);
    expect(renderCounts.timelineView).toBe(1);

    const roomsChanged = structuredClone(searchCrawlerOnly);
    roomsChanged.state.domain.rooms = [
      ...roomsChanged.state.domain.rooms,
      {
        room_id: "!room-beta:example.invalid",
        display_name: "Beta Room",
        display_label: "Beta Room",
        original_display_label: "Beta Room",
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

    await act(async () => {
      setAppStoreSnapshot(roomsChanged);
    });

    const projectedRoomsChanged = getAppStoreSnapshot();
    expect(projectedRoomsChanged).not.toBeNull();
    if (!projectedRoomsChanged) {
      return;
    }

    rerender(renderPane(projectedRoomsChanged));

    expect(renderCounts.timelineView).toBe(2);
    expect(renderCounts.composer).toBe(1);

    const usersChanged = structuredClone(roomsChanged);
    usersChanged.state.domain.profile.users["@beta-user:example.invalid"] = {
      user_id: "@beta-user:example.invalid",
      display_name: "Beta User",
      display_label: "Beta User",
      original_display_label: "Beta User",
      mention_search_terms: ["beta", "@beta-user:example.invalid"],
      avatar: null
    };

    await act(async () => {
      setAppStoreSnapshot(usersChanged);
    });

    const projectedUsersChanged = getAppStoreSnapshot();
    expect(projectedUsersChanged).not.toBeNull();
    if (!projectedUsersChanged) {
      return;
    }

    rerender(renderPane(projectedUsersChanged));

    expect(renderCounts.composer).toBe(2);
    expect(renderCounts.timelineView).toBe(3);
  });

  test("shows thread attention counts on the timeline header threads button", () => {
    const snapshot = makeSnapshot();
    snapshot.state.domain.thread_attention = {
      kind: "tracking",
      room_id: "!room-alpha:example.invalid",
      root_event_id: "$thread-root:example.invalid",
      notification_count: 3,
      highlight_count: 1,
      live_event_marker_count: 2
    };
    const noop = () => undefined;
    const timelineTransport = noopTimelineTransport();
    setAppStoreSnapshot(snapshot);

    render(
      createElement(TimelinePane, {
        activeRoomName: "Alpha Room",
        composerDraft: snapshot.state.ui.timeline.composer.draft,
        composerMode: { kind: "plain" },
        mentionIntent: { targets: [] },
        resolveComposerKeyAction: async (): Promise<"noop"> => "noop",
        searchQuery: "",
        searchResults: [],
        showSearchResults: false,
        snapshot,
        timelineTransport,
        onCancelReply: noop,
        onCancelScheduledSend: noop,
        onAttachFiles: noop,
        onClearUploadStaging: noop,
        onUpdateStagedUploadCaption: noop,
        onSelectStagedUploadVariant: noop,
        onLoadStagedUploadPreview: async () => [],
        onComposerDraftChange: noop,
        onMentionIntentChange: noop,
        onEditMessage: noop,
        onOpenContextMenu: noop,
        onOpenThread: noop,
        onRedactMessage: noop,
        onReply: noop,
        onRescheduleScheduledSend: noop,
        onResultSelect: noop,
        onScheduleSend: noop,
        onSendText: noop,
        onSetLocalUserAlias: noop,
        onUnpinPinnedEvent: noop,
        onOpenPeople: noop,
        onOpenThreads: noop,
        onToggleRoomInfo: noop
      })
    );

    const threadsButton = screen.getByRole("button", { name: "Threads" });
    expect(threadsButton.getAttribute("data-count")).toBe("3");
    expect(threadsButton.getAttribute("data-live-count")).toBe("2");
    expect(threadsButton.getAttribute("data-mention-count")).toBe("1");
  });
});

function makeSnapshot(): DesktopSnapshot {
  return {
    state: {
      schema_version: 3,
      domain: {
        session: {
          kind: "ready",
          homeserver: "https://example.invalid",
          user_id: "@user:example.invalid",
          device_id: "DEVICE"
        },
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
        navigation: {
          active_space_id: "!space-alpha:example.invalid",
          active_room_id: "!room-alpha:example.invalid"
        },
        room_list: { active_filter: { kind: "rooms" }, sort: { kind: "activity" }, items: [] },
        timeline: {
          room_id: "!room-alpha:example.invalid",
          is_subscribed: true,
          is_paginating_backwards: false,
          composer: { accepted_submission_ids: [], pending_transaction_id: null, draft_revision: COMPOSER_DRAFT_REVISION_ZERO, last_accepted_clear_revision: COMPOSER_DRAFT_REVISION_ZERO, draft: "hello", mode: "Plain" },
          submission_registry: { accepted_submission_ids: [], settled_submission_ids: [] },
          scheduled_send_capability: "unknown",
          scheduled_sends: [],
          staged_uploads: [],
          media_gallery: [],
          media_downloads: {},
          continuity: { kind: "unknown" }
        },
        thread: {
          kind: "open",
          room_id: "!room-alpha:example.invalid",
          root_event_id: "$thread-root:example.invalid",
          is_subscribed: true,
          composer: { accepted_submission_ids: [], pending_transaction_id: null, draft_revision: COMPOSER_DRAFT_REVISION_ZERO, last_accepted_clear_revision: COMPOSER_DRAFT_REVISION_ZERO, draft: "", mode: "Plain" }
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
      not_joined_space_rooms: [],
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

function noopTimelineTransport(): TimelineTransport {
  return {
    listenCoreEvents: () => () => undefined,
    paginateBackwards: async () => undefined,
    sendReaction: async () => undefined,
    retrySend: async () => undefined,
    cancelSend: async () => undefined,
    redactReaction: async () => undefined,
    sendReadReceipt: async () => undefined,
    setFullyRead: async () => undefined,
    setTyping: async () => undefined,
    editMessage: async () => undefined,
    redactMessage: async () => undefined,
    pinEvent: async () => undefined,
    unpinEvent: async () => undefined,
    downloadMedia: async () => undefined,
    downloadAvatarThumbnail: async () => undefined,
    loadMessageSource: async () => undefined,
    requestRoomKey: async () => undefined,
    forwardMessage: async () => undefined,
    loadLinkPreviews: async () => undefined,
    hideLinkPreview: async () => undefined,
    observeViewport: async () => undefined,
    openAtTimestamp: async () => undefined
  };
}
