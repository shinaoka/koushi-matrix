use futures_util::{Stream, StreamExt};
pub use matrix_desktop_state::E2eeRecoveryState;
use matrix_desktop_state::{
    CrossSigningStatus, IdentityResetAuthType, KeyBackupStatus, LoginFlow, LoginFlowKind,
    LoginRequest, RecoveryRequest, RoomAttentionSummary, SessionInfo, room_attention_summary,
};
use matrix_sdk::authentication::matrix::MatrixSession;
use matrix_sdk::room::ParentSpace;
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    future::Future,
    net::IpAddr,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
    time::Duration,
};
use thiserror::Error;
use url::Url;
use zeroize::Zeroizing;

const LOGIN_DISCOVERY_PATH: &str = "_matrix/client/v3/login";
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);
const MATRIX_ROOM_LIST_SNAPSHOT_LIMIT: usize = 4096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoginDiscovery {
    pub homeserver: String,
    pub flows: Vec<LoginFlow>,
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

#[derive(Clone, Eq, Error, PartialEq)]
pub enum E2eeTrustError {
    #[error("Matrix encryption is not initialized")]
    NoOlmMachine,
    #[error("Matrix SDK trust operation failed")]
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
) -> Result<CrossSigningStatus, E2eeTrustError> {
    session
        .client()
        .encryption()
        .bootstrap_cross_signing(None)
        .await?;
    cross_signing_status(session).await
}

pub async fn enable_key_backup(
    session: &MatrixClientSession,
) -> Result<KeyBackupStatus, E2eeTrustError> {
    let encryption = session.client().encryption();
    encryption.recovery().enable_backup().await?;
    Ok(map_backup_state_to_desktop(encryption.backups().state()))
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
    use matrix_desktop_state::{CrossSigningStatus, IdentityResetAuthType, KeyBackupStatus};
    use matrix_sdk::encryption::backups::BackupState;

    use super::{
        E2eeTrustError, MatrixCrossSigningStatus, MatrixIdentityResetAuthType,
        bootstrap_cross_signing, cross_signing_status, enable_key_backup,
        map_backup_state_to_desktop, map_cross_signing_status_to_desktop,
        map_identity_reset_auth_type_to_desktop, reset_identity,
    };

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
    fn e2ee_trust_public_async_api_is_exposed() {
        let _ = cross_signing_status;
        let _ = bootstrap_cross_signing;
        let _ = enable_key_backup;
        let _ = reset_identity;
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixRoomListRoom {
    pub room_id: String,
    pub display_name: String,
    pub is_dm: bool,
    pub unread_count: u64,
    pub notification_count: u64,
    pub highlight_count: u64,
    pub parent_space_ids: Vec<String>,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MatrixRoomListError {
    #[error("Matrix room list failed")]
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

pub async fn create_room(
    session: &MatrixClientSession,
    name: &str,
) -> Result<String, MatrixRoomOperationError> {
    let mut request = matrix_sdk::ruma::api::client::room::create_room::v3::Request::new();
    request.name = non_empty_name(name);
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
    let room_display_name = room_display_name?;

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
    for room in rooms {
        if room.state() != matrix_sdk::RoomState::Joined {
            continue;
        }

        let room_id = room.room_id().to_string();
        let display_name = room
            .cached_display_name()
            .map(|name| name.to_string())
            .unwrap_or_else(|| room_id.clone());

        if room.is_space() {
            snapshot.spaces.push(MatrixRoomListSpace {
                space_id: room_id,
                display_name,
            });
            continue;
        }

        let unread_notifications = room.unread_notification_counts();
        let notification_count = unread_notifications.notification_count.into();
        let highlight_count = unread_notifications.highlight_count.into();
        let unread_count = room_attention_unread_count(
            notification_count,
            room.num_unread_messages(),
            room.is_marked_unread(),
        );

        let parent_space_ids = matrix_parent_space_ids(&room).await;

        snapshot.rooms.push(matrix_room_list_room_from_counts(
            room_id,
            display_name,
            room.is_dm(),
            notification_count,
            highlight_count,
            unread_count,
            parent_space_ids,
        ));
    }
    snapshot
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
    is_dm: bool,
    notification_count: u64,
    highlight_count: u64,
    unread_count: u64,
    parent_space_ids: Vec<String>,
) -> MatrixRoomListRoom {
    MatrixRoomListRoom {
        room_id,
        display_name,
        is_dm,
        unread_count,
        notification_count,
        highlight_count,
        parent_space_ids,
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
    if !value.get("flows").is_some_and(serde_json::Value::is_array) {
        return Err(LoginDiscoveryError::MissingFlows);
    }

    let response = serde_json::from_value::<LoginDiscoveryResponse>(value.clone())
        .map_err(|error| LoginDiscoveryError::InvalidResponse(error.to_string()))?;

    Ok(response
        .flows
        .into_iter()
        .map(|flow| LoginFlow {
            kind: parse_flow_kind(flow.flow_type),
            delegated_oidc_compatibility: flow.delegated_oidc_compatibility,
        })
        .collect())
}

fn parse_flow_kind(flow_type: String) -> LoginFlowKind {
    match flow_type.as_str() {
        "m.login.password" => LoginFlowKind::Password,
        "m.login.sso" => LoginFlowKind::Sso,
        "m.login.token" => LoginFlowKind::Token,
        _ => LoginFlowKind::Unknown(flow_type),
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
    use std::path::PathBuf;

    use super::{
        MatrixSearchIndexKey, MatrixSearchIndexStoreConfig, matrix_room_list_room_from_counts,
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
    fn room_list_room_from_counts_carries_notification_metadata() {
        let room = matrix_room_list_room_from_counts(
            "!room:example.invalid".to_owned(),
            "Room".to_owned(),
            true,
            4,
            2,
            4,
            vec!["!space:example.invalid".to_owned()],
        );

        assert_eq!(room.notification_count, 4);
        assert_eq!(room.highlight_count, 2);
        assert_eq!(room.unread_count, 4);
        assert!(room.is_dm);
    }
}
