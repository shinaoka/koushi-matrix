import { t } from "../i18n/messages";
import {
  formatModShortcut,
  keyboardShortcutGroups,
  type KeyboardShortcut,
  type ShortcutLabelProfile
} from "../domain/shortcuts";
import type { ComposerSendShortcut, SettingsPatch, SettingsState } from "../domain/types";

export function KeyboardSettingsPanel({
  labelProfile,
  settings,
  onUpdateSettings
}: {
  labelProfile?: ShortcutLabelProfile;
  settings: SettingsState;
  onUpdateSettings: (patch: SettingsPatch) => void;
}) {
  const selectedSendShortcut = settings.values.keyboard.composer_send_shortcut;
  const isSaving = settings.persistence.kind === "saving";

  return (
    <section className="settings-panel keyboard-settings" aria-labelledby="keyboard-settings-title">
      <header className="settings-panel-header">
        <div>
          <h2 id="keyboard-settings-title">{t("panel.keyboard")}</h2>
          <p>{t("settings.keyboardDescription")}</p>
        </div>
      </header>
      <KeyboardSettingsContent
        isSaving={isSaving}
        labelProfile={labelProfile}
        selectedSendShortcut={selectedSendShortcut}
        onUpdateSettings={onUpdateSettings}
      />
    </section>
  );
}

export function KeyboardSettingsContent({
  isSaving,
  labelProfile,
  selectedSendShortcut,
  onUpdateSettings
}: {
  isSaving: boolean;
  labelProfile?: ShortcutLabelProfile;
  selectedSendShortcut: ComposerSendShortcut;
  onUpdateSettings: (patch: SettingsPatch) => void;
}) {
  const modEnterLabel = t("shortcut.modEnterSends", {
    shortcut: formatModShortcut("Enter", labelProfile)
  });

  return (
    <>
      <section className="settings-section" aria-label={t("shortcut.composerSendShortcut")}>
        <div className="settings-section-heading">
          <h3>{t("shortcut.composerSendShortcut")}</h3>
          {isSaving ? <span className="settings-save-state">{t("settings.saving")}</span> : null}
        </div>
        <div className="segmented-control" role="group" aria-label={t("shortcut.composerSendShortcut")}>
          <ComposerShortcutButton
            label={t("shortcut.enterSends")}
            selected={selectedSendShortcut === "enter"}
            value="enter"
            onSelect={onUpdateSettings}
          />
          <ComposerShortcutButton
            label={modEnterLabel}
            selected={selectedSendShortcut === "modEnter"}
            value="modEnter"
            onSelect={onUpdateSettings}
          />
        </div>
      </section>
      <div className="shortcut-groups">
        {keyboardShortcutGroups.map((group) => (
          <section className="shortcut-group" key={group.category}>
            <h3>{t(group.categoryMessageId)}</h3>
            <div className="shortcut-table">
              {group.shortcuts.map((shortcut) => (
                <ShortcutRow key={shortcut.id} shortcut={shortcut} />
              ))}
            </div>
          </section>
        ))}
      </div>
    </>
  );
}

function ComposerShortcutButton({
  label,
  selected,
  value,
  onSelect
}: {
  label: string;
  selected: boolean;
  value: ComposerSendShortcut;
  onSelect: (patch: SettingsPatch) => void;
}) {
  return (
    <button
      className={`segmented-control-option ${selected ? "is-selected" : ""}`}
      type="button"
      aria-pressed={selected}
      onClick={() => {
        if (!selected) {
          onSelect({ keyboard: { composer_send_shortcut: value } });
        }
      }}
    >
      {label}
    </button>
  );
}

function ShortcutRow({ shortcut }: { shortcut: KeyboardShortcut }) {
  const label = t(shortcut.labelMessageId);
  const note = shortcut.noteMessageId ? t(shortcut.noteMessageId) : null;
  return (
    <div className="shortcut-row">
      <div className="shortcut-label">
        <span>{label}</span>
        {note ? <small>{note}</small> : null}
      </div>
      <div className="shortcut-keys" aria-label={t("shortcut.shortcutKeys", { label })}>
        {shortcut.keys.map((key) => (
          <kbd key={key}>{key}</kbd>
        ))}
      </div>
      <span className={`shortcut-status ${shortcut.parity}`}>
        {shortcutParityLabel(shortcut.parity)}
      </span>
    </div>
  );
}

function shortcutParityLabel(parity: KeyboardShortcut["parity"]): string {
  switch (parity) {
    case "adapted":
      return t("shortcut.parityAdapted");
    case "deferred":
      return t("shortcut.parityDeferred");
    case "notApplicable":
      return t("shortcut.parityNotApplicable");
    case "same":
      return t("shortcut.paritySame");
  }
}
