export type SearchScopeKind = "currentRoom" | "currentSpace" | "dms" | "allRooms";

export interface DesktopSnapshot {
  state: AppState;
  sidebar: SidebarModel;
  timeline: TimelineMessage[];
  thread: ThreadSnapshot | null;
}

export interface SavedSessionInfo {
  homeserver: string;
  user_id: string;
  device_id: string;
}

export interface AppState {
  session: SessionState;
  auth: AuthDiscoveryState;
  sync: SyncState;
  navigation: NavigationState;
  spaces: SpaceSummary[];
  rooms: RoomSummary[];
  timeline: TimelinePaneState;
  thread: ThreadPaneState;
  focused_context: FocusedContextState;
  search: SearchState;
  errors: AppError[];
  basic_operation: BasicOperationState;
}

export type AuthDiscoveryState =
  | { kind: "unknown" }
  | { kind: "discovering"; homeserver: string }
  | { kind: "ready"; homeserver: string; flows: LoginFlow[] }
  | { kind: "failed"; homeserver: string; message: string };

export interface LoginFlow {
  kind: "password" | "sso" | "token" | { unknown: string };
  delegated_oidc_compatibility: boolean;
}

export type RecoveryMethod = "recoveryKey" | "securityPhrase";

export interface SessionState {
  kind:
    | "signedOut"
    | "restoring"
    | "switchingAccount"
    | "authenticating"
    | "needsRecovery"
    | "recovering"
    | "ready"
    | "locked"
    | "loggingOut";
  homeserver?: string;
  user_id?: string;
  device_id?: string;
  recovery_methods?: RecoveryMethod[];
}

export type SyncState =
  | "stopped"
  | "starting"
  | "running"
  | { failed: string }
  | { reconnecting: string };

export interface NavigationState {
  active_space_id: string | null;
  active_room_id: string | null;
}

export interface SpaceSummary {
  space_id: string;
  display_name: string;
  child_room_ids: string[];
}

export interface RoomSummary {
  room_id: string;
  display_name: string;
  is_dm: boolean;
  unread_count: number;
  notification_count?: number;
  highlight_count?: number;
  parent_space_ids: string[];
}

export interface TimelinePaneState {
  room_id: string | null;
  is_subscribed: boolean;
  is_paginating_backwards: boolean;
  composer: ComposerState;
}

export interface ComposerState {
  pending_transaction_id: string | null;
  draft: string;
  mode: ComposerMode;
}

// Rust ComposerMode has NO serde tag → externally tagged (like SyncState in this file)
export type ComposerMode =
  | "Plain"
  | { Reply: { in_reply_to_event_id: string } };

// Rust BasicOperationState is #[serde(tag = "kind", rename_all = "camelCase")]
// → internally tagged, camelCase VARIANT names, snake_case fields. Pending
// variants carry the correlation request_id (see docs/architecture/state-machine.md).
export type BasicOperationState =
  | { kind: "idle" }
  | { kind: "creatingRoom"; request_id: number; name: string }
  | { kind: "creatingSpace"; request_id: number; name: string }
  | { kind: "linkingSpaceChild"; request_id: number; space_id: string; child_room_id: string };

export interface ThreadPaneState {
  kind: "closed" | "opening" | "open";
  room_id?: string;
  root_event_id?: string;
  is_subscribed?: boolean;
  composer?: ComposerState;
}

export type FocusedContextState =
  | { kind: "closed" }
  | { kind: "opening"; room_id: string; event_id: string }
  | {
      kind: "open";
      room_id: string;
      event_id: string;
      is_subscribed: boolean;
    };

export type SearchState =
  | { kind: "closed" }
  | { kind: "editing"; query: string; scope: SearchScopeKind }
  | { kind: "searching"; request_id: number; query: string; scope: SearchScopeKind }
  | {
      kind: "results";
      request_id: number;
      query: string;
      scope: SearchScopeKind;
      results: SearchResult[];
    }
  | {
      kind: "failed";
      request_id: number;
      query: string;
      scope: SearchScopeKind;
      message: string;
    };

export interface SearchResult {
  room_id: string;
  event_id: string;
  sender: string;
  timestamp_ms: number;
  score_millis: number;
  snippet: string;
  match_field: "messageBody" | "attachmentFileName";
  highlights: TextRange[];
  match_kind: "exact";
}

export interface TextRange {
  start_utf16: number;
  end_utf16: number;
}

export interface AppError {
  code: string;
  message: string;
  recoverable: boolean;
}

export interface SidebarModel {
  active_space_id: string | null;
  account_home: AccountHomeItem;
  space_rail: SpaceRailItem[];
  space_rooms: RoomListItem[];
  global_dms: RoomListItem[];
  space_unread_count: number;
  dm_unread_count: number;
}

export interface AccountHomeItem {
  display_name: string;
  unread_count: number;
  is_active: boolean;
}

export interface SpaceRailItem {
  space_id: string;
  display_name: string;
  unread_count: number;
  is_active: boolean;
}

export interface RoomListItem {
  room_id: string;
  display_name: string;
  unread_count: number;
}

export interface TimelineMessage {
  room_id: string;
  event_id: string;
  sender: string;
  timestamp_ms: number;
  body: string;
  attachment_filename: string | null;
  reply_count: number;
}

export interface ThreadSnapshot {
  room_id: string;
  root_event_id: string;
  replies: ThreadMessage[];
}

export interface ThreadMessage {
  room_id: string;
  root_event_id: string;
  event_id: string;
  sender: string;
  timestamp_ms: number;
  body: string;
}

export interface VisibleRooms {
  spaceRooms: RoomListItem[];
  globalDms: RoomListItem[];
}
