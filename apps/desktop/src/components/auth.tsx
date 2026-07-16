import { type FormEvent, type RefObject } from "react";
import { Hash, KeyRound, ShieldCheck } from "lucide-react";
import { t } from "../i18n/messages";
import type {
  AppError,
  AuthFailureKind,
  DesktopSnapshot,
  LoginFlow
} from "../domain/types";
import { ICON_SIZE } from "../app/uiShared";
import { ImeSafeForm, ImeTextField, SecureImeTextField } from "./ImeTextControl";

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
  const primaryError = latestRecoveryError(snapshot.state.ui.errors);
  const session = snapshot.state.domain.session;

  return (
    <section className="recovery-panel-body" data-testid="recovery-panel">
      <ImeSafeForm className="recovery-panel-form" onSubmit={onSubmit}>
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
          <SecureImeTextField
            autoComplete="off"
            name="recoverySecret"
            ref={secretInputRef}
            spellCheck={false}
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
      </ImeSafeForm>
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
  onStartOidcLogin,
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
  onStartOidcLogin: () => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
  onUsernameChange: (value: string) => void;
}) {
  const primaryError = latestAuthError(snapshot.state.ui.errors);
  const session = snapshot.state.domain.session;
  const isLockedSession = session.kind === "locked";
  const auth = snapshot.state.domain.auth;
  const oidcFlow =
    auth.kind === "ready"
      ? auth.flows.find((flow) => flow.kind === "oidc" || flow.kind === "sso")
      : undefined;
  const registrationUrl =
    auth.kind === "ready" ? auth.delegated.registration_url : null;
  const passwordLoginAvailable =
    auth.kind !== "ready" || auth.flows.some((flow) => flow.kind === "password");

  return (
    <main className="auth-screen" data-testid="auth-screen">
      <ImeSafeForm className="auth-panel" onSubmit={onSubmit}>
        <div className="auth-brand">
          <div className="auth-mark">
            <Hash size={ICON_SIZE.large} />
          </div>
          <div>
            <h1>{t("auth.matrixDesktop")}</h1>
            <p>{sessionLabel(session.kind)}</p>
          </div>
        </div>
        {isLockedSession ? (
          <>
            <div className="auth-session-summary">
              <span>{t("settings.matrixAccount")}</span>
              <strong dir="auto">{session.user_id ?? t("auth.matrixAccount")}</strong>
            </div>
            <label className="auth-field">
              <span>{t("auth.password")}</span>
              <SecureImeTextField
                autoComplete="current-password"
                name="password"
                ref={passwordInputRef}
                onInput={(event) => onPasswordPresenceChange(event.currentTarget.value.length > 0)}
              />
            </label>
            {primaryError ? (
              <div className="auth-error" role="alert">
                {primaryError.message}
              </div>
            ) : null}
            <button
              className="auth-submit"
              disabled={isBusy || !passwordFilled}
              type="submit"
            >
              {isBusy ? t("auth.connecting") : t("auth.continue")}
            </button>
          </>
        ) : (
          <>
            <label className="auth-field">
              <span>{t("settings.homeserver")}</span>
              <ImeTextField
                autoComplete="url"
                name="homeserver"
                spellCheck={false}
                value={homeserver}
                syncKey="login-homeserver"
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
            {oidcFlow ? (
              <div className="auth-oidc-actions">
                <button
                  className="auth-secondary"
                  disabled={isBusy}
                  type="button"
                  onClick={onStartOidcLogin}
                >
                  {authFlowLabel(oidcFlow)}
                </button>
                {registrationUrl ? (
                  <a className="auth-create-account" href={registrationUrl}>
                    {t("auth.createAccount")}
                  </a>
                ) : null}
              </div>
            ) : null}
            <label className="auth-field">
              <span>{t("auth.username")}</span>
              <ImeTextField
                aria-label={t("auth.username")}
                autoComplete="username"
                name="username"
                placeholder={t("auth.usernamePlaceholder")}
                spellCheck={false}
                value={username}
                syncKey="login-username"
                onChange={(event) => onUsernameChange(event.target.value)}
              />
            </label>
            <p className="auth-field-help">{t("auth.usernameHelp")}</p>
            <label className="auth-field">
              <span>{t("auth.password")}</span>
              <SecureImeTextField
                autoComplete="current-password"
                name="password"
                ref={passwordInputRef}
                disabled={!passwordLoginAvailable}
                onInput={(event) => onPasswordPresenceChange(event.currentTarget.value.length > 0)}
              />
            </label>
            <label className="auth-field">
              <span>{t("auth.deviceName")}</span>
              <ImeTextField
                autoComplete="off"
                name="deviceName"
                spellCheck={false}
                value={deviceName}
                syncKey="login-device-name"
                onChange={(event) => onDeviceNameChange(event.target.value)}
              />
            </label>
            {primaryError ? (
              <div className="auth-error" role="alert">
                {primaryError.message}
                {primaryError.code === "login_failed" ? (
                  <p className="auth-error-help">{t("auth.loginFailureUsernameHint")}</p>
                ) : null}
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
          </>
        )}
      </ImeSafeForm>
    </main>
  );
}

function latestAuthError(errors: AppError[]): AppError | undefined {
  return [...errors]
    .reverse()
    .find((error) =>
      error.code === "login_failed" ||
      error.code === "restore_failed" ||
      error.code === "sync_auth_required"
    );
}

function latestRecoveryError(errors: AppError[]): AppError | undefined {
  return [...errors].reverse().find((error) => error.code === "e2ee_recovery_failed");
}

function authDiscoveryLabel(auth: DesktopSnapshot["state"]["domain"]["auth"]) {
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

function sessionLabel(kind: DesktopSnapshot["state"]["domain"]["session"]["kind"]) {
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
  method: NonNullable<DesktopSnapshot["state"]["domain"]["session"]["recovery_methods"]>[number]
) {
  switch (method) {
    case "recoveryKey":
      return t("auth.recoveryKey");
    case "securityPhrase":
      return t("auth.securityPhrase");
  }
}
