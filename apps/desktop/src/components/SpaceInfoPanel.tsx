import { Bell, ChevronRight, Home, MailPlus, Settings, SlidersHorizontal } from "lucide-react";
import type { ReactNode } from "react";

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
          <h2 id="space-info-title">{title}</h2>
          <p>{space?.space_id ?? "All rooms"}</p>
        </div>
      </header>

      <div className="settings-summary-grid" aria-label="Space summary">
        <SummaryTile label="Rooms" value={String(childRooms.length)} />
        <SummaryTile label="Unread" value={String(unreadTotal)} />
      </div>

      <section className="settings-section" aria-label="Rooms">
        <h3>Rooms</h3>
        <div className="settings-detail-list">
          {childRooms.map((room) => (
            <div className="settings-detail-row" key={room.room_id}>
              <span>{room.display_name}</span>
              <small>{room.unread_count ? `${room.unread_count} unread` : room.room_id}</small>
            </div>
          ))}
        </div>
      </section>

      <section className="settings-section" aria-label="Space preferences">
        <h3>Space preferences</h3>
        <div className="settings-detail-list">
          <DetailRow label="Room membership" value={space ? "Child rooms" : "All rooms"} />
          <DetailRow label="Direct messages" value="Global DM list" />
          <DetailRow label="Notifications" value={unreadTotal ? `${unreadTotal} unread` : "No unread"} />
        </div>
      </section>

      <SettingsEntryList
        entries={[
          { icon: <Home size={16} />, label: "Home" },
          { icon: <SlidersHorizontal size={16} />, label: "Preferences" },
          { icon: <Settings size={16} />, label: "Space settings" },
          { icon: <MailPlus size={16} />, label: "Invite" },
          { icon: <Bell size={16} />, label: "Notifications" }
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
