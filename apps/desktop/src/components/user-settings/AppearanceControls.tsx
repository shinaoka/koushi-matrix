import { t } from "../../i18n/messages";
import type {
  DisplayDensity,
  EmojiPreference,
  FontPreference,
  LocaleSettings,
  SettingsPatch,
  ThemePreference
} from "../../domain/types";

export function AppearanceControls({
  displayDensity,
  selectedEmoji,
  selectedFont,
  selectedTheme,
  selectedLocale,
  onDisplayDensityChange,
  onUpdateSettings
}: {
  displayDensity: DisplayDensity;
  selectedEmoji: EmojiPreference;
  selectedFont: FontPreference;
  selectedTheme: ThemePreference;
  selectedLocale: LocaleSettings;
  onDisplayDensityChange: (density: DisplayDensity) => void;
  onUpdateSettings: (patch: SettingsPatch) => void;
}) {
  return (
    <>
        <div className="settings-control-row">
          <span>{t("settings.language")}</span>
          <div className="segmented-control" role="group" aria-label={t("settings.language")}>
            <LocaleButton
              label={t("settings.languageDefault")}
              selected={selectedLocale.language_tag === null}
              value={null}
              current={selectedLocale}
              onSelect={onUpdateSettings}
            />
            <LocaleButton
              label={t("settings.languageEnglish")}
              selected={selectedLocale.language_tag === "en"}
              value="en"
              current={selectedLocale}
              onSelect={onUpdateSettings}
            />
            <LocaleButton
              label={t("settings.languageJapanese")}
              selected={selectedLocale.language_tag === "ja-JP"}
              value="ja-JP"
              current={selectedLocale}
              onSelect={onUpdateSettings}
            />
          </div>
        </div>
        <div className="segmented-control" role="group" aria-label={t("settings.theme")}>
          <ThemeButton
            label={t("settings.themeSystem")}
            selected={selectedTheme === "system"}
            value="system"
            density={displayDensity}
            onSelect={onUpdateSettings}
          />
          <ThemeButton
            label={t("settings.themeLight")}
            selected={selectedTheme === "light"}
            value="light"
            density={displayDensity}
            onSelect={onUpdateSettings}
          />
          <ThemeButton
            label={t("settings.themeDark")}
            selected={selectedTheme === "dark"}
            value="dark"
            density={displayDensity}
            onSelect={onUpdateSettings}
          />
        </div>
        <div className="settings-control-row">
          <span>{t("settings.displayDensity")}</span>
          <div className="segmented-control" role="group" aria-label={t("settings.displayDensity")}>
            <DensityButton
              label={t("settings.densityCompact")}
              selected={displayDensity === "compact"}
              value="compact"
              onSelect={onDisplayDensityChange}
            />
            <DensityButton
              label={t("settings.densityDefault")}
              selected={displayDensity === "default"}
              value="default"
              onSelect={onDisplayDensityChange}
            />
            <DensityButton
              label={t("settings.densityComfortable")}
              selected={displayDensity === "comfortable"}
              value="comfortable"
              onSelect={onDisplayDensityChange}
            />
          </div>
        </div>
        <h4 className="settings-subheading">{t("settings.typography")}</h4>
        <div className="settings-control-stack">
          <div className="settings-control-row">
            <span>{t("settings.uiFont")}</span>
            <div className="segmented-control" role="group" aria-label={t("settings.uiFont")}>
              <FontButton
                label={t("settings.fontSystem")}
                selected={selectedFont === "system"}
                value="system"
                currentEmoji={selectedEmoji}
                onSelect={onUpdateSettings}
              />
              <FontButton
                label={t("settings.fontInter")}
                selected={selectedFont === "inter"}
                value="inter"
                currentEmoji={selectedEmoji}
                onSelect={onUpdateSettings}
              />
            </div>
          </div>
          <div className="settings-control-row">
            <span>{t("settings.emojiFont")}</span>
            <div className="segmented-control" role="group" aria-label={t("settings.emojiFont")}>
              <EmojiButton
                label={t("settings.fontSystem")}
                selected={selectedEmoji === "system"}
                value="system"
                currentFont={selectedFont}
                onSelect={onUpdateSettings}
              />
              <EmojiButton
                label={t("settings.twemojiColr")}
                selected={selectedEmoji === "twemojiColr"}
                value="twemojiColr"
                currentFont={selectedFont}
                onSelect={onUpdateSettings}
              />
            </div>
          </div>
        </div>
    </>
  );
}

function LocaleButton({
  label,
  selected,
  value,
  current,
  onSelect
}: {
  label: string;
  selected: boolean;
  value: LocaleSettings["language_tag"];
  current: LocaleSettings;
  onSelect: (patch: SettingsPatch) => void;
}) {
  return (
    <button
      className={`segmented-control-option ${selected ? "is-selected" : ""}`}
      type="button"
      aria-pressed={selected}
      onClick={() => {
        if (!selected) {
          onSelect({ locale: { ...current, language_tag: value } });
        }
      }}
    >
      {label}
    </button>
  );
}

function ThemeButton({
  label,
  selected,
  value,
  density,
  onSelect
}: {
  label: string;
  selected: boolean;
  value: ThemePreference;
  density: DisplayDensity;
  onSelect: (patch: SettingsPatch) => void;
}) {
  return (
    <button
      className={`segmented-control-option ${selected ? "is-selected" : ""}`}
      type="button"
      aria-pressed={selected}
      onClick={() => {
        if (!selected) {
          onSelect({ appearance: { theme: value, density } });
        }
      }}
    >
      {label}
    </button>
  );
}

function DensityButton({
  label,
  selected,
  value,
  onSelect
}: {
  label: string;
  selected: boolean;
  value: DisplayDensity;
  onSelect: (density: DisplayDensity) => void;
}) {
  return (
    <button
      className={`segmented-control-option ${selected ? "is-selected" : ""}`}
      type="button"
      aria-pressed={selected}
      onClick={() => {
        if (!selected) {
          onSelect(value);
        }
      }}
    >
      {label}
    </button>
  );
}

function FontButton({
  label,
  selected,
  value,
  currentEmoji,
  onSelect
}: {
  label: string;
  selected: boolean;
  value: FontPreference;
  currentEmoji: EmojiPreference;
  onSelect: (patch: SettingsPatch) => void;
}) {
  return (
    <button
      className={`segmented-control-option ${selected ? "is-selected" : ""}`}
      type="button"
      aria-pressed={selected}
      onClick={() => {
        if (!selected) {
          onSelect({ typography: { font: value, emoji: currentEmoji } });
        }
      }}
    >
      {label}
    </button>
  );
}

function EmojiButton({
  label,
  selected,
  value,
  currentFont,
  onSelect
}: {
  label: string;
  selected: boolean;
  value: EmojiPreference;
  currentFont: FontPreference;
  onSelect: (patch: SettingsPatch) => void;
}) {
  return (
    <button
      className={`segmented-control-option ${selected ? "is-selected" : ""}`}
      type="button"
      aria-pressed={selected}
      onClick={() => {
        if (!selected) {
          onSelect({ typography: { font: currentFont, emoji: value } });
        }
      }}
    >
      {label}
    </button>
  );
}
