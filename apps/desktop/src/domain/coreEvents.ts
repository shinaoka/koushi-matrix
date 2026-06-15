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
 * `core_event_wire_format_matches_checked_in_contract_artifact` in src-tauri
 * lib.rs. That test compares representative serialized payloads with
 * coreEvents.generated.json, so Rust/TypeScript drift fails locally.
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
  | "InvalidReactionTarget"
  | "InvalidReactionState"
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

export interface ReactionGroup {
  key: string;
  count: number;
  reacted_by_me: boolean;
  my_reaction_event_id: string | null;
  sender_preview: string[];
}

export type TimelineMediaKind = "Image" | "File" | "Audio" | "Video";

export interface TimelineMediaSource {
  mxc_uri: string;
  encrypted: boolean;
  encryption_version: string | null;
}

export interface TimelineMediaThumbnail {
  source: TimelineMediaSource;
  mimetype: string | null;
  size: number | null;
  width: number | null;
  height: number | null;
}

export interface TimelineMedia {
  kind: TimelineMediaKind;
  filename: string;
  source: TimelineMediaSource;
  mimetype: string | null;
  size: number | null;
  width: number | null;
  height: number | null;
  thumbnail: TimelineMediaThumbnail | null;
}

export interface TimelineItem {
  id: TimelineItemId;
  sender: string | null;
  body: string | null;
  timestamp_ms: number | null;
  in_reply_to_event_id: string | null;
  thread_root: string | null;
  thread_summary: ThreadSummaryDto | null;
  media?: TimelineMedia | null;
  reactions: ReactionGroup[];
  can_react: boolean;
  is_redacted: boolean;
  can_redact: boolean;
  is_edited: boolean;
  can_edit: boolean;
}

export interface ThreadSummaryDto {
  reply_count: number;
  latest_sender: string | null;
  latest_body_preview: string | null;
  latest_timestamp_ms: number | null;
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

export interface MediaTransferProgress {
  current: number;
  total: number;
}

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
      MediaUploadProgress: {
        request_id: RequestId | null;
        key: TimelineKey;
        transaction_id: string;
        index: number;
        progress: MediaTransferProgress;
        source: TimelineMediaSource | null;
      };
    }
  | {
      MediaDownloadCompleted: {
        request_id: RequestId;
        key: TimelineKey;
        event_id: string;
        byte_count: number;
        mimetype: string | null;
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
  | { AccountSwitched: { request_id: RequestId; account_key: string } }
  | { ProfileUpdated: { request_id: RequestId; account_key: string } };

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
  | { InviteAccepted: { request_id: RequestId; room_id: string } }
  | { InviteDeclined: { request_id: RequestId; room_id: string } }
  | { DirectMessageStarted: { request_id: RequestId; room_id: string } }
  | { RoomJoined: { request_id: RequestId; room_id: string } }
  | { RoomLeft: { request_id: RequestId; room_id: string } }
  | { RoomForgotten: { request_id: RequestId; room_id: string } }
  | { RoomTagSet: { request_id: RequestId; room_id: string; tag: RoomTagKind } }
  | { RoomTagRemoved: { request_id: RequestId; room_id: string; tag: RoomTagKind } }
  | "RoomListUpdated";

export type RoomTagKind = "favourite" | "lowPriority";

export type PresenceKind = "online" | "away" | "offline";

export interface LiveReadReceipt {
  user_id: string;
  timestamp_ms: number | null;
}

export interface LiveEventReceipts {
  event_id: string;
  receipts: LiveReadReceipt[];
}

export interface LiveRoomSignalUpdate {
  receipts_by_event: LiveEventReceipts[];
  fully_read_event_id: string | null;
  typing_user_ids: string[];
}

export type LiveSignalsEvent =
  | {
      kind: "roomSignalsUpdated";
      room_id: string;
      update: LiveRoomSignalUpdate;
    }
  | {
      kind: "presenceUpdated";
      user_id: string;
      presence: PresenceKind;
    }
  | {
      kind: "readReceiptSent";
      request_id: RequestId;
      key: TimelineKey;
      event_id: string;
    }
  | {
      kind: "fullyReadSet";
      request_id: RequestId;
      key: TimelineKey;
      event_id: string;
    }
  | {
      kind: "typingSet";
      request_id: RequestId;
      key: TimelineKey;
      is_typing: boolean;
    }
  | {
      kind: "presenceSet";
      request_id: RequestId;
      presence: PresenceKind;
    };

export interface SearchResultItem {
  room_id: string;
  event_id: string;
  snippet: string;
}

export type SearchEvent = {
  Results: { request_id: RequestId; results: SearchResultItem[] };
};

// ---------------------------------------------------------------------------
// E2EE trust events (internally tagged by Rust with `kind`)
// ---------------------------------------------------------------------------

export type TrustOperationFailureKind =
  | "cancelled"
  | "mismatch"
  | "network"
  | "forbidden"
  | "timeout"
  | "sdk";

export interface VerificationTarget {
  user_id: string;
  device_id: string;
}

export interface SasEmoji {
  symbol: string;
  description: string;
}

export type VerificationFlowState =
  | { kind: "idle" }
  | { kind: "requested"; request_id: number; target: VerificationTarget }
  | { kind: "accepted"; request_id: number; target: VerificationTarget }
  | {
      kind: "sasPresented";
      request_id: number;
      target: VerificationTarget;
      emojis: SasEmoji[];
    }
  | {
      kind: "confirming";
      request_id: number;
      target: VerificationTarget;
      emojis: SasEmoji[];
    }
  | { kind: "done"; request_id: number; target: VerificationTarget }
  | {
      kind: "failed";
      request_id: number;
      target: VerificationTarget;
      failureKind: TrustOperationFailureKind;
    };

export type CrossSigningStatus =
  | { kind: "unknown" }
  | { kind: "missing" }
  | { kind: "bootstrapping"; request_id: number }
  | { kind: "trusted" }
  | { kind: "notTrusted" }
  | { kind: "failed"; request_id: number; failureKind: TrustOperationFailureKind };

export type KeyBackupStatus =
  | { kind: "unknown" }
  | { kind: "disabled" }
  | { kind: "enabling"; request_id: number }
  | { kind: "enabled"; version: string }
  | {
      kind: "restoring";
      request_id: number;
      version: string | null;
      restored_rooms: number;
      total_rooms: number | null;
    }
  | { kind: "failed"; request_id: number; failureKind: TrustOperationFailureKind };

export type IdentityResetState =
  | { kind: "idle" }
  | { kind: "resetting"; request_id: number }
  | { kind: "awaitingAuth"; request_id: number; auth_type: IdentityResetAuthType }
  | { kind: "failed"; request_id: number; failureKind: TrustOperationFailureKind };

export type IdentityResetAuthType = "uiaa" | "oauth" | "unknown";

export type E2eeTrustEvent =
  | {
      kind: "verificationProgress";
      account_key: string;
      state: VerificationFlowState;
    }
  | {
      kind: "crossSigningChanged";
      account_key: string;
      status: CrossSigningStatus;
    }
  | {
      kind: "keyBackupChanged";
      account_key: string;
      status: KeyBackupStatus;
    }
  | {
      kind: "identityResetChanged";
      account_key: string;
      state: IdentityResetState;
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
  | { kind: "LiveSignals"; event: LiveSignalsEvent }
  | { kind: "Search"; event: SearchEvent }
  | { kind: "E2eeTrust"; event: E2eeTrustEvent }
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

export function focusedTimelineKey(
  accountKey: string,
  roomId: string,
  eventId: string
): TimelineKey {
  return {
    account_key: accountKey,
    kind: { Focused: { room_id: roomId, event_id: eventId } }
  };
}

export function threadTimelineKey(
  accountKey: string,
  roomId: string,
  rootEventId: string
): TimelineKey {
  return {
    account_key: accountKey,
    kind: { Thread: { room_id: roomId, root_event_id: rootEventId } }
  };
}
