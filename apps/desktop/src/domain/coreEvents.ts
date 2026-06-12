/**
 * TypeScript types for CoreEvent and AppStateSnapshot IPC payloads.
 *
 * These are the serialised forms of the Rust CoreEvent / AppStateSnapshot
 * types as emitted on `matrix-desktop://event` and `matrix-desktop://state`.
 * Codegen from the Rust types is recorded as future work (Phase 9 cleanup).
 *
 * Design: all types are plain data (no class instances).  The transport
 * delivers JSON; React components and stores consume these shapes directly.
 *
 * Security: message bodies flow in Timeline events.  These are visible
 * content (not secrets).  Passwords, access tokens, and store keys NEVER
 * appear in any CoreEvent.  Do not add debug logging of these payloads in
 * release builds.
 *
 * References:
 *   docs/architecture/overview.md — "Async rule 4" (timeline as diffs),
 *     "Timeline Viewport And Scrollback", "Webview threat model"
 *   docs/superpowers/specs/2026-06-12-headless-core-runtime-design.md
 *     — "Public Runtime API" (CoreEvent enum, TimelineEvent, etc.)
 */

// ---------------------------------------------------------------------------
// Identity types
// ---------------------------------------------------------------------------

export interface RequestId {
  connection_id: number;
  sequence: number;
}

export interface TimelineKey {
  account_key: string;
  kind: TimelineKind;
}

export type TimelineKind =
  | { Room: { room_id: string } }
  | { Thread: { room_id: string; root_event_id: string } }
  | { Focused: { room_id: string; event_id: string } };

export interface TimelineGeneration {
  value: number;
}

export type PaginationDirection = "Backward" | "Forward";

export type PaginationState =
  | "Idle"
  | "Paginating"
  | "EndReached"
  | { Failed: { kind: TimelineFailureKind } };

export type TimelineFailureKind =
  | "InvalidDirection"
  | "NotSubscribed"
  | "Forbidden"
  | "Network"
  | "Timeout"
  | "Sdk"
  | "QueueOverflow";

export type TimelineResyncReason =
  | "QueueOverflow"
  | "GenerationReset"
  | "SubscriptionChanged";

// ---------------------------------------------------------------------------
// Timeline items (forwarded from core; never stored in AppState snapshots)
// ---------------------------------------------------------------------------

export interface TimelineItem {
  /** Stable identity: remote event_id, local txn_id, or synthetic id. */
  id: string;
  kind: TimelineItemKind;
}

export type TimelineItemKind =
  | { message: TimelineMessageContent }
  | { virtual: VirtualItemContent }
  | { redacted: { event_id: string } };

export interface TimelineMessageContent {
  event_id: string;
  /** Null for local-echo items whose remote echo has not arrived. */
  remote_event_id: string | null;
  transaction_id: string | null;
  sender: string;
  timestamp_ms: number;
  body: string;
  attachment_filename: string | null;
  reply_count: number;
  room_id: string;
  is_edited: boolean;
}

export interface VirtualItemContent {
  kind: "DayDivider" | "ReadMarker" | "TimelineStart";
  timestamp_ms?: number;
}

// ---------------------------------------------------------------------------
// VectorDiff (positional operations on a Vec<TimelineItem>)
// ---------------------------------------------------------------------------

export type TimelineDiff =
  | { PushFront: { item: TimelineItem } }
  | { PushBack: { item: TimelineItem } }
  | { Insert: { index: number; item: TimelineItem } }
  | { Set: { index: number; item: TimelineItem } }
  | { Remove: { index: number } }
  | { Truncate: { length: number } }
  | { Clear: Record<string, never> }
  | { Reset: { items: TimelineItem[] } };

// ---------------------------------------------------------------------------
// CoreEvent discriminated union (from matrix-desktop://event)
// ---------------------------------------------------------------------------

export type CoreEvent =
  | { kind: "Account"; event: AccountEvent }
  | { kind: "Sync"; event: SyncEvent }
  | { kind: "Room"; event: RoomEvent }
  | { kind: "Timeline"; event: TimelineEvent }
  | { kind: "Search"; event: SearchEvent }
  | {
      kind: "OperationFailed";
      request_id: RequestId | null;
      failure: CoreFailure;
    }
  /** Emitted by the Tauri adapter when EventStreamLag is detected. */
  | { kind: "ResyncMarker" };

// ---------------------------------------------------------------------------
// Account events
// ---------------------------------------------------------------------------

export type AccountEvent =
  | { kind: "LoggedIn"; account_key: string }
  | { kind: "SessionRestored"; account_key: string }
  | { kind: "SessionNotFound"; account_key: string }
  | { kind: "NeedsRecovery"; account_key: string }
  | { kind: "RecoveryCompleted"; account_key: string }
  | { kind: "LoggedOut"; account_key: string }
  | { kind: "AccountSwitched"; account_key: string };

// ---------------------------------------------------------------------------
// Sync events
// ---------------------------------------------------------------------------

export type SyncEvent =
  | { kind: "Started"; backend: string }
  | { kind: "Running" }
  | { kind: "Reconnecting" }
  | { kind: "Failed"; kind_str: string }
  | { kind: "Stopped" };

// ---------------------------------------------------------------------------
// Room events
// ---------------------------------------------------------------------------

export type RoomEvent =
  | { kind: "RoomListUpdated" }
  | { kind: "RoomCreated"; request_id: RequestId; room_id: string }
  | { kind: "SpaceCreated"; request_id: RequestId; space_id: string }
  | { kind: "RoomSelected"; room_id: string }
  | { kind: "SpaceSelected"; space_id: string | null };

// ---------------------------------------------------------------------------
// Timeline events
// ---------------------------------------------------------------------------

export type TimelineEvent =
  | {
      kind: "InitialItems";
      request_id: RequestId | null;
      key: TimelineKey;
      generation: number;
      items: TimelineItem[];
    }
  | {
      kind: "ItemsUpdated";
      key: TimelineKey;
      generation: number;
      batch_id: number;
      diffs: TimelineDiff[];
    }
  | {
      kind: "PaginationStateChanged";
      request_id: RequestId | null;
      key: TimelineKey;
      direction: PaginationDirection;
      state: PaginationState;
    }
  | {
      kind: "ResyncRequired";
      key: TimelineKey;
      reason: TimelineResyncReason;
    };

// ---------------------------------------------------------------------------
// Search events
// ---------------------------------------------------------------------------

export type SearchEvent =
  | {
      kind: "ResultsReady";
      request_id: RequestId;
      results: SearchEventResult[];
    }
  | { kind: "Failed"; request_id: RequestId; message: string };

export interface SearchEventResult {
  room_id: string;
  event_id: string;
  sender: string;
  timestamp_ms: number;
  score_millis: number;
  snippet: string;
  match_field: "messageBody" | "attachmentFileName";
  highlights: Array<{ start_utf16: number; end_utf16: number }>;
  match_kind: "exact";
}

// ---------------------------------------------------------------------------
// Failure types
// ---------------------------------------------------------------------------

export type CoreFailure =
  | { kind: "SessionRequired" }
  | { kind: "SessionNotFound" }
  | { kind: "LoginFailed"; message?: string }
  | { kind: "RecoveryFailed"; message?: string }
  | { kind: "SyncFailed"; message?: string }
  | { kind: "RoomOperationFailed"; message?: string }
  | { kind: "TimelineOperationFailed"; message?: string }
  | { kind: "SearchFailed"; message?: string }
  | { kind: "LocalEncryptionUnavailable" }
  | { kind: "StoreUnavailable" }
  | { kind: "ShutdownFailed" };

// ---------------------------------------------------------------------------
// Helper: extract TimelineKey room_id for the common Room kind
// ---------------------------------------------------------------------------

export function timelineKeyRoomId(key: TimelineKey): string | null {
  if ("Room" in key.kind) {
    return key.kind.Room.room_id;
  }
  if ("Thread" in key.kind) {
    return key.kind.Thread.room_id;
  }
  if ("Focused" in key.kind) {
    return key.kind.Focused.room_id;
  }
  return null;
}

export function timelineKeyEquals(a: TimelineKey, b: TimelineKey): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}
