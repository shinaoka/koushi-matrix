use std::{fmt, path::PathBuf};

use crate::ids::{AccountKey, RequestId};
use koushi_state::{
    DisplayPlatform, IdentityResetAuthRequest, LoginRequest, PresenceKind, RecoveryRequest,
    VerificationCancelReason, VerificationTarget,
};

#[derive(Clone, Eq, PartialEq)]
pub struct RoomKeyExportRequest {
    pub destination_path: PathBuf,
    pub passphrase: koushi_state::AuthSecret,
}

impl fmt::Debug for RoomKeyExportRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RoomKeyExportRequest")
            .field("destination_path", &"DestinationPath(..)")
            .field("passphrase", &"AuthSecret(..)")
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct RoomKeyImportRequest {
    pub source_path: PathBuf,
    pub passphrase: koushi_state::AuthSecret,
}

impl fmt::Debug for RoomKeyImportRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RoomKeyImportRequest")
            .field("source_path", &"SourcePath(..)")
            .field("passphrase", &"AuthSecret(..)")
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct SecureBackupSetupRequest {
    pub passphrase: Option<koushi_state::AuthSecret>,
    pub recovery_key_destination_path: Option<PathBuf>,
    pub explicit_reenable_confirmed: bool,
}

impl fmt::Debug for SecureBackupSetupRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecureBackupSetupRequest")
            .field("has_passphrase", &self.passphrase.is_some())
            .field(
                "has_recovery_key_destination_path",
                &self.recovery_key_destination_path.is_some(),
            )
            .field(
                "explicit_reenable_confirmed",
                &self.explicit_reenable_confirmed,
            )
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct SecureBackupPassphraseChangeRequest {
    pub old_secret: koushi_state::AuthSecret,
    pub new_passphrase: koushi_state::AuthSecret,
    pub recovery_key_destination_path: Option<PathBuf>,
}

impl fmt::Debug for SecureBackupPassphraseChangeRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecureBackupPassphraseChangeRequest")
            .field(
                "has_recovery_key_destination_path",
                &self.recovery_key_destination_path.is_some(),
            )
            .field("old_secret", &"AuthSecret(..)")
            .field("new_passphrase", &"AuthSecret(..)")
            .finish()
    }
}

// LoginRequest and RecoveryRequest redact their own Debug in
// koushi-state (username, password, device name, recovery secret).
pub enum AccountCommand {
    DiscoverLogin {
        request_id: RequestId,
        homeserver: String,
    },
    StartOidcLogin {
        request_id: RequestId,
        homeserver: String,
    },
    CompleteOidcLogin {
        request_id: RequestId,
        callback_url: String,
        platform: DisplayPlatform,
    },
    LoginPassword {
        request_id: RequestId,
        request: LoginRequest,
        platform: DisplayPlatform,
    },
    RestoreSession {
        request_id: RequestId,
        account_key: AccountKey,
    },
    /// Restore whichever account the last-session pointer designates. The
    /// pointer is resolved inside the StoreActor/AccountActor — the transport
    /// adapter never reads the credential store. A missing pointer (or
    /// missing session data) is a NORMAL outcome reported as
    /// `CoreFailure::SessionNotFound`; the UI goes to login quietly.
    RestoreLastSession {
        request_id: RequestId,
    },
    /// Retry the required Simplified Sliding Sync capability check for the
    /// currently blocked stored-session restore.
    RetrySlidingSyncCapability {
        request_id: RequestId,
    },
    /// Leave the current local admission and return to homeserver selection
    /// without contacting the old server or deleting its saved session/store.
    ChangeHomeserver {
        request_id: RequestId,
    },
    /// List saved sessions (homeserver / user_id / device_id only — never
    /// secrets). Answered by `AccountEvent::SavedSessionsListed`.
    QuerySavedSessions {
        request_id: RequestId,
    },
    RefreshCurrentSessionStatus {
        request_id: RequestId,
        trigger: koushi_state::SessionStatusRefreshTrigger,
    },
    LoadAccountManagementCapabilities {
        request_id: RequestId,
    },
    ChangePassword {
        request_id: RequestId,
        new_password: koushi_state::AuthSecret,
    },
    DeactivateAccount {
        request_id: RequestId,
        erase_data: bool,
    },
    SubmitAccountManagementUia {
        request_id: RequestId,
        flow_id: u64,
        auth: IdentityResetAuthRequest,
    },
    SoftLogoutReauth {
        request_id: RequestId,
        password: koushi_state::AuthSecret,
    },
    ExportRoomKeys {
        request_id: RequestId,
        request: RoomKeyExportRequest,
    },
    ImportRoomKeys {
        request_id: RequestId,
        request: RoomKeyImportRequest,
    },
    BootstrapSecureBackup {
        request_id: RequestId,
        request: SecureBackupSetupRequest,
    },
    RecoverSecureBackup {
        request_id: RequestId,
        request: RecoveryRequest,
    },
    RetrySecureBackupInspection {
        request_id: RequestId,
    },
    ChangeSecureBackupPassphrase {
        request_id: RequestId,
        request: SecureBackupPassphraseChangeRequest,
    },
    ProbeLocalEncryptionHealth {
        request_id: RequestId,
    },
    ResetLocalData {
        request_id: RequestId,
    },
    StartDeviceCleanup {
        request_id: RequestId,
    },
    SubmitDeviceCleanupUia {
        request_id: RequestId,
        flow_id: u64,
        password: koushi_state::AuthSecret,
    },
    EraseDeviceCleanupLocalDataAnyway {
        request_id: RequestId,
    },
    SubmitRecovery {
        request_id: RequestId,
        request: RecoveryRequest,
    },
    StartSessionBootstrap {
        request_id: RequestId,
        flow_id: u64,
        auth: Option<koushi_state::AuthSecret>,
        request: SecureBackupSetupRequest,
    },
    ConfirmSessionBootstrapSaved {
        request_id: RequestId,
        flow_id: u64,
    },
    StartOwnUserSas {
        request_id: RequestId,
        flow_id: u64,
    },
    RetryCurrentDeviceTrustDiscovery {
        request_id: RequestId,
    },
    RequestVerification {
        request_id: RequestId,
        target: VerificationTarget,
    },
    AcceptVerification {
        request_id: RequestId,
        flow_id: u64,
    },
    ConfirmSasVerification {
        request_id: RequestId,
        flow_id: u64,
    },
    CancelVerification {
        request_id: RequestId,
        flow_id: u64,
        reason: VerificationCancelReason,
    },
    BootstrapCrossSigning {
        request_id: RequestId,
        auth: Option<koushi_state::AuthSecret>,
    },
    EnableKeyBackup {
        request_id: RequestId,
        passphrase: Option<koushi_state::AuthSecret>,
    },
    RestoreKeyBackup {
        request_id: RequestId,
        version: Option<String>,
        request: RecoveryRequest,
    },
    #[cfg(feature = "qa-bin")]
    QaSetLocalDeviceBlacklisted {
        request_id: RequestId,
        target: VerificationTarget,
        room_id: String,
        acknowledged: tokio::sync::oneshot::Sender<Result<(), ()>>,
    },
    #[cfg(feature = "qa-bin")]
    QaRefreshDeviceKeysAndAssertKnown {
        request_id: RequestId,
        target: VerificationTarget,
        acknowledged: tokio::sync::oneshot::Sender<Result<(), ()>>,
    },
    ResetIdentity {
        request_id: RequestId,
    },
    CancelIdentityReset {
        request_id: RequestId,
        flow_id: u64,
    },
    SubmitIdentityResetAuth {
        request_id: RequestId,
        flow_id: u64,
        request: IdentityResetAuthRequest,
    },
    SetPresence {
        request_id: RequestId,
        presence: PresenceKind,
    },
    SetDisplayName {
        request_id: RequestId,
        display_name: Option<String>,
    },
    SetLocalUserAlias {
        request_id: RequestId,
        user_id: String,
        alias: Option<String>,
    },
    SetAvatar {
        request_id: RequestId,
        request: SetAvatarRequest,
    },
    DownloadAvatarThumbnail {
        request_id: RequestId,
        mxc_uri: String,
    },
    IgnoreUser {
        request_id: RequestId,
        user_id: String,
    },
    UnignoreUser {
        request_id: RequestId,
        user_id: String,
    },
    ReportUser {
        request_id: RequestId,
        user_id: String,
        reason: String,
    },
    Logout {
        request_id: RequestId,
    },
    SwitchAccount {
        request_id: RequestId,
        account_key: AccountKey,
    },
}

impl AccountCommand {
    pub fn requires_ready_session(&self) -> bool {
        #[cfg(feature = "qa-bin")]
        if matches!(self, Self::QaRefreshDeviceKeysAndAssertKnown { .. }) {
            return true;
        }

        matches!(
            self,
            Self::RequestVerification { .. }
                | Self::RetryCurrentDeviceTrustDiscovery { .. }
                | Self::AcceptVerification { .. }
                | Self::ConfirmSasVerification { .. }
                | Self::CancelVerification { .. }
                | Self::BootstrapCrossSigning { .. }
                | Self::EnableKeyBackup { .. }
                | Self::ResetIdentity { .. }
                | Self::CancelIdentityReset { .. }
                | Self::SubmitIdentityResetAuth { .. }
                | Self::RefreshCurrentSessionStatus { .. }
                | Self::LoadAccountManagementCapabilities { .. }
                | Self::ChangePassword { .. }
                | Self::DeactivateAccount { .. }
                | Self::SubmitAccountManagementUia { .. }
                | Self::ExportRoomKeys { .. }
                | Self::ImportRoomKeys { .. }
                | Self::BootstrapSecureBackup { .. }
                | Self::RecoverSecureBackup { .. }
                | Self::RetrySecureBackupInspection { .. }
                | Self::ChangeSecureBackupPassphrase { .. }
                | Self::SetPresence { .. }
                | Self::SetDisplayName { .. }
                | Self::SetLocalUserAlias { .. }
                | Self::SetAvatar { .. }
                | Self::DownloadAvatarThumbnail { .. }
                | Self::IgnoreUser { .. }
                | Self::UnignoreUser { .. }
                | Self::ReportUser { .. }
                | Self::ProbeLocalEncryptionHealth { .. }
        )
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct SetAvatarRequest {
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

impl fmt::Debug for SetAvatarRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SetAvatarRequest")
            .field("mime_type", &self.mime_type)
            .field("bytes", &"AvatarBytes(..)")
            .field("bytes_len", &self.bytes.len())
            .finish()
    }
}

impl fmt::Debug for AccountCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DiscoverLogin { request_id, .. } => formatter
                .debug_struct("DiscoverLogin")
                .field("request_id", request_id)
                .field("homeserver", &"Homeserver(..)")
                .finish(),
            Self::StartOidcLogin { request_id, .. } => formatter
                .debug_struct("StartOidcLogin")
                .field("request_id", request_id)
                .field("homeserver", &"Homeserver(..)")
                .finish(),
            Self::CompleteOidcLogin { request_id, .. } => formatter
                .debug_struct("CompleteOidcLogin")
                .field("request_id", request_id)
                .field("homeserver", &"Homeserver(..)")
                .field("callback_url", &"CallbackUrl(..)")
                .finish(),
            Self::LoginPassword {
                request_id,
                request,
                platform,
            } => formatter
                .debug_struct("LoginPassword")
                .field("request_id", request_id)
                .field("request", request)
                .field("platform", platform)
                .finish(),
            Self::RestoreSession {
                request_id,
                account_key,
            } => formatter
                .debug_struct("RestoreSession")
                .field("request_id", request_id)
                .field("account_key", account_key)
                .finish(),
            Self::RestoreLastSession { request_id } => formatter
                .debug_struct("RestoreLastSession")
                .field("request_id", request_id)
                .finish(),
            Self::RetrySlidingSyncCapability { request_id } => formatter
                .debug_struct("RetrySlidingSyncCapability")
                .field("request_id", request_id)
                .finish(),
            Self::ChangeHomeserver { request_id } => formatter
                .debug_struct("ChangeHomeserver")
                .field("request_id", request_id)
                .finish(),
            Self::QuerySavedSessions { request_id } => formatter
                .debug_struct("QuerySavedSessions")
                .field("request_id", request_id)
                .finish(),
            Self::RefreshCurrentSessionStatus {
                request_id,
                trigger,
            } => formatter
                .debug_struct("RefreshCurrentSessionStatus")
                .field("request_id", request_id)
                .field("trigger", trigger)
                .finish(),
            Self::LoadAccountManagementCapabilities { request_id } => formatter
                .debug_struct("LoadAccountManagementCapabilities")
                .field("request_id", request_id)
                .finish(),
            Self::ChangePassword { request_id, .. } => formatter
                .debug_struct("ChangePassword")
                .field("request_id", request_id)
                .field("new_password", &"AuthSecret(..)")
                .finish(),
            Self::DeactivateAccount {
                request_id,
                erase_data,
            } => formatter
                .debug_struct("DeactivateAccount")
                .field("request_id", request_id)
                .field("erase_data", erase_data)
                .finish(),
            Self::SubmitAccountManagementUia {
                request_id,
                flow_id,
                auth,
            } => formatter
                .debug_struct("SubmitAccountManagementUia")
                .field("request_id", request_id)
                .field("flow_id", flow_id)
                .field("auth", auth)
                .finish(),
            Self::SoftLogoutReauth { request_id, .. } => formatter
                .debug_struct("SoftLogoutReauth")
                .field("request_id", request_id)
                .field("password", &"AuthSecret(..)")
                .finish(),
            Self::ExportRoomKeys {
                request_id,
                request,
            } => formatter
                .debug_struct("ExportRoomKeys")
                .field("request_id", request_id)
                .field("request", request)
                .finish(),
            Self::ImportRoomKeys {
                request_id,
                request,
            } => formatter
                .debug_struct("ImportRoomKeys")
                .field("request_id", request_id)
                .field("request", request)
                .finish(),
            Self::BootstrapSecureBackup {
                request_id,
                request,
            } => formatter
                .debug_struct("BootstrapSecureBackup")
                .field("request_id", request_id)
                .field("request", request)
                .finish(),
            Self::RecoverSecureBackup {
                request_id,
                request,
            } => formatter
                .debug_struct("RecoverSecureBackup")
                .field("request_id", request_id)
                .field("request", request)
                .finish(),
            Self::RetrySecureBackupInspection { request_id } => formatter
                .debug_struct("RetrySecureBackupInspection")
                .field("request_id", request_id)
                .finish(),
            Self::ChangeSecureBackupPassphrase {
                request_id,
                request,
            } => formatter
                .debug_struct("ChangeSecureBackupPassphrase")
                .field("request_id", request_id)
                .field("request", request)
                .finish(),
            Self::ProbeLocalEncryptionHealth { request_id } => formatter
                .debug_struct("ProbeLocalEncryptionHealth")
                .field("request_id", request_id)
                .finish(),
            Self::ResetLocalData { request_id } => formatter
                .debug_struct("ResetLocalData")
                .field("request_id", request_id)
                .finish(),
            Self::StartDeviceCleanup { request_id } => formatter
                .debug_struct("StartDeviceCleanup")
                .field("request_id", request_id)
                .finish(),
            Self::SubmitDeviceCleanupUia {
                request_id,
                flow_id,
                ..
            } => formatter
                .debug_struct("SubmitDeviceCleanupUia")
                .field("request_id", request_id)
                .field("flow_id", flow_id)
                .field("password", &"AuthSecret(..)")
                .finish(),
            Self::EraseDeviceCleanupLocalDataAnyway { request_id } => formatter
                .debug_struct("EraseDeviceCleanupLocalDataAnyway")
                .field("request_id", request_id)
                .finish(),
            Self::SubmitRecovery {
                request_id,
                request,
            } => formatter
                .debug_struct("SubmitRecovery")
                .field("request_id", request_id)
                .field("request", request)
                .finish(),
            Self::StartSessionBootstrap {
                request_id,
                flow_id,
                auth,
                request,
            } => formatter
                .debug_struct("StartSessionBootstrap")
                .field("request_id", request_id)
                .field("flow_id", flow_id)
                .field("has_auth", &auth.is_some())
                .field("request", request)
                .finish(),
            Self::ConfirmSessionBootstrapSaved {
                request_id,
                flow_id,
            } => formatter
                .debug_struct("ConfirmSessionBootstrapSaved")
                .field("request_id", request_id)
                .field("flow_id", flow_id)
                .finish(),
            Self::StartOwnUserSas {
                request_id,
                flow_id,
            } => formatter
                .debug_struct("StartOwnUserSas")
                .field("request_id", request_id)
                .field("flow_id", flow_id)
                .finish(),
            Self::RetryCurrentDeviceTrustDiscovery { request_id } => formatter
                .debug_struct("RetryCurrentDeviceTrustDiscovery")
                .field("request_id", request_id)
                .finish(),
            Self::RequestVerification { request_id, .. } => formatter
                .debug_struct("RequestVerification")
                .field("request_id", request_id)
                .field("target", &"VerificationTarget(..)")
                .finish(),
            Self::AcceptVerification {
                request_id,
                flow_id,
            } => formatter
                .debug_struct("AcceptVerification")
                .field("request_id", request_id)
                .field("flow_id", flow_id)
                .finish(),
            Self::ConfirmSasVerification {
                request_id,
                flow_id,
            } => formatter
                .debug_struct("ConfirmSasVerification")
                .field("request_id", request_id)
                .field("flow_id", flow_id)
                .finish(),
            Self::CancelVerification {
                request_id,
                flow_id,
                reason,
            } => formatter
                .debug_struct("CancelVerification")
                .field("request_id", request_id)
                .field("flow_id", flow_id)
                .field("reason", reason)
                .finish(),
            Self::BootstrapCrossSigning { request_id, auth } => formatter
                .debug_struct("BootstrapCrossSigning")
                .field("request_id", request_id)
                .field("auth", auth)
                .finish(),
            Self::EnableKeyBackup {
                request_id,
                passphrase,
            } => formatter
                .debug_struct("EnableKeyBackup")
                .field("request_id", request_id)
                .field("passphrase", passphrase)
                .finish(),
            Self::RestoreKeyBackup {
                request_id,
                version,
                request,
            } => formatter
                .debug_struct("RestoreKeyBackup")
                .field("request_id", request_id)
                .field("version", &version.as_ref().map(|_| "BackupVersion(..)"))
                .field("request", request)
                .finish(),
            #[cfg(feature = "qa-bin")]
            Self::QaSetLocalDeviceBlacklisted { request_id, .. } => formatter
                .debug_struct("QaSetLocalDeviceBlacklisted")
                .field("request_id", request_id)
                .field("target", &"<redacted>")
                .finish(),
            #[cfg(feature = "qa-bin")]
            Self::QaRefreshDeviceKeysAndAssertKnown { request_id, .. } => formatter
                .debug_struct("QaRefreshDeviceKeysAndAssertKnown")
                .field("request_id", request_id)
                .field("target", &"<redacted>")
                .finish(),
            Self::ResetIdentity { request_id } => formatter
                .debug_struct("ResetIdentity")
                .field("request_id", request_id)
                .finish(),
            Self::CancelIdentityReset {
                request_id,
                flow_id,
            } => formatter
                .debug_struct("CancelIdentityReset")
                .field("request_id", request_id)
                .field("flow_id", flow_id)
                .finish(),
            Self::SubmitIdentityResetAuth {
                request_id,
                flow_id,
                request,
            } => formatter
                .debug_struct("SubmitIdentityResetAuth")
                .field("request_id", request_id)
                .field("flow_id", flow_id)
                .field("request", request)
                .finish(),
            Self::SetPresence {
                request_id,
                presence,
            } => formatter
                .debug_struct("SetPresence")
                .field("request_id", request_id)
                .field("presence", presence)
                .finish(),
            Self::SetDisplayName { request_id, .. } => formatter
                .debug_struct("SetDisplayName")
                .field("request_id", request_id)
                .field("display_name", &"ProfileDisplayName(..)")
                .finish(),
            Self::SetLocalUserAlias { request_id, .. } => formatter
                .debug_struct("SetLocalUserAlias")
                .field("request_id", request_id)
                .field("user_id", &"UserId(..)")
                .field("alias", &"LocalUserAlias(..)")
                .finish(),
            Self::SetAvatar {
                request_id,
                request,
            } => formatter
                .debug_struct("SetAvatar")
                .field("request_id", request_id)
                .field("mime_type", &request.mime_type)
                .field("bytes", &"AvatarBytes(..)")
                .field("bytes_len", &request.bytes.len())
                .finish(),
            Self::DownloadAvatarThumbnail { request_id, .. } => formatter
                .debug_struct("DownloadAvatarThumbnail")
                .field("request_id", request_id)
                .field("mxc_uri", &"MxcUri(..)")
                .finish(),
            Self::IgnoreUser { request_id, .. } => formatter
                .debug_struct("IgnoreUser")
                .field("request_id", request_id)
                .field("user_id", &"UserId(..)")
                .finish(),
            Self::UnignoreUser { request_id, .. } => formatter
                .debug_struct("UnignoreUser")
                .field("request_id", request_id)
                .field("user_id", &"UserId(..)")
                .finish(),
            Self::ReportUser { request_id, .. } => formatter
                .debug_struct("ReportUser")
                .field("request_id", request_id)
                .field("user_id", &"UserId(..)")
                .field("reason", &"ReportReason(..)")
                .finish(),
            Self::Logout { request_id } => formatter
                .debug_struct("Logout")
                .field("request_id", request_id)
                .finish(),
            Self::SwitchAccount {
                request_id,
                account_key,
            } => formatter
                .debug_struct("SwitchAccount")
                .field("request_id", request_id)
                .field("account_key", account_key)
                .finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{CoreCommand, test_support::fake_rid};
    use super::*;

    #[test]
    fn soft_logout_reauth_is_allowed_past_ready_session_gate() {
        let command = CoreCommand::Account(AccountCommand::SoftLogoutReauth {
            request_id: fake_rid(73),
            password: koushi_state::AuthSecret::new("synthetic-password"),
        });

        assert!(!command.requires_ready_session());
    }

    #[test]
    fn capability_recovery_commands_are_allowed_while_session_is_blocked() {
        let retry = CoreCommand::Account(AccountCommand::RetrySlidingSyncCapability {
            request_id: fake_rid(74),
        });
        let reset = CoreCommand::Account(AccountCommand::ResetLocalData {
            request_id: fake_rid(75),
        });
        let change_homeserver = CoreCommand::Account(AccountCommand::ChangeHomeserver {
            request_id: fake_rid(76),
        });

        assert!(!retry.requires_ready_session());
        assert!(!reset.requires_ready_session());
        assert!(!change_homeserver.requires_ready_session());
        assert_eq!(retry.request_id(), fake_rid(74));
        assert_eq!(change_homeserver.request_id(), fake_rid(76));
        assert!(format!("{retry:?}").contains("RetrySlidingSyncCapability"));
        assert!(format!("{change_homeserver:?}").contains("ChangeHomeserver"));
    }

    #[cfg(feature = "qa-bin")]
    #[test]
    fn qa_device_key_refresh_is_ready_gated_correlated_and_redacted() {
        let request_id = fake_rid(74);
        let (acknowledged, _ack) = tokio::sync::oneshot::channel();
        let command = CoreCommand::Account(AccountCommand::QaRefreshDeviceKeysAndAssertKnown {
            request_id,
            target: VerificationTarget {
                user_id: "@private-user:example.invalid".to_owned(),
                device_id: "PRIVATEDEVICE".to_owned(),
            },
            acknowledged,
        });

        assert_eq!(command.request_id(), request_id);
        assert!(command.requires_ready_session());
        let debug = format!("{command:?}");
        assert!(
            debug.contains("QaRefreshDeviceKeysAndAssertKnown"),
            "{debug}"
        );
        assert!(debug.contains("<redacted>"), "{debug}");
        assert!(!debug.contains("private-user"), "{debug}");
        assert!(!debug.contains("PRIVATEDEVICE"), "{debug}");
    }

    #[test]
    fn profile_command_debug_redacts_display_name_and_avatar_bytes() {
        let display_name = AccountCommand::SetDisplayName {
            request_id: fake_rid(9),
            display_name: Some("Private Display".to_owned()),
        };
        let alias = AccountCommand::SetLocalUserAlias {
            request_id: fake_rid(11),
            user_id: "@private:example.invalid".to_owned(),
            alias: Some("Private Alias".to_owned()),
        };
        let avatar = AccountCommand::SetAvatar {
            request_id: fake_rid(10),
            request: SetAvatarRequest {
                mime_type: "image/png".to_owned(),
                bytes: vec![9, 8, 7, 6],
            },
        };

        let display_debug = format!("{display_name:?}");
        assert!(display_debug.contains("SetDisplayName"), "{display_debug}");
        assert!(
            !display_debug.contains("Private Display"),
            "{display_debug}"
        );

        let alias_debug = format!("{alias:?}");
        assert!(alias_debug.contains("SetLocalUserAlias"), "{alias_debug}");
        assert!(
            !alias_debug.contains("@private:example.invalid"),
            "{alias_debug}"
        );
        assert!(!alias_debug.contains("Private Alias"), "{alias_debug}");

        let avatar_debug = format!("{avatar:?}");
        assert!(avatar_debug.contains("SetAvatar"), "{avatar_debug}");
        assert!(avatar_debug.contains("image/png"), "{avatar_debug}");
        assert!(avatar_debug.contains("bytes_len"), "{avatar_debug}");
        assert!(!avatar_debug.contains("9, 8, 7, 6"), "{avatar_debug}");
    }

    #[test]
    fn device_cleanup_commands_are_provisional_safe_and_redact_passwords() {
        let start = AccountCommand::StartDeviceCleanup {
            request_id: fake_rid(41),
        };
        let submit = AccountCommand::SubmitDeviceCleanupUia {
            request_id: fake_rid(42),
            flow_id: 41,
            password: koushi_state::AuthSecret::new("private-cleanup-password"),
        };
        let erase = AccountCommand::EraseDeviceCleanupLocalDataAnyway {
            request_id: fake_rid(43),
        };

        assert!(!start.requires_ready_session());
        assert!(!submit.requires_ready_session());
        assert!(!erase.requires_ready_session());
        let debug = format!("{submit:?}");
        assert!(debug.contains("AuthSecret(..)"), "{debug}");
        assert!(!debug.contains("private-cleanup-password"), "{debug}");
    }
}
