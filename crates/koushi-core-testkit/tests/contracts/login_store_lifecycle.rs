//! RED contract tests for core-owned login/store lifecycle and admission.
//!
//! The test hook named here is intentionally future-facing. It keeps this RED
//! slice headless and exercises closed observations rather than SDK internals.

use koushi_core::login_store_test_support::{self as lifecycle, LoginStoreCase};

#[test]
fn fresh_login_promotes_the_authenticated_client_directly() {
    let report = lifecycle::run(LoginStoreCase::FreshDirectPromotion);

    assert_eq!(report.client_generations, "one");
    assert_eq!(report.restore_calls, 0);
    assert_eq!(report.session_transplants, 0);
}

#[test]
fn saved_login_preflights_before_auth_and_never_falls_back_fresh() {
    for case in [
        LoginStoreCase::SavedMissingCryptoDb,
        LoginStoreCase::SavedCorruptCryptoDb,
        LoginStoreCase::SavedWrongAccount,
    ] {
        let report = lifecycle::run(case);
        assert_eq!(report.preflight, "before_network");
        assert_eq!(report.login_requests, 0);
        assert_eq!(report.fresh_fallbacks, 0);
    }
}

#[test]
fn password_oauth_and_sso_soft_logout_join_owners_then_promote_store_backed_client() {
    for case in [
        LoginStoreCase::PasswordSoftLogout,
        LoginStoreCase::OAuthSoftLogout,
        LoginStoreCase::SsoSoftLogout,
    ] {
        let report = lifecycle::run(case);
        assert_eq!(report.owner_shutdown, "joined");
        assert_eq!(report.client_generations, "one");
        assert_eq!(report.restore_calls, 0);
        assert_eq!(report.session_transplants, 0);
        assert_eq!(report.store_generation, "retained");
    }
}

#[test]
fn oidc_interruption_and_crashes_resume_one_allocation_with_stale_callbacks_inert() {
    for case in [
        LoginStoreCase::OidcInterrupted,
        LoginStoreCase::CrashBeforeResponse,
        LoginStoreCase::CrashAfterBoundTokenless,
        LoginStoreCase::CrashDuringCapability,
        LoginStoreCase::CrashDuringVerification,
    ] {
        let report = lifecycle::run(case);
        assert_eq!(report.allocations, 1);
        assert_eq!(report.callback_mutations, 0);
        assert_eq!(report.journal_state, "resumable");
    }
}

#[test]
fn client_free_locked_state_allows_only_reauth_logout_and_local_reset() {
    let report = lifecycle::locked_client_free_admission();

    assert_eq!(report.session, "locked");
    assert_eq!(report.client, "absent");
    assert_eq!(report.allowed_commands, ["reauth", "logout", "local_reset"]);
    assert_eq!(report.live_session_failures, 1);
}
