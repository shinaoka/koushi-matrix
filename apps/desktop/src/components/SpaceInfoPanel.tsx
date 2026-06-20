import {
  Bell,
  ChevronRight,
  FileText,
  Home,
  MailPlus,
  MessageCircle,
  Settings,
  SlidersHorizontal,
  Users
} from "lucide-react";
import { type ReactNode, useRef } from "react";

import { t } from "../i18n/messages";
import type { RoomManagementState, RoomSummary, SpaceSummary } from "../domain/types";

export function SpaceInfoPanel({
  fallbackName,
  rooms,
  roomManagement,
  space,
  onInvitePeople,
  onOpenFiles,
  onOpenMembers,
  onStartDirectMessage
}: {
  fallbackName: string;
  rooms: RoomSummary[];
  roomManagement?: RoomManagementState;
  space: SpaceSummary | null;
  onInvitePeople?: () => void;
  onOpenFiles?: () => void;
  onOpenMembers?: () => void;
  onStartDirectMessage?: (userId: string) => void;
}) {
  const membersSectionRef = useRef<HTMLElement>(null);
  const childRooms = space
    ? space.child_room_ids
        .map((roomId) => rooms.find((room) => room.room_id === roomId))
        .filter((room): room is RoomSummary => Boolean(room && !room.is_dm))
    : rooms.filter((room) => !room.is_dm);
  const unreadTotal = childRooms.reduce((sum, room) => sum + room.unread_count, 0);
  const title = space?.display_name ?? fallbackName;
  const loadedSpaceSettings =
    space && roomManagement?.selected_room_id === space.space_id
      ? roomManagement.settings
      : null;
  const loadingMembers =
    Boolean(space) &&
    roomManagement?.operation.kind === "pending" &&
    roomManagement.operation.room_id === space?.space_id;
  const memberCount = loadedSpaceSettings?.members.length ?? 0;

  function openMembers() {
    onOpenMembers?.();
    window.requestAnimationFrame(() => {
      membersSectionRef.current?.scrollIntoView({ block: "start" });
    });
  }

  return (
    <section className="settings-panel space-info-panel" aria-labelledby="space-info-title">
      <header className="settings-panel-header">
        <div>
          <h2 id="space-info-title" dir="auto">{title}</h2>
          <p dir="auto">{space?.space_id ?? t("space.allRooms")}</p>
        </div>
      </header>

      <div className="settings-summary-grid" aria-label={t("space.summary")}>
        <SummaryTile label={t("workspace.rooms")} value={String(childRooms.length)} />
        <SummaryTile label={t("room.members")} value={loadedSpaceSettings ? String(memberCount) : "-"} />
        <SummaryTile label={t("room.unread")} value={String(unreadTotal)} />
      </div>

      <section className="settings-section" aria-label={t("workspace.rooms")}>
        <h3>{t("workspace.rooms")}</h3>
        <div className="settings-detail-list">
          {childRooms.map((room) => (
            <div className="settings-detail-row" key={room.room_id}>
              <span dir="auto">{room.display_label}</span>
              <small dir="auto">{room.unread_count ? t("room.unreadCount", { count: room.unread_count }) : room.room_id}</small>
            </div>
          ))}
        </div>
      </section>

      <section
        ref={membersSectionRef}
        className="settings-section"
        id="space-members"
        aria-label={t("room.members")}
      >
        <h3>{t("room.members")}</h3>
        <div className="settings-detail-list">
          {loadingMembers ? (
            <DetailRow label={t("room.members")} value={t("settings.saving")} />
          ) : loadedSpaceSettings ? (
            loadedSpaceSettings.members.length > 0 ? (
              loadedSpaceSettings.members.map((member) => (
                <SpaceMemberRow
                  key={member.user_id}
                  displayLabel={member.display_label}
                  userId={member.user_id}
                  onStartDirectMessage={onStartDirectMessage}
                />
              ))
            ) : (
              <DetailRow label={t("room.members")} value={t("room.noMembers")} />
            )
          ) : (
            <DetailRow label={t("room.members")} value={t("room.noMembers")} />
          )}
        </div>
      </section>

      <section className="settings-section" aria-label={t("space.spacePreferences")}>
        <h3>{t("space.spacePreferences")}</h3>
        <div className="settings-detail-list">
          <DetailRow label={t("space.roomMembership")} value={space ? t("space.childRooms") : t("space.allRooms")} />
          <DetailRow label={t("space.directMessages")} value={t("room.globalDmList")} />
          <DetailRow label={t("room.notifications")} value={unreadTotal ? t("room.unreadCount", { count: unreadTotal }) : t("space.noUnread")} />
        </div>
      </section>

      <SettingsEntryList
        entries={[
          { icon: <Home size={16} />, label: t("space.home") },
          { icon: <SlidersHorizontal size={16} />, label: t("space.preferences") },
          { icon: <Settings size={16} />, label: t("space.spaceSettings") },
          { icon: <Users size={16} />, label: t("room.members"), onClick: space ? openMembers : undefined },
          { icon: <MailPlus size={16} />, label: t("space.invite"), onClick: onInvitePeople },
          { icon: <Bell size={16} />, label: t("room.notifications") },
          { icon: <FileText size={16} />, label: t("room.files"), onClick: onOpenFiles }
        ]}
      />
    </section>
  );
}

function DetailRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="settings-detail-row">
      <span>{label}</span>
      <small>{value}</small>
    </div>
  );
}

function SpaceMemberRow({
  displayLabel,
  userId,
  onStartDirectMessage
}: {
  displayLabel: string;
  userId: string;
  onStartDirectMessage?: (userId: string) => void;
}) {
  return (
    <div className="settings-detail-row">
      <span dir="auto">{displayLabel}</span>
      <small dir="auto">{userId}</small>
      <button
        className="profile-settings-action room-member-action"
        type="button"
        aria-label={t("room.messageMember", { name: displayLabel })}
        disabled={!onStartDirectMessage}
        onClick={() => onStartDirectMessage?.(userId)}
      >
        <MessageCircle size={14} />
        {t("workspace.newDm")}
      </button>
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
