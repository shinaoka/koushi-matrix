import { type FormEvent, type RefObject } from "react";
import { Hash, KeyRound, ShieldCheck } from "lucide-react";
import { t } from "../i18n/messages";
import type {
  AuthFailureKind,
  DesktopSnapshot,
  LoginFlow
} from "../domain/types";
import { ICON_SIZE } from "../app/uiShared";

export function RecoveryPanel({
  isBusy,
  secretFilled,
  secretInputRef,
  snapshot,
  onSecretPresenceChange,
  onSubmit
}: {
  isBusy: boolean;
  secretFilled: boolean;
  secretInputRef: RefObject<HTMLInputElement | null>;
  snapshot: DesktopSnapshot;
  onSecretPresenceChange: (value: boolean) => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
}) {
  const primaryError = snapshot.state.errors.at(-1);
  const session = snapshot.state.session;

  return (
    <section className="recovery-panel-body" data-testid="recovery-panel">
      <form className="recovery-panel-form" onSubmit={onSubmit}>
        <div className="auth-brand">
          <div className="auth-mark recovery-mark">
            <ShieldCheck size={ICON_SIZE.auth} />
          </div>
          <div>
            <h1>{t("auth.encryptionRecovery")}</h1>
            <p dir="auto">{session.user_id ?? t("auth.matrixAccount")}</p>
          </div>
        </div>
        <div className="recovery-summary">
          <KeyRound size={ICON_SIZE.control} />
          <div className="recovery-methods" aria-label={t("auth.supportedRecoveryMethods")}>
            {(session.recovery_methods ?? ["recoveryKey", "securityPhrase"]).map((method) => (
              <span className="recovery-chip" key={method}>
                {recoveryMethodLabel(method)}
              </span>
            ))}
          </div>
        </div>
        <label className="auth-field">
          <span>{t("auth.recoverySecret")}</span>
          <input
            autoComplete="off"
            name="recoverySecret"
            ref={secretInputRef}
            spellCheck={false}
            type="password"
            onInput={(event) => onSecretPresenceChange(event.currentTarget.value.length > 0)}
          />
        </label>
        {primaryError ? (
          <div className="auth-error" role="alert">
            {primaryError.message}
          </div>
        ) : null}
        <button className="auth-submit" disabled={isBusy || !secretFilled} type="submit">
          {isBusy ? t("action.recovering") : t("action.recover")}
        </button>
      </form>
    </section>
  );
}

export function AuthScreen({
  deviceName,
  homeserver,
  isBusy,
  passwordFilled,
  passwordInputRef,
  snapshot,
  username,
  onDeviceNameChange,
  onDiscoverLoginMethods,
  onHomeserverChange,
  onPasswordPresenceChange,
  onSubmit,
  onUsernameChange
}: {
  deviceName: string;
  homeserver: string;
  isBusy: boolean;
  passwordFilled: boolean;
  passwordInputRef: RefObject<HTMLInputElement | null>;
  snapshot: DesktopSnapshot;
  username: string;
  onDeviceNameChange: (value: string) => void;
  onDiscoverLoginMethods: () => void;
  onHomeserverChange: (value: string) => void;
  onPasswordPresenceChange: (value: boolean) => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
  onUsernameChange: (value: string) => void;
}) {
  const primaryError = snapshot.state.errors.at(-1);
  const auth = snapshot.state.auth;
  const passwordLoginAvailable =
    auth.kind !== "ready" || auth.flows.some((flow) => flow.kind === "password");

  return (
    <main className="auth-screen" data-testid="auth-screen">
      <form className="auth-panel" onSubmit={onSubmit}>
        <div className="auth-brand">
          <div className="auth-mark">
            <Hash size={ICON_SIZE.large} />
          </div>
          <div>
            <h1>{t("auth.matrixDesktop")}</h1>
            <p>{sessionLabel(snapshot.state.session.kind)}</p>
          </div>
        </div>
        <label className="auth-field">
          <span>{t("settings.homeserver")}</span>
          <input
            autoComplete="url"
            name="homeserver"
            spellCheck={false}
            value={homeserver}
            onChange={(event) => onHomeserverChange(event.target.value)}
          />
        </label>
        <div className="auth-discovery">
          <button
            className="auth-secondary"
            disabled={isBusy || !homeserver.trim()}
            type="button"
            onClick={onDiscoverLoginMethods}
          >
            {t("auth.checkLoginMethods")}
          </button>
          <div className="auth-flows">{authDiscoveryLabel(auth)}</div>
        </div>
        <label className="auth-field">
          <span>{t("auth.usernameOrMatrixId")}</span>
          <input
            autoComplete="username"
            name="username"
            spellCheck={false}
            value={username}
            onChange={(event) => onUsernameChange(event.target.value)}
          />
        </label>
        <label className="auth-field">
          <span>{t("auth.password")}</span>
          <input
            autoComplete="current-password"
            name="password"
            ref={passwordInputRef}
            type="password"
            disabled={!passwordLoginAvailable}
            onInput={(event) => onPasswordPresenceChange(event.currentTarget.value.length > 0)}
          />
        </label>
        <label className="auth-field">
          <span>{t("auth.deviceName")}</span>
          <input
            autoComplete="off"
            name="deviceName"
            spellCheck={false}
            value={deviceName}
            onChange={(event) => onDeviceNameChange(event.target.value)}
          />
        </label>
        {primaryError ? (
          <div className="auth-error" role="alert">
            {primaryError.message}
          </div>
        ) : null}
        <button
          className="auth-submit"
          disabled={
            isBusy ||
            !homeserver.trim() ||
            !username.trim() ||
            !passwordFilled ||
            !passwordLoginAvailable
          }
          type="submit"
        >
          {isBusy ? t("auth.connecting") : t("auth.continue")}
        </button>
      </form>
    </main>
  );
}

function authDiscoveryLabel(auth: DesktopSnapshot["state"]["auth"]) {
  switch (auth.kind) {
    case "discovering":
      return t("auth.checking");
    case "ready": {
      const labels = auth.flows.map(authFlowLabel);
      return labels.length ? labels.join(" / ") : t("auth.noLoginMethods");
    }
    case "failed":
      return authFailureLabel(auth.failureKind);
    case "unknown":
    default:
      return t("auth.notChecked");
  }
}

function authFlowLabel(flow: LoginFlow): string {
  if (flow.display_name) {
    return flow.display_name;
  }

  switch (flow.kind) {
    case "password":
      return t("auth.flowPassword");
    case "sso":
      return t("auth.flowSso");
    case "oidc":
      return t("auth.flowOidc");
    case "token":
      return t("auth.flowToken");
    default:
      return t("auth.flowUnknown");
  }
}

function authFailureLabel(kind: AuthFailureKind): string {
  switch (kind) {
    case "network":
      return t("auth.failureNetwork");
    case "unsupported":
      return t("auth.failureUnsupported");
    case "cancelled":
      return t("auth.notChecked");
    case "forbidden":
      return t("auth.failureForbidden");
    case "timeout":
      return t("auth.failureTimeout");
    case "sdk":
      return t("auth.failureSdk");
  }
}

function sessionLabel(kind: DesktopSnapshot["state"]["session"]["kind"]) {
  switch (kind) {
    case "authenticating":
      return t("auth.connecting");
    case "needsRecovery":
      return t("auth.encryptionRecovery");
    case "recovering":
      return t("action.recovering");
    case "locked":
      return t("auth.sessionLocked");
    case "signedOut":
    default:
      return t("auth.signIn");
  }
}

function recoveryMethodLabel(
  method: NonNullable<DesktopSnapshot["state"]["session"]["recovery_methods"]>[number]
) {
  switch (method) {
    case "recoveryKey":
      return t("auth.recoveryKey");
    case "securityPhrase":
      return t("auth.securityPhrase");
  }
}
