import type { DesktopSnapshot } from "./types";

export function e2eeSendDiagnosticMessage(snapshot: DesktopSnapshot, roomId: string): string {
  const room = snapshot.state.domain.rooms.find((candidate) => candidate.room_id === roomId);
  const deviceSessions = snapshot.state.domain.device_sessions;
  const ownDevices = deviceSessions.kind === "loaded" ? deviceSessions.devices : null;
  const verifiedOwnDevices = ownDevices?.filter((device) => device.verified).length ?? null;
  const unverifiedOwnDevices = ownDevices?.filter((device) => !device.verified).length ?? null;
  const currentOwnDevice = ownDevices?.find((device) => device.current) ?? null;
  const trustDevices = snapshot.state.domain.e2ee_trust.devices;
  const trustedDevices = trustDevices.filter((device) => device.trust_level === "verified").length;
  const blockedDevices = trustDevices.filter((device) => device.trust_level === "blocked").length;

  return [
    "phase=before_send",
    `room_known=${Boolean(room)}`,
    `encrypted=${Boolean(room?.is_encrypted)}`,
    `dm=${Boolean(room?.is_dm)}`,
    `dm_targets=${room?.dm_user_ids.length ?? 0}`,
    `joined_members=${room?.joined_members ?? "unknown"}`,
    `key_backup=${snapshot.state.domain.e2ee_trust.key_backup.kind}`,
    `cross_signing=${snapshot.state.domain.e2ee_trust.cross_signing.kind}`,
    `own_sessions=${ownDevices?.length ?? "unknown"}`,
    `own_sessions_verified=${verifiedOwnDevices ?? "unknown"}`,
    `own_sessions_unverified=${unverifiedOwnDevices ?? "unknown"}`,
    `current_session_verified=${currentOwnDevice?.verified ?? "unknown"}`,
    `trust_devices=${trustDevices.length}`,
    `trust_devices_verified=${trustedDevices}`,
    `trust_devices_blocked=${blockedDevices}`
  ].join(" ");
}
