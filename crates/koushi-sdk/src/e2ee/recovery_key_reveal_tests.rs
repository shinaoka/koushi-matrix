//! #927: the recovery key Koushi reveals on screen must be the secret-storage
//! key that `recovery().recover()` accepts. The fork's
//! `backups().local_recovery_key()` returns the backup decryption key instead,
//! which `recover()` rejects, so it must never be revealed or saved.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use matrix_sdk::test_utils::mocks::MatrixMockServer;
use wiremock::{Mock, Request, Respond, ResponseTemplate, matchers::path_regex};

use super::{MatrixClientSession, SessionInfo, reset_recovery_key};

/// Minimal in-memory global account-data store so secret storage written by
/// one call can be read back by `recover()`.
#[derive(Clone, Default)]
struct AccountDataStore {
    data: Arc<Mutex<HashMap<String, serde_json::Value>>>,
    /// Injects the "new recovery key could not be created" failure a reset hits
    /// when its secret-storage key upload fails (#1997).
    fail_secret_storage_key_writes: Arc<std::sync::atomic::AtomicBool>,
}

impl AccountDataStore {
    fn event_type(request: &Request) -> String {
        request
            .url
            .path()
            .rsplit('/')
            .next()
            .expect("account data path has an event type")
            .to_owned()
    }
}

impl Respond for AccountDataStore {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let event_type = Self::event_type(request);
        if request.method.as_str() == "PUT"
            && event_type.starts_with("m.secret_storage.key.")
            && self
                .fail_secret_storage_key_writes
                .load(std::sync::atomic::Ordering::SeqCst)
        {
            return ResponseTemplate::new(500).set_body_json(serde_json::json!({
                "errcode": "M_UNKNOWN",
                "error": "synthetic secret storage write failure"
            }));
        }
        let mut store = self.data.lock().expect("account data store lock");
        if request.method.as_str() == "PUT" {
            let body = serde_json::from_slice(&request.body).expect("account data body is JSON");
            store.insert(event_type, body);
            return ResponseTemplate::new(200).set_body_json(serde_json::json!({}));
        }
        match store.get(&event_type) {
            Some(content) => ResponseTemplate::new(200).set_body_json(content),
            None => ResponseTemplate::new(404).set_body_json(serde_json::json!({
                "errcode": "M_NOT_FOUND",
                "error": "Account data not found"
            })),
        }
    }
}

async fn session_with_account_data() -> (MatrixMockServer, MatrixClientSession) {
    let (server, session, _store) = session_with_account_data_store().await;
    (server, session)
}

async fn session_with_account_data_store()
-> (MatrixMockServer, MatrixClientSession, AccountDataStore) {
    let server = MatrixMockServer::new().await;
    let store = AccountDataStore::default();
    Mock::given(path_regex(
        r"^/_matrix/client/(r0|v3)/user/[^/]+/account_data/[^/]+$",
    ))
    .respond_with(store.clone())
    .mount(server.server())
    .await;
    server.mock_query_keys().ok().mount().await;
    let client = server.client_builder().build().await;
    let info = SessionInfo {
        homeserver: server.server().uri(),
        user_id: client.user_id().expect("mock user").to_string(),
        device_id: client.device_id().expect("mock device").to_string(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    };
    (
        server,
        MatrixClientSession::from_client_for_testing(client, info),
        store,
    )
}

#[tokio::test]
async fn reset_recovery_key_reveals_a_key_that_recover_accepts() {
    let (_server, session) = session_with_account_data().await;

    let summary = reset_recovery_key(&session, None)
        .await
        .expect("reset_key creates a new secret store");
    let recovery = session.client().encryption().recovery();

    recovery
        .recover(summary.recovery_key.as_str())
        .await
        .expect("the revealed key must unlock secret storage");
}

#[tokio::test]
async fn setup_reveals_the_key_enable_returned_and_recover_accepts_it() {
    let (server, session) = session_with_account_data().await;
    server.mock_room_keys_version().none().mount().await;
    server.mock_add_room_keys_version().ok().mount().await;

    let summary = super::bootstrap_secure_backup(&session, None)
        .await
        .expect("recovery().enable() creates backup and secret storage");

    session
        .client()
        .encryption()
        .recovery()
        .recover(summary.recovery_key.as_str())
        .await
        .expect("the revealed setup key must unlock secret storage");
}

#[tokio::test]
async fn a_backup_decryption_key_is_not_a_recovery_key() {
    let (server, session) = session_with_account_data().await;
    server.mock_room_keys_version().none().mount().await;
    server.mock_add_room_keys_version().ok().mount().await;
    let summary = super::bootstrap_secure_backup(&session, None)
        .await
        .expect("recovery().enable() creates backup and secret storage");

    // The actual export of the fork's `backups().local_recovery_key()`: the
    // base58 backup decryption key. `recover()` must reject it, so Koushi
    // never reveals it as a recovery key.
    let backup_key = session
        .client()
        .encryption()
        .backups()
        .local_recovery_key()
        .await
        .expect("local backup key lookup succeeds")
        .expect("enable() stored a local backup decryption key");
    assert_ne!(backup_key.as_str(), summary.recovery_key.as_str());
    assert!(
        session
            .client()
            .encryption()
            .recovery()
            .recover(backup_key.as_str())
            .await
            .is_err()
    );
}

/// The key returned by `recovery().enable()` is the only copy once setup
/// has created secret storage, so an upload steady-state failure after that
/// point must never drop it (#927 audit regression).
#[tokio::test]
async fn an_upload_settlement_failure_still_reveals_the_created_key() {
    let summary = super::SecureBackupSetupSummary {
        recovery_key: zeroize::Zeroizing::new("synthetic-created-key-927".to_owned()),
    };

    let revealed = super::reveal_created_recovery_key(summary, async {
        Err(super::E2eeTrustError::SecureBackupUploadFailed)
    })
    .await
    .expect("a created key is revealed even when upload has not settled");

    assert_eq!(revealed.recovery_key.as_str(), "synthetic-created-key-927");
}

/// Minimal server-side key-backup version store: echoes back the signed
/// `auth_data` the client created so trust inspection sees a real backup.
#[derive(Clone, Default)]
struct BackupVersionStore(Arc<Mutex<Option<serde_json::Value>>>);

impl Respond for BackupVersionStore {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let mut stored = self.0.lock().expect("backup version store lock");
        if request.method.as_str() == "POST" {
            let body: serde_json::Value =
                serde_json::from_slice(&request.body).expect("backup version body is JSON");
            *stored = Some(body);
            return ResponseTemplate::new(200).set_body_json(serde_json::json!({ "version": "1" }));
        }
        match stored.as_ref() {
            Some(body) => ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "algorithm": body["algorithm"],
                "auth_data": body["auth_data"],
                "count": 0,
                "etag": "0",
                "version": "1",
            })),
            None => ResponseTemplate::new(404).set_body_json(serde_json::json!({
                "errcode": "M_NOT_FOUND",
                "error": "No current backup version"
            })),
        }
    }
}

/// NEW-1: `enable()` created and enabled the backup but was interrupted before
/// `create_secret_store`. The lost-key reset must be admitted and complete the
/// missing secret storage with a key `recover()` accepts.
#[tokio::test]
async fn reset_completes_secret_storage_after_an_interrupted_enable() {
    let (server, session) = session_with_account_data().await;
    Mock::given(path_regex(
        r"^/_matrix/client/(r0|v3|unstable)/room_keys/version$",
    ))
    .respond_with(BackupVersionStore::default())
    .mount(server.server())
    .await;
    server.mock_upload_keys().ok().mount().await;
    server.mock_upload_cross_signing_keys().ok().mount().await;
    server
        .mock_upload_cross_signing_signatures()
        .ok()
        .mount()
        .await;
    let encryption = session.client().encryption();
    encryption
        .bootstrap_cross_signing(None)
        .await
        .expect("cross-signing bootstrap");
    // What `enable()` does before creating secret storage.
    encryption.backups().create().await.expect("backup created");

    let inspection = session
        .inspect_secure_backup()
        .await
        .expect("inspection succeeds");
    assert!(
        inspection.recovery_key_reset_is_possible(),
        "interrupted enable must admit the reset: {inspection:?}"
    );

    let summary = session
        .reset_secure_backup_recovery_key(None)
        .await
        .expect("reset completes the missing secret storage");
    encryption
        .recovery()
        .recover(summary.recovery_key.as_str())
        .await
        .expect("the reset key unlocks the new secret storage");
}

/// #1997: a reset can start from a healthy backup, so a failure inside
/// `reset_key()` reveals nothing and the previous key still works. Leaving the
/// marker set would present that key as lost, close reading, and offer only a
/// destructive reset as the way out.
#[tokio::test]
async fn a_failed_reset_from_a_healthy_backup_keeps_the_current_key() {
    let (server, session, store) = session_with_account_data_store().await;
    mount_backup_and_cross_signing(&server, &session).await;
    assert!(
        session
            .inspect_secure_backup()
            .await
            .expect("inspection succeeds")
            .recovery_key_reset_is_possible(),
        "a healthy backup admits the reset"
    );
    assert!(
        !session
            .recovery_key_delivery_pending()
            .await
            .expect("marker read"),
        "no reveal is awaited yet"
    );

    store
        .fail_secret_storage_key_writes
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(
        session
            .reset_secure_backup_recovery_key(None)
            .await
            .is_err(),
        "the injected secret-storage failure must fail the reset"
    );
    assert!(
        !session
            .recovery_key_delivery_pending()
            .await
            .expect("marker read"),
        "a failed reset reveals no key, so the delivery marker must not claim one"
    );
}

/// The lost-key origin keeps its marker: that reveal really is unknown to the
/// user, so a second failed reset must not make the account look healthy.
#[tokio::test]
async fn a_failed_reset_of_an_awaited_reveal_keeps_the_marker() {
    let (server, session, store) = session_with_account_data_store().await;
    mount_backup_and_cross_signing(&server, &session).await;
    session
        .set_recovery_key_delivery_pending(true)
        .await
        .expect("marker write");

    store
        .fail_secret_storage_key_writes
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(
        session
            .reset_secure_backup_recovery_key(None)
            .await
            .is_err(),
        "the injected secret-storage failure must fail the reset"
    );
    assert!(
        session
            .recovery_key_delivery_pending()
            .await
            .expect("marker read"),
        "an interrupted reveal stays pending"
    );
}

async fn mount_backup_and_cross_signing(server: &MatrixMockServer, session: &MatrixClientSession) {
    Mock::given(path_regex(
        r"^/_matrix/client/(r0|v3|unstable)/room_keys/version$",
    ))
    .respond_with(BackupVersionStore::default())
    .mount(server.server())
    .await;
    server.mock_upload_keys().ok().mount().await;
    server.mock_upload_cross_signing_keys().ok().mount().await;
    server
        .mock_upload_cross_signing_signatures()
        .ok()
        .mount()
        .await;
    session
        .client()
        .encryption()
        .bootstrap_cross_signing(None)
        .await
        .expect("cross-signing bootstrap");
    session
        .client()
        .encryption()
        .backups()
        .create()
        .await
        .expect("backup created");
}

/// #1049: the identity bootstrap sets the delivery-pending marker before
/// `enable()`. When `enable()` then fails no key was revealed, so the marker
/// must not survive to force a later recovery-key reset.
#[tokio::test]
async fn a_failed_identity_bootstrap_enable_clears_the_delivery_marker() {
    let (server, session) = session_with_account_data().await;
    server.mock_room_keys_version().none().mount().await;
    server.mock_add_room_keys_version().error500().mount().await;

    assert!(
        session
            .bootstrap_identity_secure_backup(None)
            .await
            .is_err(),
        "the injected backup-creation failure must fail the bootstrap"
    );
    assert!(
        !session
            .recovery_key_delivery_pending()
            .await
            .expect("marker read"),
        "a failed bootstrap must not leave the delivery marker set"
    );
}
