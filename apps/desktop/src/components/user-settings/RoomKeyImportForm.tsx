import { type FormEvent, type Ref, useRef, useState } from "react";
import { Upload } from "lucide-react";

import { t } from "../../i18n/messages";
import { ImeSafeForm, SecureImeTextField } from "../ImeTextControl";
import { NativeModal } from "../ModalDialog";
import { KeyManagementStatus, roomKeyImportStatusLabel } from "./KeyManagementStatus";
import type { RoomKeyImportState } from "../../domain/types";

/**
 * #1871 route 1: import an exported Element key file. The one path for that
 * command, mounted by the Security section and by the migration surface, so a
 * second copy of the chooser and the passphrase prompt cannot drift from it.
 */
export function RoomKeyImportForm({
  roomKeyImport,
  onChooseRoomKeyImportSource,
  onImportRoomKeys,
  label,
  buttonRef,
  statusTestId
}: {
  roomKeyImport: RoomKeyImportState;
  onChooseRoomKeyImportSource: () => Promise<string | null>;
  onImportRoomKeys: (sourcePath: string, passphrase: string) => void;
  /** Section-level heading for the aria label; defaults to the import row. */
  label?: string;
  /** Lets a caller point its own fallback link at the import button. */
  buttonRef?: Ref<HTMLButtonElement>;
  /** Distinct hook when one page mounts two of these forms. */
  statusTestId?: string;
}) {
  const passphraseRef = useRef<HTMLInputElement>(null);
  const [sourcePath, setSourcePath] = useState<string | null>(null);

  async function choose(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const chosen = await onChooseRoomKeyImportSource();
    if (chosen) {
      setSourcePath(chosen);
    }
  }

  function submitPassphrase(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const passphrase = passphraseRef.current?.value ?? "";
    if (!sourcePath || !passphrase) {
      return;
    }
    onImportRoomKeys(sourcePath, passphrase);
    if (passphraseRef.current) {
      passphraseRef.current.value = "";
    }
    setSourcePath(null);
  }

  function closePassphraseDialog() {
    if (passphraseRef.current) {
      passphraseRef.current.value = "";
    }
    setSourcePath(null);
  }

  return (
    <>
      <ImeSafeForm
        aria-label={label ?? t("settings.roomKeyImport")}
        className="profile-settings-form"
        onSubmit={(event) => {
          void choose(event);
        }}
      >
        <KeyManagementStatus
          label={t("settings.roomKeyImport")}
          value={roomKeyImportStatusLabel(roomKeyImport)}
          testId={statusTestId ?? "room-key-import-state"}
        />
        <p className="profile-settings-hint">{t("settings.chooseRoomKeyImportFile")}</p>
        <div className="profile-settings-actions">
          <button
            className="trust-action-button primary"
            ref={buttonRef}
            type="submit"
            disabled={roomKeyImport.kind === "importing"}
          >
            <Upload size={14} />
            <span>{t("settings.importRoomKeys")}</span>
          </button>
        </div>
      </ImeSafeForm>
      {sourcePath ? (
        <NativeModal
          className="dialog-overlay"
          aria-labelledby="room-key-import-passphrase-title"
          onDismiss={closePassphraseDialog}
        >
          <ImeSafeForm className="dialog-box" onSubmit={submitPassphrase}>
            <h3 className="dialog-title" id="room-key-import-passphrase-title">
              {t("settings.roomKeyPassphrase")}
            </h3>
            <p className="profile-settings-hint">
              {t("settings.roomKeyPassphrasePromptImport")}
            </p>
            <SecureImeTextField
              className="dialog-input"
              ref={passphraseRef}
              autoComplete="new-password"
              aria-label={t("settings.roomKeyPassphrase")}
            />
            <div className="dialog-actions">
              <button
                className="dialog-button"
                type="button"
                onClick={closePassphraseDialog}
              >
                {t("action.cancel")}
              </button>
              <button className="dialog-button is-primary" type="submit">
                {t("settings.importRoomKeys")}
              </button>
            </div>
          </ImeSafeForm>
        </NativeModal>
      ) : null}
    </>
  );
}
