use crate::e2ee::{
    DESKTOP_SQLITE_STORE_POOL_MAX_SIZE, desktop_room_key_recipient_strategy,
    install_room_key_diagnostic_observer,
};
use crate::{Homeserver, MatrixSearchIndexStoreConfig, PasswordLoginError};
use koushi_state::{DeviceCleanupAuthMode, SessionInfo};
use matrix_sdk::{
    authentication::{
        matrix::MatrixSession,
        oauth::{
            ClientId, ClientRegistrationData, OAuthSession, UserSession,
            registration::{ApplicationType, ClientMetadata, Localized, OAuthGrantType},
        },
    },
    cross_process_lock::CrossProcessLockConfig,
    encryption::{BackupDownloadStrategy, EncryptionSettings},
    ruma::serde::Raw,
};
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};
use url::Url;
use zeroize::Zeroizing;

#[derive(Clone)]
pub struct MatrixClientStoreConfig {
    path: PathBuf,
    cache_path: Option<PathBuf>,
    key: MatrixClientStoreKey,
    search_index_store: Option<MatrixSearchIndexStoreConfig>,
    /// #1134: whether this process is the only owner of `path`.
    exclusive_store_root: bool,
}

impl MatrixClientStoreConfig {
    pub fn new(path: impl Into<PathBuf>, key: MatrixClientStoreKey) -> Self {
        Self {
            path: path.into(),
            cache_path: None,
            key,
            search_index_store: None,
            // The SDK's multi-process store coordination stays on by default:
            // every entry point that can be pointed at a shared store root keeps
            // its cross-process locks.
            exclusive_store_root: false,
        }
    }

    /// Declare that this process is the only owner of `path`.
    ///
    /// #1134: only the packaged desktop app can promise that — the shell refuses
    /// a second instance for its identifier, each account has its own store
    /// directory, and the user cannot point two processes at one store. In that
    /// case the SDK's cross-process locks are switched off, because their
    /// multiprocess mode holds a crypto-store lease for the whole encryption-sync
    /// stream and renews it every 50 ms with a store transaction, which never
    /// stops while the session is idle.
    ///
    /// Anything that accepts a reusable store root (QA and smoke entry points,
    /// tests) must stay on the default.
    pub fn with_exclusive_store_root(mut self) -> Self {
        self.exclusive_store_root = true;
        self
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

    pub(crate) fn sdk_store_key(&self) -> &[u8; 32] {
        self.key.expose_key()
    }

    pub(crate) fn crypto_database_path(&self) -> PathBuf {
        self.path.join("matrix-sdk-crypto.sqlite3")
    }

    /// The store is keyed by construction: `MatrixClientStoreConfig::new`
    /// requires a [`MatrixClientStoreKey`], and `apply_to_builder` always
    /// passes that key into the SQLite store config.
    pub fn encrypted_at_rest_configured(&self) -> bool {
        true
    }

    fn apply_to_builder(&self, builder: matrix_sdk::ClientBuilder) -> matrix_sdk::ClientBuilder {
        let sqlite_config = matrix_sdk::SqliteStoreConfig::new(&self.path)
            .pool_max_size(DESKTOP_SQLITE_STORE_POOL_MAX_SIZE)
            .key(Some(self.key.expose_key()));
        let builder = builder
            .sqlite_store_with_config_and_cache_path(sqlite_config, self.cache_path.as_deref());
        let builder = if self.exclusive_store_root {
            builder.cross_process_store_config(CrossProcessLockConfig::SingleProcess)
        } else {
            builder
        };
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
pub struct MatrixClientSession {
    pub(super) client: matrix_sdk::Client,
    pub info: SessionInfo,
    pub(super) diagnostic_counters: Arc<koushi_diagnostics::DiagnosticCounterContext>,
}

/// Coarse result of the authenticated MSC4186 invite-list contract probe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixSlidingSyncInviteListSupport {
    Supported,
    KnownIncomplete,
    Unknown,
}

impl MatrixClientSession {
    #[cfg(any(test, feature = "test-hooks"))]
    #[doc(hidden)]
    pub fn from_client_for_testing(client: matrix_sdk::Client, info: SessionInfo) -> Self {
        Self {
            client,
            info,
            diagnostic_counters: koushi_diagnostics::DiagnosticCounterContext::registered(),
        }
    }
    pub fn client(&self) -> matrix_sdk::Client {
        self.client.clone()
    }
    pub fn diagnostic_counter_snapshot(&self) -> koushi_diagnostics::DiagnosticSnapshot {
        self.diagnostic_counters.snapshot()
    }
    pub fn device_cleanup_auth_mode(&self) -> DeviceCleanupAuthMode {
        if self.client.oauth().full_session().is_some() {
            DeviceCleanupAuthMode::OAuth
        } else {
            DeviceCleanupAuthMode::Legacy
        }
    }
    pub fn persistable_session(&self) -> Result<PersistableMatrixSession, PasswordLoginError> {
        if let Some(oauth_session) = self.client.oauth().full_session() {
            return Ok(PersistableMatrixSession {
                info: self.info.clone(),
                session: PersistableSessionKind::OAuth {
                    user_session: oauth_session.user,
                    client_id: oauth_session.client_id,
                },
                sliding_sync_positive_evidence: None,
            });
        }

        let session = self
            .client
            .matrix_auth()
            .session()
            .ok_or(PasswordLoginError::MissingSession)?;
        Ok(PersistableMatrixSession {
            info: self.info.clone(),
            session: PersistableSessionKind::Matrix(session),
            sliding_sync_positive_evidence: None,
        })
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixEventCacheStatus {
    AlreadyEnabled,
    Enabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("Matrix event cache subscription failed")]
pub enum MatrixEventCacheError {
    SubscribeFailed,
}

#[derive(Clone)]
pub struct PersistableMatrixSession {
    pub info: SessionInfo,
    session: PersistableSessionKind,
    sliding_sync_positive_evidence: Option<koushi_state::SlidingSyncPositiveEvidence>,
}

#[derive(Clone)]
enum PersistableSessionKind {
    Matrix(MatrixSession),
    OAuth {
        user_session: UserSession,
        client_id: ClientId,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersistableAuthKind {
    Password,
    OAuth,
}

impl PersistableMatrixSession {
    pub fn to_json(&self) -> Result<String, PasswordLoginError> {
        match &self.session {
            PersistableSessionKind::Matrix(session) => {
                serde_json::to_string(&SerializedTaggedMatrixSession {
                    auth_kind: PersistableSessionJsonKind::Password,
                    homeserver: self.info.homeserver.clone(),
                    authentication_method: self.info.authentication_method,
                    sliding_sync_positive_evidence: self.sliding_sync_positive_evidence.clone(),
                    session: session.clone(),
                })
                .map_err(|error| PasswordLoginError::Serialization(error.to_string()))
            }
            PersistableSessionKind::OAuth {
                user_session,
                client_id,
            } => serde_json::to_string(&SerializedOauthPersistableMatrixSession {
                auth_kind: PersistableSessionJsonKind::OAuth,
                homeserver: self.info.homeserver.clone(),
                sliding_sync_positive_evidence: self.sliding_sync_positive_evidence.clone(),
                user_session: user_session.clone(),
                client_id: client_id.clone(),
            })
            .map_err(|error| PasswordLoginError::Serialization(error.to_string())),
        }
    }

    pub fn from_json(value: &str) -> Result<Self, PasswordLoginError> {
        let value_json = serde_json::from_str::<serde_json::Value>(value)
            .map_err(|error| PasswordLoginError::Serialization(error.to_string()))?;
        if value_json
            .get("auth_kind")
            .and_then(serde_json::Value::as_str)
            == Some("oauth")
        {
            let serialized =
                serde_json::from_value::<SerializedOauthPersistableMatrixSession>(value_json)
                    .map_err(|error| PasswordLoginError::Serialization(error.to_string()))?;
            let info = SessionInfo {
                homeserver: serialized.homeserver,
                user_id: serialized.user_session.meta.user_id.to_string(),
                device_id: serialized.user_session.meta.device_id.to_string(),
                authentication_method: koushi_state::SessionAuthenticationMethod::OAuth,
            };
            return Ok(Self {
                info,
                session: PersistableSessionKind::OAuth {
                    user_session: serialized.user_session,
                    client_id: serialized.client_id,
                },
                sliding_sync_positive_evidence: serialized.sliding_sync_positive_evidence,
            });
        }

        let serialized = serde_json::from_value::<SerializedPersistableMatrixSession>(value_json)
            .map_err(|error| PasswordLoginError::Serialization(error.to_string()))?;
        let session = serialized.session;
        let info = SessionInfo {
            homeserver: serialized.homeserver,
            user_id: session.meta.user_id.to_string(),
            device_id: session.meta.device_id.to_string(),
            authentication_method: serialized.authentication_method,
        };
        Ok(Self {
            info,
            session: PersistableSessionKind::Matrix(session),
            sliding_sync_positive_evidence: serialized.sliding_sync_positive_evidence,
        })
    }

    pub fn sliding_sync_positive_evidence(
        &self,
    ) -> Option<koushi_state::SlidingSyncPositiveEvidence> {
        self.sliding_sync_positive_evidence.clone()
    }

    pub fn with_sliding_sync_positive_evidence(
        mut self,
        evidence: koushi_state::SlidingSyncPositiveEvidence,
    ) -> Self {
        self.sliding_sync_positive_evidence = Some(evidence);
        self
    }

    pub fn matrix_session(&self) -> Option<MatrixSession> {
        match &self.session {
            PersistableSessionKind::Matrix(session) => Some(session.clone()),
            PersistableSessionKind::OAuth { .. } => None,
        }
    }

    pub fn oauth_session(&self) -> Option<OAuthSession> {
        match &self.session {
            PersistableSessionKind::Matrix(_) => None,
            PersistableSessionKind::OAuth {
                user_session,
                client_id,
            } => Some(OAuthSession {
                user: user_session.clone(),
                client_id: client_id.clone(),
            }),
        }
    }

    pub fn auth_kind(&self) -> PersistableAuthKind {
        match &self.session {
            PersistableSessionKind::Matrix(_) => PersistableAuthKind::Password,
            PersistableSessionKind::OAuth { .. } => PersistableAuthKind::OAuth,
        }
    }
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum PersistableSessionJsonKind {
    Password,
    #[serde(rename = "oauth")]
    OAuth,
}

#[derive(Deserialize, Serialize)]
struct SerializedPersistableMatrixSession {
    homeserver: String,
    #[serde(default)]
    authentication_method: koushi_state::SessionAuthenticationMethod,
    #[serde(default)]
    sliding_sync_positive_evidence: Option<koushi_state::SlidingSyncPositiveEvidence>,
    #[serde(flatten)]
    session: MatrixSession,
}

#[derive(Serialize)]
struct SerializedTaggedMatrixSession {
    auth_kind: PersistableSessionJsonKind,
    homeserver: String,
    authentication_method: koushi_state::SessionAuthenticationMethod,
    #[serde(skip_serializing_if = "Option::is_none")]
    sliding_sync_positive_evidence: Option<koushi_state::SlidingSyncPositiveEvidence>,
    #[serde(flatten)]
    session: MatrixSession,
}

#[derive(Deserialize, Serialize)]
struct SerializedOauthPersistableMatrixSession {
    auth_kind: PersistableSessionJsonKind,
    homeserver: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sliding_sync_positive_evidence: Option<koushi_state::SlidingSyncPositiveEvidence>,
    user_session: UserSession,
    client_id: ClientId,
}

impl std::fmt::Debug for PersistableMatrixSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PersistableMatrixSession")
            .field("info", &self.info)
            .field("auth_kind", &self.auth_kind())
            .field("session", &"MatrixSession(..)")
            .finish()
    }
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

pub async fn restore_session(
    session: &PersistableMatrixSession,
) -> Result<MatrixClientSession, PasswordLoginError> {
    restore_session_with_store(session, None).await
}

pub(super) fn oidc_client_registration_data(redirect_uri: Url) -> ClientRegistrationData {
    let client_uri = Localized::new(
        Url::parse("https://github.com/shinaoka/koushi-matrix")
            .expect("static client URI should parse"),
        [],
    );
    let metadata = ClientMetadata {
        client_name: Some(Localized::new("Koushi".to_owned(), [])),
        policy_uri: Some(client_uri.clone()),
        tos_uri: Some(client_uri.clone()),
        ..ClientMetadata::new(
            ApplicationType::Native,
            vec![OAuthGrantType::AuthorizationCode {
                redirect_uris: vec![redirect_uri],
            }],
            client_uri,
        )
    };

    ClientRegistrationData::new(Raw::new(&metadata).expect("OIDC client metadata should serialize"))
}

pub async fn restore_session_with_store(
    session: &PersistableMatrixSession,
    store_config: Option<&MatrixClientStoreConfig>,
) -> Result<MatrixClientSession, PasswordLoginError> {
    let homeserver = Homeserver::parse(&session.info.homeserver)?;
    let client = build_client(&homeserver, store_config).await?;

    if let Some(oauth_session) = session.oauth_session() {
        client
            .restore_session(oauth_session)
            .await
            .map_err(|error| PasswordLoginError::Sdk(error.to_string()))?;
    } else if let Some(matrix_session) = session.matrix_session() {
        client
            .restore_session(matrix_session)
            .await
            .map_err(|error| PasswordLoginError::Sdk(error.to_string()))?;
    } else {
        return Err(PasswordLoginError::MissingSession);
    }
    client
        .send_queue()
        .require_secure_backup_for_encrypted_sends(false);
    let diagnostic_counters = install_room_key_diagnostic_observer(&client).await;

    Ok(MatrixClientSession {
        client,
        diagnostic_counters,
        info: session.info.clone(),
    })
}

pub async fn restore_session_with_verified_store(
    session: &PersistableMatrixSession,
    store_config: &MatrixClientStoreConfig,
) -> Result<MatrixClientSession, PasswordLoginError> {
    let identity = crate::login_store::load_saved_crypto_store_identity(
        store_config,
        Some(&session.info.user_id),
        Some(&session.info.device_id),
    )
    .await
    .map_err(PasswordLoginError::SavedCryptoStore)?;
    let restored = restore_session_with_store(session, Some(store_config)).await?;
    if crate::login_store::compare_cached_device_keys_with_saved_identity(
        &restored.client,
        &identity,
    )
    .await
        != crate::LocalServerDeviceKeyComparison::Match
    {
        return Err(PasswordLoginError::SavedCryptoStore(
            crate::SavedCryptoStorePreflight::IdentityMismatch,
        ));
    }
    Ok(restored)
}

pub async fn enable_event_cache(
    session: &MatrixClientSession,
) -> Result<MatrixEventCacheStatus, MatrixEventCacheError> {
    let client = session.client();
    let event_cache = client.event_cache();
    if event_cache.has_subscribed() {
        return Ok(MatrixEventCacheStatus::AlreadyEnabled);
    }

    event_cache
        .subscribe()
        .map_err(|_| MatrixEventCacheError::SubscribeFailed)?;
    Ok(MatrixEventCacheStatus::Enabled)
}

pub(super) async fn build_client(
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

pub(super) fn desktop_client_builder_defaults(
    builder: matrix_sdk::ClientBuilder,
) -> matrix_sdk::ClientBuilder {
    builder
        .handle_refresh_tokens()
        .with_room_key_recipient_strategy(desktop_room_key_recipient_strategy())
        .with_encryption_settings(EncryptionSettings {
            backup_download_strategy: BackupDownloadStrategy::AfterDecryptionFailure,
            ..Default::default()
        })
        .with_enable_share_history_on_invite(true)
        .with_threading_support(matrix_sdk::ThreadingSupport::Enabled {
            with_subscriptions: true,
        })
}

pub async fn logout(session: &MatrixClientSession) -> Result<(), PasswordLoginError> {
    let client = session.client();
    if client.oauth().full_session().is_some() {
        return client
            .oauth()
            .logout()
            .await
            .map_err(|error| PasswordLoginError::Sdk(error.to_string()));
    }

    client
        .logout()
        .await
        .map_err(|error| PasswordLoginError::Sdk(error.to_string()))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("provisional encryption sync failed")]
pub enum ProvisionalEncryptionSyncError {
    Sdk,
}

#[cfg(test)]
mod tests {
    use super::MatrixEventCacheError;
    use matrix_sdk::cross_process_lock::CrossProcessLockConfig;

    #[test]
    fn matrix_client_store_config_uses_the_required_key_for_sqlite_builder() {
        let config = crate::MatrixClientStoreConfig::new(
            "/tmp/example-store",
            crate::MatrixClientStoreKey::new([7; 32]),
        );
        assert!(config.encrypted_at_rest_configured());
    }
    /// #1134: an exclusive store root switches the SDK's cross-process locks off,
    /// and every other store root keeps them on.
    #[tokio::test]
    async fn only_an_exclusive_store_root_disables_the_cross_process_locks() {
        let exclusive = crate::MatrixClientStoreConfig::new(
            tempfile::tempdir().expect("store directory").path(),
            crate::MatrixClientStoreKey::new([7; 32]),
        )
        .with_exclusive_store_root();
        let client = exclusive
            .apply_to_builder(super::desktop_client_builder_defaults(
                matrix_sdk::Client::builder(),
            ))
            .homeserver_url("https://example.invalid")
            .build()
            .await
            .expect("client builds");
        assert!(
            matches!(
                client.cross_process_lock_config(),
                CrossProcessLockConfig::SingleProcess
            ),
            "an exclusive store root must not enable the multi-process store lock"
        );

        let shared = crate::MatrixClientStoreConfig::new(
            tempfile::tempdir().expect("store directory").path(),
            crate::MatrixClientStoreKey::new([8; 32]),
        );
        let shared_client = shared
            .apply_to_builder(super::desktop_client_builder_defaults(
                matrix_sdk::Client::builder(),
            ))
            .homeserver_url("https://example.invalid")
            .build()
            .await
            .expect("client builds");
        assert!(
            matches!(
                shared_client.cross_process_lock_config(),
                CrossProcessLockConfig::MultiProcess { .. }
            ),
            "a shared store root must keep the SDK's cross-process coordination"
        );
    }

    #[test]
    fn event_cache_error_is_private_data_free() {
        let error = MatrixEventCacheError::SubscribeFailed;

        assert_eq!(error.to_string(), "Matrix event cache subscription failed");
        assert_eq!(format!("{error:?}"), "SubscribeFailed");
    }
}
