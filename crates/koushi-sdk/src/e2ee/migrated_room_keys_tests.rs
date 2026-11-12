//! Tests for the local-app room-key staging import (#1871).
//!
//! Fixtures here are synthetic: one Element-compatible export envelope generated
//! for this repository, and staging files assembled in a temp directory. No test
//! reads another application's store.

use super::migrated_room_keys::{MIGRATED_ROOM_KEYS_STAGING_VERSION, import_migrated_room_keys};
use crate::e2ee::PersistableMatrixSession;
use crate::e2ee::e2ee_trust_tests::ELEMENT_COMPATIBLE_KEY_EXPORT;
use crate::e2ee::restore_session;

const SYNTHETIC_SESSION: &str = r#"{"homeserver":"https://matrix.example.invalid","user_id":"@alice:example.invalid","device_id":"ALICEDEVICE","access_token":"synthetic-access"}"#;

async fn synthetic_session() -> crate::MatrixClientSession {
    let persistable =
        PersistableMatrixSession::from_json(SYNTHETIC_SESSION).expect("synthetic session");
    restore_session(&persistable)
        .await
        .expect("synthetic session should restore")
}

fn write_staging(contents: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("staging.json"), contents).expect("write staging");
    dir
}

fn staging_path(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("staging.json")
}

#[tokio::test]
async fn staged_element_keys_import_through_the_manual_export_path() {
    let keys = matrix_sdk_base::crypto::decrypt_room_key_export(
        ELEMENT_COMPATIBLE_KEY_EXPORT.as_bytes(),
        "1234",
    )
    .expect("fixture envelope should decrypt");
    assert_eq!(keys.len(), 1);
    let staging = serde_json::json!({
        "version": MIGRATED_ROOM_KEYS_STAGING_VERSION,
        "keys": keys,
    });
    let dir = write_staging(&staging.to_string());

    let summary = import_migrated_room_keys(&synthetic_session().await, staging_path(&dir))
        .await
        .expect("staged Element keys should import");

    assert_eq!(summary.imported_keys, 1);
    assert_eq!(summary.attempted_keys, 1);
    assert_eq!(summary.imported_rooms, 1);
}

#[tokio::test]
async fn an_empty_staging_file_imports_nothing() {
    let staging = serde_json::json!({
        "version": MIGRATED_ROOM_KEYS_STAGING_VERSION,
        "keys": [],
    });
    let dir = write_staging(&staging.to_string());

    let summary = import_migrated_room_keys(&synthetic_session().await, staging_path(&dir))
        .await
        .expect("an empty staging file is a no-op, not a failure");

    assert_eq!(summary, Default::default());
}

#[tokio::test]
async fn an_unsupported_staging_version_is_rejected() {
    let staging = serde_json::json!({
        "version": MIGRATED_ROOM_KEYS_STAGING_VERSION + 1,
        "keys": [],
    });
    let dir = write_staging(&staging.to_string());

    let error = import_migrated_room_keys(&synthetic_session().await, staging_path(&dir))
        .await
        .expect_err("an unknown staging version must not import");

    assert!(matches!(error, crate::e2ee::E2eeTrustError::Sdk(_)));
}
