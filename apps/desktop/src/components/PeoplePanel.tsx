import { ChevronLeft, MessageCircle, UserPlus, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type UIEvent } from "react";
import { t } from "../i18n/messages";
import type {
  RoomManagementState,
  RoomMemberRole,
  RoomMemberSummary,
  RoomModerationAction,
  RoomSummary,
  SpaceSummary,
  UserProfile
} from "../domain/types";

import { EntityAvatar } from "./Shell";
import { ICON_SIZE, initials } from "../app/uiShared";

const PEOPLE_MEMBER_ROW_HEIGHT_PX = 58;
const PEOPLE_MEMBER_OVERSCAN_ROWS = 6;
const PEOPLE_MEMBER_FALLBACK_VIEWPORT_ROWS = 10;
const PEOPLE_MEMBER_FALLBACK_VIEWPORT_HEIGHT_PX =
  PEOPLE_MEMBER_ROW_HEIGHT_PX * PEOPLE_MEMBER_FALLBACK_VIEWPORT_ROWS;

type RoomOrSpace = RoomSummary | SpaceSummary;

interface PeoplePanelProps {
  currentUserId: string | null;
  roomOrSpace: RoomOrSpace | null;
  roomManagement: RoomManagementState;
  onOpenProfile: (userId: string) => void;
  onClose?: () => void;
  onInvitePeople?: () => void;
  onStartDirectMessage?: (userId: string) => void;
}

interface ProfilePanelProps {
  userId: string;
  currentUserId: string | null;
  ignoredUserIds?: string[];
  roomOrSpace: RoomOrSpace | null;
  roomManagement: RoomManagementState;
  profileUsers: Record<string, UserProfile>;
  onBack: () => void;
  onClose?: () => void;
  onIgnoreUser?: (userId: string) => void;
  onModerateMember?: (
    roomId: string,
    targetUserId: string,
    action: RoomModerationAction,
    reason: string | null
  ) => void;
  onReportUser?: (userId: string) => void;
  onStartDirectMessage?: (userId: string) => void;
  onSetLocalUserAlias?: (userId: string, alias: string | null) => void;
  onUnignoreUser?: (userId: string) => void;
  onUpdateMemberRole?: (
    roomId: string,
    targetUserId: string,
    powerLevel: number
  ) => void;
}

export function PeoplePanel(props: PeoplePanelProps) {
  const {
    currentUserId,
    roomOrSpace,
    roomManagement,
    onOpenProfile,
    onClose = () => undefined,
    onInvitePeople,
    onStartDirectMessage
  } = props;
  const [query, setQuery] = useState("");
  const listViewportRef = useRef<HTMLDivElement>(null);
  const [listViewport, setListViewport] = useState({ scrollTop: 0, clientHeight: 0 });
  const contextId = roomOrSpaceId(roomOrSpace);
  const members = useMembers(roomOrSpace, roomManagement);
  const filteredMembers = useFilteredMembers(members, query);
  const memberWindow = useMemo(() => {
    const viewportHeight =
      listViewport.clientHeight || PEOPLE_MEMBER_FALLBACK_VIEWPORT_HEIGHT_PX;
    const scrollTop = Math.max(0, listViewport.scrollTop);
    const visibleRows = Math.max(
      1,
      Math.ceil(viewportHeight / PEOPLE_MEMBER_ROW_HEIGHT_PX)
    );
    const start = Math.max(
      0,
      Math.floor(scrollTop / PEOPLE_MEMBER_ROW_HEIGHT_PX) - PEOPLE_MEMBER_OVERSCAN_ROWS
    );
    const end = Math.min(
      filteredMembers.length,
      start + visibleRows + PEOPLE_MEMBER_OVERSCAN_ROWS * 2
    );
    return { start, end };
  }, [filteredMembers.length, listViewport.clientHeight, listViewport.scrollTop]);
  const visibleMembers = useMemo(
    () => filteredMembers.slice(memberWindow.start, memberWindow.end),
    [filteredMembers, memberWindow.end, memberWindow.start]
  );

  useEffect(() => {
    const listViewportElement = listViewportRef.current;
    if (listViewportElement) {
      listViewportElement.scrollTop = 0;
    }
    setListViewport({ scrollTop: 0, clientHeight: listViewportElement?.clientHeight ?? 0 });
  }, [contextId, query]);

  function updateListViewport(event: UIEvent<HTMLDivElement>) {
    setListViewport({
      scrollTop: event.currentTarget.scrollTop,
      clientHeight: event.currentTarget.clientHeight
    });
  }

  return (
    <section className="people-panel" aria-labelledby="people-title">
      <header className="people-panel-header">
        <h2 id="people-title">{t("panel.people")}</h2>
        <span className="people-panel-count">
          {t("people.memberCount", { count: String(members.length) })}
        </span>
        <button
          className="icon-button people-panel-close"
          type="button"
          aria-label={t("action.close", { title: t("panel.people") })}
          onClick={onClose}
        >
          <X size={ICON_SIZE.control} />
        </button>
      </header>
      <div className="people-panel-actions">
        {onInvitePeople ? (
          <button
            className="people-panel-primary-action"
            type="button"
            onClick={onInvitePeople}
          >
            <UserPlus size={ICON_SIZE.small} aria-hidden="true" />
            <span>{t("room.invitePeople")}</span>
          </button>
        ) : null}
        <label className="people-search">
          <input
            type="search"
            value={query}
            placeholder={t("people.searchMembers")}
            aria-label={t("people.searchMembers")}
            onChange={(event) => setQuery(event.currentTarget.value)}
          />
        </label>
      </div>
      <div
        className="people-list-viewport"
        ref={listViewportRef}
        onScroll={updateListViewport}
      >
        <div className="people-list-virtual">
          <div
            className="people-list-spacer"
            style={{ height: `${memberWindow.start * PEOPLE_MEMBER_ROW_HEIGHT_PX}px` }}
          />
          <ul className="people-list" role="list" aria-label={t("room.members")}>
            {visibleMembers.map((member, index) => (
              <PeopleListRow
                key={member.user_id}
                currentUserId={currentUserId}
                member={member}
                position={memberWindow.start + index + 1}
                setSize={filteredMembers.length}
                onOpenProfile={onOpenProfile}
                onStartDirectMessage={onStartDirectMessage}
              />
            ))}
          </ul>
          <div
            className="people-list-spacer"
            style={{
              height: `${
                (filteredMembers.length - memberWindow.end) *
                PEOPLE_MEMBER_ROW_HEIGHT_PX
              }px`
            }}
          />
        </div>
      </div>
      {filteredMembers.length === 0 && query.trim() ? (
        <p className="people-empty" role="status">
          {t("people.noSearchResults")}
        </p>
      ) : null}
    </section>
  );
}

function PeopleListRow({
  currentUserId,
  member,
  position,
  setSize,
  onOpenProfile,
  onStartDirectMessage
}: {
  currentUserId: string | null;
  member: RoomMemberSummary;
  position: number;
  setSize: number;
  onOpenProfile: (userId: string) => void;
  onStartDirectMessage?: (userId: string) => void;
}) {
  const displayLabel = member.display_label;
  const isCurrentUser = member.user_id === currentUserId;
  return (
    <li className="people-list-row" aria-posinset={position} aria-setsize={setSize}>
      <button
        className="people-list-main"
        type="button"
        aria-label={t("people.openProfile", { name: displayLabel })}
        onClick={() => onOpenProfile(member.user_id)}
      >
        <EntityAvatar
          avatar={null}
          className="people-list-avatar is-user"
          fallback={initials(displayLabel)}
        />
        <span className="people-list-text">
          <span className="people-list-name" dir="auto">
            {displayLabel}
            {isCurrentUser ? (
              <span className="people-list-you">{t("people.you")}</span>
            ) : null}
          </span>
          <span className="people-list-meta" dir="auto">
            {roomMemberRoleLabel(member.role)}
          </span>
        </span>
      </button>
      {!isCurrentUser && onStartDirectMessage ? (
        <button
          className="icon-button people-list-action"
          type="button"
          aria-label={t("room.messageMember", { name: displayLabel })}
          onClick={() => onStartDirectMessage(member.user_id)}
        >
          <MessageCircle size={ICON_SIZE.small} />
        </button>
      ) : null}
    </li>
  );
}

export function ProfilePanel({
  userId,
  currentUserId,
  ignoredUserIds = [],
  roomOrSpace,
  roomManagement,
  profileUsers,
  onBack,
  onClose = () => undefined,
  onIgnoreUser,
  onModerateMember,
  onReportUser,
  onStartDirectMessage,
  onSetLocalUserAlias,
  onUnignoreUser,
  onUpdateMemberRole
}: ProfilePanelProps) {
  const contextId = roomOrSpaceId(roomOrSpace);
  const settings =
    roomManagement.selected_room_id === contextId ? roomManagement.settings : null;
  const members = useMembers(roomOrSpace, roomManagement);
  const member = members.find((candidate) => candidate.user_id === userId);
  const profile = profileUsers[userId];
  const displayLabel = member?.display_label ?? profile?.display_label ?? userId;
  const avatar = member ? null : (profile?.avatar ?? null);
  const roomId = roomOrSpace && "room_id" in roomOrSpace ? roomOrSpace.room_id : null;
  const permissions = settings?.permissions ?? null;
  const isCurrentUser = userId === currentUserId;
  const rolePending =
    roomManagement.operation.kind === "pending" &&
    roomManagement.operation.operation === "roles";
  const moderationPending =
    roomManagement.operation.kind === "pending" &&
    roomManagement.operation.operation === "moderation";
  const canTargetMember = Boolean(roomId && member && !isCurrentUser);
  const isIgnored = ignoredUserIds.includes(userId);
  const [aliasDraft, setAliasDraft] = useState("");
  const [showAliasForm, setShowAliasForm] = useState(false);

  return (
    <section className="people-panel profile-panel" aria-labelledby="profile-title">
      <header className="people-panel-header">
        <button
          className="icon-button people-back-button"
          type="button"
          aria-label={t("action.back")}
          onClick={onBack}
        >
          <ChevronLeft size={ICON_SIZE.control} />
        </button>
        <h2 id="profile-title">{t("panel.profile")}</h2>
        <button
          className="icon-button people-panel-close"
          type="button"
          aria-label={t("action.close", { title: t("panel.profile") })}
          onClick={onClose}
        >
          <X size={ICON_SIZE.control} />
        </button>
      </header>
      <div className="profile-identity">
        <EntityAvatar
          avatar={avatar}
          className="profile-large-avatar is-user"
          fallback={initials(displayLabel)}
        />
        <h3 dir="auto">{displayLabel}</h3>
        <p dir="auto">{userId}</p>
      </div>
      <div className="profile-actions">
        {!isCurrentUser && onStartDirectMessage ? (
          <button
            className="profile-primary-action"
            type="button"
            onClick={() => onStartDirectMessage(userId)}
          >
            <MessageCircle size={ICON_SIZE.small} aria-hidden="true" />
            <span>{t("people.sendMessage")}</span>
          </button>
        ) : null}
      </div>
      {member ? (
        <div className="profile-room-details">
          <div className="profile-detail-row">
            <span>{t("room.memberRole")}</span>
            {roomId ? (
              <select
                aria-label={t("room.memberRoleFor", { name: displayLabel })}
                value={member.power_level === null ? "creator" : String(member.power_level)}
                disabled={
                  member.power_level === null ||
                  !permissions?.can_edit_roles ||
                  rolePending ||
                  !onUpdateMemberRole ||
                  !canTargetMember
                }
                onChange={(event) => {
                  if (event.currentTarget.value === "creator") {
                    return;
                  }
                  onUpdateMemberRole?.(roomId, userId, Number(event.currentTarget.value));
                }}
              >
                {member.power_level === null ? (
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
            ) : (
              <strong>{roomMemberRoleLabel(member.role)}</strong>
            )}
          </div>
          {onSetLocalUserAlias ? (
            <div className="profile-detail-row">
              <button
                className="profile-text-button"
                type="button"
                onClick={() => {
                  setAliasDraft(aliasIsActive(member) ? member.display_label : "");
                  setShowAliasForm((show) => !show);
                }}
              >
                {t("people.setAlias")}
              </button>
            </div>
          ) : null}
          {aliasIsActive(member) ? (
            <p className="room-member-original-context" dir="auto">
              {t("room.memberOriginalName", {
                name: member.original_display_label
              })}
            </p>
          ) : null}
          {showAliasForm ? (
            <form
              className="profile-alias-form"
              onSubmit={(event) => {
                event.preventDefault();
                onSetLocalUserAlias?.(userId, aliasDraft.trim() || null);
                setShowAliasForm(false);
              }}
            >
              <input
                type="text"
                value={aliasDraft}
                aria-label={t("room.aliasInput")}
                onChange={(event) => setAliasDraft(event.currentTarget.value)}
              />
              <button className="dialog-button" type="submit">
                {t("room.saveAlias")}
              </button>
              <button
                className="dialog-button secondary"
                type="button"
                onClick={() => setShowAliasForm(false)}
              >
                {t("action.cancel")}
              </button>
            </form>
          ) : null}
          {canTargetMember ? (
            <div className="profile-member-actions" aria-label={t("people.memberActions")}>
              {isIgnored ? (
                <button
                  className="profile-text-button"
                  type="button"
                  disabled={!onUnignoreUser}
                  onClick={() => onUnignoreUser?.(userId)}
                >
                  {t("context.unignoreUser")}
                </button>
              ) : (
                <button
                  className="profile-text-button"
                  type="button"
                  disabled={!onIgnoreUser}
                  onClick={() => onIgnoreUser?.(userId)}
                >
                  {t("context.ignoreUser")}
                </button>
              )}
              <button
                className="profile-text-button"
                type="button"
                disabled={!onReportUser}
                onClick={() => onReportUser?.(userId)}
              >
                {t("context.reportUser")}
              </button>
              <ModerationButton
                action="kick"
                disabled={!permissions?.can_kick || moderationPending || !onModerateMember}
                label={t("room.kickMember", { name: displayLabel })}
                onClick={() => onModerateMember?.(roomId!, userId, "kick", null)}
              />
              <ModerationButton
                action="ban"
                disabled={!permissions?.can_ban || moderationPending || !onModerateMember}
                label={t("room.banMember", { name: displayLabel })}
                onClick={() => onModerateMember?.(roomId!, userId, "ban", null)}
              />
            </div>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}

function useMembers(
  roomOrSpace: RoomOrSpace | null,
  roomManagement: RoomManagementState
): RoomMemberSummary[] {
  return useMemo(() => {
    const contextId = roomOrSpaceId(roomOrSpace);
    const settings =
      roomManagement?.selected_room_id === contextId ? roomManagement.settings : null;
    return (settings?.members ?? [])
      .slice()
      .sort((left, right) => left.display_label.localeCompare(right.display_label));
  }, [roomOrSpace, roomManagement]);
}

function roomOrSpaceId(roomOrSpace: RoomOrSpace | null): string | null {
  return roomOrSpace
    ? "room_id" in roomOrSpace
      ? roomOrSpace.room_id
      : roomOrSpace.space_id
    : null;
}

function useFilteredMembers(
  members: RoomMemberSummary[],
  query: string
): RoomMemberSummary[] {
  const trimmed = query.trim().toLowerCase();
  if (!trimmed) {
    return members;
  }
  return members.filter((member) => {
    return (
      member.display_label.toLowerCase().includes(trimmed) ||
      member.original_display_label.toLowerCase().includes(trimmed) ||
      member.user_id.toLowerCase().includes(trimmed)
    );
  });
}

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

const roomMemberRoleOptions: Array<{ role: RoomMemberRole; powerLevel: number }> = [
  { role: "administrator", powerLevel: 100 },
  { role: "moderator", powerLevel: 50 },
  { role: "user", powerLevel: 0 }
];

function aliasIsActive(profile: RoomMemberSummary): boolean {
  const displayLabel = profile.display_label.trim();
  const originalDisplayLabel = profile.original_display_label.trim();
  return Boolean(displayLabel && originalDisplayLabel && displayLabel !== originalDisplayLabel);
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
      className="profile-text-button"
      data-action={action}
      type="button"
      disabled={disabled}
      onClick={onClick}
    >
      {label}
    </button>
  );
}
