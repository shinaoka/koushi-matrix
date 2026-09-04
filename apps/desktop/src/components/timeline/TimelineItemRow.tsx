import {
  Copy,
  Edit3,
  FileCode2,
  Forward,
  KeyRound,
  MessageCircle,
  MoreHorizontal,
  Pin,
  PinOff,
  RefreshCw,
  Reply,
  SmilePlus,
  Trash2,
  XCircle
} from "lucide-react";
import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useRef,
  useState,
  type FormEvent,
  type MouseEvent
} from "react";

import {
  ignoreComposerKeyAction,
  peopleFacingLabel,
  type MentionCandidate
} from "../../app/uiShared";
import {
  contextMenuItems,
  type ContextMenuItem
} from "../../domain/contextMenus";
import { getActiveLocale, t } from "../../i18n/messages";
import { useRecoverableImageSource } from "../avatarImage";
import { onMenuKeyDown } from "../ContextMenuSurface";
import { Tooltip } from "../Tooltip";

import {
  documentFromText,
  plainBodyFromDocument,
  trimDocument
} from "../../domain/composerDocument";
import type {
  MediaTransferProgress,
  ReactionSender,
  TimelineItem
} from "../../domain/coreEvents";
import { timelineItemDomId } from "../../domain/coreEvents";
import {
  openExternalHttpUrl,
  renderableThumbnailSourceUrl
} from "../../backend/linkMediaRuntime";
import { resolvedAvatar } from "../../domain/avatarThumbnails";
import { toExternalHttpUrl } from "../../domain/externalLinks";
import type { TimelineForwardDestination } from "../../domain/projectionTypes";
import type { TimelineDisplayRow } from "../../domain/timelineDisplayProjection";
import type {
  AvatarThumbnailState,
  ComposerDocument,
  DisplayDensity,
  LiveReadReceipt,
  PresenceKind,
  ResolveComposerKeyAction,
  ThreadOpenIntent,
  TimelineMediaDownloadState,
  TextRange,
  UserProfile
} from "../../domain/types";
import { Composer } from "../composer";
import { ImeSafeForm } from "../ImeTextControl";
import type { TimelineTransport } from "./TimelineTransport";
import { formatMessageTimestamp, MessageMeta } from "./MessageMeta";
import { formatReceiptDetails, ReceiptReaders } from "./ReceiptReaders";
import {
  TimelineMediaAttachment,
  type TimelineMediaViewerItem
} from "./TimelineMedia";
import {
  renderFormattedBody,
  renderPlainTextBody,
  type OpenMatrixTargetHandler
} from "./TimelineMessageBody";

export type TimelineThreadAttention = {
  rootEventId: string;
  notificationCount: number;
  highlightCount: number;
  liveEventMarkerCount: number;
};

/**
 * Row-level actions surfaced on timeline items. Matrix semantics stay
 * Rust-owned: the row reports event-backed intent plus Rust-projected reaction
 * ownership; reply targeting, reaction send/redact, edits, redaction, and
 * download all travel through typed core transport paths.
 */
export interface TimelineRowActionHandlers {
  onReply: (roomId: string, eventId: string) => void;
  onOpenThread: (
    roomId: string,
    rootEventId: string,
    intent: ThreadOpenIntent
  ) => void;
  onSendReaction: (roomId: string, eventId: string, reactionKey: string) => void;
  onRedactReaction: (
    roomId: string,
    eventId: string,
    reactionKey: string,
    reactionEventId: string
  ) => void;
  onEdit: (roomId: string, eventId: string, document: ComposerDocument) => void;
  onRedact: (roomId: string, eventId: string) => void;
  onPin: (roomId: string, eventId: string) => void;
  onUnpin: (roomId: string, eventId: string) => void;
  onDownloadMedia: (roomId: string, eventId: string) => void;
  onLoadMessageSource: (roomId: string, eventId: string) => void;
  onRequestRoomKey: (roomId: string, eventId: string) => void;
  onForwardMessage: (roomId: string, sourceEventId: string, destinationRoomId: string) => void;
  onLoadLinkPreviews: (roomId: string, eventId: string, pendingCount?: number) => void;
  onHideLinkPreview: (roomId: string, eventId: string) => void;
  onCopyText: (value: string) => void;
  onSetLocalUserAlias: (userId: string, alias: string | null) => void;
  onRetrySend: (roomId: string, transactionId: string) => void;
  onCancelSend: (roomId: string, transactionId: string) => void;
  /**
   * Opens a Matrix entity a message links to. Optional so surfaces that render
   * rows without in-app navigation keep matrix.to links as ordinary links
   * rather than swallowing the click.
   */
  onOpenMatrixTarget?: OpenMatrixTargetHandler;
  onOpenSenderProfile?: (roomId: string, userId: string) => void;
}

function reactionPickerBoundaryElement(anchor: Element): Element | null {
  return anchor.closest(".timeline-view") ?? anchor.closest(".main-pane");
}

const ignoreSendQueueAction = () => undefined;

const LazyEmojiPicker = lazy(() =>
  import("../EmojiPicker").then((module) => ({ default: module.EmojiPicker }))
);

export type TimelineAliasTarget = {
  userId: string;
  displayLabel: string;
  originalDisplayLabel: string;
};

export function ThreadRootStatusPlaceholder({
  row,
  state,
  showThreadSummary = true
}: {
  row: TimelineDisplayRow;
  state: "pending" | "failed";
  showThreadSummary?: boolean;
}) {
  const summary = row.item.thread_summary;
  const replyCount = summary?.reply_count ?? 0;
  return (
    <article
      className="message thread-root-projection-placeholder"
      data-item-id={row.row_id}
      data-row-id={row.row_id}
      data-content-event-id={row.content_event_id ?? undefined}
      data-activity-event-id={row.activity_event_id ?? undefined}
      data-event-id={row.activity_event_id ?? undefined}
      data-thread-root-projection-state={state}
    >
      <p className="timeline-thread-root-projection-status" role="status">
        {state === "pending"
          ? t("timeline.threadRootLoading")
          : t("timeline.threadRootUnavailable")}
      </p>
      {showThreadSummary ? (
        <span className="thread-reply-count">
          {replyCount === 1 ? "1 reply" : `${replyCount} replies`}
        </span>
      ) : null}
    </article>
  );
}

export function TimelineItemRow({
  item,
  rowId,
  contentEventId,
  activityEventId,
  contentTimestampMs,
  roomId,
  presentationContext = "room",
  codeBlockWrap = true,
  showThreadSummary = true,
  searchHighlights = [],
  onReply,
  onOpenThread = () => undefined,
  resolveComposerKeyAction = ignoreComposerKeyAction,
  recentEmojis = [],
  onRecentEmojisChange,
  mediaUploadProgress = null,
  onSendReaction,
  onRedactReaction,
  onEdit,
  onRedact,
  isPinned = false,
  isContinuation = false,
  isTarget = false,
  onPin = () => undefined,
  onUnpin = () => undefined,
  onDownloadMedia = () => undefined,
  onLoadMessageSource = () => undefined,
  onRequestRoomKey = () => undefined,
  onForwardMessage = () => undefined,
  autoLoadLinkPreviews = false,
  onLoadLinkPreviews = () => undefined,
  onHideLinkPreview = () => undefined,
  onCopyText = () => undefined,
  onOpenAliasDialog,
  onOpenMediaViewer = () => undefined,
  onSaveMediaFile,
  forwardDestinations = [],
  onRetrySend = ignoreSendQueueAction,
  onCancelSend = ignoreSendQueueAction,
  onOpenMatrixTarget,
  onOpenSenderProfile,
  onStartDirectMessage,
  density = "default",
  presence,
  profile,
  reactionSenderLabelsByUserId = {},
  mentionProfileUsers = {},
  mentionCandidates = [],
  mentionCandidatesLoading = false,
  onMentionQueryChange,
  receipts = [],
  receiptTotalCount = receipts.length,
  receiptOverflowCount = 0,
  currentUserId,
  ignoredUserIds = [],
  onOpenContextMenu,
  threadAttention = null,
  mediaDownload,
  keyRequestPending = false
}: {
  item: TimelineItem;
  /** Stable presentation identity used by DOM/virtualization rows. */
  rowId?: string;
  /** Root/content identity for every message action. */
  contentEventId?: string | null;
  /** Latest-activity identity for Room viewport observation. */
  activityEventId?: string | null;
  /** The content event timestamp, independent of presentation placement. */
  contentTimestampMs?: number | null;
  roomId: string;
  keyRequestPending?: boolean;
  presentationContext?: "room" | "thread" | "focused";
  codeBlockWrap?: boolean;
  showThreadSummary?: boolean;
  searchHighlights?: TextRange[];
  onReply: TimelineRowActionHandlers["onReply"];
  onOpenThread?: TimelineRowActionHandlers["onOpenThread"];
  resolveComposerKeyAction?: ResolveComposerKeyAction;
  recentEmojis?: string[];
  onRecentEmojisChange?: (emojis: string[]) => void | Promise<void>;
  mediaUploadProgress?: MediaTransferProgress | null;
  onSendReaction: TimelineRowActionHandlers["onSendReaction"];
  onRedactReaction: TimelineRowActionHandlers["onRedactReaction"];
  onEdit: TimelineRowActionHandlers["onEdit"];
  onRedact: TimelineRowActionHandlers["onRedact"];
  isPinned?: boolean;
  isContinuation?: boolean;
  isTarget?: boolean;
  onPin?: TimelineRowActionHandlers["onPin"];
  onUnpin?: TimelineRowActionHandlers["onUnpin"];
  onDownloadMedia?: TimelineRowActionHandlers["onDownloadMedia"];
  onLoadMessageSource?: TimelineRowActionHandlers["onLoadMessageSource"];
  onRequestRoomKey?: TimelineRowActionHandlers["onRequestRoomKey"];
  onForwardMessage?: TimelineRowActionHandlers["onForwardMessage"];
  autoLoadLinkPreviews?: boolean;
  onLoadLinkPreviews?: TimelineRowActionHandlers["onLoadLinkPreviews"];
  onHideLinkPreview?: TimelineRowActionHandlers["onHideLinkPreview"];
  onCopyText?: TimelineRowActionHandlers["onCopyText"];
  onOpenAliasDialog?: (target: TimelineAliasTarget) => void;
  onOpenMediaViewer?: (item: TimelineMediaViewerItem) => void;
  onSaveMediaFile?: TimelineTransport["saveMediaFile"];
  forwardDestinations?: readonly TimelineForwardDestination[];
  onRetrySend?: TimelineRowActionHandlers["onRetrySend"];
  onCancelSend?: TimelineRowActionHandlers["onCancelSend"];
  onOpenMatrixTarget?: TimelineRowActionHandlers["onOpenMatrixTarget"];
  onOpenSenderProfile?: TimelineRowActionHandlers["onOpenSenderProfile"];
  onStartDirectMessage?: (userId: string) => void;
  density?: DisplayDensity;
  presence?: PresenceKind;
  profile?: UserProfile;
  reactionSenderLabelsByUserId?: Readonly<Record<string, string>>;
  mentionProfileUsers?: Record<string, UserProfile>;
  mentionCandidates?: MentionCandidate[];
  mentionCandidatesLoading?: boolean;
  onMentionQueryChange?: (roomId: string, query: string | null) => void;
  receipts?: LiveReadReceipt[];
  receiptTotalCount?: number;
  receiptOverflowCount?: number;
  currentUserId?: string;
  ignoredUserIds?: string[];
  onOpenContextMenu?: (
    event: MouseEvent<HTMLElement>,
    target: {
      kind: "message";
      message: {
        sender: string;
        room_id: string;
        event_id: string;
        body: string;
        reply_count: number;
      };
    },
    items: ContextMenuItem[]
  ) => void;
  threadAttention?: TimelineThreadAttention | null;
  mediaDownload?: TimelineMediaDownloadState;
}) {
  const itemDomId = timelineItemDomId(item.id);
  const domId = rowId ?? itemDomId;
  const syntheticId = "Synthetic" in item.id ? item.id.Synthetic.synthetic_id : null;
  const dateDividerTimestampMs = syntheticDateDividerTimestampMs(syntheticId, item.timestamp_ms);
  if (dateDividerTimestampMs !== null) {
    return (
      <div className="read-marker timeline-date-divider" role="separator">
        <span>{formatDateDividerLabel(dateDividerTimestampMs)}</span>
      </div>
    );
  }
  if (syntheticId !== null) {
    return null;
  }
  const transactionId = "Transaction" in item.id ? item.id.Transaction.transaction_id : null;
  const itemEventId = "Event" in item.id ? item.id.Event.event_id : null;
  const eventId = contentEventId ?? itemEventId;
  const activityId = activityEventId ?? eventId;
  const isRedacted = item.is_redacted;
  const sendState = item.send_state ?? null;
  const sendStateKind = sendState?.kind ?? null;
  const messageKind = item.message_kind ?? "text";
  const [isEditing, setEditing] = useState(false);
  const [editDocument, setEditDocument] = useState(() => documentFromText(item.body ?? ""));
  const [isReactionPickerOpen, setReactionPickerOpen] = useState(false);
  const [isActionMenuOpen, setActionMenuOpen] = useState(false);
  const [isForwardMenuOpen, setForwardMenuOpen] = useState(false);
  const [actionMenuPlacement, setActionMenuPlacement] = useState<"above" | "below">("above");
  const [revealedSpoilers, setRevealedSpoilers] = useState<ReadonlySet<string>>(
    () => new Set()
  );
  const reactionTriggerRef = useRef<HTMLButtonElement>(null);
  const actionMenuControlRef = useRef<HTMLDivElement>(null);
  const actionMenuTriggerRef = useRef<HTMLButtonElement>(null);
  const firstActionMenuItemRef = useRef<HTMLButtonElement>(null);
  const requestedLinkPreviewsRef = useRef<Set<string>>(new Set());

  useEffect(() => {
    if (!autoLoadLinkPreviews) {
      return;
    }
    const pendingCount =
      item.link_previews?.filter((preview) => preview.state === "pending").length ?? 0;
    if (!eventId || pendingCount === 0) {
      return;
    }
    if (requestedLinkPreviewsRef.current.has(eventId)) {
      return;
    }
    requestedLinkPreviewsRef.current.add(eventId);
    onLoadLinkPreviews(roomId, eventId, pendingCount);
  }, [autoLoadLinkPreviews, eventId, item.link_previews, onLoadLinkPreviews, roomId]);

  useEffect(() => {
    if (!isActionMenuOpen) {
      return;
    }
    firstActionMenuItemRef.current?.focus();
  }, [isActionMenuOpen]);

  useEffect(() => {
    if (!isActionMenuOpen) {
      return;
    }
    const handlePointerDown = (event: PointerEvent) => {
      const control = actionMenuControlRef.current;
      if (!control || control.contains(event.target as Node)) {
        return;
      }
      setActionMenuOpen(false);
      setForwardMenuOpen(false);
      actionMenuTriggerRef.current?.focus();
    };
    document.addEventListener("pointerdown", handlePointerDown);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
    };
  }, [isActionMenuOpen]);

  // Outside-press dismissal is owned by EmojiPicker, which knows its own
  // floating panel: a row-scoped containment check would read the panel's emoji
  // buttons as outside presses now that the panel renders in the body layer.
  const closeReactionPicker = useCallback(() => {
    setReactionPickerOpen(false);
    reactionTriggerRef.current?.focus();
  }, []);

  const toggleReactionPicker = useCallback(() => {
    setActionMenuOpen(false);
    setForwardMenuOpen(false);
    setReactionPickerOpen((current) => !current);
  }, []);

  const closeActionMenu = useCallback(() => {
    setActionMenuOpen(false);
    setForwardMenuOpen(false);
    actionMenuTriggerRef.current?.focus();
  }, []);

  const revealSpoiler = useCallback((spoilerKey: string) => {
    setRevealedSpoilers((current) => {
      if (current.has(spoilerKey)) {
        return current;
      }
      const next = new Set(current);
      next.add(spoilerKey);
      return next;
    });
  }, []);

  const openEditForm = useCallback(() => {
    if (!eventId || isRedacted) {
      return;
    }
    setReactionPickerOpen(false);
    setActionMenuOpen(false);
    setForwardMenuOpen(false);
    setEditDocument(item.actions?.editable_document ?? documentFromText(item.body ?? ""));
    setEditing(true);
  }, [eventId, isRedacted, item.actions?.editable_document, item.body]);

  const closeEditForm = useCallback(() => {
    setEditing(false);
    setEditDocument(item.actions?.editable_document ?? documentFromText(item.body ?? ""));
    onMentionQueryChange?.(roomId, null);
  }, [item.actions?.editable_document, item.body, onMentionQueryChange, roomId]);

  const submitEditDocument = useCallback(
    (document: ComposerDocument) => {
      if (!eventId) return;
      const trimmedDocument = trimDocument(document);
      if (!plainBodyFromDocument(trimmedDocument)) return;
      onEdit(roomId, eventId, trimmedDocument);
      closeEditForm();
    },
    [closeEditForm, eventId, onEdit, roomId]
  );

  const submitEdit = useCallback(
    (event: FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      submitEditDocument(editDocument);
    },
    [editDocument, submitEditDocument]
  );

  const submitReaction = useCallback(
    (reactionKey: string) => {
      if (!eventId) {
        return;
      }
      const existingOwnReaction = item.reactions.find(
        (reaction) => reaction.key === reactionKey && reaction.reacted_by_me
      );
      if (existingOwnReaction) {
        if (existingOwnReaction.my_reaction_event_id) {
          onRedactReaction(
            roomId,
            eventId,
            reactionKey,
            existingOwnReaction.my_reaction_event_id
          );
        }
      } else {
        onSendReaction(roomId, eventId, reactionKey);
      }
      closeReactionPicker();
    },
    [closeReactionPicker, eventId, item.reactions, onRedactReaction, onSendReaction, roomId]
  );
  const submitReply = useCallback(() => {
    if (!eventId) {
      return;
    }
    onReply(roomId, eventId);
  }, [eventId, onReply, roomId]);
  const submitOpenThread = useCallback(() => {
    if (!eventId) {
      return;
    }
    const intent: ThreadOpenIntent =
      (item.thread_summary?.reply_count ?? 0) > 0
        ? "existingThread"
        : "newThreadDraft";
    onOpenThread(roomId, eventId, intent);
  }, [eventId, item.thread_summary?.reply_count, onOpenThread, roomId]);
  const submitRedaction = useCallback(() => {
    if (!eventId) {
      return;
    }
    onRedact(roomId, eventId);
  }, [eventId, onRedact, roomId]);
  const submitPin = useCallback(() => {
    if (!eventId) {
      return;
    }
    onPin(roomId, eventId);
  }, [eventId, onPin, roomId]);
  const submitUnpin = useCallback(() => {
    if (!eventId) {
      return;
    }
    onUnpin(roomId, eventId);
  }, [eventId, onUnpin, roomId]);
  const submitDownloadMedia = useCallback(() => {
    if (!eventId) {
      return;
    }
    onDownloadMedia(roomId, eventId);
  }, [eventId, onDownloadMedia, roomId]);
  const openActionMenu = useCallback(() => {
    const control = actionMenuControlRef.current;
    if (control) {
      const controlRect = control.getBoundingClientRect();
      const panelTop =
        control.closest<HTMLElement>(".main-pane")?.getBoundingClientRect().top ?? 0;
      const availableAbove = controlRect.top - panelTop;
      setActionMenuPlacement(availableAbove < 180 ? "below" : "above");
    }
    setReactionPickerOpen(false);
    setForwardMenuOpen(false);
    setActionMenuOpen((current) => !current);
  }, []);
  const copyMessageBody = useCallback(() => {
    if (!item.actions?.can_copy || item.body === null) {
      return;
    }
    onCopyText(item.body);
    closeActionMenu();
  }, [closeActionMenu, item.actions?.can_copy, item.body, onCopyText]);
  const copyPermalink = useCallback(() => {
    const permalink = item.actions?.permalink;
    if (!item.actions?.can_permalink || !permalink) {
      return;
    }
    onCopyText(permalink);
    closeActionMenu();
  }, [closeActionMenu, item.actions?.can_permalink, item.actions?.permalink, onCopyText]);
  const loadMessageSource = useCallback(() => {
    if (!eventId || !item.actions?.can_view_source) {
      return;
    }
    onLoadMessageSource(roomId, eventId);
    closeActionMenu();
  }, [closeActionMenu, eventId, item.actions?.can_view_source, onLoadMessageSource, roomId]);
  const requestRoomKey = useCallback(() => {
    if (!eventId || !item.unable_to_decrypt?.can_request_keys) {
      return;
    }
    onRequestRoomKey(roomId, eventId);
  }, [eventId, item.unable_to_decrypt?.can_request_keys, onRequestRoomKey, roomId]);
  const submitForward = useCallback(
    (destinationRoomId: string) => {
      if (!eventId || !item.actions?.can_forward) {
        return;
      }
      onForwardMessage(roomId, eventId, destinationRoomId);
      closeActionMenu();
    },
    [closeActionMenu, eventId, item.actions?.can_forward, onForwardMessage, roomId]
  );
  const submitRetrySend = useCallback(() => {
    if (!transactionId) {
      return;
    }
    onRetrySend(roomId, transactionId);
  }, [onRetrySend, roomId, transactionId]);
  const submitCancelSend = useCallback(() => {
    if (!transactionId) {
      return;
    }
    onCancelSend(roomId, transactionId);
  }, [onCancelSend, roomId, transactionId]);
  const canShowActionButtons = Boolean(eventId) && !isRedacted;
  // Koushi threads are linear: the thread composer already sends into the open
  // thread, so a thread-pane row offers no reply-composition affordance. Rich
  // replies received from other clients still render their quoted context.
  const canComposeReply = presentationContext !== "thread";
  const canShowReply = canShowActionButtons && item.actions?.can_reply === true && canComposeReply;
  const canShowReplyInThread = canShowReply && presentationContext === "room";
  const canCopyMessage = Boolean(eventId && item.actions?.can_copy && item.body !== null);
  const canCopyPermalink = Boolean(
    eventId && item.actions?.can_permalink && item.actions.permalink
  );
  const canViewSource = Boolean(eventId && item.actions?.can_view_source);
  const canRequestRoomKey = Boolean(eventId && item.unable_to_decrypt?.can_request_keys);
  const canForward = Boolean(eventId && item.actions?.can_forward);
  const canSetSenderAlias = Boolean(eventId && item.sender && onOpenAliasDialog);
  const canShowMessageActionMenu =
    canSetSenderAlias ||
    canCopyMessage ||
    canCopyPermalink ||
    canViewSource ||
    canForward;
  const canShowThreadSummary = Boolean(showThreadSummary && eventId && item.thread_summary);
  const canShowReactions = !isRedacted && !isEditing && item.reactions.length > 0;
  const senderAvatar = resolvedAvatar(item.sender_avatar, profile?.avatar);
  const avatarUrl = thumbnailSourceUrl(senderAvatar?.thumbnail);
  const {
    displaySourceUrl: displayAvatarUrl,
    onImageError: onAvatarImageError,
    onImageLoad: onAvatarImageLoad
  } = useRecoverableImageSource(avatarUrl);
  const showAvatarImage = Boolean(displayAvatarUrl);
  const senderDisplayLabel = peopleFacingLabel(item.sender_label);
  const senderProfileUserId =
    isContinuation && density === "compact" ? null : item.sender;
  const canStartDirectMessage = Boolean(
    !(isContinuation && density === "compact") &&
      item.sender &&
      currentUserId &&
      item.sender !== currentUserId &&
      onStartDirectMessage
  );
  const senderOriginalLabel =
    profile?.original_display_label.trim() || profile?.display_name?.trim() || "";
  const senderAliasTarget =
    item.sender && canSetSenderAlias
      ? {
          userId: item.sender,
          displayLabel: senderDisplayLabel,
          originalDisplayLabel: senderOriginalLabel
        }
      : null;
  const threadSummaryText = item.thread_summary
    ? formatThreadSummary(
        item.thread_summary.reply_count,
        item.thread_summary.latest_sender
          ? peopleFacingLabel(item.thread_summary.latest_sender_label)
          : null,
        item.thread_summary.latest_body_preview,
        item.thread_summary.latest_timestamp_ms
      )
    : "";
  const threadNotificationCount =
    eventId && threadAttention?.rootEventId === eventId
      ? threadAttention.notificationCount
      : 0;
  const threadNotificationsText =
    threadNotificationCount > 0
      ? t("timeline.threadNotificationCount", { count: threadNotificationCount })
      : "";
  const receiptDetails = formatReceiptDetails(receipts, receiptOverflowCount);
  const receiptLabel = t("timeline.readBy", { count: receiptTotalCount });
  const receiptAriaLabel =
    receiptDetails.length > 0 ? `${receiptLabel}: ${receiptDetails.join("; ")}` : receiptLabel;
  const receiptTitle = receiptDetails.join("\n");
  const spoilerState = { revealed: revealedSpoilers, reveal: revealSpoiler };
  const displayBody = localizedTimelineItemBody(item);
  const replyLabel = t("timeline.replyToMessage");
  const replyInThreadLabel = t("timeline.replyInThread");
  const messageBodyClassName = [
    "message-body",
    item.formatted ? "message-formatted-body" : null,
    messageKind === "emote" ? "message-emote" : null,
    messageKind === "notice" ? "message-notice" : null
  ]
    .filter(Boolean)
    .join(" ");
  const messageBodyContent = item.formatted
    ? renderFormattedBody(
        item.formatted,
        item.link_ranges ?? [],
        codeBlockWrap,
        onCopyText,
        searchHighlights,
        spoilerState,
        onOpenMatrixTarget
      )
    : renderPlainTextBody(
        displayBody,
        item.link_ranges ?? [],
        item.spoiler_spans,
        searchHighlights,
        mentionProfileUsers,
        spoilerState,
        onOpenMatrixTarget
      );
  const emotePrefix =
    messageKind === "emote" ? (
      <span className="message-emote-prefix" dir="auto">
        * <span className="message-emote-sender">{senderDisplayLabel}</span>
      </span>
    ) : null;
  const replyQuoteContent =
    !isRedacted && item.reply_quote ? (
      <div className="reply-quote" data-reply-state={item.reply_quote.state}>
        <div className="reply-quote-sender" dir="auto">
          {peopleFacingLabel(item.reply_quote.sender_label)}
        </div>
        <div className="reply-quote-body" dir="auto">
          {item.reply_quote.formatted
            ? renderFormattedBody(
                item.reply_quote.formatted,
                [],
                codeBlockWrap,
                onCopyText,
                [],
                spoilerState,
                onOpenMatrixTarget
              )
            : replyQuoteBody(item.reply_quote)}
        </div>
      </div>
    ) : null;
  const bodyContent = isRedacted ? (
    <div className="message-body message-redacted" dir="auto">
      {t("timeline.redactedMessage")}
    </div>
  ) : isEditing ? (
    <ImeSafeForm className="message-edit-form" onSubmit={submitEdit}>
      <Composer
        editorOnly
        surface="edit"
        ariaLabel={t("timeline.editBody")}
        canEdit
        composerMode={{ kind: "plain" }}
        document={editDocument}
        draftKey={`edit:${eventId ?? "no-event"}`}
        isSending={false}
        mentionCandidates={mentionCandidates}
        mentionCandidatesLoading={mentionCandidatesLoading}
        resolveComposerKeyAction={resolveComposerKeyAction}
        roomName=""
        onCancel={closeEditForm}
        onCancelReply={closeEditForm}
        onDocumentChange={setEditDocument}
        onMentionQueryChange={(query) => onMentionQueryChange?.(roomId, query)}
        onSend={submitEditDocument}
      />
      <div className="message-edit-actions">
        <button className="message-edit-button" type="submit">
          {t("timeline.saveEdit")}
        </button>
        <button
          className="message-edit-button"
          type="button"
          onClick={closeEditForm}
        >
          {t("timeline.cancelEdit")}
        </button>
      </div>
    </ImeSafeForm>
  ) : (
    <div
      className={messageBodyClassName}
      dir="auto"
      data-code-block-wrap={item.formatted && codeBlockWrap ? "true" : undefined}
    >
      {emotePrefix}
      {messageBodyContent}
    </div>
  );
  const mediaContent =
    !isRedacted && item.media ? (
      <TimelineMediaAttachment
        media={item.media}
        progress={mediaUploadProgress}
        downloadState={mediaDownload}
        canDownload={Boolean(eventId)}
        onDownload={submitDownloadMedia}
        onOpenMediaViewer={onOpenMediaViewer}
        onSaveMediaFile={onSaveMediaFile}
        viewerActions={{
          canForward,
          forwardDestinations,
          onForward: submitForward,
          canViewSource,
          onViewSource: loadMessageSource,
          canRedact: Boolean(canShowActionButtons && item.can_redact),
          onRedact: submitRedaction
        }}
      />
    ) : null;
  function handleContextMenu(event: MouseEvent<HTMLElement>) {
    if (!onOpenContextMenu || !eventId || !item.sender) {
      return;
    }
    const items = contextMenuItems({
      kind: "message",
      canManage: currentUserId === item.sender,
      canReply: canShowReply,
      hasThread: item.thread_summary != null && canComposeReply,
      senderUserId: item.sender,
      currentUserId: currentUserId ?? "",
      roomId,
      eventId,
      isIgnored: ignoredUserIds.includes(item.sender)
    });
    if (items.length === 0) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    onOpenContextMenu(event, {
      kind: "message",
      message: {
        sender: item.sender,
        room_id: roomId,
        event_id: eventId,
        body: item.body ?? "",
        reply_count: item.thread_summary?.reply_count ?? 0
      }
    }, items);
  }

  const avatar = (
    <>
      {showAvatarImage ? (
        <img
          src={displayAvatarUrl ?? undefined}
          onError={onAvatarImageError}
          onLoad={onAvatarImageLoad}
        />
      ) : (
        senderInitials(senderDisplayLabel || item.sender)
      )}
    </>
  );
  const avatarElement = canStartDirectMessage ? (
    <button
      className="avatar avatar-button"
      type="button"
      aria-label={t("room.messageMember", { name: senderDisplayLabel })}
      onClick={(event) => {
        event.stopPropagation();
        onStartDirectMessage?.(item.sender!);
      }}
    >
      {avatar}
    </button>
  ) : (
    <div className="avatar" aria-hidden="true">
      {avatar}
    </div>
  );

  return (
    <article
      className={`message${isTarget ? " pinned-target" : ""}${
        isContinuation ? " is-continuation" : ""
      }`}
      data-item-id={domId}
      data-row-id={domId}
      data-content-event-id={eventId ?? undefined}
      data-activity-event-id={activityId ?? undefined}
      data-send-state={sendStateKind ?? undefined}
      data-event-id={activityId ?? undefined}
      data-redacted={isRedacted || undefined}
      data-reply={item.in_reply_to_event_id ? "true" : undefined}
      data-message-kind={messageKind}
      onContextMenu={handleContextMenu}
    >
      {avatarElement}
      <div className="message-main">
        <div className="message-heading">
          <MessageMeta
            senderDisplayLabel={senderDisplayLabel}
            timestampMs={contentTimestampMs ?? item.timestamp_ms ?? null}
            isEdited={item.is_edited}
            isRedacted={isRedacted}
            sendStateKind={sendStateKind}
            presence={presence}
            onOpenSenderProfile={
              senderProfileUserId && onOpenSenderProfile
                ? () => onOpenSenderProfile(roomId, senderProfileUserId)
                : undefined
            }
          />
        </div>
        {replyQuoteContent}
        {mediaContent ? (
          <>
            {mediaContent}
            {bodyContent}
          </>
        ) : (
          bodyContent
        )}
        {!isRedacted && eventId && item.link_previews && item.link_previews.length > 0 ? (
          <div className="link-preview-cards">
            {item.link_previews.map((preview) => {
              const previewUrl = toExternalHttpUrl(preview.url);
              const previewPending =
                preview.state === "pending" || preview.state === "loading";
              return (
                <div
                  key={preview.url}
                  className="link-preview-card"
                  data-link-preview-state={preview.state}
                >
                  {previewPending ? (
                    <div className="link-preview-main link-preview-skeleton" aria-hidden="true">
                      <span className="link-preview-skeleton-image" />
                      <span className="link-preview-skeleton-text">
                        <span />
                        <span />
                        <span />
                      </span>
                    </div>
                  ) : (
                    <a
                      className="link-preview-main"
                      href={previewUrl || undefined}
                      target="_blank"
                      rel="noopener noreferrer"
                      onClick={(event) => {
                        event.preventDefault();
                        if (previewUrl) {
                          void openExternalHttpUrl(previewUrl);
                        }
                      }}
                    >
                      {preview.image?.thumbnail && thumbnailSourceUrl(preview.image.thumbnail) ? (
                        <img
                          src={thumbnailSourceUrl(preview.image.thumbnail) ?? undefined}
                          alt={""}
                          className="link-preview-image"
                        />
                      ) : (
                        <span className="link-preview-image-placeholder" aria-hidden="true" />
                      )}
                      <div className="link-preview-text">
                        {preview.title ? (
                          <div className="link-preview-title">{preview.title}</div>
                        ) : null}
                        {preview.description ? (
                          <div className="link-preview-description">{preview.description}</div>
                        ) : null}
                        <div className="link-preview-url">{preview.url}</div>
                      </div>
                    </a>
                  )}
                  <button
                    type="button"
                    className="link-preview-hide"
                    onClick={() => onHideLinkPreview(roomId, eventId)}
                    aria-label={t("timeline.linkPreviewHide")}
                  >
                    ×
                  </button>
                </div>
              );
            })}
          </div>
        ) : null}
        {transactionId && sendStateKind === "notSent" ? (
          <div className="message-send-actions">
            <button className="message-send-action" type="button" onClick={submitRetrySend}>
              <RefreshCw size={13} aria-hidden="true" />
              <span>{t("timeline.resendSend")}</span>
            </button>
            <button
              className="message-send-action danger"
              type="button"
              onClick={submitCancelSend}
            >
              <Trash2 size={13} aria-hidden="true" />
              <span>{t("timeline.deleteSend")}</span>
            </button>
          </div>
        ) : null}
        {transactionId && sendStateKind === "sending" ? (
          <div className="message-send-actions">
            <button className="message-send-action" type="button" onClick={submitCancelSend}>
              <XCircle size={13} aria-hidden="true" />
              <span>{t("timeline.cancelSend")}</span>
            </button>
          </div>
        ) : null}
        {canRequestRoomKey ? (
          <div className="message-send-actions">
            <button className="message-send-action" type="button" onClick={requestRoomKey}>
              <KeyRound size={13} aria-hidden="true" />
              <span>{t("timeline.requestRoomKey")}</span>
            </button>
          </div>
        ) : null}
        {item.unable_to_decrypt?.recovery_stage ? (
          <p className="profile-settings-hint">
            {recoveryStageText(t, item.unable_to_decrypt.recovery_stage)}
          </p>
        ) : null}
        {item.unable_to_decrypt?.recovery_guidance ? (
          <p className="profile-settings-hint error">
            {recoveryGuidanceText(t, item.unable_to_decrypt.recovery_guidance)}
          </p>
        ) : null}
        {keyRequestPending || item.request_state ? (
          <p
            className={
              (item.request_state?.stage === "withheld" ||
                item.request_state?.stage === "still_waiting") &&
              item.request_state
                ? "profile-settings-hint error"
                : item.request_state?.stage === "decryption_recovered"
                  ? "profile-settings-hint success"
                  : "profile-settings-hint"
            }
          >
            {keyRequestStateText(
              t,
              item.request_state?.stage ?? "sent",
              item.request_state?.withheldCode ?? null
            )}
          </p>
        ) : null}
        {threadNotificationCount > 0 ? (
          <button
            className="thread-summary-chip thread-new-replies-chip"
            type="button"
            aria-label={t("timeline.openThreadSummary", { summary: threadNotificationsText })}
            onClick={submitOpenThread}
          >
            <MessageCircle size={13} />
            <span>{threadNotificationsText}</span>
          </button>
        ) : null}
        {canShowThreadSummary ? (
          <button
            className="thread-summary-chip"
            type="button"
            aria-label={t("timeline.openThreadSummary", { summary: threadSummaryText })}
            onClick={submitOpenThread}
          >
            <MessageCircle size={13} />
            <span>{threadSummaryText}</span>
          </button>
        ) : null}
        {canShowReactions || receiptTotalCount > 0 ? (
          <div className="message-status-row">
            {canShowReactions ? (
              <div className="message-reactions">
                {item.reactions.map((reaction, index) => {
                  const ariaLabel = t("timeline.reactionSummary", {
                    key: reaction.key,
                    count: reaction.count
                  });
                  const reactionTooltip = formatReactionTooltip(
                    reaction.key,
                    reaction.count,
                    reaction.sender_preview,
                    reactionSenderLabelsByUserId
                  );
                  const pillKey = `${reaction.key}:${reaction.my_reaction_event_id ?? index}`;
                  if (!eventId) {
                    return (
                      <span
                        aria-label={ariaLabel}
                        className="reaction-pill"
                        data-reacted-by-me={reaction.reacted_by_me || undefined}
                        key={pillKey}
                      >
                        <span className="reaction-pill-key" dir="auto">
                          {reaction.key}
                        </span>
                        <span className="reaction-pill-count">{reaction.count}</span>
                        {reactionTooltip ? (
                          <span className="reaction-tooltip" role="tooltip" dir="auto">
                            {reactionTooltip}
                          </span>
                        ) : null}
                      </span>
                    );
                  }
                  return (
                    <button
                      aria-label={ariaLabel}
                      className="reaction-pill"
                      data-reacted-by-me={reaction.reacted_by_me || undefined}
                      key={pillKey}
                      type="button"
                      aria-pressed={reaction.reacted_by_me}
                      onClick={() => {
                        if (reaction.reacted_by_me) {
                          if (reaction.my_reaction_event_id) {
                            onRedactReaction(
                              roomId,
                              eventId,
                              reaction.key,
                              reaction.my_reaction_event_id
                            );
                          }
                        } else {
                          onSendReaction(roomId, eventId, reaction.key);
                        }
                      }}
                    >
                      <span className="reaction-pill-key" dir="auto">
                        {reaction.key}
                      </span>
                      <span className="reaction-pill-count">{reaction.count}</span>
                      {reactionTooltip ? (
                        <span className="reaction-tooltip" role="tooltip" dir="auto">
                          {reactionTooltip}
                        </span>
                      ) : null}
                    </button>
                  );
                })}
              </div>
            ) : null}
            {receiptTotalCount > 0 ? (
              <ReceiptReaders
                ariaLabel={receiptAriaLabel}
                details={receiptDetails}
                overflowCount={receiptOverflowCount}
                receipts={receipts}
                title={receiptTitle}
              />
            ) : null}
          </div>
        ) : null}
      </div>
      <div className="message-actions message-actions-floating">
        {!isEditing && canShowActionButtons && item.can_react ? (
          <div className="reaction-control">
            <button
              ref={reactionTriggerRef}
              className="message-action"
              type="button"
              aria-label={t("timeline.addReaction")}
              aria-expanded={isReactionPickerOpen}
              aria-haspopup="dialog"
              onClick={toggleReactionPicker}
            >
              <SmilePlus size={14} />
            </button>
            {isReactionPickerOpen ? (
              <Suspense fallback={null}>
                <LazyEmojiPicker
                  anchorRef={reactionTriggerRef}
                  align="end"
                  placement="below"
                  resolveBoundaryElement={reactionPickerBoundaryElement}
                  className="timeline-reaction-emoji-picker"
                  recentEmojis={recentEmojis}
                  onRecentEmojisChange={onRecentEmojisChange}
                  onSelect={submitReaction}
                  onClose={closeReactionPicker}
                />
              </Suspense>
            ) : null}
          </div>
        ) : null}
        {!isEditing && canShowReply ? (
          <Tooltip label={replyLabel}>
            {(tooltipProps) => (
              <button
                {...tooltipProps}
                className="message-action"
                type="button"
                aria-label={replyLabel}
                onClick={submitReply}
              >
                <Reply
                  size={14}
                  aria-hidden="true"
                  data-message-action-icon="reply"
                />
              </button>
            )}
          </Tooltip>
        ) : null}
        {!isEditing && canShowReplyInThread ? (
          <Tooltip label={replyInThreadLabel}>
            {(tooltipProps) => (
              <button
                {...tooltipProps}
                className="message-action"
                type="button"
                aria-label={replyInThreadLabel}
                onClick={submitOpenThread}
              >
                <MessageCircle
                  size={14}
                  aria-hidden="true"
                  data-message-action-icon="reply-in-thread"
                />
              </button>
            )}
          </Tooltip>
        ) : null}
        {!isEditing && canShowActionButtons && item.can_edit ? (
          <button
            className="message-action"
            type="button"
            aria-label={t("timeline.editMessage")}
            onClick={openEditForm}
          >
            <Edit3 size={14} />
          </button>
        ) : null}
        {!isEditing && canShowActionButtons ? (
          <button
            className="message-action"
            type="button"
            aria-label={isPinned ? t("timeline.unpinMessage") : t("timeline.pinMessage")}
            aria-pressed={isPinned}
            onClick={isPinned ? submitUnpin : submitPin}
          >
            {isPinned ? <PinOff size={14} /> : <Pin size={14} />}
          </button>
        ) : null}
        {!isEditing && canShowMessageActionMenu ? (
          <div className="message-action-menu-control" ref={actionMenuControlRef}>
            <button
              ref={actionMenuTriggerRef}
              className="message-action"
              type="button"
              aria-label={t("timeline.messageActions")}
              aria-expanded={isActionMenuOpen}
              aria-haspopup="menu"
              onClick={openActionMenu}
            >
              <MoreHorizontal size={14} />
            </button>
            {isActionMenuOpen ? (
              <div
                className={`message-action-menu is-${actionMenuPlacement}`}
                role="menu"
                aria-label={t("timeline.messageActions")}
                onKeyDown={(event) => {
                  if (event.key === "Escape") {
                    event.preventDefault();
                    closeActionMenu();
                    return;
                  }
                  onMenuKeyDown(event, event.currentTarget);
                }}
              >
                {senderAliasTarget ? (
                  <button
                    ref={firstActionMenuItemRef}
                    className="message-action-menu-item"
                    type="button"
                    role="menuitem"
                    onClick={() => {
                      onOpenAliasDialog?.(senderAliasTarget);
                      closeActionMenu();
                    }}
                  >
                    <Edit3 size={14} aria-hidden="true" />
                    <span>
                      {t(
                        aliasTargetIsActive(senderAliasTarget)
                          ? "room.editAliasForMember"
                          : "room.setAliasForMember",
                        { name: senderAliasTarget.displayLabel }
                      )}
                    </span>
                  </button>
                ) : null}
                {canCopyMessage ? (
                  <button
                    ref={!senderAliasTarget ? firstActionMenuItemRef : undefined}
                    className="message-action-menu-item"
                    type="button"
                    role="menuitem"
                    onClick={copyMessageBody}
                  >
                    <Copy size={14} aria-hidden="true" />
                    <span>{t("timeline.copyMessage")}</span>
                  </button>
                ) : null}
                {canCopyPermalink ? (
                  <button
                    ref={
                      !senderAliasTarget && !canCopyMessage
                        ? firstActionMenuItemRef
                        : undefined
                    }
                    className="message-action-menu-item"
                    type="button"
                    role="menuitem"
                    onClick={copyPermalink}
                  >
                    <span aria-hidden="true" />
                    <span>{t("timeline.copyPermalink")}</span>
                  </button>
                ) : null}
                {canViewSource ? (
                  <button
                    ref={
                      !senderAliasTarget && !canCopyMessage && !canCopyPermalink
                        ? firstActionMenuItemRef
                        : undefined
                    }
                    className="message-action-menu-item"
                    type="button"
                    role="menuitem"
                    onClick={loadMessageSource}
                  >
                    <FileCode2 size={14} aria-hidden="true" />
                    <span>{t("timeline.viewSource")}</span>
                  </button>
                ) : null}
                {canForward ? (
                  <div className="message-forward-menu-control">
                    <button
                      ref={
                        !senderAliasTarget &&
                        !canCopyMessage &&
                        !canCopyPermalink &&
                        !canViewSource
                          ? firstActionMenuItemRef
                          : undefined
                      }
                      className="message-action-menu-item"
                      type="button"
                      role="menuitem"
                      aria-haspopup="menu"
                      aria-expanded={isForwardMenuOpen}
                      onClick={() => setForwardMenuOpen((current) => !current)}
                    >
                      <Forward size={14} aria-hidden="true" />
                      <span>{t("timeline.forwardMessage")}</span>
                    </button>
                    {isForwardMenuOpen ? (
                      <div className="message-forward-menu" role="menu">
                        {forwardDestinations.map((destination) => (
                          <button
                            className="message-action-menu-item"
                            type="button"
                            role="menuitem"
                            key={destination.room_id}
                            onClick={() => submitForward(destination.room_id)}
                          >
                            <MessageCircle size={14} aria-hidden="true" />
                            <span dir="auto">{destination.display_name}</span>
                          </button>
                        ))}
                      </div>
                    ) : null}
                  </div>
                ) : null}
              </div>
            ) : null}
          </div>
        ) : null}
        {!isEditing && canShowActionButtons && item.can_redact ? (
          <button
            className="message-action"
            type="button"
            aria-label={t("timeline.redactMessage")}
            onClick={submitRedaction}
          >
            <Trash2 size={14} />
          </button>
        ) : null}
      </div>
    </article>
  );
}

export function aliasTargetIsActive(target: TimelineAliasTarget): boolean {
  const displayLabel = target.displayLabel.trim();
  const originalDisplayLabel = target.originalDisplayLabel.trim();
  return Boolean(displayLabel && originalDisplayLabel && displayLabel !== originalDisplayLabel);
}

function formatReactionTooltip(
  reactionKey: string,
  totalCount: number,
  senderPreview: readonly ReactionSender[],
  senderLabelsByUserId: Readonly<Record<string, string>> = {}
): string | null {
  if (totalCount <= 0) {
    return null;
  }
  const previewLabels = senderPreview.map((sender) =>
    peopleFacingLabel(sender.display_label?.trim() || senderLabelsByUserId[sender.user_id] || null)
  );
  const overflowCount = Math.max(0, totalCount - previewLabels.length);
  const labels =
    overflowCount > 0
      ? [...previewLabels, t("timeline.reactionSenderOverflow", { count: overflowCount })]
      : previewLabels;
  const names =
    labels.length > 0
      ? new Intl.ListFormat(getActiveLocale(), { style: "long", type: "conjunction" }).format(labels)
      : t("timeline.reactionSenderUnknown", { count: totalCount });
  return t("timeline.reactionTooltip", { names, key: reactionKey });
}

function syntheticDateDividerTimestampMs(
  syntheticId: string | null,
  timestampMs: number | null
): number | null {
  if (!syntheticId?.startsWith("date-divider-")) {
    return null;
  }
  if (timestampMs !== null) {
    return timestampMs;
  }
  const parsed = Number(syntheticId.slice("date-divider-".length));
  return Number.isFinite(parsed) ? parsed : null;
}

function formatDateDividerLabel(timestampMs: number): string {
  return new Intl.DateTimeFormat(getActiveLocale(), {
    weekday: "short",
    year: "numeric",
    month: "short",
    day: "numeric"
  }).format(new Date(timestampMs));
}

function thumbnailSourceUrl(thumbnail: AvatarThumbnailState | null | undefined): string | null {
  return thumbnail?.kind === "ready"
    ? renderableThumbnailSourceUrl(thumbnail.source_ref)
    : null;
}

function replyQuoteBody(quote: NonNullable<TimelineItem["reply_quote"]>): string {
  if (quote.body_preview) {
    return quote.body_preview;
  }
  if (quote.state === "redacted") {
    return t("timeline.redactedMessage");
  }
  if (quote.state === "missing") {
    return t("timeline.replyQuoteMissing");
  }
  if (quote.state === "unsupported") {
    return t("timeline.replyQuoteUnsupported");
  }
  return t("timeline.replyQuoteUnavailable");
}

function localizedTimelineItemBody(item: TimelineItem): string {
  const notice = item.notice_i18n;
  switch (notice?.key) {
    case "timeline.notice.roomCreate":
      return t("timeline.notice.roomCreate");
    case "timeline.notice.roomPowerLevels":
      return t("timeline.notice.roomPowerLevels");
    case "timeline.notice.roomGuestAccess":
      return t("timeline.notice.roomGuestAccess");
    case "timeline.notice.roomEncryption":
      return t("timeline.notice.roomEncryption");
    case "timeline.notice.spaceParent":
      return t("timeline.notice.spaceParent");
    case "timeline.notice.roomJoinRules":
      return t("timeline.notice.roomJoinRules");
    case "timeline.notice.roomHistoryVisibility":
      return t("timeline.notice.roomHistoryVisibility");
    case "timeline.notice.roomPinnedEvents":
      return t("timeline.notice.roomPinnedEvents");
    case "timeline.notice.roomNameSet":
      return t("timeline.notice.roomNameSet", {
        newName: notice.new_name ?? ""
      });
    case "timeline.notice.roomNameChanged":
      return t("timeline.notice.roomNameChanged", {
        oldName: notice.old_name ?? "",
        newName: notice.new_name ?? ""
      });
    case "timeline.notice.roomNameRemoved":
      return t("timeline.notice.roomNameRemoved");
    case "timeline.notice.roomNameChangedGeneric":
      return t("timeline.notice.roomNameChangedGeneric");
    default:
      return item.body ?? "";
  }
}

function senderInitials(sender: string | null): string {
  if (!sender) {
    return "?";
  }
  const ascii = sender.match(/[A-Za-z]/g);
  if (ascii?.length) {
    return ascii.slice(0, 2).join("").toUpperCase();
  }
  return sender.slice(0, 2);
}

function formatThreadSummary(
  replyCount: number,
  latestSender: string | null,
  latestPreview: string | null,
  latestTimestampMs: number | null
): string {
  const countText = t(
    replyCount === 1 ? "timeline.threadReplyCountOne" : "timeline.threadReplyCountMany",
    { count: replyCount }
  );
  let summary: string;
  if (latestSender && latestPreview) {
    summary = t("timeline.threadSummaryWithPreview", {
      count: countText,
      sender: latestSender,
      preview: latestPreview
    });
  } else if (latestPreview) {
    summary = t("timeline.threadSummaryWithBody", {
      count: countText,
      preview: latestPreview
    });
  } else if (latestSender) {
    summary = t("timeline.threadSummaryWithSender", {
      count: countText,
      sender: latestSender
    });
  } else {
    summary = countText;
  }
  const timestamp = formatMessageTimestamp(latestTimestampMs);
  return timestamp ? `${summary} · ${timestamp}` : summary;
}

function recoveryStageText(
  t: (key: import("../../i18n/messages").MessageId) => string,
  stage: string
): string {
  switch (stage) {
    case "checking_local":
      return t("timeline.recoveryCheckingLocal");
    case "checking_backup":
      return t("timeline.recoveryCheckingBackup");
    case "requesting_own_devices":
      return t("timeline.recoveryRequestingDevices");
    case "repairing_olm":
      return t("timeline.recoveryRepairingOlm");
    case "waiting_for_key":
      return t("timeline.recoveryWaitingForKey");
    case "key_received":
    case "retrying_decryption":
      return t("timeline.recoveryRetrying");
    case "recovered":
      return t("timeline.recoveryRecovered");
    case "temporarily_failed":
      return t("timeline.recoveryTemporarilyFailed");
    case "automatic_paths_exhausted":
      return t("timeline.recoveryExhausted");
    case "unrecoverable_no_known_holder":
      return t("timeline.recoveryUnrecoverable");
    default:
      return "";
  }
}

function recoveryGuidanceText(
  t: (key: import("../../i18n/messages").MessageId) => string,
  guidance: string
): string {
  switch (guidance) {
    case "another_own_device":
      return t("timeline.recoveryGuidanceAnotherDevice");
    case "backup_unavailable":
      return t("timeline.recoveryGuidanceBackup");
    case "sender_reshare_may_recover":
      return t("timeline.recoveryGuidanceSenderMessage");
    case "ask_sender_to_repost":
      return t("timeline.recoveryGuidanceRepost");
    default:
      return "";
  }
}

function keyRequestStateText(
  t: (key: import("../../i18n/messages").MessageId) => string,
  stage: string,
  withheldCode: string | null
): string {
  switch (stage) {
    case "sent":
    case "automatic":
      return t("timeline.keyRequestAwaiting");
    case "still_waiting":
      return t("timeline.keyRequestStillWaiting");
    case "withheld":
      return withheldCodeText(t, withheldCode);
    case "decryption_recovered":
      return t("timeline.keyRequestRecovered");
    case "send_failed":
      return t("timeline.keyRequestWithheld");
    default:
      return "";
  }
}

function withheldCodeText(
  t: (key: import("../../i18n/messages").MessageId) => string,
  code: string | null
): string {
  switch (code) {
    case "unavailable":
      return t("timeline.keyRequestUnavailable");
    case "unauthorised":
      return t("timeline.keyRequestUnauthorised");
    case "unverified":
      return t("timeline.keyRequestUnverified");
    case "blacklisted":
      return t("timeline.keyRequestBlacklisted");
    default:
      return t("timeline.keyRequestWithheld");
  }
}
