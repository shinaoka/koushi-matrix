//! Legacy SSO callback correlation across the Tauri adapter and core (#1266).
//!
//! The adapter only classifies the callback URL shape; the core account-tab
//! manager owns whether that correlation matches exactly one pending attempt.
//! OAuth/MAS CSRF-state validation must stay exactly as strict as before.

use koushi_core::account_runtime_manager::{
    AccountTabId, OidcCallbackCorrelation, PendingSignInAttempts,
};
use koushi_desktop::oidc_callback_correlation;

const CALLBACK: &str = "com.github.shinaoka.koushi-matrix:/auth/callback";
const LEGACY_CALLBACK: &str =
    "com.github.shinaoka.koushi-matrix:/auth/callback?loginToken=synthetic";

fn tab(name: &str) -> AccountTabId {
    AccountTabId::from_string(name.to_owned())
}

fn oauth(state: &str) -> OidcCallbackCorrelation {
    OidcCallbackCorrelation::OAuthState(state.to_owned())
}

fn legacy() -> OidcCallbackCorrelation {
    OidcCallbackCorrelation::LegacySso
}

#[test]
fn oauth_callback_correlation_still_requires_exactly_one_nonempty_state() {
    assert_eq!(
        oidc_callback_correlation(
            "com.github.shinaoka.koushi-matrix:/auth/callback?code=synthetic&state=attempt%2Fone"
        ),
        Some(oauth("attempt/one"))
    );
    assert_eq!(
        oidc_callback_correlation(
            "com.github.shinaoka.koushi-matrix:/auth/callback?code=synthetic"
        ),
        None
    );
    assert_eq!(
        oidc_callback_correlation("com.github.shinaoka.koushi-matrix:/auth/callback?state="),
        None
    );
    assert_eq!(
        oidc_callback_correlation(
            "com.github.shinaoka.koushi-matrix:/auth/callback?state=one&state=two"
        ),
        None
    );
}

#[test]
fn stateless_legacy_sso_callback_is_classified_and_never_as_oauth_state() {
    assert_eq!(oidc_callback_correlation(LEGACY_CALLBACK), Some(legacy()));
}

#[test]
fn malformed_or_hybrid_callbacks_are_rejected() {
    // No callback parameters at all.
    assert_eq!(oidc_callback_correlation(CALLBACK), None);
    // An OAuth authorization-code callback without a state is not a legacy one.
    assert_eq!(
        oidc_callback_correlation(
            "com.github.shinaoka.koushi-matrix:/auth/callback?code=synthetic"
        ),
        None
    );
    // Empty and duplicated login tokens are malformed.
    assert_eq!(
        oidc_callback_correlation("com.github.shinaoka.koushi-matrix:/auth/callback?loginToken="),
        None
    );
    assert_eq!(
        oidc_callback_correlation(
            "com.github.shinaoka.koushi-matrix:/auth/callback?loginToken=one&loginToken=two"
        ),
        None
    );
    // A callback mixing OAuth state and loginToken matches neither shape.
    assert_eq!(
        oidc_callback_correlation(
            "com.github.shinaoka.koushi-matrix:/auth/callback?state=one&loginToken=synthetic"
        ),
        None
    );
    assert_eq!(
        oidc_callback_correlation(
            "com.github.shinaoka.koushi-matrix:/auth/callback?state=one&state=two&loginToken=synthetic"
        ),
        None
    );
}

#[test]
fn stateless_legacy_sso_callback_correlates_with_one_pending_tab_and_cannot_replay() {
    let correlation = oidc_callback_correlation(LEGACY_CALLBACK).expect("legacy callback shape");
    let mut attempts = PendingSignInAttempts::default();
    let pending = tab("add:1");
    attempts.register(&pending, String::new());

    assert_eq!(attempts.tab(&correlation), Some(pending.clone()));
    assert_eq!(attempts.take(&correlation), Some(pending));

    // A replayed deep link resolves nothing: the attempt was consumed.
    assert_eq!(
        attempts.take(&oidc_callback_correlation(LEGACY_CALLBACK).expect("legacy shape")),
        None
    );
}

#[test]
fn two_simultaneous_legacy_tabs_make_a_stateless_callback_ambiguous() {
    let correlation = oidc_callback_correlation(LEGACY_CALLBACK).expect("legacy callback shape");
    let mut attempts = PendingSignInAttempts::default();
    attempts.register(&tab("add:1"), String::new());
    attempts.register(&tab("add:2"), String::new());

    assert_eq!(attempts.tab(&correlation), None);
    assert_eq!(attempts.take(&correlation), None);
}

#[test]
fn stateless_callback_never_completes_an_oauth_tab() {
    let correlation = oidc_callback_correlation(LEGACY_CALLBACK).expect("legacy callback shape");
    let mut attempts = PendingSignInAttempts::default();
    let oauth_tab = tab("add:1");
    attempts.register(&oauth_tab, "state-one".to_owned());

    assert_eq!(attempts.take(&correlation), None);

    // With one legacy tab alongside the OAuth tab, the stateless callback
    // resolves only the legacy tab and leaves the OAuth attempt intact.
    let legacy_tab = tab("add:2");
    attempts.register(&legacy_tab, String::new());
    assert_eq!(attempts.take(&correlation), Some(legacy_tab));
    assert_eq!(attempts.take(&oauth("state-one")), Some(oauth_tab));
}

#[test]
fn unsolicited_stateless_callback_is_rejected() {
    let correlation = oidc_callback_correlation(LEGACY_CALLBACK).expect("legacy callback shape");
    let mut attempts = PendingSignInAttempts::default();

    assert_eq!(attempts.tab(&correlation), None);
    assert_eq!(attempts.take(&correlation), None);
}
