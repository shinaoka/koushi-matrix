//! RED contract tests for unopened legacy-root migration.

use koushi_core::login_store_test_support::{self as store, MigrationCase};

#[test]
fn credential_payload_v1_decodes_and_is_rewritten_as_v2() {
    let report = store::migrate(MigrationCase::PayloadV1);

    assert_eq!(report.decoded_version, 1);
    assert_eq!(report.persisted_version, 2);
    assert_eq!(report.crypto_db, "present");
}

#[test]
fn migration_resumes_after_marker_and_rename_interruptions() {
    for case in [
        MigrationCase::InterruptedAfterMarker,
        MigrationCase::InterruptedAfterRename,
    ] {
        let report = store::migrate(case);
        assert_eq!(report.final_state, "ready");
        assert_eq!(report.rename, "same_volume_atomic");
        assert_eq!(report.parent_syncs, 2);
    }
}

#[test]
fn collision_and_cross_account_roots_fail_closed_without_cleanup() {
    for case in [MigrationCase::Collision, MigrationCase::CrossAccount] {
        let report = store::migrate(case);
        assert_eq!(report.final_state, "refused");
        assert_eq!(report.deleted_roots, 0);
        assert_eq!(report.credentials, "retained");
    }
}
