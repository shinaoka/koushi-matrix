import {
  AlertTriangle,
  ArrowLeft,
  Bell,
  ChevronRight,
  Copy,
  FileText,
  Globe2,
  History,
  KeyRound,
  Link,
  Lock,
  LockOpen,
  Users
} from "lucide-react";
import { Fragment, useEffect, useRef, useState, type ReactNode } from "react";

import { t } from "../i18n/messages";
import { ImeSafeForm, ImeTextField } from "./ImeTextControl";
import {
  InlineChoicePropertyEditor,
  InlineTextPropertyEditor,
  type PropertySaveStatus
} from "./SettingsPropertyCard";
import { EntityAvatar } from "./Shell";
import { avatarInitial, roomAccessTooltipLabel, roomDisplayLabel } from "../app/uiShared";
import {
  ROOM_ACCESS_CHECKING,
  roomAccessHeaderBadges,
  roomAccessIndicator,
  type RoomAccessProjection
} from "../domain/accessCondition";
import { Tooltip, type TooltipTriggerProps } from "./Tooltip";
import {
  HistoryExportSection,
  type HistoryExportControls
} from "./HistoryExportDialog";
import type {
  RoomHistoryVisibility,
  HistoryExportState,
  InviteHistoryPolicy,
  RoomJoinRule,
  RoomManagementState,
  OperationFailureKind,
  RoomNotificationMode,
  RoomNotificationSettings,
  RoomSettingChange,
  RoomSummary,
  LinkPreviewSettingsState,
  SettingsState,
  SpaceSummary
} from "../domain/types";

export function RoomInfoPanel({
  room,
  roomManagement,
  roomNotificationSettings,
  appSettings,
  linkPreviewSettings,
  spaces,
  onInvitePeople,
  onOpenFiles,
  onSetRoomNotificationMode,
  onUpdateRoomSetting,
  access,
  onSetRoomUrlPreviewOverride,
  onOpenPeople,
  onRepairRoomTimeline,
  onForceRotateOutboundSession,
  inviteHistoryPolicy,
  onOpenRecovery,
  onReturnToInvite,
  historyExport,
  historyExportControls
}: {
  room: RoomSummary | null;
  roomManagement?: RoomManagementState;
  roomNotificationSettings: RoomNotificationSettings | undefined;
  appSettings?: SettingsState;
  linkPreviewSettings?: LinkPreviewSettingsState;
  spaces: SpaceSummary[];
  onInvitePeople?: () => void;
  onOpenFiles?: () => void;
  onSetRoomNotificationMode?: (roomId: string, mode: RoomNotificationMode) => void;
  onUpdateRoomSetting?: (roomId: string, change: RoomSettingChange) => void;
  /** The room's projected access facts from the shared sidebar contract (#1220). */
  access?: RoomAccessProjection | null;
  onSetRoomUrlPreviewOverride?: (roomId: string, enabled: boolean) => void;
  onOpenPeople?: () => void;
  onRepairRoomTimeline?: (roomId: string) => void | Promise<void>;
  onForceRotateOutboundSession?: (roomId: string) => void | Promise<void>;
  inviteHistoryPolicy?: InviteHistoryPolicy | null;
  onOpenRecovery?: () => void;
  onReturnToInvite?: () => void;
  historyExport?: HistoryExportState;
  historyExportControls?: HistoryExportControls;
}) {
  const roomId = room?.room_id ?? "";
  const roomName = room ? roomDisplayLabel(room) : "";
  const isEncrypted = room?.is_encrypted ?? false;
  const globalUrlPreviewsEnabled = isEncrypted
    ? appSettings?.values.display.encrypted_url_previews_enabled ?? false
    : appSettings?.values.display.url_previews_enabled ?? true;
  const roomOverride = linkPreviewSettings?.room_overrides[roomId];
  const roomUrlPreviewsEnabled = roomOverride ?? globalUrlPreviewsEnabled;
  const parentSpaces = room
    ? spaces.filter((space) => room.parent_space_ids.includes(space.space_id))
    : [];
  const managementForRoom =
    roomManagement?.selected_room_id === roomId ? roomManagement : null;
  const settings = managementForRoom?.settings ?? null;
  const shareLink = settings?.share_link?.trim() || null;
  const operation = managementForRoom?.operation ?? { kind: "idle" as const };
  const settingsPending = operation.kind === "pending" && operation.operation === "settings";
  const permissions = settings?.permissions ?? null;
  const historyPolicy = inviteHistoryPolicy ?? {
    current_visibility: settings?.history_visibility ?? "joined",
    encrypted: isEncrypted,
    can_edit: Boolean(settings?.permissions.can_edit_settings),
    readiness: "ready" as const
  };
  const [nameDraft, setNameDraft] = useState(settings?.name ?? roomName);
  // Issue #1008: which settings change is this panel's own, so Rust's pending
  // or failed operation is shown on that property rather than in a shared
  // footer. Reset per room: a result never carries across to another room.
  const [submission, setSubmission] = useState<{
    roomId: string;
    field: RoomSettingField;
    target: string | null;
    /** A failure already on screen when this submission started is not its outcome. */
    priorFailureRequestId: number | null;
  } | null>(null);
  const joinRuleHeadingRef = useRef<HTMLHeadingElement>(null);
  const historyHeadingRef = useRef<HTMLHeadingElement>(null);
  const notificationsHeadingRef = useRef<HTMLHeadingElement>(null);
  const [rotationConfirm, setRotationConfirm] = useState(false);
  const [rotationState, setRotationState] = useState<"idle" | "pending" | "completed" | "failed">(
    "idle"
  );
  const rotationEpochRef = useRef(0);
  const copyEpochRef = useRef(0);
  const [copyStatus, setCopyStatus] = useState<"copied" | "failed" | null>(null);
  useEffect(() => {
    setCopyStatus(null);
    copyEpochRef.current += 1;
    return () => { copyEpochRef.current += 1; };
  }, [roomId, shareLink]);

  useEffect(() => {
    setNameDraft(settings?.name ?? roomName);
  }, [roomId, roomName, settings?.name]);

  useEffect(() => {
    rotationEpochRef.current += 1;
    setRotationConfirm(false);
    setRotationState("idle");
    setSubmission(null);
  }, [roomId]);

  async function forceRotation() {
    const epoch = ++rotationEpochRef.current;
    setRotationConfirm(false);
    setRotationState("pending");
    try {
      await onForceRotateOutboundSession?.(roomId);
      if (rotationEpochRef.current === epoch) setRotationState("completed");
    } catch {
      if (rotationEpochRef.current === epoch) setRotationState("failed");
    }
  }

  function repairRoomTimeline() {
    if (!onRepairRoomTimeline) {
      return;
    }
    void onRepairRoomTimeline(roomId);
  }

  const mayEditSettings =
    Boolean(settings?.permissions.can_edit_settings) && Boolean(onUpdateRoomSetting);
  const canEditSettings = mayEditSettings && !settingsPending;
  const readOnlyReason =
    settings && !settings.permissions.can_edit_settings ? t("room.settingNoPermission") : null;
  // #1220: the join-rule control is governed by the event-specific fact, not the
  // aggregate; an account that may change join rules but not rename the room can
  // still hold it. Rust enforces the same single authority before it sends.
  const mayChangeJoinRule =
    Boolean(settings?.permissions.can_change_join_rule) && Boolean(onUpdateRoomSetting);
  const readOnlyJoinRuleReason =
    settings && !settings.permissions.can_change_join_rule
      ? t("room.settingNoPermission")
      : null;
  const statusBadges = roomStatusBadges(isEncrypted, settings, access);

  function submitSetting(field: RoomSettingField, change: RoomSettingChange, target: string | null) {
    const allowed = "joinRule" in change ? mayChangeJoinRule : canEditSettings;
    if (!allowed) return;
    setSubmission({
      roomId,
      field,
      target,
      priorFailureRequestId: operation.kind === "failed" ? operation.request_id : null
    });
    onUpdateRoomSetting?.(roomId, change);
  }

  function confirmedValue(field: RoomSettingField): string | null {
    switch (field) {
      case "name":
        return settings?.name ?? null;
      case "topic":
        return settings?.topic ?? null;
      case "avatar":
        return settings?.avatar_url ?? null;
      case "joinRule":
        return settings?.join_rule ?? null;
      case "historyVisibility":
        return settings?.history_visibility ?? null;
    }
  }

  function fieldStatus(field: RoomSettingField): PropertySaveStatus {
    if (!submission || submission.roomId !== roomId || submission.field !== field) return null;
    const ownOperation =
      operation.kind !== "idle" && operation.operation === "settings" && operation.room_id === roomId;
    if (ownOperation && operation.kind === "pending") return { kind: "saving" };
    if (
      ownOperation &&
      operation.kind === "failed" &&
      operation.request_id !== submission.priorFailureRequestId
    ) {
      return { kind: "failed", message: roomSettingFailureMessage(operation.failureKind) };
    }
    // Saved only once Rust's snapshot carries the submitted value.
    return (confirmedValue(field)?.trim() || null) === submission.target
      ? { kind: "saved" }
      : null;
  }

  function revealSetting(heading: HTMLHeadingElement | null) {
    heading?.scrollIntoView?.({ block: "nearest" });
    heading?.focus();
  }

  function renderStatusBadge(badge: StatusBadge, triggerProps?: TooltipTriggerProps) {
    if (!badge.setting) {
      return (
        <span className="room-status-badge" {...triggerProps}>
          {badge.icon}
          <span>{badge.label}</span>
        </span>
      );
    }
    return (
      <button
        className="room-status-badge room-status-badge-link"
        type="button"
        aria-label={t("room.statusShowSetting", { status: badge.label })}
        onClick={() =>
          revealSetting(
            badge.setting === "joinRule"
              ? joinRuleHeadingRef.current
              : historyHeadingRef.current
          )
        }
        {...triggerProps}
      >
        {badge.icon}
        <span>{badge.label}</span>
      </button>
    );
  }

  const nameStatus = fieldStatus("name");

  async function copyShareLink() {
    if (!shareLink) return;
    const epoch = ++copyEpochRef.current;
    try {
      await navigator.clipboard.writeText(shareLink);
      if (epoch === copyEpochRef.current) setCopyStatus("copied");
    } catch {
      if (epoch === copyEpochRef.current) setCopyStatus("failed");
    }
  }

  if (!room) {
    return (
      <section className="settings-panel" aria-labelledby="room-info-title">
        <header className="settings-panel-header">
          <div>
            <h2 id="room-info-title">{t("room.roomInfo")}</h2>
            <p>{t("room.noRoomSelected")}</p>
          </div>
        </header>
      </section>
    );
  }

  return (
    <section className="settings-panel room-info-panel" aria-labelledby="room-info-title">
      <header className="settings-panel-header">
        <div>
          <h2 id="room-info-title" className="sr-only" dir="auto">
            {roomDisplayLabel(room)}
          </h2>
          <ImeSafeForm
            className="room-name-header-form"
            aria-label={t("dialog.roomName")}
            onSubmit={(event) => {
              event.preventDefault();
              const name = nameDraft.trim() || null;
              submitSetting("name", { name }, name);
            }}
          >
            <label className="room-name-header-field">
              <span className="sr-only">{t("dialog.roomName")}</span>
              <ImeTextField
                value={nameDraft}
                syncKey={`${roomId}:name`}
                aria-label={t("dialog.roomName")}
                dir="auto"
                disabled={!canEditSettings}
                onChange={(event) => setNameDraft(event.currentTarget.value)}
              />
            </label>
            <button
              className="profile-settings-action"
              type="submit"
              aria-label={t("room.saveName")}
              disabled={!canEditSettings || nameDraft.trim() === (settings?.name ?? roomName)}
            >
              {t("settings.propertySave")}
            </button>
          </ImeSafeForm>
          {nameStatus ? (
            <p
              className={
                nameStatus.kind === "failed"
                  ? "settings-property-status settings-property-status-failed"
                  : "settings-property-status"
              }
              role="status"
            >
              {nameStatus.kind === "saving"
                ? t("settings.propertySaving")
                : nameStatus.kind === "saved"
                  ? t("settings.propertySaved")
                  : nameStatus.message}
            </p>
          ) : null}
          <p dir="auto">{room.room_id}</p>
        </div>
      </header>

      <div className="room-status-bar" aria-label={t("room.status")}>
        <div className="room-status-badges">
          {statusBadges.map((badge) =>
            // #1220: a summary badge with its own route explanation explains it
            // on hover and on keyboard focus, while the badge still navigates
            // to the property it summarizes.
            badge.description ? (
              <Tooltip key={badge.label} label={badge.description}>
                {(triggerProps) => renderStatusBadge(badge, triggerProps)}
              </Tooltip>
            ) : (
              <Fragment key={badge.label}>{renderStatusBadge(badge)}</Fragment>
            )
          )}
        </div>
        {shareLink ? (
          <button className="room-share-link-button" type="button" onClick={copyShareLink}>
            <Copy size={14} aria-hidden="true" />
            <span>{t("room.copyShareLink")}</span>
          </button>
        ) : null}
      </div>

      {shareLink ? (
        <div className="settings-detail-list">
          {settings?.canonical_alias ? <DetailRow label={t("dialog.roomAddress")} value={settings.canonical_alias} /> : null}
          <DetailRow label={t("room.shareUrl")} value={shareLink} />
          {copyStatus ? <p role="status">{t(copyStatus === "copied" ? "room.shareLinkCopied" : "room.shareLinkCopyFailed")}</p> : null}
        </div>
      ) : null}

      <div className="settings-summary-grid" aria-label={t("room.summary")}>
        <SummaryTile label={t("room.type")} value={room.is_dm ? t("room.directMessage") : t("search.scopeRoom")} />
        <SummaryTile label={t("room.unread")} value={String(room.unread_count)} />
        <SummaryTile label={t("room.spaces")} value={parentSpaces.length ? String(parentSpaces.length) : t("room.noSpaces")} />
      </div>

      <section className="settings-section" aria-label={t("room.details")}>
        <h3>{t("room.details")}</h3>
        {settings ? (
          <>
            <InlineTextPropertyEditor
              key={`${roomId}:topic`}
              property="topic"
              label={t("room.topicLabel")}
              value={settings.topic ?? ""}
              emptyText={t("room.noTopic")}
              display={<span className="settings-property-multiline">{settings.topic?.trim()}</span>}
              inputLabel={t("room.topic")}
              editLabel={t("room.editTopic")}
              saveLabel={t("room.saveTopic")}
              multiline
              syncKey={`${roomId}:topic`}
              canEdit={mayEditSettings}
              busy={settingsPending}
              readOnlyReason={readOnlyReason}
              status={fieldStatus("topic")}
              onSave={(next) => submitSetting("topic", { topic: next || null }, next || null)}
            />
            <InlineTextPropertyEditor
              key={`${roomId}:avatar`}
              property="avatar"
              label={t("room.avatar")}
              value={settings.avatar_url ?? ""}
              emptyText={t("room.noAvatar")}
              userText={false}
              display={
                <span className="settings-property-avatar-row">
                  <EntityAvatar
                    avatar={room.avatar}
                    className="settings-property-avatar"
                    colorSeed={room.room_id}
                    fallback={avatarInitial(roomDisplayLabel(room))}
                  />
                  <small className="settings-property-secondary" dir="ltr">
                    {settings.avatar_url?.trim()}
                  </small>
                </span>
              }
              inputLabel={t("room.avatarUrl")}
              editLabel={t("room.editAvatar")}
              saveLabel={t("room.saveAvatar")}
              syncKey={`${roomId}:avatar`}
              canEdit={mayEditSettings}
              busy={settingsPending}
              readOnlyReason={readOnlyReason}
              status={fieldStatus("avatar")}
              onSave={(next) => submitSetting("avatar", { avatarUrl: next || null }, next || null)}
            />
          </>
        ) : (
          <div className="settings-detail-row">
            <span>{t("room.settingsLoading")}</span>
          </div>
        )}
      </section>

      <section className="settings-section room-access-history" aria-label={t("room.accessAndHistory")}>
        <h3>{t("room.accessAndHistory")}</h3>
        <p className="profile-settings-hint">{t("room.accessAndHistoryHint")}</p>
        {settings ? (
          <>
            <InlineChoicePropertyEditor<RoomJoinRule>
              key={`${roomId}:join-rule`}
              property="join-rule"
              label={t("room.joinRule")}
              headingRef={joinRuleHeadingRef}
              value={settings.join_rule}
              valueLabel={roomJoinRuleLabel}
              options={joinRuleOptions(settings.join_rule).map((rule) => ({
                value: rule,
                disabled: !SETTABLE_JOIN_RULES.includes(rule)
              }))}
              selectLabel={t("room.joinRule")}
              changeLabel={t("room.changeJoinRule")}
              saveLabel={t("room.saveJoinRule")}
              canEdit={mayChangeJoinRule}
              busy={settingsPending}
              readOnlyReason={readOnlyJoinRuleReason}
              status={fieldStatus("joinRule")}
              onSave={(joinRule) => submitSetting("joinRule", { joinRule }, joinRule)}
            />
            <InlineChoicePropertyEditor<RoomHistoryVisibility>
              key={`${roomId}:history-visibility`}
              property="history-visibility"
              label={t("room.historyVisibility")}
              headingRef={historyHeadingRef}
              value={settings.history_visibility}
              valueLabel={roomHistoryVisibilityLabel}
              options={HISTORY_VISIBILITY_OPTIONS.map((visibility) => ({ value: visibility }))}
              selectLabel={t("room.historyVisibility")}
              changeLabel={t("room.changeHistoryVisibility")}
              saveLabel={t("room.saveHistoryVisibility")}
              canEdit={mayEditSettings}
              busy={settingsPending}
              readOnlyReason={readOnlyReason}
              status={fieldStatus("historyVisibility")}
              notes={(visibility) => (
                <>
                  <p className="profile-settings-hint">
                    {roomHistoryVisibilityDescription(visibility)}
                  </p>
                  {visibility === "worldReadable" ? (
                    <p className="settings-notice" role="note">
                      <AlertTriangle size={15} aria-hidden="true" />
                      {t("room.historyWorldReadableWarning")}
                    </p>
                  ) : null}
                  {isEncrypted && visibility === "shared" ? (
                    <p className="settings-notice" role="note">
                      <KeyRound size={15} aria-hidden="true" />
                      {t("room.historySharedEncryptedHint")}
                    </p>
                  ) : null}
                  <p className="settings-notice" role="note">
                    {t("room.historyNonRetroactive")}
                  </p>
                </>
              )}
              onSave={(historyVisibility) =>
                submitSetting("historyVisibility", { historyVisibility }, historyVisibility)
              }
            />
            {historyPolicy.readiness === "recoveryRequired" ? (
              <div className="settings-notice" role="alert">
                <AlertTriangle size={15} aria-hidden="true" />
                <span>{t("room.historyRecoveryRequired")}</span>
                {onOpenRecovery ? (
                  <button className="inline-link-button" type="button" onClick={onOpenRecovery}>
                    {t("settings.openRecovery")}
                  </button>
                ) : null}
              </div>
            ) : null}
            {onReturnToInvite ? (
              <button className="profile-settings-action" type="button" onClick={onReturnToInvite}>
                <ArrowLeft size={15} aria-hidden="true" />
                {t("room.returnToInvite")}
              </button>
            ) : null}
          </>
        ) : (
          <div className="settings-detail-row">
            <span>{t("room.settingsLoading")}</span>
          </div>
        )}
      </section>

      <section className="settings-section" aria-label={t("room.spaces")}>
        <h3>{t("room.spaces")}</h3>
        <div className="settings-detail-list">
          {parentSpaces.length ? (
            parentSpaces.map((space) => (
              <div className="settings-detail-row" key={space.space_id}>
                <span dir="auto">{space.display_name}</span>
                <small dir="auto">{space.space_id}</small>
              </div>
            ))
          ) : (
            <div className="settings-detail-row">
              <span>{t("room.noSpaces")}</span>
            </div>
          )}
        </div>
      </section>

      <section className="settings-section" aria-label={t("room.roomSettings")}>
        <h3>{t("room.roomSettings")}</h3>
        <div className="settings-detail-list">
          <DetailRow label={t("room.timeline")} value={t("room.subscribed")} />
          <DetailRow label={t("room.searchIndex")} value={t("room.exactVerifiedResults")} />
          <DetailRow label={t("room.dmList")} value={room.is_dm ? t("room.globalDmList") : t("room.roomScoped")} />
        </div>
      </section>

      {appSettings && linkPreviewSettings && onSetRoomUrlPreviewOverride ? (
        <section className="settings-section" aria-label={t("settings.urlPreviews")}>
          <h3>{t("settings.urlPreviews")}</h3>
          <button
            className="settings-toggle-row"
            type="button"
            role="switch"
            aria-checked={roomUrlPreviewsEnabled}
            onClick={() => {
              onSetRoomUrlPreviewOverride(roomId, !roomUrlPreviewsEnabled);
            }}
          >
            <span className="settings-toggle-copy">
              <span className="settings-toggle-label">
                <Link size={15} aria-hidden="true" />
                <span>{t("settings.urlPreviewsEnabledForRoom")}</span>
              </span>
            </span>
            <span className="settings-switch-track" aria-hidden="true">
              <span className="settings-switch-thumb" />
            </span>
          </button>
          {isEncrypted ? (
            <p className="settings-notice" role="note">
              {t("settings.urlPreviewsEncryptedNotice")}
            </p>
          ) : null}
        </section>
      ) : null}

      <section className="settings-section" aria-label={t("room.notifications")}>
        <h3 ref={notificationsHeadingRef} tabIndex={-1}>{t("room.notifications")}</h3>
        <div className="settings-detail-list">
          <label className="settings-select-row" htmlFor={`room-notification-${roomId}`}>
            <span>{t("room.notifications")}</span>
            <select
              id={`room-notification-${roomId}`}
              value={roomNotificationSettings?.mode.kind ?? "all"}
              onChange={(event) =>
                onSetRoomNotificationMode?.(roomId, {
                  kind: event.target.value as RoomNotificationMode["kind"]
                })
              }
              disabled={
                !onSetRoomNotificationMode ||
                roomNotificationSettings?.operation.kind === "pending"
              }
            >
              <option value="all">{t("room.notifyModeAll")}</option>
              <option value="mentions">{t("room.notifyModeMentions")}</option>
              <option value="mute">{t("room.notifyModeMute")}</option>
            </select>
          </label>
        </div>
      </section>

      <section className="settings-section" aria-label={t("room.rolePermissions")}>
        <h3>{t("room.rolePermissions")}</h3>
        <div className="settings-detail-list">
          <DetailRow
            label={t("room.editSettings")}
            value={permissions?.can_edit_settings ? t("settings.current") : t("auth.notChecked")}
          />
          <DetailRow
            label={t("room.editRoles")}
            value={permissions?.can_edit_roles ? t("settings.current") : t("auth.notChecked")}
          />
          <DetailRow
            label={t("room.kick")}
            value={permissions?.can_kick ? t("settings.current") : t("auth.notChecked")}
          />
          <DetailRow
            label={t("room.ban")}
            value={permissions?.can_ban ? t("settings.current") : t("auth.notChecked")}
          />
          <DetailRow
            label={t("room.unban")}
            value={permissions?.can_unban ? t("settings.current") : t("auth.notChecked")}
          />
        </div>
      </section>

      {/*
        Issue #1008: auxiliary actions — download, repair and diagnostics —
        follow the room's properties instead of splitting them.
      */}
      {historyExport && historyExportControls ? (
        <HistoryExportSection
          key={roomId}
          target={{ kind: "room", roomId, name: roomDisplayLabel(room), encrypted: room.is_encrypted }}
          exportState={historyExport}
          controls={historyExportControls}
        />
      ) : null}

      {onRepairRoomTimeline ? (
        <section className="settings-section" aria-label={t("room.repair")}>
          <h3>{t("room.repair")}</h3>
          <div className="room-key-actions">
            <button
              className="profile-settings-action"
              type="button"
              onClick={repairRoomTimeline}
            >
              <History size={16} aria-hidden="true" />
              <span>{t("room.repairTimeline")}</span>
            </button>
            <p className="profile-settings-hint">{t("room.repairTimelineHint")}</p>
          </div>
        </section>
      ) : null}

      {isEncrypted && onForceRotateOutboundSession ? (
        <section className="settings-section" aria-label={t("room.encryptionDebugging")}>
          <h3>{t("room.encryptionDebugging")}</h3>
          <div className="room-key-actions">
            <button
              className="profile-settings-action"
              type="button"
              disabled={rotationState === "pending"}
              onClick={() => setRotationConfirm(true)}
            >
              <KeyRound size={16} aria-hidden="true" />
              <span>{t("room.forceEncryptionKeyRotation")}</span>
            </button>
            <p className="profile-settings-hint">{t("room.forceEncryptionKeyRotationHint")}</p>
            {rotationConfirm ? (
              <div className="settings-detail-row">
                <p className="profile-settings-hint">{t("room.forceEncryptionKeyRotationConfirm")}</p>
                <button
                  className="profile-settings-action"
                  type="button"
                  disabled={rotationState === "pending"}
                  onClick={() => void forceRotation()}
                >
                  {t("room.confirmRotation")}
                </button>
                <button
                  className="profile-settings-action"
                  type="button"
                  onClick={() => setRotationConfirm(false)}
                >
                  {t("action.cancel")}
                </button>
              </div>
            ) : null}
            {rotationState === "completed" ? (
              <p className="profile-settings-hint success">{t("room.rotationDiscardCompleted")}</p>
            ) : rotationState === "failed" ? (
              <p className="profile-settings-hint error">{t("room.rotationDiscardFailed")}</p>
            ) : null}
          </div>
        </section>
      ) : null}

      <SettingsEntryList
        entries={[
          { icon: <Users size={16} />, label: t("room.invitePeople"), onClick: onInvitePeople },
          {
            icon: <Users size={16} />,
            label: t("room.people"),
            onClick: onOpenPeople
          },
          { icon: <FileText size={16} />, label: t("room.files"), onClick: onOpenFiles },
          // Issue #1008: the notification setting is on this panel; the entry
          // leads to it rather than being a dead end.
          {
            icon: <Bell size={16} />,
            label: t("room.notifications"),
            onClick: () => revealSetting(notificationsHeadingRef.current)
          }
        ]}
      />
    </section>
  );
}

type RoomSettingField = "name" | "topic" | "avatar" | "joinRule" | "historyVisibility";

interface StatusBadge {
  label: string;
  icon: ReactNode;
  /** The setting this badge summarizes, when it is changed on this panel. */
  setting?: "joinRule" | "historyVisibility";
  /** The route explanation this badge shows on hover and keyboard focus. */
  description?: string;
}

const HISTORY_VISIBILITY_OPTIONS: readonly RoomHistoryVisibility[] = [
  "worldReadable",
  "shared",
  "invited",
  "joined"
];

function roomSettingFailureMessage(kind: OperationFailureKind): string {
  return kind === "forbidden" ? t("room.settingForbidden") : t("room.operationFailed");
}

function roomStatusBadges(
  isEncrypted: boolean,
  settings: RoomManagementState["settings"],
  access: RoomAccessProjection | null | undefined
): StatusBadge[] {
  const badges: StatusBadge[] = [
    {
      label: isEncrypted ? t("room.statusEncrypted") : t("room.statusNotEncrypted"),
      icon: isEncrypted ? (
        <Lock size={14} aria-hidden="true" />
      ) : (
        <LockOpen size={14} aria-hidden="true" />
      )
    }
  ];

  if (settings) {
    // #1220: the summary is the same people-facing condition the row and header
    // render, never a binary Public/Private that reads a restricted rule as
    // `private`. It is shown for DMs too, which have their own access condition.
    //
    // A supplied access projection is authoritative, including an explicit
    // `joinRule: null`: Room Info then shows the shared checking indicator
    // instead of the settings snapshot's defaulted `invite`. Falling back to
    // the snapshot is reserved for a caller that has no access projection at
    // all.
    const indicator = access
      ? roomAccessIndicator(access.joinRule, access.restricted, {
          spaceMembersRoute: access.spaceMembersRoute,
          allowedRoomNames: access.allowedRoomNames
        }) ?? ROOM_ACCESS_CHECKING
      : roomAccessIndicator(settings.join_rule);
    if (indicator) {
      // One badge per shared route label, each with its own explanation and any
      // Space-name substitution, so a knock-restricted room keeps both the
      // membership and the request route here as it does in the row and header.
      for (const badge of roomAccessHeaderBadges(indicator)) {
        badges.push({
          label: t(badge.labelMessageId),
          description: roomAccessTooltipLabel(
            badge.descriptionMessageId,
            badge.descriptionAllowedRoomNames,
            badge.descriptionSpaceName
          ),
          icon: <Globe2 size={14} aria-hidden="true" />,
          setting: "joinRule"
        });
      }
    } else {
      badges.push({
        label: roomJoinRuleLabel(settings.join_rule),
        icon: <Globe2 size={14} aria-hidden="true" />,
        setting: "joinRule"
      });
    }
    badges.push({
      label: roomHistoryStatusLabel(settings.history_visibility),
      icon: <History size={14} aria-hidden="true" />,
      setting: "historyVisibility"
    });
  }

  return badges;
}

function roomHistoryStatusLabel(visibility: RoomHistoryVisibility): string {
  switch (visibility) {
    case "worldReadable":
      return t("room.statusHistoryWorldReadable");
    case "shared":
      return t("room.statusHistoryShared");
    case "invited":
    case "joined":
      return t("room.statusHistoryLimited");
  }
}

function roomJoinRuleLabel(rule: RoomJoinRule): string {
  switch (rule) {
    case "public":
      return t("room.joinRulePublic");
    case "invite":
      return t("room.joinRuleInvite");
    case "knock":
      return t("room.joinRuleKnock");
    case "restricted":
      return t("room.joinRuleRestricted");
    case "knockRestricted":
      return t("room.joinRuleKnockRestricted");
    case "private":
      return t("room.joinRulePrivate");
    case "unknown":
      return t("room.joinRuleUnknown");
  }
}

/** The rules a join-rule change can carry; mirrors Rust `RoomJoinRule::is_settable`. */
const SETTABLE_JOIN_RULES: readonly RoomJoinRule[] = ["public", "invite", "knock", "private"];

/**
 * The settable rules, plus the current one when it is not settable, so the
 * select shows the room's real rule instead of silently landing on another.
 */
function joinRuleOptions(current: RoomJoinRule): readonly RoomJoinRule[] {
  return SETTABLE_JOIN_RULES.includes(current)
    ? SETTABLE_JOIN_RULES
    : [...SETTABLE_JOIN_RULES, current];
}

function roomHistoryVisibilityLabel(visibility: RoomHistoryVisibility): string {
  switch (visibility) {
    case "worldReadable":
      return t("room.historyWorldReadable");
    case "shared":
      return t("room.historyShared");
    case "invited":
      return t("room.historyInvited");
    case "joined":
      return t("room.historyJoined");
  }
}

function roomHistoryVisibilityDescription(visibility: RoomHistoryVisibility): string {
  switch (visibility) {
    case "worldReadable":
      return t("room.historyWorldReadableDescription");
    case "shared":
      return t("room.historySharedDescription");
    case "invited":
      return t("room.historyInvitedDescription");
    case "joined":
      return t("room.historyJoinedDescription");
  }
}

function DetailRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="settings-detail-row">
      <span>{label}</span>
      <small>{value}</small>
    </div>
  );
}

function SummaryTile({ label, value }: { label: string; value: string }) {
  return (
    <div className="settings-summary-tile">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function SettingsEntryList({
  entries
}: {
  entries: Array<{ icon: ReactNode; label: string; onClick?: () => void }>;
}) {
  return (
    <div className="settings-list">
      {entries.map((entry) => (
        <button
          className="settings-list-item"
          key={entry.label}
          type="button"
          disabled={!entry.onClick}
          onClick={entry.onClick}
        >
          <span className="settings-list-label">
            <span className="settings-list-icon" aria-hidden="true">
              {entry.icon}
            </span>
            <span>{entry.label}</span>
          </span>
          <ChevronRight size={14} />
        </button>
      ))}
    </div>
  );
}
