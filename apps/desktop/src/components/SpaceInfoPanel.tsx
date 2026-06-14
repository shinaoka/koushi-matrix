import { Bell, ChevronRight, Home, MailPlus, Settings, SlidersHorizontal } from "lucide-react";
import type { ReactNode } from "react";

import { t } from "../i18n/messages";
import type { RoomSummary, SpaceSummary } from "../domain/types";

export function SpaceInfoPanel({
  fallbackName,
  rooms,
  space
}: {
  fallbackName: string;
  rooms: RoomSummary[];
  space: SpaceSummary | null;
}) {
  const childRooms = space
    ? space.child_room_ids
        .map((roomId) => rooms.find((room) => room.room_id === roomId))
        .filter((room): room is RoomSummary => Boolean(room && !room.is_dm))
    : rooms.filter((room) => !room.is_dm);
  const unreadTotal = childRooms.reduce((sum, room) => sum + room.unread_count, 0);
  const title = space?.display_name ?? fallbackName;

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
        <SummaryTile label={t("room.unread")} value={String(unreadTotal)} />
      </div>

      <section className="settings-section" aria-label={t("workspace.rooms")}>
        <h3>{t("workspace.rooms")}</h3>
        <div className="settings-detail-list">
          {childRooms.map((room) => (
            <div className="settings-detail-row" key={room.room_id}>
              <span dir="auto">{room.display_name}</span>
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
          { icon: <MailPlus size={16} />, label: t("space.invite") },
          { icon: <Bell size={16} />, label: t("room.notifications") }
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
  entries: Array<{ icon: ReactNode; label: string }>;
}) {
  return (
    <div className="settings-list">
      {entries.map((entry) => (
        <button className="settings-list-item" key={entry.label} type="button">
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
