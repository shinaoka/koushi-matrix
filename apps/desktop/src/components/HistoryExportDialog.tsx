import { AlertTriangle, Download } from "lucide-react";
import { useEffect, useState } from "react";

import { t, type MessageId } from "../i18n/messages";
import { ModalDialog } from "./ModalDialog";
import { ImeSafeForm } from "./ImeTextControl";
import type {
  HistoryExportFailureKind,
  HistoryExportRangeInput,
  HistoryExportRoom,
  HistoryExportRoomFailureKind,
  HistoryExportScopeInput,
  HistoryExportStart,
  HistoryExportState
} from "../domain/types";

/** Platform capabilities for the history export. The Tauri adapter owns the
 * time zone and the folder dialog; Rust owns the export. */
export interface HistoryExportControls {
  loadTimeZone: () => Promise<string>;
  /** Opens the native folder dialog and, unless it is dismissed, submits the
   * export. Resolves after the admitted snapshot has been applied. */
  start: (
    scope: HistoryExportScopeInput,
    range: HistoryExportRangeInput,
    dialogTitle: string,
    folderNameStem: string
  ) => Promise<HistoryExportStart>;
  stop: (requestId: number) => Promise<void>;
  /** Resumes the folder of a settled export under a new request. */
  retry: (requestId: number) => Promise<HistoryExportStart>;
}

/** The room or Space a dialog exports. */
export type HistoryExportTarget =
  | { kind: "room"; roomId: string; name: string; encrypted: boolean }
  | { kind: "space"; spaceId: string; name: string };

type RangeKind = HistoryExportRangeInput["kind"];
type InFlight = Extract<HistoryExportState, { kind: "preparing" | "running" }>;
type Settled = Extract<HistoryExportState, { kind: "completed" | "stopped" | "failed" }>;

const EARLIEST_DATE = "1970-01-01";

const FAILURE_MESSAGES: Record<HistoryExportFailureKind, MessageId> = {
  invalidRange: "historyExport.failedInvalidRange",
  roomNotFound: "historyExport.failedRoomNotFound",
  spaceNotFound: "historyExport.failedSpaceNotFound",
  destinationUnavailable: "historyExport.failedDestination",
  manifestMismatch: "historyExport.failedManifestMismatch",
  write: "historyExport.failedWrite",
  noSpace: "historyExport.failedNoSpace",
  network: "historyExport.failedNetwork",
  sdk: "historyExport.failedSdk"
};

const ROOM_FAILURE_MESSAGES: Record<HistoryExportRoomFailureKind, MessageId> = {
  network: "historyExport.roomFailedNetwork",
  sdk: "historyExport.roomFailedSdk",
  write: "historyExport.roomFailedWrite"
};

/** Today's civil date in `timeZone`, as the `YYYY-MM-DD` a date input uses. */
function civilToday(timeZone: string): string {
  try {
    return new Intl.DateTimeFormat("en-CA", { timeZone, year: "numeric", month: "2-digit", day: "2-digit" })
      .format(new Date());
  } catch {
    return "";
  }
}

function scopeInput(target: HistoryExportTarget): HistoryExportScopeInput {
  return target.kind === "room" ? { kind: "room", roomId: target.roomId } : { kind: "space", spaceId: target.spaceId };
}

/** Whether the Rust state belongs to an export of `target`. */
export function historyExportIsFor(state: HistoryExportState, target: HistoryExportTarget): boolean {
  if (state.kind === "idle") return false;
  return target.kind === "room"
    ? state.scope.kind === "room" && state.scope.room_id === target.roomId
    : state.scope.kind === "space" && state.scope.space_id === target.spaceId;
}

function isInFlight(state: HistoryExportState): state is InFlight {
  return state.kind === "preparing" || state.kind === "running";
}

function settledRooms(rooms: HistoryExportRoom[]): number {
  return rooms.filter((room) => room.phase === "completed" || room.phase === "skipped" || room.phase === "failed")
    .length;
}

function progressLine(state: InFlight): string {
  if (state.stop_requested) return t("historyExport.stopping");
  if (state.kind === "preparing") return t("historyExport.preparing");
  return t("historyExport.roomsProgress", { done: settledRooms(state.rooms), total: state.rooms.length });
}

function resultLines(state: Settled): string[] {
  const failedRooms = state.rooms.filter((room) => room.phase === "failed").length;
  switch (state.kind) {
    case "completed":
      return [
        failedRooms > 0
          ? t("historyExport.completedWithFailures", { count: failedRooms })
          : t("historyExport.completed")
      ];
    case "stopped":
      return [t("historyExport.stopped")];
    case "failed":
      return [t("historyExport.failed"), t(FAILURE_MESSAGES[state.failure_kind])];
  }
}

/** Room or Space info's summary of the latest export of `target`: progress
 * while it runs, then its Rust-reported outcome until the next export. */
export function historyExportSummary(state: HistoryExportState, target: HistoryExportTarget): string[] {
  if (!historyExportIsFor(state, target)) return [];
  if (isInFlight(state)) {
    if (state.stop_requested) return [t("historyExport.stopping")];
    if (state.kind === "preparing") return [t("historyExport.preparing")];
    return [t("historyExport.summaryRunning", { done: settledRooms(state.rooms), total: state.rooms.length })];
  }
  return resultLines(state as Settled);
}

function roomPhaseText(room: HistoryExportRoom): string {
  const counts = room.counts;
  switch (room.phase) {
    case "pending":
      return t("historyExport.phasePending");
    case "fetching":
      return t("historyExport.phaseFetching", { fetched: counts.fetched_events });
    case "attachments":
      return t("historyExport.phaseAttachments", { done: counts.attachments_done, total: counts.attachments_total });
    case "rendering":
      return t("historyExport.phaseRendering");
    case "completed":
      return t("historyExport.phaseCompleted", {
        exported: counts.exported_events,
        attachments: counts.attachments_done - counts.attachments_failed
      });
    case "skipped":
      return t("historyExport.phaseSkipped");
    case "failed":
      return room.failure_kind
        ? `${t("historyExport.phaseFailed")}: ${t(ROOM_FAILURE_MESSAGES[room.failure_kind])}`
        : t("historyExport.phaseFailed");
  }
}

function RoomList({ rooms }: { rooms: HistoryExportRoom[] }) {
  if (rooms.length === 0) return null;
  return (
    <ul className="export-progress-list" data-testid="history-export-rooms">
      {rooms.map((room) => (
        <li key={room.room_id} data-phase={room.phase}>
          <span className="export-progress-name" dir="auto">{room.display_name}</span>
          <span className="export-progress-phase">{roomPhaseText(room)}</span>
          {room.phase === "completed" && room.counts.attachments_failed > 0 ? (
            <span className="export-progress-note">
              {t("historyExport.attachmentsFailed", { count: room.counts.attachments_failed })}
            </span>
          ) : null}
          {room.phase === "completed" && room.counts.undecryptable_events > 0 ? (
            <span className="export-progress-note">
              {t("historyExport.undecryptable", { count: room.counts.undecryptable_events })}
            </span>
          ) : null}
        </li>
      ))}
    </ul>
  );
}

export function HistoryExportDialog({
  target,
  exportState,
  controls,
  onClose
}: {
  target: HistoryExportTarget;
  exportState: HistoryExportState;
  controls: HistoryExportControls;
  onClose: () => void;
}) {
  const [timeZone, setTimeZone] = useState<string | null>(null);
  const [rangeKind, setRangeKind] = useState<RangeKind>("allAvailable");
  const [startDate, setStartDate] = useState("");
  const [endDate, setEndDate] = useState("");
  // Presentation only: the request this dialog started, so that its Rust
  // settlement is shown and an earlier export's result is not.
  const [submittedRequestId, setSubmittedRequestId] = useState<number | null>(null);
  const [starting, setStarting] = useState(false);
  const [startFailed, setStartFailed] = useState(false);
  // Presentation only: the user chose to start a new download over an
  // earlier export's result.
  const [showForm, setShowForm] = useState(false);

  useEffect(() => {
    let current = true;
    controls.loadTimeZone().then(
      (zone) => {
        if (!current) return;
        setTimeZone(zone);
        const today = civilToday(zone);
        setStartDate((value) => value || today);
        setEndDate((value) => value || today);
      },
      () => {
        if (current) setTimeZone(null);
      }
    );
    return () => {
      current = false;
    };
  }, [controls]);

  const forTarget = historyExportIsFor(exportState, target);
  const inFlightHere = forTarget && isInFlight(exportState) ? exportState : null;
  const busyElsewhere = isInFlight(exportState) && !forTarget;
  // The latest settled export of this target: the one this dialog started,
  // or, when the dialog is reopened, the one that finished while it was
  // closed, until the user asks for a new download.
  const settledForTarget =
    forTarget && !isInFlight(exportState) && exportState.kind !== "idle" ? (exportState as Settled) : null;
  const settledHere =
    settledForTarget &&
    (submittedRequestId !== null ? settledForTarget.request_id === submittedRequestId : !showForm)
      ? settledForTarget
      : null;
  // A rejected start leaves the Rust state unchanged. Core admits a command
  // only after handling it, so the admitted snapshot then holds neither this
  // request in flight nor its settlement.
  const notStarted =
    startFailed ||
    (submittedRequestId !== null &&
      !starting &&
      !(inFlightHere && inFlightHere.request_id === submittedRequestId) &&
      settledHere === null);
  // Instants before the Unix epoch cannot be exported; the adapter rejects them.
  const periodTooEarly = (startDate !== "" && startDate < EARLIEST_DATE) || (endDate !== "" && endDate < EARLIEST_DATE);
  const periodValid = startDate !== "" && endDate !== "" && startDate <= endDate && !periodTooEarly;
  const canSave =
    !starting && !busyElsewhere && (rangeKind === "allAvailable" || (timeZone !== null && periodValid));

  async function submit(run: () => Promise<HistoryExportStart>) {
    setShowForm(true);
    setStarting(true);
    setStartFailed(false);
    setSubmittedRequestId(null);
    try {
      const started = await run();
      if (started.kind === "submitted") setSubmittedRequestId(started.requestId);
    } catch {
      setStartFailed(true);
    } finally {
      setStarting(false);
    }
  }

  function save() {
    if (!canSave) return;
    const range: HistoryExportRangeInput =
      rangeKind === "period" && timeZone !== null
        ? { kind: "period", startDate, endDate, timeZone }
        : { kind: "allAvailable" };
    void submit(() =>
      controls.start(scopeInput(target), range, t("historyExport.folderDialogTitle"), target.name)
    );
  }

  function retry(requestId: number) {
    void submit(() => controls.retry(requestId));
  }

  function startOver() {
    setSubmittedRequestId(null);
    setStartFailed(false);
    setShowForm(true);
  }

  const title = t(target.kind === "space" ? "historyExport.spaceTitle" : "historyExport.title");
  const showPlaintextWarning = target.kind === "space" || target.encrypted;

  return (
    <ModalDialog title={title} className="room-history-export-modal" dismissible={!starting} onClose={onClose}>
      <div className="room-history-export-content">
        {inFlightHere ? (
          <>
            <p role="status" data-testid="history-export-state">{progressLine(inFlightHere)}</p>
            {inFlightHere.kind === "running" ? <RoomList rooms={inFlightHere.rooms} /> : null}
            <p className="profile-settings-hint">{t("historyExport.continuesInBackground")}</p>
            <div className="dialog-actions">
              <button
                type="button"
                className="dialog-button"
                disabled={inFlightHere.stop_requested}
                onClick={() => {
                  // A failed submit leaves the Rust state, and so this view, unchanged.
                  controls.stop(inFlightHere.request_id).catch(() => undefined);
                }}
              >
                {t("historyExport.stop")}
              </button>
            </div>
          </>
        ) : settledHere ? (
          <>
            <div role="status" data-testid="history-export-state" data-export-result={settledHere.kind}>
              {resultLines(settledHere).map((line) => (
                <p key={line}>{line}</p>
              ))}
            </div>
            <RoomList rooms={settledHere.rooms} />
            <div className="dialog-actions">
              {settledHere.kind === "stopped" ? (
                <button type="button" className="dialog-button" onClick={() => retry(settledHere.request_id)}>
                  {t("historyExport.resume")}
                </button>
              ) : settledHere.kind === "failed" || settledHere.rooms.some((room) => room.phase === "failed") ? (
                <button type="button" className="dialog-button" onClick={() => retry(settledHere.request_id)}>
                  {t("historyExport.retryFailed")}
                </button>
              ) : null}
              <button type="button" className="dialog-button" onClick={startOver}>
                {t("historyExport.again")}
              </button>
              <button type="button" className="dialog-button is-primary" onClick={onClose}>
                {t("action.done")}
              </button>
            </div>
          </>
        ) : (
          <ImeSafeForm
            className="room-history-export-form"
            aria-label={title}
            onSubmit={(event) => {
              event.preventDefault();
              save();
            }}
          >
            <div className="create-room-visibility" role="radiogroup" aria-label={t("historyExport.range")}>
              <label className="create-room-option">
                <input
                  type="radio"
                  name="history-export-range"
                  checked={rangeKind === "allAvailable"}
                  onChange={() => setRangeKind("allAvailable")}
                />
                <span>{t("historyExport.rangeAll")}</span>
              </label>
              <label className="create-room-option">
                <input
                  type="radio"
                  name="history-export-range"
                  checked={rangeKind === "period"}
                  disabled={timeZone === null}
                  onChange={() => setRangeKind("period")}
                />
                <span>{t("historyExport.rangePeriod")}</span>
              </label>
            </div>
            {rangeKind === "period" && timeZone !== null ? (
              <div className="room-history-export-period">
                <label className="room-history-export-date">
                  <span>{t("historyExport.startDate")}</span>
                  <input
                    aria-label={t("historyExport.startDate")}
                    type="date"
                    value={startDate}
                    onChange={(event) => setStartDate(event.currentTarget.value)}
                  />
                </label>
                <label className="room-history-export-date">
                  <span>{t("historyExport.endDate")}</span>
                  <input
                    aria-label={t("historyExport.endDate")}
                    type="date"
                    value={endDate}
                    onChange={(event) => setEndDate(event.currentTarget.value)}
                  />
                </label>
                <p className="profile-settings-hint" data-testid="history-export-time-zone">
                  {t("historyExport.timeZone", { timeZone })}
                </p>
                {!periodValid ? (
                  <p className="profile-settings-hint error" role="alert">
                    {t(periodTooEarly ? "historyExport.periodTooEarly" : "historyExport.invalidPeriod")}
                  </p>
                ) : null}
              </div>
            ) : null}
            <p className="profile-settings-hint">{t("historyExport.availability")}</p>
            <p className="room-history-export-warning" role="note" data-testid="history-export-size-warning">
              <AlertTriangle size={14} aria-hidden="true" />
              <span>{t("historyExport.attachments")}</span>
            </p>
            {showPlaintextWarning ? (
              <p className="room-history-export-warning" role="note" data-testid="history-export-plaintext-warning">
                <AlertTriangle size={14} aria-hidden="true" />
                <span>{t("historyExport.plaintextWarning")}</span>
              </p>
            ) : null}
            <p className="profile-settings-hint">{t("historyExport.resumeHint")}</p>
            {busyElsewhere ? (
              <p className="profile-settings-hint" role="status">{t("historyExport.busy")}</p>
            ) : null}
            {notStarted ? (
              <p className="profile-settings-hint error" role="alert">{t("historyExport.notStarted")}</p>
            ) : null}
            <div className="dialog-actions">
              <button type="button" className="dialog-button" disabled={starting} onClick={onClose}>
                {t("action.cancel")}
              </button>
              <button type="submit" className="dialog-button is-primary" disabled={!canSave}>
                {t("historyExport.save")}
              </button>
            </div>
          </ImeSafeForm>
        )}
      </div>
    </ModalDialog>
  );
}

/** Room or Space info section that opens the export dialog. */
export function HistoryExportSection({
  target,
  exportState,
  controls
}: {
  target: HistoryExportTarget;
  exportState: HistoryExportState;
  controls: HistoryExportControls;
}) {
  const [open, setOpen] = useState(false);
  const section = t(target.kind === "space" ? "historyExport.spaceSection" : "historyExport.section");
  return (
    <section className="settings-section" aria-label={section}>
      <h3>{section}</h3>
      <div className="room-key-actions">
        <button className="profile-settings-action" type="button" onClick={() => setOpen(true)}>
          <Download size={16} aria-hidden="true" />
          <span>{t("historyExport.open")}</span>
        </button>
        <p className="profile-settings-hint">
          {t(target.kind === "space" ? "historyExport.spaceHint" : "historyExport.hint")}
        </p>
        <div role="status" data-testid="history-export-summary">
          {historyExportSummary(exportState, target).map((line) => (
            <p className="profile-settings-hint" key={line}>{line}</p>
          ))}
        </div>
      </div>
      {open ? (
        <HistoryExportDialog target={target} exportState={exportState} controls={controls} onClose={() => setOpen(false)} />
      ) : null}
    </section>
  );
}
