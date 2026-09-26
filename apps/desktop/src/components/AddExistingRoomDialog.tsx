// #1007: add an existing joined room to the selected Space.
//
// A thin view of the Rust-projected `SidebarModel.space_add_rooms`: Rust owns
// eligibility (parent-side `m.space.child`, never a child-side parent claim),
// the in-flight pair, and each room's added/failed status. This component keeps
// only the unsent search text and renders the rows; the add and retry actions
// dispatch `set_space_child`, and the visible status changes only when a new
// Rust snapshot arrives.

import { useEffect, useState } from "react";
import { t } from "../i18n/messages";
import type { SpaceAddRoomCandidate, SpaceAddRoomsModel } from "../domain/types";
import { operationFailureLabel } from "../app/uiShared";
import { ImeTextField } from "./ImeTextControl";
import { ModalDialog } from "./ModalDialog";

export function AddExistingRoomDialog({
  model,
  spaceName,
  busy,
  onAdd,
  onClose
}: {
  model: SpaceAddRoomsModel;
  spaceName: string;
  /** Another basic operation (e.g. room creation) is in flight. */
  busy: boolean;
  /** Resolves at command admission; rejects when the command is refused. */
  onAdd: (roomId: string) => Promise<unknown>;
  onClose: () => void;
}) {
  const [query, setQuery] = useState("");
  // Presentation-only double-click fence: a clicked row stays busy until the
  // Rust snapshot moves it away from the status it had when clicked.
  const [submitted, setSubmitted] = useState<{ roomId: string; statusKind: string } | null>(null);
  const submittedCandidate = submitted
    ? model.candidates.find((candidate) => candidate.room_id === submitted.roomId)
    : undefined;
  const submittedPending = Boolean(
    submitted && submittedCandidate && submittedCandidate.status.kind === submitted.statusKind
  );
  useEffect(() => {
    if (submitted && !submittedPending) setSubmitted(null);
  }, [submitted, submittedPending]);
  function add(candidate: SpaceAddRoomCandidate) {
    if (submittedPending) return;
    const marker = { roomId: candidate.room_id, statusKind: candidate.status.kind };
    setSubmitted(marker);
    void Promise.resolve(onAdd(candidate.room_id)).catch(() => {
      setSubmitted((current) => (current === marker ? null : current));
    });
  }
  const title = t("spaceAddRooms.title", { spaceName });
  const candidates = model.candidates;
  const normalized = query.trim().toLocaleLowerCase();
  // Text filtering of an already classified and ordered Rust list, as the
  // sidebar room filter does.
  const visible = normalized
    ? candidates.filter((candidate) => candidate.display_name.toLocaleLowerCase().includes(normalized))
    : candidates;
  const anyAdding = candidates.some((candidate) => candidate.status.kind === "adding");

  return (
    <ModalDialog title={title} className="space-add-rooms-modal" onClose={onClose}>
      <div className="space-add-rooms" data-testid="space-add-rooms">
        <p className="profile-settings-hint">{t("spaceAddRooms.scope")}</p>
        <ImeTextField
          className="dialog-input"
          type="search"
          autoFocus
          aria-label={t("spaceAddRooms.search")}
          placeholder={t("spaceAddRooms.search")}
          spellCheck={false}
          value={query}
          syncKey="space-add-rooms-search"
          onChange={(event) => setQuery(event.target.value)}
        />
        <ul className="space-add-rooms-list" aria-label={t("spaceAddRooms.listLabel")}>
          {visible.map((candidate) => (
            <SpaceAddRoomRow
              key={candidate.room_id}
              candidate={candidate}
              spaceName={spaceName}
              disabled={busy || anyAdding || submittedPending}
              onAdd={() => add(candidate)}
            />
          ))}
        </ul>
        {visible.length === 0 ? (
          <p className="space-add-rooms-empty" role="status">
            {candidates.length === 0 ? t("spaceAddRooms.empty") : t("spaceAddRooms.noMatches")}
          </p>
        ) : null}
      </div>
    </ModalDialog>
  );
}

function SpaceAddRoomRow({
  candidate,
  spaceName,
  disabled,
  onAdd
}: {
  candidate: SpaceAddRoomCandidate;
  spaceName: string;
  disabled: boolean;
  onAdd: () => void;
}) {
  const roomName = candidate.display_name;
  const status = candidate.status;
  return (
    <li className="space-add-room" data-status={status.kind}>
      <span className="space-add-room-name" dir="auto">{roomName}</span>
      {status.kind === "failed" ? (
        <span className="space-add-room-failure" role="alert">
          {status.reason === "forbidden"
            ? t("spaceAddRooms.failedForbidden")
            : t("spaceAddRooms.failed", { reason: operationFailureLabel(status.reason) })}
        </span>
      ) : null}
      {status.kind === "added" ? (
        <span className="space-add-room-status">{t("spaceAddRooms.added")}</span>
      ) : status.kind === "adding" ? (
        <span className="space-add-room-status" role="status">{t("spaceAddRooms.adding")}</span>
      ) : (
        <button
          type="button"
          className="dialog-button"
          disabled={disabled}
          aria-label={
            status.kind === "failed"
              ? t("spaceAddRooms.retryAccessible", { roomName })
              : t("spaceAddRooms.addAccessible", { roomName, spaceName })
          }
          onClick={onAdd}
        >
          {status.kind === "failed" ? t("spaceAddRooms.retry") : t("action.add")}
        </button>
      )}
    </li>
  );
}
