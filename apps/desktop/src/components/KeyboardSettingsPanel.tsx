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
  const modEnterLabel = `${formatModShortcut("Enter", labelProfile)} sends`;

  return (
    <section className="settings-panel keyboard-settings" aria-labelledby="keyboard-settings-title">
      <header className="settings-panel-header">
        <div>
          <h2 id="keyboard-settings-title">{t("panel.keyboard")}</h2>
          <p>Element-compatible shortcuts for implemented desktop actions.</p>
        </div>
      </header>
      <section className="settings-section" aria-label="Composer send shortcut">
        <div className="settings-section-heading">
          <h3>Composer send shortcut</h3>
          {isSaving ? <span className="settings-save-state">Saving</span> : null}
        </div>
        <div className="segmented-control" role="group" aria-label="Composer send shortcut">
          <ComposerShortcutButton
            label="Enter sends"
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
            <h3>{group.category}</h3>
            <div className="shortcut-table">
              {group.shortcuts.map((shortcut) => (
                <ShortcutRow key={shortcut.id} shortcut={shortcut} />
              ))}
            </div>
          </section>
        ))}
      </div>
    </section>
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
  return (
    <div className="shortcut-row">
      <div className="shortcut-label">
        <span>{shortcut.label}</span>
        {shortcut.note ? <small>{shortcut.note}</small> : null}
      </div>
      <div className="shortcut-keys" aria-label={`${shortcut.label} shortcut`}>
        {shortcut.keys.map((key) => (
          <kbd key={key}>{key}</kbd>
        ))}
      </div>
      <span className={`shortcut-status ${shortcut.parity}`}>{shortcut.parity}</span>
    </div>
  );
}
