import { type FormEvent, type RefObject } from "react";
import { MoreHorizontal, X } from "lucide-react";
import { t } from "../i18n/messages";
import type {
  AttachmentFilter,
  AttachmentScope,
  AttachmentSort,
  DesktopSnapshot,
  FilesViewScope,
  ResolveComposerKeyAction,
  RoomModerationAction,
  RoomNotificationMode,
  RoomSettingChange,
  SavedSessionInfo,
  SearchResult,
  SettingsPatch
} from "../domain/types";
import {
  focusedTimelineKey,
  threadTimelineKey
} from "../domain/coreEvents";
import {
  currentSavedSession,
  forwardDestinationsFromSnapshot,
  ICON_SIZE,
  ignoreComposerKeyAction,
  pinnedEventsForRoom,
  shortcutLabelProfileFromLocaleProfile,
  threadReplyToTimelineMessage
} from "../app/uiShared";
import type { DisplayDensity, SpaceLocalOverrides } from "../app/localPresentation";
import {
  roomOrSpaceForPeoplePanelScope,
  type PeoplePanelScope,
  type RightPanelMode
} from "../domain/rightPanel";
import { RecoveryPanel } from "./auth";
import {
  TimelineView,
  type TimelineDiagnosticLogEntry,
  type TimelineRowActionHandlers,
  type TimelineTransport
} from "./TimelineView";
import { FilesView } from "./FilesView";
import { KeyboardSettingsPanel } from "./KeyboardSettingsPanel";
import { RoomInfoPanel } from "./RoomInfoPanel";
import { SpaceInfoPanel } from "./SpaceInfoPanel";
import { ThreadsListView } from "./ThreadsListView";
import { UserSettingsPanel } from "./UserSettingsPanel";
import { PeoplePanel, ProfilePanel } from "./PeoplePanel";
import { MessageArticle, SearchResults } from "./mediaLists";
import { ThreadComposer } from "./composer";

export function ContextualRightPanel({
  activeRoom,
  activeSpace,
  activeSpaceName,
  displayDensity = "comfortable",
  isRecoveryBusy,
  mode,
  peoplePanelScope = null,
  selectedProfileUserId = null,
  recoverySecretFilled,
  recoverySecretInputRef,
  snapshot,
  timelineTransport = null,
  searchIndexingPending = false,
  searchQuery,
  searchResults,
  savedSessions,
  onCloseThread,
  onClosePanel,
  onOpenThread,
  onOpenFiles,
  onOpenSpaceMembers,
  onOpenPeople: _onOpenPeople,
  onOpenProfile,
  onBackToPeople,
  onRefreshFilesView,
  onPaginateThreadsList,
  onOpenKeyboardSettings,
  onOpenRecovery,
  onProbeLocalEncryption,
  onResetLocalData,
  onLogout = () => undefined,
  onInviteUser = () => undefined,
  onModerateMember = () => undefined,
  onSetLocalUserAlias = () => undefined,
  onSetRoomNotificationMode = () => undefined,
  onStartDirectMessage = () => undefined,
  onUpdateMemberRole = () => undefined,
  onReshareRoomKey = () => undefined,
  onRecoverySecretPresenceChange,
  onReply,
  onResultSelect,
  onSubmitRecovery,
  onSwitchAccount,
  onAcceptVerification,
  onBootstrapCrossSigning,
  onCancelVerification,
  onConfirmSasVerification,
  onChooseRoomKeyExportDestination = async () => null,
  onChooseRoomKeyImportSource = async () => null,
  onExportRoomKeys,
  onImportRoomKeys,
  onBootstrapSecureBackup,
  onChangeSecureBackupPassphrase,
  onEnableKeyBackup,
  onResetIdentity,
  onCancelIdentityReset,
  onResolveComposerKeyAction = ignoreComposerKeyAction,
  onSetAvatar = () => undefined,
  onSetDisplayName = () => undefined,
  onSubmitIdentityResetOAuth,
  onSubmitIdentityResetPassword,
  onUpdateSettings = () => undefined,
  onRebuildSearchIndex = () => undefined,
  onSetRoomUrlPreviewOverride = () => undefined,
  onResetRoomTimelineCache = () => undefined,
  onUpdateRoomSetting = () => undefined,
  onIgnoreUser = () => undefined,
  onUnignoreUser = () => undefined,
  onReportUser = () => undefined,
  onQueryDevices = () => undefined,
  onRenameDevice = () => undefined,
  onDeleteDevices = () => undefined,
  onLoadAccountManagementCapabilities = () => undefined,
  onChangePassword = () => undefined,
  onDeactivateAccount = () => undefined,
  onSubmitAccountManagementUia = () => undefined,
  onStartCrawlRoom = () => undefined,
  onStopCrawlRoom = () => undefined,
  onDisplayDensityChange = () => undefined,
  onSetSpaceLocalOverride = () => undefined,
  spaceLocalOverrides = {},
  onTimelineDiagnosticLogEntry,
  onThreadComposerDraftChange,
  onThreadReplySend,
  threadComposerDraftOverrides = {}
}: {
  activeRoom: DesktopSnapshot["state"]["domain"]["rooms"][number] | null;
  activeSpace: DesktopSnapshot["state"]["domain"]["spaces"][number] | null;
  activeSpaceName: string;
  displayDensity?: DisplayDensity;
  isRecoveryBusy: boolean;
  mode: RightPanelMode;
  peoplePanelScope?: PeoplePanelScope | null;
  roomInfoInitialSection?: "members" | null;
  selectedProfileUserId?: string | null;
  recoverySecretFilled: boolean;
  recoverySecretInputRef: RefObject<HTMLInputElement | null>;
  snapshot: DesktopSnapshot;
  timelineTransport?: TimelineTransport | null;
  searchIndexingPending?: boolean;
  searchQuery: string;
  searchResults: SearchResult[];
  savedSessions: SavedSessionInfo[];
  onCloseThread: () => void;
  onClosePanel: () => void;
  onOpenThread: (roomId: string, rootEventId: string) => void;
  onOpenFiles: (scope: FilesViewScope) => void;
  onOpenSpaceMembers?: () => void;
  onOpenPeople?: () => void;
  onOpenProfile?: (userId: string) => void;
  onBackToPeople?: () => void;
  onRefreshFilesView: (scope: AttachmentScope, filter: AttachmentFilter, sort: AttachmentSort) => void;
  onPaginateThreadsList: (roomId: string) => void;
  onOpenKeyboardSettings: () => void;
  onOpenRecovery: () => void;
  onProbeLocalEncryption: () => void;
  onResetLocalData: () => void;
  onLogout?: () => void;
  onInviteUser?: (roomId: string, title: string) => void;
  onModerateMember?: (
    roomId: string,
    targetUserId: string,
    action: RoomModerationAction,
    reason: string | null
  ) => void;
  onSetLocalUserAlias?: (userId: string, alias: string | null) => void;
  onRequestMemberAvatarThumbnail?: (mxcUri: string) => void | Promise<void>;
  onSetRoomNotificationMode?: (roomId: string, mode: RoomNotificationMode) => void;
  onStartDirectMessage?: (userId: string) => void;
  onUpdateMemberRole?: (
    roomId: string,
    targetUserId: string,
    powerLevel: number
  ) => void;
  onReshareRoomKey?: (roomId: string) => void;
  onRecoverySecretPresenceChange: (value: boolean) => void;
  onReply: TimelineRowActionHandlers["onReply"];
  onResultSelect: (roomId: string, eventId: string) => void;
  onSubmitRecovery: (event: FormEvent<HTMLFormElement>) => void;
  onSwitchAccount: (session: SavedSessionInfo) => void;
  onAcceptVerification: (flowId: number) => void;
  onBootstrapCrossSigning: () => void;
  onCancelVerification: (flowId: number) => void;
  onConfirmSasVerification: (flowId: number) => void;
  onChooseRoomKeyExportDestination?: () => Promise<string | null>;
  onChooseRoomKeyImportSource?: () => Promise<string | null>;
  onExportRoomKeys: (destinationPath: string, passphrase: string) => void;
  onImportRoomKeys: (sourcePath: string, passphrase: string) => void;
  onBootstrapSecureBackup: (
    passphrase: string | null,
    recoveryKeyDestinationPath: string | null
  ) => void;
  onChangeSecureBackupPassphrase: (
    oldSecret: string,
    newPassphrase: string,
    recoveryKeyDestinationPath: string | null
  ) => void;
  onEnableKeyBackup: () => void;
  onResetIdentity: () => void;
  onCancelIdentityReset: (flowId: number) => void;
  onResolveComposerKeyAction?: ResolveComposerKeyAction;
  onSetAvatar?: (file: File) => void;
  onSetDisplayName?: (displayName: string | null) => void;
  onSubmitIdentityResetOAuth: (flowId: number) => void;
  onSubmitIdentityResetPassword: (flowId: number, password: string) => void;
  onUpdateSettings?: (patch: SettingsPatch) => void;
  onRebuildSearchIndex?: () => void;
  onSetRoomUrlPreviewOverride?: (roomId: string, enabled: boolean) => void;
  onResetRoomTimelineCache?: (roomId: string) => void | Promise<void>;
  onQueryDevices?: () => void;
  onRenameDevice?: (deviceOrdinal: number, displayName: string) => void;
  onDeleteDevices?: (deviceOrdinals: number[]) => void;
  onLoadAccountManagementCapabilities?: () => void;
  onChangePassword?: (newPassword: string) => void;
  onDeactivateAccount?: (eraseData: boolean) => void;
  onSubmitAccountManagementUia?: (flowId: number, password: string) => void;
  onStartCrawlRoom?: (roomId: string) => void;
  onStopCrawlRoom?: (roomId: string) => void;
  onDisplayDensityChange?: (density: DisplayDensity) => void;
  onSetSpaceLocalOverride?: (
    spaceId: string,
    override: { name?: string; icon?: string } | null
  ) => void;
  spaceLocalOverrides?: SpaceLocalOverrides;
  onTimelineDiagnosticLogEntry?: (entry: TimelineDiagnosticLogEntry) => void;
  onUpdateRoomSetting?: (roomId: string, change: RoomSettingChange) => void;
  onIgnoreUser?: (userId: string) => void;
  onUnignoreUser?: (userId: string) => void;
  onReportUser?: (userId: string) => void;
  onThreadComposerDraftChange: (roomId: string, rootEventId: string, draft: string) => void;
  onThreadReplySend: (roomId: string, rootEventId: string, body: string) => void;
  threadComposerDraftOverrides?: Record<string, string>;
}) {
  const mediaDownloads = snapshot.state.ui.timeline.media_downloads ?? {};

  if (mode === "closed") {
    return <aside className="thread-pane" aria-label={t("panel.context")} />;
  }

  if (mode === "recovery") {
    return (
      <aside className="thread-pane" aria-label={t("panel.context")}>
        <PanelHeader title={t("panel.recovery")} onClose={onClosePanel} showClose={false} />
        <RecoveryPanel
          isBusy={isRecoveryBusy}
          secretFilled={recoverySecretFilled}
          secretInputRef={recoverySecretInputRef}
          snapshot={snapshot}
          onSecretPresenceChange={onRecoverySecretPresenceChange}
          onSubmit={onSubmitRecovery}
        />
      </aside>
    );
  }

  if (mode === "keyboardSettings") {
    return (
      <aside className="thread-pane" aria-label={t("panel.context")}>
        <PanelHeader title={t("panel.keyboard")} onClose={onClosePanel} />
        <KeyboardSettingsPanel
          labelProfile={shortcutLabelProfileFromLocaleProfile(snapshot.state.domain.locale_profile)}
          settings={snapshot.state.domain.settings}
          onUpdateSettings={onUpdateSettings}
        />
      </aside>
    );
  }

  if (mode === "userSettings") {
    return (
      <aside className="thread-pane" aria-label={t("panel.context")}>
        <PanelHeader title={t("panel.userSettings")} onClose={onClosePanel} />
        <UserSettingsPanel
          currentSession={currentSavedSession(snapshot)}
          displayDensity={displayDensity}
          e2eeTrust={snapshot.state.domain.e2ee_trust}
          localEncryption={snapshot.state.domain.local_encryption}
          keyboardLabelProfile={shortcutLabelProfileFromLocaleProfile(snapshot.state.domain.locale_profile)}
          platform={snapshot.state.domain.locale_profile.platform}
          profile={snapshot.state.domain.profile}
          savedSessions={savedSessions}
          searchCrawlerState={snapshot.state.domain.search_crawler}
          settings={snapshot.state.domain.settings}
          onAcceptVerification={onAcceptVerification}
          onBootstrapCrossSigning={onBootstrapCrossSigning}
          onCancelVerification={onCancelVerification}
          onConfirmSasVerification={onConfirmSasVerification}
          onChooseRoomKeyExportDestination={onChooseRoomKeyExportDestination}
          onChooseRoomKeyImportSource={onChooseRoomKeyImportSource}
          onExportRoomKeys={onExportRoomKeys}
          onImportRoomKeys={onImportRoomKeys}
          onBootstrapSecureBackup={onBootstrapSecureBackup}
          onChangeSecureBackupPassphrase={onChangeSecureBackupPassphrase}
          onEnableKeyBackup={onEnableKeyBackup}
          onOpenRecovery={onOpenRecovery}
          onOpenKeyboardSettings={onOpenKeyboardSettings}
          onProbeLocalEncryption={onProbeLocalEncryption}
          onResetLocalData={onResetLocalData}
          onLogout={onLogout}
          onResetIdentity={onResetIdentity}
          onCancelIdentityReset={onCancelIdentityReset}
          onSetAvatar={onSetAvatar}
          onSetDisplayName={onSetDisplayName}
          onSubmitIdentityResetOAuth={onSubmitIdentityResetOAuth}
          onSubmitIdentityResetPassword={onSubmitIdentityResetPassword}
          onUpdateSettings={onUpdateSettings}
          onRebuildSearchIndex={onRebuildSearchIndex}
          onSwitchAccount={onSwitchAccount}
          deviceSessions={snapshot.state.domain.device_sessions}
          accountManagement={snapshot.state.domain.account_management}
          accountManagementCapabilities={snapshot.state.domain.account_management_capabilities}
          onQueryDevices={onQueryDevices ?? (() => undefined)}
          onRenameDevice={onRenameDevice ?? (() => undefined)}
          onDeleteDevices={onDeleteDevices ?? (() => undefined)}
          onLoadAccountManagementCapabilities={
            onLoadAccountManagementCapabilities ?? (() => undefined)
          }
          onChangePassword={onChangePassword ?? (() => undefined)}
          onDeactivateAccount={onDeactivateAccount ?? (() => undefined)}
          onSubmitAccountManagementUia={onSubmitAccountManagementUia ?? (() => undefined)}
          onStartCrawlRoom={onStartCrawlRoom}
          onStopCrawlRoom={onStopCrawlRoom}
          onDisplayDensityChange={onDisplayDensityChange}
          rooms={snapshot.state.domain.rooms}
        />
      </aside>
    );
  }

  if (mode === "roomInfo") {
    return (
      <aside className="thread-pane" aria-label={t("panel.context")}>
        <PanelHeader title={t("panel.roomInfo")} onClose={onClosePanel} />
        <RoomInfoPanel
          room={activeRoom}
          roomManagement={snapshot.state.domain.room_management}
          roomNotificationSettings={
            activeRoom ? snapshot.state.domain.room_notification_settings[activeRoom.room_id] : undefined
          }
          appSettings={snapshot.state.domain.settings}
          linkPreviewSettings={snapshot.state.domain.link_preview_settings}
          spaces={snapshot.state.domain.spaces}
          onInvitePeople={
            activeRoom
              ? () =>
                  onInviteUser(
                    activeRoom.room_id,
                    t("dialog.invitePeopleTitle", { name: activeRoom.display_label })
                  )
              : undefined
          }
          onOpenFiles={
            activeRoom
              ? () => onOpenFiles({ kind: "room", room_id: activeRoom.room_id })
              : undefined
          }
          onSetRoomNotificationMode={onSetRoomNotificationMode}
          onReshareRoomKey={onReshareRoomKey}
          onUpdateRoomSetting={onUpdateRoomSetting}
          onSetRoomUrlPreviewOverride={(roomId, enabled) => {
            void onSetRoomUrlPreviewOverride(roomId, enabled);
          }}
          onResetRoomTimelineCache={onResetRoomTimelineCache}
          onOpenPeople={() => {
            void _onOpenPeople?.();
          }}
        />
      </aside>
    );
  }

  if (mode === "people" || mode === "profile") {
    const roomOrSpace = roomOrSpaceForPeoplePanelScope(
      peoplePanelScope,
      activeRoom,
      activeSpace,
      snapshot.state.domain.rooms,
      snapshot.state.domain.spaces
    );
    return (
      <aside className="thread-pane" aria-label={t("panel.context")}>
        {mode === "profile" && selectedProfileUserId ? (
          <ProfilePanel
            userId={selectedProfileUserId}
            currentUserId={snapshot.state.domain.session.user_id ?? null}
            ignoredUserIds={snapshot.state.domain.profile.ignored_user_ids}
            roomOrSpace={roomOrSpace}
            roomManagement={snapshot.state.domain.room_management}
            profileUsers={snapshot.state.domain.profile.users}
            onBack={onBackToPeople ?? onClosePanel}
            onClose={onClosePanel}
            onIgnoreUser={onIgnoreUser}
            onModerateMember={onModerateMember}
            onReportUser={onReportUser}
            onStartDirectMessage={onStartDirectMessage}
            onSetLocalUserAlias={onSetLocalUserAlias}
            onUnignoreUser={onUnignoreUser}
            onUpdateMemberRole={onUpdateMemberRole}
          />
        ) : (
          <PeoplePanel
            currentUserId={snapshot.state.domain.session.user_id ?? null}
            roomOrSpace={roomOrSpace}
            roomManagement={snapshot.state.domain.room_management}
            onOpenProfile={onOpenProfile ?? (() => undefined)}
            onClose={onClosePanel}
            onInvitePeople={
              roomOrSpace
                ? () =>
                    onInviteUser(
                      "room_id" in roomOrSpace ? roomOrSpace.room_id : roomOrSpace.space_id,
                      t("dialog.invitePeopleTitle", {
                        name:
                          "display_label" in roomOrSpace
                            ? roomOrSpace.display_label
                            : roomOrSpace.display_name
                      })
                    )
                : undefined
            }
            onStartDirectMessage={onStartDirectMessage}
          />
        )}
      </aside>
    );
  }

  if (mode === "spaceInfo") {
    return (
      <aside className="thread-pane" aria-label={t("panel.context")}>
        <PanelHeader title={t("panel.spaceInfo")} onClose={onClosePanel} />
        <SpaceInfoPanel
          fallbackName={activeSpaceName}
          localIcon={activeSpace ? spaceLocalOverrides[activeSpace.space_id]?.icon ?? "" : ""}
          localName={activeSpace ? spaceLocalOverrides[activeSpace.space_id]?.name ?? "" : ""}
          roomManagement={snapshot.state.domain.room_management}
          rooms={snapshot.state.domain.rooms}
          space={activeSpace}
          onInvitePeople={
            activeSpace
              ? () =>
                  onInviteUser(
                    activeSpace.space_id,
                    t("dialog.invitePeopleTitle", { name: activeSpace.display_name })
                  )
              : undefined
          }
          onOpenFiles={
            activeSpace
              ? () => onOpenFiles({ kind: "space", space_id: activeSpace.space_id })
              : undefined
          }
          onOpenMembers={
            activeSpace
              ? onOpenSpaceMembers
              : undefined
          }
          onSetLocalPresentation={
            activeSpace
              ? (override) => onSetSpaceLocalOverride(activeSpace.space_id, override)
              : undefined
          }
        />
      </aside>
    );
  }

  if (mode === "files") {
    return (
      <aside className="thread-pane" aria-label={t("panel.context")}>
        <PanelHeader title={t("files.title")} onClose={onClosePanel} />
        <FilesView
          filesView={snapshot.state.ui.files_view}
          onChangeFilterSort={onRefreshFilesView}
        />
      </aside>
    );
  }

  if (mode === "threads") {
    return (
      <aside className="thread-pane" aria-label={t("panel.context")}>
        <PanelHeader title={t("threads.title")} onClose={onClosePanel} />
        <ThreadsListView
          threadsList={snapshot.state.ui.threads_list}
          roomId={activeRoom?.room_id ?? null}
          onClose={onClosePanel}
          onOpenThread={onOpenThread}
          onPaginate={onPaginateThreadsList}
        />
      </aside>
    );
  }

  if (mode === "search" || mode === "focusedContext") {
    const focusedContext = snapshot.state.ui.focused_context;
    const currentUserId = snapshot.state.domain.session.user_id ?? null;
    const focusedTimelineKeyValue =
      currentUserId &&
      timelineTransport &&
      (focusedContext.kind === "opening" || focusedContext.kind === "open")
        ? focusedTimelineKey(currentUserId, focusedContext.room_id, focusedContext.event_id)
        : null;
    const focusedRoomId =
      focusedContext.kind === "opening" || focusedContext.kind === "open"
        ? focusedContext.room_id
        : null;
    const focusedTimelineTransport = timelineTransport;
    const focusedPinnedEventIds = pinnedEventsForRoom(snapshot, focusedRoomId).map(
      (event) => event.event_id
    );

    return (
      <aside className="thread-pane" aria-label={t("panel.context")}>
        <PanelHeader
          title={mode === "search" ? t("panel.search") : t("panel.focusedContext")}
          onClose={onClosePanel}
        />
        {focusedTimelineKeyValue && focusedRoomId && focusedTimelineTransport ? (
          <section className="focused-context-panel" aria-label={t("panel.focusedContext")}>
            {mode === "search" ? (
              <div className="search-results-header">
                <span>{t("panel.focusedContext")}</span>
              </div>
            ) : null}
            <TimelineView
              roomId={focusedRoomId}
              timelineKey={focusedTimelineKeyValue}
              transport={focusedTimelineTransport}
              suppressPaginationUi={true}
              onReply={onReply}
              resolveComposerKeyAction={onResolveComposerKeyAction}
              liveSignals={snapshot.state.domain.live_signals}
              profileUsers={snapshot.state.domain.profile.users}
              pinnedEventIds={focusedPinnedEventIds}
              forwardDestinations={forwardDestinationsFromSnapshot(snapshot)}
              onSetLocalUserAlias={onSetLocalUserAlias}
              autoLoadOlderMessages={snapshot.state.domain.settings.values.timeline.auto_load_older_messages}
              codeBlockWrap={snapshot.state.domain.settings.values.display.code_block_wrap}
              searchQuery={searchQuery}
              mediaDownloads={mediaDownloads}
            />
          </section>
        ) : null}
        {mode === "search" ? (
          <SearchResults
            indexingPending={searchIndexingPending}
            query={searchQuery}
            results={searchResults}
            rooms={snapshot.state.domain.rooms}
            onResultSelect={onResultSelect}
          />
        ) : null}
      </aside>
    );
  }

  const threadState = snapshot.state.ui.thread;
  if (threadState.kind !== "opening" && threadState.kind !== "open") {
    return <aside className="thread-pane" aria-label={t("panel.context")} />;
  }

  const currentUserId = snapshot.state.domain.session.user_id ?? null;
  const threadRoomId = threadState.room_id;
  const rootEventId = threadState.root_event_id;
  const threadComposer = threadState.kind === "open" ? threadState.composer : undefined;
  const threadDraftKeyValue =
    threadRoomId && rootEventId ? threadComposerDraftKey(threadRoomId, rootEventId) : null;
  const threadDraft =
    threadDraftKeyValue &&
    Object.prototype.hasOwnProperty.call(threadComposerDraftOverrides, threadDraftKeyValue)
      ? threadComposerDraftOverrides[threadDraftKeyValue] ?? ""
      : threadComposer?.draft ?? "";
  const threadSendPending = Boolean(threadComposer?.pending_transaction_id);
  const threadTimelineKeyValue =
    currentUserId && timelineTransport && threadRoomId && rootEventId
      ? threadTimelineKey(currentUserId, threadRoomId, rootEventId)
      : null;
  const fixtureThreadSnapshot = snapshot.thread;
  const browserThreadSnapshot =
    !timelineTransport &&
    fixtureThreadSnapshot &&
    fixtureThreadSnapshot.room_id === threadRoomId &&
    fixtureThreadSnapshot.root_event_id === rootEventId
      ? fixtureThreadSnapshot
      : null;
  const threadPinnedEventIds = pinnedEventsForRoom(snapshot, threadRoomId).map(
    (event) => event.event_id
  );

  return (
    <aside className="thread-pane" aria-label={t("panel.context")}>
      <PanelHeader title={t("panel.thread")} onClose={onCloseThread} />
      <section className="thread-scroll thread-timeline-panel">
        {threadTimelineKeyValue && threadRoomId && timelineTransport ? (
          <TimelineView
            key={`${threadRoomId}:${rootEventId}`}
            roomId={threadRoomId}
            timelineKey={threadTimelineKeyValue}
            transport={timelineTransport}
            onReply={onReply}
            onOpenThread={() => undefined}
            resolveComposerKeyAction={onResolveComposerKeyAction}
            liveSignals={snapshot.state.domain.live_signals}
            profileUsers={snapshot.state.domain.profile.users}
            pinnedEventIds={threadPinnedEventIds}
            forwardDestinations={forwardDestinationsFromSnapshot(snapshot)}
            onSetLocalUserAlias={onSetLocalUserAlias}
            autoLoadOlderMessages={snapshot.state.domain.settings.values.timeline.auto_load_older_messages}
            codeBlockWrap={snapshot.state.domain.settings.values.display.code_block_wrap}
            searchQuery={searchQuery}
            mediaDownloads={mediaDownloads}
            onDiagnosticLogEntry={onTimelineDiagnosticLogEntry}
          />
        ) : browserThreadSnapshot ? (
          <div className="message-fixture-list thread-fixture-list">
            {browserThreadSnapshot.replies.map((reply) => (
              <MessageArticle
                key={reply.event_id}
                message={threadReplyToTimelineMessage(reply)}
                query={searchQuery}
                currentUserId={currentUserId}
                onEditMessage={() => undefined}
                onOpenThread={() => undefined}
                onRedactMessage={() => undefined}
                profileUsers={snapshot.state.domain.profile.users}
                isIgnored={snapshot.state.domain.profile.ignored_user_ids.includes(reply.sender)}
              />
            ))}
          </div>
        ) : (
          <div className="thread-root-placeholder">{t("timeline.openingThread")}</div>
        )}
      </section>
      <ThreadComposer
        draft={threadDraft}
        isSending={threadSendPending}
        resolveComposerKeyAction={onResolveComposerKeyAction}
        canEdit={threadState.kind === "open" && Boolean(threadRoomId && rootEventId && threadComposer)}
        onDraftChange={(draft) => {
          if (threadRoomId && rootEventId) {
            onThreadComposerDraftChange(threadRoomId, rootEventId, draft);
          }
        }}
        onSend={() => {
          if (threadRoomId && rootEventId) {
            onThreadReplySend(threadRoomId, rootEventId, threadDraft);
          }
        }}
      />
    </aside>
  );
}

function threadComposerDraftKey(roomId: string, rootEventId: string): string {
  return `${roomId}\u0000${rootEventId}`;
}

export function PanelHeader({
  title,
  onClose,
  showClose = true
}: {
  title: string;
  onClose: () => void;
  showClose?: boolean;
}) {
  return (
    <header className="thread-header">
      <div className="thread-title">{title}</div>
      <button className="icon-button" type="button" aria-label={t("action.more")}>
        <MoreHorizontal size={ICON_SIZE.panel} />
      </button>
      {showClose ? (
        <button className="icon-button" type="button" aria-label={t("action.close", { title })} onClick={onClose}>
          <X size={ICON_SIZE.panel} />
        </button>
      ) : null}
    </header>
  );
}
