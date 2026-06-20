/**
 * TypeScript types for CoreEvent and AppStateSnapshot IPC payloads.
 *
 * These are the EXACT serialised forms of the Rust CoreEvent payloads as
 * emitted on `koushi-desktop://event` (see `serialize_core_event` in
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

import type { AttachmentResult, SearchCrawlerFailureKind, SyncMode, ThreadsListItem } from "./types";
import type { LinkPreview } from "./linkPreview";

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
  | "InvalidSendTarget"
  | "InvalidSendState"
  | "UnsupportedSlashCommand"
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

export type { LinkPreview, LinkPreviewImage, LinkPreviewState } from "./linkPreview";

export type TimelineSendFailureReason = "recoverable" | "unrecoverable";

export type TimelineSendState =
  | { kind: "sending" }
  | { kind: "notSent"; reason: TimelineSendFailureReason }
  | { kind: "cancelled" }
  | { kind: "sent" };

export interface TimelineMessageActions {
  can_copy: boolean;
  can_forward: boolean;
  can_permalink: boolean;
  can_view_source: boolean;
  permalink?: string | null;
}

export interface TimelineMessageSource {
  event_id: string;
  sender: string | null;
  timestamp_ms: number | null;
  body: string | null;
  in_reply_to_event_id: string | null;
  thread_root: string | null;
  is_redacted: boolean;
  is_edited: boolean;
  has_media: boolean;
  original_json?: unknown | null;
}

export interface TimelineCodeBlock {
  language: string | null;
  body: string;
}

export interface TimelineFormattedBody {
  html: string;
  plain_text: string;
  code_blocks: TimelineCodeBlock[];
}

export type TimelineMessageKind = "text" | "emote" | "notice";

export interface TimelineSpoilerSpan {
  start_utf16: number;
  end_utf16: number;
  reason?: string | null;
}

export interface TimelineItem {
  id: TimelineItemId;
  sender: string | null;
  sender_label?: string | null;
  sender_avatar?: AvatarImage | null;
  body: string | null;
  notice_i18n_key?: string | null;
  message_kind?: TimelineMessageKind;
  spoiler_spans?: TimelineSpoilerSpan[];
  timestamp_ms: number | null;
  in_reply_to_event_id: string | null;
  formatted?: TimelineFormattedBody | null;
  reply_quote?: ReplyQuote | null;
  thread_root: string | null;
  thread_summary: ThreadSummaryDto | null;
  media?: TimelineMedia | null;
  link_previews?: LinkPreview[];
  reactions: ReactionGroup[];
  can_react: boolean;
  is_redacted: boolean;
  is_hidden: boolean;
  can_redact: boolean;
  is_edited: boolean;
  can_edit: boolean;
  actions?: TimelineMessageActions;
  send_state?: TimelineSendState | null;
}

export interface ThreadSummaryDto {
  reply_count: number;
  latest_sender: string | null;
  latest_sender_label?: string | null;
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

export interface TimelineDisplayLabelUpdate {
  user_id: string;
  display_label: string;
}

export type TimelineUnreadPosition =
  | "none"
  | "aboveViewport"
  | "insideViewport"
  | "belowViewport"
  | "unknown";

export interface TimelineNavigationSnapshot {
  read_marker_event_id: string | null;
  first_unread_event_id: string | null;
  unread_event_count: number;
  unread_position: TimelineUnreadPosition;
  newer_event_count: number;
  can_jump_to_bottom: boolean;
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
      NavigationUpdated: {
        key: TimelineKey;
        snapshot: TimelineNavigationSnapshot;
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
      MessageForwarded: {
        request_id: RequestId;
        key: TimelineKey;
        destination_room_id: string;
        transaction_id: string;
        event_id: string;
      };
    }
  | {
      MessageSourceLoaded: {
        request_id: RequestId;
        key: TimelineKey;
        source: TimelineMessageSource;
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
      MediaDownloadProgress: {
        request_id: RequestId;
        key: TimelineKey;
        event_id: string;
        progress: MediaTransferProgress;
      };
    }
  | {
      MediaDownloadCompleted: {
        request_id: RequestId;
        key: TimelineKey;
        event_id: string;
        source_url: string;
        byte_count: number;
        mimetype: string | null;
        width: number | null;
        height: number | null;
      };
    }
  | {
      MediaDownloadFailed: {
        request_id: RequestId;
        key: TimelineKey;
        event_id: string;
        kind: TimelineFailureKind;
      };
    }
  | {
      ResyncRequired: {
        key: TimelineKey;
        reason: TimelineResyncReason;
      };
    }
  | {
      DisplayPolicyUpdated: {
        hide_redacted: boolean;
      };
    }
  | {
      DisplayLabelsUpdated: {
        labels: TimelineDisplayLabelUpdate[];
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
  | { ProfileUpdated: { request_id: RequestId; account_key: string } }
  | {
      AvatarThumbnailDownloaded: {
        request_id: RequestId;
        mxc_uri: string;
        thumbnail: AvatarThumbnailState;
      };
    }
  | { ReportCompleted: { request_id: RequestId; kind: ReportKind } };

export type SyncBackendKind = "SyncService" | "LegacySync";

export type SyncEvent =
  | { Started: { request_id: RequestId | null; backend: SyncBackendKind } }
  | "Running"
  | "Reconnecting"
  | "Failed"
  | { Stopped: { request_id: RequestId | null } }
  | { ModeChanged: { mode: SyncMode } };

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
  | { PinnedEventsUpdated: { room_id: string; pinned: PinnedEvent[] } }
  | { PinEventCompleted: { request_id: RequestId; room_id: string } }
  | { UnpinEventCompleted: { request_id: RequestId; room_id: string } }
  | {
      DirectoryQueryCompleted: {
        request_id: RequestId;
        query: DirectoryQuery;
        rooms: DirectoryRoomSummary[];
        next_batch: string | null;
      };
    }
  | { RoomSettingsLoaded: { request_id: RequestId; settings: RoomSettingsSnapshot } }
  | { RoomSettingUpdated: { request_id: RequestId; settings: RoomSettingsSnapshot } }
  | {
      RoomMemberModerated: {
        request_id: RequestId;
        room_id: string;
        target_user_id: string;
        action: RoomModerationAction;
      };
    }
  | {
      RoomMemberRoleUpdated: {
        request_id: RequestId;
        room_id: string;
        target_user_id: string;
        power_level: number;
      };
    }
  | { MarkedAsRead: { request_id: RequestId; room_id: string } }
  | { MarkedAsUnread: { request_id: RequestId; room_id: string; unread: boolean } }
  | { ReportCompleted: { request_id: RequestId; kind: ReportKind } }
  | "RoomListUpdated";

export type ReportKind = "event" | "room" | "user";

export type RoomTagKind = "favourite" | "lowPriority";

export interface PinnedEvent {
  event_id: string;
  sender: string | null;
  body_preview: string | null;
  redacted: boolean;
}

export interface ReplyQuote {
  event_id: string;
  sender: string | null;
  sender_label?: string | null;
  body_preview: string | null;
  state: ReplyQuoteState;
}

export type ReplyQuoteState = "ready" | "redacted" | "missing" | "unsupported";

export interface DirectoryQuery {
  term: string | null;
  server_name: string | null;
  limit: number | null;
  since: string | null;
}

export interface DirectoryRoomSummary {
  room_id: string;
  canonical_alias: string | null;
  name: string;
  topic: string | null;
  avatar_url: string | null;
  joined_members: number;
  world_readable: boolean;
  guest_can_join: boolean;
}

export interface RoomSettingsSnapshot {
  room_id: string;
  name: string | null;
  topic: string | null;
  avatar_url: string | null;
  join_rule: RoomJoinRule;
  history_visibility: RoomHistoryVisibility;
  permissions: RoomPermissionFacts;
  members: RoomMemberSummary[];
}

export interface RoomMemberSummary {
  user_id: string;
  display_name: string | null;
  display_label: string;
  original_display_label: string;
  avatar_url: string | null;
  power_level: number | null;
  role: RoomMemberRole;
}

export type RoomMemberRole = "creator" | "administrator" | "moderator" | "user";

export type RoomJoinRule = "public" | "invite" | "knock" | "restricted" | "private";

export type RoomHistoryVisibility = "worldReadable" | "shared" | "invited" | "joined";

export interface RoomPermissionFacts {
  can_edit_settings: boolean;
  can_edit_roles: boolean;
  can_kick: boolean;
  can_ban: boolean;
  can_unban: boolean;
}

export type RoomModerationAction = "kick" | "ban" | "unban";

export type PresenceKind = "online" | "away" | "offline";

export interface AvatarImage {
  mxc_uri: string;
  thumbnail: AvatarThumbnailState;
}

export type AvatarThumbnailState =
  | { kind: "notRequested" }
  | { kind: "loading"; request_id: number }
  | {
      kind: "ready";
      source_url: string;
      width: number | null;
      height: number | null;
      mime_type: string | null;
    }
  | { kind: "failed"; request_id: number; failureKind: AvatarThumbnailFailureKind };

export type AvatarThumbnailFailureKind = "network" | "forbidden" | "unsupported" | "sdk";

export interface LiveReadReceipt {
  user_id: string;
  display_name: string | null;
  original_display_label: string;
  avatar: AvatarImage | null;
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
  AttachmentsResults: { request_id: RequestId; results: AttachmentResult[] };
  AttachmentsFailed: { request_id: RequestId; message: string };
  IndexUpdated: { room_id: string; event_id: string };
  HistoryCrawlProgress: { room_id: string; processed: number; indexed: number };
  HistoryCrawlCompleted: { room_id: string; indexed: number };
  /** Failure carries only a coarse kind — no raw SDK error text (privacy rule). */
  HistoryCrawlFailed: { room_id: string; failureKind: SearchCrawlerFailureKind };
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

export type ActivityEvent =
  | { Opened: { request_id: RequestId } }
  | { Closed: { request_id: RequestId } }
  | {
      SnapshotLoaded: {
        request_id: RequestId;
        active_tab: ActivityTab;
        recent: ActivityStream;
        unread: ActivityStream;
      };
    }
  | { TabSelected: { request_id: RequestId; tab: ActivityTab } }
  | { MarkedRead: { request_id: RequestId; cleared_event_ids: string[] } };

export type ActivityTab = "recent" | "unread";

export interface ActivityStream {
  rows: ActivityRow[];
  next_batch: string | null;
}

export interface ActivityRow {
  room_id: string;
  event_id: string;
  room_label: string;
  sender_label: string | null;
  preview: string | null;
  timestamp_ms: number;
  unread: boolean;
  highlight: boolean;
}

export type LocalEncryptionHealth =
  | "unknown"
  | "healthy"
  | "unavailable"
  | "lockedOrInaccessible"
  | "missingCredential"
  | "resetRequired";

export type LocalEncryptionEvent = {
  kind: "healthChanged";
  health: LocalEncryptionHealth;
};

export type OperationFailureKind =
  | "forbidden"
  | "notFound"
  | "network"
  | "timeout"
  | "invalid"
  | "sdk";

export interface NativeAttentionSummary {
  unread_count: number;
  highlight_count: number;
  badge_count: number;
  candidate: NativeAttentionCandidate | null;
  capabilities: NativeAttentionCapabilities;
}

export interface NativeAttentionCandidate {
  room_display_name: string;
  kind: RoomAttentionKind;
  unread_count: number;
  highlight_count: number;
}

export type RoomAttentionKind = "mention" | "dm" | "message";

export interface NativeAttentionCapabilities {
  notifications: NativeAttentionCapability;
  badge: NativeAttentionCapability;
  overlay_icon: NativeAttentionCapability;
  sound: NativeAttentionCapability;
  tray: NativeAttentionCapability;
  activation: NativeAttentionCapability;
}

export type NativeAttentionCapability = "available" | "unavailable" | "unknown";

export type NativeAttentionEvent = {
  kind: "summaryUpdated";
  summary: NativeAttentionSummary;
};

export interface JapaneseCatalogProfile {
  catalog_locale: string;
  complete: boolean;
  missing_message_ids: string[];
}

export type CjkTextPolicyEvent = {
  kind: "japaneseCatalogProfileChanged";
  profile: JapaneseCatalogProfile;
};

export type ThreadsListEvent =
  | {
      kind: "opened";
      request_id: RequestId;
      room_id: string;
      items: ThreadsListItem[];
      end_reached: boolean;
    }
  | {
      kind: "updated";
      request_id: RequestId;
      room_id: string;
      items: ThreadsListItem[];
      is_paginating: boolean;
      end_reached: boolean;
    }
  | {
      kind: "paginationCompleted";
      request_id: RequestId;
      room_id: string;
      items: ThreadsListItem[];
      end_reached: boolean;
    }
  | {
      kind: "failed";
      request_id: RequestId;
      room_id: string;
      failure_kind: OperationFailureKind;
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
  | { ReportOperationFailed: { kind: ReportFailureKind } }
  | { SearchFailed: { kind: string } }
  | "LocalEncryptionUnavailable"
  | "StoreUnavailable"
  | "ShutdownFailed";

export type ReportFailureKind =
  | "Forbidden"
  | "Network"
  | "InvalidUserId"
  | "InvalidRoomId"
  | "InvalidEventId"
  | "Sdk";

// ---------------------------------------------------------------------------
// CoreEvent envelope (the `koushi-desktop://event` payload shape produced by
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
  | { kind: "Activity"; event: ActivityEvent }
  | { kind: "LocalEncryption"; event: LocalEncryptionEvent }
  | { kind: "NativeAttention"; event: NativeAttentionEvent }
  | { kind: "CjkTextPolicy"; event: CjkTextPolicyEvent }
  | { kind: "ThreadsList"; event: ThreadsListEvent }
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
