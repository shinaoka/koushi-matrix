import { t } from "../i18n/messages";
import { keyboardShortcutGroups, type KeyboardShortcut } from "../domain/shortcuts";

export function KeyboardSettingsPanel() {
  return (
    <section className="settings-panel keyboard-settings" aria-labelledby="keyboard-settings-title">
      <header className="settings-panel-header">
        <div>
          <h2 id="keyboard-settings-title">{t("panel.keyboard")}</h2>
          <p>Element-compatible shortcuts for implemented desktop actions.</p>
        </div>
      </header>
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
