//! RED contract tests for the Element X-compatible login/store lifecycle.
//!
//! The `login_store_test_support` surface is intentionally a future SDK test
//! hook. These tests specify the closed, private-data-free observations without
//! adding production behavior or fixtures that contain credentials.

use koushi_sdk::login_store_test_support::{self as store, LoginStoreCase};

#[test]
fn saved_missing_corrupt_and_wrong_account_stay_offline() {
    for case in [
        LoginStoreCase::MissingCryptoDb,
        LoginStoreCase::CorruptCryptoDb,
        LoginStoreCase::WrongKey,
        LoginStoreCase::WrongAccount,
    ] {
        let report = store::run(case);
        assert_eq!(report.login_requests, 0);
        assert_eq!(report.keys_upload_requests, 0);
        assert_eq!(report.keys_query_requests, 0);
        assert!(matches!(
            report.saved_device,
            "refused_missing_crypto" | "refused_mismatch"
        ));
    }
}

#[test]
fn fresh_login_and_reopen_use_one_closed_store_client_generation() {
    let report = store::run(LoginStoreCase::FreshLoginThenReopen);

    assert_eq!(report.crypto_client_generations, "one");
    assert_eq!(report.crypto_identity_generations, "one");
    assert_eq!(report.saved_device, "new_device");
}

#[test]
fn identity_mismatch_is_closed_and_original_store_remains_preflightable() {
    let report = store::run(LoginStoreCase::ServerIdentityMismatch);

    assert_eq!(report.local_server_identity, "mismatch");
    assert_eq!(report.repreflight, "present_matching");
    assert_eq!(report.saved_device, "refused_mismatch");
}

#[test]
fn oauth_sso_and_soft_logout_keep_the_journaled_store_and_device() {
    for case in [
        LoginStoreCase::OAuthCompletion,
        LoginStoreCase::SsoCompletion,
        LoginStoreCase::OAuthSoftLogoutReauth,
        LoginStoreCase::SsoSoftLogoutReauth,
    ] {
        let report = store::run(case);
        assert_eq!(report.crypto_client_generations, "one");
        assert_eq!(report.store_generation, "retained");
        assert_eq!(report.device_generation, "retained");
    }
}

#[test]
fn journal_boundaries_reuse_store_device_and_fence_callbacks() {
    for case in [
        LoginStoreCase::CrashBeforeResponse,
        LoginStoreCase::CrashAfterBoundTokenless,
        LoginStoreCase::CrashDuringCapability,
        LoginStoreCase::CrashDuringVerification,
        LoginStoreCase::StaleCallback,
        LoginStoreCase::StaleBaseUrl,
    ] {
        let report = store::run(case);
        assert_eq!(report.allocations, 1);
        assert_eq!(report.callback_mutations, 0);
        assert_eq!(report.device_generation, "retained");
    }
}

#[test]
fn saved_selection_requires_full_id_or_unique_localpart() {
    assert_eq!(
        store::select_saved_device("@member:example.invalid", "example.invalid"),
        "full_id"
    );
    assert_eq!(
        store::select_saved_device("member", "example.invalid"),
        "unique_localpart"
    );
    assert_eq!(
        store::select_saved_device("member", "example.invalid/ambiguous"),
        "fresh_device"
    );
}
