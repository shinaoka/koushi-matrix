/**
 * TypeScript types for CoreEvent and AppStateSnapshot IPC payloads.
 *
 * These are the EXACT serialised forms of the Rust CoreEvent payloads as
 * emitted on `matrix-desktop://event` (see `serialize_core_event` in
 * apps/desktop/src-tauri/src/lib.rs). Serde enums are externally tagged:
 * struct variants serialise as `{"Variant":{..}}`, unit variants as
 * `"Variant"`, and newtype wrappers (AccountKey, TimelineGeneration,
 * TimelineBatchId) collapse to their inner value.
 *
 * The wire format is pinned by the Rust contract test
 * `core_event_wire_format_matches_typescript_contract` in src-tauri lib.rs.
 * If that test changes, this module must change with it. Codegen from the
 * Rust types is recorded as future work (Phase 9 cleanup).
 *
 * Security: message bodies flow in Timeline events. These are visible
 * content (not secrets). Passwords, access tokens, and store keys NEVER
 * appear in any CoreEvent. Do not add debug logging of these payloads in
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
  /** RuntimeConnectionId is a newtype over u64 → plain number on the wire. */
  connection_id: number;
  sequence: number;
}

export interface TimelineKey {
  /** AccountKey is a newtype over String → plain string on the wire. */
  account_key: string;
  kind: TimelineKind;
}

export type TimelineKind =
  | { Room: { room_id: string } }
  | { Thread: { room_id: string; root_event_id: string } }
  | { Focused: { room_id: string; event_id: string } };

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

export type TimelineResyncReason = "QueueOverflow" | "SubscriptionRestarted";

// ---------------------------------------------------------------------------
// Timeline items (stable identity contract; Viewport/Scrollback)
// ---------------------------------------------------------------------------

export type TimelineItemId =
  | { Event: { event_id: string } }
  | { Transaction: { transaction_id: string } }
  | { Synthetic: { synthetic_id: string } };

export interface TimelineItem {
  id: TimelineItemId;
  sender: string | null;
  body: string | null;
  timestamp_ms: number | null;
}

/** Stable string id usable as a React key and a `data-item-id` DOM hook. */
export function timelineItemDomId(id: TimelineItemId): string {
  if ("Event" in id) {
    return id.Event.event_id;
  }
  if ("Transaction" in id) {
    return `txn:${id.Transaction.transaction_id}`;
  }
  return `syn:${id.Synthetic.synthetic_id}`;
}

// ---------------------------------------------------------------------------
// VectorDiff (externally tagged; unit variants are bare strings)
// ---------------------------------------------------------------------------

export type TimelineDiff =
  | { PushFront: { item: TimelineItem } }
  | { PushBack: { item: TimelineItem } }
  | { Insert: { index: number; item: TimelineItem } }
  | { Set: { index: number; item: TimelineItem } }
  | { Remove: { index: number } }
  | { Truncate: { length: number } }
  | "Clear"
  | { Reset: { items: TimelineItem[] } };

// ---------------------------------------------------------------------------
// Timeline events (externally tagged on the wire)
// ---------------------------------------------------------------------------

export type TimelineEvent =
  | {
      InitialItems: {
        request_id: RequestId | null;
        key: TimelineKey;
        /** TimelineGeneration newtype → number. */
        generation: number;
        items: TimelineItem[];
      };
    }
  | {
      ItemsUpdated: {
        key: TimelineKey;
        generation: number;
        /** TimelineBatchId newtype → number. */
        batch_id: number;
        diffs: TimelineDiff[];
      };
    }
  | {
      PaginationStateChanged: {
        request_id: RequestId | null;
        key: TimelineKey;
        direction: PaginationDirection;
        state: PaginationState;
      };
    }
  | {
      SendCompleted: {
        request_id: RequestId;
        key: TimelineKey;
        transaction_id: string;
        event_id: string;
      };
    }
  | {
      ResyncRequired: {
        key: TimelineKey;
        reason: TimelineResyncReason;
      };
    };

// ---------------------------------------------------------------------------
// Account / Sync / Room / Search events (externally tagged)
// ---------------------------------------------------------------------------

export interface SessionInfo {
  homeserver: string;
  user_id: string;
  device_id: string;
}

export type AccountEvent =
  | { LoggedIn: { request_id: RequestId; account_key: string } }
  | { SessionRestored: { request_id: RequestId; account_key: string } }
  | { SavedSessionsListed: { request_id: RequestId; sessions: SessionInfo[] } }
  | { RecoveryRequired: { account_key: string } }
  | { RecoveryCompleted: { request_id: RequestId; account_key: string } }
  | { LoggedOut: { request_id: RequestId; account_key: string } }
  | { AccountSwitched: { request_id: RequestId; account_key: string } };

export type SyncBackendKind = "SyncService" | "LegacySync";

export type SyncEvent =
  | { Started: { request_id: RequestId | null; backend: SyncBackendKind } }
  | "Running"
  | "Reconnecting"
  | "Failed"
  | { Stopped: { request_id: RequestId | null } };

export type RoomEvent =
  | { RoomCreated: { request_id: RequestId; room_id: string } }
  | { SpaceCreated: { request_id: RequestId; space_id: string } }
  | {
      SpaceChildSet: {
        request_id: RequestId;
        space_id: string;
        child_room_id: string;
      };
    }
  | { UserInvited: { request_id: RequestId; room_id: string; user_id: string } }
  | { RoomJoined: { request_id: RequestId; room_id: string } }
  | "RoomListUpdated";

export interface SearchResultItem {
  room_id: string;
  event_id: string;
  snippet: string;
}

export type SearchEvent = {
  Results: { request_id: RequestId; results: SearchResultItem[] };
};

// ---------------------------------------------------------------------------
// Failures (externally tagged; unit variants are bare strings)
// ---------------------------------------------------------------------------

export type CoreFailure =
  | "SessionRequired"
  | "SessionNotFound"
  | { LoginFailed: { kind: string } }
  | { RecoveryFailed: { kind: string } }
  | { SyncFailed: { kind: string } }
  | { RoomOperationFailed: { kind: string } }
  | { TimelineOperationFailed: { kind: TimelineFailureKind } }
  | { SearchFailed: { kind: string } }
  | "LocalEncryptionUnavailable"
  | "StoreUnavailable"
  | "ShutdownFailed";

// ---------------------------------------------------------------------------
// CoreEvent envelope (the `matrix-desktop://event` payload shape produced by
// serialize_core_event in src-tauri lib.rs)
// ---------------------------------------------------------------------------

export type CoreEventPayload =
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
// Helpers
// ---------------------------------------------------------------------------

export function timelineKeyRoomId(key: TimelineKey): string {
  if ("Room" in key.kind) {
    return key.kind.Room.room_id;
  }
  if ("Thread" in key.kind) {
    return key.kind.Thread.room_id;
  }
  return key.kind.Focused.room_id;
}

export function timelineKeyEquals(a: TimelineKey, b: TimelineKey): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

export function roomTimelineKey(accountKey: string, roomId: string): TimelineKey {
  return { account_key: accountKey, kind: { Room: { room_id: roomId } } };
}
