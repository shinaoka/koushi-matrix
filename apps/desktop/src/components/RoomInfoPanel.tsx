import {
  Bell,
  ChevronRight,
  FileText,
  KeyRound,
  Link,
  MessageCircle,
  MoreHorizontal,
  Settings,
  Users
} from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import { t } from "../i18n/messages";
import { UserTrustChip } from "./TrustHelp";
import type {
  RoomHistoryVisibility,
  RoomJoinRule,
  RoomManagementState,
  RoomMemberRole,
  RoomMemberSummary,
  RoomModerationAction,
  RoomNotificationMode,
  RoomNotificationSettings,
  RoomSettingChange,
  RoomSummary,
  LinkPreviewSettingsState,
  SettingsState,
  SpaceSummary
} from "../domain/types";

const ROOM_MEMBER_ROW_HEIGHT_PX = 76;
const ROOM_MEMBER_OVERSCAN_ROWS = 4;
const ROOM_MEMBER_FALLBACK_VIEWPORT_ROWS = 8;
const ROOM_MEMBER_FALLBACK_VIEWPORT_HEIGHT_PX =
  ROOM_MEMBER_ROW_HEIGHT_PX * ROOM_MEMBER_FALLBACK_VIEWPORT_ROWS;

export function RoomInfoPanel({
  currentUserId = null,
  ignoredUserIds = [],
  initialSection = null,
  room,
  roomManagement,
  roomNotificationSettings,
  appSettings,
  linkPreviewSettings,
  spaces,
  onInvitePeople,
  onIgnoreUser,
  onUnignoreUser,
  onReportUser,
  onModerateMember,
  onOpenFiles,
  onSetLocalUserAlias,
  onRequestMemberAvatarThumbnail,
  onSetRoomNotificationMode,
  onStartDirectMessage,
  onUpdateMemberRole,
  onReshareRoomKey,
  onUpdateRoomSetting,
  onSetRoomUrlPreviewOverride
}: {
  currentUserId?: string | null;
  ignoredUserIds?: string[];
  initialSection?: "members" | null;
  room: RoomSummary | null;
  roomManagement?: RoomManagementState;
  roomNotificationSettings: RoomNotificationSettings | undefined;
  appSettings?: SettingsState;
  linkPreviewSettings?: LinkPreviewSettingsState;
  spaces: SpaceSummary[];
  onInvitePeople?: () => void;
  onIgnoreUser?: (userId: string) => void;
  onUnignoreUser?: (userId: string) => void;
  onReportUser?: (userId: string) => void;
  onModerateMember?: (
    roomId: string,
    targetUserId: string,
    action: RoomModerationAction,
    reason: string | null
  ) => void;
  onOpenFiles?: () => void;
  onSetLocalUserAlias?: (userId: string, alias: string | null) => void;
  onRequestMemberAvatarThumbnail?: (mxcUri: string) => void | Promise<void>;
  onSetRoomNotificationMode?: (roomId: string, mode: RoomNotificationMode) => void;
  onStartDirectMessage?: (userId: string) => void;
  onUpdateRoomSetting?: (roomId: string, change: RoomSettingChange) => void;
  onUpdateMemberRole?: (roomId: string, targetUserId: string, powerLevel: number) => void;
  onReshareRoomKey?: (roomId: string) => void | Promise<void>;
  onSetRoomUrlPreviewOverride?: (roomId: string, enabled: boolean) => void;
}) {
  const roomId = room?.room_id ?? "";
  const roomName = room?.display_label ?? "";
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
  const operation = managementForRoom?.operation ?? { kind: "idle" as const };
  const settingsPending = operation.kind === "pending" && operation.operation === "settings";
  const moderationPending =
    operation.kind === "pending" && operation.operation === "moderation";
  const rolePending = operation.kind === "pending" && operation.operation === "roles";
  const permissions = settings?.permissions ?? null;
  const memberProfiles = useMemo(
    () =>
      (settings?.members ?? [])
        .filter((profile) => profile.user_id !== currentUserId)
        .sort((left, right) => memberLabel(left).localeCompare(memberLabel(right))),
    [currentUserId, settings?.members]
  );
  const [nameDraft, setNameDraft] = useState(settings?.name ?? roomName);
  const [topicDraft, setTopicDraft] = useState(settings?.topic ?? "");
  const [avatarDraft, setAvatarDraft] = useState(settings?.avatar_url ?? "");
  const [joinRuleDraft, setJoinRuleDraft] = useState<RoomJoinRule>(
    settings?.join_rule ?? "invite"
  );
  const [historyVisibilityDraft, setHistoryVisibilityDraft] =
    useState<RoomHistoryVisibility>(settings?.history_visibility ?? "shared");
  const [aliasTarget, setAliasTarget] = useState<RoomMemberSummary | null>(null);
  const [aliasDraft, setAliasDraft] = useState("");
  const [reshareState, setReshareState] = useState<"idle" | "pending" | "success" | "error">("idle");
  const membersRef = useRef<HTMLElement>(null);
  const memberListScrollRef = useRef<HTMLDivElement>(null);
  const memberAvatarRequestsRef = useRef<Set<string>>(new Set());
  const [memberListViewport, setMemberListViewport] = useState({
    scrollTop: 0,
    clientHeight: ROOM_MEMBER_FALLBACK_VIEWPORT_HEIGHT_PX
  });

  useEffect(() => {
    setNameDraft(settings?.name ?? roomName);
    setTopicDraft(settings?.topic ?? "");
    setAvatarDraft(settings?.avatar_url ?? "");
    setJoinRuleDraft(settings?.join_rule ?? "invite");
    setHistoryVisibilityDraft(settings?.history_visibility ?? "shared");
  }, [roomName, settings]);

  useEffect(() => {
    if (initialSection !== "members") {
      return;
    }
    window.requestAnimationFrame(() => {
      membersRef.current?.scrollIntoView({ behavior: "smooth", block: "start" });
    });
  }, [initialSection, roomId, memberProfiles.length]);

  useEffect(() => {
    setReshareState("idle");
  }, [roomId]);

  useEffect(() => {
    memberAvatarRequestsRef.current.clear();
  }, [roomId]);

  useEffect(() => {
    setMemberListViewport({
      scrollTop: 0,
      clientHeight: ROOM_MEMBER_FALLBACK_VIEWPORT_HEIGHT_PX
    });
    if (memberListScrollRef.current) {
      memberListScrollRef.current.scrollTop = 0;
    }
  }, [memberProfiles.length, roomId]);

  const memberListWindow = useMemo(() => {
    const clientHeight =
      memberListViewport.clientHeight || ROOM_MEMBER_FALLBACK_VIEWPORT_HEIGHT_PX;
    const scrollTop = Math.max(0, memberListViewport.scrollTop);
    const start = Math.max(
      0,
      Math.floor(scrollTop / ROOM_MEMBER_ROW_HEIGHT_PX) - ROOM_MEMBER_OVERSCAN_ROWS
    );
    const end = Math.min(
      memberProfiles.length,
      Math.ceil((scrollTop + clientHeight) / ROOM_MEMBER_ROW_HEIGHT_PX) +
        ROOM_MEMBER_OVERSCAN_ROWS
    );
    return { start, end };
  }, [memberListViewport.clientHeight, memberListViewport.scrollTop, memberProfiles.length]);

  const visibleMembers = useMemo(
    () => memberProfiles.slice(memberListWindow.start, memberListWindow.end),
    [memberListWindow.end, memberListWindow.start, memberProfiles]
  );

  useEffect(() => {
    if (!onRequestMemberAvatarThumbnail) {
      return;
    }

    for (const profile of visibleMembers) {
      const mxcUri = profile.avatar_url;
      if (!mxcUri || memberAvatarRequestsRef.current.has(mxcUri)) {
        continue;
      }
      memberAvatarRequestsRef.current.add(mxcUri);
      void Promise.resolve(onRequestMemberAvatarThumbnail(mxcUri)).catch(() => {
        memberAvatarRequestsRef.current.delete(mxcUri);
      });
    }
  }, [onRequestMemberAvatarThumbnail, visibleMembers]);

  async function reshareRoomKeys() {
    if (!onReshareRoomKey || reshareState === "pending") {
      return;
    }
    setReshareState("pending");
    try {
      await onReshareRoomKey(roomId);
      setReshareState("success");
    } catch {
      setReshareState("error");
    }
  }

  const closeAliasDialog = () => {
    setAliasTarget(null);
    setAliasDraft("");
  };

  const openAliasDialog = (profile: RoomMemberSummary) => {
    setAliasTarget(profile);
    setAliasDraft(aliasIsActive(profile) ? profile.display_label : "");
  };

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
          <h2 id="room-info-title" dir="auto">{room.display_label}</h2>
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
        {isEncrypted ? (
          <div className="room-key-actions">
            <button
              className="profile-settings-action"
              type="button"
              disabled={!onReshareRoomKey || reshareState === "pending"}
              onClick={() => {
                void reshareRoomKeys();
              }}
            >
              <KeyRound size={16} aria-hidden="true" />
              <span>
                {reshareState === "pending"
                  ? t("room.reshareRoomKeysPending")
                  : t("room.reshareRoomKeys")}
              </span>
            </button>
            <p className="profile-settings-hint">{t("room.reshareRoomKeysHint")}</p>
            {reshareState === "success" ? (
              <p className="profile-settings-hint success">{t("room.reshareRoomKeysSuccess")}</p>
            ) : reshareState === "error" ? (
              <p className="profile-settings-hint error">{t("room.reshareRoomKeysError")}</p>
            ) : null}
          </div>
        ) : null}
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
                    avatarUrl: avatarDraft.trim() || null
                  });
                }
              }}
            >
              <label className="profile-settings-field">
                <span>{t("room.avatarUrl")}</span>
                <input
                  value={avatarDraft}
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

      <section ref={membersRef} className="settings-section" aria-label={t("room.members")}>
        <h3>{t("room.members")}</h3>
        {memberProfiles.length ? (
          <div
            ref={memberListScrollRef}
            className="room-member-scroll-container"
            onScroll={(event) => {
              const currentTarget = event.currentTarget;
              setMemberListViewport({
                scrollTop: currentTarget.scrollTop,
                clientHeight: currentTarget.clientHeight || ROOM_MEMBER_FALLBACK_VIEWPORT_HEIGHT_PX
              });
            }}
          >
            <div className="room-member-virtual-list">
              <div
                aria-hidden="true"
                className="room-member-spacer"
                style={{ height: `${memberListWindow.start * ROOM_MEMBER_ROW_HEIGHT_PX}px` }}
              />
              <ul className="room-member-list">
                {visibleMembers.map((profile) => (
                  <li className="room-member-row" key={profile.user_id}>
                    <span className="room-member-main">
                      <button
                        className="room-member-name-button"
                        type="button"
                        disabled={!onStartDirectMessage}
                        onClick={() => onStartDirectMessage?.(profile.user_id)}
                      >
                        <span dir="auto">{memberLabel(profile)}</span>
                      </button>
                      <small dir="auto">{profile.user_id}</small>
                      {aliasIsActive(profile) ? (
                        <small className="room-member-original-context" dir="auto">
                          {t("room.memberOriginalName", {
                            name: profile.original_display_label
                          })}
                        </small>
                      ) : null}
                      <small>{roomMemberRoleLabel(profile.role)}</small>
                      <UserTrustChip state={profile.user_trust ?? null} />
                    </span>
                    <span className="room-member-actions">
                      <button
                        className="profile-settings-action room-member-action room-member-icon-action"
                        type="button"
                        aria-label={t("room.messageMember", { name: memberLabel(profile) })}
                        disabled={!onStartDirectMessage}
                        onClick={() => onStartDirectMessage?.(profile.user_id)}
                      >
                        <MessageCircle size={14} />
                      </button>
                      <label className="room-member-role-field">
                        <select
                          aria-label={t("room.memberRoleFor", { name: memberLabel(profile) })}
                          value={
                            profile.power_level === null ? "creator" : String(profile.power_level)
                          }
                          disabled={
                            !permissions?.can_edit_roles || rolePending || !onUpdateMemberRole
                          }
                          onChange={(event) => {
                            if (event.currentTarget.value === "creator") {
                              return;
                            }
                            onUpdateMemberRole?.(
                              room.room_id,
                              profile.user_id,
                              Number(event.currentTarget.value)
                            );
                          }}
                        >
                          {profile.power_level === null ? (
                            <option value="creator" disabled>
                              {roomMemberRoleLabel("creator")}
                            </option>
                          ) : null}
                          {roomMemberRoleOptions.map((option) => (
                            <option key={option.powerLevel} value={String(option.powerLevel)}>
                              {roomMemberRoleLabel(option.role)}
                            </option>
                          ))}
                        </select>
                      </label>
                      {onSetLocalUserAlias ? (
                        <>
                          <button
                            className="profile-settings-action room-member-action"
                            type="button"
                            aria-label={t(
                              aliasIsActive(profile)
                                ? "room.editAliasForMember"
                                : "room.setAliasForMember",
                              { name: memberLabel(profile) }
                            )}
                            onClick={() => openAliasDialog(profile)}
                          >
                            {t(aliasIsActive(profile) ? "room.editAlias" : "room.setAlias")}
                          </button>
                        </>
                      ) : null}
                      <details className="room-member-menu">
                        <summary
                          className="profile-settings-action room-member-action room-member-icon-action"
                          aria-label={t("context.openRoomInfo")}
                        >
                          <MoreHorizontal size={14} />
                        </summary>
                        <div className="room-member-menu-popover">
                          {aliasIsActive(profile) && onSetLocalUserAlias ? (
                            <button
                              className="room-member-menu-item"
                              type="button"
                              aria-label={t("room.clearAliasForMember", {
                                name: memberLabel(profile)
                              })}
                              onClick={() => onSetLocalUserAlias(profile.user_id, null)}
                            >
                              {t("room.clearAlias")}
                            </button>
                          ) : null}
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
                          {ignoredUserIds.includes(profile.user_id) ? (
                            <button
                              className="room-member-menu-item"
                              type="button"
                              aria-label={t("context.unignoreUser")}
                              disabled={!onUnignoreUser}
                              onClick={() => onUnignoreUser?.(profile.user_id)}
                            >
                              {t("context.unignoreUser")}
                            </button>
                          ) : (
                            <button
                              className="room-member-menu-item"
                              type="button"
                              aria-label={t("context.ignoreUser")}
                              disabled={!onIgnoreUser}
                              onClick={() => onIgnoreUser?.(profile.user_id)}
                            >
                              {t("context.ignoreUser")}
                            </button>
                          )}
                          <button
                            className="room-member-menu-item"
                            type="button"
                            aria-label={t("context.reportUser")}
                            disabled={!onReportUser}
                            onClick={() => onReportUser?.(profile.user_id)}
                          >
                            {t("context.reportUser")}
                          </button>
                        </div>
                      </details>
                    </span>
                  </li>
                ))}
              </ul>
              <div
                aria-hidden="true"
                className="room-member-spacer"
                style={{
                  height: `${(memberProfiles.length - memberListWindow.end) * ROOM_MEMBER_ROW_HEIGHT_PX}px`
                }}
              />
            </div>
          </div>
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
            onClick: () => {
              membersRef.current?.scrollIntoView({ behavior: "smooth", block: "start" });
            }
          },
          { icon: <FileText size={16} />, label: t("room.files"), onClick: onOpenFiles },
          { icon: <Bell size={16} />, label: t("room.notifications") },
          { icon: <Settings size={16} />, label: t("room.roomSettings") }
        ]}
      />
      {aliasTarget ? (
        <div className="dialog-overlay" role="presentation" onMouseDown={closeAliasDialog}>
          <form
            className="dialog-box room-alias-dialog"
            aria-label={t("room.aliasDialogTitle", { name: memberLabel(aliasTarget) })}
            onMouseDown={(event) => event.stopPropagation()}
            onSubmit={(event) => {
              event.preventDefault();
              onSetLocalUserAlias?.(aliasTarget.user_id, aliasDraft.trim() || null);
              closeAliasDialog();
            }}
          >
            <h3 className="dialog-title">
              {t("room.aliasDialogTitle", { name: memberLabel(aliasTarget) })}
            </h3>
            {aliasIsActive(aliasTarget) ? (
              <p className="room-member-original-context" dir="auto">
                {t("room.memberOriginalName", {
                  name: aliasTarget.original_display_label
                })}
              </p>
            ) : null}
            <input
              className="dialog-input"
              aria-label={t("room.aliasInput")}
              value={aliasDraft}
              onChange={(event) => setAliasDraft(event.currentTarget.value)}
              autoFocus
            />
            <div className="dialog-actions">
              <button className="dialog-button" type="button" onClick={closeAliasDialog}>
                {t("action.cancel")}
              </button>
              <button className="dialog-button is-primary" type="submit">
                {t("room.saveAlias")}
              </button>
            </div>
          </form>
        </div>
      ) : null}
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
  return profile.display_label;
}

function aliasIsActive(profile: RoomMemberSummary): boolean {
  const displayLabel = profile.display_label.trim();
  const originalDisplayLabel = profile.original_display_label.trim();
  return Boolean(displayLabel && originalDisplayLabel && displayLabel !== originalDisplayLabel);
}

const roomMemberRoleOptions: Array<{ role: RoomMemberRole; powerLevel: number }> = [
  { role: "administrator", powerLevel: 100 },
  { role: "moderator", powerLevel: 50 },
  { role: "user", powerLevel: 0 }
];

function roomMemberRoleLabel(role: RoomMemberRole): string {
  switch (role) {
    case "creator":
      return t("room.roleCreator");
    case "administrator":
      return t("room.roleAdministrator");
    case "moderator":
      return t("room.roleModerator");
    case "user":
      return t("room.roleUser");
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
