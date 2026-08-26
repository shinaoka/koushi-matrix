use crate::MatrixClientStoreConfig;
use matrix_sdk::SqliteCryptoStore;
use matrix_sdk_base::crypto::store::CryptoStore;
use std::fmt;

/// Closed result of checking a saved crypto store without creating it.
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum SavedCryptoStorePreflight {
    PresentMatching,
    Missing,
    OpenFailed,
    IdentityMismatch,
}

impl SavedCryptoStorePreflight {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PresentMatching => "present_matching",
            Self::Missing => "missing",
            Self::OpenFailed => "open_failed",
            Self::IdentityMismatch => "identity_mismatch",
        }
    }
}

impl fmt::Debug for SavedCryptoStorePreflight {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Closed local/server device-key comparison. Keys and identifiers never leave
/// the SDK boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalServerDeviceKeyComparison {
    Match,
    Mismatch,
    Unknown,
}

#[derive(Clone)]
pub struct SavedCryptoStoreIdentity {
    curve25519: String,
    ed25519: String,
}

impl fmt::Debug for SavedCryptoStoreIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SavedCryptoStoreIdentity(..)")
    }
}

/// Open a saved crypto database only when its exact database file already
/// exists, load its Olm account, and close it before returning.
pub async fn preflight_saved_crypto_store(
    config: &MatrixClientStoreConfig,
    expected_user_id: Option<&str>,
    expected_device_id: Option<&str>,
) -> SavedCryptoStorePreflight {
    match load_saved_crypto_store_identity(config, expected_user_id, expected_device_id).await {
        Ok(_) => SavedCryptoStorePreflight::PresentMatching,
        Err(outcome) => outcome,
    }
}

pub(crate) async fn load_saved_crypto_store_identity(
    config: &MatrixClientStoreConfig,
    expected_user_id: Option<&str>,
    expected_device_id: Option<&str>,
) -> Result<SavedCryptoStoreIdentity, SavedCryptoStorePreflight> {
    if !config.crypto_database_path().is_file() {
        return Err(SavedCryptoStorePreflight::Missing);
    }
    let store = SqliteCryptoStore::open_with_key(config.path(), Some(config.sdk_store_key()))
        .await
        .map_err(|_| SavedCryptoStorePreflight::OpenFailed)?;
    let account = match store.load_account().await {
        Ok(Some(account)) => account,
        Ok(None) => {
            let _ = close_store(&store).await;
            return Err(SavedCryptoStorePreflight::IdentityMismatch);
        }
        Err(_) => {
            let _ = close_store(&store).await;
            return Err(SavedCryptoStorePreflight::OpenFailed);
        }
    };
    if !expected_user_id.is_none_or(|expected| account.user_id().as_str() == expected)
        || !expected_device_id.is_none_or(|expected| account.device_id().as_str() == expected)
    {
        let _ = close_store(&store).await;
        return Err(SavedCryptoStorePreflight::IdentityMismatch);
    }
    let keys = account.identity_keys();
    let identity = SavedCryptoStoreIdentity {
        curve25519: keys.curve25519.to_base64(),
        ed25519: keys.ed25519.to_base64(),
    };
    close_store(&store)
        .await
        .map_err(|_| SavedCryptoStorePreflight::OpenFailed)?;
    Ok(identity)
}

async fn close_store(store: &SqliteCryptoStore) -> Result<(), ()> {
    CryptoStore::close(store).await.map_err(|_| ())
}

/// Compare the SDK's local own-device decision with the server-advertised
/// device returned by the same client. A missing response is unknown, never a
/// match.
pub(crate) fn compare_local_server_device_key_values(
    local_curve25519: &str,
    local_ed25519: &str,
    server_curve25519: Option<&str>,
    server_ed25519: Option<&str>,
) -> LocalServerDeviceKeyComparison {
    match (server_curve25519, server_ed25519) {
        (Some(server_curve25519), Some(server_ed25519))
            if server_curve25519 == local_curve25519 && server_ed25519 == local_ed25519 =>
        {
            LocalServerDeviceKeyComparison::Match
        }
        (Some(_), Some(_)) => LocalServerDeviceKeyComparison::Mismatch,
        _ => LocalServerDeviceKeyComparison::Unknown,
    }
}

pub(crate) async fn compare_server_device_keys_with_saved_identity(
    client: &matrix_sdk::Client,
    local: &SavedCryptoStoreIdentity,
) -> LocalServerDeviceKeyComparison {
    compare_device_keys_with_saved_identity(client, local, true).await
}

pub(crate) async fn compare_cached_device_keys_with_saved_identity(
    client: &matrix_sdk::Client,
    local: &SavedCryptoStoreIdentity,
) -> LocalServerDeviceKeyComparison {
    compare_device_keys_with_saved_identity(client, local, false).await
}

async fn compare_device_keys_with_saved_identity(
    client: &matrix_sdk::Client,
    local: &SavedCryptoStoreIdentity,
    refresh_server: bool,
) -> LocalServerDeviceKeyComparison {
    let (Some(user_id), Some(device_id)) = (client.user_id(), client.device_id()) else {
        return LocalServerDeviceKeyComparison::Unknown;
    };
    if refresh_server
        && client
            .encryption()
            .request_user_identity(user_id)
            .await
            .is_err()
    {
        return LocalServerDeviceKeyComparison::Unknown;
    }
    let devices = match client.encryption().get_user_devices(user_id).await {
        Ok(devices) => devices,
        Err(_) => return LocalServerDeviceKeyComparison::Unknown,
    };
    let Some(device) = devices.get(device_id) else {
        return LocalServerDeviceKeyComparison::Unknown;
    };
    compare_local_server_device_key_values(
        &local.curve25519,
        &local.ed25519,
        device
            .curve25519_key()
            .map(|key| key.to_base64())
            .as_deref(),
        device.ed25519_key().map(|key| key.to_base64()).as_deref(),
    )
}
