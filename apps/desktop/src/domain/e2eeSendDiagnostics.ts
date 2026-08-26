import type { DesktopSnapshot } from "./types";

export function e2eeSendDiagnosticMessage(snapshot: DesktopSnapshot, roomId: string): string {
  const room = snapshot.state.domain.rooms.find((candidate) => candidate.room_id === roomId);
  const currentSessionStatus = snapshot.state.domain.current_session_status;
  const currentDeviceVerification =
    currentSessionStatus.status === "ready"
      ? currentSessionStatus.details.verification
      : "unknown";
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
    `current_session_verification=${currentDeviceVerification}`,
    `trust_devices=${trustDevices.length}`,
    `trust_devices_verified=${trustedDevices}`,
    `trust_devices_blocked=${blockedDevices}`
  ].join(" ");
}
