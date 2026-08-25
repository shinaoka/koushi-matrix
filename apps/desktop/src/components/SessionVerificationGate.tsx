import { useRef, useState } from "react";
import { ImeSafeForm, ImeTextField, SecureImeTextField } from "./ImeTextControl";
import { ResetLocalDataConfirmationDialog } from "./dialogs";
import { t } from "../i18n/messages";
import { api, startSessionVerificationWindowDrag } from "../backend/appRuntime";
import type { DesktopSnapshot, PendingKeyCountBucket, SecureBackupGateFailureKind, SecureBackupGateState } from "../domain/types";

function provisionalPhaseKind(
  phase: import("../domain/types").ProvisionalPhase | undefined
): "checkingTrust" | "discoveringMethods" | "recheckingTrust" | null {
  if (phase === "checkingTrust" || phase === "discoveringMethods") {
    return phase;
  }
  if (!phase || typeof phase !== "object") {
    return null;
  }
  if ("kind" in phase) {
    return phase.kind;
  }
  if ("recheckingTrust" in phase) {
    return "recheckingTrust";
  }
  return null;
}

function provisionalPhaseFailure(
  phase: import("../domain/types").ProvisionalPhase | undefined
): import("../domain/types").VerificationGateFailureKind | null {
  if (!phase || typeof phase !== "object") {
    return null;
  }
  if ("kind" in phase && phase.kind === "recheckingTrust") {
    return phase.failureKind ?? null;
  }
  if ("recheckingTrust" in phase) {
    return phase.recheckingTrust.failureKind ?? null;
  }
  return null;
}

type SecureBackupOperationKind = "recovery" | "setup" | "reenable" | "retry";

export interface SessionVerificationGateOperations {
  startOwnUserSas: () => Promise<DesktopSnapshot>;
  submitRecovery: (secret: string) => Promise<DesktopSnapshot>;
  retryCurrentDeviceTrustDiscovery?: () => Promise<DesktopSnapshot>;
  startDeviceCleanup?: () => Promise<DesktopSnapshot>;
  submitDeviceCleanupUia?: (flowId: number, password: string) => Promise<DesktopSnapshot>;
  eraseLocalDataAnyway?: () => Promise<DesktopSnapshot>;
  recoverSecureBackup?: (secret: string) => Promise<DesktopSnapshot>;
  setupSecureBackup?: (
    passphrase: string | null,
    recoveryKeyDestinationPath: string | null
  ) => Promise<DesktopSnapshot>;
  reenableSecureBackup?: (
    passphrase: string | null,
    recoveryKeyDestinationPath: string | null
  ) => Promise<DesktopSnapshot>;
  chooseSecureBackupDestination?: () => Promise<string | null>;
  retrySecureBackupInspection?: () => Promise<DesktopSnapshot>;
  openSecureBackupDiagnostics?: () => Promise<void> | void;
}

const defaultSessionVerificationGateOperations: SessionVerificationGateOperations = {
  startOwnUserSas: () => api.startOwnUserSas(),
  submitRecovery: (secret) => api.submitRecovery(secret),
  retryCurrentDeviceTrustDiscovery: () => api.retryCurrentDeviceTrustDiscovery(),
  startDeviceCleanup: () => api.startDeviceCleanup(),
  submitDeviceCleanupUia: (flowId, password) =>
    api.submitDeviceCleanupUia(flowId, password),
  eraseLocalDataAnyway: () => api.eraseLocalDataAnyway(),
  recoverSecureBackup: api.recoverSecureBackup,
  setupSecureBackup: api.setupSecureBackup,
  reenableSecureBackup: api.reenableSecureBackup,
  retrySecureBackupInspection: api.retrySecureBackupInspection,
  openSecureBackupDiagnostics: () => api.getDiagnosticSnapshot().then(() => undefined)
};

export function secureBackupFailureLabel(kind: SecureBackupGateFailureKind): string {
  return t(
    ({
      network: "gate.secureBackupFailureNetwork",
      rateLimited: "gate.secureBackupFailureRateLimited",
      invalidRecoveryKey: "gate.secureBackupFailureInvalidRecoveryKey",
      backupKeyMismatch: "gate.secureBackupFailureBackupKeyMismatch",
      secretStorageIncomplete: "gate.secureBackupFailureSecretStorageIncomplete",
      artifactDelivery: "gate.secureBackupFailureArtifactDelivery",
      forbidden: "gate.secureBackupFailureForbidden",
      timeout: "gate.secureBackupFailureTimeout",
      sdk: "gate.secureBackupFailureSdk"
    } as const)[kind]
  );
}

function secureBackupPendingLabel(bucket: PendingKeyCountBucket): string {
  return t(
    ({
      zero: "gate.secureBackupPendingZero",
      one: "gate.secureBackupPendingOne",
      two_to_ten: "gate.secureBackupPendingTwoToTen",
      eleven_to_one_hundred: "gate.secureBackupPendingElevenToOneHundred",
      over_one_hundred: "gate.secureBackupPendingOverOneHundred",
      unknown: "gate.secureBackupPendingUnknown"
    } as const)[bucket]
  );
}

function secureBackupGateHeading(gate: SecureBackupGateState): string {
  switch (gate.kind) {
    case "checking":
      return t("gate.secureBackupChecking");
    case "creatingBackup":
      return t("gate.secureBackupCreating");
    case "recoveryKeyDeliveryRequired":
      return t("gate.secureBackupDeliveryRequired");
    case "uploadingExistingKeys":
      return t("gate.secureBackupUploading");
    case "degradedRetrying":
      return t("gate.secureBackupRetrying");
    default:
      return t("gate.secureBackupTitle");
  }
}

export function secureBackupGateFailure(
  gate: SecureBackupGateState
): SecureBackupGateFailureKind | null {
  if (gate.kind === "existingBackupNeedsRecovery") {
    return gate.failure ?? null;
  }
  if (gate.kind === "degradedRetrying" || gate.kind === "blockedFailed") {
    return gate.failure;
  }
  return null;
}

export function SessionVerificationGate({
  snapshot,
  onSnapshot,
  onSignOut,
  onStartWindowDrag = startSessionVerificationWindowDrag,
  operations: providedOperations
}: {
  snapshot: DesktopSnapshot;
  onSnapshot: (snapshot: DesktopSnapshot) => void;
  onSignOut: () => void;
  onStartWindowDrag?: () => void;
  operations?: SessionVerificationGateOperations;
}) {
  const session = snapshot.state.domain.session;
  const authenticationInvalidated =
    session.kind === "locked" &&
    snapshot.state.domain.session_lock_reason?.kind === "unknownToken";
  const operations = {
    ...defaultSessionVerificationGateOperations,
    ...providedOperations
  };
  const secureBackupGate = snapshot.state.domain.secure_backup_gate;
  const secureBackupGateRequired =
    session.kind === "ready" && secureBackupGate.kind !== "ready";
  const deviceCleanup = snapshot.state.domain.device_cleanup;
  const recoveryRef = useRef<HTMLInputElement>(null);
  const secureBackupRecoveryRef = useRef<HTMLInputElement>(null);
  const secureBackupPassphraseRef = useRef<HTMLInputElement>(null);
  const secureBackupDestinationPathRef = useRef<string | null>(null);
  const cleanupPasswordRef = useRef<HTMLInputElement>(null);
  const passphraseRef = useRef<HTMLInputElement>(null);
  const destinationRef = useRef<HTMLInputElement>(null);
  const flowId = session.flow_id;
  const methods = session.gate?.methods ?? [];
  const awaiting = session.kind === "awaitingVerification";
  const canUseRecoverySecret = methods.includes("recoveryKey") || methods.includes("securityPhrase");
  const deviceVerificationAvailable = methods.includes("existingDeviceSas");
  const sasVerifying =
    session.kind === "verifying" && session.method === "existingDeviceSas";
  const phaseKind = session.kind === "provisional" ? provisionalPhaseKind(session.phase) : null;
  const checking = phaseKind === "checkingTrust";
  const discovering = phaseKind === "discoveringMethods";
  const rechecking = phaseKind === "recheckingTrust";
  const cleanupSurfaceOwned = awaiting || rechecking;
  const preparationFailure =
    session.kind === "provisional" ? provisionalPhaseFailure(session.phase) : null;
  const activelyVerifying = session.kind === "verifying";
  const [gateOperation, setGateOperation] = useState<"recovery" | "sas" | "cleanup" | null>(null);
  const [secureBackupOperation, setSecureBackupOperation] =
    useState<SecureBackupOperationKind | null>(null);
  const [secureBackupOperationError, setSecureBackupOperationError] = useState(false);
  const [secureBackupDestinationSelectionError, setSecureBackupDestinationSelectionError] =
    useState(false);
  const [operationError, setOperationError] = useState<string | null>(null);
  const [confirmDeviceVerification, setConfirmDeviceVerification] = useState(false);
  const [confirmDeviceCleanup, setConfirmDeviceCleanup] = useState(false);
  const [confirmEraseLocalAnyway, setConfirmEraseLocalAnyway] = useState(false);
  const [confirmSecureBackupReenable, setConfirmSecureBackupReenable] = useState(false);
  const [secureBackupDestinationSelected, setSecureBackupDestinationSelected] = useState(false);
  const [secureBackupDestinationChoosing, setSecureBackupDestinationChoosing] = useState(false);
  const gateOperationRef = useRef<"recovery" | "sas" | "cleanup" | null>(null);
  const secureBackupOperationRef = useRef<SecureBackupOperationKind | null>(null);
  const run = async (
    kind: "recovery" | "sas" | "cleanup",
    operation: () => Promise<DesktopSnapshot>
  ) => {
    if (gateOperationRef.current === kind) return;
    gateOperationRef.current = kind;
    setOperationError(null);
    setGateOperation(kind);
    try {
      onSnapshot(await operation());
    } catch {
      setOperationError(
        kind === "cleanup"
          ? t("gate.cleanupCommandFailed")
          : t("gate.verificationCommandFailed")
      );
    } finally {
      gateOperationRef.current = null;
      setGateOperation(null);
    }
  };
  const runSecureBackup = async (
    kind: SecureBackupOperationKind,
    operation: (() => Promise<DesktopSnapshot>) | undefined
  ) => {
    if (secureBackupOperationRef.current !== null) return;
    if (!operation) {
      setSecureBackupOperationError(true);
      return;
    }
    secureBackupOperationRef.current = kind;
    setSecureBackupOperationError(false);
    setSecureBackupOperation(kind);
    try {
      onSnapshot(await operation());
    } catch {
      setSecureBackupOperationError(true);
    } finally {
      secureBackupOperationRef.current = null;
      setSecureBackupOperation(null);
    }
  };
  const recoverSecureBackup = (secret: string) => {
    void runSecureBackup(
      "recovery",
      operations.recoverSecureBackup
        ? () => operations.recoverSecureBackup!(secret)
        : undefined
    );
  };
  const chooseSecureBackupDestination = async () => {
    const operation = operations.chooseSecureBackupDestination;
    if (!operation || secureBackupDestinationChoosing) return;
    setSecureBackupDestinationChoosing(true);
    setSecureBackupOperationError(false);
    setSecureBackupDestinationSelectionError(false);
    try {
      const selected = (await operation())?.trim() || null;
      if (selected) {
        secureBackupDestinationPathRef.current = selected;
        setSecureBackupDestinationSelected(true);
      }
    } catch {
      setSecureBackupDestinationSelectionError(true);
    } finally {
      setSecureBackupDestinationChoosing(false);
    }
  };
  const submitSecureBackupSetup = (kind: "setup" | "reenable") => {
    const passphrase = secureBackupPassphraseRef.current?.value || null;
    const destination = secureBackupDestinationPathRef.current;
    if (!destination) return;
    if (secureBackupPassphraseRef.current) secureBackupPassphraseRef.current.value = "";
    secureBackupDestinationPathRef.current = null;
    setSecureBackupDestinationSelected(false);
    if (kind === "reenable") setConfirmSecureBackupReenable(false);
    void runSecureBackup(
      kind,
      kind === "setup"
        ? operations.setupSecureBackup
          ? () => operations.setupSecureBackup!(passphrase, destination)
          : undefined
        : operations.reenableSecureBackup
          ? () => operations.reenableSecureBackup!(passphrase, destination)
          : undefined
    );
  };
  const secureBackupSetupForm = (kind: "setup" | "reenable") => (
    <ImeSafeForm
      aria-label={t("gate.secureBackupSetupTitle")}
      onSubmit={(event) => {
        event.preventDefault();
        submitSecureBackupSetup(kind);
      }}
    >
      <SecureImeTextField
        ref={secureBackupPassphraseRef}
        aria-label={t("gate.secureBackupPassphrase")}
        autoComplete="new-password"
      />
      <div className="secure-backup-destination-selector">
        <button
          className="dialog-button"
          disabled={
            !operations.chooseSecureBackupDestination ||
            secureBackupDestinationChoosing ||
            secureBackupOperation !== null
          }
          type="button"
          onClick={() => void chooseSecureBackupDestination()}
        >
          {t("gate.secureBackupChooseDestination")}
        </button>
        <span role="status" aria-live="polite">
          {secureBackupDestinationSelected
            ? t("gate.secureBackupDestinationSelected")
            : t("gate.secureBackupDestinationNotSelected")}
        </span>
      </div>
      <button
        className="dialog-button is-primary"
        disabled={secureBackupOperation === kind}
        type="submit"
      >
        {kind === "reenable"
          ? t("gate.secureBackupReenableConfirm")
          : t("gate.secureBackupSetup")}
      </button>
    </ImeSafeForm>
  );
  const retrySecureBackup = () => {
    void runSecureBackup(
      "retry",
      operations.retrySecureBackupInspection
    );
  };
  const openSecureBackupDiagnostics = () => {
    const operation =
      operations.openSecureBackupDiagnostics ??
      (() => api.getDiagnosticSnapshot().then(() => undefined));
    void Promise.resolve(operation()).catch(() => undefined);
  };
  const useRecoveryKey = () => {
    setConfirmDeviceVerification(false);
    recoveryRef.current?.focus();
  };
  const tryDeviceVerification = () => {
    setConfirmDeviceVerification(false);
    void run("sas", operations.startOwnUserSas);
  };
  const startDeviceCleanup = () => {
    setConfirmDeviceCleanup(false);
    void run("cleanup", operations.startDeviceCleanup ?? (() => api.startDeviceCleanup()));
  };
  const eraseLocalDataAnyway = () => {
    setConfirmEraseLocalAnyway(false);
    void run(
      "cleanup",
      operations.eraseLocalDataAnyway ?? (() => api.eraseLocalDataAnyway())
    );
  };
  const heading = authenticationInvalidated
    ? t("gate.sessionExpired")
    : secureBackupGateRequired
    ? secureBackupGateHeading(secureBackupGate)
    : checking
      ? t("gate.checking")
      : rechecking
        ? t("gate.finishing")
        : activelyVerifying
          ? t("gate.verifying")
          : t("gate.title");
  const secureBackupFailureKind = secureBackupGateFailure(secureBackupGate);
  const secureBackupNeedsSetup =
    secureBackupGate.kind === "setupRequired" ||
    secureBackupGate.kind === "recoveryKeyDeliveryRequired";
  const secureBackupNeedsRecovery =
    secureBackupGate.kind === "existingBackupNeedsRecovery" ||
    secureBackupGate.kind === "secureStorageIncomplete";
  return <main className="session-verification-gate" aria-label={secureBackupGateRequired ? t("gate.secureBackupTitle") : heading}>
    <div
      className="session-verification-drag-region"
      data-tauri-drag-region=""
      aria-hidden="true"
      onMouseDown={(event) => {
        if (event.buttons !== 1) return;
        event.preventDefault();
        onStartWindowDrag();
      }}
    />
    <h1>{heading}</h1>
    {secureBackupGateRequired && secureBackupGate.kind === "inactive" && (
      <p>{t("gate.secureBackupInactive")}</p>
    )}
    {secureBackupGateRequired && secureBackupNeedsRecovery && (
      <p>{t("gate.secureBackupNeedsRecovery")}</p>
    )}
    {secureBackupGateRequired && secureBackupFailureKind && (
      <p role="alert">{secureBackupFailureLabel(secureBackupFailureKind)}</p>
    )}
    {secureBackupGateRequired && secureBackupOperationError && (
      <p role="alert">{t("gate.secureBackupCommandFailed")}</p>
    )}
    {secureBackupGateRequired && secureBackupDestinationSelectionError && (
      <p role="alert">{t("gate.secureBackupDestinationSelectionFailed")}</p>
    )}
    {secureBackupGateRequired && secureBackupNeedsRecovery && (
      <ImeSafeForm
        onSubmit={(event) => {
          event.preventDefault();
          const secret = secureBackupRecoveryRef.current?.value.trim() ?? "";
          if (secureBackupRecoveryRef.current) secureBackupRecoveryRef.current.value = "";
          if (secret) recoverSecureBackup(secret);
        }}
      >
        <SecureImeTextField
          ref={secureBackupRecoveryRef}
          aria-label={t("gate.secureBackupRecoveryKey")}
          autoComplete="off"
        />
        <button
          className="dialog-button is-primary"
          disabled={secureBackupOperation === "recovery"}
          type="submit"
        >
          {t("gate.secureBackupRecover")}
        </button>
      </ImeSafeForm>
    )}
    {secureBackupGateRequired && secureBackupNeedsSetup && (
      <>
        <p>{t("gate.secureBackupSetupCopy")}</p>
        {secureBackupGate.kind === "recoveryKeyDeliveryRequired" && (
          <p>{t("gate.secureBackupDeliveryRequired")}</p>
        )}
        {secureBackupSetupForm("setup")}
      </>
    )}
    {secureBackupGateRequired && secureBackupGate.kind === "explicitlyDisabledRequiresSetup" && (
      <>
        <h2>{t("gate.secureBackupExplicitDisabledTitle")}</h2>
        <p>{t("gate.secureBackupExplicitDisabledCopy")}</p>
        <button
          className="dialog-button is-primary"
          disabled={secureBackupOperation !== null}
          type="button"
          onClick={() => setConfirmSecureBackupReenable(true)}
        >
          {t("gate.secureBackupReenable")}
        </button>
        {confirmSecureBackupReenable && (
          <div
            className="trust-verification-dialog"
            role="dialog"
            aria-modal="true"
            aria-label={t("gate.secureBackupReenable")}
          >
            <p>{t("gate.secureBackupExplicitDisabledCopy")}</p>
            {secureBackupSetupForm("reenable")}
            <div className="dialog-actions">
              <button
                className="dialog-button"
                type="button"
                onClick={() => setConfirmSecureBackupReenable(false)}
              >
                {t("action.cancel")}
              </button>
            </div>
          </div>
        )}
      </>
    )}
    {secureBackupGateRequired && secureBackupGate.kind === "uploadingExistingKeys" && (
      <p>{secureBackupPendingLabel(secureBackupGate.pending)}</p>
    )}
    {secureBackupGateRequired && secureBackupGate.kind === "blockedFailed" && (
      <div className="dialog-actions">
        <button
          className="dialog-button is-primary"
          disabled={secureBackupOperation !== null}
          type="button"
          onClick={retrySecureBackup}
        >
          {t("gate.secureBackupRetry")}
        </button>
        <button
          className="dialog-button"
          type="button"
          onClick={openSecureBackupDiagnostics}
        >
          {t("gate.secureBackupDiagnostics")}
        </button>
      </div>
    )}
    {discovering && <p>{t("gate.discovering")}</p>}
    {rechecking && <button
      className="dialog-button is-primary"
      type="button"
      disabled={gateOperation !== null}
      onClick={() => void run(
        "recovery",
        operations.retryCurrentDeviceTrustDiscovery ?? (() => api.retryCurrentDeviceTrustDiscovery())
      )}
    >{t("gate.retry")}</button>}
    {session.kind === "rejecting" && <p>{t("gate.rejecting")}</p>}
    {authenticationInvalidated ? (
      <p>{t("gate.sessionExpiredCopy")}</p>
    ) : session.kind === "locked" ? (
      <p>{t("gate.locked")}</p>
    ) : null}
    {session.gate?.failureKind && <p role="alert">{gateFailureLabel(session.gate.failureKind)}</p>}
    {preparationFailure && <p role="alert">{gateFailureLabel(preparationFailure)}</p>}
    {operationError && !session.gate?.failureKind && !preparationFailure && <p role="alert">{operationError}</p>}
    {session.kind === "rejecting" && session.reason && <p role="alert">{gateRejectLabel(session.reason)}</p>}
    {awaiting && canUseRecoverySecret && <ImeSafeForm onSubmit={(event) => { event.preventDefault(); const secret = recoveryRef.current?.value.trim() ?? ""; if (secret) void run("recovery", () => operations.submitRecovery(secret)); if (recoveryRef.current) recoveryRef.current.value = ""; }}><SecureImeTextField ref={recoveryRef} aria-label={t("gate.recoverySecret")} autoComplete="off"/><button className="dialog-button is-primary" disabled={gateOperation === "recovery"} type="submit">{t("gate.verifyRecoveryKey")}</button></ImeSafeForm>}
    {awaiting && !canUseRecoverySecret && !deviceVerificationAvailable && !methods.includes("bootstrap") && <div className="gate-no-recovery">
      <h2>{t("gate.noRecoveryKeyTitle")}</h2>
      <p>{t("gate.noRecoveryKeyCopy")}</p>
    </div>}
    {awaiting && deviceVerificationAvailable && <button className="dialog-button" disabled={gateOperation === "sas"} onClick={() => setConfirmDeviceVerification(true)}>{t("gate.otherDevice")}</button>}
    {awaiting && deviceVerificationAvailable && confirmDeviceVerification && <div className="trust-verification-dialog" role="dialog" aria-modal="true" aria-labelledby="device-verification-confirm-title">
      <h2 id="device-verification-confirm-title">{t("gate.deviceVerificationDialogTitle")}</h2>
      <p>{t("gate.deviceVerificationDialogCopy")}</p>
      <div className="dialog-actions">
        {canUseRecoverySecret && <button className="dialog-button is-primary" type="button" onClick={useRecoveryKey}>{t("gate.useRecoveryKey")}</button>}
        <button className="dialog-button" type="button" onClick={tryDeviceVerification}>{t("gate.tryDeviceVerificationAnyway")}</button>
        <button className="dialog-button" type="button" onClick={() => setConfirmDeviceVerification(false)}>{t("action.cancel")}</button>
      </div>
    </div>}
    {sasVerifying && session.sas_emojis?.length === 7 && <div className="session-verification-emojis">{session.sas_emojis.map((emoji, index) => <span key={index}>{emoji.symbol} {emoji.description}</span>)}</div>}
    {sasVerifying && session.sas_emojis?.length === 7 && flowId !== undefined && <><button onClick={() => void run("sas", () => api.confirmSasVerification(flowId))}>{t("gate.match")}</button><button onClick={() => void run("sas", () => api.mismatchSasVerification(flowId))}>{t("gate.mismatch")}</button></>}
    {awaiting && methods.includes("bootstrap") && <ImeSafeForm onSubmit={(event) => { event.preventDefault(); const destination = destinationRef.current?.value.trim() ?? ""; const passphrase = passphraseRef.current?.value || null; if (destinationRef.current) destinationRef.current.value = ""; if (passphraseRef.current) passphraseRef.current.value = ""; if (destination) void run("recovery", () => api.startSessionBootstrap(passphrase, destination)); }}><ImeTextField ref={destinationRef} aria-label={t("gate.destination")} syncKey="session-bootstrap-destination"/><SecureImeTextField ref={passphraseRef} aria-label={t("gate.passphrase")} autoComplete="new-password"/><button type="submit">{t("gate.bootstrap")}</button></ImeSafeForm>}
    {session.kind === "awaitingBootstrapConfirmation" && flowId !== undefined && <button onClick={() => void run("recovery", () => api.confirmSessionBootstrapSaved(flowId))}>{t("gate.saved")}</button>}
    {sasVerifying && flowId !== undefined && <button onClick={() => void run("sas", () => api.cancelVerification(flowId))}>{t("action.cancel")}</button>}
    {cleanupSurfaceOwned && deviceCleanup.kind === "offered" && <button className="dialog-button danger" type="button" disabled={gateOperation !== null} onClick={() => setConfirmDeviceCleanup(true)}>{t("gate.cleanupOffer")}</button>}
    {cleanupSurfaceOwned && confirmDeviceCleanup && <ResetLocalDataConfirmationDialog
      isBusy={gateOperation !== null}
      title={t("gate.cleanupDialogTitle")}
      copy={t("gate.cleanupDialogCopy")}
      confirmLabel={t("gate.cleanupConfirm")}
      onCancel={() => setConfirmDeviceCleanup(false)}
      onConfirm={startDeviceCleanup}
    />}
    {cleanupSurfaceOwned && deviceCleanup.kind === "resolvingRemote" && <p>{t("gate.cleanupResolving")}</p>}
    {cleanupSurfaceOwned && deviceCleanup.kind === "removingRemote" && <p>{t("gate.cleanupRemoving")}</p>}
    {cleanupSurfaceOwned && deviceCleanup.kind === "awaitingUia" && <ImeSafeForm onSubmit={(event) => {
      event.preventDefault();
      const password = cleanupPasswordRef.current?.value ?? "";
      if (cleanupPasswordRef.current) cleanupPasswordRef.current.value = "";
      if (password) {
        void run(
          "cleanup",
          () => (operations.submitDeviceCleanupUia ?? ((flowId, secret) => api.submitDeviceCleanupUia(flowId, secret)))(deviceCleanup.flow_id, password)
        );
      }
    }}>
      <SecureImeTextField ref={cleanupPasswordRef} aria-label={t("gate.cleanupAccountPassword")} autoComplete="current-password"/>
      <button className="dialog-button is-primary" disabled={gateOperation === "cleanup"} type="submit">{t("gate.cleanupContinue")}</button>
    </ImeSafeForm>}
    {cleanupSurfaceOwned && deviceCleanup.kind === "remoteFailed" && <>
      <p role="alert">{t("gate.cleanupRemoteFailed")}</p>
      <button className="dialog-button" type="button" disabled={gateOperation !== null} onClick={startDeviceCleanup}>{t("gate.cleanupRetryRemote")}</button>
      <button className="dialog-button danger" type="button" disabled={gateOperation !== null} onClick={() => setConfirmEraseLocalAnyway(true)}>{t("gate.cleanupEraseAnywayOffer")}</button>
    </>}
    {cleanupSurfaceOwned && confirmEraseLocalAnyway && <ResetLocalDataConfirmationDialog
      isBusy={gateOperation !== null}
      title={t("gate.cleanupEraseAnywayTitle")}
      copy={t("gate.cleanupEraseAnywayCopy")}
      confirmLabel={t("gate.cleanupEraseAnywayConfirm")}
      onCancel={() => setConfirmEraseLocalAnyway(false)}
      onConfirm={eraseLocalDataAnyway}
    />}
    {cleanupSurfaceOwned && deviceCleanup.kind === "resettingLocal" && <p>{t("gate.cleanupResettingLocal")}</p>}
    {cleanupSurfaceOwned && deviceCleanup.kind === "erasingLocalAnyway" && <p>{t("gate.cleanupErasingLocal")}</p>}
    {cleanupSurfaceOwned && deviceCleanup.kind === "localResetFailed" && <>
      <p role="alert">{t("gate.cleanupLocalFailed")}</p>
      <button className="dialog-button" type="button" disabled={gateOperation !== null} onClick={startDeviceCleanup}>{t("gate.cleanupRetryLocal")}</button>
    </>}
    <button onClick={onSignOut}>{t("gate.signOut")}</button>
  </main>;
}

function gateFailureLabel(kind: import("../domain/types").VerificationGateFailureKind): string {
  return t(({ network: "trust.failureNetwork", cancelled: "gate.failureCancelled", mismatch: "trust.failureMismatch", forbidden: "gate.failureForbidden", timeout: "trust.failureTimeout", sdk: "gate.failureSdk", noProofMethod: "gate.failureNoProof" } as const)[kind]);
}

function gateRejectLabel(reason: NonNullable<import("../domain/types").SessionState["reason"]>): string {
  return t(reason === "existingIdentityWithoutProof" ? "gate.rejectNoProof" : "gate.rejectUser");
}
