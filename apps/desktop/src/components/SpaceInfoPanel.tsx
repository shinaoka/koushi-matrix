import {
  Bell,
  ChevronRight,
  FileText,
  Home,
  MailPlus,
  Settings,
  SlidersHorizontal,
  Users
} from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";

import { t } from "../i18n/messages";
import type { RoomManagementState, RoomSummary, SpaceSummary } from "../domain/types";
import { ImeTextField } from "./ImeTextControl";

export function SpaceInfoPanel({
  fallbackName,
  localIcon = "",
  localName = "",
  rooms,
  roomManagement,
  space,
  onInvitePeople,
  onOpenFiles,
  onOpenMembers,
  onSetLocalPresentation
}: {
  fallbackName: string;
  localIcon?: string;
  localName?: string;
  rooms: RoomSummary[];
  roomManagement?: RoomManagementState;
  space: SpaceSummary | null;
  onInvitePeople?: () => void;
  onOpenFiles?: () => void;
  onOpenMembers?: () => void;
  onSetLocalPresentation?: (override: { name?: string; icon?: string } | null) => void;
}) {
  const [localNameDraft, setLocalNameDraft] = useState(localName);
  const [localIconDraft, setLocalIconDraft] = useState(localIcon);
  const childRooms = space
    ? space.child_room_ids
        .map((roomId) => rooms.find((room) => room.room_id === roomId))
        .filter((room): room is RoomSummary => Boolean(room && !room.is_dm))
    : rooms.filter((room) => !room.is_dm);
  const unreadTotal = childRooms.reduce((sum, room) => sum + room.unread_count, 0);
  const title = localName.trim() || space?.display_name || fallbackName;
  const loadedSpaceSettings =
    space && roomManagement?.selected_room_id === space.space_id
      ? roomManagement.settings
      : null;
  const memberCount = loadedSpaceSettings?.members.length ?? 0;

  useEffect(() => {
    setLocalNameDraft(localName);
    setLocalIconDraft(localIcon);
  }, [localIcon, localName]);

  function openMembers() {
    onOpenMembers?.();
  }

  function updateLocalPresentation(next: { name: string; icon: string }) {
    setLocalNameDraft(next.name);
    setLocalIconDraft(next.icon);
    onSetLocalPresentation?.(next);
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

      {space && onSetLocalPresentation ? (
        <section className="settings-section" aria-label={t("space.localPresentation")}>
          <h3>{t("space.localPresentation")}</h3>
          <div className="profile-settings-form">
            <label className="profile-settings-field">
              <span>{t("space.localName")}</span>
              <ImeTextField
                value={localNameDraft}
                syncKey={`${space.space_id}:local-name`}
                placeholder={t("space.localNamePlaceholder")}
                onChange={(event) =>
                  updateLocalPresentation({
                    name: event.currentTarget.value,
                    icon: localIconDraft
                  })
                }
              />
            </label>
            <label className="profile-settings-field">
              <span>{t("space.localIcon")}</span>
              <ImeTextField
                value={localIconDraft}
                syncKey={`${space.space_id}:local-icon`}
                placeholder={t("space.localIconPlaceholder")}
                maxLength={12}
                onChange={(event) =>
                  updateLocalPresentation({
                    name: localNameDraft,
                    icon: event.currentTarget.value
                  })
                }
              />
            </label>
            <div className="profile-settings-actions">
              <button
                className="profile-settings-action"
                type="button"
                onClick={() => {
                  setLocalNameDraft("");
                  setLocalIconDraft("");
                  onSetLocalPresentation(null);
                }}
              >
                {t("space.resetLocalPresentation")}
              </button>
            </div>
          </div>
        </section>
      ) : null}

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
