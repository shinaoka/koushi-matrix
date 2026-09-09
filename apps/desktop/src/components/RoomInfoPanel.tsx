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
  Settings,
  Users
} from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";

import { t } from "../i18n/messages";
import { ImeSafeForm, ImeTextArea, ImeTextField } from "./ImeTextControl";
import type {
  RoomHistoryVisibility,
  InviteHistoryPolicy,
  RoomJoinRule,
  RoomManagementState,
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
  onSetRoomUrlPreviewOverride,
  onOpenPeople,
  onRepairRoomTimeline,
  onForceRotateOutboundSession,
  inviteHistoryPolicy,
  onOpenRecovery,
  onReturnToInvite
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
  onSetRoomUrlPreviewOverride?: (roomId: string, enabled: boolean) => void;
  onOpenPeople?: () => void;
  onRepairRoomTimeline?: (roomId: string) => void | Promise<void>;
  onForceRotateOutboundSession?: (roomId: string) => void | Promise<void>;
  inviteHistoryPolicy?: InviteHistoryPolicy | null;
  onOpenRecovery?: () => void;
  onReturnToInvite?: () => void;
}) {
  const roomId = room?.room_id ?? "";
  const roomName = room?.display_label ?? "";
  const isEncrypted = room?.is_encrypted ?? false;
  const globalUrlPreviewsEnabled = isEncrypted
    ? appSettings?.values.display.encrypted_url_previews_enabled ?? true
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
  const [topicDraft, setTopicDraft] = useState(settings?.topic ?? "");
  const [avatarDraft, setAvatarDraft] = useState(settings?.avatar_url ?? "");
  const [joinRuleDraft, setJoinRuleDraft] = useState<RoomJoinRule>(
    settings?.join_rule ?? "invite"
  );
  const [historyVisibilityDraft, setHistoryVisibilityDraft] =
    useState<RoomHistoryVisibility>(settings?.history_visibility ?? "shared");
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
    setTopicDraft(settings?.topic ?? "");
    setAvatarDraft(settings?.avatar_url ?? "");
    setJoinRuleDraft(settings?.join_rule ?? "invite");
    setHistoryVisibilityDraft(settings?.history_visibility ?? "shared");
  }, [
    roomId,
    roomName,
    settings?.avatar_url,
    settings?.history_visibility,
    settings?.join_rule,
    settings?.name,
    settings?.topic
  ]);

  useEffect(() => {
    rotationEpochRef.current += 1;
    setRotationConfirm(false);
    setRotationState("idle");
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

  const canEditSettings =
    Boolean(settings?.permissions.can_edit_settings) &&
    Boolean(onUpdateRoomSetting) &&
    !settingsPending;
  const statusBadges = roomStatusBadges(isEncrypted, Boolean(room?.is_dm), settings);

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
          <h2 id="room-info-title" dir="auto">{room.display_label}</h2>
          <p dir="auto">{room.room_id}</p>
        </div>
      </header>

      <div className="room-status-bar" aria-label={t("room.status")}>
        <div className="room-status-badges">
          {statusBadges.map((badge) => (
            <span className="room-status-badge" key={badge.label}>
              {badge.icon}
              <span>{badge.label}</span>
            </span>
          ))}
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
        <h3>{t("room.notifications")}</h3>
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

      <section className="settings-section room-access-history" aria-label={t("room.accessAndHistory")}>
        <h3>{t("room.accessAndHistory")}</h3>
        <p className="profile-settings-hint">{t("room.accessAndHistoryHint")}</p>
        {settings ? (
          <div className="room-management-grid">
            <div className="settings-detail-list">
              <DetailRow label={t("room.joinRule")} value={roomJoinRuleLabel(settings.join_rule)} />
              <DetailRow
                label={t("room.historyVisibility")}
                value={roomHistoryVisibilityLabel(settings.history_visibility)}
              />
            </div>
            <ImeSafeForm
              className="room-management-form"
              onSubmit={(event) => {
                event.preventDefault();
                if (canEditSettings) {
                  onUpdateRoomSetting?.(room.room_id, { joinRule: joinRuleDraft });
                }
              }}
            >
              <label className="profile-settings-field">
                <span>{t("room.joinRule")}</span>
                <select
                  value={joinRuleDraft}
                  aria-label={t("room.joinRule")}
                  disabled={!canEditSettings}
                  onChange={(event) => setJoinRuleDraft(event.currentTarget.value as RoomJoinRule)}
                >
                  {(["public", "invite", "knock", "restricted", "private"] as const).map(
                    (rule) => (
                      <option key={rule} value={rule}>
                        {roomJoinRuleLabel(rule)}
                      </option>
                    )
                  )}
                </select>
              </label>
              <button
                className="profile-settings-action"
                type="submit"
                disabled={!canEditSettings || joinRuleDraft === settings.join_rule}
              >
                {t("room.saveJoinRule")}
              </button>
            </ImeSafeForm>
            <ImeSafeForm
              className="room-management-form"
              onSubmit={(event) => {
                event.preventDefault();
                if (canEditSettings) {
                  onUpdateRoomSetting?.(room.room_id, {
                    historyVisibility: historyVisibilityDraft
                  });
                }
              }}
            >
              <label className="profile-settings-field">
                <span>{t("room.historyVisibility")}</span>
                <select
                  value={historyVisibilityDraft}
                  aria-label={t("room.historyVisibility")}
                  disabled={!canEditSettings}
                  onChange={(event) =>
                    setHistoryVisibilityDraft(event.currentTarget.value as RoomHistoryVisibility)
                  }
                >
                  {(["worldReadable", "shared", "invited", "joined"] as const).map(
                    (visibility) => (
                      <option key={visibility} value={visibility}>
                        {roomHistoryVisibilityLabel(visibility)}
                      </option>
                    )
                  )}
                </select>
              </label>
              <button
                className="profile-settings-action"
                type="submit"
                disabled={!canEditSettings || historyVisibilityDraft === settings.history_visibility}
              >
                {t("room.saveHistoryVisibility")}
              </button>
            </ImeSafeForm>
            <p className="profile-settings-hint">
              {roomHistoryVisibilityDescription(historyVisibilityDraft)}
            </p>
            {historyVisibilityDraft === "worldReadable" ? (
              <p className="settings-notice" role="note">
                <AlertTriangle size={15} aria-hidden="true" />
                {t("room.historyWorldReadableWarning")}
              </p>
            ) : null}
            {isEncrypted && historyVisibilityDraft === "shared" ? (
              <p className="settings-notice" role="note">
                <KeyRound size={15} aria-hidden="true" />
                {t("room.historySharedEncryptedHint")}
              </p>
            ) : null}
            <p className="settings-notice" role="note">
              {t("room.historyNonRetroactive")}
            </p>
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
          </div>
        ) : (
          <div className="settings-detail-row">
            <span>{t("room.settingsLoading")}</span>
          </div>
        )}
      </section>

      <section className="settings-section" aria-label={t("room.management")}>
        <h3>{t("room.management")}</h3>
        {settings ? (
          <div className="room-management-grid">
            <div className="settings-detail-list">
              <DetailRow
                label={t("room.currentTopic")}
                value={settings.topic?.trim() || t("room.noTopic")}
              />
              <DetailRow
                label={t("room.currentAvatar")}
                value={settings.avatar_url?.trim() || t("room.noAvatar")}
              />
            </div>
            <ImeSafeForm
              className="room-management-form"
              onSubmit={(event) => {
                event.preventDefault();
                if (canEditSettings) {
                  onUpdateRoomSetting?.(room.room_id, {
                    name: nameDraft.trim() || null
                  });
                }
              }}
            >
              <label className="profile-settings-field">
                <span>{t("dialog.roomName")}</span>
                <ImeTextField
                  value={nameDraft}
                  syncKey={`${roomId}:name`}
                  aria-label={t("dialog.roomName")}
                  disabled={!canEditSettings}
                  onChange={(event) => setNameDraft(event.currentTarget.value)}
                />
              </label>
              <button
                className="profile-settings-action"
                type="submit"
                disabled={!canEditSettings || nameDraft.trim() === (settings.name ?? "")}
              >
                {t("room.saveName")}
              </button>
            </ImeSafeForm>
            <ImeSafeForm
              className="room-management-form"
              onSubmit={(event) => {
                event.preventDefault();
                if (canEditSettings) {
                  onUpdateRoomSetting?.(room.room_id, {
                    avatarUrl: avatarDraft.trim() || null
                  });
                }
              }}
            >
              <label className="profile-settings-field">
                <span>{t("room.avatarUrl")}</span>
                <ImeTextField
                  value={avatarDraft}
                  syncKey={`${roomId}:avatar`}
                  aria-label={t("room.avatarUrl")}
                  disabled={!canEditSettings}
                  onChange={(event) => setAvatarDraft(event.currentTarget.value)}
                />
              </label>
              <button
                className="profile-settings-action"
                type="submit"
                disabled={!canEditSettings || avatarDraft.trim() === (settings.avatar_url ?? "")}
              >
                {t("room.saveAvatar")}
              </button>
            </ImeSafeForm>
            <ImeSafeForm
              className="room-management-form"
              onSubmit={(event) => {
                event.preventDefault();
                if (canEditSettings) {
                  onUpdateRoomSetting?.(room.room_id, {
                    topic: topicDraft.trim() || null
                  });
                }
              }}
            >
              <label className="profile-settings-field">
                <span>{t("room.topic")}</span>
                <ImeTextArea
                  value={topicDraft}
                  syncKey={`${roomId}:topic`}
                  aria-label={t("room.topic")}
                  disabled={!canEditSettings}
                  onChange={(event) => setTopicDraft(event.currentTarget.value)}
                />
              </label>
              <button
                className="profile-settings-action"
                type="submit"
                disabled={!canEditSettings || topicDraft.trim() === (settings.topic ?? "")}
              >
                {t("room.saveTopic")}
              </button>
            </ImeSafeForm>
            {operation.kind === "failed" ? (
              <div className="room-management-status" role="status">
                {t("room.operationFailed")}
              </div>
            ) : null}
          </div>
        ) : (
          <div className="settings-detail-row">
            <span>{t("room.settingsLoading")}</span>
          </div>
        )}
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

      <SettingsEntryList
        entries={[
          { icon: <Users size={16} />, label: t("room.invitePeople"), onClick: onInvitePeople },
          {
            icon: <Users size={16} />,
            label: t("room.people"),
            onClick: onOpenPeople
          },
          { icon: <FileText size={16} />, label: t("room.files"), onClick: onOpenFiles },
          { icon: <Bell size={16} />, label: t("room.notifications") },
          { icon: <Settings size={16} />, label: t("room.roomSettings") }
        ]}
      />
    </section>
  );
}

function roomStatusBadges(
  isEncrypted: boolean,
  isDm: boolean,
  settings: RoomManagementState["settings"]
): Array<{ label: string; icon: ReactNode }> {
  const badges: Array<{ label: string; icon: ReactNode }> = [
    {
      label: isEncrypted ? t("room.statusEncrypted") : t("room.statusNotEncrypted"),
      icon: isEncrypted ? (
        <Lock size={14} aria-hidden="true" />
      ) : (
        <LockOpen size={14} aria-hidden="true" />
      )
    }
  ];

  if (settings && !isDm) {
    badges.push({
      label:
        settings.join_rule === "public"
          ? t("room.statusPublic")
          : t("room.statusPrivate"),
      icon: <Globe2 size={14} aria-hidden="true" />
    });
    badges.push({
      label: roomHistoryStatusLabel(settings.history_visibility),
      icon: <History size={14} aria-hidden="true" />
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
    case "private":
      return t("room.joinRulePrivate");
  }
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
