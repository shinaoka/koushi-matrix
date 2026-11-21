import { type FormEvent, useEffect, useRef, useState } from "react";
import { KeyRound, Laptop, Smartphone } from "lucide-react";

import { t } from "../../i18n/messages";
import type { DesktopApi } from "../../backend/desktopApi";
import { api as defaultApi } from "../../backend/appRuntime";
import { openExternalHttpUrl } from "../../backend/linkMediaRuntime";
import { ImeSafeForm, SecureImeTextField } from "../ImeTextControl";
import { RoomKeyImportForm } from "./RoomKeyImportForm";
import type { MigrationImportAdmission, MigrationSources, RoomKeyImportState } from "../../domain/types";

/** Where a browser-only account reviews or resets its secure backup. */
const ELEMENT_WEB_SECURITY_URL = "https://app.element.io/#/settings/security";

/**
 * #1871 "Migrate to Koushi": the four routes for bringing old history over,
 * in the order that works most often. Exporting a key file first and the
 * recovery key second are deterministic; the Element Desktop import is opt-in
 * behind an adapter flag; verifying from another device comes last because it
 * is the route that keeps failing in the field (#1780, #1279, nightly-qa
 * user_verification), so the screen says so and points back at 1 and 2.
 */
export function MigrationSection({
  desktopApi,
  roomKeyImport,
  onChooseRoomKeyImportSource,
  onImportRoomKeys,
  onCreateSecureBackup
}: {
  desktopApi?: DesktopApi;
  roomKeyImport: RoomKeyImportState;
  onChooseRoomKeyImportSource: () => Promise<string | null>;
  onImportRoomKeys: (sourcePath: string, passphrase: string) => void;
  /** Omitted where the caller cannot actually start a new backup. */
  onCreateSecureBackup?: () => void;
}) {
  const api = desktopApi ?? defaultApi;
  const [sources, setSources] = useState<MigrationSources | null>(null);
  const [detectionFailed, setDetectionFailed] = useState(false);
  const [recoveryEmpty, setRecoveryEmpty] = useState(true);
  const [busy, setBusy] = useState<MigrationRoute | null>(null);
  const [error, setError] = useState<MigrationRoute | null>(null);
  const [confirmElementImport, setConfirmElementImport] = useState(false);
  const [staged, setStaged] = useState<MigrationImportAdmission | null>(null);
  const keyFileButtonRef = useRef<HTMLButtonElement>(null);
  const recoveryFieldRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    let cancelled = false;
    void api
      .migrationDetectSources()
      .then((detected) => {
        if (!cancelled) {
          setSources(detected);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setDetectionFailed(true);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [api]);

  async function run(route: MigrationRoute, action: () => Promise<unknown>) {
    setBusy(route);
    setError(null);
    setStaged(null);
    try {
      await action();
    } catch {
      setError(route);
    } finally {
      setBusy(null);
    }
  }

  function submitRecovery(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    // The secret is read from the element and never copied into state (#927).
    const secret = recoveryFieldRef.current?.value.trim() ?? "";
    if (!secret) {
      return;
    }
    void run("recovery", async () => {
      await api.recoverSecureBackup(secret);
      if (recoveryFieldRef.current) {
        recoveryFieldRef.current.value = "";
      }
      setRecoveryEmpty(true);
    });
  }

  async function importFromElementDesktop() {
    setConfirmElementImport(false);
    await run("element", async () => {
      setStaged(await api.migrationImportElementDesktop());
    });
  }

  const element = sources?.element_desktop;
  const elementRouteAvailable = Boolean(
    element?.installed && element.import_supported && element.import_enabled
  );

  return (
    <section
      className="settings-section"
      aria-label={t("settings.migrateToKoushi")}
      data-migration-section=""
    >
      <div className="settings-section-heading">
        <h3>{t("settings.migrateToKoushi")}</h3>
      </div>
      <p className="profile-settings-hint">{t("settings.migrateIntro")}</p>

      <section aria-label={t("settings.migrateKeyFileTitle")} data-migration-route="key-file">
        <h4 className="settings-subheading">
          {t("settings.migrateKeyFileTitle")}{" "}
          <small data-migration-recommended="">{t("settings.migrateRecommended")}</small>
        </h4>
        <p className="profile-settings-hint">{t("settings.migrateKeyFileCopy")}</p>
        <RoomKeyImportForm
          roomKeyImport={roomKeyImport}
          onChooseRoomKeyImportSource={onChooseRoomKeyImportSource}
          onImportRoomKeys={onImportRoomKeys}
          label={t("settings.migrateKeyFileTitle")}
          buttonRef={keyFileButtonRef}
          statusTestId="migrate-key-file-state"
        />
      </section>

      <section aria-label={t("settings.migrateRecoveryTitle")} data-migration-route="recovery-key">
        <h4 className="settings-subheading">{t("settings.migrateRecoveryTitle")}</h4>
        <p className="profile-settings-hint">{t("settings.migrateRecoveryCopy")}</p>
        <ImeSafeForm
          className="profile-settings-form"
          aria-label={t("settings.migrateRecoveryTitle")}
          onSubmit={submitRecovery}
        >
          <SecureImeTextField
            ref={recoveryFieldRef}
            aria-label={t("settings.migrateRecoverySecret")}
            autoComplete="off"
            onChange={(event) => setRecoveryEmpty(event.currentTarget.value.trim() === "")}
          />
          <div className="profile-settings-actions">
            <button
              className="trust-action-button primary"
              type="submit"
              disabled={busy !== null || recoveryEmpty}
            >
              <KeyRound size={14} />
              <span>{t("settings.migrateRecoveryAction")}</span>
            </button>
          </div>
        </ImeSafeForm>
        {error === "recovery" ? (
          <p className="profile-settings-hint error" role="alert">
            {t("settings.migrateRequestFailed")}
          </p>
        ) : null}
      </section>

      {elementRouteAvailable ? (
        <section aria-label={t("settings.migrateElementTitle")} data-migration-route="element-desktop">
          <h4 className="settings-subheading">{t("settings.migrateElementTitle")}</h4>
          <p className="profile-settings-hint">
            {element?.running
              ? t("settings.migrateElementCopyRunning")
              : t("settings.migrateElementCopy")}
          </p>
          <div className="profile-settings-actions">
            <button
              className="trust-action-button primary"
              type="button"
              disabled={busy !== null}
              onClick={() => setConfirmElementImport(true)}
            >
              <Laptop size={14} />
              <span>{t("settings.migrateElementAction")}</span>
            </button>
          </div>
          {confirmElementImport ? (
            <div role="region" aria-label={t("settings.migrateElementConfirm")}>
              <p className="profile-settings-hint">{t("settings.migrateElementConfirmBody")}</p>
              <div className="profile-settings-actions">
                <button
                  className="trust-action-button primary"
                  type="button"
                  onClick={() => void importFromElementDesktop()}
                >
                  {t("settings.migrateElementConfirm")}
                </button>
                <button
                  className="trust-action-button"
                  type="button"
                  onClick={() => setConfirmElementImport(false)}
                >
                  {t("action.cancel")}
                </button>
              </div>
            </div>
          ) : null}
          {staged ? (
            <p className="profile-settings-hint success" role="status">
              {t("settings.migrateElementStaged", {
                keys: staged.staged_keys,
                rooms: staged.staged_rooms
              })}
            </p>
          ) : null}
          {error === "element" ? (
            <p className="profile-settings-hint error" role="alert">
              {t("settings.migrateElementFailed")}
            </p>
          ) : null}
        </section>
      ) : null}

      <section aria-label={t("settings.migrateVerifyTitle")} data-migration-route="verify-device">
        <h4 className="settings-subheading">{t("settings.migrateVerifyTitle")}</h4>
        <p className="profile-settings-hint">{t("settings.migrateVerifyCopy")}</p>
        <div className="profile-settings-actions">
          <button
            className="trust-action-button"
            type="button"
            disabled={busy !== null}
            onClick={() => void run("verify", () => api.startOwnUserSas())}
          >
            <Smartphone size={14} />
            <span>{t("settings.migrateVerifyAction")}</span>
          </button>
        </div>
        <p className="profile-settings-hint">{t("settings.migrateVerifyFallback")}</p>
        <div className="profile-settings-actions">
          <button
            className="trust-action-button"
            type="button"
            onClick={() => keyFileButtonRef.current?.focus()}
          >
            {t("settings.migrateVerifyFallbackKeyFile")}
          </button>
          <button
            className="trust-action-button"
            type="button"
            onClick={() => recoveryFieldRef.current?.focus()}
          >
            {t("settings.migrateVerifyFallbackRecovery")}
          </button>
        </div>
        {error === "verify" ? (
          <p className="profile-settings-hint error" role="alert">
            {t("settings.migrateRequestFailed")}
          </p>
        ) : null}
      </section>

      <section aria-label={t("settings.migrateNothingTitle")} data-migration-route="nothing">
        <h4 className="settings-subheading">{t("settings.migrateNothingTitle")}</h4>
        <p className="profile-settings-hint">{t("settings.migrateNothingCopy")}</p>
        <div className="profile-settings-actions">
          {onCreateSecureBackup ? (
            <button
              className="trust-action-button primary"
              type="button"
              onClick={onCreateSecureBackup}
            >
              <KeyRound size={14} />
              <span>{t("settings.migrateNothingAction")}</span>
            </button>
          ) : null}
          <button
            className="trust-action-button"
            type="button"
            onClick={() => void openExternalHttpUrl(ELEMENT_WEB_SECURITY_URL)}
          >
            {t("settings.migrateNothingElementWeb")}
          </button>
        </div>
      </section>

      <p className="profile-settings-hint" data-migration-detection="">
        {detectionSummary(sources, detectionFailed)}
      </p>
    </section>
  );
}

type MigrationRoute = "recovery" | "element" | "verify";

/** One line about what is on this machine; codes only, no paths or key data. */
function detectionSummary(
  sources: MigrationSources | null,
  detectionFailed: boolean
): string {
  if (detectionFailed) {
    return t("settings.migrateDetectUnknown");
  }
  if (!sources) {
    return "";
  }
  const element = sources.element_desktop;
  const desktop = !element.installed
    ? t("settings.migrateDetectAbsent")
    : element.running
      ? t("settings.migrateDetectRunning")
      : t("settings.migrateDetectInstalled");
  const unavailable =
    element.unavailable_reason === "migration_element_desktop_import_disabled"
      ? ` ${t("settings.migrateDetectImportDisabled")}`
      : element.unavailable_reason === "migration_element_desktop_platform_unsupported"
        ? ` ${t("settings.migrateDetectPlatformUnsupported")}`
        : "";
  const phone =
    sources.element_x.sessions > 0 ? ` ${t("settings.migrateDetectElementX")}` : "";
  const unsupported =
    sources.unsupported_reason === "migration_element_x_unreadable"
      ? ` ${t("settings.migrateDetectUnknown")}`
      : "";
  return `${desktop}${unavailable}${phone}${unsupported}`;
}
