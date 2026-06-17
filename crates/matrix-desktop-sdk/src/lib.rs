use futures_util::{Stream, StreamExt};
pub use matrix_desktop_state::E2eeRecoveryState;
use matrix_desktop_state::{
    AuthSecret, CrossSigningStatus, DelegatedAuthLinks, IdentityResetAuthRequest,
    IdentityResetAuthType, KeyBackupStatus, LoginFlow, LoginFlowKind, LoginRequest,
    RecoveryRequest, RoomAttentionSummary, SasEmoji, SessionInfo, VerificationTarget,
    room_attention_summary,
};
use matrix_sdk::authentication::matrix::MatrixSession;
use matrix_sdk::room::ParentSpace;
use matrix_sdk::ruma::{
    events::{AnyGlobalAccountDataEventContent, GlobalAccountDataEventType},
    serde::Raw,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fmt,
    future::Future,
    net::IpAddr,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
    time::{Duration, SystemTime},
};
use thiserror::Error;
use url::Url;
use zeroize::Zeroizing;

const LOGIN_DISCOVERY_PATH: &str = "_matrix/client/v3/login";
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);
const MATRIX_ROOM_LIST_SNAPSHOT_LIMIT: usize = 4096;
pub const LOCAL_USER_ALIASES_ACCOUNT_DATA_TYPE: &str = "app.ruri.local_aliases";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoginDiscovery {
    pub homeserver: String,
    pub flows: Vec<LoginFlow>,
    pub delegated: DelegatedAuthLinks,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixLoginDiscovery {
    pub homeserver: String,
    pub flows: Vec<MatrixLoginFlow>,
    pub delegated: DelegatedAuthLinks,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixLoginFlow {
    pub kind: MatrixLoginFlowKind,
    pub delegated_oidc_compatibility: bool,
    pub display_name: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MatrixLoginFlowKind {
    Password,
    Sso,
    Oidc,
    Token,
    Unknown(String),
}

#[derive(Clone, Eq, PartialEq)]
pub struct MatrixDeviceSessionSummary {
    pub raw_device_id: String,
    pub display_name: Option<String>,
    pub current: bool,
    pub verified: bool,
    pub inactive: bool,
}

impl fmt::Debug for MatrixDeviceSessionSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixDeviceSessionSummary")
            .field("raw_device_id", &"DeviceId(..)")
            .field(
                "display_name",
                &self.display_name.as_ref().map(|_| "DeviceDisplayName(..)"),
            )
            .field("current", &self.current)
            .field("verified", &self.verified)
            .field("inactive", &self.inactive)
            .finish()
    }
}

pub type E2eeRecoveryStateStream = Pin<Box<dyn Stream<Item = E2eeRecoveryState> + Send>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MatrixCrossSigningStatus {
    pub has_master: bool,
    pub has_self_signing: bool,
    pub has_user_signing: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixIdentityResetAuthType {
    Uiaa,
    OAuth,
    Unknown,
}

pub struct MatrixIdentityResetHandle {
    inner: matrix_sdk::encryption::recovery::IdentityResetHandle,
    auth_type: MatrixIdentityResetAuthType,
}

impl fmt::Debug for MatrixIdentityResetHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixIdentityResetHandle")
            .field("auth_type", &self.auth_type)
            .finish_non_exhaustive()
    }
}

impl MatrixIdentityResetHandle {
    pub fn auth_type(&self) -> MatrixIdentityResetAuthType {
        self.auth_type
    }

    pub fn desktop_auth_type(&self) -> IdentityResetAuthType {
        map_identity_reset_auth_type_to_desktop(self.auth_type)
    }

    pub async fn cancel(&self) {
        self.inner.cancel().await;
    }

    pub async fn reset(
        &self,
        session: &MatrixClientSession,
        request: &IdentityResetAuthRequest,
    ) -> Result<(), E2eeTrustError> {
        let auth = match request {
            IdentityResetAuthRequest::OAuthApproved => None,
            IdentityResetAuthRequest::UiaaPassword { password } => {
                let matrix_sdk::encryption::CrossSigningResetAuthType::Uiaa(uiaa) =
                    self.inner.auth_type()
                else {
                    return Err(E2eeTrustError::Sdk(
                        "identity reset auth type mismatch".to_owned(),
                    ));
                };
                let identifier = matrix_sdk::ruma::api::client::uiaa::UserIdentifier::Matrix(
                    matrix_sdk::ruma::api::client::uiaa::MatrixUserIdentifier::new(
                        session.info.user_id.clone(),
                    ),
                );
                let mut password_auth = matrix_sdk::ruma::api::client::uiaa::Password::new(
                    identifier,
                    password.expose_secret().to_owned(),
                );
                password_auth.session.clone_from(&uiaa.session);
                Some(matrix_sdk::ruma::api::client::uiaa::AuthData::Password(
                    password_auth,
                ))
            }
        };

        self.inner.reset(auth).await?;
        Ok(())
    }
}

pub enum IdentityResetOutcome {
    Completed,
    AuthRequired(MatrixIdentityResetHandle),
}

impl fmt::Debug for IdentityResetOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Completed => formatter.write_str("Completed"),
            Self::AuthRequired(handle) => formatter
                .debug_tuple("AuthRequired")
                .field(&handle.auth_type())
                .finish(),
        }
    }
}

#[derive(Clone)]
pub struct MatrixVerificationRequestHandle {
    inner: matrix_sdk::encryption::verification::VerificationRequest,
}

impl fmt::Debug for MatrixVerificationRequestHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixVerificationRequestHandle")
            .field("flow_id", &"FlowId(..)")
            .finish_non_exhaustive()
    }
}

impl MatrixVerificationRequestHandle {
    pub fn flow_id(&self) -> &str {
        self.inner.flow_id()
    }

    pub fn state(&self) -> MatrixVerificationRequestState {
        map_sdk_verification_request_state(self.inner.state())
    }

    pub fn changes(&self) -> MatrixVerificationRequestStateStream {
        Box::pin(self.inner.changes().map(map_sdk_verification_request_state))
    }
}

#[derive(Clone)]
pub struct MatrixIncomingVerificationRequest {
    target: VerificationTarget,
    handle: MatrixVerificationRequestHandle,
}

impl MatrixIncomingVerificationRequest {
    pub fn target(&self) -> &VerificationTarget {
        &self.target
    }

    pub fn handle(&self) -> &MatrixVerificationRequestHandle {
        &self.handle
    }

    pub fn into_parts(self) -> (VerificationTarget, MatrixVerificationRequestHandle) {
        (self.target, self.handle)
    }
}

impl fmt::Debug for MatrixIncomingVerificationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixIncomingVerificationRequest")
            .field("target", &"VerificationTarget(..)")
            .field("handle", &self.handle)
            .finish()
    }
}

pub struct MatrixIncomingVerificationRequestObserver {
    client: matrix_sdk::Client,
    receiver: tokio::sync::mpsc::Receiver<MatrixIncomingVerificationRequest>,
    handlers: Vec<matrix_sdk::event_handler::EventHandlerHandle>,
}

impl MatrixIncomingVerificationRequestObserver {
    pub async fn recv(&mut self) -> Option<MatrixIncomingVerificationRequest> {
        self.receiver.recv().await
    }
}

impl fmt::Debug for MatrixIncomingVerificationRequestObserver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixIncomingVerificationRequestObserver")
            .field("pending", &"Receiver(..)")
            .finish_non_exhaustive()
    }
}

impl Drop for MatrixIncomingVerificationRequestObserver {
    fn drop(&mut self) {
        for handler in self.handlers.drain(..) {
            self.client.remove_event_handler(handler);
        }
    }
}

#[derive(Clone)]
pub struct MatrixSasVerificationHandle {
    inner: matrix_sdk::encryption::verification::SasVerification,
}

impl fmt::Debug for MatrixSasVerificationHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixSasVerificationHandle")
            .field("flow_id", &"FlowId(..)")
            .finish_non_exhaustive()
    }
}

impl MatrixSasVerificationHandle {
    pub fn state(&self) -> MatrixSasState {
        map_sdk_sas_state(self.inner.state())
    }

    pub fn changes(&self) -> MatrixSasStateStream {
        Box::pin(self.inner.changes().map(map_sdk_sas_state))
    }

    pub fn emojis(&self) -> Option<Vec<SasEmoji>> {
        self.inner.emoji().map(map_sdk_sas_emojis_to_desktop)
    }
}

pub type MatrixVerificationRequestStateStream =
    Pin<Box<dyn Stream<Item = MatrixVerificationRequestState> + Send>>;
pub type MatrixSasStateStream = Pin<Box<dyn Stream<Item = MatrixSasState> + Send>>;

#[derive(Clone, Debug)]
pub enum MatrixVerificationRequestState {
    Created,
    Requested,
    Ready,
    SasStarted(MatrixSasVerificationHandle),
    Done,
    Cancelled,
    UnsupportedMethod,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MatrixSasState {
    Created,
    Started,
    Accepted,
    SasPresented { emojis: Vec<SasEmoji> },
    Confirmed,
    Done,
    Cancelled,
    UnsupportedShortAuth,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyBackupRestoreScope {
    JoinedRooms,
}

#[derive(Clone, Eq, PartialEq)]
pub struct KeyBackupRestoreSummary {
    pub scope: KeyBackupRestoreScope,
    pub version: Option<String>,
    pub restored_rooms: u64,
    pub total_rooms: Option<u64>,
}

impl fmt::Debug for KeyBackupRestoreSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KeyBackupRestoreSummary")
            .field("scope", &self.scope)
            .field(
                "version",
                &self.version.as_ref().map(|_| "BackupVersion(..)"),
            )
            .field("restored_rooms", &self.restored_rooms)
            .field("total_rooms", &self.total_rooms)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoomKeyExportSummary {
    pub exported_sessions: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoomKeyImportSummary {
    pub imported_count: u64,
    pub total_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SecureBackupSetupSummary {
    pub recovery_key_written: bool,
}

#[derive(Clone, Eq, Error, PartialEq)]
pub enum E2eeTrustError {
    #[error("Matrix encryption is not initialized")]
    NoOlmMachine,
    #[error("Matrix SDK trust operation failed")]
    Sdk(String),
}

#[derive(Debug, thiserror::Error)]
pub enum DeleteDevicesError {
    #[error("interactive authentication required")]
    UiaaChallenge { session: Option<String> },
    #[error("Matrix SDK delete devices failed")]
    Sdk(String),
}

impl fmt::Debug for E2eeTrustError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoOlmMachine => formatter.write_str("NoOlmMachine"),
            Self::Sdk(_) => formatter.write_str("Sdk(..)"),
        }
    }
}

impl From<matrix_sdk::Error> for E2eeTrustError {
    fn from(error: matrix_sdk::Error) -> Self {
        match error {
            matrix_sdk::Error::NoOlmMachine => Self::NoOlmMachine,
            other => Self::Sdk(other.to_string()),
        }
    }
}

impl From<matrix_sdk::encryption::recovery::RecoveryError> for E2eeTrustError {
    fn from(error: matrix_sdk::encryption::recovery::RecoveryError) -> Self {
        Self::Sdk(error.to_string())
    }
}

pub fn map_cross_signing_status_to_desktop(
    status: Option<MatrixCrossSigningStatus>,
) -> CrossSigningStatus {
    match status {
        None => CrossSigningStatus::Missing,
        Some(status) if status.has_master && status.has_self_signing && status.has_user_signing => {
            CrossSigningStatus::Trusted
        }
        Some(_) => CrossSigningStatus::NotTrusted,
    }
}

pub fn map_identity_reset_auth_type_to_desktop(
    auth_type: MatrixIdentityResetAuthType,
) -> IdentityResetAuthType {
    match auth_type {
        MatrixIdentityResetAuthType::Uiaa => IdentityResetAuthType::Uiaa,
        MatrixIdentityResetAuthType::OAuth => IdentityResetAuthType::OAuth,
        MatrixIdentityResetAuthType::Unknown => IdentityResetAuthType::Unknown,
    }
}

fn map_sdk_identity_reset_auth_type(
    auth_type: &matrix_sdk::encryption::CrossSigningResetAuthType,
) -> MatrixIdentityResetAuthType {
    match auth_type {
        matrix_sdk::encryption::CrossSigningResetAuthType::Uiaa(_) => {
            MatrixIdentityResetAuthType::Uiaa
        }
        matrix_sdk::encryption::CrossSigningResetAuthType::OAuth(_) => {
            MatrixIdentityResetAuthType::OAuth
        }
    }
}

fn map_sdk_verification_request_state(
    state: matrix_sdk::encryption::verification::VerificationRequestState,
) -> MatrixVerificationRequestState {
    use matrix_sdk::encryption::verification::{
        Verification, VerificationRequestState as SdkVerificationRequestState,
    };

    match state {
        SdkVerificationRequestState::Created { .. } => MatrixVerificationRequestState::Created,
        SdkVerificationRequestState::Requested { .. } => MatrixVerificationRequestState::Requested,
        SdkVerificationRequestState::Ready { .. } => MatrixVerificationRequestState::Ready,
        SdkVerificationRequestState::Transitioned { verification } => match verification {
            Verification::SasV1(inner) => {
                MatrixVerificationRequestState::SasStarted(MatrixSasVerificationHandle { inner })
            }
            #[allow(unreachable_patterns)]
            _ => MatrixVerificationRequestState::UnsupportedMethod,
        },
        SdkVerificationRequestState::Done => MatrixVerificationRequestState::Done,
        SdkVerificationRequestState::Cancelled(_) => MatrixVerificationRequestState::Cancelled,
    }
}

fn map_sdk_sas_state(state: matrix_sdk::encryption::verification::SasState) -> MatrixSasState {
    use matrix_sdk::encryption::verification::SasState as SdkSasState;

    match state {
        SdkSasState::Created { .. } => MatrixSasState::Created,
        SdkSasState::Started { .. } => MatrixSasState::Started,
        SdkSasState::Accepted { .. } => MatrixSasState::Accepted,
        SdkSasState::KeysExchanged { emojis, .. } => match emojis {
            Some(emojis) => MatrixSasState::SasPresented {
                emojis: map_sdk_sas_emojis_to_desktop(emojis.emojis),
            },
            None => MatrixSasState::UnsupportedShortAuth,
        },
        SdkSasState::Confirmed => MatrixSasState::Confirmed,
        SdkSasState::Done { .. } => MatrixSasState::Done,
        SdkSasState::Cancelled(_) => MatrixSasState::Cancelled,
    }
}

pub fn map_sdk_sas_emojis_to_desktop(
    emojis: [matrix_sdk::encryption::verification::Emoji; 7],
) -> Vec<SasEmoji> {
    emojis
        .into_iter()
        .map(|emoji| SasEmoji {
            symbol: emoji.symbol.to_owned(),
            description: emoji.description.to_owned(),
        })
        .collect()
}

pub async fn cross_signing_status(
    session: &MatrixClientSession,
) -> Result<CrossSigningStatus, E2eeTrustError> {
    let status = session
        .client()
        .encryption()
        .cross_signing_status()
        .await
        .map(|status| MatrixCrossSigningStatus {
            has_master: status.has_master,
            has_self_signing: status.has_self_signing,
            has_user_signing: status.has_user_signing,
        });
    Ok(map_cross_signing_status_to_desktop(status))
}

pub async fn bootstrap_cross_signing(
    session: &MatrixClientSession,
    auth_secret: Option<&AuthSecret>,
) -> Result<CrossSigningStatus, E2eeTrustError> {
    let encryption = session.client().encryption();
    match encryption.bootstrap_cross_signing(None).await {
        Ok(()) => {}
        Err(error) => {
            let Some(auth_secret) = auth_secret else {
                return Err(error.into());
            };
            let Some(uiaa_session) = error
                .as_uiaa_response()
                .and_then(|response| response.session.clone())
            else {
                return Err(error.into());
            };
            let identifier = matrix_sdk::ruma::api::client::uiaa::UserIdentifier::Matrix(
                matrix_sdk::ruma::api::client::uiaa::MatrixUserIdentifier::new(
                    session.info.user_id.clone(),
                ),
            );
            let mut password_auth = matrix_sdk::ruma::api::client::uiaa::Password::new(
                identifier,
                auth_secret.expose_secret().to_owned(),
            );
            password_auth.session = Some(uiaa_session);
            encryption
                .bootstrap_cross_signing(Some(
                    matrix_sdk::ruma::api::client::uiaa::AuthData::Password(password_auth),
                ))
                .await?;
        }
    }
    cross_signing_status(session).await
}

pub async fn enable_key_backup(
    session: &MatrixClientSession,
    passphrase: Option<&AuthSecret>,
) -> Result<KeyBackupStatus, E2eeTrustError> {
    let encryption = session.client().encryption();
    if let Some(passphrase) = passphrase {
        let _recovery_key = encryption
            .recovery()
            .enable()
            .wait_for_backups_to_upload()
            .with_passphrase(passphrase.expose_secret())
            .await?;
    } else {
        encryption.recovery().enable_backup().await?;
    }
    Ok(map_backup_state_to_desktop(encryption.backups().state()))
}

pub async fn restore_key_backup(
    session: &MatrixClientSession,
    request: &RecoveryRequest,
    version: Option<&str>,
) -> Result<KeyBackupRestoreSummary, E2eeTrustError> {
    let encryption = session.client().encryption();
    encryption
        .recovery()
        .recover(request.secret.expose_secret())
        .await?;

    let backup_state = encryption.backups().state();
    if !matches!(
        backup_state,
        matrix_sdk::encryption::backups::BackupState::Enabled
            | matrix_sdk::encryption::backups::BackupState::Downloading
    ) {
        return Err(E2eeTrustError::Sdk(
            "key backup unavailable after recovery".to_owned(),
        ));
    }

    let rooms = session.client().joined_rooms();
    let total_rooms = rooms.len() as u64;
    let mut restored_rooms = 0;
    for room in rooms {
        encryption
            .backups()
            .download_room_keys_for_room(room.room_id())
            .await?;
        restored_rooms += 1;
    }

    let backup_status = map_backup_state_to_desktop(encryption.backups().state());
    let backup_version = version.map(str::to_owned).or_else(|| match backup_status {
        KeyBackupStatus::Enabled { version } => Some(version),
        KeyBackupStatus::Restoring { version, .. } => version,
        _ => None,
    });

    Ok(KeyBackupRestoreSummary {
        scope: KeyBackupRestoreScope::JoinedRooms,
        version: backup_version,
        restored_rooms,
        total_rooms: Some(total_rooms),
    })
}

#[cfg(not(target_family = "wasm"))]
pub async fn export_room_keys_to_file(
    session: &MatrixClientSession,
    path: PathBuf,
    passphrase: &AuthSecret,
) -> Result<RoomKeyExportSummary, E2eeTrustError> {
    session
        .client()
        .encryption()
        .export_room_keys(path, passphrase.expose_secret(), |_| true)
        .await?;
    Ok(RoomKeyExportSummary {
        exported_sessions: None,
    })
}

#[cfg(not(target_family = "wasm"))]
pub async fn import_room_keys_from_file(
    session: &MatrixClientSession,
    path: PathBuf,
    passphrase: &AuthSecret,
) -> Result<RoomKeyImportSummary, E2eeTrustError> {
    let result = session
        .client()
        .encryption()
        .import_room_keys(path, passphrase.expose_secret())
        .await
        .map_err(|error| E2eeTrustError::Sdk(error.to_string()))?;
    Ok(RoomKeyImportSummary {
        imported_count: result.imported_count as u64,
        total_count: result.total_count as u64,
    })
}

pub async fn bootstrap_secure_backup(
    session: &MatrixClientSession,
    passphrase: Option<&AuthSecret>,
    recovery_key_destination_path: Option<PathBuf>,
) -> Result<SecureBackupSetupSummary, E2eeTrustError> {
    let recovery = session.client().encryption().recovery();
    let recovery_key = match passphrase {
        Some(passphrase) => {
            recovery
                .enable()
                .wait_for_backups_to_upload()
                .with_passphrase(passphrase.expose_secret())
                .await?
        }
        None => recovery.enable().wait_for_backups_to_upload().await?,
    };
    let recovery_key_written =
        write_recovery_key_if_requested(recovery_key, recovery_key_destination_path)?;
    Ok(SecureBackupSetupSummary {
        recovery_key_written,
    })
}

pub async fn change_secure_backup_passphrase(
    session: &MatrixClientSession,
    old_secret: &AuthSecret,
    new_passphrase: &AuthSecret,
    recovery_key_destination_path: Option<PathBuf>,
) -> Result<SecureBackupSetupSummary, E2eeTrustError> {
    let recovery_key = session
        .client()
        .encryption()
        .recovery()
        .recover_and_reset(old_secret.expose_secret())
        .with_passphrase(new_passphrase.expose_secret())
        .await?;
    let recovery_key_written =
        write_recovery_key_if_requested(recovery_key, recovery_key_destination_path)?;
    Ok(SecureBackupSetupSummary {
        recovery_key_written,
    })
}

fn write_recovery_key_if_requested(
    recovery_key: String,
    destination_path: Option<PathBuf>,
) -> Result<bool, E2eeTrustError> {
    let recovery_key = Zeroizing::new(recovery_key);
    let Some(destination_path) = destination_path else {
        return Ok(false);
    };
    std::fs::write(destination_path, recovery_key.as_bytes()).map_err(|_| {
        E2eeTrustError::Sdk("secure backup recovery key delivery failed".to_owned())
    })?;
    Ok(true)
}

pub async fn reset_identity(
    session: &MatrixClientSession,
) -> Result<IdentityResetOutcome, E2eeTrustError> {
    let outcome = session
        .client()
        .encryption()
        .recovery()
        .reset_identity()
        .await?;

    Ok(match outcome {
        Some(handle) => {
            let auth_type = map_sdk_identity_reset_auth_type(handle.auth_type());
            IdentityResetOutcome::AuthRequired(MatrixIdentityResetHandle {
                inner: handle,
                auth_type,
            })
        }
        None => IdentityResetOutcome::Completed,
    })
}

pub async fn complete_identity_reset(
    session: &MatrixClientSession,
    handle: &MatrixIdentityResetHandle,
    request: &IdentityResetAuthRequest,
) -> Result<(), E2eeTrustError> {
    handle.reset(session, request).await
}

/// Threshold after which a device is considered inactive (90 days).
const INACTIVE_DEVICE_THRESHOLD_DAYS: u64 = 90;

pub async fn list_devices(
    session: &MatrixClientSession,
) -> Result<Vec<MatrixDeviceSessionSummary>, E2eeTrustError> {
    let response = session
        .client()
        .devices()
        .await
        .map_err(|error| E2eeTrustError::Sdk(error.to_string()))?;
    let user_id = match matrix_sdk::ruma::OwnedUserId::try_from(session.info.user_id.as_str()) {
        Ok(user_id) => user_id,
        Err(_) => {
            return Err(E2eeTrustError::Sdk("invalid session user id".to_owned()));
        }
    };
    let now_millis = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default();
    let inactive_threshold_millis = INACTIVE_DEVICE_THRESHOLD_DAYS * 24 * 60 * 60 * 1000;

    let mut summaries = Vec::with_capacity(response.devices.len());
    for device in response.devices {
        let device_id = device.device_id.clone();
        let is_current = device_id.as_str() == session.info.device_id;
        let verified = if is_current {
            // The current device is implicitly verified from the session's
            // own perspective; the crypto store lookup is still safe but
            // avoids extra work.
            true
        } else {
            match session
                .client()
                .encryption()
                .get_device(&user_id, &device_id)
                .await
            {
                Ok(Some(crypto_device)) => crypto_device.is_verified(),
                Ok(None) | Err(_) => false,
            }
        };
        let inactive = device
            .last_seen_ts
            .map(|timestamp| i64::from(timestamp.0) as u64)
            .is_some_and(|timestamp_millis| {
                now_millis.saturating_sub(timestamp_millis) > inactive_threshold_millis
            });

        summaries.push(MatrixDeviceSessionSummary {
            current: is_current,
            raw_device_id: device_id.to_string(),
            display_name: device.display_name,
            verified,
            inactive,
        });
    }
    Ok(summaries)
}

pub async fn rename_device(
    session: &MatrixClientSession,
    raw_device_id: &str,
    display_name: &str,
) -> Result<(), E2eeTrustError> {
    let device_id = matrix_sdk::ruma::OwnedDeviceId::from(raw_device_id);
    session
        .client()
        .rename_device(&device_id, display_name)
        .await
        .map_err(|error| E2eeTrustError::Sdk(error.to_string()))?;
    Ok(())
}

pub async fn delete_devices(
    session: &MatrixClientSession,
    raw_device_ids: &[String],
    auth: Option<&IdentityResetAuthRequest>,
    uiaa_session: Option<&str>,
) -> Result<(), DeleteDevicesError> {
    let device_ids = raw_device_ids
        .iter()
        .map(|id| matrix_sdk::ruma::OwnedDeviceId::from(id.as_str()))
        .collect::<Vec<_>>();
    let auth_data = delete_devices_auth_data(session, auth, uiaa_session);
    match session
        .client()
        .delete_devices(&device_ids, auth_data)
        .await
    {
        Ok(_) => Ok(()),
        Err(error) => {
            if let Some(uiaa) = error.as_uiaa_response() {
                Err(DeleteDevicesError::UiaaChallenge {
                    session: uiaa.session.clone(),
                })
            } else {
                Err(DeleteDevicesError::Sdk(error.to_string()))
            }
        }
    }
}

fn delete_devices_auth_data(
    session: &MatrixClientSession,
    auth: Option<&IdentityResetAuthRequest>,
    uiaa_session: Option<&str>,
) -> Option<matrix_sdk::ruma::api::client::uiaa::AuthData> {
    let IdentityResetAuthRequest::UiaaPassword { password } = auth? else {
        return None;
    };
    let identifier = matrix_sdk::ruma::api::client::uiaa::UserIdentifier::Matrix(
        matrix_sdk::ruma::api::client::uiaa::MatrixUserIdentifier::new(
            session.info.user_id.clone(),
        ),
    );
    let mut password_auth = matrix_sdk::ruma::api::client::uiaa::Password::new(
        identifier,
        password.expose_secret().to_owned(),
    );
    password_auth.session = uiaa_session.map(str::to_owned);
    Some(matrix_sdk::ruma::api::client::uiaa::AuthData::Password(
        password_auth,
    ))
}

pub async fn request_device_verification(
    session: &MatrixClientSession,
    target: &VerificationTarget,
) -> Result<MatrixVerificationRequestHandle, E2eeTrustError> {
    let user_id = matrix_sdk::ruma::OwnedUserId::try_from(target.user_id.as_str())
        .map_err(|_| E2eeTrustError::Sdk("invalid verification user id".to_owned()))?;
    let device_id = matrix_sdk::ruma::OwnedDeviceId::from(target.device_id.as_str());
    let device = session
        .client()
        .encryption()
        .get_device(&user_id, &device_id)
        .await
        .map_err(|error| E2eeTrustError::Sdk(error.to_string()))?
        .ok_or_else(|| E2eeTrustError::Sdk("verification target device not found".to_owned()))?;

    let inner = device
        .request_verification_with_methods(vec![
            matrix_sdk::ruma::events::key::verification::VerificationMethod::SasV1,
        ])
        .await?;
    Ok(MatrixVerificationRequestHandle { inner })
}

pub fn observe_incoming_verification_requests(
    session: &MatrixClientSession,
) -> MatrixIncomingVerificationRequestObserver {
    let client = session.client();
    let (sender, receiver) = tokio::sync::mpsc::channel(32);

    let to_device_client = client.clone();
    let to_device_sender = sender.clone();
    let to_device_handler = client.add_event_handler(
        move |event: matrix_sdk::ruma::events::key::verification::request::ToDeviceKeyVerificationRequestEvent| {
            let client = to_device_client.clone();
            let sender = to_device_sender.clone();
            async move {
                if let Some(request) =
                    incoming_verification_request_for_flow(&client, &event.sender, event.content.transaction_id.as_str()).await
                {
                    let _ = sender.send(request).await;
                }
            }
        },
    );

    let room_client = client.clone();
    let room_sender = sender;
    let room_handler = client.add_event_handler(
        move |event: matrix_sdk::ruma::events::room::message::OriginalSyncRoomMessageEvent| {
            let client = room_client.clone();
            let sender = room_sender.clone();
            async move {
                if !matches!(
                    &event.content.msgtype,
                    matrix_sdk::ruma::events::room::message::MessageType::VerificationRequest(_)
                ) {
                    return;
                }
                if let Some(request) = incoming_verification_request_for_flow(
                    &client,
                    &event.sender,
                    event.event_id.as_str(),
                )
                .await
                {
                    let _ = sender.send(request).await;
                }
            }
        },
    );

    MatrixIncomingVerificationRequestObserver {
        client,
        receiver,
        handlers: vec![to_device_handler, room_handler],
    }
}

async fn incoming_verification_request_for_flow(
    client: &matrix_sdk::Client,
    sender: &matrix_sdk::ruma::UserId,
    flow_id: &str,
) -> Option<MatrixIncomingVerificationRequest> {
    let request = client
        .encryption()
        .get_verification_request(sender, flow_id)
        .await?;
    let matrix_sdk::encryption::verification::VerificationRequestState::Requested {
        other_device_data,
        ..
    } = request.state()
    else {
        return None;
    };

    Some(MatrixIncomingVerificationRequest {
        target: VerificationTarget {
            user_id: sender.to_string(),
            device_id: other_device_data.device_id().to_string(),
        },
        handle: MatrixVerificationRequestHandle { inner: request },
    })
}

pub async fn accept_verification_request(
    handle: &MatrixVerificationRequestHandle,
) -> Result<(), E2eeTrustError> {
    handle
        .inner
        .accept_with_methods(vec![
            matrix_sdk::ruma::events::key::verification::VerificationMethod::SasV1,
        ])
        .await?;
    Ok(())
}

pub async fn start_sas_verification(
    handle: &MatrixVerificationRequestHandle,
) -> Result<Option<MatrixSasVerificationHandle>, E2eeTrustError> {
    Ok(handle
        .inner
        .start_sas()
        .await?
        .map(|inner| MatrixSasVerificationHandle { inner }))
}

pub async fn accept_sas_verification(
    handle: &MatrixSasVerificationHandle,
) -> Result<(), E2eeTrustError> {
    handle.inner.accept().await?;
    Ok(())
}

pub async fn confirm_sas_verification(
    handle: &MatrixSasVerificationHandle,
) -> Result<(), E2eeTrustError> {
    handle.inner.confirm().await?;
    Ok(())
}

pub async fn mismatch_sas_verification(
    handle: &MatrixSasVerificationHandle,
) -> Result<(), E2eeTrustError> {
    handle.inner.mismatch().await?;
    Ok(())
}

pub async fn cancel_verification_request(
    handle: &MatrixVerificationRequestHandle,
) -> Result<(), E2eeTrustError> {
    handle.inner.cancel().await?;
    Ok(())
}

pub async fn cancel_sas_verification(
    handle: &MatrixSasVerificationHandle,
) -> Result<(), E2eeTrustError> {
    handle.inner.cancel().await?;
    Ok(())
}

pub fn map_backup_state_to_desktop(
    state: matrix_sdk::encryption::backups::BackupState,
) -> KeyBackupStatus {
    use matrix_sdk::encryption::backups::BackupState;

    match state {
        BackupState::Unknown => KeyBackupStatus::Unknown,
        BackupState::Creating | BackupState::Enabling | BackupState::Resuming => {
            KeyBackupStatus::Enabling { request_id: 0 }
        }
        BackupState::Enabled => KeyBackupStatus::Enabled {
            version: "available".to_owned(),
        },
        BackupState::Downloading => KeyBackupStatus::Restoring {
            request_id: 0,
            version: None,
            restored_rooms: 0,
            total_rooms: None,
        },
        BackupState::Disabling => KeyBackupStatus::Disabled,
    }
}

#[cfg(test)]
mod e2ee_trust_tests {
    use matrix_desktop_state::{
        AuthSecret, CrossSigningStatus, IdentityResetAuthType, KeyBackupStatus, SasEmoji,
    };
    use matrix_sdk::encryption::backups::BackupState;

    use super::{
        E2eeTrustError, KeyBackupRestoreScope, KeyBackupRestoreSummary, MatrixCrossSigningStatus,
        MatrixDeviceSessionSummary, MatrixIdentityResetAuthType, MatrixIncomingVerificationRequest,
        MatrixIncomingVerificationRequestObserver, PersistableMatrixSession, RoomKeyExportSummary,
        RoomKeyImportSummary, SecureBackupSetupSummary, accept_sas_verification,
        accept_verification_request, bootstrap_cross_signing, bootstrap_secure_backup,
        cancel_sas_verification, cancel_verification_request, change_secure_backup_passphrase,
        complete_identity_reset, confirm_sas_verification, cross_signing_status, delete_devices,
        enable_key_backup, export_room_keys_to_file, import_room_keys_from_file, list_devices,
        map_backup_state_to_desktop, map_cross_signing_status_to_desktop,
        map_identity_reset_auth_type_to_desktop, map_sdk_sas_emojis_to_desktop,
        mismatch_sas_verification, observe_incoming_verification_requests, rename_device,
        request_device_verification, reset_identity, restore_key_backup, restore_session,
        start_sas_verification, write_recovery_key_if_requested,
    };

    const MATRIX_KEY_EXPORT_HEADER: &str = "-----BEGIN MEGOLM SESSION DATA-----";
    const MATRIX_KEY_EXPORT_FOOTER: &str = "-----END MEGOLM SESSION DATA-----";
    const ELEMENT_COMPATIBLE_KEY_EXPORT: &str = "\
-----BEGIN MEGOLM SESSION DATA-----\n\
Af7mGhlzQ+eGvHu93u0YXd3D/+vYMs3E7gQqOhuCtkvGAAAAASH7pEdWvFyAP1JUisAcpEo\n\
Xke2Q7Kr9hVl/SCc6jXBNeJCZcrUbUV4D/tRQIl3E9L4fOk928YI1J+3z96qiH0uE7hpsCI\n\
CkHKwjPU+0XTzFdIk1X8H7sZ+MD/2Sg/q3y8rtUjz7uEj4GUTnb+9SCOTVmJsRfqgUpM1CU\n\
bDLytHf1JkohY4tWEgpsCc67xdzgodjr12qYrfg/zNm3LGpxlrffJknw4rk5QFTj4kMbqbD\n\
ZZgDTni+HxRTDGge2J620lMOiznvXX+H09Rwruqx5aJvvaaKd86jWRpiO2oSFqHn4u5ONl9\n\
41uzm62Sj0eIm6ZbA9NQs87jQw4LxsejhZVL+NdjIg80zVSBTWhTdo0DTnbFSNP4ReOiz0U\n\
XosOF8A5T8Vdx2nvA0GXltfcHKVKQYh/LJAkNQ7P9UYL4ae/5TtQZkhB1KxCLTRWqADCl53\n\
uBMGpG53EMgY6G6K2DEIOkcv7sdXQF5WpemiSWZqJRWj+cjfs9BpCTbkp/rszWFl2TniWpR\n\
RqIbT2jORlN4rTvdtF0F4z1pqP4qWyR3sLNTkXm9CFRzWADNG0RDZKxbCoo6RPvtaCTfaHo\n\
SwfvzBS6CjfAG+FOugpV48o7+XetaUUPZ6/tZSPhCdeV8eP9q5r0QwWeXFogzoNzWt4HYx9\n\
MdXxzD+f0mtg5gzehrrEEARwI2bCvPpHxlt/Na9oW/GBpkjwR1LSKgg4CtpRyWngPjdEKpZ\n\
GYW19pdjg0qdXNk/eqZsQTsNWVo6A\n\
-----END MEGOLM SESSION DATA-----";

    #[test]
    fn cross_signing_status_maps_to_private_data_free_desktop_status() {
        assert_eq!(
            map_cross_signing_status_to_desktop(None),
            CrossSigningStatus::Missing
        );
        assert_eq!(
            map_cross_signing_status_to_desktop(Some(MatrixCrossSigningStatus {
                has_master: true,
                has_self_signing: true,
                has_user_signing: true,
            })),
            CrossSigningStatus::Trusted
        );
        assert_eq!(
            map_cross_signing_status_to_desktop(Some(MatrixCrossSigningStatus {
                has_master: true,
                has_self_signing: false,
                has_user_signing: true,
            })),
            CrossSigningStatus::NotTrusted
        );
    }

    #[test]
    fn key_backup_state_maps_to_private_data_free_desktop_status() {
        assert_eq!(
            map_backup_state_to_desktop(BackupState::Unknown),
            KeyBackupStatus::Unknown
        );
        assert_eq!(
            map_backup_state_to_desktop(BackupState::Enabled),
            KeyBackupStatus::Enabled {
                version: "available".to_owned(),
            }
        );
        assert_eq!(
            map_backup_state_to_desktop(BackupState::Downloading),
            KeyBackupStatus::Restoring {
                request_id: 0,
                version: None,
                restored_rooms: 0,
                total_rooms: None,
            }
        );
    }

    #[test]
    fn e2ee_trust_error_debug_redacts_sdk_details() {
        let error = E2eeTrustError::Sdk("raw matrix sdk error with @alice:example.test".to_owned());
        let debug = format!("{error:?}");

        assert!(!debug.contains("@alice:example.test"));
        assert!(!debug.contains("raw matrix sdk error"));
        assert!(debug.contains("Sdk"));
    }

    #[test]
    fn key_backup_restore_summary_declares_joined_room_scope() {
        let summary = KeyBackupRestoreSummary {
            scope: KeyBackupRestoreScope::JoinedRooms,
            version: Some("available".to_owned()),
            restored_rooms: 2,
            total_rooms: Some(3),
        };

        let debug = format!("{summary:?}");
        assert!(debug.contains("JoinedRooms"));
        assert!(!debug.contains("BackupWide"));
        assert!(!debug.contains("AllRooms"));
    }

    #[test]
    fn device_session_summary_is_private_data_free() {
        let summary = MatrixDeviceSessionSummary {
            raw_device_id: "DEVICEID".to_owned(),
            display_name: Some("Alice private laptop".to_owned()),
            current: true,
            verified: false,
            inactive: false,
        };

        assert_eq!(summary.raw_device_id, "DEVICEID");
        let debug = format!("{summary:?}");
        assert!(!debug.contains("DEVICEID"), "{debug}");
        assert!(!debug.contains("Alice private laptop"), "{debug}");
        assert!(debug.contains("current"));
        assert!(debug.contains("verified"));
        assert!(debug.contains("inactive"));
    }

    #[test]
    fn room_key_file_transfer_summaries_are_private_data_free() {
        let export_summary = RoomKeyExportSummary {
            exported_sessions: None,
        };
        let import_summary = RoomKeyImportSummary {
            imported_count: 1,
            total_count: 1,
        };

        assert_eq!(export_summary.exported_sessions, None);
        assert_eq!(import_summary.imported_count, 1);
        assert_eq!(import_summary.total_count, 1);
        assert!(!format!("{export_summary:?}").contains("MEGOLM"));
        assert!(!format!("{import_summary:?}").contains("MEGOLM"));
    }

    #[test]
    fn secure_backup_setup_summary_is_private_data_free() {
        let summary = SecureBackupSetupSummary {
            recovery_key_written: true,
        };

        let debug = format!("{summary:?}");
        assert!(debug.contains("recovery_key_written"));
        assert!(!debug.contains("RecoveryKey("));
    }

    #[test]
    fn recovery_key_delivery_writes_native_artifact_without_debugging_material() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("recovery-artifact.txt");
        let artifact_payload = String::from("fixture-artifact-material");

        let written = write_recovery_key_if_requested(artifact_payload.clone(), Some(path.clone()))
            .expect("artifact write should succeed");

        assert!(written);
        assert_eq!(
            std::fs::read_to_string(path).expect("read artifact"),
            artifact_payload
        );
    }

    #[tokio::test]
    async fn room_key_import_accepts_element_compatible_key_export_envelope() {
        assert!(ELEMENT_COMPATIBLE_KEY_EXPORT.starts_with(MATRIX_KEY_EXPORT_HEADER));
        assert!(ELEMENT_COMPATIBLE_KEY_EXPORT.ends_with(MATRIX_KEY_EXPORT_FOOTER));

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("element-compatible-room-keys.txt");
        std::fs::write(&path, ELEMENT_COMPATIBLE_KEY_EXPORT).expect("write fixture");
        let persistable = PersistableMatrixSession::from_json(
            r#"{"homeserver":"https://matrix.example.invalid","user_id":"@alice:example.invalid","device_id":"ALICEDEVICE","access_token":"synthetic-access"}"#,
        )
        .expect("synthetic session should deserialize");
        let session = restore_session(&persistable)
            .await
            .expect("synthetic session should restore");

        let summary = import_room_keys_from_file(&session, path, &AuthSecret::new("1234"))
            .await
            .expect("Matrix/Element key export envelope should import");

        assert_eq!(summary.total_count, 1);
    }

    #[test]
    fn e2ee_trust_public_async_api_is_exposed() {
        let _ = cross_signing_status;
        let _ = bootstrap_cross_signing;
        let _ = enable_key_backup;
        let _ = restore_key_backup;
        let _ = reset_identity;
        let _ = complete_identity_reset;
        let _ = request_device_verification;
        let _ = accept_verification_request;
        let _ = start_sas_verification;
        let _ = accept_sas_verification;
        let _ = confirm_sas_verification;
        let _ = mismatch_sas_verification;
        let _ = cancel_verification_request;
        let _ = cancel_sas_verification;
        let _ = observe_incoming_verification_requests;
        let _ = list_devices;
        let _ = rename_device;
        let _ = delete_devices;
        let _ = export_room_keys_to_file;
        let _ = import_room_keys_from_file;
        let _ = bootstrap_secure_backup;
        let _ = change_secure_backup_passphrase;
        let _: Option<MatrixIncomingVerificationRequest> = None;
        let _: Option<MatrixIncomingVerificationRequestObserver> = None;
    }

    #[test]
    fn sas_emojis_map_to_desktop_dto_without_sdk_types() {
        let emojis = [
            matrix_sdk::encryption::verification::Emoji {
                symbol: "🐶",
                description: "Dog",
            },
            matrix_sdk::encryption::verification::Emoji {
                symbol: "🐱",
                description: "Cat",
            },
            matrix_sdk::encryption::verification::Emoji {
                symbol: "🦁",
                description: "Lion",
            },
            matrix_sdk::encryption::verification::Emoji {
                symbol: "🐎",
                description: "Horse",
            },
            matrix_sdk::encryption::verification::Emoji {
                symbol: "🦄",
                description: "Unicorn",
            },
            matrix_sdk::encryption::verification::Emoji {
                symbol: "🐷",
                description: "Pig",
            },
            matrix_sdk::encryption::verification::Emoji {
                symbol: "🐘",
                description: "Elephant",
            },
        ];

        assert_eq!(
            map_sdk_sas_emojis_to_desktop(emojis),
            vec![
                SasEmoji {
                    symbol: "🐶".to_owned(),
                    description: "Dog".to_owned(),
                },
                SasEmoji {
                    symbol: "🐱".to_owned(),
                    description: "Cat".to_owned(),
                },
                SasEmoji {
                    symbol: "🦁".to_owned(),
                    description: "Lion".to_owned(),
                },
                SasEmoji {
                    symbol: "🐎".to_owned(),
                    description: "Horse".to_owned(),
                },
                SasEmoji {
                    symbol: "🦄".to_owned(),
                    description: "Unicorn".to_owned(),
                },
                SasEmoji {
                    symbol: "🐷".to_owned(),
                    description: "Pig".to_owned(),
                },
                SasEmoji {
                    symbol: "🐘".to_owned(),
                    description: "Elephant".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn identity_reset_auth_type_maps_to_private_data_free_desktop_status() {
        assert_eq!(
            map_identity_reset_auth_type_to_desktop(MatrixIdentityResetAuthType::Uiaa),
            IdentityResetAuthType::Uiaa
        );
        assert_eq!(
            map_identity_reset_auth_type_to_desktop(MatrixIdentityResetAuthType::OAuth),
            IdentityResetAuthType::OAuth
        );
    }
}

#[derive(Clone)]
pub struct MatrixClientStoreConfig {
    path: PathBuf,
    cache_path: Option<PathBuf>,
    key: MatrixClientStoreKey,
    search_index_store: Option<MatrixSearchIndexStoreConfig>,
}

impl MatrixClientStoreConfig {
    pub fn new(path: impl Into<PathBuf>, key: MatrixClientStoreKey) -> Self {
        Self {
            path: path.into(),
            cache_path: None,
            key,
            search_index_store: None,
        }
    }

    pub fn with_cache_path(mut self, cache_path: impl Into<PathBuf>) -> Self {
        self.cache_path = Some(cache_path.into());
        self
    }

    pub fn with_search_index_store(
        mut self,
        search_index_store: MatrixSearchIndexStoreConfig,
    ) -> Self {
        self.search_index_store = Some(search_index_store);
        self
    }

    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    pub fn cache_path(&self) -> Option<&Path> {
        self.cache_path.as_deref()
    }

    fn apply_to_builder(&self, builder: matrix_sdk::ClientBuilder) -> matrix_sdk::ClientBuilder {
        let sqlite_config =
            matrix_sdk::SqliteStoreConfig::new(&self.path).key(Some(self.key.expose_key()));
        let builder = builder
            .sqlite_store_with_config_and_cache_path(sqlite_config, self.cache_path.as_deref());
        match &self.search_index_store {
            Some(search_index_store) => {
                builder.search_index_store(search_index_store.as_sdk_store_kind())
            }
            None => builder,
        }
    }
}

impl fmt::Debug for MatrixClientStoreConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixClientStoreConfig")
            .field("path", &self.path)
            .field("cache_path", &self.cache_path)
            .field("key", &self.key)
            .field("search_index_store", &self.search_index_store)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct MatrixClientStoreKey {
    key: Zeroizing<[u8; 32]>,
}

impl MatrixClientStoreKey {
    pub fn new(key: [u8; 32]) -> Self {
        Self {
            key: Zeroizing::new(key),
        }
    }

    fn expose_key(&self) -> &[u8; 32] {
        &self.key
    }
}

impl fmt::Debug for MatrixClientStoreKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MatrixClientStoreKey(..)")
    }
}

#[derive(Clone)]
pub struct MatrixSearchIndexStoreConfig {
    path: PathBuf,
    key: MatrixSearchIndexKey,
}

impl MatrixSearchIndexStoreConfig {
    pub fn new(path: impl Into<PathBuf>, key: MatrixSearchIndexKey) -> Self {
        Self {
            path: path.into(),
            key,
        }
    }

    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    fn as_sdk_store_kind(&self) -> matrix_sdk::search_index::SearchIndexStoreKind {
        matrix_sdk::search_index::SearchIndexStoreKind::encrypted_directory_ngram(
            self.path.clone(),
            self.key.expose_key().to_owned(),
            2,
            4,
        )
        .expect("desktop ngram search bounds should be valid")
    }
}

impl fmt::Debug for MatrixSearchIndexStoreConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixSearchIndexStoreConfig")
            .field("path", &self.path)
            .field("key", &self.key)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct MatrixSearchIndexKey {
    key: Zeroizing<String>,
}

impl MatrixSearchIndexKey {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: Zeroizing::new(key.into()),
        }
    }

    fn expose_key(&self) -> &str {
        self.key.as_str()
    }
}

impl fmt::Debug for MatrixSearchIndexKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MatrixSearchIndexKey(..)")
    }
}

#[derive(Clone)]
pub struct MatrixClientSession {
    client: matrix_sdk::Client,
    pub info: SessionInfo,
}

impl MatrixClientSession {
    pub fn client(&self) -> matrix_sdk::Client {
        self.client.clone()
    }

    pub fn persistable_session(&self) -> Result<PersistableMatrixSession, PasswordLoginError> {
        let session = self
            .client
            .matrix_auth()
            .session()
            .ok_or(PasswordLoginError::MissingSession)?;
        Ok(PersistableMatrixSession {
            info: self.info.clone(),
            session,
        })
    }

    pub fn e2ee_recovery_state(&self) -> E2eeRecoveryState {
        map_sdk_recovery_state(self.client().encryption().recovery().state())
    }

    pub fn e2ee_recovery_state_stream(&self) -> E2eeRecoveryStateStream {
        Box::pin(
            self.client()
                .encryption()
                .recovery()
                .state_stream()
                .map(map_sdk_recovery_state),
        )
    }
}

impl std::fmt::Debug for MatrixClientSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MatrixClientSession")
            .field("info", &self.info)
            .field("client", &"MatrixClient(..)")
            .finish()
    }
}

#[derive(Clone)]
pub struct PersistableMatrixSession {
    pub info: SessionInfo,
    session: MatrixSession,
}

impl PersistableMatrixSession {
    pub fn to_json(&self) -> Result<String, PasswordLoginError> {
        serde_json::to_string(&SerializedPersistableMatrixSession {
            homeserver: self.info.homeserver.clone(),
            session: self.session.clone(),
        })
        .map_err(|error| PasswordLoginError::Serialization(error.to_string()))
    }

    pub fn from_json(value: &str) -> Result<Self, PasswordLoginError> {
        let serialized = serde_json::from_str::<SerializedPersistableMatrixSession>(value)
            .map_err(|error| PasswordLoginError::Serialization(error.to_string()))?;
        let session = serialized.session;
        let info = SessionInfo {
            homeserver: serialized.homeserver,
            user_id: session.meta.user_id.to_string(),
            device_id: session.meta.device_id.to_string(),
        };
        Ok(Self { info, session })
    }

    pub fn matrix_session(&self) -> MatrixSession {
        self.session.clone()
    }
}

#[derive(Deserialize, Serialize)]
struct SerializedPersistableMatrixSession {
    homeserver: String,
    #[serde(flatten)]
    session: MatrixSession,
}

impl std::fmt::Debug for PersistableMatrixSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PersistableMatrixSession")
            .field("info", &self.info)
            .field("session", &"MatrixSession(..)")
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct Homeserver {
    base_url: Url,
}

impl Homeserver {
    pub fn parse(input: &str) -> Result<Self, LoginDiscoveryError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(LoginDiscoveryError::InvalidHomeserver(
                "homeserver is empty".to_owned(),
            ));
        }

        let candidate = if trimmed.contains("://") {
            trimmed.to_owned()
        } else {
            format!("https://{trimmed}")
        };
        let mut base_url = Url::parse(&candidate)
            .map_err(|error| LoginDiscoveryError::InvalidHomeserver(error.to_string()))?;

        if !matches!(base_url.scheme(), "http" | "https") {
            return Err(LoginDiscoveryError::UnsupportedHomeserverScheme);
        }
        if base_url.host_str().is_none() {
            return Err(LoginDiscoveryError::InvalidHomeserver(
                "homeserver URL is missing a host".to_owned(),
            ));
        }
        if base_url.scheme() == "http" && !is_loopback_homeserver(&base_url) {
            return Err(LoginDiscoveryError::InsecureHomeserverScheme);
        }
        if base_url.query().is_some() || base_url.fragment().is_some() {
            return Err(LoginDiscoveryError::InvalidHomeserver(
                "homeserver URL must not include query or fragment".to_owned(),
            ));
        }

        if !base_url.path().ends_with('/') {
            let mut path = base_url.path().to_owned();
            path.push('/');
            base_url.set_path(&path);
        }

        Ok(Self { base_url })
    }

    pub fn normalized(&self) -> String {
        let mut normalized = self.base_url.to_string();
        if normalized.ends_with('/') {
            normalized.pop();
        }
        normalized
    }

    pub fn login_discovery_url(&self) -> Url {
        self.base_url
            .join(LOGIN_DISCOVERY_PATH)
            .expect("login discovery path should be relative")
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum LoginDiscoveryError {
    #[error("homeserver URL is invalid: {0}")]
    InvalidHomeserver(String),
    #[error("homeserver URL scheme must be http or https")]
    UnsupportedHomeserverScheme,
    #[error("homeserver URL must use https unless it is localhost or loopback")]
    InsecureHomeserverScheme,
    #[error("login discovery request failed: {0}")]
    RequestFailed(String),
    #[error("login discovery failed with HTTP {status}: {message}")]
    HttpStatus { status: u16, message: String },
    #[error("login discovery response is missing flows")]
    MissingFlows,
    #[error("login discovery response is invalid: {0}")]
    InvalidResponse(String),
}

#[derive(Debug, Error)]
pub enum PasswordLoginError {
    #[error(transparent)]
    InvalidHomeserver(#[from] LoginDiscoveryError),
    #[error("password login runtime failed: {0}")]
    Runtime(String),
    #[error("password login failed: {0}")]
    Sdk(String),
    #[error("SDK session is not available")]
    MissingSession,
    #[error("session serialization failed: {0}")]
    Serialization(String),
}

#[derive(Debug, Error)]
pub enum E2eeRecoveryError {
    #[error("E2EE recovery runtime failed: {0}")]
    Runtime(String),
    #[error("E2EE recovery failed: {0}")]
    Sdk(String),
}

#[derive(Debug, Error)]
pub enum MatrixSyncError {
    #[error("Matrix sync failed")]
    Sdk,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MatrixRoomOperationError {
    #[error("Matrix room id is invalid")]
    InvalidRoomId,
    #[error("Matrix room alias is invalid")]
    InvalidRoomAlias,
    #[error("Matrix room setting is invalid")]
    InvalidRoomSetting,
    #[error("Matrix event id is invalid")]
    InvalidEventId,
    #[error("Matrix user id is invalid")]
    InvalidUserId,
    #[error("Matrix server name is invalid")]
    InvalidServerName,
    #[error("Matrix room is not available")]
    RoomUnavailable,
    #[error("Matrix room operation failed: {0}")]
    Sdk(MatrixRoomOperationFailureKind),
}

impl MatrixRoomOperationError {
    pub fn failure_kind(&self) -> Option<MatrixRoomOperationFailureKind> {
        match self {
            Self::Sdk(kind) => Some(*kind),
            Self::InvalidRoomId
            | Self::InvalidRoomAlias
            | Self::InvalidRoomSetting
            | Self::InvalidEventId
            | Self::InvalidUserId
            | Self::InvalidServerName
            | Self::RoomUnavailable => None,
        }
    }

    fn from_sdk_error(error: matrix_sdk::Error) -> Self {
        if std::env::var_os("MATRIX_DESKTOP_DEBUG_SDK_ERROR").is_some() {
            eprintln!("raw matrix room operation error: {error:?}");
        }
        Self::Sdk(matrix_room_operation_failure_kind(&error))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixRoomOperationFailureKind {
    AuthenticationRequired,
    Encryption,
    Forbidden,
    Http,
    Store,
    WrongRoomState,
    Sdk,
}

impl fmt::Display for MatrixRoomOperationFailureKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::AuthenticationRequired => "authentication_required",
            Self::Encryption => "encryption",
            Self::Forbidden => "forbidden",
            Self::Http => "http",
            Self::Store => "store",
            Self::WrongRoomState => "wrong_room_state",
            Self::Sdk => "sdk",
        };
        formatter.write_str(label)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixSearchCandidate {
    pub room_id: String,
    pub event_id: String,
    pub score_millis: u32,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MatrixSearchError {
    #[error("Matrix search failed")]
    Sdk,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MatrixRoomListSnapshot {
    pub spaces: Vec<MatrixRoomListSpace>,
    pub rooms: Vec<MatrixRoomListRoom>,
    pub invites: Vec<MatrixInvitePreview>,
    pub user_profiles: Vec<MatrixUserProfile>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RoomListSmokeReport {
    pub rooms: usize,
    pub spaces: usize,
    pub dms: usize,
    pub unread_rooms: usize,
}

impl std::fmt::Display for RoomListSmokeReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "rooms={} spaces={} dms={} unread_rooms={}",
            self.rooms, self.spaces, self.dms, self.unread_rooms
        )
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TimelineSmokeReport {
    pub selected_room_present: bool,
    pub timeline_items: usize,
}

impl std::fmt::Display for TimelineSmokeReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "selected_room_present={} timeline_items={}",
            self.selected_room_present, self.timeline_items
        )
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SearchSmokeReport {
    pub invoked: bool,
    pub candidates: usize,
}

impl std::fmt::Display for SearchSmokeReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "search_invoked={} search_candidates={}",
            self.invoked, self.candidates
        )
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RealAccountQaReport {
    pub room_list: RoomListSmokeReport,
    pub timeline: TimelineSmokeReport,
    pub session_restored: bool,
    pub search: SearchSmokeReport,
}

impl std::fmt::Display for RealAccountQaReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} {} session_restored={} {}",
            self.room_list, self.timeline, self.session_restored, self.search
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixRoomListSpace {
    pub space_id: String,
    pub display_name: String,
    pub avatar_mxc_uri: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixRoomListRoom {
    pub room_id: String,
    pub display_name: String,
    pub avatar_mxc_uri: Option<String>,
    pub is_dm: bool,
    pub dm_user_ids: Vec<String>,
    pub tags: MatrixRoomTags,
    pub unread_count: u64,
    pub notification_count: u64,
    pub highlight_count: u64,
    pub marked_unread: bool,
    pub last_activity_ms: u64,
    pub parent_space_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MatrixRoomTags {
    pub favourite: Option<MatrixRoomTagInfo>,
    pub low_priority: Option<MatrixRoomTagInfo>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixRoomTagInfo {
    pub order: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixRoomTagKind {
    Favourite,
    LowPriority,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixInvitePreview {
    pub room_id: String,
    pub display_name: String,
    pub avatar_mxc_uri: Option<String>,
    pub topic: Option<String>,
    pub inviter_display_name: Option<String>,
    pub is_dm: bool,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MatrixRoomListError {
    #[error("Matrix room list failed")]
    Sdk,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixOwnProfile {
    pub display_name: Option<String>,
    pub avatar_mxc_uri: Option<String>,
}

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct MatrixLocalUserAliases {
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
}

impl fmt::Debug for MatrixLocalUserAliases {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixLocalUserAliases")
            .field("alias_count", &self.aliases.len())
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixUserProfile {
    pub user_id: String,
    pub display_name: Option<String>,
    pub avatar_mxc_uri: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixPublicRoomDirectoryQuery {
    pub term: Option<String>,
    pub server_name: Option<String>,
    pub limit: Option<u32>,
    pub since: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixPublicRoomDirectoryResult {
    pub rooms: Vec<MatrixPublicRoomDirectoryRoom>,
    pub next_batch: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixPublicRoomDirectoryRoom {
    pub room_id: String,
    pub canonical_alias: Option<String>,
    pub name: String,
    pub topic: Option<String>,
    pub avatar_url: Option<String>,
    pub joined_members: u64,
    pub world_readable: bool,
    pub guest_can_join: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixRoomSettingsSnapshot {
    pub room_id: String,
    pub name: Option<String>,
    pub topic: Option<String>,
    pub avatar_url: Option<String>,
    pub join_rule: MatrixRoomJoinRule,
    pub history_visibility: MatrixRoomHistoryVisibility,
    pub permissions: MatrixRoomPermissionFacts,
    pub members: Vec<MatrixRoomMemberSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixRoomMemberSummary {
    pub user_id: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub power_level: Option<i64>,
    pub role: MatrixRoomMemberRole,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixRoomMemberRole {
    Creator,
    Administrator,
    Moderator,
    User,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixRoomJoinRule {
    Public,
    Invite,
    Knock,
    Restricted,
    Private,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixRoomHistoryVisibility {
    WorldReadable,
    Shared,
    Invited,
    Joined,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MatrixRoomPermissionFacts {
    pub can_edit_settings: bool,
    pub can_edit_roles: bool,
    pub can_kick: bool,
    pub can_ban: bool,
    pub can_unban: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MatrixRoomSettingChange {
    Name(Option<String>),
    Topic(Option<String>),
    AvatarUrl(Option<String>),
    JoinRule(MatrixRoomJoinRule),
    HistoryVisibility(MatrixRoomHistoryVisibility),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixRoomModerationAction {
    Kick,
    Ban,
    Unban,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MatrixProfileError {
    #[error("Matrix profile mime type is invalid")]
    InvalidMimeType,
    #[error("Matrix profile operation failed")]
    Sdk(MatrixProfileFailureKind),
}

impl MatrixProfileError {
    pub fn failure_kind(&self) -> MatrixProfileFailureKind {
        match self {
            Self::InvalidMimeType => MatrixProfileFailureKind::InvalidMimeType,
            Self::Sdk(kind) => *kind,
        }
    }

    fn from_sdk_error(error: matrix_sdk::Error) -> Self {
        if std::env::var_os("MATRIX_DESKTOP_DEBUG_SDK_ERROR").is_some() {
            eprintln!("raw matrix profile operation error: {error:?}");
        }
        Self::Sdk(matrix_profile_failure_kind(&error))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixProfileFailureKind {
    Forbidden,
    Network,
    InvalidMimeType,
    Sdk,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixTimelineItem {
    pub room_id: String,
    pub event_id: String,
    pub sender: String,
    pub timestamp_ms: u64,
    pub body: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MatrixTimelineUpdate {
    Upsert(MatrixTimelineItem),
    Remove { room_id: String, event_id: String },
}

pub type MatrixTimelineUpdateStream = Pin<Box<dyn Stream<Item = Vec<MatrixTimelineUpdate>> + Send>>;

pub struct MatrixTimelineSubscription {
    timeline: Arc<matrix_sdk_ui::Timeline>,
    initial_items: Vec<MatrixTimelineItem>,
    updates: MatrixTimelineUpdateStream,
}

#[derive(Clone)]
pub struct MatrixTimelinePaginationHandle {
    timeline: Arc<matrix_sdk_ui::Timeline>,
}

impl MatrixTimelinePaginationHandle {
    pub async fn paginate_backwards(&self, event_count: u16) -> Result<bool, MatrixTimelineError> {
        self.timeline
            .paginate_backwards(event_count)
            .await
            .map_err(|_| MatrixTimelineError::Sdk)
    }
}

impl fmt::Debug for MatrixTimelinePaginationHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixTimelinePaginationHandle")
            .field("timeline", &"Timeline(..)")
            .finish()
    }
}

impl MatrixTimelineSubscription {
    pub fn initial_items(&self) -> &[MatrixTimelineItem] {
        &self.initial_items
    }

    pub async fn next_update(&mut self) -> Option<Vec<MatrixTimelineUpdate>> {
        self.updates.next().await
    }

    pub fn pagination_handle(&self) -> MatrixTimelinePaginationHandle {
        MatrixTimelinePaginationHandle {
            timeline: self.timeline.clone(),
        }
    }

    pub async fn paginate_backwards(&self, event_count: u16) -> Result<bool, MatrixTimelineError> {
        self.timeline
            .paginate_backwards(event_count)
            .await
            .map_err(|_| MatrixTimelineError::Sdk)
    }
}

impl fmt::Debug for MatrixTimelineSubscription {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixTimelineSubscription")
            .field("initial_item_count", &self.initial_items.len())
            .field("timeline", &"Timeline(..)")
            .field("updates", &"TimelineUpdateStream(..)")
            .finish()
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MatrixTimelineError {
    #[error("Matrix room id is invalid")]
    InvalidRoomId,
    #[error("Matrix room is not available")]
    RoomUnavailable,
    #[error("Matrix timeline operation failed")]
    Sdk,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixSyncLoopControl {
    Continue,
    Stop,
}

#[derive(Deserialize)]
struct LoginDiscoveryResponse {
    flows: Vec<RawLoginFlow>,
}

#[derive(Deserialize)]
struct RawLoginFlow {
    #[serde(rename = "type")]
    flow_type: String,
    #[serde(default, rename = "org.matrix.msc3824.delegated_oidc_compatibility")]
    delegated_oidc_compatibility: bool,
    #[serde(default)]
    display_name: Option<String>,
}

#[derive(Deserialize)]
struct MatrixErrorResponse {
    error: Option<String>,
}

pub fn discover_login_flows(homeserver: &str) -> Result<LoginDiscovery, LoginDiscoveryError> {
    let homeserver = Homeserver::parse(homeserver)?;
    let response = reqwest::blocking::Client::builder()
        .timeout(DISCOVERY_TIMEOUT)
        .user_agent("matrix-desktop-prelogin/0.1")
        .build()
        .map_err(|error| LoginDiscoveryError::RequestFailed(error.to_string()))?
        .get(homeserver.login_discovery_url())
        .send()
        .map_err(|error| LoginDiscoveryError::RequestFailed(error.to_string()))?;

    let status = response.status().as_u16();
    let body = response
        .text()
        .map_err(|error| LoginDiscoveryError::RequestFailed(error.to_string()))?;
    let flows = parse_login_discovery_http_response(status, &body)?;

    Ok(LoginDiscovery {
        homeserver: homeserver.normalized(),
        flows,
        delegated: DelegatedAuthLinks::default(),
    })
}

pub fn login_with_password_blocking(
    request: &LoginRequest,
) -> Result<MatrixClientSession, PasswordLoginError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| PasswordLoginError::Runtime(error.to_string()))?;

    runtime.block_on(login_with_password(request))
}

pub fn logout_blocking(session: &MatrixClientSession) -> Result<(), PasswordLoginError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| PasswordLoginError::Runtime(error.to_string()))?;

    runtime.block_on(logout(session))
}

pub fn sync_once_blocking(session: &MatrixClientSession) -> Result<(), MatrixSyncError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|_| MatrixSyncError::Sdk)?;

    runtime.block_on(sync_once(session))
}

pub fn room_list_snapshot_blocking(
    session: &MatrixClientSession,
) -> Result<MatrixRoomListSnapshot, MatrixRoomListError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|_| MatrixRoomListError::Sdk)?;

    runtime.block_on(room_list_snapshot(session))
}

pub fn subscribe_room_timeline_blocking(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<MatrixTimelineSubscription, MatrixTimelineError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|_| MatrixTimelineError::Sdk)?;

    runtime.block_on(subscribe_room_timeline(session, room_id))
}

pub fn room_timeline_visible_items_blocking(
    session: &MatrixClientSession,
    room_id: &str,
    backfill_event_count: u16,
) -> Result<Vec<MatrixTimelineItem>, MatrixTimelineError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|_| MatrixTimelineError::Sdk)?;

    runtime.block_on(room_timeline_visible_items(
        session,
        room_id,
        backfill_event_count,
    ))
}

pub fn search_message_candidates_blocking(
    session: &MatrixClientSession,
    query: &str,
    limit: usize,
) -> Result<Vec<MatrixSearchCandidate>, MatrixSearchError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|_| MatrixSearchError::Sdk)?;

    runtime.block_on(search_message_candidates(session, query, limit))
}

pub fn restore_session_blocking(
    session: &PersistableMatrixSession,
) -> Result<MatrixClientSession, PasswordLoginError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| PasswordLoginError::Runtime(error.to_string()))?;

    runtime.block_on(restore_session(session))
}

pub fn recover_e2ee_blocking(
    session: &MatrixClientSession,
    request: &RecoveryRequest,
) -> Result<(), E2eeRecoveryError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| E2eeRecoveryError::Runtime(error.to_string()))?;

    runtime.block_on(recover_e2ee(session, request))
}

pub async fn login_with_password(
    request: &LoginRequest,
) -> Result<MatrixClientSession, PasswordLoginError> {
    login_with_password_with_store(request, None).await
}

pub async fn login_with_password_with_store(
    request: &LoginRequest,
    store_config: Option<&MatrixClientStoreConfig>,
) -> Result<MatrixClientSession, PasswordLoginError> {
    let homeserver = Homeserver::parse(&request.homeserver)?;
    let client = build_client(&homeserver, store_config).await?;

    let mut login = client
        .matrix_auth()
        .login_username(&request.username, request.password.expose_secret());
    if let Some(device_display_name) = request.device_display_name.as_deref() {
        login = login.initial_device_display_name(device_display_name);
    }

    let response = login
        .send()
        .await
        .map_err(|error| PasswordLoginError::Sdk(error.to_string()))?;
    let user_id = response.user_id.to_string();
    let device_id = response.device_id.to_string();

    Ok(MatrixClientSession {
        client,
        info: SessionInfo {
            homeserver: homeserver.normalized(),
            user_id,
            device_id,
        },
    })
}

/// Re-authenticate an existing soft-logged-out session with the same device id.
/// The returned storeless session must be persisted and restored into the
/// existing per-account store by the caller so crypto/cached data is preserved.
pub async fn login_with_existing_device(
    homeserver: &str,
    user_id: &str,
    device_id: &str,
    password: &AuthSecret,
) -> Result<MatrixClientSession, PasswordLoginError> {
    let homeserver = Homeserver::parse(homeserver)?;
    let client = build_client(&homeserver, None).await?;

    let response = client
        .matrix_auth()
        .login_username(user_id, password.expose_secret())
        .device_id(device_id)
        .send()
        .await
        .map_err(|error| PasswordLoginError::Sdk(error.to_string()))?;

    Ok(MatrixClientSession {
        client,
        info: SessionInfo {
            homeserver: homeserver.normalized(),
            user_id: response.user_id.to_string(),
            device_id: response.device_id.to_string(),
        },
    })
}

pub async fn restore_session(
    session: &PersistableMatrixSession,
) -> Result<MatrixClientSession, PasswordLoginError> {
    restore_session_with_store(session, None).await
}

pub async fn restore_session_with_store(
    session: &PersistableMatrixSession,
    store_config: Option<&MatrixClientStoreConfig>,
) -> Result<MatrixClientSession, PasswordLoginError> {
    let homeserver = Homeserver::parse(&session.info.homeserver)?;
    let client = build_client(&homeserver, store_config).await?;

    client
        .restore_session(session.matrix_session())
        .await
        .map_err(|error| PasswordLoginError::Sdk(error.to_string()))?;

    Ok(MatrixClientSession {
        client,
        info: session.info.clone(),
    })
}

async fn build_client(
    homeserver: &Homeserver,
    store_config: Option<&MatrixClientStoreConfig>,
) -> Result<matrix_sdk::Client, PasswordLoginError> {
    let builder = desktop_client_builder_defaults(matrix_sdk::Client::builder())
        .homeserver_url(homeserver.normalized());
    let builder = match store_config {
        Some(store_config) => store_config.apply_to_builder(builder),
        None => builder,
    };
    builder
        .build()
        .await
        .map_err(|error| PasswordLoginError::Sdk(error.to_string()))
}

fn desktop_client_builder_defaults(
    builder: matrix_sdk::ClientBuilder,
) -> matrix_sdk::ClientBuilder {
    builder.with_threading_support(matrix_sdk::ThreadingSupport::Enabled {
        with_subscriptions: true,
    })
}

pub async fn recover_e2ee(
    session: &MatrixClientSession,
    request: &RecoveryRequest,
) -> Result<(), E2eeRecoveryError> {
    session
        .client()
        .encryption()
        .recovery()
        .recover(request.secret.expose_secret())
        .await
        .map_err(|error| E2eeRecoveryError::Sdk(error.to_string()))
}

pub async fn logout(session: &MatrixClientSession) -> Result<(), PasswordLoginError> {
    session
        .client()
        .logout()
        .await
        .map_err(|error| PasswordLoginError::Sdk(error.to_string()))
}

pub async fn get_own_profile(
    session: &MatrixClientSession,
) -> Result<MatrixOwnProfile, MatrixProfileError> {
    matrix_own_profile_from_session(session).await
}

pub async fn set_display_name(
    session: &MatrixClientSession,
    display_name: Option<&str>,
) -> Result<MatrixOwnProfile, MatrixProfileError> {
    session
        .client()
        .account()
        .set_display_name(display_name)
        .await
        .map_err(MatrixProfileError::from_sdk_error)?;
    matrix_own_profile_from_session(session).await
}

pub async fn set_avatar(
    session: &MatrixClientSession,
    mime_type: &str,
    bytes: Vec<u8>,
) -> Result<MatrixOwnProfile, MatrixProfileError> {
    let mime = mime_type
        .parse::<mime::Mime>()
        .map_err(|_| MatrixProfileError::InvalidMimeType)?;
    session
        .client()
        .account()
        .upload_avatar(&mime, bytes)
        .await
        .map_err(MatrixProfileError::from_sdk_error)?;
    matrix_own_profile_from_session(session).await
}

pub async fn get_local_user_aliases(
    session: &MatrixClientSession,
) -> Result<MatrixLocalUserAliases, MatrixProfileError> {
    let raw = session
        .client()
        .account()
        .fetch_account_data(local_user_aliases_event_type())
        .await
        .map_err(MatrixProfileError::from_sdk_error)?;
    let Some(raw) = raw else {
        return Ok(MatrixLocalUserAliases::default());
    };
    let content = raw
        .deserialize_as_unchecked::<MatrixLocalUserAliases>()
        .map_err(|_| matrix_profile_serialization_error())?;

    Ok(MatrixLocalUserAliases {
        aliases: normalized_local_user_aliases(content.aliases),
    })
}

pub async fn set_local_user_aliases(
    session: &MatrixClientSession,
    aliases: BTreeMap<String, String>,
) -> Result<MatrixLocalUserAliases, MatrixProfileError> {
    let content = MatrixLocalUserAliases {
        aliases: normalized_local_user_aliases(aliases),
    };
    let raw: Raw<AnyGlobalAccountDataEventContent> = Raw::new(&content)
        .map_err(|_| matrix_profile_serialization_error())?
        .cast_unchecked();
    session
        .client()
        .account()
        .set_account_data_raw(local_user_aliases_event_type(), raw)
        .await
        .map_err(MatrixProfileError::from_sdk_error)?;

    Ok(content)
}

pub async fn update_local_user_alias(
    session: &MatrixClientSession,
    user_id: &str,
    alias: Option<&str>,
) -> Result<MatrixLocalUserAliases, MatrixProfileError> {
    let mut aliases = get_local_user_aliases(session).await?.aliases;
    if let Some(alias) = normalize_local_user_alias(alias) {
        aliases.insert(user_id.to_owned(), alias);
    } else {
        aliases.remove(user_id);
    }
    set_local_user_aliases(session, aliases).await
}

pub async fn send_text_message(
    session: &MatrixClientSession,
    room_id: &str,
    body: &str,
    transaction_id: &str,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let txn_id = matrix_sdk::ruma::OwnedTransactionId::from(transaction_id);
    let content =
        matrix_sdk::ruma::events::room::message::RoomMessageEventContent::text_plain(body);

    room.send(content)
        .with_transaction_id(txn_id)
        .await
        .map(|_| ())
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn room_can_send_text_message(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<bool, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    if room.state() != matrix_sdk::RoomState::Joined {
        return Ok(false);
    }

    let power_levels = room.power_levels_or_default().await;
    Ok(power_levels.user_can_send_message(
        room.own_user_id(),
        matrix_sdk::ruma::events::MessageLikeEventType::RoomMessage,
    ))
}

pub async fn get_room_settings_snapshot(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<MatrixRoomSettingsSnapshot, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    Ok(matrix_room_settings_snapshot(&room).await)
}

pub async fn update_room_setting(
    session: &MatrixClientSession,
    room_id: &str,
    change: MatrixRoomSettingChange,
) -> Result<MatrixRoomSettingsSnapshot, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let snapshot = matrix_room_settings_snapshot(&room).await;
    match &change {
        MatrixRoomSettingChange::Name(name) => {
            room.set_name(name.clone().unwrap_or_default())
                .await
                .map_err(MatrixRoomOperationError::from_sdk_error)?;
        }
        MatrixRoomSettingChange::Topic(topic) => {
            room.set_room_topic(topic.as_deref().unwrap_or_default())
                .await
                .map_err(MatrixRoomOperationError::from_sdk_error)?;
        }
        MatrixRoomSettingChange::AvatarUrl(Some(avatar_url)) => {
            let avatar_url = matrix_sdk::ruma::OwnedMxcUri::from(avatar_url.clone());
            room.set_avatar_url(avatar_url.as_ref(), None)
                .await
                .map_err(MatrixRoomOperationError::from_sdk_error)?;
        }
        MatrixRoomSettingChange::AvatarUrl(None) => {
            room.remove_avatar()
                .await
                .map_err(MatrixRoomOperationError::from_sdk_error)?;
        }
        MatrixRoomSettingChange::JoinRule(join_rule) => {
            let join_rule = sdk_join_rule_for_update(*join_rule)?;
            room.privacy_settings()
                .update_join_rule(join_rule)
                .await
                .map_err(MatrixRoomOperationError::from_sdk_error)?;
        }
        MatrixRoomSettingChange::HistoryVisibility(history_visibility) => {
            room.privacy_settings()
                .update_room_history_visibility(sdk_history_visibility(*history_visibility))
                .await
                .map_err(MatrixRoomOperationError::from_sdk_error)?;
        }
    }

    Ok(room_settings_snapshot_with_change(snapshot, &change))
}

pub async fn moderate_room_member(
    session: &MatrixClientSession,
    room_id: &str,
    target_user_id: &str,
    action: MatrixRoomModerationAction,
    reason: Option<&str>,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let target_user_id = matrix_sdk::ruma::UserId::parse(target_user_id)
        .map_err(|_| MatrixRoomOperationError::InvalidUserId)?;

    match action {
        MatrixRoomModerationAction::Kick => room.kick_user(&target_user_id, reason).await,
        MatrixRoomModerationAction::Ban => room.ban_user(&target_user_id, reason).await,
        MatrixRoomModerationAction::Unban => room.unban_user(&target_user_id, reason).await,
    }
    .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn update_room_member_power_level(
    session: &MatrixClientSession,
    room_id: &str,
    target_user_id: &str,
    power_level: i64,
) -> Result<MatrixRoomSettingsSnapshot, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let target_user_id = matrix_sdk::ruma::UserId::parse(target_user_id)
        .map_err(|_| MatrixRoomOperationError::InvalidUserId)?;
    let power_level = matrix_sdk::ruma::Int::try_from(power_level)
        .map_err(|_| MatrixRoomOperationError::InvalidRoomSetting)?;

    room.update_power_levels(vec![(target_user_id.as_ref(), power_level)])
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;

    let target_user_id_ref: &matrix_sdk::ruma::UserId = target_user_id.as_ref();
    Ok(room_settings_snapshot_with_member_power_level(
        matrix_room_settings_snapshot(&room).await,
        target_user_id_ref.as_str(),
        power_level.into(),
    ))
}

pub async fn create_room(
    session: &MatrixClientSession,
    name: &str,
    encrypted: bool,
) -> Result<String, MatrixRoomOperationError> {
    let mut request = matrix_sdk::ruma::api::client::room::create_room::v3::Request::new();
    request.name = non_empty_name(name);
    if encrypted {
        request.initial_state.push(
            matrix_sdk::ruma::events::InitialStateEvent::with_empty_state_key(
                matrix_sdk::ruma::events::room::encryption::RoomEncryptionEventContent::with_recommended_defaults(),
            )
            .to_raw_any(),
        );
    }
    let room = session
        .client()
        .create_room(request)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    Ok(room.room_id().to_string())
}

pub async fn create_public_directory_room(
    session: &MatrixClientSession,
    name: &str,
    alias_localpart: &str,
) -> Result<String, MatrixRoomOperationError> {
    let alias_localpart = alias_localpart.trim();
    if alias_localpart.is_empty()
        || alias_localpart.starts_with('#')
        || alias_localpart.contains(':')
    {
        return Err(MatrixRoomOperationError::InvalidRoomAlias);
    }

    let mut request = matrix_sdk::ruma::api::client::room::create_room::v3::Request::new();
    request.name = non_empty_name(name);
    request.room_alias_name = Some(alias_localpart.to_owned());
    request.visibility = matrix_sdk::ruma::api::client::room::Visibility::Public;
    request.preset =
        Some(matrix_sdk::ruma::api::client::room::create_room::v3::RoomPreset::PublicChat);

    let room = session
        .client()
        .create_room(request)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    Ok(room.room_id().to_string())
}

pub async fn create_space(
    session: &MatrixClientSession,
    name: &str,
) -> Result<String, MatrixRoomOperationError> {
    let mut creation_content =
        matrix_sdk::ruma::api::client::room::create_room::v3::CreationContent::new();
    creation_content.room_type = Some(matrix_sdk::ruma::room::RoomType::Space);

    let mut request = matrix_sdk::ruma::api::client::room::create_room::v3::Request::new();
    request.name = non_empty_name(name);
    request.creation_content = Some(
        matrix_sdk::ruma::serde::Raw::new(&creation_content)
            .map_err(|_| MatrixRoomOperationError::Sdk(MatrixRoomOperationFailureKind::Sdk))?,
    );

    let room = session
        .client()
        .create_room(request)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    Ok(room.room_id().to_string())
}

pub async fn invite_user_to_room(
    session: &MatrixClientSession,
    room_id: &str,
    user_id: &str,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let user_id = matrix_sdk::ruma::UserId::parse(user_id)
        .map_err(|_| MatrixRoomOperationError::InvalidUserId)?;
    room.invite_user_by_id(&user_id)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn start_direct_message(
    session: &MatrixClientSession,
    user_id: &str,
) -> Result<String, MatrixRoomOperationError> {
    let user_id = matrix_sdk::ruma::UserId::parse(user_id)
        .map_err(|_| MatrixRoomOperationError::InvalidUserId)?;
    let room = session
        .client()
        .create_dm(&user_id)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    Ok(room.room_id().to_string())
}

pub async fn join_room_by_id(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<String, MatrixRoomOperationError> {
    let room_id = matrix_sdk::ruma::RoomId::parse(room_id)
        .map_err(|_| MatrixRoomOperationError::InvalidRoomId)?;
    let room = session
        .client()
        .join_room_by_id(&room_id)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    Ok(room.room_id().to_string())
}

pub async fn query_public_room_directory(
    session: &MatrixClientSession,
    query: MatrixPublicRoomDirectoryQuery,
) -> Result<MatrixPublicRoomDirectoryResult, MatrixRoomOperationError> {
    let mut filter = matrix_sdk::ruma::directory::Filter::new();
    filter.generic_search_term = query.term;

    let mut request =
        matrix_sdk::ruma::api::client::directory::get_public_rooms_filtered::v3::Request::new();
    request.filter = filter;
    request.limit = query.limit.map(Into::into);
    request.since = query.since;
    request.server = query
        .server_name
        .map(matrix_sdk::ruma::OwnedServerName::try_from)
        .transpose()
        .map_err(|_| MatrixRoomOperationError::InvalidServerName)?;

    let response = session
        .client()
        .public_rooms_filtered(request)
        .await
        .map_err(|error| MatrixRoomOperationError::from_sdk_error(error.into()))?;

    Ok(MatrixPublicRoomDirectoryResult {
        rooms: response
            .chunk
            .into_iter()
            .map(matrix_public_room_from_chunk)
            .collect(),
        next_batch: response.next_batch,
    })
}

pub async fn join_room_by_alias(
    session: &MatrixClientSession,
    alias: &str,
    via_server: Option<&str>,
) -> Result<String, MatrixRoomOperationError> {
    let alias = matrix_sdk::ruma::RoomAliasId::parse(alias)
        .map_err(|_| MatrixRoomOperationError::InvalidRoomAlias)?;
    let room_or_alias = matrix_sdk::ruma::OwnedRoomOrAliasId::from(alias);
    let via = via_server
        .map(matrix_sdk::ruma::OwnedServerName::try_from)
        .transpose()
        .map_err(|_| MatrixRoomOperationError::InvalidServerName)?
        .into_iter()
        .collect::<Vec<_>>();
    let room = session
        .client()
        .join_room_by_id_or_alias(room_or_alias.as_ref(), &via)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    Ok(room.room_id().to_string())
}

pub async fn leave_room(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<String, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let room_id = room.room_id().to_string();
    room.leave()
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    Ok(room_id)
}

pub async fn forget_room(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<String, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let room_id = room.room_id().to_string();
    room.forget()
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    Ok(room_id)
}

pub async fn set_space_child(
    session: &MatrixClientSession,
    space_id: &str,
    child_room_id: &str,
    via_server: &str,
) -> Result<(), MatrixRoomOperationError> {
    let space = matrix_room(session, space_id)?;
    let child_room_id = matrix_sdk::ruma::OwnedRoomId::try_from(child_room_id)
        .map_err(|_| MatrixRoomOperationError::InvalidRoomId)?;
    let via_server = matrix_sdk::ruma::OwnedServerName::try_from(via_server)
        .map_err(|_| MatrixRoomOperationError::InvalidServerName)?;
    let content =
        matrix_sdk::ruma::events::space::child::SpaceChildEventContent::new(vec![via_server]);

    space
        .send_state_event_for_key(&child_room_id, content)
        .await
        .map(|_| ())
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn set_room_tag(
    session: &MatrixClientSession,
    room_id: &str,
    tag: MatrixRoomTagKind,
    order: Option<f64>,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    match tag {
        MatrixRoomTagKind::Favourite => room.set_is_favourite(true, order).await,
        MatrixRoomTagKind::LowPriority => room.set_is_low_priority(true, order).await,
    }
    .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn remove_room_tag(
    session: &MatrixClientSession,
    room_id: &str,
    tag: MatrixRoomTagKind,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    match tag {
        MatrixRoomTagKind::Favourite => room.set_is_favourite(false, None).await,
        MatrixRoomTagKind::LowPriority => room.set_is_low_priority(false, None).await,
    }
    .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn pin_event(
    session: &MatrixClientSession,
    room_id: &str,
    event_id: &str,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let event_id = matrix_sdk::ruma::EventId::parse(event_id)
        .map_err(|_| MatrixRoomOperationError::InvalidEventId)?;
    room.pin_event(&event_id)
        .await
        .map(|_| ())
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn unpin_event(
    session: &MatrixClientSession,
    room_id: &str,
    event_id: &str,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let event_id = matrix_sdk::ruma::EventId::parse(event_id)
        .map_err(|_| MatrixRoomOperationError::InvalidEventId)?;
    room.unpin_event(&event_id)
        .await
        .map(|_| ())
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn mark_room_as_read(
    session: &MatrixClientSession,
    room_id: &str,
    event_id: &str,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let event_id = matrix_sdk::ruma::EventId::parse(event_id)
        .map_err(|_| MatrixRoomOperationError::InvalidEventId)?;
    use matrix_sdk::ruma::api::client::receipt::create_receipt::v3::ReceiptType;
    use matrix_sdk::ruma::events::receipt::ReceiptThread;
    room.send_single_receipt(ReceiptType::FullyRead, ReceiptThread::Unthreaded, event_id)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn mark_room_as_unread(
    session: &MatrixClientSession,
    room_id: &str,
    unread: bool,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    room.set_unread_flag(unread)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn load_pinned_event_ids(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<Vec<String>, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let pinned = room
        .load_pinned_events()
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?
        .unwrap_or_default();
    Ok(pinned
        .into_iter()
        .map(|event_id| event_id.to_string())
        .collect())
}

pub async fn edit_text_message(
    session: &MatrixClientSession,
    room_id: &str,
    event_id: &str,
    body: &str,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let event_id = matrix_sdk::ruma::EventId::parse(event_id)
        .map_err(|_| MatrixRoomOperationError::InvalidEventId)?;
    let content =
        matrix_sdk::ruma::events::room::message::RoomMessageEventContentWithoutRelation::text_plain(
            body,
        );
    let edit_content = matrix_sdk::room::edit::EditedContent::RoomMessage(content);
    let edit_event = room
        .make_edit_event(&event_id, edit_content)
        .await
        .map_err(|_| MatrixRoomOperationError::Sdk(MatrixRoomOperationFailureKind::Sdk))?;

    room.send(edit_event)
        .await
        .map(|_| ())
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn redact_message(
    session: &MatrixClientSession,
    room_id: &str,
    event_id: &str,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let event_id = matrix_sdk::ruma::EventId::parse(event_id)
        .map_err(|_| MatrixRoomOperationError::InvalidEventId)?;

    room.redact(&event_id, None, None)
        .await
        .map(|_| ())
        .map_err(|_| MatrixRoomOperationError::Sdk(MatrixRoomOperationFailureKind::Sdk))
}

pub async fn search_message_candidates(
    session: &MatrixClientSession,
    query: &str,
    limit: usize,
) -> Result<Vec<MatrixSearchCandidate>, MatrixSearchError> {
    if query.trim().is_empty() || limit == 0 {
        return Ok(Vec::new());
    }

    let mut iterator = session
        .client()
        .search_messages(query.to_owned(), limit)
        .build();
    let Some(candidates) = iterator.next().await.map_err(|_| MatrixSearchError::Sdk)? else {
        return Ok(Vec::new());
    };

    Ok(candidates
        .into_iter()
        .take(limit)
        .enumerate()
        .map(|(index, (room_id, event_id))| MatrixSearchCandidate {
            room_id: room_id.to_string(),
            event_id: event_id.to_string(),
            score_millis: 1_000_u32.saturating_sub(index as u32),
        })
        .collect())
}

/// Normalize a room list snapshot from caller-provided SDK rooms.
///
/// This is the normalization entry point for callers that already hold the
/// room list source of truth: entries from the ONE live `RoomListService`
/// owned by the running `SyncService` (converted with
/// `RoomListItem::into_inner()`), or `client.joined_rooms()` on the
/// `LegacySync` backend. Unlike [`room_list_snapshot`], it never constructs
/// a `RoomListService` of its own.
pub async fn room_list_snapshot_from_sdk_rooms(
    rooms: impl IntoIterator<Item = matrix_sdk::Room>,
) -> MatrixRoomListSnapshot {
    matrix_room_list_snapshot_from_rooms(rooms).await
}

/// Normalize joined rooms from the caller's source of truth plus invited rooms
/// from the base client. The live `RoomListService` path remains the owner of
/// joined-room entries; invites are projected from `client.invited_rooms()`
/// because the live entries adapter is intentionally joined-filtered.
pub async fn room_list_snapshot_from_sdk_rooms_with_invites(
    session: &MatrixClientSession,
    rooms: impl IntoIterator<Item = matrix_sdk::Room>,
) -> MatrixRoomListSnapshot {
    let mut snapshot = matrix_room_list_snapshot_from_rooms(rooms).await;
    snapshot.invites = matrix_invite_previews_from_rooms(session.client().invited_rooms()).await;
    snapshot
}

/// One-shot room list snapshot that constructs a DISPOSABLE
/// `RoomListService` internally.
///
/// DEPRECATED FOR CORE USE (canon, overview.md RoomActor): a disposable
/// `RoomListService` is not driven by the sync loop, races the running
/// `SyncService`, and returns entries without the live service's
/// `required_state` (e.g. `m.room.create`), so space classification is
/// unreliable (deterministically broken on Conduit). The core runtime must
/// use [`room_list_snapshot_from_sdk_rooms`] with rooms taken from the live
/// service's entries or from `client.joined_rooms()`. This function remains
/// only for the legacy auth-crate QA flow, which runs without a
/// `SyncService`.
pub async fn room_list_snapshot(
    session: &MatrixClientSession,
) -> Result<MatrixRoomListSnapshot, MatrixRoomListError> {
    let client = session.client();
    let service = match matrix_sdk_ui::room_list_service::RoomListService::new_with(
        client.clone(),
        false,
        "matrix-desktop-room-list-snapshot",
        1,
    )
    .await
    {
        Ok(service) => service,
        Err(_) => return Ok(matrix_room_list_snapshot_from_rooms(client.joined_rooms()).await),
    };
    let all_rooms = service
        .all_rooms()
        .await
        .map_err(|_| MatrixRoomListError::Sdk)?;
    let (entries, entries_controller) =
        all_rooms.entries_with_dynamic_adapters(MATRIX_ROOM_LIST_SNAPSHOT_LIMIT);
    entries_controller.set_filter(Box::new(
        matrix_sdk_ui::room_list_service::filters::new_filter_joined(),
    ));

    let mut entries = Box::pin(entries);
    let Some(diffs) = entries.next().await else {
        return Ok(MatrixRoomListSnapshot::default());
    };

    let snapshot = matrix_room_list_snapshot_from_diffs(diffs).await;
    if snapshot.rooms.is_empty() && snapshot.spaces.is_empty() {
        return Ok(matrix_room_list_snapshot_from_rooms(client.joined_rooms()).await);
    }

    Ok(snapshot)
}

pub fn room_list_smoke_report(snapshot: &MatrixRoomListSnapshot) -> RoomListSmokeReport {
    RoomListSmokeReport {
        rooms: snapshot.rooms.len(),
        spaces: snapshot.spaces.len(),
        dms: snapshot.rooms.iter().filter(|room| room.is_dm).count(),
        unread_rooms: snapshot
            .rooms
            .iter()
            .filter(|room| room.unread_count > 0)
            .count(),
    }
}

pub fn room_attention_summary_from_counts(
    room_display_name: Option<String>,
    is_dm: bool,
    notification_count: u64,
    highlight_count: u64,
    unread_messages: u64,
    is_marked_unread: bool,
) -> Option<RoomAttentionSummary> {
    let unread_count =
        room_attention_unread_count(notification_count, unread_messages, is_marked_unread);
    let room_display_name = room_display_name.unwrap_or_else(|| "Room".to_owned());

    room_attention_summary(
        room_display_name,
        is_dm,
        notification_count,
        highlight_count,
        unread_count,
    )
}

pub fn timeline_smoke_report(
    selected_room_present: bool,
    initial_items: &[MatrixTimelineItem],
) -> TimelineSmokeReport {
    TimelineSmokeReport {
        selected_room_present,
        timeline_items: initial_items.len(),
    }
}

pub fn real_account_qa_report(
    snapshot: &MatrixRoomListSnapshot,
    selected_room_present: bool,
    initial_items: &[MatrixTimelineItem],
) -> RealAccountQaReport {
    real_account_qa_report_with_restore_state(snapshot, selected_room_present, initial_items, false)
}

pub fn restored_real_account_qa_report(
    snapshot: &MatrixRoomListSnapshot,
    selected_room_present: bool,
    initial_items: &[MatrixTimelineItem],
) -> RealAccountQaReport {
    real_account_qa_report_with_restore_state(snapshot, selected_room_present, initial_items, true)
}

pub fn real_account_qa_report_with_search(
    snapshot: &MatrixRoomListSnapshot,
    selected_room_present: bool,
    initial_items: &[MatrixTimelineItem],
    session_restored: bool,
    search_candidates: &[MatrixSearchCandidate],
) -> RealAccountQaReport {
    let mut report = real_account_qa_report_with_restore_state(
        snapshot,
        selected_room_present,
        initial_items,
        session_restored,
    );
    report.search = search_smoke_report(true, search_candidates);
    report
}

pub fn search_smoke_report(
    invoked: bool,
    candidates: &[MatrixSearchCandidate],
) -> SearchSmokeReport {
    SearchSmokeReport {
        invoked,
        candidates: candidates.len(),
    }
}

fn real_account_qa_report_with_restore_state(
    snapshot: &MatrixRoomListSnapshot,
    selected_room_present: bool,
    initial_items: &[MatrixTimelineItem],
    session_restored: bool,
) -> RealAccountQaReport {
    RealAccountQaReport {
        room_list: room_list_smoke_report(snapshot),
        timeline: timeline_smoke_report(selected_room_present, initial_items),
        session_restored,
        search: SearchSmokeReport::default(),
    }
}

pub async fn subscribe_room_timeline(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<MatrixTimelineSubscription, MatrixTimelineError> {
    let room = timeline_room(session, room_id)?;
    let timeline = matrix_sdk_ui::timeline::TimelineBuilder::new(&room)
        .build()
        .await
        .map_err(|_| MatrixTimelineError::Sdk)?;
    let timeline = Arc::new(timeline);
    let (items, updates) = timeline.subscribe().await;
    let initial_items = items
        .iter()
        .filter_map(|item| matrix_timeline_item_from_ui(room_id, item))
        .collect();
    let update_room_id = room_id.to_owned();
    let updates = updates
        .map(move |diffs| matrix_timeline_updates_from_diffs(&update_room_id, diffs))
        .boxed();

    Ok(MatrixTimelineSubscription {
        timeline,
        initial_items,
        updates,
    })
}

pub async fn room_timeline_visible_items(
    session: &MatrixClientSession,
    room_id: &str,
    backfill_event_count: u16,
) -> Result<Vec<MatrixTimelineItem>, MatrixTimelineError> {
    let room = timeline_room(session, room_id)?;
    let timeline = matrix_sdk_ui::timeline::TimelineBuilder::new(&room)
        .build()
        .await
        .map_err(|_| MatrixTimelineError::Sdk)?;
    let (items, updates) = timeline.subscribe().await;
    let mut items = items
        .iter()
        .filter_map(|item| matrix_timeline_item_from_ui(room_id, item))
        .collect::<Vec<_>>();
    if !items.is_empty() || backfill_event_count == 0 {
        return Ok(items);
    }

    let mut updates = Box::pin(updates);
    timeline
        .paginate_backwards(backfill_event_count)
        .await
        .map_err(|_| MatrixTimelineError::Sdk)?;

    for _ in 0..3 {
        let Some(diffs) = tokio::time::timeout(Duration::from_secs(10), updates.next())
            .await
            .map_err(|_| MatrixTimelineError::Sdk)?
        else {
            break;
        };
        items.extend(
            matrix_timeline_updates_from_diffs(room_id, diffs)
                .into_iter()
                .filter_map(|update| match update {
                    MatrixTimelineUpdate::Upsert(item) => Some(item),
                    MatrixTimelineUpdate::Remove { .. } => None,
                }),
        );
        if !items.is_empty() {
            break;
        }
    }

    Ok(items)
}

pub async fn sync_once(session: &MatrixClientSession) -> Result<(), MatrixSyncError> {
    session
        .client()
        .sync_once(matrix_sdk::config::SyncSettings::default())
        .await
        .map(|_| ())
        .map_err(|_| MatrixSyncError::Sdk)
}

pub async fn sync_loop<F, C>(
    session: &MatrixClientSession,
    on_successful_sync: F,
) -> Result<(), MatrixSyncError>
where
    F: Fn() -> C,
    C: Future<Output = MatrixSyncLoopControl>,
{
    session
        .client()
        .sync_with_callback(matrix_sdk::config::SyncSettings::default(), move |_| {
            let callback = on_successful_sync();
            async move {
                match callback.await {
                    MatrixSyncLoopControl::Continue => matrix_sdk::LoopCtrl::Continue,
                    MatrixSyncLoopControl::Stop => matrix_sdk::LoopCtrl::Break,
                }
            }
        })
        .await
        .map_err(|_| MatrixSyncError::Sdk)
}

fn matrix_room(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<matrix_sdk::Room, MatrixRoomOperationError> {
    let room_id = matrix_sdk::ruma::RoomId::parse(room_id)
        .map_err(|_| MatrixRoomOperationError::InvalidRoomId)?;
    session
        .client()
        .get_room(&room_id)
        .ok_or(MatrixRoomOperationError::RoomUnavailable)
}

fn matrix_public_room_from_chunk(
    chunk: matrix_sdk::ruma::directory::PublicRoomsChunk,
) -> MatrixPublicRoomDirectoryRoom {
    MatrixPublicRoomDirectoryRoom {
        room_id: chunk.room_id.to_string(),
        canonical_alias: chunk.canonical_alias.map(|alias| alias.to_string()),
        name: chunk.name.unwrap_or_else(|| "Public room".to_owned()),
        topic: chunk.topic,
        avatar_url: chunk.avatar_url.map(|avatar_url| avatar_url.to_string()),
        joined_members: chunk.num_joined_members.into(),
        world_readable: chunk.world_readable,
        guest_can_join: chunk.guest_can_join,
    }
}

async fn matrix_room_settings_snapshot(room: &matrix_sdk::Room) -> MatrixRoomSettingsSnapshot {
    let power_levels = room.power_levels_or_default().await;
    let own_user_id = room.own_user_id();
    let members = matrix_room_member_summaries(room).await;
    let can_edit_settings = power_levels.user_can_send_state(
        own_user_id,
        matrix_sdk::ruma::events::StateEventType::RoomName,
    ) && power_levels.user_can_send_state(
        own_user_id,
        matrix_sdk::ruma::events::StateEventType::RoomTopic,
    ) && power_levels.user_can_send_state(
        own_user_id,
        matrix_sdk::ruma::events::StateEventType::RoomAvatar,
    ) && power_levels.user_can_send_state(
        own_user_id,
        matrix_sdk::ruma::events::StateEventType::RoomJoinRules,
    ) && power_levels.user_can_send_state(
        own_user_id,
        matrix_sdk::ruma::events::StateEventType::RoomHistoryVisibility,
    );

    MatrixRoomSettingsSnapshot {
        room_id: room.room_id().to_string(),
        name: room.name(),
        topic: room.topic(),
        avatar_url: room.avatar_url().map(|url| url.to_string()),
        join_rule: room
            .join_rule()
            .as_ref()
            .map(matrix_room_join_rule)
            .unwrap_or(MatrixRoomJoinRule::Invite),
        history_visibility: matrix_room_history_visibility(&room.history_visibility_or_default()),
        permissions: MatrixRoomPermissionFacts {
            can_edit_settings,
            can_edit_roles: power_levels.user_can_send_state(
                own_user_id,
                matrix_sdk::ruma::events::StateEventType::RoomPowerLevels,
            ),
            can_kick: power_levels.user_can_kick(own_user_id),
            can_ban: power_levels.user_can_ban(own_user_id),
            can_unban: power_levels.user_can_ban(own_user_id),
        },
        members,
    }
}

async fn matrix_room_member_summaries(room: &matrix_sdk::Room) -> Vec<MatrixRoomMemberSummary> {
    let Ok(members) = room.members(matrix_sdk::RoomMemberships::ACTIVE).await else {
        return Vec::new();
    };
    let mut summaries: Vec<MatrixRoomMemberSummary> = members
        .into_iter()
        .map(|member| {
            let power_level = matrix_room_member_power_level(member.power_level());
            MatrixRoomMemberSummary {
                user_id: member.user_id().to_string(),
                display_name: member.display_name().map(ToOwned::to_owned),
                avatar_url: member.avatar_url().map(ToString::to_string),
                power_level,
                role: matrix_room_member_role(power_level),
            }
        })
        .collect();
    summaries.sort_by(|left, right| left.user_id.cmp(&right.user_id));
    summaries
}

fn room_settings_snapshot_with_member_power_level(
    mut snapshot: MatrixRoomSettingsSnapshot,
    target_user_id: &str,
    power_level: i64,
) -> MatrixRoomSettingsSnapshot {
    if let Some(member) = snapshot
        .members
        .iter_mut()
        .find(|member| member.user_id == target_user_id)
    {
        member.power_level = Some(power_level);
        member.role = matrix_room_member_role(Some(power_level));
    }
    snapshot
}

fn matrix_room_member_power_level(
    power_level: matrix_sdk::ruma::events::room::power_levels::UserPowerLevel,
) -> Option<i64> {
    match power_level {
        matrix_sdk::ruma::events::room::power_levels::UserPowerLevel::Infinite => None,
        matrix_sdk::ruma::events::room::power_levels::UserPowerLevel::Int(value) => {
            Some(value.into())
        }
        _ => None,
    }
}

fn matrix_room_member_role(power_level: Option<i64>) -> MatrixRoomMemberRole {
    match power_level {
        None => MatrixRoomMemberRole::Creator,
        Some(level) if level >= 100 => MatrixRoomMemberRole::Administrator,
        Some(level) if level >= 50 => MatrixRoomMemberRole::Moderator,
        Some(_) => MatrixRoomMemberRole::User,
    }
}

fn room_settings_snapshot_with_change(
    mut snapshot: MatrixRoomSettingsSnapshot,
    change: &MatrixRoomSettingChange,
) -> MatrixRoomSettingsSnapshot {
    match change {
        MatrixRoomSettingChange::Name(name) => {
            snapshot.name = name.clone();
        }
        MatrixRoomSettingChange::Topic(topic) => {
            snapshot.topic = topic.clone();
        }
        MatrixRoomSettingChange::AvatarUrl(avatar_url) => {
            snapshot.avatar_url = avatar_url.clone();
        }
        MatrixRoomSettingChange::JoinRule(join_rule) => {
            snapshot.join_rule = *join_rule;
        }
        MatrixRoomSettingChange::HistoryVisibility(history_visibility) => {
            snapshot.history_visibility = *history_visibility;
        }
    }
    snapshot
}

fn matrix_room_join_rule(
    join_rule: &matrix_sdk::ruma::events::room::join_rules::JoinRule,
) -> MatrixRoomJoinRule {
    use matrix_sdk::ruma::events::room::join_rules::JoinRule;
    match join_rule {
        JoinRule::Public => MatrixRoomJoinRule::Public,
        JoinRule::Invite => MatrixRoomJoinRule::Invite,
        JoinRule::Knock => MatrixRoomJoinRule::Knock,
        JoinRule::Restricted(_) | JoinRule::KnockRestricted(_) => MatrixRoomJoinRule::Restricted,
        JoinRule::Private => MatrixRoomJoinRule::Private,
        _ => MatrixRoomJoinRule::Invite,
    }
}

fn sdk_join_rule_for_update(
    join_rule: MatrixRoomJoinRule,
) -> Result<matrix_sdk::ruma::events::room::join_rules::JoinRule, MatrixRoomOperationError> {
    use matrix_sdk::ruma::events::room::join_rules::JoinRule;
    match join_rule {
        MatrixRoomJoinRule::Public => Ok(JoinRule::Public),
        MatrixRoomJoinRule::Invite => Ok(JoinRule::Invite),
        MatrixRoomJoinRule::Knock => Ok(JoinRule::Knock),
        MatrixRoomJoinRule::Private => Ok(JoinRule::Private),
        MatrixRoomJoinRule::Restricted => Err(MatrixRoomOperationError::InvalidRoomSetting),
    }
}

fn matrix_room_history_visibility(
    history_visibility: &matrix_sdk::ruma::events::room::history_visibility::HistoryVisibility,
) -> MatrixRoomHistoryVisibility {
    use matrix_sdk::ruma::events::room::history_visibility::HistoryVisibility;
    match history_visibility {
        HistoryVisibility::WorldReadable => MatrixRoomHistoryVisibility::WorldReadable,
        HistoryVisibility::Shared => MatrixRoomHistoryVisibility::Shared,
        HistoryVisibility::Invited => MatrixRoomHistoryVisibility::Invited,
        HistoryVisibility::Joined => MatrixRoomHistoryVisibility::Joined,
        _ => MatrixRoomHistoryVisibility::Shared,
    }
}

fn sdk_history_visibility(
    history_visibility: MatrixRoomHistoryVisibility,
) -> matrix_sdk::ruma::events::room::history_visibility::HistoryVisibility {
    use matrix_sdk::ruma::events::room::history_visibility::HistoryVisibility;
    match history_visibility {
        MatrixRoomHistoryVisibility::WorldReadable => HistoryVisibility::WorldReadable,
        MatrixRoomHistoryVisibility::Shared => HistoryVisibility::Shared,
        MatrixRoomHistoryVisibility::Invited => HistoryVisibility::Invited,
        MatrixRoomHistoryVisibility::Joined => HistoryVisibility::Joined,
    }
}

fn non_empty_name(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_owned())
    }
}

fn matrix_room_operation_failure_kind(error: &matrix_sdk::Error) -> MatrixRoomOperationFailureKind {
    match error {
        matrix_sdk::Error::AuthenticationRequired => {
            MatrixRoomOperationFailureKind::AuthenticationRequired
        }
        matrix_sdk::Error::WrongRoomState(_) => MatrixRoomOperationFailureKind::WrongRoomState,
        matrix_sdk::Error::Http(error) => {
            if error
                .as_client_api_error()
                .is_some_and(|error| error.status_code.as_u16() == 403)
                || matches!(
                    error.client_api_error_kind(),
                    Some(matrix_sdk::ruma::api::error::ErrorKind::Forbidden)
                )
            {
                MatrixRoomOperationFailureKind::Forbidden
            } else {
                MatrixRoomOperationFailureKind::Http
            }
        }
        matrix_sdk::Error::BadCryptoStoreState
        | matrix_sdk::Error::NoOlmMachine
        | matrix_sdk::Error::CryptoStoreError(_)
        | matrix_sdk::Error::OlmError(_)
        | matrix_sdk::Error::MegolmError(_)
        | matrix_sdk::Error::DecryptorError(_) => MatrixRoomOperationFailureKind::Encryption,
        matrix_sdk::Error::StateStore(_)
        | matrix_sdk::Error::EventCacheStore(_)
        | matrix_sdk::Error::MediaStore(_) => MatrixRoomOperationFailureKind::Store,
        matrix_sdk::Error::SerdeJson(_)
        | matrix_sdk::Error::Io(_)
        | matrix_sdk::Error::CrossProcessLockError(_)
        | matrix_sdk::Error::Identifier(_)
        | matrix_sdk::Error::Url(_)
        | matrix_sdk::Error::SlidingSync(_)
        | matrix_sdk::Error::MultipleSessionCallbacks
        | matrix_sdk::Error::OAuth(_)
        | matrix_sdk::Error::ConcurrentRequestFailed
        | matrix_sdk::Error::UnknownError(_)
        | matrix_sdk::Error::EventCache(_)
        | matrix_sdk::Error::SendQueueWedgeError(_)
        | matrix_sdk::Error::BackupNotEnabled
        | matrix_sdk::Error::CantIgnoreLoggedInUser
        | matrix_sdk::Error::Media(_)
        | matrix_sdk::Error::ReplyError(_)
        | matrix_sdk::Error::PowerLevels(_)
        | matrix_sdk::Error::Timeout
        | matrix_sdk::Error::InsufficientData => MatrixRoomOperationFailureKind::Sdk,
        _ => MatrixRoomOperationFailureKind::Sdk,
    }
}

fn timeline_room(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<matrix_sdk::Room, MatrixTimelineError> {
    let room_id =
        matrix_sdk::ruma::RoomId::parse(room_id).map_err(|_| MatrixTimelineError::InvalidRoomId)?;
    session
        .client()
        .get_room(&room_id)
        .ok_or(MatrixTimelineError::RoomUnavailable)
}

fn matrix_timeline_updates_from_diffs(
    room_id: &str,
    diffs: Vec<eyeball_im::VectorDiff<Arc<matrix_sdk_ui::timeline::TimelineItem>>>,
) -> Vec<MatrixTimelineUpdate> {
    let mut updates = Vec::new();
    for diff in diffs {
        match diff {
            eyeball_im::VectorDiff::Append { values }
            | eyeball_im::VectorDiff::Reset { values } => {
                updates.extend(
                    values
                        .iter()
                        .filter_map(|item| matrix_timeline_update_from_ui(room_id, item)),
                );
            }
            eyeball_im::VectorDiff::PushFront { value }
            | eyeball_im::VectorDiff::PushBack { value }
            | eyeball_im::VectorDiff::Insert { value, .. }
            | eyeball_im::VectorDiff::Set { value, .. } => {
                if let Some(update) = matrix_timeline_update_from_ui(room_id, &value) {
                    updates.push(update);
                }
            }
            eyeball_im::VectorDiff::Clear
            | eyeball_im::VectorDiff::PopFront
            | eyeball_im::VectorDiff::PopBack
            | eyeball_im::VectorDiff::Remove { .. }
            | eyeball_im::VectorDiff::Truncate { .. } => {}
        }
    }
    updates
}

fn matrix_timeline_update_from_ui(
    room_id: &str,
    item: &matrix_sdk_ui::timeline::TimelineItem,
) -> Option<MatrixTimelineUpdate> {
    let event = item.as_event()?;
    let event_id = event.event_id()?.to_string();
    match event.content().as_message() {
        Some(content) => Some(MatrixTimelineUpdate::Upsert(MatrixTimelineItem {
            room_id: room_id.to_owned(),
            event_id,
            sender: event.sender().to_string(),
            timestamp_ms: event.timestamp().0.into(),
            body: content.body().to_owned(),
        })),
        None => Some(MatrixTimelineUpdate::Remove {
            room_id: room_id.to_owned(),
            event_id,
        }),
    }
}

async fn matrix_room_list_snapshot_from_diffs(
    diffs: Vec<eyeball_im::VectorDiff<matrix_sdk_ui::room_list_service::RoomListItem>>,
) -> MatrixRoomListSnapshot {
    let mut items = Vec::new();
    for diff in diffs {
        match diff {
            eyeball_im::VectorDiff::Append { values }
            | eyeball_im::VectorDiff::Reset { values } => {
                items.extend(values.into_iter());
            }
            eyeball_im::VectorDiff::PushFront { value }
            | eyeball_im::VectorDiff::PushBack { value }
            | eyeball_im::VectorDiff::Insert { value, .. }
            | eyeball_im::VectorDiff::Set { value, .. } => {
                items.push(value);
            }
            eyeball_im::VectorDiff::Clear
            | eyeball_im::VectorDiff::PopFront
            | eyeball_im::VectorDiff::PopBack
            | eyeball_im::VectorDiff::Remove { .. }
            | eyeball_im::VectorDiff::Truncate { .. } => {}
        }
    }

    matrix_room_list_snapshot_from_items(items).await
}

async fn matrix_room_list_snapshot_from_items(
    items: Vec<matrix_sdk_ui::room_list_service::RoomListItem>,
) -> MatrixRoomListSnapshot {
    matrix_room_list_snapshot_from_rooms(items.into_iter().map(|item| item.into_inner())).await
}

async fn matrix_room_list_snapshot_from_rooms(
    rooms: impl IntoIterator<Item = matrix_sdk::Room>,
) -> MatrixRoomListSnapshot {
    let mut snapshot = MatrixRoomListSnapshot::default();
    let mut user_profiles = BTreeMap::new();
    for room in rooms {
        if room.state() != matrix_sdk::RoomState::Joined {
            continue;
        }

        let active_user_ids = collect_active_member_profiles(&room, &mut user_profiles).await;

        let room_id = room.room_id().to_string();
        let display_name = room
            .cached_display_name()
            .map(|name| name.to_string())
            .unwrap_or_else(|| room_id.clone());

        if room.is_space() {
            snapshot.spaces.push(MatrixRoomListSpace {
                space_id: room_id,
                display_name,
                avatar_mxc_uri: room.avatar_url().map(|uri| uri.to_string()),
            });
            continue;
        }

        let unread_notifications = room.unread_notification_counts();
        let notification_count = unread_notifications.notification_count.into();
        let highlight_count = unread_notifications.highlight_count.into();
        let is_marked_unread = room.is_marked_unread();
        let unread_count = room_attention_unread_count(
            notification_count,
            room.num_unread_messages(),
            is_marked_unread,
        );

        let parent_space_ids = matrix_parent_space_ids(&room).await;
        let tags = matrix_room_tags(&room).await;

        let is_dm = room.is_dm();
        let own_user_id = room.own_user_id().to_string();
        let dm_user_ids = if is_dm {
            active_user_ids
                .into_iter()
                .filter(|user_id| user_id != &own_user_id)
                .collect()
        } else {
            Vec::new()
        };

        snapshot.rooms.push(matrix_room_list_room_from_counts(
            room_id,
            display_name,
            room.avatar_url().map(|uri| uri.to_string()),
            is_dm,
            dm_user_ids,
            tags,
            notification_count,
            highlight_count,
            unread_count,
            is_marked_unread,
            room.recency_stamp().map(|stamp| stamp.into()).unwrap_or(0),
            parent_space_ids,
        ));
    }
    snapshot.user_profiles = user_profiles.into_values().collect();
    snapshot
}

async fn collect_active_member_profiles(
    room: &matrix_sdk::Room,
    user_profiles: &mut BTreeMap<String, MatrixUserProfile>,
) -> Vec<String> {
    let Ok(members) = room.members(matrix_sdk::RoomMemberships::ACTIVE).await else {
        return Vec::new();
    };
    let mut user_ids = Vec::new();
    for member in members {
        let user_id = member.user_id().to_string();
        user_ids.push(user_id.clone());
        user_profiles
            .entry(user_id.clone())
            .or_insert_with(|| MatrixUserProfile {
                user_id,
                display_name: member.display_name().map(ToOwned::to_owned),
                avatar_mxc_uri: member.avatar_url().map(ToString::to_string),
            });
    }
    user_ids.sort();
    user_ids.dedup();
    user_ids
}

async fn matrix_invite_previews_from_rooms(
    rooms: impl IntoIterator<Item = matrix_sdk::Room>,
) -> Vec<MatrixInvitePreview> {
    let mut invites = Vec::new();
    for room in rooms {
        if room.state() != matrix_sdk::RoomState::Invited {
            continue;
        }

        let display_name = room
            .display_name()
            .await
            .ok()
            .map(|name| name.to_string())
            .or_else(|| room.name())
            .unwrap_or_else(|| "Invite".to_owned());
        let inviter_display_name = room
            .invite_details()
            .await
            .ok()
            .and_then(|details| details.inviter)
            .and_then(|inviter| inviter.display_name().map(ToOwned::to_owned));
        let is_dm = room.is_direct().await.unwrap_or(false);

        invites.push(MatrixInvitePreview {
            room_id: room.room_id().to_string(),
            display_name,
            avatar_mxc_uri: room.avatar_url().map(|uri| uri.to_string()),
            topic: room.topic(),
            inviter_display_name,
            is_dm,
        });
    }
    invites
}

fn room_attention_unread_count(
    notification_count: u64,
    unread_messages: u64,
    is_marked_unread: bool,
) -> u64 {
    let unread_count = notification_count.max(unread_messages);
    if unread_count == 0 && is_marked_unread {
        1
    } else {
        unread_count
    }
}

fn matrix_room_list_room_from_counts(
    room_id: String,
    display_name: String,
    avatar_mxc_uri: Option<String>,
    is_dm: bool,
    dm_user_ids: Vec<String>,
    tags: MatrixRoomTags,
    notification_count: u64,
    highlight_count: u64,
    unread_count: u64,
    marked_unread: bool,
    last_activity_ms: u64,
    parent_space_ids: Vec<String>,
) -> MatrixRoomListRoom {
    MatrixRoomListRoom {
        room_id,
        display_name,
        avatar_mxc_uri,
        is_dm,
        dm_user_ids,
        tags,
        unread_count,
        notification_count,
        highlight_count,
        marked_unread,
        last_activity_ms,
        parent_space_ids,
    }
}

async fn matrix_room_tags(room: &matrix_sdk::Room) -> MatrixRoomTags {
    let tags = room.tags().await.ok().flatten();
    let favourite = tags
        .as_ref()
        .and_then(|tags| tags.get(&matrix_sdk::ruma::events::tag::TagName::Favorite))
        .map(matrix_room_tag_info_from_sdk)
        .or_else(|| {
            room.is_favourite()
                .then_some(MatrixRoomTagInfo { order: None })
        });
    let low_priority = tags
        .as_ref()
        .and_then(|tags| tags.get(&matrix_sdk::ruma::events::tag::TagName::LowPriority))
        .map(matrix_room_tag_info_from_sdk)
        .or_else(|| {
            room.is_low_priority()
                .then_some(MatrixRoomTagInfo { order: None })
        });

    MatrixRoomTags {
        favourite,
        low_priority,
    }
}

fn matrix_room_tag_info_from_sdk(
    info: &matrix_sdk::ruma::events::tag::TagInfo,
) -> MatrixRoomTagInfo {
    MatrixRoomTagInfo {
        order: info.order.map(|order| order.to_string()),
    }
}

pub fn room_attention_summary_from_room(room: &matrix_sdk::Room) -> Option<RoomAttentionSummary> {
    let room_display_name = room.cached_display_name().map(|name| name.to_string())?;
    let unread_notifications = room.unread_notification_counts();

    room_attention_summary_from_counts(
        Some(room_display_name),
        room.is_dm(),
        unread_notifications.notification_count.into(),
        unread_notifications.highlight_count.into(),
        room.num_unread_messages(),
        room.is_marked_unread(),
    )
}

async fn matrix_own_profile_from_session(
    session: &MatrixClientSession,
) -> Result<MatrixOwnProfile, MatrixProfileError> {
    let account = session.client().account();
    let display_name = account
        .get_display_name()
        .await
        .map_err(MatrixProfileError::from_sdk_error)?;
    let avatar_mxc_uri = account
        .get_avatar_url()
        .await
        .map_err(MatrixProfileError::from_sdk_error)?
        .map(|uri| uri.to_string());
    Ok(MatrixOwnProfile {
        display_name,
        avatar_mxc_uri,
    })
}

fn local_user_aliases_event_type() -> GlobalAccountDataEventType {
    GlobalAccountDataEventType::from(LOCAL_USER_ALIASES_ACCOUNT_DATA_TYPE.to_owned())
}

fn normalized_local_user_aliases(aliases: BTreeMap<String, String>) -> BTreeMap<String, String> {
    aliases
        .into_iter()
        .filter_map(|(user_id, alias)| {
            if user_id.trim().is_empty() {
                return None;
            }
            normalize_local_user_alias(Some(&alias)).map(|alias| (user_id, alias))
        })
        .collect()
}

fn normalize_local_user_alias(alias: Option<&str>) -> Option<String> {
    alias
        .map(str::trim)
        .filter(|alias| !alias.is_empty())
        .map(ToOwned::to_owned)
}

fn matrix_profile_serialization_error() -> MatrixProfileError {
    MatrixProfileError::Sdk(MatrixProfileFailureKind::Sdk)
}

fn matrix_profile_failure_kind(error: &matrix_sdk::Error) -> MatrixProfileFailureKind {
    match error {
        matrix_sdk::Error::Http(error) => {
            if error
                .as_client_api_error()
                .is_some_and(|error| error.status_code.as_u16() == 403)
                || matches!(
                    error.client_api_error_kind(),
                    Some(matrix_sdk::ruma::api::error::ErrorKind::Forbidden)
                )
            {
                MatrixProfileFailureKind::Forbidden
            } else {
                MatrixProfileFailureKind::Sdk
            }
        }
        matrix_sdk::Error::Timeout => MatrixProfileFailureKind::Network,
        _ => MatrixProfileFailureKind::Sdk,
    }
}

async fn matrix_parent_space_ids(room: &matrix_sdk::Room) -> Vec<String> {
    let Ok(parent_spaces) = room.parent_spaces().await else {
        return Vec::new();
    };

    parent_spaces
        .filter_map(|parent_space| async move {
            match parent_space.ok()? {
                ParentSpace::Reciprocal(space) | ParentSpace::WithPowerlevel(space) => {
                    Some(space.room_id().to_string())
                }
                ParentSpace::Illegitimate(_) | ParentSpace::Unverifiable(_) => None,
            }
        })
        .collect()
        .await
}

fn matrix_timeline_item_from_ui(
    room_id: &str,
    item: &matrix_sdk_ui::timeline::TimelineItem,
) -> Option<MatrixTimelineItem> {
    match matrix_timeline_update_from_ui(room_id, item)? {
        MatrixTimelineUpdate::Upsert(item) => Some(item),
        MatrixTimelineUpdate::Remove { .. } => None,
    }
}

fn map_sdk_recovery_state(
    state: matrix_sdk::encryption::recovery::RecoveryState,
) -> E2eeRecoveryState {
    match state {
        matrix_sdk::encryption::recovery::RecoveryState::Unknown => E2eeRecoveryState::Unknown,
        matrix_sdk::encryption::recovery::RecoveryState::Enabled => E2eeRecoveryState::Enabled,
        matrix_sdk::encryption::recovery::RecoveryState::Disabled => E2eeRecoveryState::Disabled,
        matrix_sdk::encryption::recovery::RecoveryState::Incomplete => {
            E2eeRecoveryState::Incomplete
        }
    }
}

pub fn parse_login_discovery_http_response(
    status: u16,
    body: &str,
) -> Result<Vec<LoginFlow>, LoginDiscoveryError> {
    if status != 200 {
        return Err(LoginDiscoveryError::HttpStatus {
            status,
            message: matrix_error_message(body),
        });
    }

    let value = serde_json::from_str::<serde_json::Value>(body)
        .map_err(|error| LoginDiscoveryError::InvalidResponse(error.to_string()))?;
    parse_login_discovery(&value)
}

pub fn parse_login_discovery(
    value: &serde_json::Value,
) -> Result<Vec<LoginFlow>, LoginDiscoveryError> {
    Ok(map_login_flows_to_desktop(parse_matrix_login_flows(value)?))
}

pub fn parse_matrix_login_flows(
    value: &serde_json::Value,
) -> Result<Vec<MatrixLoginFlow>, LoginDiscoveryError> {
    if !value.get("flows").is_some_and(serde_json::Value::is_array) {
        return Err(LoginDiscoveryError::MissingFlows);
    }

    let response = serde_json::from_value::<LoginDiscoveryResponse>(value.clone())
        .map_err(|error| LoginDiscoveryError::InvalidResponse(error.to_string()))?;

    Ok(response
        .flows
        .into_iter()
        .map(|flow| MatrixLoginFlow {
            kind: parse_flow_kind(flow.flow_type),
            delegated_oidc_compatibility: flow.delegated_oidc_compatibility,
            display_name: flow.display_name,
        })
        .collect())
}

pub fn map_login_flows_to_desktop(flows: Vec<MatrixLoginFlow>) -> Vec<LoginFlow> {
    flows
        .into_iter()
        .map(|flow| LoginFlow {
            kind: match flow.kind {
                MatrixLoginFlowKind::Password => LoginFlowKind::Password,
                MatrixLoginFlowKind::Sso => LoginFlowKind::Sso,
                MatrixLoginFlowKind::Oidc => LoginFlowKind::Oidc,
                MatrixLoginFlowKind::Token => LoginFlowKind::Token,
                MatrixLoginFlowKind::Unknown(value) => LoginFlowKind::Unknown(value),
            },
            delegated_oidc_compatibility: flow.delegated_oidc_compatibility,
            display_name: flow.display_name,
        })
        .collect()
}

fn parse_flow_kind(flow_type: String) -> MatrixLoginFlowKind {
    match flow_type.as_str() {
        "m.login.password" => MatrixLoginFlowKind::Password,
        "m.login.sso" => MatrixLoginFlowKind::Sso,
        "m.login.oidc" | "m.login.oauth2" => MatrixLoginFlowKind::Oidc,
        "m.login.token" => MatrixLoginFlowKind::Token,
        _ => MatrixLoginFlowKind::Unknown(flow_type),
    }
}

fn is_loopback_homeserver(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };

    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn matrix_error_message(body: &str) -> String {
    serde_json::from_str::<MatrixErrorResponse>(body)
        .ok()
        .and_then(|response| response.error)
        .filter(|message| !message.trim().is_empty())
        .unwrap_or_else(|| "homeserver did not return login flows".to_owned())
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, path::PathBuf};

    use super::{
        LOCAL_USER_ALIASES_ACCOUNT_DATA_TYPE, MatrixLocalUserAliases,
        MatrixPublicRoomDirectoryQuery, MatrixPublicRoomDirectoryRoom, MatrixRoomHistoryVisibility,
        MatrixRoomJoinRule, MatrixRoomMemberRole, MatrixRoomModerationAction,
        MatrixRoomPermissionFacts, MatrixRoomSettingChange, MatrixRoomSettingsSnapshot,
        MatrixRoomTagInfo, MatrixRoomTags, MatrixSearchIndexKey, MatrixSearchIndexStoreConfig,
        create_public_directory_room, get_room_settings_snapshot, join_room_by_alias,
        matrix_room_list_room_from_counts, matrix_room_member_role, moderate_room_member,
        normalized_local_user_aliases, query_public_room_directory,
        room_settings_snapshot_with_change, room_settings_snapshot_with_member_power_level,
        update_room_member_power_level, update_room_setting,
    };

    #[test]
    fn client_builder_defaults_enable_thread_subscriptions() {
        let source = include_str!("lib.rs");
        let defaults_body = source
            .split("fn desktop_client_builder_defaults")
            .nth(1)
            .expect("desktop client builder defaults helper should exist")
            .split("pub async fn recover_e2ee")
            .next()
            .expect("recover_e2ee should follow desktop client builder defaults");

        assert!(defaults_body.contains("with_threading_support"));
        assert!(defaults_body.contains("ThreadingSupport::Enabled"));
        assert!(defaults_body.contains("with_subscriptions: true"));
    }

    #[test]
    fn search_index_store_config_uses_encrypted_ngram_index() {
        let config = MatrixSearchIndexStoreConfig::new(
            PathBuf::from("search-index"),
            MatrixSearchIndexKey::new("synthetic-search-key"),
        );

        let kind = config.as_sdk_store_kind();

        assert!(matches!(
            kind,
            matrix_sdk::search_index::SearchIndexStoreKind::EncryptedDirectoryWithConfig(_, _, _)
        ));
    }

    #[test]
    fn local_user_aliases_account_data_serde_uses_private_flat_map() {
        let mut aliases = BTreeMap::new();
        aliases.insert(
            "@alice:example.invalid".to_owned(),
            "Local Alice".to_owned(),
        );
        let content = MatrixLocalUserAliases {
            aliases: aliases.clone(),
        };

        let value = serde_json::to_value(&content).expect("serialize local aliases");
        assert_eq!(
            LOCAL_USER_ALIASES_ACCOUNT_DATA_TYPE,
            "app.ruri.local_aliases"
        );
        assert_eq!(
            value["aliases"]["@alice:example.invalid"],
            serde_json::json!("Local Alice")
        );

        let parsed: MatrixLocalUserAliases =
            serde_json::from_value(value).expect("deserialize local aliases");
        assert_eq!(parsed.aliases, aliases);
    }

    #[test]
    fn local_user_aliases_debug_redacts_user_ids_and_aliases() {
        let content = MatrixLocalUserAliases {
            aliases: BTreeMap::from([(
                "@alice:example.invalid".to_owned(),
                "Local Alice".to_owned(),
            )]),
        };

        let debug = format!("{content:?}");

        assert!(debug.contains("MatrixLocalUserAliases"));
        assert!(debug.contains("alias_count"));
        assert!(!debug.contains("@alice:example.invalid"));
        assert!(!debug.contains("Local Alice"));
    }

    #[test]
    fn normalized_local_user_aliases_trims_and_drops_empty_entries() {
        let aliases = BTreeMap::from([
            (
                "@alice:example.invalid".to_owned(),
                "  Local Alice  ".to_owned(),
            ),
            ("@bob:example.invalid".to_owned(), "   ".to_owned()),
        ]);

        let normalized = normalized_local_user_aliases(aliases);

        assert_eq!(
            normalized,
            BTreeMap::from([(
                "@alice:example.invalid".to_owned(),
                "Local Alice".to_owned()
            )])
        );
    }

    #[test]
    fn room_list_room_from_counts_carries_notification_metadata() {
        let room = matrix_room_list_room_from_counts(
            "!room:example.invalid".to_owned(),
            "Room".to_owned(),
            None,
            true,
            vec!["@alice:example.invalid".to_owned()],
            MatrixRoomTags::default(),
            4,
            2,
            4,
            false,
            0,
            vec!["!space:example.invalid".to_owned()],
        );

        assert_eq!(room.notification_count, 4);
        assert_eq!(room.highlight_count, 2);
        assert_eq!(room.unread_count, 4);
        assert!(room.is_dm);
    }

    #[test]
    fn room_list_room_from_counts_carries_room_tags() {
        let tags = MatrixRoomTags {
            favourite: Some(MatrixRoomTagInfo {
                order: Some("0.25".to_owned()),
            }),
            low_priority: None,
        };

        let room = matrix_room_list_room_from_counts(
            "!room:example.invalid".to_owned(),
            "Room".to_owned(),
            None,
            false,
            Vec::new(),
            tags.clone(),
            0,
            0,
            0,
            false,
            0,
            vec![],
        );

        assert_eq!(room.tags, tags);
    }

    #[test]
    fn room_tag_operations_use_sdk_tag_methods() {
        let source = include_str!("lib.rs");

        assert!(source.contains("set_is_favourite(true"));
        assert!(source.contains("set_is_favourite(false"));
        assert!(source.contains("set_is_low_priority(true"));
        assert!(source.contains("set_is_low_priority(false"));
    }

    #[test]
    fn pin_operations_use_sdk_pinned_event_methods() {
        let source = include_str!("lib.rs");
        let pin_body = source
            .split("pub async fn pin_event")
            .nth(1)
            .expect("pin_event wrapper")
            .split("pub async fn unpin_event")
            .next()
            .expect("pin_event body");
        let unpin_body = source
            .split("pub async fn unpin_event")
            .nth(1)
            .expect("unpin_event wrapper")
            .split("pub async fn edit_text_message")
            .next()
            .expect("unpin_event body");

        assert!(pin_body.contains(".pin_event(&event_id)"));
        assert!(unpin_body.contains(".unpin_event(&event_id)"));
    }

    #[test]
    fn directory_operations_use_public_room_and_alias_join_apis() {
        let _query = MatrixPublicRoomDirectoryQuery {
            term: Some("synthetic".to_owned()),
            server_name: Some("example.invalid".to_owned()),
            limit: Some(10),
            since: None,
        };
        let _room = MatrixPublicRoomDirectoryRoom {
            room_id: "!room:example.invalid".to_owned(),
            canonical_alias: Some("#room:example.invalid".to_owned()),
            name: "Synthetic Room".to_owned(),
            topic: None,
            avatar_url: None,
            joined_members: 1,
            world_readable: true,
            guest_can_join: false,
        };
        let _query_fn = query_public_room_directory;
        let _join_fn = join_room_by_alias;
        let _create_public_fn = create_public_directory_room;
    }

    #[test]
    fn room_management_wrappers_use_settings_privacy_and_moderation_apis() {
        let _snapshot = MatrixRoomSettingsSnapshot {
            room_id: "!room:example.invalid".to_owned(),
            name: Some("Synthetic Room".to_owned()),
            topic: Some("Synthetic topic".to_owned()),
            avatar_url: None,
            join_rule: MatrixRoomJoinRule::Invite,
            history_visibility: MatrixRoomHistoryVisibility::Shared,
            permissions: MatrixRoomPermissionFacts {
                can_edit_settings: true,
                can_edit_roles: true,
                can_kick: true,
                can_ban: true,
                can_unban: false,
            },
            members: vec![super::MatrixRoomMemberSummary {
                user_id: "@member:example.invalid".to_owned(),
                display_name: Some("Synthetic Member".to_owned()),
                avatar_url: None,
                power_level: Some(50),
                role: MatrixRoomMemberRole::Moderator,
            }],
        };
        let _change = MatrixRoomSettingChange::JoinRule(MatrixRoomJoinRule::Public);
        let _moderation = MatrixRoomModerationAction::Kick;
        let _snapshot_fn = get_room_settings_snapshot;
        let _update_fn = update_room_setting;
        let _moderate_fn = moderate_room_member;
        let _role_fn = update_room_member_power_level;

        let source = include_str!("lib.rs");
        assert!(source.contains(".set_name("));
        assert!(source.contains(".set_room_topic("));
        assert!(source.contains(".set_avatar_url("));
        assert!(source.contains(".remove_avatar("));
        assert!(source.contains(".privacy_settings()"));
        assert!(source.contains(".update_join_rule("));
        assert!(source.contains(".update_room_history_visibility("));
        assert!(source.contains(".kick_user("));
        assert!(source.contains(".ban_user("));
        assert!(source.contains(".unban_user("));
        assert!(source.contains(".update_power_levels("));
    }

    #[test]
    fn room_setting_update_projects_the_sent_change_into_the_success_snapshot() {
        let original = MatrixRoomSettingsSnapshot {
            room_id: "!room:example.invalid".to_owned(),
            name: Some("Original Room".to_owned()),
            topic: Some("Original topic".to_owned()),
            avatar_url: Some("mxc://example.invalid/original".to_owned()),
            join_rule: MatrixRoomJoinRule::Invite,
            history_visibility: MatrixRoomHistoryVisibility::Shared,
            permissions: MatrixRoomPermissionFacts {
                can_edit_settings: true,
                can_edit_roles: true,
                can_kick: true,
                can_ban: true,
                can_unban: true,
            },
            members: vec![],
        };

        assert_eq!(
            room_settings_snapshot_with_change(
                original.clone(),
                &MatrixRoomSettingChange::Topic(Some("Updated topic".to_owned())),
            )
            .topic
            .as_deref(),
            Some("Updated topic")
        );
        assert_eq!(
            room_settings_snapshot_with_change(
                original.clone(),
                &MatrixRoomSettingChange::Name(None),
            )
            .name,
            None
        );
        assert_eq!(
            room_settings_snapshot_with_change(
                original.clone(),
                &MatrixRoomSettingChange::AvatarUrl(None),
            )
            .avatar_url,
            None
        );
        assert_eq!(
            room_settings_snapshot_with_change(
                original.clone(),
                &MatrixRoomSettingChange::JoinRule(MatrixRoomJoinRule::Public),
            )
            .join_rule,
            MatrixRoomJoinRule::Public
        );
        assert_eq!(
            room_settings_snapshot_with_change(
                original,
                &MatrixRoomSettingChange::HistoryVisibility(MatrixRoomHistoryVisibility::Joined,),
            )
            .history_visibility,
            MatrixRoomHistoryVisibility::Joined
        );
    }

    #[test]
    fn room_member_power_level_projection_updates_role_in_success_snapshot() {
        let original = MatrixRoomSettingsSnapshot {
            room_id: "!room:example.invalid".to_owned(),
            name: Some("Original Room".to_owned()),
            topic: Some("Original topic".to_owned()),
            avatar_url: None,
            join_rule: MatrixRoomJoinRule::Invite,
            history_visibility: MatrixRoomHistoryVisibility::Shared,
            permissions: MatrixRoomPermissionFacts {
                can_edit_settings: true,
                can_edit_roles: true,
                can_kick: true,
                can_ban: true,
                can_unban: true,
            },
            members: vec![super::MatrixRoomMemberSummary {
                user_id: "@member:example.invalid".to_owned(),
                display_name: Some("Synthetic Member".to_owned()),
                avatar_url: None,
                power_level: Some(0),
                role: MatrixRoomMemberRole::User,
            }],
        };

        let updated =
            room_settings_snapshot_with_member_power_level(original, "@member:example.invalid", 50);
        let member = updated.members.first().expect("member summary");
        assert_eq!(member.power_level, Some(50));
        assert_eq!(member.role, MatrixRoomMemberRole::Moderator);
        assert_eq!(
            matrix_room_member_role(Some(100)),
            MatrixRoomMemberRole::Administrator
        );
        assert_eq!(matrix_room_member_role(None), MatrixRoomMemberRole::Creator);
    }
}
