import {
  type KeyboardEvent,
  type ReactNode,
  type Ref,
  useEffect,
  useId,
  useImperativeHandle,
  useRef,
  useState
} from "react";

import { t } from "../i18n/messages";
import { ImeSafeForm, ImeTextArea, ImeTextField } from "./ImeTextControl";

/**
 * Issue #1008: one property, one place. A property card holds the property's
 * confirmed value, the control that changes it and the result of the last
 * change, so the reader never has to find a separate "current X" row and an
 * "X settings" form elsewhere in the panel.
 *
 * The status is the caller's: Rust owns pending/failed state and the confirmed
 * value, and the caller only attributes that state to the property it
 * submitted. The card owns nothing but the open editor and its DOM draft.
 */
export type PropertySaveStatus =
  | { kind: "saving" }
  | { kind: "saved" }
  | { kind: "failed"; message: string }
  | null;

export function SettingsPropertyCard({
  property,
  label,
  headingRef,
  children,
  hint,
  status = null
}: {
  /** Stable, unlocalized property name used by QA and tests. */
  property: string;
  label: string;
  headingRef?: Ref<HTMLHeadingElement>;
  children: ReactNode;
  hint?: ReactNode;
  status?: PropertySaveStatus;
}) {
  const headingId = useId();
  return (
    <div
      className="settings-property-card"
      data-setting-property={property}
      role="group"
      aria-labelledby={headingId}
    >
      <h4 className="settings-property-label" id={headingId} ref={headingRef} tabIndex={-1}>
        {label}
      </h4>
      {children}
      {hint}
      {status ? (
        <p
          className={
            status.kind === "failed"
              ? "settings-property-status settings-property-status-failed"
              : status.kind === "saved"
                ? "settings-property-status settings-property-status-saved"
                : "settings-property-status"
          }
          role="status"
        >
          {status.kind === "saving"
            ? t("settings.propertySaving")
            : status.kind === "saved"
              ? t("settings.propertySaved")
              : status.message}
        </p>
      ) : null}
    </div>
  );
}

type FocusTarget = "control" | "trigger" | "heading" | null;

/**
 * Focus follows the edit flow: into the control when the editor opens, back to
 * the trigger on cancel, and to the card heading once submitted, since each
 * step unmounts the element that had focus.
 */
function useEditFocus() {
  const headingRef = useRef<HTMLHeadingElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const controlRef = useRef<HTMLInputElement & HTMLTextAreaElement & HTMLSelectElement>(null);
  const [target, setTarget] = useState<FocusTarget>(null);
  useEffect(() => {
    if (!target) return;
    const element =
      target === "control"
        ? controlRef.current
        : target === "trigger"
          ? triggerRef.current
          : headingRef.current;
    element?.focus();
    setTarget(null);
  }, [target]);
  return { headingRef, triggerRef, controlRef, focus: setTarget };
}

function isPlainEscape(event: KeyboardEvent): boolean {
  return event.key === "Escape" && !event.nativeEvent.isComposing;
}

export function InlineTextPropertyEditor({
  property,
  label,
  value,
  emptyText,
  display,
  userText = true,
  inputLabel,
  editLabel,
  saveLabel,
  clearLabel,
  placeholder,
  multiline = false,
  maxLength,
  syncKey,
  canEdit,
  busy = false,
  readOnlyReason,
  hint,
  status,
  headingRef,
  onSave,
  onClear
}: {
  property: string;
  label: string;
  /** The confirmed value; empty means unset. */
  value: string;
  emptyText: string;
  /** Replaces the plain text rendering of a set value, e.g. with a preview. */
  display?: ReactNode;
  userText?: boolean;
  /** Accessible name of the text control. */
  inputLabel: string;
  /** Accessible name of the Edit button; its visible text is the verb alone. */
  editLabel: string;
  /** Accessible name of the Save button; its visible text is the verb alone. */
  saveLabel: string;
  /** Accessible name of the Clear button, offered only while a value is set. */
  clearLabel?: string;
  placeholder?: string;
  multiline?: boolean;
  maxLength?: number;
  syncKey: string;
  canEdit: boolean;
  /** A change is in flight: controls stay visible but inert. */
  busy?: boolean;
  /** Why the value cannot be changed here, shown in place of the controls. */
  readOnlyReason?: string | null;
  hint?: ReactNode;
  status?: PropertySaveStatus;
  headingRef?: Ref<HTMLHeadingElement>;
  onSave: (next: string) => void;
  onClear?: () => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(value);
  const focus = useEditFocus();
  useImperativeHandle(headingRef, () => focus.headingRef.current as HTMLHeadingElement);
  const trimmed = value.trim();

  function open() {
    setDraft(value);
    setEditing(true);
    focus.focus("control");
  }

  function cancel() {
    setEditing(false);
    focus.focus("trigger");
  }

  const unchanged = draft.trim() === trimmed;
  const controlProps = {
    value: draft,
    syncKey,
    "aria-label": inputLabel,
    placeholder,
    maxLength,
    onChange: (event: { currentTarget: { value: string } }) => setDraft(event.currentTarget.value),
    onKeyDown: (event: KeyboardEvent) => {
      if (isPlainEscape(event)) {
        event.preventDefault();
        cancel();
      }
    }
  };

  return (
    <SettingsPropertyCard
      property={property}
      label={label}
      headingRef={focus.headingRef}
      hint={hint}
      status={status}
    >
      {editing ? (
        <ImeSafeForm
          className="settings-property-editor"
          onSubmit={(event) => {
            event.preventDefault();
            if (!canEdit || busy || unchanged) return;
            onSave(draft.trim());
            setEditing(false);
            focus.focus("heading");
          }}
        >
          {multiline ? (
            <ImeTextArea ref={focus.controlRef} className="settings-property-textarea" {...controlProps} />
          ) : (
            <ImeTextField ref={focus.controlRef} className="settings-property-input" {...controlProps} />
          )}
          <div className="profile-settings-actions">
            <button
              className="profile-settings-action"
              type="submit"
              aria-label={saveLabel}
              disabled={!canEdit || busy || unchanged}
            >
              {t("settings.propertySave")}
            </button>
            <button className="profile-settings-action" type="button" onClick={cancel}>
              {t("action.cancel")}
            </button>
          </div>
        </ImeSafeForm>
      ) : (
        <div className="settings-property-row">
          <div className="settings-property-value" dir={trimmed && userText ? "auto" : undefined}>
            {trimmed ? (display ?? trimmed) : <span className="settings-property-empty">{emptyText}</span>}
          </div>
          {canEdit ? (
            <div className="settings-property-actions">
              <button
                className="profile-settings-action"
                ref={focus.triggerRef}
                type="button"
                aria-label={editLabel}
                disabled={busy}
                onClick={open}
              >
                {t("settings.propertyEdit")}
              </button>
              {trimmed && onClear && clearLabel ? (
                <button
                  className="profile-settings-action"
                  type="button"
                  aria-label={clearLabel}
                  disabled={busy}
                  onClick={() => {
                    onClear();
                    focus.focus("heading");
                  }}
                >
                  {t("settings.propertyClear")}
                </button>
              ) : null}
            </div>
          ) : null}
        </div>
      )}
      {!canEdit && readOnlyReason ? (
        <p className="profile-settings-hint">{readOnlyReason}</p>
      ) : null}
    </SettingsPropertyCard>
  );
}

export function InlineChoicePropertyEditor<T extends string>({
  property,
  label,
  value,
  valueLabel,
  options,
  selectLabel,
  changeLabel,
  saveLabel,
  canEdit,
  busy = false,
  readOnlyReason,
  notes,
  status,
  headingRef,
  onSave
}: {
  property: string;
  label: string;
  value: T;
  valueLabel: (value: T) => string;
  options: readonly { value: T; disabled?: boolean }[];
  /** Accessible name of the select. */
  selectLabel: string;
  /** Accessible name of the Change button; its visible text is the verb alone. */
  changeLabel: string;
  /** Accessible name of the Save button; its visible text is the verb alone. */
  saveLabel: string;
  canEdit: boolean;
  busy?: boolean;
  readOnlyReason?: string | null;
  /** Explanation for the value being shown: the draft while editing, else the confirmed value. */
  notes?: (shown: T) => ReactNode;
  status?: PropertySaveStatus;
  headingRef?: Ref<HTMLHeadingElement>;
  onSave: (next: T) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<T>(value);
  const focus = useEditFocus();
  useImperativeHandle(headingRef, () => focus.headingRef.current as HTMLHeadingElement);

  function open() {
    setDraft(value);
    setEditing(true);
    focus.focus("control");
  }

  function cancel() {
    setEditing(false);
    focus.focus("trigger");
  }

  return (
    <SettingsPropertyCard
      property={property}
      label={label}
      headingRef={focus.headingRef}
      status={status}
    >
      {editing ? (
        <ImeSafeForm
          className="settings-property-editor"
          onKeyDown={(event) => {
            if (isPlainEscape(event)) {
              event.preventDefault();
              cancel();
            }
          }}
          onSubmit={(event) => {
            event.preventDefault();
            if (!canEdit || busy || draft === value) return;
            onSave(draft);
            setEditing(false);
            focus.focus("heading");
          }}
        >
          <select
            ref={focus.controlRef}
            className="settings-property-select"
            value={draft}
            aria-label={selectLabel}
            onChange={(event) => setDraft(event.currentTarget.value as T)}
          >
            {options.map((option) => (
              <option key={option.value} value={option.value} disabled={option.disabled}>
                {valueLabel(option.value)}
              </option>
            ))}
          </select>
          {notes?.(draft)}
          <div className="profile-settings-actions">
            <button
              className="profile-settings-action"
              type="submit"
              aria-label={saveLabel}
              disabled={!canEdit || busy || draft === value}
            >
              {t("settings.propertySave")}
            </button>
            <button className="profile-settings-action" type="button" onClick={cancel}>
              {t("action.cancel")}
            </button>
          </div>
        </ImeSafeForm>
      ) : (
        <>
          <div className="settings-property-row">
            <strong className="settings-property-value">{valueLabel(value)}</strong>
            {canEdit ? (
              <div className="settings-property-actions">
                <button
                  className="profile-settings-action"
                  ref={focus.triggerRef}
                  type="button"
                  aria-label={changeLabel}
                  disabled={busy}
                  onClick={open}
                >
                  {t("settings.propertyChange")}
                </button>
              </div>
            ) : null}
          </div>
          {notes?.(value)}
        </>
      )}
      {!canEdit && readOnlyReason ? (
        <p className="profile-settings-hint">{readOnlyReason}</p>
      ) : null}
    </SettingsPropertyCard>
  );
}
