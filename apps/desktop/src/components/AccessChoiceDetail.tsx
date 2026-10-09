import {
  type KeyboardEvent,
  type ReactNode,
  type Ref,
  useId,
  useImperativeHandle,
  useRef
} from "react";

import type { MessageId } from "../i18n/messages";
import { t } from "../i18n/messages";
import type { RoomAccessOutcome, RoomAccessOutcomeLine } from "../domain/types";
import { SettingsPropertyCard, type PropertySaveStatus } from "./SettingsPropertyCard";

/** One choice of an access/history panel: a short label plus a one-line summary. */
export interface AccessChoiceDetailChoice {
  value: string;
  label: string;
  summary: string;
  /** A choice that cannot be selected; `disabledReason` states the concrete reason. */
  disabled?: boolean;
  disabledReason?: string;
}

/**
 * Issue #1177: the shared choice-and-detail editor over the Rust-owned access
 * draft, used by Room Info's access/history section and by the create dialog.
 *
 * Left: every relevant choice with a short label and a one-line summary, the
 * selected one marked. Right: the Rust outcome for the current draft, stating
 * whether it describes confirmed or unsaved values. Native radios keep keyboard
 * navigation and visible focus; every control is reachable at narrow and short
 * sizes because the two panes stack.
 */
export function AccessChoiceDetail({
  property,
  label,
  headingRef,
  choices,
  selected,
  details,
  detailsConfirmed,
  canEdit,
  busy = false,
  readOnlyReason,
  rejection,
  status = null,
  saveEnabled = true,
  saveLabel,
  notes,
  onSelect,
  onSave,
  onCancel
}: {
  /** Stable, unlocalized property name used by QA and tests. */
  property: string;
  label: string;
  headingRef?: Ref<HTMLHeadingElement>;
  choices: readonly AccessChoiceDetailChoice[];
  /** The currently selected value: the Rust draft while editing, else confirmed. */
  selected: string;
  /** The Rust outcome for this context. */
  details: ReactNode;
  /** Whether `details` describes confirmed values (false: unsaved). */
  detailsConfirmed: boolean;
  canEdit: boolean;
  busy?: boolean;
  readOnlyReason?: string | null;
  /** A typed Rust rejection for the current draft, shown in place of the actions. */
  rejection?: string | null;
  status?: PropertySaveStatus;
  /** Whether a valid real change may be committed (the caller owns the comparison). */
  saveEnabled?: boolean;
  /** Accessible name of the Save button. */
  saveLabel: string;
  /** Extra property-specific notes (encryption/history caveats). */
  notes?: ReactNode;
  onSelect: (value: string) => void;
  onSave?: () => void;
  onCancel?: () => void;
}) {
  const groupId = useId();
  const detailsId = useId();
  const heading = useRef<HTMLHeadingElement>(null);
  useImperativeHandle(headingRef, () => heading.current as HTMLHeadingElement);

  function onKeyDown(event: KeyboardEvent<HTMLFieldSetElement>) {
    if (event.key === "Escape" && !event.nativeEvent.isComposing && onCancel) {
      event.preventDefault();
      onCancel();
      heading.current?.focus();
    }
  }

  const actions =
    canEdit && onSave && onCancel ? (
      <div className="profile-settings-actions">
        <button
          className="profile-settings-action"
          type="button"
          aria-label={saveLabel}
          disabled={busy || !saveEnabled || Boolean(rejection)}
          onClick={() => {
            onSave();
            heading.current?.focus();
          }}
        >
          {t("settings.propertySave")}
        </button>
        <button
          className="profile-settings-action"
          type="button"
          disabled={busy}
          onClick={() => {
            onCancel();
            heading.current?.focus();
          }}
        >
          {t("action.cancel")}
        </button>
      </div>
    ) : null;

  return (
    <SettingsPropertyCard
      property={property}
      label={label}
      headingRef={heading}
      status={status}
    >
      <fieldset
        className="access-choice-detail"
        aria-describedby={detailsId}
        onKeyDown={onKeyDown}
      >
        <legend className="access-choice-detail-legend">{t("room.accessChooseLabel")}</legend>
        <div className="access-choice-list" role="radiogroup" aria-label={t("room.accessChooseLabel")}>
          {choices.map((choice) => {
            const choiceId = `${groupId}-${choice.value}`;
            return (
              <label
                className="access-choice-item"
                key={choice.value}
                htmlFor={choiceId}
                data-selected={choice.value === selected ? "true" : "false"}
              >
                <input
                  id={choiceId}
                  type="radio"
                  name={groupId}
                  value={choice.value}
                  checked={choice.value === selected}
                  disabled={!canEdit || busy || choice.disabled}
                  aria-describedby={detailsId}
                  onChange={() => onSelect(choice.value)}
                />
                <span className="access-choice-text">
                  <span className="access-choice-label" dir="auto">
                    {choice.label}
                  </span>
                  <span className="access-choice-summary">{choice.summary}</span>
                  {choice.disabled && choice.disabledReason ? (
                    <span className="access-choice-reason">{choice.disabledReason}</span>
                  ) : null}
                </span>
              </label>
            );
          })}
        </div>
        <div className="access-detail-panel" id={detailsId} role="region" aria-live="polite">
          <h5 className="access-detail-heading">{t("room.accessDetailsLabel")}</h5>
          <p className="access-detail-state">
            {detailsConfirmed ? t("room.accessDetailsConfirmed") : t("room.accessDetailsUnsaved")}
          </p>
          {details}
          {notes}
          {rejection ? (
            <p className="settings-notice" role="alert">
              {rejection}
            </p>
          ) : null}
        </div>
        {actions}
      </fieldset>
      {!canEdit && readOnlyReason ? (
        <p className="profile-settings-hint">{readOnlyReason}</p>
      ) : null}
    </SettingsPropertyCard>
  );
}

/** The catalog text of one Rust outcome line; the only substitution is the Space name. */
export function outcomeLineText(line: RoomAccessOutcomeLine): string {
  const space = line.substitutions?.[0];
  return t(line.messageId, space ? { space } : {});
}

/**
 * The details a panel shows for one Rust outcome: the join, history,
 * encryption and directory facts, plus the key caveat and non-retroactivity
 * note. Server eligibility and key availability stay separate lines.
 */
export function accessOutcomeDetails(outcome: RoomAccessOutcome): ReactNode {
  return (
    <ul className="access-detail-list">
      <li>{outcomeLineText(outcome.join)}</li>
      {outcome.joinRequest ? <li>{outcomeLineText(outcome.joinRequest)}</li> : null}
      <li>{outcomeLineText(outcome.history)}</li>
      {outcome.historyKeyCaveat ? (
        <li className="access-detail-caveat">{outcomeLineText(outcome.historyKeyCaveat)}</li>
      ) : null}
      <li>{outcomeLineText(outcome.encryption)}</li>
      <li>{outcomeLineText(outcome.directory)}</li>
      <li className="access-detail-note">{outcomeLineText(outcome.nonRetroactive)}</li>
    </ul>
  );
}

export type { MessageId };
