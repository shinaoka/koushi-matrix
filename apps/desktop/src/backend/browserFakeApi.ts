import { composeSidebar, roomIsInScope, textRangeUtf16 } from "../domain/desktopModel";
import type {
  DesktopSnapshot,
  RoomSummary,
  SearchResult,
  SearchScopeKind,
  SpaceSummary,
  TimelineMessage
} from "../domain/types";

export interface DesktopApi {
  getSnapshot(): Promise<DesktopSnapshot>;
  selectSpace(spaceId: string | null): Promise<DesktopSnapshot>;
  selectRoom(roomId: string): Promise<DesktopSnapshot>;
  openThread(roomId: string, rootEventId: string): Promise<DesktopSnapshot>;
  closeThread(): Promise<DesktopSnapshot>;
  submitSearch(query: string, scope: SearchScopeKind): Promise<DesktopSnapshot>;
}

export function createBrowserFakeApi(): DesktopApi {
  return new BrowserFakeApi();
}

class BrowserFakeApi implements DesktopApi {
  private snapshot = createInitialSnapshot();

  async getSnapshot(): Promise<DesktopSnapshot> {
    return clone(this.snapshot);
  }

  async selectSpace(spaceId: string | null): Promise<DesktopSnapshot> {
    this.snapshot.state.navigation.active_space_id = spaceId;
    this.snapshot.sidebar = composeSidebar(
      this.snapshot.state.navigation.active_space_id,
      this.snapshot.state.spaces,
      this.snapshot.state.rooms
    );

    const firstRoom = this.snapshot.sidebar.space_rooms[0];
    if (firstRoom) {
      await this.selectRoom(firstRoom.room_id);
    }

    return this.getSnapshot();
  }

  async selectRoom(roomId: string): Promise<DesktopSnapshot> {
    this.snapshot.state.navigation.active_room_id = roomId;
    this.snapshot.state.timeline.room_id = roomId;
    this.snapshot.state.timeline.is_subscribed = true;
    this.snapshot.state.thread = { kind: "closed" };
    this.snapshot.thread = null;
    this.snapshot.timeline = timelineMessages.filter((message) => message.room_id === roomId);
    return this.getSnapshot();
  }

  async openThread(roomId: string, rootEventId: string): Promise<DesktopSnapshot> {
    this.snapshot.state.thread = {
      kind: "open",
      room_id: roomId,
      root_event_id: rootEventId,
      is_subscribed: true,
      composer: { pending_transaction_id: null, draft: "" }
    };
    this.snapshot.thread = {
      room_id: roomId,
      root_event_id: rootEventId,
      replies: threadReplies.filter(
        (reply) => reply.room_id === roomId && reply.root_event_id === rootEventId
      )
    };
    return this.getSnapshot();
  }

  async closeThread(): Promise<DesktopSnapshot> {
    this.snapshot.state.thread = { kind: "closed" };
    this.snapshot.thread = null;
    return this.getSnapshot();
  }

  async submitSearch(query: string, scope: SearchScopeKind): Promise<DesktopSnapshot> {
    const results = search(query, scope, this.snapshot);
    this.snapshot.state.search = {
      kind: "results",
      request_id: Date.now(),
      query,
      scope,
      results
    };
    return this.getSnapshot();
  }
}

function createInitialSnapshot(): DesktopSnapshot {
  const active_space_id = "!space-alpha:example.invalid";
  const active_room_id = "!room-alpha:example.invalid";
  const sidebar = composeSidebar(active_space_id, spaces, rooms);
  const snapshot: DesktopSnapshot = {
    state: {
      session: {
        kind: "ready",
        homeserver: "https://matrix.org",
        user_id: "@demo-user:example.invalid",
        device_id: "FAKEDEVICE"
      },
      sync: "running",
      navigation: {
        active_space_id,
        active_room_id
      },
      spaces,
      rooms,
      timeline: {
        room_id: active_room_id,
        is_subscribed: true,
        is_paginating_backwards: false,
        composer: {
          pending_transaction_id: null,
          draft: ""
        }
      },
      thread: {
        kind: "open",
        room_id: active_room_id,
        root_event_id: "$alpha-update",
        is_subscribed: true,
        composer: {
          pending_transaction_id: null,
          draft: ""
        }
      },
      search: { kind: "closed" },
      errors: []
    },
    sidebar,
    timeline: timelineMessages.filter((message) => message.room_id === active_room_id),
    thread: {
      room_id: active_room_id,
      root_event_id: "$alpha-update",
      replies: threadReplies.filter(
        (reply) => reply.room_id === active_room_id && reply.root_event_id === "$alpha-update"
      )
    }
  };

  return snapshot;
}

function search(
  query: string,
  scope: SearchScopeKind,
  snapshot: DesktopSnapshot
): SearchResult[] {
  if (query.length === 0) {
    return [];
  }

  return timelineMessages
    .filter((message) => roomIsInScope(message.room_id, scope, snapshot))
    .map((message) => searchMessage(message, query))
    .filter((result): result is SearchResult => Boolean(result))
    .sort(
      (left, right) =>
        right.score_millis - left.score_millis ||
        left.timestamp_ms - right.timestamp_ms ||
        left.event_id.localeCompare(right.event_id)
    );
}

function searchMessage(message: TimelineMessage, query: string): SearchResult | null {
  const bodyRange = textRangeUtf16(message.body, query);
  if (bodyRange) {
    return {
      room_id: message.room_id,
      event_id: message.event_id,
      sender: message.sender,
      timestamp_ms: message.timestamp_ms,
      score_millis: candidateScore(message.event_id),
      snippet: message.body,
      match_field: "messageBody",
      highlights: [bodyRange],
      match_kind: "exact"
    };
  }

  if (message.attachment_filename) {
    const attachmentRange = textRangeUtf16(message.attachment_filename, query);
    if (attachmentRange) {
      return {
        room_id: message.room_id,
        event_id: message.event_id,
        sender: message.sender,
        timestamp_ms: message.timestamp_ms,
        score_millis: candidateScore(message.event_id),
        snippet: message.attachment_filename,
        match_field: "attachmentFileName",
        highlights: [attachmentRange],
        match_kind: "exact"
      };
    }
  }

  return null;
}

function candidateScore(eventId: string): number {
  switch (eventId) {
    case "$false-positive":
      return 1000;
    case "$alpha-update":
      return 950;
    case "$budget-file":
      return 900;
    case "$late-original":
      return 850;
    default:
      return 700;
  }
}

function clone<T>(value: T): T {
  return structuredClone(value);
}

const spaces: SpaceSummary[] = [
  {
    space_id: "!space-alpha:example.invalid",
    display_name: "Synthetic Workspace",
    child_room_ids: ["!room-alpha:example.invalid", "!room-planning:example.invalid"]
  },
  {
    space_id: "!space-beta:example.invalid",
    display_name: "Synthetic Lab",
    child_room_ids: ["!room-search:example.invalid"]
  }
];

const rooms: RoomSummary[] = [
  {
    room_id: "!room-alpha:example.invalid",
    display_name: "synthetic-room",
    is_dm: false,
    unread_count: 8,
    parent_space_ids: ["!space-alpha:example.invalid"]
  },
  {
    room_id: "!room-planning:example.invalid",
    display_name: "planning-room",
    is_dm: false,
    unread_count: 2,
    parent_space_ids: ["!space-alpha:example.invalid"]
  },
  {
    room_id: "!room-search:example.invalid",
    display_name: "matrix-sdk-search",
    is_dm: false,
    unread_count: 1,
    parent_space_ids: ["!space-beta:example.invalid"]
  },
  {
    room_id: "!dm-member-1:example.invalid",
    display_name: "Member 1",
    is_dm: true,
    unread_count: 1,
    parent_space_ids: []
  },
  {
    room_id: "!dm-member-2:example.invalid",
    display_name: "Member 2",
    is_dm: true,
    unread_count: 0,
    parent_space_ids: []
  }
];

const timelineMessages: TimelineMessage[] = [
  {
    room_id: "!room-alpha:example.invalid",
    event_id: "$alpha-update",
    sender: "Demo Coordinator",
    timestamp_ms: 1_806_986_400_000,
    body: "Alpha keyword update from demo coordinator.",
    attachment_filename: null,
    reply_count: 2
  },
  {
    room_id: "!room-alpha:example.invalid",
    event_id: "$agenda",
    sender: "Demo Coordinator",
    timestamp_ms: 1_806_990_000_000,
    body: "Synthetic planning note.\n\n- Fixture item one\n- Fixture item two",
    attachment_filename: null,
    reply_count: 0
  },
  {
    room_id: "!room-alpha:example.invalid",
    event_id: "$budget-file",
    sender: "Slackbot",
    timestamp_ms: 1_806_993_600_000,
    body: "Budget spreadsheet attached.",
    attachment_filename: "fixture_budget.xlsx",
    reply_count: 0
  },
  {
    room_id: "!room-alpha:example.invalid",
    event_id: "$false-positive",
    sender: "Member 3",
    timestamp_ms: 1_806_997_200_000,
    body: "Non-matching synthetic note.",
    attachment_filename: null,
    reply_count: 0
  },
  {
    room_id: "!room-planning:example.invalid",
    event_id: "$late-original",
    sender: "Member 1",
    timestamp_ms: 1_807_000_800_000,
    body: "Final synthetic checklist",
    attachment_filename: null,
    reply_count: 0
  },
  {
    room_id: "!room-search:example.invalid",
    event_id: "$search-dev-note",
    sender: "Member 4",
    timestamp_ms: 1_807_004_400_000,
    body: "matrix-sdk-search adapter review notes",
    attachment_filename: null,
    reply_count: 0
  }
];

const threadReplies = [
  {
    room_id: "!room-alpha:example.invalid",
    root_event_id: "$alpha-update",
    event_id: "$thread-1",
    sender: "Member 2",
    timestamp_ms: 1_806_987_000_000,
    body: "Synthetic follow-up item one."
  },
  {
    room_id: "!room-alpha:example.invalid",
    root_event_id: "$alpha-update",
    event_id: "$thread-2",
    sender: "Member 1",
    timestamp_ms: 1_806_987_600_000,
    body: "Synthetic follow-up item two."
  }
];
