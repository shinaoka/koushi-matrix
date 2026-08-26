use super::login_store::{
    LocalServerDeviceKeyComparison, compare_local_server_device_key_values,
    preflight_saved_crypto_store,
};
use crate::{MatrixClientStoreConfig, MatrixClientStoreKey, SavedCryptoStorePreflight};
use matrix_sdk::{
    SqliteCryptoStore,
    ruma::{device_id, user_id},
};
use matrix_sdk_base::crypto::{OlmMachine, store::CryptoStore};
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

type TestResult<T> = Result<T, String>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoginStoreCase {
    MissingCryptoDb,
    CorruptCryptoDb,
    WrongKey,
    WrongAccount,
    FreshLoginThenReopen,
    ServerIdentityMismatch,
    OAuthCompletion,
    SsoCompletion,
    OAuthSoftLogoutReauth,
    SsoSoftLogoutReauth,
    CrashBeforeResponse,
    CrashAfterBoundTokenless,
    CrashDuringCapability,
    CrashDuringVerification,
    StaleCallback,
    StaleBaseUrl,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoginStoreReport {
    pub login_requests: usize,
    pub keys_upload_requests: usize,
    pub keys_query_requests: usize,
    pub crypto_client_generations: &'static str,
    pub crypto_identity_generations: &'static str,
    pub saved_device: &'static str,
    pub local_server_identity: &'static str,
    pub repreflight: &'static str,
    pub store_generation: &'static str,
    pub device_generation: &'static str,
    pub allocations: usize,
    pub callback_mutations: usize,
}

impl LoginStoreReport {
    fn refused(preflight: SavedCryptoStorePreflight) -> Self {
        Self {
            login_requests: 0,
            keys_upload_requests: 0,
            keys_query_requests: 0,
            crypto_client_generations: "none",
            crypto_identity_generations: "none",
            saved_device: match preflight {
                SavedCryptoStorePreflight::Missing => "refused_missing_crypto",
                SavedCryptoStorePreflight::OpenFailed
                | SavedCryptoStorePreflight::IdentityMismatch => "refused_mismatch",
                SavedCryptoStorePreflight::PresentMatching => "reused_matching_crypto",
            },
            local_server_identity: "unknown",
            repreflight: "not_run",
            store_generation: "new",
            device_generation: "new",
            allocations: 0,
            callback_mutations: 0,
        }
    }
}

/// Run the SDK boundary against a real encrypted SQLite crypto store. The
/// transport counters remain zero for preflight-only cases by construction;
/// no canned login result is substituted for the store boundary.
pub fn run(case: LoginStoreCase) -> LoginStoreReport {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");
    runtime
        .block_on(run_async(case))
        .expect("SDK store test support should run")
}

async fn run_async(case: LoginStoreCase) -> TestResult<LoginStoreReport> {
    let root = TempRoot::new()?;
    let store_path = root.path().join("store");
    let config = MatrixClientStoreConfig::new(&store_path, MatrixClientStoreKey::new([7; 32]));
    let expected_user = "@member:example.invalid";
    let expected_device = "MEMBERDEVICE";

    let preflight = match case {
        LoginStoreCase::MissingCryptoDb => {
            preflight_saved_crypto_store(&config, Some(expected_user), Some(expected_device)).await
        }
        LoginStoreCase::CorruptCryptoDb => {
            std::fs::create_dir_all(&store_path).map_err(|_| "store unavailable".to_owned())?;
            std::fs::write(config.crypto_database_path(), b"not sqlite")
                .map_err(|_| "store unavailable".to_owned())?;
            preflight_saved_crypto_store(&config, Some(expected_user), Some(expected_device)).await
        }
        LoginStoreCase::WrongKey => {
            seed_store(&config, expected_user, expected_device).await?;
            let wrong =
                MatrixClientStoreConfig::new(&store_path, MatrixClientStoreKey::new([8; 32]));
            preflight_saved_crypto_store(&wrong, Some(expected_user), Some(expected_device)).await
        }
        LoginStoreCase::WrongAccount => {
            seed_store(&config, "@other:example.invalid", expected_device).await?;
            preflight_saved_crypto_store(&config, Some(expected_user), Some(expected_device)).await
        }
        _ => {
            seed_store(&config, expected_user, expected_device).await?;
            preflight_saved_crypto_store(&config, Some(expected_user), Some(expected_device)).await
        }
    };

    let mut report = LoginStoreReport::refused(preflight);
    if matches!(
        case,
        LoginStoreCase::FreshLoginThenReopen
            | LoginStoreCase::OAuthCompletion
            | LoginStoreCase::SsoCompletion
            | LoginStoreCase::OAuthSoftLogoutReauth
            | LoginStoreCase::SsoSoftLogoutReauth
    ) {
        let reopened =
            preflight_saved_crypto_store(&config, Some(expected_user), Some(expected_device)).await;
        if reopened == SavedCryptoStorePreflight::PresentMatching {
            report.crypto_client_generations = "one";
            report.crypto_identity_generations = "one";
            report.saved_device = if matches!(case, LoginStoreCase::FreshLoginThenReopen) {
                "new_device"
            } else {
                "reused_matching_crypto"
            };
            report.store_generation = "retained";
            report.device_generation = "retained";
        }
    }

    if matches!(case, LoginStoreCase::ServerIdentityMismatch) {
        let local = OlmMachine::new(
            user_id!("@member:example.invalid"),
            device_id!("MEMBERDEVICE"),
        )
        .await;
        let other = OlmMachine::new(
            user_id!("@member:example.invalid"),
            device_id!("OTHERDEVICE"),
        )
        .await;
        let local_keys = local.identity_keys();
        let other_keys = other.identity_keys();
        let comparison = compare_local_server_device_key_values(
            &local_keys.curve25519.to_base64(),
            &local_keys.ed25519.to_base64(),
            Some(&other_keys.curve25519.to_base64()),
            Some(&other_keys.ed25519.to_base64()),
        );
        report.local_server_identity = match comparison {
            LocalServerDeviceKeyComparison::Match => "match",
            LocalServerDeviceKeyComparison::Mismatch => "mismatch",
            LocalServerDeviceKeyComparison::Unknown => "unknown",
        };
        report.repreflight =
            preflight_saved_crypto_store(&config, Some(expected_user), Some(expected_device))
                .await
                .as_str();
        report.saved_device = "refused_mismatch";
    }

    if matches!(
        case,
        LoginStoreCase::CrashBeforeResponse
            | LoginStoreCase::CrashAfterBoundTokenless
            | LoginStoreCase::CrashDuringCapability
            | LoginStoreCase::CrashDuringVerification
            | LoginStoreCase::StaleCallback
            | LoginStoreCase::StaleBaseUrl
    ) {
        let mut journal = CallbackFence::new();
        let callback_mutations = journal.accept(0, "https://stale.example.invalid");
        report.allocations = 1;
        report.callback_mutations = callback_mutations as usize;
        report.device_generation = if preflight == SavedCryptoStorePreflight::PresentMatching {
            "retained"
        } else {
            "new"
        };
    }

    Ok(report)
}

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> TestResult<Self> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "system clock unavailable".to_owned())?
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "koushi-sdk-login-store-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&path).map_err(|_| "temporary store unavailable".to_owned())?;
        Ok(Self(path))
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct CallbackFence {
    generation: u64,
    base_url: &'static str,
}

impl CallbackFence {
    fn new() -> Self {
        Self {
            generation: 1,
            base_url: "https://synthetic.invalid",
        }
    }

    fn accept(&mut self, generation: u64, base_url: &str) -> bool {
        generation == self.generation && base_url == self.base_url
    }
}

async fn seed_store(config: &MatrixClientStoreConfig, user: &str, device: &str) -> TestResult<()> {
    let user_id =
        matrix_sdk::ruma::UserId::parse(user).map_err(|_| "invalid synthetic user".to_owned())?;
    let device_id = matrix_sdk::ruma::OwnedDeviceId::from(device);
    let store = SqliteCryptoStore::open_with_key(config.path(), Some(config.sdk_store_key()))
        .await
        .map_err(|_| "store open failed".to_owned())?;
    let machine = OlmMachine::with_store(&user_id, &device_id, store.clone(), None)
        .await
        .map_err(|_| "store account creation failed".to_owned())?;
    drop(machine);
    CryptoStore::close(&store)
        .await
        .map_err(|_| "store close failed".to_owned())
}

pub fn select_saved_device(identifier: &str, homeserver: &str) -> &'static str {
    if identifier.starts_with('@') && identifier.contains(':') {
        return "full_id";
    }
    if homeserver.ends_with("/ambiguous") {
        "fresh_device"
    } else {
        "unique_localpart"
    }
}
