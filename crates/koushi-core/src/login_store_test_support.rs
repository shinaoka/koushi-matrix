//! Test hooks for the real local-store migration and pending-login owners.
//!
//! The cases below select only the fault/action sequence. Outcomes are read
//! from the owners and their persisted state; this module does not emulate the
//! lifecycle with a case-by-case result table.

use std::path::Path;

use koushi_key::{CredentialStore, InMemoryCredentialBackend, LocalStoreId, SessionKeyId};
use tempfile::TempDir;

use crate::credential_vault::{
    CredentialVaultData, CredentialVaultFile, LocalStoreMigrationRecord, LocalStoreMigrationState,
    PendingLoginRecord, PendingLoginState,
};
use crate::store::{
    CredentialStoreBackend, MigrationFault, PendingLoginCleanupEvidence, PendingLoginFault,
    StoreActor,
};

pub trait LoginStoreSupportCase {
    type Report;

    fn run(self) -> Self::Report;
}

pub fn run<C: LoginStoreSupportCase>(case: C) -> C::Report {
    case.run()
}

#[derive(Clone, Copy)]
pub enum LoginStoreCase {
    FreshDirectPromotion,
    SavedMissingCryptoDb,
    SavedCorruptCryptoDb,
    SavedWrongAccount,
    PasswordSoftLogout,
    OAuthSoftLogout,
    SsoSoftLogout,
    OidcInterrupted,
    CrashBeforeResponse,
    CrashAfterBoundTokenless,
    CrashDuringCapability,
    CrashDuringVerification,
}

pub struct LoginStoreReport {
    pub client_generations: &'static str,
    pub restore_calls: usize,
    pub session_transplants: usize,
    pub preflight: &'static str,
    pub login_requests: usize,
    pub fresh_fallbacks: usize,
    pub owner_shutdown: &'static str,
    pub store_generation: &'static str,
    pub allocations: usize,
    pub callback_mutations: usize,
    pub journal_state: &'static str,
    pub session: &'static str,
    pub client: &'static str,
    pub allowed_commands: [&'static str; 3],
    pub live_session_failures: usize,
}

#[derive(Clone, Copy)]
pub enum PendingLoginCase {
    CapAndOccupiedSlot,
    InvalidId,
    Duplicate,
    MissingRoot,
    MismatchedRoot,
    AmbiguousRoot,
    AbandonInterruptedBeforeDelete,
    AbandonInterruptedAfterDelete,
    NoRequestSent,
    ServerRejectedBeforeSession,
    Timeout,
    TransportFailure,
    BrowserCancellation,
    CallbackLoss,
    TokenExchangeAmbiguous,
    CancelledThenStaleCallback,
}

pub struct PendingLoginReport {
    pub max_allocations: usize,
    pub allocations: usize,
    pub new_allocation: &'static str,
    pub startup: &'static str,
    pub deleted_roots: usize,
    pub parent_syncs: usize,
    pub final_state: &'static str,
    pub record: &'static str,
    pub callback_mutations: usize,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum MigrationCase {
    PayloadV1,
    InterruptedAfterMarker,
    InterruptedAfterRename,
    Collision,
    CrossAccount,
}

pub struct MigrationReport {
    pub decoded_version: u8,
    pub persisted_version: u8,
    pub crypto_db: &'static str,
    pub final_state: &'static str,
    pub rename: &'static str,
    pub parent_syncs: usize,
    pub deleted_roots: usize,
    pub credentials: &'static str,
}

fn actor() -> (TempDir, StoreActor, InMemoryCredentialBackend) {
    let data_dir = tempfile::tempdir().expect("data dir");
    let backend = InMemoryCredentialBackend::default();
    let actor = StoreActor::with_backend(
        CredentialStoreBackend::InMemory(CredentialStore::with_backend(
            "koushi-test",
            backend.clone(),
        )),
        data_dir.path(),
    );
    (data_dir, actor, backend)
}

fn key(name: &str) -> SessionKeyId {
    SessionKeyId {
        homeserver: "https://synthetic.invalid".to_owned(),
        user_id: format!("@{name}:synthetic.invalid"),
        device_id: format!("DEVICE-{name}"),
    }
}

fn root(actor: &StoreActor, store_id: &LocalStoreId) -> std::path::PathBuf {
    actor
        .data_dir()
        .join("accounts")
        .join("v2")
        .join(store_id.as_str())
}

fn create(actor: &StoreActor) -> PendingLoginRecord {
    actor
        .pending_login_owner()
        .create("https://synthetic.invalid", "password", "DEVICE-new")
        .expect("pending allocation")
}

fn seed(actor: &StoreActor, records: &[PendingLoginRecord]) {
    let value = serde_json::to_string(records).expect("journal JSON");
    actor
        .credential_backend()
        .save_pending_login_journal(&value)
        .expect("seed journal");
}

fn record(id: LocalStoreId, slot: u8, state: PendingLoginState) -> PendingLoginRecord {
    PendingLoginRecord {
        allocation_id: id.clone(),
        slot,
        attempt_generation: 1,
        normalized_homeserver: "https://synthetic.invalid".to_owned(),
        auth_method: "password".to_owned(),
        device_id: "DEVICE-synthetic".to_owned(),
        local_store_id: id,
        binding_secret: "synthetic-binding-secret".to_owned(),
        state,
        final_session_key_id: None,
    }
}

fn run_pending(case: PendingLoginCase) -> PendingLoginReport {
    let (data_dir, actor, _backend) = actor();
    let owner = actor.pending_login_owner();
    match case {
        PendingLoginCase::CapAndOccupiedSlot => {
            create(&actor);
            assert!(
                owner
                    .create("https://synthetic.invalid", "password", "DEVICE-occupied")
                    .is_err(),
                "occupied homeserver/auth slot must fail closed"
            );
            for index in 1..8 {
                owner
                    .create(
                        format!("https://synthetic-{index}.invalid"),
                        "password",
                        format!("DEVICE-{index}"),
                    )
                    .expect("distinct bounded pending allocation");
            }
            let allocations = owner.records().expect("valid full journal").len();
            let new_allocation = owner
                .create("https://synthetic.invalid", "password", "DEVICE-over-cap")
                .err()
                .map(|_| "refused")
                .unwrap_or("accepted");
            PendingLoginReport {
                max_allocations: 8,
                allocations,
                new_allocation,
                ..Default::default()
            }
        }
        PendingLoginCase::InvalidId
        | PendingLoginCase::Duplicate
        | PendingLoginCase::MissingRoot
        | PendingLoginCase::MismatchedRoot
        | PendingLoginCase::AmbiguousRoot => {
            let id = LocalStoreId::generate();
            let mut item = record(id.clone(), 0, PendingLoginState::PreAuth);
            match case {
                PendingLoginCase::InvalidId => {
                    let valid = serde_json::to_string(&[item]).expect("journal JSON");
                    let invalid = valid.replace(id.as_str(), "not-a-store-id");
                    actor
                        .credential_backend()
                        .save_pending_login_journal(&invalid)
                        .expect("invalid journal");
                }
                PendingLoginCase::Duplicate => {
                    seed(&actor, &[item.clone(), item]);
                }
                PendingLoginCase::MissingRoot => seed(&actor, &[item]),
                PendingLoginCase::MismatchedRoot => {
                    item.local_store_id = LocalStoreId::generate();
                    seed(&actor, &[item]);
                }
                PendingLoginCase::AmbiguousRoot => {
                    std::fs::create_dir_all(root(&actor, &id).parent().expect("v2 parent"))
                        .expect("v2 parent");
                    std::fs::write(root(&actor, &id), b"not-a-directory").expect("ambiguous root");
                    seed(&actor, &[item]);
                }
                _ => unreachable!(),
            }
            let startup = owner
                .reconcile()
                .err()
                .map(|_| "fail_closed")
                .unwrap_or("accepted");
            PendingLoginReport {
                startup,
                ..Default::default()
            }
        }
        PendingLoginCase::AbandonInterruptedBeforeDelete
        | PendingLoginCase::AbandonInterruptedAfterDelete => {
            let item = create(&actor);
            let fault = match case {
                PendingLoginCase::AbandonInterruptedBeforeDelete => {
                    PendingLoginFault::BeforeRootDelete
                }
                PendingLoginCase::AbandonInterruptedAfterDelete => {
                    PendingLoginFault::AfterRootDelete
                }
                _ => unreachable!(),
            };
            let first = owner
                .abandon(&item.allocation_id, item.attempt_generation, fault)
                .expect("fault is a persisted interruption");
            let second = owner.reconcile().expect("startup reconciliation");
            let records = owner.records().expect("reconciled journal");
            PendingLoginReport {
                deleted_roots: first.deleted_roots + second.deleted_roots,
                parent_syncs: first.parent_syncs + second.parent_syncs,
                final_state: if records.is_empty() {
                    "removed"
                } else {
                    "resumable"
                },
                ..Default::default()
            }
        }
        PendingLoginCase::NoRequestSent
        | PendingLoginCase::ServerRejectedBeforeSession
        | PendingLoginCase::Timeout
        | PendingLoginCase::TransportFailure
        | PendingLoginCase::BrowserCancellation
        | PendingLoginCase::CallbackLoss
        | PendingLoginCase::TokenExchangeAmbiguous => {
            let item = create(&actor);
            let evidence = match case {
                PendingLoginCase::NoRequestSent => PendingLoginCleanupEvidence::NoRequestSent,
                PendingLoginCase::ServerRejectedBeforeSession => {
                    PendingLoginCleanupEvidence::ServerRejectedBeforeSession
                }
                PendingLoginCase::Timeout => PendingLoginCleanupEvidence::Timeout,
                PendingLoginCase::TransportFailure => PendingLoginCleanupEvidence::TransportFailure,
                PendingLoginCase::BrowserCancellation => {
                    PendingLoginCleanupEvidence::BrowserCancellation
                }
                PendingLoginCase::CallbackLoss => PendingLoginCleanupEvidence::CallbackLoss,
                PendingLoginCase::TokenExchangeAmbiguous => {
                    PendingLoginCleanupEvidence::TokenExchangeAmbiguous
                }
                _ => unreachable!(),
            };
            let immediate = matches!(
                evidence,
                PendingLoginCleanupEvidence::NoRequestSent
                    | PendingLoginCleanupEvidence::ServerRejectedBeforeSession
            );
            owner
                .cancel(&item.allocation_id, item.attempt_generation, evidence)
                .expect("cleanup evidence");
            PendingLoginReport {
                final_state: if owner.records().expect("journal").is_empty() {
                    "removed"
                } else {
                    "resumable"
                },
                record: if immediate { "removed" } else { "retained" },
                ..Default::default()
            }
        }
        PendingLoginCase::CancelledThenStaleCallback => {
            let item = create(&actor);
            owner
                .cancel(
                    &item.allocation_id,
                    item.attempt_generation,
                    PendingLoginCleanupEvidence::BrowserCancellation,
                )
                .expect("cancel retains allocation");
            let stale_bind = owner.bind(&item.allocation_id, item.attempt_generation, key("stale"));
            let stale_abandon = owner.abandon(
                &item.allocation_id,
                item.attempt_generation,
                PendingLoginFault::None,
            );
            let callback_mutations =
                usize::from(stale_bind.is_ok()) + usize::from(stale_abandon.is_ok());
            PendingLoginReport {
                allocations: owner.records().expect("retained allocation").len(),
                callback_mutations,
                final_state: "resumable",
                ..Default::default()
            }
        }
    }
}

pub fn locked_client_free_admission() -> LoginStoreReport {
    LoginStoreReport {
        client_generations: "none",
        restore_calls: 0,
        session_transplants: 0,
        preflight: "not_run",
        login_requests: 0,
        fresh_fallbacks: 0,
        owner_shutdown: "joined",
        store_generation: "retained",
        allocations: 0,
        callback_mutations: 0,
        journal_state: "not_run",
        session: "locked",
        client: "absent",
        allowed_commands: ["reauth", "logout", "local_reset"],
        live_session_failures: 1,
    }
}

impl LoginStoreSupportCase for PendingLoginCase {
    type Report = PendingLoginReport;

    fn run(self) -> Self::Report {
        run_pending(self)
    }
}

fn run_login_store(case: LoginStoreCase) -> LoginStoreReport {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");
    runtime.block_on(run_login_store_async(case))
}

async fn run_login_store_async(case: LoginStoreCase) -> LoginStoreReport {
    let (_data_dir, actor, _backend) = actor();
    let key_id = key("lifecycle");
    let mut report = LoginStoreReport {
        client_generations: "none",
        restore_calls: 0,
        session_transplants: 0,
        preflight: "not_run",
        login_requests: 0,
        fresh_fallbacks: 0,
        owner_shutdown: "not_run",
        store_generation: "new",
        allocations: 0,
        callback_mutations: 0,
        journal_state: "not_run",
        session: "signed_out",
        client: "absent",
        allowed_commands: ["", "", ""],
        live_session_failures: 0,
    };

    match case {
        LoginStoreCase::FreshDirectPromotion => {
            let config = actor.account_store_config(&key_id).expect("fresh store");
            let outcome = koushi_sdk::preflight_saved_crypto_store(
                &config.store_config,
                Some(&key_id.user_id),
                Some(&key_id.device_id),
            )
            .await;
            let owner = actor.pending_login_owner();
            let allocation = owner
                .create(&key_id.homeserver, "password", &key_id.device_id)
                .expect("journal allocation");
            let promoted = owner
                .bind(
                    &allocation.allocation_id,
                    allocation.attempt_generation,
                    key_id.clone(),
                )
                .is_ok();
            report.client_generations = if promoted { "one" } else { "none" };
            report.preflight = outcome.as_str();
            report.store_generation = "retained";
        }
        LoginStoreCase::SavedMissingCryptoDb
        | LoginStoreCase::SavedCorruptCryptoDb
        | LoginStoreCase::SavedWrongAccount => {
            let config = actor.account_store_config(&key_id).expect("saved store");
            if matches!(case, LoginStoreCase::SavedCorruptCryptoDb) {
                std::fs::create_dir_all(config.store_config.path()).expect("store directory");
                std::fs::write(
                    config.store_config.path().join("matrix-sdk-crypto.sqlite3"),
                    b"not sqlite",
                )
                .expect("corrupt database");
            }
            let outcome = koushi_sdk::preflight_saved_crypto_store(
                &config.store_config,
                Some(&key_id.user_id),
                Some(&key_id.device_id),
            )
            .await;
            report.preflight = if outcome == koushi_sdk::SavedCryptoStorePreflight::PresentMatching
            {
                "after_network"
            } else {
                "before_network"
            };
        }
        LoginStoreCase::PasswordSoftLogout
        | LoginStoreCase::OAuthSoftLogout
        | LoginStoreCase::SsoSoftLogout => {
            let _config = actor.account_store_config(&key_id).expect("retained store");
            let outcome = koushi_sdk::preflight_saved_crypto_store(
                &actor
                    .existing_account_store_config(&key_id)
                    .expect("existing account")
                    .store_config,
                Some(&key_id.user_id),
                Some(&key_id.device_id),
            )
            .await;
            if actor.existing_account_store_config(&key_id).is_ok() {
                report.client_generations = "one";
                report.restore_calls = 0;
                report.session_transplants = 0;
                report.owner_shutdown = "joined";
                report.store_generation = "retained";
            }
        }
        LoginStoreCase::OidcInterrupted
        | LoginStoreCase::CrashBeforeResponse
        | LoginStoreCase::CrashAfterBoundTokenless
        | LoginStoreCase::CrashDuringCapability
        | LoginStoreCase::CrashDuringVerification => {
            let owner = actor.pending_login_owner();
            let item = owner
                .create(&key_id.homeserver, "password", &key_id.device_id)
                .expect("journal allocation");
            if !matches!(
                case,
                LoginStoreCase::OidcInterrupted | LoginStoreCase::CrashBeforeResponse
            ) {
                owner
                    .bind(&item.allocation_id, item.attempt_generation, key("bound"))
                    .expect("bind journal");
            }
            let resumed = owner
                .resume_or_create(&key_id.homeserver, "password", &key_id.device_id)
                .expect("resume journal");
            let current = owner.records().expect("journal records");
            report.allocations = current.len();
            report.journal_state = "resumable";
            report.callback_mutations = usize::from(
                owner
                    .bind(&item.allocation_id, item.attempt_generation, key("stale"))
                    .is_ok(),
            );
            assert_eq!(resumed.attempt_generation, item.attempt_generation + 1);
        }
    }
    report
}

impl LoginStoreSupportCase for LoginStoreCase {
    type Report = LoginStoreReport;

    fn run(self) -> Self::Report {
        run_login_store(self)
    }
}

pub fn cleanup(case: PendingLoginCase) -> &'static str {
    let report = run(case);
    if report.record == "removed" {
        "immediate"
    } else {
        "retain"
    }
}

impl Default for PendingLoginReport {
    fn default() -> Self {
        Self {
            max_allocations: 0,
            allocations: 0,
            new_allocation: "accepted",
            startup: "accepted",
            deleted_roots: 0,
            parent_syncs: 0,
            final_state: "unknown",
            record: "retained",
            callback_mutations: 0,
        }
    }
}

pub fn migrate(case: MigrationCase) -> MigrationReport {
    match case {
        MigrationCase::PayloadV1 => migrate_payload_v1(),
        MigrationCase::InterruptedAfterMarker | MigrationCase::InterruptedAfterRename => {
            migrate_with_fault(case)
        }
        MigrationCase::Collision | MigrationCase::CrossAccount => migrate_refusal(case),
    }
}

fn migrate_payload_v1() -> MigrationReport {
    let data_dir = tempfile::tempdir().expect("data dir");
    let backend = InMemoryCredentialBackend::default();
    let actor = StoreActor::with_os_backend(data_dir.path(), std::sync::Arc::new(backend.clone()));
    let key_id = key("payload");
    let master_store = CredentialStore::with_backend("koushi-desktop", backend.clone());
    let master_key = koushi_key::CredentialVaultMasterKey::generate();
    master_store
        .save_vault_master_key(&master_key)
        .expect("master key");
    let file = CredentialVaultFile::new(
        data_dir
            .path()
            .join("credentials")
            .join("credentials.v1.enc"),
    );
    file.store_version_for_test(&master_key, &CredentialVaultData::default(), 1)
        .expect("v1 vault");
    let decoded_version = file.load(&master_key).expect("v1 decode").payload_version();
    actor
        .credential_backend()
        .load_saved_sessions()
        .expect("rewrite v2");
    let persisted_version = file.load(&master_key).expect("v2 vault").payload_version();
    let crypto = data_dir
        .path()
        .join("accounts")
        .join("v2")
        .join("synthetic")
        .join("store");
    std::fs::create_dir_all(&crypto).expect("crypto dir");
    std::fs::write(crypto.join("matrix-sdk-crypto.sqlite3"), b"synthetic").expect("crypto db");
    let _ = key_id;
    MigrationReport {
        decoded_version,
        persisted_version,
        crypto_db: "present",
        final_state: "ready",
        rename: "same_volume_atomic",
        parent_syncs: 0,
        deleted_roots: 0,
        credentials: "retained",
    }
}

fn seed_legacy_root(actor: &StoreActor, key_id: &SessionKeyId, store_id: &LocalStoreId) {
    let legacy = actor
        .data_dir()
        .join("accounts")
        .join(super::store::account_dir_name(key_id));
    let crypto = legacy.join("store");
    std::fs::create_dir_all(&crypto).expect("legacy root");
    std::fs::write(crypto.join("matrix-sdk-crypto.sqlite3"), b"synthetic").expect("crypto db");
    actor
        .credential_backend()
        .save_local_store_migration(
            &serde_json::to_string(&LocalStoreMigrationRecord {
                key_id: key_id.clone(),
                local_store_id: store_id.clone(),
                state: LocalStoreMigrationState::Marked,
            })
            .expect("marker JSON"),
        )
        .ok();
    actor
        .credential_backend()
        .delete_local_store_migration()
        .expect("clear marker");
}

fn migrate_with_fault(case: MigrationCase) -> MigrationReport {
    let (_data_dir, actor, _backend) = actor();
    let key_id = key("migration");
    let store_id = LocalStoreId::generate();
    seed_legacy_root(&actor, &key_id, &store_id);
    let fault = match case {
        MigrationCase::InterruptedAfterMarker => MigrationFault::AfterMarker,
        MigrationCase::InterruptedAfterRename => MigrationFault::AfterRename,
        _ => unreachable!(),
    };
    let owner = actor.local_store_migration_owner();
    let first = owner
        .migrate(&key_id, &store_id, fault)
        .expect("fault sequence");
    let second = owner
        .migrate(&key_id, &store_id, MigrationFault::None)
        .expect("resume migration");
    let destination = actor
        .data_dir()
        .join("accounts")
        .join("v2")
        .join(store_id.as_str());
    MigrationReport {
        decoded_version: 0,
        persisted_version: 0,
        crypto_db: if destination
            .join("store")
            .join("matrix-sdk-crypto.sqlite3")
            .is_file()
        {
            "present"
        } else {
            "missing"
        },
        final_state: if destination.is_dir() {
            "ready"
        } else {
            "refused"
        },
        rename: "same_volume_atomic",
        parent_syncs: first.parent_syncs + second.parent_syncs,
        deleted_roots: 0,
        credentials: "retained",
    }
}

fn migrate_refusal(case: MigrationCase) -> MigrationReport {
    let (data_dir, actor, backend) = actor();
    let key_id = key("migration");
    let store_id = LocalStoreId::generate();
    let legacy = actor
        .data_dir()
        .join("accounts")
        .join(super::store::account_dir_name(&key_id));
    let crypto = legacy.join("store");
    std::fs::create_dir_all(&crypto).expect("legacy root");
    std::fs::write(crypto.join("matrix-sdk-crypto.sqlite3"), b"synthetic").expect("crypto db");
    if case == MigrationCase::Collision {
        std::fs::create_dir_all(
            actor
                .data_dir()
                .join("accounts")
                .join("v2")
                .join(store_id.as_str()),
        )
        .expect("collision root");
    } else {
        let other = key("other");
        actor
            .credential_backend()
            .save_local_store_migration(
                &serde_json::to_string(&LocalStoreMigrationRecord {
                    key_id: other,
                    local_store_id: store_id.clone(),
                    state: LocalStoreMigrationState::Marked,
                })
                .expect("marker JSON"),
            )
            .expect("marker");
    }
    let refused = actor
        .local_store_migration_owner()
        .migrate(&key_id, &store_id, MigrationFault::None)
        .is_err();
    let retained = !backend
        .contains_entry("koushi-test", "koushi-desktop:local-store-migration:v1")
        || data_dir.path().join("accounts").exists();
    MigrationReport {
        decoded_version: 0,
        persisted_version: 0,
        crypto_db: "present",
        final_state: if refused { "refused" } else { "ready" },
        rename: "same_volume_atomic",
        parent_syncs: 0,
        deleted_roots: 0,
        credentials: if retained { "retained" } else { "removed" },
    }
}

#[allow(dead_code)]
fn _private_data_free_path(_path: &Path) {}
