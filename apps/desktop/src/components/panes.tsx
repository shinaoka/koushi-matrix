import {
  useCallback,
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
  StagedUploadOutputSelection,
  ActivityMarkReadTarget,
  ActivityRow,
  ActivityState,
  ActivityStream,
  ActivityTab,
  DesktopSnapshot,
  DirectoryRoomSummary,
  ComposerDocument,
  ResolveComposerKeyAction,
  SearchResult
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
  invokeReturnToLiveSafely,
  roomLatestDisplayEventId,
  type TimelineDiagnosticLogEntry,
  type TimelineDiagnostics,
  type TimelineRowActionHandlers,
  type TimelineThreadAttention,
  type TimelineTransport,
  type ReturnToLiveHandler
} from "./TimelineView";
import { EntityAvatar } from "./Shell";
import {
  MessageArticle,
  RoomMediaGallery,
  MediaViewer,
  ScheduledMessagesList,
  PinnedMessagesEntry,
  SearchResults
} from "./mediaLists";
import { Composer } from "./composer";
import { UploadStagingDialog, uploadStagingItemsAreSendable } from "./dialogs";
import { ImeSafeForm, ImeTextField } from "./ImeTextControl";
import { useStableEvent } from "./useStableEvent";

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

export function ActivityPane({
  activity,
  onClose,
  onLoadMore,
  onMarkRead,
  onOpenRow,
  onRetryResolution,
  onSetTab
}: {
  activity: ActivityState;
  onClose: () => void;
  onLoadMore: (tab: ActivityTab, cursor: string | null) => void;
  onMarkRead: (target: ActivityMarkReadTarget) => void;
  onOpenRow: (row: ActivityRow) => void;
  onRetryResolution: () => void;
  onSetTab: (tab: ActivityTab) => void;
}) {
  const activeTab =
    activity.kind === "open" ? activity.active_tab : activity.kind === "opening" ? activity.tab : "recent";
  const stream = activity.kind === "open" ? activityStream(activity, activeTab) : null;
  const rows = stream?.rows ?? [];
  const resolution = activeTab === "unread" ? stream?.resolution : undefined;
  const visibleRows = activeTab === "unread"
    ? rows.filter((row) => row.kind === "event")
    : rows;
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
        ) : (
          <>
            {resolution?.kind === "resolving" ? (
              <div className="activity-empty" role="status">
                <Clock3 size={ICON_SIZE.emptyState} />
                <span>{t("activity.resolvingUnread")}</span>
              </div>
            ) : resolution?.kind === "failed" ? (
              <div className="activity-empty" role="alert">
                <span>{t("activity.resolveFailed")}</span>
                <button className="dialog-button secondary" type="button" onClick={onRetryResolution}>
                  {t("activity.retryResolution")}
                </button>
              </div>
            ) : null}
            {visibleRows.length === 0 && resolution?.kind !== "resolving" && resolution?.kind !== "failed" ? (
              <div className="activity-empty">
                <Clock3 size={ICON_SIZE.emptyState} />
                <span>
                  {activeTab === "recent" ? t("activity.noRecent") : t("activity.noUnread")}
                </span>
              </div>
            ) : visibleRows.length > 0 ? (
              <ol className="activity-list">
            {visibleRows.map((row) => {
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
                        colorSeed={row.room_id}
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
                        colorSeed={row.sender_id ?? row.room_id}
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
            ) : null}
          </>
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
  addressDraft,
  addressNotice,
  isBusy,
  queryDraft,
  serverDraft,
  snapshot,
  onAddressChange,
  onJoinRoom,
  onQueryChange,
  onServerChange,
  onSearch,
  onSubmitAddress
}: {
  /** Matrix address or link to preview; separate from the directory search term. */
  addressDraft: string;
  /**
   * Why the last address submission produced nothing. Rust owns preview/join
   * state; this only explains input that never became a target.
   */
  addressNotice: "user" | "notRecognized" | null;
  isBusy: boolean;
  queryDraft: string;
  /** Homeserver whose public directory to query; blank means the user's own. */
  serverDraft: string;
  snapshot: DesktopSnapshot;
  onAddressChange: (value: string) => void;
  onJoinRoom: (room: DirectoryRoomSummary) => void;
  onQueryChange: (value: string) => void;
  onServerChange: (value: string) => void;
  onSearch: () => void;
  onSubmitAddress: () => void;
}) {
  const queryState = snapshot.state.domain.directory.query;
  const joinState = snapshot.state.domain.directory.join;
  const rooms = queryState.kind === "results" ? queryState.rooms : [];
  const searchDisabled = isBusy || queryState.kind === "querying";

  function submitSearch(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    onSearch();
  }

  function submitAddress(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    onSubmitAddress();
  }

  return (
    <main className="main-pane explore-pane" aria-labelledby="explore-title">
      <header className="channel-header">
        <div className="channel-title">
          <Compass size={ICON_SIZE.large} />
          <h1 id="explore-title">{t("workspace.explore")}</h1>
        </div>
      </header>
      <section className="directory-section" aria-label={t("directory.joinSectionTitle")}>
        <h2>{t("directory.joinSectionTitle")}</h2>
        <ImeSafeForm className="directory-search directory-address-form" onSubmit={submitAddress}>
          <label className="directory-search-field">
            <span>{t("directory.addressLabel")}</span>
            <ImeTextField
              type="text"
              value={addressDraft}
              syncKey="directory-address"
              aria-label={t("directory.addressLabel")}
              placeholder={t("directory.addressPlaceholder")}
              onChange={(event) => onAddressChange(event.currentTarget.value)}
            />
          </label>
          <button
            className="dialog-button is-primary"
            type="submit"
            aria-label={t("directory.preview")}
            disabled={isBusy}
          >
            <Compass size={ICON_SIZE.small} />
            <span>{t("directory.preview")}</span>
          </button>
        </ImeSafeForm>
        <p className="directory-field-helper">{t("directory.addressHelper")}</p>
        {addressNotice ? (
          <div className="directory-status" role="status">
            {addressNotice === "user"
              ? t("directory.addressIsUser")
              : t("directory.addressNotRecognized")}
          </div>
        ) : null}
      </section>
      <section className="directory-section" aria-label={t("directory.searchSectionTitle")}>
        <h2>{t("directory.searchSectionTitle")}</h2>
        <ImeSafeForm className="directory-search" onSubmit={submitSearch}>
          <label className="directory-search-field">
            <span>{t("directory.searchTermLabel")}</span>
            <ImeTextField
              type="search"
              value={queryDraft}
              syncKey="directory-search"
              aria-label={t("directory.searchTermLabel")}
              placeholder={t("directory.searchPlaceholder")}
              onChange={(event) => onQueryChange(event.currentTarget.value)}
            />
          </label>
          <label className="directory-search-field directory-search-server">
            <span>{t("directory.searchServer")}</span>
            <ImeTextField
              type="text"
              value={serverDraft}
              syncKey="directory-search-server"
              aria-label={t("directory.searchServer")}
              placeholder={t("directory.searchServerPlaceholder")}
              onChange={(event) => onServerChange(event.currentTarget.value)}
            />
          </label>
          <button
            className="dialog-button is-primary"
            type="submit"
            aria-label={t("directory.search")}
            disabled={searchDisabled}
          >
            <Search size={ICON_SIZE.small} />
            <span>
              {queryState.kind === "querying"
                ? t("directory.searching")
                : t("directory.search")}
            </span>
          </button>
        </ImeSafeForm>
        <p className="directory-field-helper">{t("directory.searchServerHelper")}</p>
      </section>
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
            // Join by alias when there is one, otherwise by room id: a public
            // space often has no canonical alias, and refusing those would
            // make it findable but unjoinable.
            const joinTarget = alias ?? room.room_id;
            const joiningThisRoom =
              joinState.kind === "joining" && joinState.room_id_or_alias === joinTarget;
            const joinFailed =
              joinState.kind === "failed" && joinState.room_id_or_alias === joinTarget
                ? joinState
                : null;
            const isSpace = room.room_type === "m.space";
            const displayName =
              room.name.trim() ||
              alias ||
              t(isSpace ? "directory.unnamedSpace" : "directory.unnamedRoom");
            const canJoin = !joiningThisRoom && !isBusy;
            return (
              <article className="directory-result" key={room.room_id}>
                <div className="directory-result-avatar" aria-hidden="true">
                  <span dir="auto">{initials(displayName)}</span>
                </div>
                <div className="directory-result-main">
                  <h2>
                    <span className="directory-result-name" dir="auto">
                      {displayName}
                    </span>
                    <span className="directory-result-type">
                      {isSpace ? t("directory.spaceBadge") : t("directory.roomBadge")}
                    </span>
                  </h2>
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
                  aria-label={t("directory.joinRoom", { name: displayName })}
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
                  colorSeed={invite.room_id}
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
                  colorSeed={selectedInvite.room_id}
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
  canEdit = true,
  composerDocument,
  composerNotice = null,
  composerDraftKey,
  composerMode,
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
  onSelectStagedUploadOutput,
  onSendStagedAttachments,
  onLoadStagedUploadPreview,
  onRetryStagedUploadPreparation = () => undefined,
  onUseOriginalStagedUpload = () => undefined,
  onComposerDocumentChange,
  onComposerMathModeChange,
  onRecentEmojisChange = () => undefined,
  onMentionQueryChange,
  onEditMessage,
  onOpenContextMenu,
  onOpenThread,
  onRedactMessage,
  onReply,
  onOpenMatrixTarget,
  onOpenSenderProfile,
  onStartDirectMessage,
  onRescheduleScheduledSend,
  onResultSelect,
  onScheduleSend,
  onSendText,
  onSetLocalUserAlias,
  onUnpinPinnedEvent: _onUnpinPinnedEvent,
  onOpenPinnedMessages = () => undefined,
  onOpenPeople,
  onOpenThreads,
  onToggleRoomInfo,
  onReturnToLive,
  onTimelineDiagnosticsChange,
  onTimelineDiagnosticLogEntry
}: {
  activeRoomName: string;
  canEdit?: boolean;
  composerDocument: ComposerDocument;
  composerDraftKey?: string;
  composerMode: ComposerModeProp;
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
  onUpdateStagedUploadCaption: (stagedId: string, document: ComposerDocument) => void | Promise<void>;
  onSelectStagedUploadOutput: (
    stagedId: string,
    selection: StagedUploadOutputSelection
  ) => void | Promise<void>;
  onSendStagedAttachments: () => void | Promise<void>;
  onLoadStagedUploadPreview: (stagedId: string, variantId: string) => Promise<number[]>;
  onRetryStagedUploadPreparation?: (stagedId: string) => void | Promise<void>;
  onUseOriginalStagedUpload?: (stagedId: string) => void | Promise<void>;
  onComposerDocumentChange: (document: ComposerDocument) => void;
  onComposerMathModeChange: (enabled: boolean) => void | Promise<void>;
  onRecentEmojisChange?: (emojis: string[]) => void | Promise<void>;
  onMentionQueryChange?: (roomId: string, query: string | null) => void;
  onEditMessage: (message: { body: string | null; room_id: string; event_id: string }) => void;
  onOpenContextMenu: OpenContextMenu;
  onOpenThread: TimelineRowActionHandlers["onOpenThread"];
  onRedactMessage: (roomId: string, eventId: string) => void;
  onReply: TimelineRowActionHandlers["onReply"];
  onOpenMatrixTarget?: TimelineRowActionHandlers["onOpenMatrixTarget"];
  onOpenSenderProfile?: TimelineRowActionHandlers["onOpenSenderProfile"];
  onStartDirectMessage?: (userId: string) => void;
  onRescheduleScheduledSend: (scheduledId: string, body: string, sendAtMs: number) => void;
  onResultSelect: (roomId: string, eventId: string) => void;
  onScheduleSend: (sendAtMs: number, document: ComposerDocument) => void;
  onSendText: (document: ComposerDocument) => void;
  /** Localized transient notice rendered above the main composer (#450). */
  composerNotice?: string | null;
  onSetLocalUserAlias: (userId: string, alias: string | null) => void;
  /** Kept for fixture compatibility; pinned unpin actions live in the panel. */
  onUnpinPinnedEvent?: (roomId: string, eventId: string) => void;
  onOpenPinnedMessages?: () => void;
  onOpenPeople: () => void;
  onOpenThreads: () => void;
  onToggleRoomInfo: () => void;
  onReturnToLive?: ReturnToLiveHandler;
  onTimelineDiagnosticsChange?: (diagnostics: TimelineDiagnostics) => void;
  onTimelineDiagnosticLogEntry?: (entry: TimelineDiagnosticLogEntry) => void;
}) {
  const timelineRoomId = snapshot.state.ui.timeline.room_id;
  const currentUserId = snapshot.state.domain.session.user_id ?? null;
  const activeRoom = timelineRoomId
    ? snapshot.state.domain.rooms.find((room) => room.room_id === timelineRoomId) ?? null
    : null;
  const liveLatestEventId = roomLatestDisplayEventId(activeRoom?.latest_event);
  const threadAttention = snapshot.state.domain.thread_attention;
  const trackingThreadAttention = threadAttention.kind === "tracking" ? threadAttention : null;
  const timelineThreadAttention = useMemo<TimelineThreadAttention | null>(() => {
    if (!timelineRoomId || !trackingThreadAttention || trackingThreadAttention.room_id !== timelineRoomId) {
      return null;
    }
    return {
      rootEventId: trackingThreadAttention.root_event_id,
      notificationCount: trackingThreadAttention.notification_count,
      highlightCount: trackingThreadAttention.highlight_count,
      liveEventMarkerCount: trackingThreadAttention.live_event_marker_count
    };
  }, [
    timelineRoomId,
    trackingThreadAttention?.room_id,
    trackingThreadAttention?.root_event_id,
    trackingThreadAttention?.notification_count,
    trackingThreadAttention?.highlight_count,
    trackingThreadAttention?.live_event_marker_count
  ]);
  const threadsHeaderNotificationCount = timelineThreadAttention?.notificationCount ?? 0;
  const threadsHeaderHighlightCount = timelineThreadAttention?.highlightCount ?? 0;
  const threadsHeaderLiveCount = timelineThreadAttention?.liveEventMarkerCount ?? 0;
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
  const searchHighlightsByEventId = useMemo(
    () =>
      Object.fromEntries(
        searchResults
          .filter((result) => result.match_field === "messageBody")
          .map((result) => [
            result.event_id,
            { snippet: result.snippet, ranges: result.highlights }
          ])
      ),
    [searchResults]
  );
  // Same guard as the staging panel's Send button: every item prepared and
  // none still recompressing (#500).
  const stagedUploadsReady = uploadStagingItemsAreSendable(stagedUploads);
  const mediaGallery = snapshot.state.ui.timeline.media_gallery ?? [];
  const mediaDownloads = snapshot.state.ui.timeline.media_downloads ?? {};
  const forwardDestinations = useAppStore(selectForwardDestinations);
  const mentionCandidates = useAppStore((state) =>
    selectMentionCandidates(state, timelineRoomId, "main")
  );
  const mentionCandidateTarget = snapshot.state.domain.mention_candidates.targets.find(
    (target) => target.room_id === timelineRoomId && target.surface === "main"
  );
  const mentionCandidatesLoading =
    mentionCandidateTarget?.completeness === "loading" ||
    mentionCandidateTarget?.completeness === "partial";
  const resolveComposerKeyActionStable = useStableEvent(resolveComposerKeyAction);
  const onCancelReplyStable = useStableEvent(onCancelReply);
  const onCancelScheduledSendStable = useStableEvent(onCancelScheduledSend);
  const onAttachFilesStable = useStableEvent(onAttachFiles);
  const onClearUploadStagingStable = useStableEvent(onClearUploadStaging);
  const onUpdateStagedUploadCaptionStable = useStableEvent(onUpdateStagedUploadCaption);
  const onSelectStagedUploadOutputStable = useStableEvent(onSelectStagedUploadOutput);
  const onSendStagedAttachmentsStable = useStableEvent(onSendStagedAttachments);
  const onLoadStagedUploadPreviewStable = useStableEvent(onLoadStagedUploadPreview);
  const onRetryStagedUploadPreparationStable = useStableEvent(onRetryStagedUploadPreparation);
  const onUseOriginalStagedUploadStable = useStableEvent(onUseOriginalStagedUpload);
  const onComposerDocumentChangeStable = useStableEvent(onComposerDocumentChange);
  const onComposerMathModeChangeStable = useStableEvent(onComposerMathModeChange);
  const onRecentEmojisChangeStable = useStableEvent(onRecentEmojisChange);
  const onMentionQueryChangeStable = useStableEvent((query: string | null) => {
    if (timelineRoomId) {
      onMentionQueryChange?.(timelineRoomId, query);
    }
  });
  const onEditMessageStable = useStableEvent(onEditMessage);
  const onOpenContextMenuStable = useStableEvent(onOpenContextMenu);
  const onOpenThreadStable = useStableEvent(onOpenThread);
  const onRedactMessageStable = useStableEvent(onRedactMessage);
  const onReplyStable = useStableEvent(onReply);
  const onOpenMatrixTargetStable = useStableEvent(
    // A pane without in-app navigation must leave matrix.to links external,
    // so absence stays absent rather than becoming a click-swallowing no-op.
    onOpenMatrixTarget ?? (() => undefined)
  );
  const onOpenSenderProfileStable = useStableEvent(
    onOpenSenderProfile ?? (() => undefined)
  );
  const onStartDirectMessageStable = useStableEvent(
    onStartDirectMessage ?? (() => undefined)
  );
  const onRescheduleScheduledSendStable = useStableEvent(onRescheduleScheduledSend);
  const onResultSelectStable = useStableEvent(onResultSelect);
  const onScheduleSendStable = useStableEvent(onScheduleSend);
  const onSendTextStable = useStableEvent(onSendText);
  const onSetLocalUserAliasStable = useStableEvent(onSetLocalUserAlias);
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
            colorSeed={activeRoom?.room_id ?? activeRoomName}
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
                  invokeReturnToLiveSafely(onReturnToLive);
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
          {/* #330: the only entry point to this room's threads, so it is always
              offered. The counts render as badges when non-zero. */}
          <button
            className="icon-button"
            type="button"
            data-count={threadsHeaderNotificationCount || undefined}
            data-live-count={threadsHeaderLiveCount || undefined}
            data-mention-count={threadsHeaderHighlightCount || undefined}
            aria-label={t("workspace.threads")}
            title={t("workspace.threads")}
            onClick={onOpenThreadsStable}
          >
            <MessageCircle size={ICON_SIZE.panel} />
          </button>
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
          <PinnedMessagesEntry count={pinnedEvents.length} onOpen={onOpenPinnedMessages} />
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
              liveLatestEventId={liveLatestEventId}
              transport={timelineTransport}
              onReply={onReplyStable}
              onOpenMatrixTarget={onOpenMatrixTarget ? onOpenMatrixTargetStable : undefined}
              onOpenSenderProfile={
                onOpenSenderProfile ? onOpenSenderProfileStable : undefined
              }
              onStartDirectMessage={
                onStartDirectMessage ? onStartDirectMessageStable : undefined
              }
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
              recentEmojis={snapshot.state.domain.settings.values.composer.recent_emojis}
              onRecentEmojisChange={onRecentEmojisChangeStable}
              searchHighlightsByEventId={searchHighlightsByEventId}
              mediaDownloads={mediaDownloads}
              mentionCandidates={mentionCandidates}
              mentionCandidatesLoading={mentionCandidatesLoading}
              onMentionQueryChange={onMentionQueryChangeStable}
              continuity={snapshot.state.ui.timeline.continuity ?? { kind: "unknown" }}
              density={snapshot.state.domain.settings.values.appearance.density}
              roomScrollAnchor={
                mainTimelineAnchorEventId
                  ? null
                  : (snapshot.state.ui.navigation.room_scroll_anchors?.[timelineRoomId] ?? null)
              }
              onDiagnosticsChange={onTimelineDiagnosticsChangeStable}
              onDiagnosticLogEntry={onTimelineDiagnosticLogEntryStable}
              onRegisterJumpToLatest={registerJumpToLatest}
              threadAttention={timelineThreadAttention}
            />
          ) : (
            // Browser fixture preview only (no Tauri runtime).
            <div className="message-fixture-list">
              {snapshot.timeline.map((message) => (
                <MessageArticle
                  key={message.event_id}
                  message={message}
                  highlights={
                    searchHighlightsByEventId[message.event_id]?.snippet === message.body
                      ? searchHighlightsByEventId[message.event_id]?.ranges ?? []
                      : []
                  }
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
          onSelectOutput={onSelectStagedUploadOutputStable}
          onSendAttachments={onSendStagedAttachmentsStable}
          onRetryPreparation={onRetryStagedUploadPreparationStable}
          onUseOriginal={onUseOriginalStagedUploadStable}
          loadPreview={onLoadStagedUploadPreviewStable}
          surface="main"
          resolveComposerKeyAction={resolveComposerKeyActionStable}
          mentionCandidates={mentionCandidates}
          mentionCandidatesLoading={mentionCandidatesLoading}
          onMentionQueryChange={onMentionQueryChangeStable}
          mathModeEnabled={snapshot.state.domain.settings.values.composer.math_mode}
          recentEmojis={snapshot.state.domain.settings.values.composer.recent_emojis}
          onMathModeChange={onComposerMathModeChangeStable}
          onRecentEmojisChange={onRecentEmojisChangeStable}
          roomName={activeRoomName}
        />
      ) : null}
      <Composer
        canEdit={canEdit}
        composerMode={composerModeForComposer}
        preferSendOnForwardTab
        hasStagedUploads={stagedUploads.length > 0}
        stagedUploadsReady={stagedUploadsReady}
        onSendStagedUploads={onSendStagedAttachmentsStable}
        isSending={Boolean(snapshot.state.ui.timeline.composer.pending_transaction_id)}
        mathModeEnabled={snapshot.state.domain.settings.values.composer.math_mode}
        recentEmojis={snapshot.state.domain.settings.values.composer.recent_emojis}
        onRecentEmojisChange={onRecentEmojisChangeStable}
        mentionCandidates={mentionCandidates}
        mentionCandidatesLoading={mentionCandidatesLoading}
        resolveComposerKeyAction={resolveComposerKeyActionStable}
        document={composerDocument}
        notice={composerNotice}
        draftKey={composerDraftKey ?? timelineRoomId ?? "no-room"}
        roomName={activeRoomName}
        onCancelReply={onCancelReplyStable}
        onAttachFiles={onAttachFilesStable}
        onDocumentChange={onComposerDocumentChangeStable}
        onMathModeChange={onComposerMathModeChangeStable}
        onMentionQueryChange={onMentionQueryChangeStable}
        onScheduleSend={onScheduleSendStable}
        onSend={onSendTextStable}
        onDiagnosticLogEntry={onTimelineDiagnosticLogEntryStable}
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
