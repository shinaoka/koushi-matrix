import { roomTimelineKey, type CoreEventPayload, type TimelineItem } from "../src/domain/coreEvents";
import type { DesktopSnapshot, RoomListItem, RoomSummary } from "../src/domain/types";
import type { StateUpdateEnvelope } from "../src/domain/coreEvents";

const LATTICE_SPACE_ID = "!lattice-lab:example.invalid";
const PHOTON_SPACE_ID = "!photon-reading:example.invalid";
const RELEASE_SPACE_ID = "!release-crew:example.invalid";
const GENERAL_ROOM_ID = "!general:example.invalid";
const DESIGN_ROOM_ID = "!design:example.invalid";
const PAPERS_ROOM_ID = "!papers:example.invalid";
const RANDOM_ROOM_ID = "!random:example.invalid";
const AKI_ROOM_ID = "!aki:example.invalid";
const AKI_USER_ID = "@aki:example.invalid";
const DATE_START = Date.UTC(2026, 2, 10, 9, 0);

export interface ReadmeFixture {
  snapshot: DesktopSnapshot;
  stateUpdate: StateUpdateEnvelope;
  initialItems: CoreEventPayload;
}

function roomSummary(
  roomId: string,
  displayName: string,
  options: {
    isDm?: boolean;
    dmUserIds?: string[];
    favourite?: boolean;
    unreadCount?: number;
  } = {}
): RoomSummary {
  return {
    room_id: roomId,
    display_name: displayName,
    display_label: displayName,
    original_display_label: displayName,
    avatar: null,
    is_dm: options.isDm ?? false,
    dm_user_ids: options.dmUserIds ?? [],
    tags: {
      favourite: options.favourite ? { order: "a" } : null,
      low_priority: null
    },
    unread_count: options.unreadCount ?? 0,
    notification_count: options.unreadCount ?? 0,
    highlight_count: 0,
    latest_event: null,
    parent_space_ids: options.isDm ? [] : [LATTICE_SPACE_ID],
    dm_space_ids: options.isDm ? [LATTICE_SPACE_ID] : [],
    is_encrypted: !options.isDm,
    joined_members: options.isDm ? 2 : 8
  };
}

function roomListItem(
  room: RoomSummary,
  options: { kind?: "room" | "dm"; displayCount?: number } = {}
): RoomListItem {
  const displayCount = options.displayCount ?? room.unread_count;
  return {
    room_id: room.room_id,
    display_name: room.display_label,
    avatar: null,
    tags: room.tags,
    unread_count: room.unread_count,
    highlight_count: room.highlight_count ?? 0,
    notification_count: room.notification_count ?? 0,
    display_count: displayCount,
    has_unread_content: room.unread_count > 0,
    is_attention_highlighted: false,
    has_unread_mention: false,
    is_muted: false
  };
}

function message(
  currentUserId: string,
  index: number,
  sender: string,
  senderLabel: string,
  body: string,
  options: Pick<TimelineItem, "reply_quote" | "thread_summary" | "reactions"> = {
    reply_quote: null,
    thread_summary: null,
    reactions: []
  }
): TimelineItem {
  const eventId = `$readme-message-${String(index).padStart(2, "0")}:example.invalid`;
  return {
    id: { Event: { event_id: eventId } },
    sender,
    sender_label: senderLabel,
    body,
    timestamp_ms: DATE_START + index * 5 * 60_000,
    in_reply_to_event_id: options.reply_quote?.event_id ?? null,
    reply_quote: options.reply_quote ?? null,
    thread_root: null,
    thread_summary: options.thread_summary ?? null,
    reactions: options.reactions ?? [],
    can_react: true,
    is_redacted: false,
    is_hidden: false,
    can_redact: sender === currentUserId,
    is_edited: index === 5,
    can_edit: sender === currentUserId,
    actions: {
      can_copy: true,
      can_forward: true,
      can_reply: true,
      can_permalink: true,
      can_view_source: true,
      permalink: `https://matrix.to/#/!general%3Aexample.invalid/${encodeURIComponent(eventId)}`
    },
    ...options
  };
}

export function createReadmeFixture(source: DesktopSnapshot): ReadmeFixture {
  const session = source.state.domain.session;
  if (session.kind !== "ready") {
    throw new Error("README screenshot requires the ready harness session");
  }

  const general = roomSummary(GENERAL_ROOM_ID, "General");
  const design = roomSummary(DESIGN_ROOM_ID, "Design", { unreadCount: 3 });
  const papers = roomSummary(PAPERS_ROOM_ID, "Papers", { favourite: true });
  const random = roomSummary(RANDOM_ROOM_ID, "Random");
  const aki = roomSummary(AKI_ROOM_ID, "Aki", {
    isDm: true,
    dmUserIds: [AKI_USER_ID]
  });
  const roomItems = [
    roomListItem(general),
    roomListItem(design, { displayCount: 3 }),
    roomListItem(random)
  ];
  const favouriteItems = [roomListItem(papers)];
  const dmItems = [roomListItem(aki, { kind: "dm" })];
  const generation = (source.state_generation ?? 0) + 1;
  const snapshot: DesktopSnapshot = {
    ...source,
    state_generation: generation,
    state: {
      ...source.state,
      domain: {
        ...source.state.domain,
        spaces: [
          {
            space_id: LATTICE_SPACE_ID,
            display_name: "Lattice Lab",
            avatar: null,
            child_room_ids: [GENERAL_ROOM_ID, DESIGN_ROOM_ID, PAPERS_ROOM_ID, RANDOM_ROOM_ID]
          },
          { space_id: PHOTON_SPACE_ID, display_name: "Photon Reading Group", avatar: null, child_room_ids: [] },
          { space_id: RELEASE_SPACE_ID, display_name: "Release Crew", avatar: null, child_room_ids: [] }
        ],
        rooms: [general, design, papers, random, aki],
        invites: [],
        settings: {
          ...source.state.domain.settings,
          values: {
            ...source.state.domain.settings.values,
            appearance: {
              ...source.state.domain.settings.values.appearance,
              theme: "light",
              density: "compact"
            },
            typography: { font: "inter", emoji: "twemojiColr" },
            sidebar: {
              ...source.state.domain.settings.values.sidebar,
              category: "rooms",
              collapsed: { favourites: false, low_priority: false, not_joined: false }
            }
          }
        },
        typography_profile: {
          ...source.state.domain.typography_profile,
          font: "inter",
          emoji: "twemojiColr",
          font_asset: "bundledPreferred",
          emoji_asset: "bundledPreferred"
        }
      },
      ui: {
        ...source.state.ui,
        navigation: {
          ...source.state.ui.navigation,
          active_space_id: LATTICE_SPACE_ID,
          active_room_id: GENERAL_ROOM_ID,
          home_selection: { kind: "activity" }
        },
        room_list: {
          ...source.state.ui.room_list,
          readiness: { kind: "ready", source: "live", generation },
          active_filter: { kind: "rooms" },
          items: [
            ...[general, design, papers, random].map((room) => ({ room_id: room.room_id, kind: "room" as const })),
            { room_id: AKI_ROOM_ID, kind: "room" as const }
          ]
        },
        timeline: {
          ...source.state.ui.timeline,
          room_id: GENERAL_ROOM_ID,
          is_subscribed: true,
          composer: {
            ...source.state.ui.timeline.composer,
            draft: "",
            document: { version: 2, inlines: [] }
          }
        },
        thread: { kind: "closed" }
      }
    },
    sidebar: {
      ...source.sidebar,
      active_space_id: LATTICE_SPACE_ID,
      account_home: { ...source.sidebar.account_home, is_active: false, invite_count: 0, attention_count: 0 },
      space_rail: [
        { space_id: LATTICE_SPACE_ID, display_name: "Lattice Lab", avatar: null, unread_count: 0, highlight_count: 0, is_active: true },
        { space_id: PHOTON_SPACE_ID, display_name: "Photon Reading Group", avatar: null, unread_count: 0, highlight_count: 0, is_active: false },
        { space_id: RELEASE_SPACE_ID, display_name: "Release Crew", avatar: null, unread_count: 0, highlight_count: 0, is_active: false }
      ],
      space_rooms: [roomListItem(general), roomListItem(design, { displayCount: 3 }), roomListItem(papers), roomListItem(random)],
      not_joined_space_rooms: [],
      global_dms: dmItems,
      space_unread_count: 3,
      dm_unread_count: 0,
      space_highlight_count: 0,
      dm_highlight_count: 0,
      sections: {
        favourites: favouriteItems,
        rooms: [...roomItems, ...dmItems],
        people: dmItems,
        low_priority: [],
        not_joined: []
      }
    },
    timeline: [],
    thread: null
  };

  const initialItems: TimelineItem[] = [
    {
      id: { Synthetic: { synthetic_id: "date-divider-1773133200000" } },
      sender: null,
      body: null,
      timestamp_ms: 1773133200000,
      in_reply_to_event_id: null,
      thread_root: null,
      thread_summary: null,
      can_react: false,
      is_redacted: false,
      is_hidden: false,
      can_redact: false,
      is_edited: false,
      can_edit: false,
      reactions: []
    },
    message(session.user_id, 0, "@aki:example.invalid", "Aki", "Welcome to the planning room."),
    message(session.user_id, 1, "@aki:example.invalid", "Aki", "I drafted a small outline for today."),
    message(session.user_id, 2, "@ren:example.invalid", "Ren", "The main flow should stay easy to scan."),
    message(session.user_id, 3, "@sora:example.invalid", "Sora", "I agree; the quiet layout helps.", {
      reply_quote: {
        event_id: "$readme-message-00:example.invalid",
        sender: "@aki:example.invalid",
        sender_label: "Aki",
        body_preview: "Welcome to the planning room.",
        state: "ready"
      },
      thread_summary: null,
      reactions: []
    }),
    message(session.user_id, 4, session.user_id, "You (Koushi)", "Let's keep the first pass focused.", {
      reply_quote: null,
      thread_summary: {
        reply_count: 3,
        latest_event_id: "$readme-thread-latest:example.invalid",
        latest_sender: "@sora:example.invalid",
        latest_sender_label: "Sora",
        latest_body_preview: "I can review the edge cases.",
        latest_timestamp_ms: DATE_START + 26 * 60_000
      },
      reactions: [
        {
          key: "👍",
          count: 2,
          reacted_by_me: true,
          my_reaction_event_id: "$readme-reaction-thumb:example.invalid",
          sender_preview: [
            { user_id: session.user_id, display_label: "You (Koushi)" },
            { user_id: "@ren:example.invalid", display_label: "Ren" }
          ]
        },
        {
          key: "🎉",
          count: 1,
          reacted_by_me: false,
          my_reaction_event_id: null,
          sender_preview: [{ user_id: "@aki:example.invalid", display_label: "Aki" }]
        }
      ]
    }),
    message(session.user_id, 5, session.user_id, "You (Koushi)", "The room list reads clearly to me."),
    message(session.user_id, 6, session.user_id, "You (Koushi)", "I will check the typography next."),
    message(session.user_id, 7, session.user_id, "You (Koushi)", "I will share the final notes here."),
    message(session.user_id, 8, "@aki:example.invalid", "Aki", "Great, this is ready for a calm review.")
  ];
  const key = roomTimelineKey(session.user_id, GENERAL_ROOM_ID);
  const stateUpdate: StateUpdateEnvelope = {
    protocol_version: 1,
    kind: "snapshot",
    generation,
    snapshot,
    reason: "settlement"
  };
  const initialItemsEvent: CoreEventPayload = {
    kind: "Timeline",
    event: {
      InitialItems: {
        request_id: null,
        key,
        generation: 1,
        items: initialItems
      }
    }
  };
  return { snapshot, stateUpdate, initialItems: initialItemsEvent };
}
