import { useState } from "react";
import { useNowMs } from "../../app/useNowMs";
import { ResetLocalDataConfirmationDialog } from "../dialogs";
import { t } from "../../i18n/messages";
import { roomDisplayLabel } from "../../domain/roomDisplayLabel";
import type {
  RoomSummary,
  SearchCrawlerFailureKind,
  SearchCrawlerRoomState,
  SearchCrawlerSettings,
  SearchCrawlerSpeed,
  SearchCrawlerState,
  SettingsPatch
} from "../../domain/types";

// ---------------------------------------------------------------------------
// #77 Search History Crawler section
// ---------------------------------------------------------------------------

export function SearchHistorySection({
  crawlerSettings,
  crawlerState,
  settingsScope = "account",
  rooms,
  isSaving,
  onUpdateSettings,
  onRebuildSearchIndex,
  onStartCrawlRoom,
  onStopCrawlRoom
}: {
  crawlerSettings: SearchCrawlerSettings;
  crawlerState: SearchCrawlerState;
  settingsScope?: "account" | "app";
  rooms?: RoomSummary[];
  isSaving: boolean;
  onUpdateSettings: (patch: SettingsPatch) => void;
  onRebuildSearchIndex?: () => void;
  onStartCrawlRoom?: (roomId: string) => void;
  onStopCrawlRoom?: (roomId: string) => void;
}) {
  // Minute granularity: half a minute keeps "N minutes ago" within one step.
  const nowMs = useNowMs(30_000);
  // #1100: the rebuild guard is an in-app confirmation. `window.confirm` never
  // renders in the Tauri macOS webview, so the destructive action would look
  // like a dead button.
  const [rebuildConfirmOpen, setRebuildConfirmOpen] = useState(false);
  const roomEntries = crawlerRoomEntries(crawlerState.rooms, rooms);
  const crawlerSummary = summarizeCrawlerRooms(roomEntries);
  const crawlerPaused = crawlerSettings.speed === "paused";
  const activeRoomEntry = crawlerPaused
    ? roomEntries.find((entry) => entry.roomState.kind === "queued")
    : roomEntries.find((entry) => entry.roomState.kind === "running") ??
      roomEntries.find((entry) => entry.roomState.kind === "queued");
  const lastActiveEntry = crawlerLastActiveEntry(crawlerState.last_active, rooms);
  const crawlerProgressLabel = crawlerPaused
    ? t("settings.searchHistoryPausedProgress", {
        completed: crawlerSummary.completed,
        total: roomEntries.length
      })
    : t("settings.searchHistoryIndexingProgress", {
        completed: crawlerSummary.completed,
        total: roomEntries.length
      });

  function toggleCrawlerPaused() {
    onUpdateSettings({
      search_crawler: {
        ...crawlerSettings,
        speed: crawlerPaused ? "standard" : "paused"
      }
    });
  }

  function confirmRebuildSearchIndex() {
    setRebuildConfirmOpen(true);
  }

  return (
    <>
      {settingsScope !== "account" ? (
        <div className="settings-control-stack">
          <div className="settings-control-row crawler-speed-row">
            <span>{t("settings.searchHistorySpeed")}</span>
            <div className="segmented-control crawler-speed-control" role="group" aria-label={t("settings.searchHistorySpeed")}>
              {(["standard", "fast", "slow"] as const).map((speed) => (
                <CrawlerSpeedButton
                  key={speed}
                  value={speed}
                  selected={crawlerSettings.speed === speed}
                  disabled={isSaving}
                  onSelect={onUpdateSettings}
                  currentSettings={crawlerSettings}
                />
              ))}
            </div>
          </div>
          <div className="settings-control-row">
            <span>{t("settings.searchHistoryBudget")}</span>
            <button
              className={`dialog-button secondary crawler-pause-button ${crawlerPaused ? "is-active" : ""}`}
              type="button"
              disabled={isSaving}
              aria-pressed={crawlerPaused}
              data-active={crawlerPaused ? "true" : "false"}
              onClick={toggleCrawlerPaused}
            >
              {crawlerPaused ? t("settings.searchHistoryResume") : t("settings.searchHistoryPause")}
            </button>
          </div>
          <p className="settings-notice">{t("settings.searchHistoryBudgetDescription")}</p>
        </div>
      ) : null}
      {settingsScope !== "app" ? (
        <div className="settings-control-stack">
          <div className="settings-control-row">
            <span>{t("settings.searchHistoryCrawler")}</span>
            <div className="crawler-action-row">
              <button
                className="dialog-button danger"
                type="button"
                disabled={isSaving || !onRebuildSearchIndex}
                onClick={confirmRebuildSearchIndex}
              >
                {t("settings.searchHistoryRebuild")}
              </button>
              {isSaving ? <span className="settings-save-state crawler-control-status">{t("settings.saving")}</span> : null}
            </div>
          </div>
        </div>
      ) : null}
      {settingsScope !== "app" ? <>
      <section
        className="settings-section crawler-activity-section"
      aria-label={t("settings.searchHistoryActivity")}
    >
      <div className="settings-section-heading">
        <h4 className="settings-subheading">{t("settings.searchHistoryActivity")}</h4>
        <span className="settings-save-state crawler-progress-state">{crawlerProgressLabel}</span>
      </div>
      <p className="settings-muted-note crawler-activity-summary">
        {t("settings.searchHistoryActivitySummary", crawlerSummary)}
      </p>
      {activeRoomEntry ? (
        <div className="settings-detail-list compact crawler-activity-list">
          <CrawlerRoomRow
            roomId={activeRoomEntry.roomId}
            displayLabel={activeRoomEntry.displayLabel}
              roomState={activeRoomEntry.roomState}
              onStart={onStartCrawlRoom}
              onStop={onStopCrawlRoom}
              showActions={false}
            />
          </div>
        ) : lastActiveEntry ? (
          <p className="settings-muted-note">
            {t("settings.searchHistoryActivityLastIndexed", {
              room: lastActiveEntry.displayLabel,
              age: crawlerActivityAgeLabel(lastActiveEntry.updatedAtMs, nowMs)
            })}
          </p>
        ) : (
          <p className="settings-muted-note">{t("settings.searchHistoryActivityIdle")}</p>
        )}
        <p className="settings-muted-note">{t("settings.searchHistoryActivityHint")}</p>
      </section>
      <div className="settings-toggle-list">
        <CrawlerToggle
          label={t("settings.searchHistoryIncludeCaptions")}
          settingKey="include_media_captions"
          current={crawlerSettings}
          disabled={isSaving}
          onSelect={onUpdateSettings}
        />
        <CrawlerToggle
          label={t("settings.searchHistoryIncludeFilenames")}
          settingKey="include_filenames"
          current={crawlerSettings}
          disabled={isSaving}
          onSelect={onUpdateSettings}
        />
      </div>
      {roomEntries.length > 0 ? (
        <section
          className="settings-section crawler-room-status-panel"
          aria-label={t("settings.searchHistoryRoomStatus")}
        >
          <h4 className="settings-subheading">{t("settings.searchHistoryRoomStatus")}</h4>
          <div className="settings-detail-list crawler-room-status-list">
            {roomEntries.map(({ roomId, roomState, displayLabel }) => {
              return (
                <CrawlerRoomRow
                  key={roomId}
                  roomId={roomId}
                  displayLabel={displayLabel}
                  roomState={roomState}
                  onStart={onStartCrawlRoom}
                  onStop={onStopCrawlRoom}
                  showActions={true}
                />
              );
            })}
          </div>
        </section>
      ) : null}
      </> : null}
      {rebuildConfirmOpen ? (
        <ResetLocalDataConfirmationDialog
          isBusy={false}
          title={t("settings.searchHistoryRebuild")}
          copy={t("settings.searchHistoryRebuildConfirm")}
          confirmLabel={t("settings.searchHistoryRebuild")}
          onCancel={() => setRebuildConfirmOpen(false)}
          onConfirm={() => {
            setRebuildConfirmOpen(false);
            onRebuildSearchIndex?.();
          }}
        />
      ) : null}
    </>
  );
}

function CrawlerSpeedButton({
  value,
  selected,
  disabled,
  onSelect,
  currentSettings
}: {
  value: SearchCrawlerSpeed;
  selected: boolean;
  disabled: boolean;
  onSelect: (patch: SettingsPatch) => void;
  currentSettings: SearchCrawlerSettings;
}) {
  const label = crawlerSpeedLabel(value);
  return (
    <button
      className={`segmented-control-option crawler-speed-option ${selected ? "is-selected" : ""}`}
      type="button"
      aria-label={selected ? `${label}, ${t("settings.searchHistorySpeedCurrent")}` : label}
      aria-pressed={selected}
      disabled={disabled}
      data-speed={value}
      onClick={() =>
        onSelect({ search_crawler: { ...currentSettings, speed: value } })
      }
    >
      <span>{label}</span>
      {selected ? <small>{t("settings.searchHistorySpeedCurrent")}</small> : null}
    </button>
  );
}

type CrawlerRoomEntry = {
  roomId: string;
  displayLabel: string | null;
  roomState: SearchCrawlerRoomState;
};

type CrawlerLastActiveEntry = {
  roomId: string;
  displayLabel: string;
  updatedAtMs: number;
};

function crawlerRoomEntries(
  roomStates: Record<string, SearchCrawlerRoomState>,
  rooms?: RoomSummary[]
): CrawlerRoomEntry[] {
  const labels = new Map((rooms ?? []).map((room) => [room.room_id, roomDisplayLabel(room)]));
  return Object.entries(roomStates)
    .map(([roomId, roomState]) => ({
      roomId,
      roomState,
      displayLabel: labels.get(roomId) ?? null
    }))
    .sort((a, b) => {
      const rank = crawlerRoomRank(a.roomState) - crawlerRoomRank(b.roomState);
      if (rank !== 0) {
        return rank;
      }
      return (a.displayLabel ?? "").localeCompare(b.displayLabel ?? "");
    });
}

function crawlerRoomRank(roomState: SearchCrawlerRoomState): number {
  switch (roomState.kind) {
    case "running":
      return 0;
    case "queued":
      return 1;
    case "idle":
      return 2;
    case "failed":
      return 3;
    case "completed":
      return 4;
  }
}

function summarizeCrawlerRooms(entries: CrawlerRoomEntry[]) {
  return entries.reduce(
    (summary, entry) => ({
      ...summary,
      [entry.roomState.kind]: summary[entry.roomState.kind] + 1
    }),
    { running: 0, idle: 0, completed: 0, failed: 0, queued: 0 }
  );
}

function crawlerLastActiveEntry(
  lastActive: SearchCrawlerState["last_active"],
  rooms?: RoomSummary[]
): CrawlerLastActiveEntry | null {
  if (!lastActive || lastActive.status !== "completed") {
    return null;
  }
  const room = rooms?.find((candidate) => candidate.room_id === lastActive.room_id);
  return {
    roomId: lastActive.room_id,
    displayLabel: room ? roomDisplayLabel(room) : t("settings.searchHistoryRoomUnknown"),
    updatedAtMs: lastActive.updated_at_ms
  };
}

function crawlerActivityAgeLabel(timestampMs: number, nowMs = Date.now()): string {
  const elapsedSeconds = Math.max(0, Math.floor((nowMs - timestampMs) / 1000));
  if (elapsedSeconds < 60) {
    return t("settings.searchHistoryActivityJustNow");
  }
  const elapsedMinutes = Math.floor(elapsedSeconds / 60);
  if (elapsedMinutes < 60) {
    return t("settings.searchHistoryActivityMinutesAgo", { count: elapsedMinutes });
  }
  const elapsedHours = Math.floor(elapsedMinutes / 60);
  if (elapsedHours < 24) {
    return t("settings.searchHistoryActivityHoursAgo", { count: elapsedHours });
  }
  return t("settings.searchHistoryActivityDaysAgo", { count: Math.floor(elapsedHours / 24) });
}

function CrawlerToggle({
  label,
  settingKey,
  current,
  disabled,
  onSelect
}: {
  label: string;
  settingKey: "include_media_captions" | "include_filenames";
  current: SearchCrawlerSettings;
  disabled: boolean;
  onSelect: (patch: SettingsPatch) => void;
}) {
  return (
    <button
      className="settings-toggle-row"
      type="button"
      role="switch"
      aria-checked={current[settingKey]}
      disabled={disabled}
      onClick={() =>
        onSelect({ search_crawler: { ...current, [settingKey]: !current[settingKey] } })
      }
    >
      <span className="settings-toggle-copy">
        <span className="settings-toggle-label">
          <span>{label}</span>
        </span>
      </span>
      <span className="settings-switch-track" aria-hidden="true">
        <span className="settings-switch-thumb" />
      </span>
    </button>
  );
}

function CrawlerRoomRow({
  roomId,
  displayLabel,
  roomState,
  onStart,
  onStop,
  showActions = true
}: {
  roomId: string;
  /** Rust-projected display label from RoomSummary; never render the raw roomId. */
  displayLabel: string | null;
  roomState: SearchCrawlerRoomState;
  onStart?: (roomId: string) => void;
  onStop?: (roomId: string) => void;
  showActions?: boolean;
}) {
  const statusLabel = crawlerRoomStatusLabel(roomState);
  const isRunning = roomState.kind === "running";
  // displayLabel is the Rust-projected label; fall back to a neutral placeholder
  // (never the raw room id, which is a private identifier).
  const visibleLabel = displayLabel ?? t("settings.searchHistoryRoomUnknown");

  return (
    <div className="settings-detail-row">
      <span dir="auto">{visibleLabel}</span>
      <small data-crawler-room-kind={roomState.kind}>{statusLabel}</small>
      {showActions && isRunning && onStop ? (
        <button
          className="profile-settings-action"
          type="button"
          aria-label={t("settings.searchHistoryStopRoom")}
          onClick={() => onStop(roomId)}
        >
          {t("settings.searchHistoryStopRoom")}
        </button>
      ) : showActions && roomState.kind === "completed" && onStart ? (
        // #996: a completed room can still miss messages that arrived while
        // the app was closed, so it can be indexed again on its own.
        <button
          className="profile-settings-action"
          type="button"
          aria-label={t("settings.searchHistoryReindexRoom")}
          onClick={() => onStart(roomId)}
        >
          {t("settings.searchHistoryReindexRoom")}
        </button>
      ) : showActions && !isRunning && onStart ? (
        <button
          className="profile-settings-action"
          type="button"
          aria-label={t("settings.searchHistoryStartRoom")}
          onClick={() => onStart(roomId)}
        >
          {t("settings.searchHistoryStartRoom")}
        </button>
      ) : null}
    </div>
  );
}

function crawlerSpeedLabel(speed: SearchCrawlerSpeed): string {
  switch (speed) {
    case "standard":
      return t("settings.searchHistorySpeedStandard");
    case "fast":
      return t("settings.searchHistorySpeedFast");
    case "slow":
      return t("settings.searchHistorySpeedSlow");
    case "paused":
      return t("settings.searchHistorySpeedPaused");
  }
}

function crawlerRoomStatusLabel(state: SearchCrawlerRoomState): string {
  switch (state.kind) {
    case "idle":
      return t("settings.searchHistoryRoomIdle");
    case "queued":
      return t("settings.searchHistoryRoomQueued");
    case "running":
      return t("settings.searchHistoryRoomRunning", {
        processed: state.processed,
        indexed: state.indexed
      });
    case "completed":
      return t("settings.searchHistoryRoomCompleted", { indexed: state.indexed });
    case "failed":
      return t("settings.searchHistoryRoomFailed") + ` (${crawlerFailureKindLabel(state.failureKind)})`;
  }
}

function crawlerFailureKindLabel(kind: SearchCrawlerFailureKind): string {
  switch (kind) {
    case "roomNotFound":
      return "roomNotFound";
    case "sdk":
      return "sdk";
    case "decryption":
      return "decryption";
    case "indexUnavailable":
      return "indexUnavailable";
  }
}
