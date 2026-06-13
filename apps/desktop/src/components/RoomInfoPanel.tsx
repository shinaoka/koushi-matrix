import { Bell, ChevronRight, FileText, Settings, Users } from "lucide-react";
import type { ReactNode } from "react";

import { t } from "../i18n/messages";
import type { RoomSummary, SpaceSummary } from "../domain/types";

export function RoomInfoPanel({
  room,
  spaces
}: {
  room: RoomSummary | null;
  spaces: SpaceSummary[];
}) {
  if (!room) {
    return (
      <section className="settings-panel" aria-labelledby="room-info-title">
        <header className="settings-panel-header">
          <div>
            <h2 id="room-info-title">{t("room.roomInfo")}</h2>
            <p>No room selected</p>
          </div>
        </header>
      </section>
    );
  }

  const parentSpaces = spaces.filter((space) => room.parent_space_ids.includes(space.space_id));

  return (
    <section className="settings-panel room-info-panel" aria-labelledby="room-info-title">
      <header className="settings-panel-header">
        <div>
          <h2 id="room-info-title">{room.display_name}</h2>
          <p>{room.room_id}</p>
        </div>
      </header>

      <div className="settings-summary-grid" aria-label="Room summary">
        <SummaryTile label="Type" value={room.is_dm ? "Direct message" : "Room"} />
        <SummaryTile label="Unread" value={String(room.unread_count)} />
        <SummaryTile label="Spaces" value={parentSpaces.length ? String(parentSpaces.length) : "No Spaces"} />
      </div>

      <section className="settings-section" aria-label="Spaces">
        <h3>Spaces</h3>
        <div className="settings-detail-list">
          {parentSpaces.length ? (
            parentSpaces.map((space) => (
              <div className="settings-detail-row" key={space.space_id}>
                <span>{space.display_name}</span>
                <small>{space.space_id}</small>
              </div>
            ))
          ) : (
            <div className="settings-detail-row">
              <span>No Spaces</span>
            </div>
          )}
        </div>
      </section>

      <section className="settings-section" aria-label="Room settings">
        <h3>Room settings</h3>
        <div className="settings-detail-list">
          <DetailRow label="Timeline" value="Subscribed" />
          <DetailRow label="Search index" value="Exact verified results" />
          <DetailRow label="DM list" value={room.is_dm ? "Global DM list" : "Room scoped"} />
        </div>
      </section>

      <SettingsEntryList
        entries={[
          { icon: <Users size={16} />, label: "People" },
          { icon: <FileText size={16} />, label: "Files" },
          { icon: <Bell size={16} />, label: "Notifications" },
          { icon: <Settings size={16} />, label: "Room settings" }
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
