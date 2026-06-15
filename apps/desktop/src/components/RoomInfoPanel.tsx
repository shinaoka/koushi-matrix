import { Bell, ChevronRight, FileText, Settings, Users } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";

import { t } from "../i18n/messages";
import type {
  RoomHistoryVisibility,
  RoomJoinRule,
  RoomManagementState,
  RoomMemberSummary,
  RoomModerationAction,
  RoomSettingChange,
  RoomSummary,
  SpaceSummary
} from "../domain/types";

export function RoomInfoPanel({
  currentUserId = null,
  room,
  roomManagement,
  spaces,
  onInvitePeople,
  onModerateMember,
  onUpdateRoomSetting
}: {
  currentUserId?: string | null;
  room: RoomSummary | null;
  roomManagement?: RoomManagementState;
  spaces: SpaceSummary[];
  onInvitePeople?: () => void;
  onModerateMember?: (
    roomId: string,
    targetUserId: string,
    action: RoomModerationAction,
    reason: string | null
  ) => void;
  onUpdateRoomSetting?: (roomId: string, change: RoomSettingChange) => void;
}) {
  const roomId = room?.room_id ?? "";
  const roomName = room?.display_name ?? "";
  const parentSpaces = room
    ? spaces.filter((space) => room.parent_space_ids.includes(space.space_id))
    : [];
  const managementForRoom =
    roomManagement?.selected_room_id === roomId ? roomManagement : null;
  const settings = managementForRoom?.settings ?? null;
  const operation = managementForRoom?.operation ?? { kind: "idle" as const };
  const settingsPending = operation.kind === "pending" && operation.operation === "settings";
  const moderationPending =
    operation.kind === "pending" && operation.operation === "moderation";
  const permissions = settings?.permissions ?? null;
  const memberProfiles = (settings?.members ?? [])
    .filter((profile) => profile.user_id !== currentUserId)
    .sort((left, right) => memberLabel(left).localeCompare(memberLabel(right)));
  const [nameDraft, setNameDraft] = useState(settings?.name ?? roomName);
  const [topicDraft, setTopicDraft] = useState(settings?.topic ?? "");
  const [joinRuleDraft, setJoinRuleDraft] = useState<RoomJoinRule>(
    settings?.join_rule ?? "invite"
  );
  const [historyVisibilityDraft, setHistoryVisibilityDraft] =
    useState<RoomHistoryVisibility>(settings?.history_visibility ?? "shared");

  useEffect(() => {
    setNameDraft(settings?.name ?? roomName);
    setTopicDraft(settings?.topic ?? "");
    setJoinRuleDraft(settings?.join_rule ?? "invite");
    setHistoryVisibilityDraft(settings?.history_visibility ?? "shared");
  }, [roomName, settings]);

  const canEditSettings =
    Boolean(settings?.permissions.can_edit_settings) &&
    Boolean(onUpdateRoomSetting) &&
    !settingsPending;

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
          <h2 id="room-info-title" dir="auto">{room.display_name}</h2>
          <p dir="auto">{room.room_id}</p>
        </div>
      </header>

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

      <section className="settings-section" aria-label={t("room.management")}>
        <h3>{t("room.management")}</h3>
        {settings ? (
          <div className="room-management-grid">
            <div className="settings-detail-list">
              <DetailRow
                label={t("room.currentTopic")}
                value={settings.topic?.trim() || t("room.noTopic")}
              />
              <DetailRow label={t("room.joinRule")} value={roomJoinRuleLabel(settings.join_rule)} />
              <DetailRow
                label={t("room.historyVisibility")}
                value={roomHistoryVisibilityLabel(settings.history_visibility)}
              />
            </div>
            <form
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
                <input
                  value={nameDraft}
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
            </form>
            <form
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
                <textarea
                  value={topicDraft}
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
            </form>
            <form
              className="room-management-form"
              onSubmit={(event) => {
                event.preventDefault();
                if (canEditSettings) {
                  onUpdateRoomSetting?.(room.room_id, { joinRule: joinRuleDraft });
                  onUpdateRoomSetting?.(room.room_id, {
                    historyVisibility: historyVisibilityDraft
                  });
                }
              }}
            >
              <label className="profile-settings-field">
                <span>{t("room.joinRule")}</span>
                <select
                  value={joinRuleDraft}
                  aria-label={t("room.joinRule")}
                  disabled={!canEditSettings}
                  onChange={(event) =>
                    setJoinRuleDraft(event.currentTarget.value as RoomJoinRule)
                  }
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
              <label className="profile-settings-field">
                <span>{t("room.historyVisibility")}</span>
                <select
                  value={historyVisibilityDraft}
                  aria-label={t("room.historyVisibility")}
                  disabled={!canEditSettings}
                  onChange={(event) =>
                    setHistoryVisibilityDraft(
                      event.currentTarget.value as RoomHistoryVisibility
                    )
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
                disabled={
                  !canEditSettings ||
                  (joinRuleDraft === settings.join_rule &&
                    historyVisibilityDraft === settings.history_visibility)
                }
              >
                {t("room.saveAccess")}
              </button>
            </form>
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

      <section className="settings-section" aria-label={t("room.members")}>
        <h3>{t("room.members")}</h3>
        {memberProfiles.length ? (
          <ul className="room-member-list">
            {memberProfiles.map((profile) => (
              <li className="room-member-row" key={profile.user_id}>
                <span className="room-member-main">
                  <span dir="auto">{memberLabel(profile)}</span>
                  <small dir="auto">{profile.user_id}</small>
                </span>
                <span className="room-member-actions">
                  <ModerationButton
                    action="kick"
                    disabled={
                      !permissions?.can_kick || moderationPending || !onModerateMember
                    }
                    label={t("room.kickMember", { name: memberLabel(profile) })}
                    onClick={() =>
                      onModerateMember?.(room.room_id, profile.user_id, "kick", null)
                    }
                  />
                  <ModerationButton
                    action="ban"
                    disabled={
                      !permissions?.can_ban || moderationPending || !onModerateMember
                    }
                    label={t("room.banMember", { name: memberLabel(profile) })}
                    onClick={() =>
                      onModerateMember?.(room.room_id, profile.user_id, "ban", null)
                    }
                  />
                  <ModerationButton
                    action="unban"
                    disabled={
                      !permissions?.can_unban || moderationPending || !onModerateMember
                    }
                    label={t("room.unbanMember", { name: memberLabel(profile) })}
                    onClick={() =>
                      onModerateMember?.(room.room_id, profile.user_id, "unban", null)
                    }
                  />
                </span>
              </li>
            ))}
          </ul>
        ) : (
          <div className="settings-detail-row">
            <span>{t("room.noMembers")}</span>
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
          { icon: <Users size={16} />, label: t("room.people") },
          { icon: <FileText size={16} />, label: t("room.files") },
          { icon: <Bell size={16} />, label: t("room.notifications") },
          { icon: <Settings size={16} />, label: t("room.roomSettings") }
        ]}
      />
    </section>
  );
}

function ModerationButton({
  action,
  disabled,
  label,
  onClick
}: {
  action: RoomModerationAction;
  disabled: boolean;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      className="profile-settings-action room-member-action"
      data-action={action}
      type="button"
      aria-label={label}
      disabled={disabled}
      onClick={onClick}
    >
      {label}
    </button>
  );
}

function memberLabel(profile: RoomMemberSummary): string {
  return profile.display_name?.trim() || profile.user_id;
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
