import { useCallback, useEffect, useRef, useState } from "react";
import { RefreshCcw } from "lucide-react";
import { api } from "../backend/appRuntime";
import { desktopEventPort } from "../backend/desktopEventRuntime";
import { isTauriRuntime } from "../backend/runtimeEnvironment";
import { getAppStoreSnapshot, setAppStoreSnapshot, useAppStore } from "../domain/appStore";
import { createCommandReceiptReconciler } from "../domain/commandWatermark";
import { SNAPSHOT_SCHEMA_VERSION, type DesktopUpdateState, type SettingsPatch, type UpdatesSettings } from "../domain/types";
import { t } from "../i18n/messages";
import { ModalDialog } from "./ModalDialog";

/** Application-level presentation over the single Rust updater owner. */
export function DesktopUpdates() {
  const settings = useAppStore(store => store.snapshot?.state.domain.settings);
  const [state, setState] = useState<DesktopUpdateState>({ kind: "idle" });
  const [open, setOpen] = useState(false);
  const [presentationKey, setPresentationKey] = useState(0);
  const [commandFailed, setCommandFailed] = useState(false);
  const mounted = useRef(false);
  const offeredVersion = useRef<string | null>(null);
  const reconcile = useRef(createCommandReceiptReconciler({
    currentGeneration: () => getAppStoreSnapshot()?.state_generation ?? null,
    settlementSnapshot: () => api.settlementSnapshot(),
    applySnapshot: snapshot => {
      if (snapshot.state.schema_version !== SNAPSHOT_SCHEMA_VERSION) {
        throw new Error("Incompatible update settings snapshot");
      }
      setAppStoreSnapshot(snapshot);
    }
  }));
  const run = useCallback((operation: Promise<unknown>) => {
    setCommandFailed(false);
    void operation.catch(() => {
      if (mounted.current) setCommandFailed(true);
    });
  }, []);

  useEffect(() => {
    mounted.current = true;
    if (!isTauriRuntime()) return () => { mounted.current = false; };
    let disposed = false;
    let revision = 0;
    const disposers: Array<() => void> = [];
    const keep = (dispose: () => void) => {
      if (disposed) dispose();
      else disposers.push(dispose);
    };
    const showState = (next: DesktopUpdateState) => {
      if (disposed) return;
      setState(next);
      setCommandFailed(false);
      if (next.kind === "available" && offeredVersion.current !== next.version) {
        offeredVersion.current = next.version;
        setOpen(true);
      }
    };
    const updatesReady = desktopEventPort.listenDesktopUpdates(next => {
      revision++;
      showState(next);
    });
    run(updatesReady.then(async dispose => {
      keep(dispose);
      if (disposed) return;
      const requestedRevision = revision;
      const initial = await api.getDesktopUpdateState();
      // A live event observed while the initial read was in flight wins.
      if (revision === requestedRevision) showState(initial);
    }));
    run(desktopEventPort.listenMenuActions(action => {
      if (disposed || action !== "checkForUpdates") return;
      // Re-present one modal above any dialog opened since the previous check.
      setPresentationKey(key => key + 1);
      setOpen(true);
      run(api.checkForDesktopUpdate());
    }).then(keep));
    return () => {
      disposed = true;
      mounted.current = false;
      disposers.forEach(dispose => dispose());
    };
  }, [run]);

  if (!open) return null;
  return (
    <ModalDialog key={presentationKey} title={t("settings.updateTitle")} className="desktop-update-modal" onClose={() => setOpen(false)}>
      <div className="desktop-update-content">
        {commandFailed ? <p role="alert">{t("settings.updateCommandFailed")}</p> : null}
        {settings ? (
          <DesktopUpdateControls
            current={settings.values.updates}
            state={state}
            disabled={settings.persistence.kind === "saving"}
            onSelect={patch => run(api.updateSettings(patch).then(receipt => reconcile.current(receipt)))}
            onCheck={() => run(api.checkForDesktopUpdate())}
            onDownload={() => {
              if (state.kind === "available") run(api.downloadDesktopUpdate(state.generation));
            }}
            onRestart={() => run(api.restartToInstallDesktopUpdate())}
          />
        ) : <p role="status">{t("settings.updateChecking")}</p>}
      </div>
    </ModalDialog>
  );
}

export function DesktopUpdateControls({
  current,
  state,
  onSelect,
  onCheck,
  onDownload,
  onRestart,
  disabled = false
}: {
  current: UpdatesSettings;
  state: DesktopUpdateState;
  onSelect: (patch: SettingsPatch) => void;
  onCheck: () => void;
  onDownload: () => void;
  onRestart: () => void;
  disabled?: boolean;
}) {
  return (
    <>
      <button
        className="settings-toggle-row"
        type="button"
        role="switch"
        disabled={disabled || state.kind === "unsupported"}
        aria-checked={current.auto_check}
        aria-label={t("settings.autoUpdate")}
        onClick={() => onSelect({ updates: { ...current, auto_check: !current.auto_check } })}
      >
        <span className="settings-toggle-copy">
          <span className="settings-toggle-label">
            <RefreshCcw size={15} aria-hidden="true" />
            <span>{t("settings.autoUpdate")}</span>
          </span>
          <span className="settings-toggle-description">
            {t("settings.autoUpdateDescription")}
          </span>
        </span>
        <span className="settings-switch-track" aria-hidden="true">
          <span className="settings-switch-thumb" />
        </span>
      </button>
      <button
        className="settings-toggle-row"
        type="button"
        role="switch"
        disabled={disabled || state.kind === "unsupported" || state.kind === "downloading" || state.kind === "ready" || state.kind === "installing"}
        aria-checked={current.include_prereleases}
        aria-label={t("settings.includePrereleases")}
        onClick={() => onSelect({ updates: { ...current, include_prereleases: !current.include_prereleases } })}
      >
        <span className="settings-toggle-copy">
          <span className="settings-toggle-label">
            <RefreshCcw size={15} aria-hidden="true" />
            <span>{t("settings.includePrereleases")}</span>
          </span>
          <span className="settings-toggle-description">
            {t("settings.includePrereleasesDescription")}
          </span>
        </span>
        <span className="settings-switch-track" aria-hidden="true">
          <span className="settings-switch-thumb" />
        </span>
      </button>
      <div className="settings-update-status" aria-live="polite">
          <p className="settings-status-text">{desktopUpdateStatusText(state)}</p>
          {state.kind === "idle" || state.kind === "up_to_date" || state.kind === "failed" ? (
            <button className="profile-settings-action" type="button" onClick={onCheck}>
              <RefreshCcw size={14} aria-hidden="true" />
              {t("settings.updateCheck")}
            </button>
          ) : null}
          {state.kind === "available" ? (
            <button className="profile-settings-action" type="button" onClick={onDownload}>
              <RefreshCcw size={14} aria-hidden="true" />
              {t("settings.updateDownload")}
            </button>
          ) : null}
          {state.kind === "ready" ? (
            <button className="profile-settings-action" type="button" onClick={onRestart}>
              <RefreshCcw size={14} aria-hidden="true" />
              {t("settings.updateRestart")}
            </button>
          ) : null}
      </div>
    </>
  );
}

function desktopUpdateStatusText(state: DesktopUpdateState): string {
  switch (state.kind) {
    case "idle":
      return t("settings.updateIdle");
    case "up_to_date":
      return t("settings.updateUpToDate", { version: state.version });
    case "checking":
      return t("settings.updateChecking");
    case "available":
      return t("settings.updateAvailable", { version: state.version });
    case "downloading":
      return t("settings.updateDownloading", { version: state.version });
    case "ready":
      return t("settings.updateReady", { version: state.version });
    case "installing":
      return t("settings.updateInstalling", { version: state.version });
    case "failed":
      return state.stage === "install"
        ? t("settings.updateInstallFailed")
        : t("settings.updateCheckFailed");
    case "unsupported":
      return t("settings.updateUnsupported");
  }
}
