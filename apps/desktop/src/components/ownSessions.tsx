import { ModalDialog } from "./ModalDialog";
import { uiDateTimeFormat } from "../domain/intlFormatCache";
import type {
  DeviceTrustLevel,
  DeviceTrustSummary,
  UnverifiedOwnSessionsAlert
} from "../domain/types";
import { type MessageId, t } from "../i18n/messages";

const TRUST_LABEL: Record<DeviceTrustLevel, MessageId> = {
  unknown: "trust.deviceUnknown",
  unverified: "trust.deviceUnverified",
  verified: "trust.deviceVerified",
  blocked: "trust.deviceBlocked"
};

/** The account's other sessions, unverified first (#1505). */
export function otherOwnSessions(devices: readonly DeviceTrustSummary[]): DeviceTrustSummary[] {
  return devices
    .filter((device) => !device.is_current)
    .sort(
      (a, b) =>
        Number(a.trust_level === "verified") - Number(b.trust_level === "verified") ||
        (b.last_seen_ts ?? 0) - (a.last_seen_ts ?? 0)
    );
}

function OwnSessionRow({ device }: { device: DeviceTrustSummary }) {
  const name = device.display_name?.trim() || device.device_id;
  return (
    <li className="own-session-row" data-trust-level={device.trust_level}>
      <span className="own-session-name">{name}</span>
      {name !== device.device_id ? <code className="own-session-id">{device.device_id}</code> : null}
      <span className="own-session-trust">{t(TRUST_LABEL[device.trust_level])}</span>
      {device.last_seen_ts !== null ? (
        <span className="own-session-last-seen">
          {t("ownSessions.lastSeen", {
            time: uiDateTimeFormat({ dateStyle: "medium", timeStyle: "short" }).format(
              device.last_seen_ts
            )
          })}
        </span>
      ) : null}
    </li>
  );
}

/** Read-only list of the account's other sessions for Settings → Sessions. */
export function OtherOwnSessionsList({
  devices,
  label
}: {
  devices: readonly DeviceTrustSummary[];
  label: string;
}) {
  const others = otherOwnSessions(devices);
  if (devices.length === 0) {
    return <p className="profile-settings-hint">{t("ownSessions.notChecked")}</p>;
  }
  if (others.length === 0) {
    return <p className="profile-settings-hint">{t("ownSessions.none")}</p>;
  }
  return (
    <ul className="own-session-list" aria-label={label}>
      {others.map((device) => (
        <OwnSessionRow key={device.device_id} device={device} />
      ))}
    </ul>
  );
}

/**
 * #1505: the popup raised by an own-session check that found unverified
 * sessions. It names the account (display name, user ID, homeserver) so that,
 * with several accounts, the warning cannot be read as about another one.
 */
export function UnverifiedOwnSessionsDialog({
  account,
  userId,
  homeserver,
  alert,
  devices,
  onReview,
  onLater
}: {
  account: string;
  userId: string;
  homeserver: string;
  alert: UnverifiedOwnSessionsAlert;
  devices: readonly DeviceTrustSummary[];
  onReview: () => void;
  onLater: () => void;
}) {
  const unverified = otherOwnSessions(devices).filter(
    (device) => device.trust_level !== "verified"
  );
  return (
    <ModalDialog
      title={t("ownSessions.title", { account })}
      className="confirmation-modal unverified-own-sessions-modal"
      showCloseButton={false}
      onClose={onLater}
    >
      <div className="confirmation-content">
        <p className="own-sessions-account">{t("ownSessions.account", { userId, homeserver })}</p>
        <p>
          {t("ownSessions.counts", {
            total: alert.total_sessions,
            unverified: alert.unverified_sessions
          })}
        </p>
        <ul className="own-session-list" aria-label={t("ownSessions.unverifiedList")}>
          {unverified.map((device) => (
            <OwnSessionRow key={device.device_id} device={device} />
          ))}
        </ul>
        <p>{t("ownSessions.explain")}</p>
        <div className="dialog-actions">
          <button type="button" className="dialog-button" onClick={onLater}>
            {t("ownSessions.later")}
          </button>
          <button type="button" className="dialog-button is-primary" autoFocus onClick={onReview}>
            {t("ownSessions.review")}
          </button>
        </div>
      </div>
    </ModalDialog>
  );
}
