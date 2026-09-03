import { type FormEvent, type RefObject, useMemo } from "react";
import { X } from "lucide-react";
import { t } from "../i18n/messages";
import type {
  StagedUploadOutputSelection,
  AttachmentFilter,
  AttachmentScope,
  AttachmentSort,
  DesktopSnapshot,
  DisplayDensity,
  FilesViewScope,
  ComposerDocument,
  ResolveComposerKeyAction,
  RoomModerationAction,
  InviteTargetCandidate,
  RoomNotificationMode,
  RoomSettingChange,
  SavedSessionInfo,
  SearchResult,
  SettingsPatch,
  SpaceLocalPresentation,
  SpaceMemberRoleOption,
  SecureBackupSetupIntent,
  PinnedEventNavigation,
  ThreadOpenIntent,
  ThreadsListScope
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
import { selectMentionCandidates } from "../domain/appStore";
import { documentFromText } from "../domain/composerDocument";
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
import {
  SpaceMembersPanel,
  type SpaceInviteAvailabilityReason,
  type SpaceInviteCancellationAvailabilityReason
} from "./SpaceMembersPanel";
import { MessageArticle, PinnedEventsList, SearchResults } from "./mediaLists";
import { ThreadComposer } from "./composer";
import { UploadStagingDialog, uploadStagingItemsAreSendable } from "./dialogs";
import type { OpenContextMenu } from "../app/uiShared";
import { useStableEvent } from "./useStableEvent";

const noopSearchSpaceInviteTargets = async (): Promise<InviteTargetCandidate[]> => [];
const noopResetSpaceInviteSearch = (): void => undefined;

export function ContextualRightPanel({
  activeRoom,
  activeSpace,
  activeSpaceName,
  accountManagementUrl = null,
  displayDensity = "comfortable",
  encryptedComposerBlocked = false,
  isRecoveryBusy,
  mode,
  threadsListScope = { kind: "home" },
  peoplePanelScope = null,
  selectedProfileUserId = null,
  recoverySecretFilled,
  recoverySecretInputRef,
  snapshot,
  timelineTransport = null,
  searchIndexingPending = false,
  searchPending = false,
  searchTooShortMinChars = null,
  searchQuery,
  searchResults,
  savedSessions,
  onCloseThread,
  onClosePanel,
  onOpenThread,
  onOpenFiles,
  onOpenPinnedEvent = () => undefined,
  onUnpinPinnedEvent = () => undefined,
  pinnedNavigation = null,
  onRetryPinnedEvent = () => undefined,
  onOpenSpaceMembers,
  onOpenContextMenu,
  onDiagnostic,
  onRequestMemberAvatarThumbnail,
  spaceInviteAvailabilityReason,
  onCancelInvite,
  canCancelInvite = false,
  cancelAvailabilityReason,
  cancelInviteFailure = false,
  roleUpdateFailure = false,
  onOpenPeople: _onOpenPeople,
  onOpenProfile,
  onBackToPeople,
  onRefreshFilesView,
  onPaginateThreadsList,
  onOpenKeyboardSettings,
  onOpenRecovery,
  onManageAccount = () => undefined,
  onRefreshCurrentSessionStatus = () => undefined,
  onProbeLocalEncryption,
  onResetLocalData,
  onLogout = () => undefined,
  onInviteUser = () => undefined,
  onReturnToInvite = () => undefined,
  onInviteUserToSpace = () => undefined,
  onInviteSearchCandidateToSpace = () => undefined,
  onSearchSpaceInviteTargets = noopSearchSpaceInviteTargets,
  onResetSpaceInviteSearch = noopResetSpaceInviteSearch,
  canInviteToSpace = false,
  onModerateMember = () => undefined,
  onSetLocalUserAlias = () => undefined,
  onSetRoomNotificationMode = () => undefined,
  onStartDirectMessage = () => undefined,
  onUpdateMemberRole = () => undefined,
  onUpdateSpaceMemberRole = () => undefined,
  onReloadSpaceMemberRoles = () => undefined,
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
  onChooseSecureBackupDestination = async () => null,
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
  onRepairRoomTimeline = () => undefined,
  onForceRotateOutboundSession = () => undefined,
  onUpdateRoomSetting = () => undefined,
  onIgnoreUser = () => undefined,
  onUnignoreUser = () => undefined,
  onReportUser = () => undefined,
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
  onThreadComposerDocumentChange,
  onThreadAttachFiles = () => undefined,
  onThreadClearUploadStaging = () => undefined,
  onThreadLoadStagedUploadPreview = async () => [],
  onThreadMentionQueryChange = () => undefined,
  onThreadRetryStagedUploadPreparation = () => undefined,
  onThreadReplySend,
  onThreadScheduleSend,
  onThreadSelectStagedUploadOutput = () => undefined,
  onThreadSendStagedAttachments = () => undefined,
  onThreadUseOriginalStagedUpload = () => undefined,
  onThreadUpdateStagedUploadCaption = () => undefined,
  threadComposerDraftImeKey,
  threadComposerNotice = null,
  threadComposerDocumentOverride
}: {
  activeRoom: DesktopSnapshot["state"]["domain"]["rooms"][number] | null;
  activeSpace: DesktopSnapshot["state"]["domain"]["spaces"][number] | null;
  activeSpaceName: string;
  displayDensity?: DisplayDensity;
  encryptedComposerBlocked?: boolean;
  isRecoveryBusy: boolean;
  mode: RightPanelMode;
  threadsListScope?: ThreadsListScope;
  peoplePanelScope?: PeoplePanelScope | null;
  roomInfoInitialSection?: "members" | null;
  selectedProfileUserId?: string | null;
  recoverySecretFilled: boolean;
  recoverySecretInputRef: RefObject<HTMLInputElement | null>;
  snapshot: DesktopSnapshot;
  timelineTransport?: TimelineTransport | null;
  searchIndexingPending?: boolean;
  searchPending?: boolean;
  searchTooShortMinChars?: number | null;
  searchQuery: string;
  searchResults: SearchResult[];
  savedSessions: SavedSessionInfo[];
  onCloseThread: () => void;
  onClosePanel: () => void;
  onOpenThread: (
    roomId: string,
    rootEventId: string,
    intent: ThreadOpenIntent
  ) => void;
  onOpenFiles: (scope: FilesViewScope) => void;
  onOpenPinnedEvent?: (roomId: string, eventId: string, threadRootEventId: string | null) => void;
  onUnpinPinnedEvent?: (roomId: string, eventId: string) => void;
  pinnedNavigation?: PinnedEventNavigation | null;
  onRetryPinnedEvent?: (roomId: string, eventId: string, threadRootEventId: string | null) => void;
  onOpenSpaceMembers?: () => void;
  onOpenContextMenu?: OpenContextMenu;
  onDiagnostic?: (message: string) => void;
  spaceInviteAvailabilityReason?: SpaceInviteAvailabilityReason;
  onCancelInvite?: (userId: string) => void;
  canCancelInvite?: boolean;
  cancelAvailabilityReason?: SpaceInviteCancellationAvailabilityReason;
  cancelInviteFailure?: boolean;
  roleUpdateFailure?: boolean;
  onOpenPeople?: () => void;
  onOpenProfile?: (userId: string) => void;
  onBackToPeople?: () => void;
  onRefreshFilesView: (scope: AttachmentScope, filter: AttachmentFilter, sort: AttachmentSort) => void;
  onPaginateThreadsList: (scope: ThreadsListScope) => void;
  onOpenKeyboardSettings: () => void;
  onOpenRecovery: () => void;
  onManageAccount?: () => void;
  onRefreshCurrentSessionStatus?: () => void;
  accountManagementUrl?: string | null;
  onProbeLocalEncryption: () => void;
  onResetLocalData: () => void;
  onLogout?: () => void;
  onInviteUser?: (roomId: string, title: string) => void;
  onReturnToInvite?: () => void;
  onInviteUserToSpace?: (userId: string) => void;
  onInviteSearchCandidateToSpace?: (userId: string) => void;
  onSearchSpaceInviteTargets?: (query: string) => Promise<InviteTargetCandidate[]>;
  onResetSpaceInviteSearch?: () => void;
  canInviteToSpace?: boolean;
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
  onUpdateSpaceMemberRole?: (userId: string, option: SpaceMemberRoleOption) => void;
  onReloadSpaceMemberRoles?: () => void;
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
  onChooseSecureBackupDestination?: () => Promise<string | null>;
  onExportRoomKeys: (destinationPath: string, passphrase: string) => void;
  onImportRoomKeys: (sourcePath: string, passphrase: string) => void;
  onBootstrapSecureBackup: (
    passphrase: string | null,
    recoveryKeyDestinationPath: string | null,
    intent: SecureBackupSetupIntent
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
  onRepairRoomTimeline?: (roomId: string) => void | Promise<void>;
  onForceRotateOutboundSession?: (roomId: string) => void | Promise<void>;
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
  spaceLocalOverrides?: Record<string, SpaceLocalPresentation>;
  onTimelineDiagnosticLogEntry?: (entry: TimelineDiagnosticLogEntry) => void;
  onUpdateRoomSetting?: (roomId: string, change: RoomSettingChange) => void;
  onIgnoreUser?: (userId: string) => void;
  onUnignoreUser?: (userId: string) => void;
  onReportUser?: (userId: string) => void;
  onThreadComposerDocumentChange: (
    roomId: string,
    rootEventId: string,
    document: ComposerDocument
  ) => void;
  onThreadAttachFiles?: (roomId: string, rootEventId: string, files: File[]) => void;
  onThreadClearUploadStaging?: (roomId: string, rootEventId: string) => void;
  onThreadLoadStagedUploadPreview?: (
    roomId: string,
    rootEventId: string,
    stagedId: string,
    variantId: string
  ) => Promise<number[]>;
  onThreadMentionQueryChange?: (
    roomId: string,
    query: string | null
  ) => void;
  onThreadRetryStagedUploadPreparation?: (
    roomId: string,
    rootEventId: string,
    stagedId: string
  ) => void;
  onThreadReplySend: (
    roomId: string,
    rootEventId: string,
    document: ComposerDocument
  ) => void;
  onThreadScheduleSend?: (
    roomId: string,
    rootEventId: string,
    sendAtMs: number,
    document: ComposerDocument
  ) => void;
  onThreadSelectStagedUploadOutput?: (
    roomId: string,
    rootEventId: string,
    stagedId: string,
    selection: StagedUploadOutputSelection
  ) => void;
  onThreadSendStagedAttachments?: (roomId: string, rootEventId: string) => void;
  onThreadUseOriginalStagedUpload?: (
    roomId: string,
    rootEventId: string,
    stagedId: string
  ) => void;
  onThreadUpdateStagedUploadCaption?: (
    roomId: string,
    rootEventId: string,
    stagedId: string,
    document: ComposerDocument
  ) => void | Promise<void>;
  threadComposerDraftImeKey?: string;
  /** Localized transient notice for the open thread composer (#450). */
  threadComposerNotice?: string | null;
  threadComposerDocumentOverride?: ComposerDocument;
}) {
  const composerSettings = snapshot.state.domain.settings?.values.composer ?? {
    math_mode: true,
    recent_emojis: []
  };
  const mediaDownloads = snapshot.state.ui.timeline.media_downloads ?? {};
  const loadThreadPreview = useStableEvent(onThreadLoadStagedUploadPreview);
  const onRecentEmojisChange = useStableEvent((recent_emojis: string[]) =>
    onUpdateSettings?.({ composer: { ...composerSettings, recent_emojis } })
  );
  const threadTarget = snapshot.state.ui.thread ?? { kind: "closed" as const };
  const threadPreviewRoomId =
    threadTarget.kind === "opening" || threadTarget.kind === "open" ? threadTarget.room_id : null;
  const threadPreviewRootEventId =
    threadTarget.kind === "opening" || threadTarget.kind === "open"
      ? threadTarget.root_event_id
      : null;
  const threadPreviewLoader = useMemo(() => {
    return (stagedId: string, variantId: string) =>
      threadPreviewRoomId && threadPreviewRootEventId
        ? loadThreadPreview(threadPreviewRoomId, threadPreviewRootEventId, stagedId, variantId)
        : Promise.resolve([]);
  }, [loadThreadPreview, threadPreviewRoomId, threadPreviewRootEventId]);
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
          currentSessionStatus={snapshot.state.domain.current_session_status}
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
          onChooseSecureBackupDestination={onChooseSecureBackupDestination}
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
          accountManagement={snapshot.state.domain.account_management}
          accountManagementCapabilities={snapshot.state.domain.account_management_capabilities}
          accountManagementUrl={accountManagementUrl}
          onManageAccount={onManageAccount}
          onRefreshCurrentSessionStatus={onRefreshCurrentSessionStatus}
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
          onUpdateRoomSetting={onUpdateRoomSetting}
          inviteHistoryPolicy={
            snapshot.state.domain.invite_workflow?.query.room_id === activeRoom?.room_id
              ? snapshot.state.domain.invite_workflow?.history_policy ?? null
              : null
          }
          onOpenRecovery={
            snapshot.state.domain.invite_workflow?.query.room_id === activeRoom?.room_id
              ? onOpenRecovery
              : undefined
          }
          onReturnToInvite={
            snapshot.state.domain.invite_workflow?.query.room_id === activeRoom?.room_id
              ? onReturnToInvite
              : undefined
          }
          onSetRoomUrlPreviewOverride={(roomId, enabled) => {
            void onSetRoomUrlPreviewOverride(roomId, enabled);
          }}
          onRepairRoomTimeline={onRepairRoomTimeline}
          onForceRotateOutboundSession={onForceRotateOutboundSession}
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
    const childRoomLabels = new Map<string, string>();
    for (const room of snapshot.state.domain.rooms) {
      const label = [room.display_label, room.display_name]
        .map((value) => value.trim())
        .find((value) => value.length > 0 && value !== room.room_id);
      if (label) {
        childRoomLabels.set(room.room_id, label);
      }
    }
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
        ) : mode === "people" && peoplePanelScope?.kind === "space" ? (
          <SpaceMembersPanel
            state={snapshot.state.domain.space_members}
            canInvite={canInviteToSpace}
            onClose={onClosePanel}
            profileUsers={snapshot.state.domain.profile.users}
            onRequestAvatarThumbnail={onRequestMemberAvatarThumbnail}
            childRoomLabels={childRoomLabels}
            onInviteUser={onInviteUserToSpace}
            onInviteSearchCandidate={onInviteSearchCandidateToSpace}
            onSearchInviteTargets={onSearchSpaceInviteTargets}
            onResetInviteSearch={onResetSpaceInviteSearch}
            onCancelInvite={onCancelInvite}
            onUpdateRole={onUpdateSpaceMemberRole}
            onReloadRoles={onReloadSpaceMemberRoles}
            canCancelInvite={canCancelInvite}
            onOpenProfile={onOpenProfile ?? (() => undefined)}
            onOpenContextMenu={onOpenContextMenu}
            onDiagnostic={onDiagnostic}
            inviteAvailabilityReason={spaceInviteAvailabilityReason}
            cancelAvailabilityReason={cancelAvailabilityReason}
            cancelInviteFailure={cancelInviteFailure}
            roleUpdateFailure={roleUpdateFailure}
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

  if (mode === "pinned") {
    const pinnedRoomId = activeRoom?.room_id ?? snapshot.state.ui.timeline.room_id;
    const pinnedEvents = pinnedRoomId
      ? snapshot.state.domain.room_interactions[pinnedRoomId]?.pinned_events ?? []
      : [];
    return (
      <aside className="thread-pane" aria-label={t("panel.context")}>
        <PanelHeader title={t("timeline.pinnedMessages")} onClose={onClosePanel} />
        {pinnedRoomId && pinnedEvents.length > 0 ? (
          <PinnedEventsList
            roomId={pinnedRoomId}
            pinnedEvents={pinnedEvents}
            profileUsers={snapshot.state.domain.profile.users}
            onOpen={onOpenPinnedEvent}
            navigation={pinnedNavigation}
            onRetry={onRetryPinnedEvent}
            onUnpin={onUnpinPinnedEvent}
          />
        ) : (
          <p className="panel-empty-state">{t("timeline.pinnedMessagesEmpty")}</p>
        )}
      </aside>
    );
  }

  if (mode === "threads") {
    return (
      <aside className="thread-pane" aria-label={t("panel.context")}>
        <PanelHeader title={t("threads.title")} onClose={onClosePanel} />
        <ThreadsListView
          threadsList={snapshot.state.ui.threads_list}
          scope={threadsListScope}
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
      <aside
        className={mode === "search" ? "thread-pane search-panel" : "thread-pane"}
        aria-label={t("panel.context")}
      >
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
              recentEmojis={composerSettings.recent_emojis}
              onRecentEmojisChange={onRecentEmojisChange}
              searchHighlightsByEventId={searchHighlightsByEventId}
              mediaDownloads={mediaDownloads}
            />
          </section>
        ) : null}
        {mode === "search" ? (
          <SearchResults
            indexingPending={searchIndexingPending}
            pending={searchPending}
            tooShortMinChars={searchTooShortMinChars}
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
  const threadDocument =
    threadComposerDocumentOverride ??
    threadComposer?.document ??
    documentFromText(threadComposer?.draft ?? "");
  const threadSendPending = Boolean(threadComposer?.pending_transaction_id);
  const threadStagedUploads = threadState.kind === "open" ? threadState.staged_uploads ?? [] : [];
  const threadMentionCandidates = selectMentionCandidates(
    { snapshot },
    threadRoomId ?? null,
    "thread"
  );
  const threadMentionCandidateTarget = snapshot.state.domain.mention_candidates.targets.find(
    (target) => target.room_id === threadRoomId && target.surface === "thread"
  );
  const threadMentionCandidatesLoading =
    threadMentionCandidateTarget?.completeness === "loading" ||
    threadMentionCandidateTarget?.completeness === "partial";
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
            presentationContext="thread"
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
            automaticBackfillEligible={
              threadState.intent === "existingThread" ||
              (typeof threadState.intent === "object" && "pinnedReply" in threadState.intent)
            }
            initialTargetEventId={
              typeof threadState.intent === "object" && "pinnedReply" in threadState.intent
                ? threadState.intent.pinnedReply.event_id
                : null
            }
            autoLoadOlderMessages={snapshot.state.domain.settings.values.timeline.auto_load_older_messages}
            codeBlockWrap={snapshot.state.domain.settings.values.display.code_block_wrap}
            recentEmojis={composerSettings.recent_emojis}
            onRecentEmojisChange={onRecentEmojisChange}
            searchHighlightsByEventId={searchHighlightsByEventId}
            mediaDownloads={mediaDownloads}
            mentionCandidates={threadMentionCandidates}
            mentionCandidatesLoading={threadMentionCandidatesLoading}
            onMentionQueryChange={onThreadMentionQueryChange}
            onDiagnosticLogEntry={onTimelineDiagnosticLogEntry}
          />
        ) : browserThreadSnapshot ? (
          <div className="message-fixture-list thread-fixture-list">
            {browserThreadSnapshot.replies.map((reply) => (
              <MessageArticle
                key={reply.event_id}
                message={threadReplyToTimelineMessage(reply)}
                highlights={
                  searchHighlightsByEventId[reply.event_id]?.snippet === reply.body
                    ? searchHighlightsByEventId[reply.event_id]?.ranges ?? []
                    : []
                }
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
      {threadStagedUploads.length > 0 && threadRoomId && rootEventId ? (
        <UploadStagingDialog
          items={threadStagedUploads}
          onClear={() => onThreadClearUploadStaging(threadRoomId, rootEventId)}
          onUpdateCaption={(stagedId, document) =>
            onThreadUpdateStagedUploadCaption(
              threadRoomId,
              rootEventId,
              stagedId,
              document
            )
          }
          onSendAttachments={() =>
            onThreadSendStagedAttachments(threadRoomId, rootEventId)
          }
          onSelectOutput={(stagedId, selection) =>
            onThreadSelectStagedUploadOutput(
              threadRoomId,
              rootEventId,
              stagedId,
              selection
            )
          }
          onRetryPreparation={(stagedId) =>
            onThreadRetryStagedUploadPreparation(threadRoomId, rootEventId, stagedId)
          }
          onUseOriginal={(stagedId) =>
            onThreadUseOriginalStagedUpload(threadRoomId, rootEventId, stagedId)
          }
          loadPreview={threadPreviewLoader}
          surface="thread"
          resolveComposerKeyAction={onResolveComposerKeyAction}
          mentionCandidates={threadMentionCandidates}
          mentionCandidatesLoading={threadMentionCandidatesLoading}
          onMentionQueryChange={(query) => {
            if (threadRoomId) onThreadMentionQueryChange(threadRoomId, query);
          }}
          mathModeEnabled={composerSettings.math_mode}
          recentEmojis={composerSettings.recent_emojis}
          onMathModeChange={(enabled) =>
            onUpdateSettings?.({
              composer: { ...composerSettings, math_mode: enabled }
            })
          }
          onRecentEmojisChange={onRecentEmojisChange}
          roomName={t("panel.thread")}
        />
      ) : null}
      <ThreadComposer
        stagedUploadsReady={uploadStagingItemsAreSendable(threadStagedUploads)}
        onSendStagedUploads={() => {
          if (threadRoomId && rootEventId) {
            onThreadSendStagedAttachments(threadRoomId, rootEventId);
          }
        }}
        notice={threadComposerNotice}
        document={threadDocument}
        draftKey={
          threadComposerDraftImeKey ??
          `${threadRoomId ?? "no-room"}\u0000${rootEventId ?? "no-root"}\u00000`
        }
        isSending={threadSendPending}
        hasStagedUploads={threadStagedUploads.length > 0}
        mathModeEnabled={composerSettings.math_mode}
        recentEmojis={composerSettings.recent_emojis}
        onMathModeChange={(enabled) =>
          onUpdateSettings?.({
            composer: { ...composerSettings, math_mode: enabled }
          })
        }
        onRecentEmojisChange={onRecentEmojisChange}
        mentionCandidates={threadMentionCandidates}
        mentionCandidatesLoading={threadMentionCandidatesLoading}
        resolveComposerKeyAction={onResolveComposerKeyAction}
        canEdit={
          !encryptedComposerBlocked &&
          threadState.kind === "open" &&
          Boolean(threadRoomId && rootEventId && threadComposer)
        }
        onDocumentChange={(document) => {
          if (threadRoomId && rootEventId) {
            onThreadComposerDocumentChange(threadRoomId, rootEventId, document);
          }
        }}
        onAttachFiles={(files) => {
          if (threadRoomId && rootEventId) {
            onThreadAttachFiles(threadRoomId, rootEventId, files);
          }
        }}
        onMentionQueryChange={(query) => {
          if (threadRoomId) {
            onThreadMentionQueryChange(threadRoomId, query);
          }
        }}
        onScheduleSend={
          onThreadScheduleSend && threadRoomId && rootEventId
            ? (sendAtMs, document) =>
                onThreadScheduleSend(threadRoomId, rootEventId, sendAtMs, document)
            : undefined
        }
        onSend={(document) => {
          if (threadRoomId && rootEventId) {
            onThreadReplySend(threadRoomId, rootEventId, document);
          }
        }}
        onDiagnosticLogEntry={onTimelineDiagnosticLogEntry}
      />
    </aside>
  );
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
      {showClose ? (
        <button className="icon-button" type="button" aria-label={t("action.close", { title })} onClick={onClose}>
          <X size={ICON_SIZE.panel} />
        </button>
      ) : null}
    </header>
  );
}
