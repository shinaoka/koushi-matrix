import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent
} from "react";
import {
  ArrowDown,
  Bell,
  Check,
  Clock3,
  Compass,
  Image as ImageIcon,
  MessageCircle,
  MoreHorizontal,
  Search,
  Users,
  X
} from "lucide-react";
import { t } from "../i18n/messages";
import type {
  ActivityMarkReadTarget,
  ActivityRow,
  ActivityState,
  ActivityStream,
  ActivityTab,
  DesktopSnapshot,
  DirectoryRoomSummary,
  MentionIntent,
  ResolveComposerKeyAction,
  SearchResult,
  StagedUploadCompressionChoice
} from "../domain/types";
import { focusedTimelineKey, roomTimelineKey } from "../domain/coreEvents";
import {
  ICON_SIZE,
  initials,
  operationFailureLabel,
  type ComposerModeProp,
  type OpenContextMenu
} from "../app/uiShared";
import {
  selectForwardDestinations,
  selectMentionCandidates,
  useAppStore
} from "../domain/appStore";
import {
  TimelineView,
  type TimelineDiagnosticLogEntry,
  type TimelineDiagnostics,
  type TimelineRowActionHandlers,
  type TimelineTransport
} from "./TimelineView";
import { EntityAvatar } from "./Shell";
import {
  MessageArticle,
  RoomMediaGallery,
  MediaViewer,
  ScheduledMessagesList,
  PinnedEventsList,
  SearchResults
} from "./mediaLists";
import { Composer } from "./composer";
import { UploadStagingDialog } from "./dialogs";

const EMPTY_PINNED_EVENTS: DesktopSnapshot["state"]["domain"]["room_interactions"][string]["pinned_events"] = [];

function activityStream(activity: Extract<ActivityState, { kind: "open" }>, tab: ActivityTab): ActivityStream {
  return tab === "recent" ? activity.recent : activity.unread;
}

function activityTabLabel(tab: ActivityTab): string {
  return tab === "recent" ? t("activity.recent") : t("activity.unread");
}

function activityTimestamp(timestampMs: number): string {
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short"
  }).format(new Date(timestampMs));
}

function useStableEvent<T extends (...args: any[]) => unknown>(handler: T): T {
  const handlerRef = useRef(handler);

  useEffect(() => {
    handlerRef.current = handler;
  }, [handler]);

  return useCallback(((...args: any[]) => handlerRef.current(...args)) as T, []);
}

export function ActivityPane({
  activity,
  onClose,
  onLoadMore,
  onMarkRead,
  onOpenRow,
  onSetTab
}: {
  activity: ActivityState;
  onClose: () => void;
  onLoadMore: (tab: ActivityTab, cursor: string | null) => void;
  onMarkRead: (target: ActivityMarkReadTarget) => void;
  onOpenRow: (row: ActivityRow) => void;
  onSetTab: (tab: ActivityTab) => void;
}) {
  const activeTab =
    activity.kind === "open" ? activity.active_tab : activity.kind === "opening" ? activity.tab : "recent";
  const stream = activity.kind === "open" ? activityStream(activity, activeTab) : null;
  const rows = stream?.rows ?? [];
  const markReadState = activity.kind === "open" ? activity.mark_read : { kind: "idle" as const };
  const markAllPending =
    markReadState.kind === "pending" && markReadState.target.kind === "all";
  const markRoomPending = (row: ActivityRow) =>
    markReadState.kind === "pending" &&
    markReadState.target.kind === "room" &&
    markReadState.target.room_id === row.room_id;

  return (
    <main className="main-pane activity-pane" aria-labelledby="activity-title">
      <header className="channel-header">
        <div className="channel-title">
          <Clock3 size={ICON_SIZE.large} />
          <h1 id="activity-title">{t("workspace.activity")}</h1>
        </div>
        <div className="activity-actions">
          {activity.kind === "open" && activeTab === "unread" && rows.length > 0 ? (
            <button
              className="dialog-button secondary"
              type="button"
              disabled={markAllPending}
              onClick={() => onMarkRead({ kind: "all" })}
            >
              <Check size={ICON_SIZE.small} />
              <span>{t("activity.markAllRead")}</span>
            </button>
          ) : null}
          <button
            className="icon-button"
            type="button"
            aria-label={t("action.close", { title: t("workspace.activity") })}
            onClick={onClose}
          >
            <X size={ICON_SIZE.control} />
          </button>
        </div>
      </header>
      <div className="tabs" role="tablist" aria-label={t("activity.tabs")}>
        {(["recent", "unread"] as ActivityTab[]).map((tab) => (
          <button
            className={`tab ${activeTab === tab ? "is-active" : ""}`}
            role="tab"
            aria-selected={activeTab === tab}
            type="button"
            key={tab}
            disabled={activity.kind !== "open"}
            onClick={() => onSetTab(tab)}
          >
            {activityTabLabel(tab)}
          </button>
        ))}
      </div>
      {markReadState.kind === "failed" ? (
        <p className="activity-status" role="alert">
          {t("activity.markReadFailed")}
        </p>
      ) : null}
      <section className="activity-scroll" aria-label={activityTabLabel(activeTab)}>
        {activity.kind === "opening" ? (
          <div className="activity-empty">
            <Clock3 size={ICON_SIZE.emptyState} />
            <span>{t("activity.loading")}</span>
          </div>
        ) : rows.length === 0 ? (
          <div className="activity-empty">
            <Clock3 size={ICON_SIZE.emptyState} />
            <span>
              {activeTab === "recent" ? t("activity.noRecent") : t("activity.noUnread")}
            </span>
          </div>
        ) : (
          <ol className="activity-list">
            {rows.map((row) => {
              const isPlaceholder = row.kind === "roomUnread";
              return (
                <li
                  className={`activity-row ${row.unread ? "is-unread" : ""} ${
                    row.highlight ? "is-highlight" : ""
                  }`}
                  data-event-id={row.event_id ?? undefined}
                  data-room-id={row.room_id}
                  data-kind={row.kind}
                  key={`${row.room_id}:${isPlaceholder ? "roomUnread" : row.event_id}`}
                >
                  {isPlaceholder ? (
                    <button
                      className="activity-row-open"
                      type="button"
                      aria-label={t("activity.openItem", { room: row.room_label })}
                      onClick={() => onOpenRow(row)}
                    >
                      <EntityAvatar
                        avatar={null}
                        className="activity-row-avatar is-room"
                        fallback={initials(row.room_label)}
                      />
                      <span className="activity-row-body">
                        <span className="activity-row-topline">
                          <strong dir="auto">{row.room_label}</strong>
                        </span>
                        <span className="activity-row-meta">
                          {row.unread ? <span>{t("activity.unreadBadge")}</span> : null}
                          {row.highlight ? <span>{t("activity.highlightBadge")}</span> : null}
                        </span>
                      </span>
                    </button>
                  ) : (
                    <button
                      className="activity-row-open"
                      type="button"
                      aria-label={t("activity.openItem", { room: row.room_label })}
                      onClick={() => onOpenRow(row)}
                    >
                      <EntityAvatar
                        avatar={row.sender_avatar}
                        className="activity-row-avatar is-user"
                        fallback={initials(row.sender_label ?? row.room_label)}
                      />
                      <span className="activity-row-body">
                        <span className="activity-row-topline">
                          <strong dir="auto">
                            {row.sender_label ?? t("timeline.replyQuoteUnknownSender")}
                          </strong>
                          <time dateTime={new Date(row.timestamp_ms).toISOString()}>
                            {activityTimestamp(row.timestamp_ms)}
                          </time>
                        </span>
                        <span className="activity-row-context" dir="auto">
                          {row.context_label || row.room_label}
                        </span>
                        <span className="activity-row-preview" dir="auto">
                          {row.preview ?? t("activity.noPreview")}
                        </span>
                      </span>
                      <span className="activity-row-badges">
                        {row.unread ? <span>{t("activity.unreadBadge")}</span> : null}
                        {row.highlight ? <span>{t("activity.highlightBadge")}</span> : null}
                      </span>
                    </button>
                  )}
                  {activeTab === "unread" && !isPlaceholder ? (
                    <button
                      className="activity-row-action"
                      type="button"
                      aria-label={t("activity.markRoomRead")}
                      disabled={markRoomPending(row)}
                      onClick={() =>
                        onMarkRead({
                          kind: "room",
                          room_id: row.room_id,
                          up_to_event_id: row.event_id
                        })
                      }
                    >
                      <Check size={ICON_SIZE.small} />
                    </button>
                  ) : null}
                </li>
              );
            })}
          </ol>
        )}
      </section>
      {stream?.next_batch ? (
        <div className="activity-load-more">
          <button
            className="load-more-button"
            type="button"
            onClick={() => onLoadMore(activeTab, stream.next_batch)}
          >
            {t("activity.loadMore")}
          </button>
        </div>
      ) : null}
    </main>
  );
}

export function ExplorePane({
  isBusy,
  queryDraft,
  snapshot,
  onJoinRoom,
  onQueryChange,
  onSearch
}: {
  isBusy: boolean;
  queryDraft: string;
  snapshot: DesktopSnapshot;
  onJoinRoom: (room: DirectoryRoomSummary) => void;
  onQueryChange: (value: string) => void;
  onSearch: () => void;
}) {
  const queryState = snapshot.state.domain.directory.query;
  const joinState = snapshot.state.domain.directory.join;
  const rooms = queryState.kind === "results" ? queryState.rooms : [];
  const searchDisabled = isBusy || queryState.kind === "querying";

  function submitSearch(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    onSearch();
  }

  return (
    <main className="main-pane explore-pane" aria-labelledby="explore-title">
      <header className="channel-header">
        <div className="channel-title">
          <Compass size={ICON_SIZE.large} />
          <h1 id="explore-title">{t("workspace.explore")}</h1>
        </div>
      </header>
      <form className="directory-search" onSubmit={submitSearch}>
        <label className="directory-search-field">
          <span>{t("directory.searchPublicRooms")}</span>
          <input
            type="search"
            value={queryDraft}
            aria-label={t("directory.searchPublicRooms")}
            placeholder={t("directory.searchPlaceholder")}
            onChange={(event) => onQueryChange(event.currentTarget.value)}
          />
        </label>
        <button
          className="dialog-button is-primary"
          type="submit"
          aria-label={t("directory.searchPublicRooms")}
          disabled={searchDisabled}
        >
          <Search size={ICON_SIZE.small} />
          <span>
            {queryState.kind === "querying"
              ? t("directory.searching")
              : t("directory.search")}
          </span>
        </button>
      </form>
      {queryState.kind === "failed" ? (
        <div className="directory-status" role="status">
          {t("directory.searchFailed", {
            reason: operationFailureLabel(queryState.failureKind)
          })}
        </div>
      ) : null}
      <section className="directory-results" aria-label={t("directory.results")}>
        {queryState.kind === "querying" ? (
          <div className="empty-results" role="status">
            {t("directory.searching")}
          </div>
        ) : rooms.length ? (
          rooms.map((room) => {
            const alias = room.canonical_alias?.trim() || null;
            const joiningThisRoom =
              joinState.kind === "joining" && joinState.alias === alias;
            const joinFailed =
              joinState.kind === "failed" && joinState.alias === alias ? joinState : null;
            const canJoin = Boolean(alias) && !joiningThisRoom && !isBusy;
            return (
              <article className="directory-result" key={room.room_id}>
                <div className="directory-result-avatar" aria-hidden="true">
                  <span dir="auto">{initials(room.name)}</span>
                </div>
                <div className="directory-result-main">
                  <h2 dir="auto">{room.name}</h2>
                  <p dir="auto">
                    {room.topic?.trim() || alias || t("directory.noAlias")}
                  </p>
                  <div className="directory-result-meta">
                    <span>
                      {t("directory.memberCount", {
                        count: new Intl.NumberFormat().format(room.joined_members)
                      })}
                    </span>
                    {room.world_readable ? <span>{t("directory.worldReadable")}</span> : null}
                    {room.guest_can_join ? <span>{t("directory.guestCanJoin")}</span> : null}
                  </div>
                  {joinFailed ? (
                    <div className="directory-status" role="status">
                      {t("directory.joinFailed", {
                        reason: operationFailureLabel(joinFailed.failureKind)
                      })}
                    </div>
                  ) : null}
                </div>
                <button
                  className="dialog-button is-primary directory-join-button"
                  type="button"
                  aria-label={t("directory.joinRoom", { name: room.name })}
                  disabled={!canJoin}
                  onClick={() => onJoinRoom(room)}
                >
                  {joiningThisRoom ? t("directory.joining") : t("directory.join")}
                </button>
              </article>
            );
          })
        ) : (
          <div className="empty-results" role="status">
            {t("directory.noResults")}
          </div>
        )}
      </section>
    </main>
  );
}

export function InvitesPane({
  isBusy,
  snapshot,
  onAcceptInvite,
  onDeclineInvite,
  onNewDm
}: {
  isBusy: boolean;
  snapshot: DesktopSnapshot;
  onAcceptInvite: (roomId: string) => void;
  onDeclineInvite: (roomId: string) => void;
  onNewDm: () => void;
}) {
  const invites = snapshot.state.domain.invites;
  const [selectedInviteId, setSelectedInviteId] = useState<string | null>(null);
  const selectedInvite =
    invites.find((invite) => invite.room_id === selectedInviteId) ?? invites[0] ?? null;

  return (
    <main className="main-pane invites-pane" aria-labelledby="invites-title">
      <header className="channel-header">
        <div className="channel-title">
          <Bell size={ICON_SIZE.large} />
          <h1 id="invites-title">{t("workspace.invites")}</h1>
        </div>
        <div className="channel-actions">
          <button
            className="member-pill"
            type="button"
            aria-label={t("workspace.newDm")}
            onClick={onNewDm}
          >
            <MessageCircle size={ICON_SIZE.small} />
            <span>{t("workspace.newDm")}</span>
          </button>
        </div>
      </header>
      <nav className="tabs" aria-label={t("invite.tabs")}>
        <button className="tab is-active" type="button">
          {t("invite.pendingInvites")}
        </button>
      </nav>
      <section className="invites-layout" aria-label={t("invite.pendingInvites")}>
        <div className="invite-list">
          {invites.length ? (
            invites.map((invite) => (
              <button
                className={`invite-row ${invite.room_id === selectedInvite?.room_id ? "is-active" : ""}`}
                key={invite.room_id}
                type="button"
                aria-label={invite.display_name}
                onClick={() => setSelectedInviteId(invite.room_id)}
              >
                <EntityAvatar
                  avatar={invite.avatar}
                  className={`invite-row-icon ${invite.is_dm ? "is-user" : "is-room"}`}
                  fallback={initials(invite.display_name)}
                />
                <span className="invite-row-main">
                  <strong dir="auto">{invite.display_name}</strong>
                  <small dir="auto">
                    {invite.inviter_display_name ?? t("invite.unknownInviter")}
                  </small>
                </span>
              </button>
            ))
          ) : (
            <div className="empty-results" role="status">
              {t("invite.noPending")}
            </div>
          )}
        </div>
        <section className="invite-preview" aria-label={t("invite.preview")}>
          {selectedInvite ? (
            <>
              <div className="invite-preview-heading">
                <EntityAvatar
                  avatar={selectedInvite.avatar}
                  className={`invite-preview-icon ${selectedInvite.is_dm ? "is-user" : "is-room"}`}
                  fallback={initials(selectedInvite.display_name)}
                />
                <div>
                  <h2 dir="auto">{selectedInvite.display_name}</h2>
                  <p dir="auto">
                    {selectedInvite.inviter_display_name
                      ? t("invite.fromInviter", {
                          inviter: selectedInvite.inviter_display_name
                        })
                      : t("invite.unknownInviter")}
                  </p>
                </div>
              </div>
              <div className="settings-summary-grid" aria-label={t("invite.summary")}>
                <SummaryTile
                  label={t("room.type")}
                  value={
                    selectedInvite.is_dm
                      ? t("room.directMessage")
                      : t("search.scopeRoom")
                  }
                />
                <SummaryTile
                  label={t("invite.topic")}
                  value={selectedInvite.topic ?? t("invite.noTopic")}
                />
              </div>
              <div className="invite-actions">
                <button
                  className="dialog-button"
                  type="button"
                  aria-label={t("invite.decline")}
                  disabled={isBusy}
                  onClick={() => onDeclineInvite(selectedInvite.room_id)}
                >
                  <X size={ICON_SIZE.small} />
                  <span>{t("invite.decline")}</span>
                </button>
                <button
                  className="dialog-button is-primary"
                  type="button"
                  aria-label={t("invite.accept")}
                  disabled={isBusy}
                  onClick={() => onAcceptInvite(selectedInvite.room_id)}
                >
                  <Check size={ICON_SIZE.small} />
                  <span>{t("invite.accept")}</span>
                </button>
              </div>
            </>
          ) : (
            <div className="invite-empty-preview">
              <Bell size={ICON_SIZE.emptyState} />
              <span>{t("invite.noPending")}</span>
            </div>
          )}
        </section>
      </section>
    </main>
  );
}

export function SummaryTile({ label, value }: { label: string; value: string }) {
  return (
    <div className="settings-summary-tile">
      <span>{label}</span>
      <strong dir="auto">{value}</strong>
    </div>
  );
}

export function TimelinePane({
  activeRoomName,
  composerDraft,
  composerMode,
  mentionIntent,
  resolveComposerKeyAction,
  searchQuery,
  searchResults,
  showSearchResults,
  snapshot,
  timelineTransport,
  onCancelReply,
  onCancelScheduledSend,
  onAttachFiles,
  onClearUploadStaging,
  onUpdateStagedUploadCaption,
  onUpdateStagedUploadCompression,
  onComposerDraftChange,
  onMentionIntentChange,
  onEditMessage,
  onOpenContextMenu,
  onOpenThread,
  onRedactMessage,
  onReply,
  onRescheduleScheduledSend,
  onResultSelect,
  onScheduleSend,
  onSendText,
  onSetLocalUserAlias,
  onUnpinPinnedEvent,
  onOpenPeople,
  onOpenThreads,
  onToggleRoomInfo,
  onReturnToLive,
  onTimelineDiagnosticsChange,
  onTimelineDiagnosticLogEntry
}: {
  activeRoomName: string;
  composerDraft: string;
  composerMode: ComposerModeProp;
  mentionIntent: MentionIntent;
  resolveComposerKeyAction: ResolveComposerKeyAction;
  searchQuery: string;
  searchResults: SearchResult[];
  showSearchResults: boolean;
  snapshot: DesktopSnapshot;
  timelineTransport: TimelineTransport | null;
  onCancelReply: () => void;
  onCancelScheduledSend: (scheduledId: string) => void;
  onAttachFiles: (files: File[]) => void | Promise<void>;
  onClearUploadStaging: () => void | Promise<void>;
  onUpdateStagedUploadCaption: (stagedId: string, caption: string) => void | Promise<void>;
  onUpdateStagedUploadCompression: (
    stagedId: string,
    compressionChoice: StagedUploadCompressionChoice
  ) => void | Promise<void>;
  onComposerDraftChange: (value: string) => void;
  onMentionIntentChange: (intent: MentionIntent) => void;
  onEditMessage: (message: { body: string | null; room_id: string; event_id: string }) => void;
  onOpenContextMenu: OpenContextMenu;
  onOpenThread: (roomId: string, rootEventId: string) => void;
  onRedactMessage: (roomId: string, eventId: string) => void;
  onReply: TimelineRowActionHandlers["onReply"];
  onRescheduleScheduledSend: (scheduledId: string, sendAtMs: number) => void;
  onResultSelect: (roomId: string, eventId: string) => void;
  onScheduleSend: (sendAtMs: number, body: string) => void;
  onSendText: (body: string) => void;
  onSetLocalUserAlias: (userId: string, alias: string | null) => void;
  onUnpinPinnedEvent: (roomId: string, eventId: string) => void;
  onOpenPeople: () => void;
  onOpenThreads: () => void;
  onToggleRoomInfo: () => void;
  onReturnToLive?: () => void;
  onTimelineDiagnosticsChange?: (diagnostics: TimelineDiagnostics) => void;
  onTimelineDiagnosticLogEntry?: (entry: TimelineDiagnosticLogEntry) => void;
}) {
  const timelineRoomId = snapshot.state.ui.timeline.room_id;
  const currentUserId = snapshot.state.domain.session.user_id ?? null;
  const activeRoom = timelineRoomId
    ? snapshot.state.domain.rooms.find((room) => room.room_id === timelineRoomId) ?? null
    : null;
  const threadAttention = snapshot.state.domain.thread_attention;
  const showThreadsHeader =
    timelineRoomId &&
    threadAttention.kind === "tracking" &&
    threadAttention.room_id === timelineRoomId &&
    (threadAttention.notification_count > 0 ||
      threadAttention.highlight_count > 0 ||
      threadAttention.live_event_marker_count > 0);
  // #161: when the main pane is anchored (jump-to-date landed on an event), it
  // renders the focused (event-centered) timeline instead of the live room
  // timeline; the right panel is not opened.
  const mainTimelineAnchorEventId =
    snapshot.state.ui.navigation.main_timeline_anchor?.event_id ?? null;
  const timelineKey = useMemo(() => {
    if (!currentUserId || !timelineRoomId) {
      return null;
    }
    if (mainTimelineAnchorEventId) {
      return focusedTimelineKey(currentUserId, timelineRoomId, mainTimelineAnchorEventId);
    }
    return roomTimelineKey(currentUserId, timelineRoomId);
  }, [currentUserId, timelineRoomId, mainTimelineAnchorEventId]);
  const composerModeForComposer = useMemo(
    () => composerMode,
    [
      composerMode.kind,
      composerMode.kind === "reply" ? composerMode.in_reply_to_event_id : null
    ]
  );
  const pinnedEvents = timelineRoomId
    ? snapshot.state.domain.room_interactions[timelineRoomId]?.pinned_events ?? EMPTY_PINNED_EVENTS
    : EMPTY_PINNED_EVENTS;
  const pinnedEventIds = useMemo(
    () => pinnedEvents.map((event) => event.event_id),
    [pinnedEvents]
  );
  const stagedUploads = snapshot.state.ui.timeline.staged_uploads ?? [];
  const mediaGallery = snapshot.state.ui.timeline.media_gallery ?? [];
  const mediaDownloads = snapshot.state.ui.timeline.media_downloads ?? {};
  const forwardDestinations = useAppStore(selectForwardDestinations);
  const mentionCandidates = useAppStore(selectMentionCandidates);
  const resolveComposerKeyActionStable = useStableEvent(resolveComposerKeyAction);
  const onCancelReplyStable = useStableEvent(onCancelReply);
  const onCancelScheduledSendStable = useStableEvent(onCancelScheduledSend);
  const onAttachFilesStable = useStableEvent(onAttachFiles);
  const onClearUploadStagingStable = useStableEvent(onClearUploadStaging);
  const onUpdateStagedUploadCaptionStable = useStableEvent(onUpdateStagedUploadCaption);
  const onUpdateStagedUploadCompressionStable = useStableEvent(onUpdateStagedUploadCompression);
  const onComposerDraftChangeStable = useStableEvent(onComposerDraftChange);
  const onMentionIntentChangeStable = useStableEvent(onMentionIntentChange);
  const onEditMessageStable = useStableEvent(onEditMessage);
  const onOpenContextMenuStable = useStableEvent(onOpenContextMenu);
  const onOpenThreadStable = useStableEvent(onOpenThread);
  const onRedactMessageStable = useStableEvent(onRedactMessage);
  const onReplyStable = useStableEvent(onReply);
  const onRescheduleScheduledSendStable = useStableEvent(onRescheduleScheduledSend);
  const onResultSelectStable = useStableEvent(onResultSelect);
  const onScheduleSendStable = useStableEvent(onScheduleSend);
  const onSendTextStable = useStableEvent(onSendText);
  const onSetLocalUserAliasStable = useStableEvent(onSetLocalUserAlias);
  const onUnpinPinnedEventStable = useStableEvent(onUnpinPinnedEvent);
  const onOpenPeopleStable = useStableEvent(onOpenPeople);
  const onOpenThreadsStable = useStableEvent(onOpenThreads);
  const onToggleRoomInfoStable = useStableEvent(onToggleRoomInfo);
  const onTimelineDiagnosticsChangeStable = useStableEvent(
    (diagnostics: TimelineDiagnostics) => onTimelineDiagnosticsChange?.(diagnostics)
  );
  const onTimelineDiagnosticLogEntryStable = useStableEvent(
    (entry: TimelineDiagnosticLogEntry) => onTimelineDiagnosticLogEntry?.(entry)
  );
  const [galleryOpen, setGalleryOpen] = useState(false);
  const [viewerIndex, setViewerIndex] = useState<number | null>(null);
  const jumpToLatestRef = useRef<(() => void) | null>(null);
  const registerJumpToLatest = useCallback((handler: (() => void) | null) => {
    jumpToLatestRef.current = handler;
  }, []);

  return (
    <main className="main-pane" aria-label={t("timeline.conversation")}>
      <header className="channel-header">
        <div className="channel-title">
          <EntityAvatar
            avatar={activeRoom?.avatar ?? null}
            className="channel-avatar is-room"
            fallback={initials(activeRoomName)}
          />
          <span>{activeRoomName}</span>
        </div>
        <div className="channel-actions">
          <nav className="timeline-header-navigation" aria-label={t("timeline.navigation")}>
            <button
              className="icon-button timeline-control"
              type="button"
              aria-label={t("timeline.latest")}
              title={t("timeline.latest")}
              onClick={() => {
                if (mainTimelineAnchorEventId && onReturnToLive) {
                  onReturnToLive();
                  return;
                }
                jumpToLatestRef.current?.();
              }}
            >
              <ArrowDown size={ICON_SIZE.control} aria-hidden="true" />
            </button>
          </nav>
          <button
            className="icon-button"
            type="button"
            aria-label={t("panel.people")}
            title={t("panel.people")}
            onClick={onOpenPeopleStable}
          >
            <Users size={ICON_SIZE.panel} />
          </button>
          <button
            className="icon-button"
            type="button"
            aria-label={t("mediaGallery.open")}
            title={t("mediaGallery.open")}
            onClick={() => setGalleryOpen((open) => !open)}
          >
            <ImageIcon size={ICON_SIZE.panel} />
          </button>
          {showThreadsHeader ? (
            <button
              className="icon-button"
              type="button"
              aria-label={t("workspace.threads")}
              title={t("workspace.threads")}
              onClick={onOpenThreadsStable}
            >
              <MessageCircle size={ICON_SIZE.panel} />
            </button>
          ) : null}
          <button
            className="icon-button"
            type="button"
            aria-label={t("room.roomInfo")}
            title={t("room.roomInfo")}
            onClick={onToggleRoomInfoStable}
          >
            <MoreHorizontal size={ICON_SIZE.panel} />
          </button>
        </div>
      </header>
      {galleryOpen ? (
        <RoomMediaGallery
          items={mediaGallery}
          mediaDownloads={mediaDownloads}
          onOpenItem={(index) => setViewerIndex(index)}
        />
      ) : null}
      <section className="timeline-scroll">
        {timelineRoomId && pinnedEvents.length > 0 ? (
          <PinnedEventsList
            roomId={timelineRoomId}
            pinnedEvents={pinnedEvents}
            onUnpin={onUnpinPinnedEventStable}
          />
        ) : null}
        {showSearchResults ? (
          <SearchResults
            query={searchQuery}
            results={searchResults}
            rooms={snapshot.state.domain.rooms}
            onResultSelect={onResultSelectStable}
          />
        ) : null}
        <div className="message-list">
          {timelineTransport && timelineRoomId && currentUserId ? (
            // Production path: render from the event-driven timeline store
            // (CoreEvent diffs), never from AppState timeline fields.
            <TimelineView
              key={
                mainTimelineAnchorEventId
                  ? `anchored:${timelineRoomId}:${mainTimelineAnchorEventId}`
                  : timelineRoomId
              }
              roomId={timelineRoomId}
              timelineKey={timelineKey!}
              isAnchored={Boolean(mainTimelineAnchorEventId)}
              onReturnToLive={onReturnToLive}
              transport={timelineTransport}
              onReply={onReplyStable}
              onOpenThread={onOpenThreadStable}
              resolveComposerKeyAction={resolveComposerKeyActionStable}
              liveSignals={snapshot.state.domain.live_signals}
              profileUsers={snapshot.state.domain.profile.users}
              pinnedEventIds={pinnedEventIds}
              forwardDestinations={forwardDestinations}
              onSetLocalUserAlias={onSetLocalUserAliasStable}
              onOpenContextMenu={onOpenContextMenuStable}
              currentUserId={currentUserId}
              ignoredUserIds={snapshot.state.domain.profile.ignored_user_ids}
              autoLoadOlderMessages={snapshot.state.domain.settings.values.timeline.auto_load_older_messages}
              codeBlockWrap={snapshot.state.domain.settings.values.display.code_block_wrap}
              searchQuery={searchQuery}
              mediaDownloads={mediaDownloads}
              roomScrollAnchor={
                mainTimelineAnchorEventId
                  ? null
                  : (snapshot.state.ui.navigation.room_scroll_anchors?.[timelineRoomId] ?? null)
              }
              onDiagnosticsChange={onTimelineDiagnosticsChangeStable}
              onDiagnosticLogEntry={onTimelineDiagnosticLogEntryStable}
              onRegisterJumpToLatest={registerJumpToLatest}
            />
          ) : (
            // Browser fixture preview only (no Tauri runtime).
            <div className="message-fixture-list">
              {snapshot.timeline.map((message) => (
                <MessageArticle
                  key={message.event_id}
                  message={message}
                  query={searchQuery}
                  currentUserId={currentUserId}
                  onOpenContextMenu={onOpenContextMenuStable}
                  onEditMessage={onEditMessageStable}
                  onOpenThread={onOpenThreadStable}
                  onRedactMessage={onRedactMessageStable}
                  profileUsers={snapshot.state.domain.profile.users}
                  isIgnored={snapshot.state.domain.profile.ignored_user_ids.includes(message.sender)}
                />
              ))}
            </div>
          )}
        </div>
      </section>
      <ScheduledMessagesList
        capability={snapshot.state.ui.timeline.scheduled_send_capability}
        items={snapshot.state.ui.timeline.scheduled_sends}
        onCancel={onCancelScheduledSendStable}
        onReschedule={onRescheduleScheduledSendStable}
      />
      {stagedUploads.length > 0 ? (
        <UploadStagingDialog
          items={stagedUploads}
          onClear={onClearUploadStagingStable}
          onUpdateCaption={onUpdateStagedUploadCaptionStable}
          onUpdateCompression={onUpdateStagedUploadCompressionStable}
        />
      ) : null}
      <Composer
        composerMode={composerModeForComposer}
        hasStagedUploads={stagedUploads.length > 0}
        isSending={Boolean(snapshot.state.ui.timeline.composer.pending_transaction_id)}
        mentionCandidates={mentionCandidates}
        mentionIntent={mentionIntent}
        resolveComposerKeyAction={resolveComposerKeyActionStable}
        draftKey={timelineRoomId ?? "no-room"}
        roomName={activeRoomName}
        value={composerDraft}
        onCancelReply={onCancelReplyStable}
        onAttachFiles={onAttachFilesStable}
        onMentionIntentChange={onMentionIntentChangeStable}
        onScheduleSend={onScheduleSendStable}
        onSend={onSendTextStable}
        onValueChange={onComposerDraftChangeStable}
      />
      {viewerIndex !== null && mediaGallery[viewerIndex] ? (
        <MediaViewer
          index={viewerIndex}
          items={mediaGallery}
          mediaDownloads={mediaDownloads}
          onClose={() => setViewerIndex(null)}
          onSelectIndex={setViewerIndex}
        />
      ) : null}
    </main>
  );
}
