import { t } from "../../i18n/messages";
import { useEmojiPickerSize } from "../useEmojiPickerSize";

/**
 * The emoji picker's saved size is a device-local window preference (#1583) and
 * this row is the only place it is shown. Clearing writes `null`, the Rust
 * default, through the picker's own app-scoped write path.
 */
export function EmojiPickerSizeRow() {
  const { size, save } = useEmojiPickerSize();
  return (
    <div className="settings-control-row">
      <span>{t("settings.emojiPickerSize")}</span>
      <div className="settings-inline-actions">
        <span className="settings-muted-note">
          {size ? `${size[0]}×${size[1]}` : t("settings.emojiPickerSizeDefault")}
        </span>
        <button
          className="profile-settings-action"
          type="button"
          disabled={!size}
          onClick={() => save(null)}
        >
          {t("settings.emojiPickerSizeReset")}
        </button>
      </div>
    </div>
  );
}
