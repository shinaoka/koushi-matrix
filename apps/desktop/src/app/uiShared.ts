// Shared module-level items extracted from App.tsx.
// This module is PURE — MUST NOT import @tauri-apps/*.

import {
  type MouseEvent
} from "react";
import type {
  ComposerMode,
  ImageUploadCompressionMode,
  ImageUploadCompressionPolicy,
  MentionIntent,
  MentionTarget,
  OperationFailureKind,
  ResolveComposerKeyAction,
  RoomTags,
  StagedUploadItem,
  TimelineMediaGalleryItem,
  TimelineMessage,
  UserProfile
} from "../domain/types";
import type { MentionCandidate } from "../domain/projectionTypes";
import type { ContextMenuItem } from "../domain/contextMenus";
import { t } from "../i18n/messages";

export type { MentionCandidate } from "../domain/projectionTypes";

// ===== Types =====

export type ContextMenuTarget =
  | {
      kind: "message";
      message: Pick<TimelineMessage, "sender" | "room_id" | "event_id" | "body">;
    }
  | { kind: "room"; roomId: string }
  | { kind: "space"; spaceId: string }
  | { kind: "account" };

export type OpenContextMenu = (
  event: MouseEvent<HTMLElement>,
  target: ContextMenuTarget,
  items: ContextMenuItem[]
) => void;

export type ActiveContextMenu = {
  x: number;
  y: number;
  target: ContextMenuTarget;
  items: ContextMenuItem[];
};

export type PrimaryView = "timeline" | "invites" | "explore" | "activity";

/**
 * React-local view of the composer mode. Matrix semantics (the reply target)
 * stay Rust-owned; this is a presentational mapping of the WIRE value
 * `snapshot.state.ui.timeline.composer.mode` (externally tagged ComposerMode).
 */
export type ComposerModeProp =
  | { kind: "plain" }
  | { kind: "reply"; in_reply_to_event_id: string };

export type ImageUploadDimensionsPayload = {
  width: number;
  height: number;
};

export type ImageUploadVariantInfoPayload = {
  mime_type: string;
  byte_count: number;
  dimensions: ImageUploadDimensionsPayload | null;
};

export type ImageUploadVariantKindPayload = "Original" | "Compressed";

export type ImageUploadCompressionStatePayload = {
  mode: ImageUploadCompressionMode;
  policy: ImageUploadCompressionPolicy;
  original: ImageUploadVariantInfoPayload;
  selected: ImageUploadVariantInfoPayload;
  selected_variant: ImageUploadVariantKindPayload;
  skipped_small_image: boolean;
  metadata_stripped: boolean;
  thumbnail_refreshed: boolean;
};

export type UploadMediaThumbnailPayload = {
  mime_type: string;
  bytes: number[];
  width: number;
  height: number;
};

export type PreparedMediaUpload = {
  filename: string;
  mimeType: string;
  bytes: number[];
  imageDimensions?: ImageUploadDimensionsPayload;
  imageCompression?: ImageUploadCompressionStatePayload;
  thumbnail?: UploadMediaThumbnailPayload;
};

export type ImageCompressionVariant = {
  filename: string;
  mimeType: string;
  bytes: number[];
  byteCount: number;
  dimensions: ImageUploadDimensionsPayload;
  previewUrl: string;
  thumbnail: UploadMediaThumbnailPayload;
};

export type ImageCompressionPlan = {
  mode: ImageUploadCompressionMode;
  policy: ImageUploadCompressionPolicy;
  original: ImageCompressionVariant;
  compressed: ImageCompressionVariant;
  skippedSmallImage: boolean;
};

export type ImageCompressionDialogState = {
  plan: ImageCompressionPlan;
  resolve: (choice: ImageUploadVariantKindPayload | "cancel") => void;
};

export type SyncPresentation = {
  state: "running" | "starting" | "reconnecting" | "failed" | "stopped";
  label: string;
  detail: string | null;
  ariaLabel: string;
  restartable: boolean;
};

// ===== Constants =====

export const EMPTY_ROOM_TAGS: RoomTags = { favourite: null, low_priority: null };

export const EMPTY_MENTION_INTENT: MentionIntent = { targets: [] };

export const ICON_SIZE = {
  micro: 14,
  compact: 15,
  small: 16,
  input: 17,
  control: 18,
  panel: 19,
  rail: 20,
  large: 22,
  auth: 23,
  emptyState: 24
} as const;

// ===== Helpers =====

export const ignoreComposerKeyAction: ResolveComposerKeyAction = async () => "noop";

export function formatUploadBytes(byteCount: number): string {
  if (byteCount >= 1024 * 1024) {
    return `${(byteCount / (1024 * 1024)).toFixed(1)} MB`;
  }
  if (byteCount >= 1024) {
    return `${Math.max(1, Math.round(byteCount / 1024))} KB`;
  }
  return `${byteCount} B`;
}

export function formatUploadDimensions(dimensions: ImageUploadDimensionsPayload): string {
  return `${dimensions.width}x${dimensions.height}`;
}

export function captionBody(item: StagedUploadItem): string {
  return item.caption?.plain_body ?? "";
}

export function mediaGalleryItemLabel(item: TimelineMediaGalleryItem): string {
  return item.media.filename || item.event_id;
}

export function composerModeProp(mode: ComposerMode): ComposerModeProp {
  return mode === "Plain"
    ? { kind: "plain" }
    : { kind: "reply", in_reply_to_event_id: mode.Reply.in_reply_to_event_id };
}

export function syncStatePresentation(
  sync: import("../domain/types").DesktopSnapshot["state"]["domain"]["sync"]
): SyncPresentation {
  if (typeof sync === "string") {
    switch (sync) {
      case "starting":
        return {
          state: "starting",
          label: t("sync.starting"),
          detail: null,
          ariaLabel: t("sync.starting"),
          restartable: false
        };
      case "running":
        return {
          state: "running",
          label: t("sync.running"),
          detail: null,
          ariaLabel: t("sync.running"),
          restartable: false
        };
      case "stopped":
      default:
        return {
          state: "stopped",
          label: t("sync.stopped"),
          detail: null,
          ariaLabel: t("sync.stopped"),
          restartable: true
        };
    }
  }

  if ("reconnecting" in sync) {
    return {
      state: "reconnecting",
      label: t("sync.reconnecting"),
      detail: sync.reconnecting,
      ariaLabel: sync.reconnecting
        ? t("sync.reconnectingWithReason", { reason: sync.reconnecting })
        : t("sync.reconnecting"),
      restartable: true
    };
  }

  return {
    state: "failed",
    label: t("sync.failed"),
    detail: sync.failed,
    ariaLabel: sync.failed ? t("sync.failedWithReason", { reason: sync.failed }) : t("sync.failed"),
    restartable: true
  };
}

export function initials(value: string): string {
  const ascii = value.match(/[A-Za-z]/g);
  if (ascii?.length) {
    return ascii.slice(0, 2).join("").toUpperCase();
  }
  return value.slice(0, 2);
}

export function compactAvatarLabel(value: string): string {
  const normalized = value.trim().replace(/\s+/g, " ");
  return normalized || initials(value);
}

export function operationFailureLabel(kind: OperationFailureKind): string {
  switch (kind) {
    case "forbidden":
      return t("directory.failureForbidden");
    case "notFound":
      return t("directory.failureNotFound");
    case "network":
      return t("directory.failureNetwork");
    case "timeout":
      return t("directory.failureTimeout");
    case "invalid":
      return t("directory.failureInvalid");
    case "sdk":
      return t("directory.failureSdk");
  }
}

export function serverNameFromAlias(alias: string): string | null {
  const separatorIndex = alias.indexOf(":");
  if (separatorIndex < 0 || separatorIndex + 1 >= alias.length) {
    return null;
  }
  return alias.slice(separatorIndex + 1).trim() || null;
}

export function serverNameFromRoomId(roomId: string): string | null {
  if (!roomId.startsWith("!")) {
    return null;
  }
  return serverNameFromAlias(roomId);
}

export function formatTime(timestampMs: number): string {
  return new Intl.DateTimeFormat("ja-JP", {
    hour: "2-digit",
    minute: "2-digit"
  }).format(new Date(timestampMs));
}

export function formatScheduledSendTime(timestampMs: number): string {
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit"
  }).format(new Date(timestampMs));
}

export function defaultScheduleDateTimeValue(): string {
  return datetimeLocalValueFromTimestamp(Date.now() + 10 * 60 * 1000);
}

export function datetimeLocalValueFromTimestamp(timestampMs: number): string {
  const date = new Date(timestampMs);
  const pad = (value: number) => String(value).padStart(2, "0");
  return [
    date.getFullYear(),
    "-",
    pad(date.getMonth() + 1),
    "-",
    pad(date.getDate()),
    "T",
    pad(date.getHours()),
    ":",
    pad(date.getMinutes())
  ].join("");
}

export function scheduledSendTimestampFromInput(value: string): number | null {
  if (!value.trim()) {
    return null;
  }
  const timestampMs = new Date(`${value}:00`).getTime();
  return Number.isFinite(timestampMs) ? timestampMs : null;
}

export function scheduledSendCapabilityLabel(capability: import("../domain/types").ScheduledSendCapability): string {
  switch (capability) {
    case "serverDelayedEvents":
      return t("scheduled.serverDelayedEvents");
    case "localFallback":
      return t("scheduled.localFallback");
    case "unknown":
      return t("scheduled.unknownCapability");
  }
}

export function pinnedEventsForRoom(
  snapshot: import("../domain/types").DesktopSnapshot,
  roomId: string | null | undefined
): import("../domain/types").DesktopSnapshot["state"]["domain"]["room_interactions"][string]["pinned_events"] {
  return roomId ? snapshot.state.domain.room_interactions[roomId]?.pinned_events ?? [] : [];
}

export function forwardDestinationsFromSnapshot(
  snapshot: import("../domain/types").DesktopSnapshot
): import("../domain/projectionTypes").TimelineForwardDestination[] {
  return snapshot.state.domain.rooms.map((room) => ({
    room_id: room.room_id,
    display_name: room.display_label
  }));
}

export function mentionCandidatesFromSnapshot(
  snapshot: import("../domain/types").DesktopSnapshot
): MentionCandidate[] {
  return Object.values(snapshot.state.domain.profile.users)
    .map((profile) => {
      const label = mentionLabel(profile);
      const target: MentionTarget = {
        kind: "user",
        user_id: profile.user_id,
        display_label: label
      };
      return {
        key: profile.user_id,
        label,
        searchText: (
          profile.mention_search_terms.length
            ? profile.mention_search_terms.join(" ")
            : `${label} ${profile.user_id}`
        ).toLowerCase(),
        target
      };
    })
    .sort(
      (a, b) =>
        a.label.localeCompare(b.label, undefined, { sensitivity: "base" }) ||
        a.key.localeCompare(b.key)
    );
}

export function mentionLabel(profile: UserProfile): string {
  return profile.display_label.trim() || profile.user_id;
}

export function activeMentionQuery(value: string): { start: number; end: number; query: string } | null {
  const match = /(^|\s)@([^\s@]*)$/u.exec(value);
  if (!match) {
    return null;
  }
  const query = match[2] ?? "";
  return {
    start: value.length - query.length - 1,
    end: value.length,
    query
  };
}

export function appendMentionTarget(intent: MentionIntent, target: MentionTarget): MentionIntent {
  const targetKey = mentionTargetKey(target);
  if (intent.targets.some((candidate) => mentionTargetKey(candidate) === targetKey)) {
    return intent;
  }
  return { targets: [...intent.targets, target] };
}

export function mentionTargetKey(target: MentionTarget): string {
  switch (target.kind) {
    case "user":
      return `user:${target.user_id}`;
    case "room":
      return `room:${target.room_id}`;
    case "roomMention":
      return "roomMention";
  }
}

export function mentionDraftToken(target: MentionTarget): string {
  return `@${target.display_label}`;
}

export function mentionPillLabel(target: MentionTarget): string {
  return mentionDraftToken(target);
}

export function pruneMentionIntentForDraft(intent: MentionIntent, draft: string): MentionIntent {
  const targets = intent.targets.filter((target) => draft.includes(mentionDraftToken(target)));
  return targets.length === intent.targets.length ? intent : { targets };
}

export function threadReplyToTimelineMessage(
  reply: import("../domain/types").ThreadMessage
): import("../domain/types").TimelineMessage {
  return {
    room_id: reply.room_id,
    event_id: reply.event_id,
    sender: reply.sender,
    timestamp_ms: reply.timestamp_ms,
    body: reply.body,
    attachment_filename: null,
    reply_count: 0
  };
}

export function currentSavedSession(
  snapshot: import("../domain/types").DesktopSnapshot
): import("../domain/types").SavedSessionInfo | null {
  const session = snapshot.state.domain.session;
  if (!session.homeserver || !session.user_id || !session.device_id) {
    return null;
  }
  return {
    homeserver: session.homeserver,
    user_id: session.user_id,
    device_id: session.device_id
  };
}

export function shortcutLabelProfileFromLocaleProfile(
  profile: import("../domain/types").LocaleDisplayProfile
): import("../domain/shortcuts").ShortcutLabelProfile {
  return {
    platform: profile.platform,
    modLabel: profile.modifier_labels.primary
  };
}
