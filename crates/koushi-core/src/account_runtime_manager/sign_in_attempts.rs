//! Correlation registry for pending interactive sign-in attempts.
//!
//! One account tab owns at most one pending browser sign-in attempt. A callback
//! delivered by the platform resolves back to its tab through this registry, so
//! the registry is the single owner of "which account tab may this callback
//! complete". The platform adapter only classifies the callback URL shape
//! ([`OidcCallbackCorrelation`]); it never decides account state itself.

use std::collections::HashMap;

use super::AccountTabId;

/// How a browser sign-in callback is matched to a pending account-tab attempt.
#[derive(Clone, Eq, PartialEq)]
pub enum OidcCallbackCorrelation {
    /// OAuth/MAS authorization. The callback must repeat the exact CSRF state
    /// the SDK minted for this attempt; anything else is a mismatch.
    OAuthState(String),
    /// Legacy `m.login.sso`. The homeserver callback carries only `loginToken`
    /// and no OAuth state, so it can complete a tab only while exactly one
    /// legacy attempt is pending (#1266).
    LegacySso,
}

impl std::fmt::Debug for OidcCallbackCorrelation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The OAuth state is a CSRF secret; only the correlation kind is
        // printable, never the value.
        match self {
            Self::OAuthState(_) => formatter.write_str("OidcCallbackCorrelation::OAuthState(..)"),
            Self::LegacySso => formatter.write_str("OidcCallbackCorrelation::LegacySso"),
        }
    }
}

/// Pending interactive sign-in attempts owned by the account-tab manager.
///
/// At most one attempt per tab is retained: registering a new attempt for a tab
/// retires that tab's previous attempt, so a delayed callback for a superseded
/// authorization is rejected instead of completing the newer attempt.
#[derive(Default)]
pub struct PendingSignInAttempts {
    oauth_states: HashMap<String, AccountTabId>,
    legacy_sso: Vec<AccountTabId>,
}

impl PendingSignInAttempts {
    /// Register the tab's pending attempt.
    ///
    /// `state` is empty exactly on the legacy `m.login.sso` fallback, where the
    /// SDK mints no OAuth CSRF state and the homeserver callback has none to
    /// repeat. A nonempty `state` is an OAuth/MAS CSRF state.
    pub fn register(&mut self, id: &AccountTabId, state: String) {
        self.forget(id);
        if state.is_empty() {
            // The legacy fallback mints no CSRF state, so the callback has none
            // to repeat and can only be correlated by uniqueness.
            self.legacy_sso.push(id.clone());
        } else {
            self.oauth_states.insert(state, id.clone());
        }
    }

    /// The tab a callback with this correlation belongs to, without consuming
    /// the attempt.
    pub fn tab(&self, correlation: &OidcCallbackCorrelation) -> Option<AccountTabId> {
        match correlation {
            OidcCallbackCorrelation::OAuthState(state) => self.oauth_states.get(state).cloned(),
            OidcCallbackCorrelation::LegacySso => self.single_legacy_sso(),
        }
    }

    /// Consume the attempt a callback with this correlation belongs to. A
    /// consumed attempt cannot be replayed.
    pub fn take(&mut self, correlation: &OidcCallbackCorrelation) -> Option<AccountTabId> {
        match correlation {
            OidcCallbackCorrelation::OAuthState(state) => self.oauth_states.remove(state),
            OidcCallbackCorrelation::LegacySso => {
                let id = self.single_legacy_sso()?;
                self.legacy_sso.retain(|tab_id| tab_id != &id);
                Some(id)
            }
        }
    }

    /// Drop every attempt owned by the tab.
    pub fn forget(&mut self, id: &AccountTabId) {
        self.oauth_states.retain(|_, tab_id| tab_id != id);
        self.legacy_sso.retain(|tab_id| tab_id != id);
    }

    /// The one tab a stateless legacy callback may complete. A stateless
    /// callback cannot name its tab, so it is admissible only while exactly one
    /// legacy attempt is pending; two pending legacy tabs are ambiguous and are
    /// rejected rather than misrouted (#1266).
    fn single_legacy_sso(&self) -> Option<AccountTabId> {
        match self.legacy_sso.as_slice() {
            [only] => Some(only.clone()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AccountTabId, OidcCallbackCorrelation, PendingSignInAttempts};

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
    fn oauth_callback_must_repeat_exactly_one_registered_state() {
        let mut attempts = PendingSignInAttempts::default();
        let pending = tab("add:1");
        attempts.register(&pending, "state-one".to_owned());

        assert_eq!(attempts.tab(&oauth("state-one")), Some(pending.clone()));
        assert_eq!(attempts.tab(&oauth("state-two")), None);
        assert_eq!(attempts.tab(&oauth("")), None);
        // An OAuth attempt is never completed by a stateless legacy callback.
        assert_eq!(attempts.tab(&legacy()), None);
        assert_eq!(attempts.take(&legacy()), None);

        // The real OAuth callback still completes the attempt.
        assert_eq!(attempts.take(&oauth("state-one")), Some(pending));
    }

    #[test]
    fn stateless_legacy_sso_callback_correlates_with_one_pending_attempt() {
        let mut attempts = PendingSignInAttempts::default();
        let pending = tab("add:1");
        attempts.register(&pending, String::new());

        // The legacy attempt is not reachable through the OAuth key space.
        assert_eq!(attempts.tab(&oauth("")), None);
        assert_eq!(attempts.tab(&oauth("state-one")), None);

        assert_eq!(attempts.tab(&legacy()), Some(pending.clone()));
        assert_eq!(attempts.take(&legacy()), Some(pending));
    }

    #[test]
    fn two_simultaneous_legacy_sso_tabs_are_ambiguous_and_rejected() {
        let mut attempts = PendingSignInAttempts::default();
        let first = tab("add:1");
        let second = tab("add:2");
        attempts.register(&first, String::new());
        attempts.register(&second, String::new());

        // A stateless callback cannot name one of two pending legacy attempts.
        assert_eq!(attempts.tab(&legacy()), None);
        assert_eq!(attempts.take(&legacy()), None);

        // Both attempts remain pending; retiring one makes the other resolvable.
        attempts.forget(&first);
        assert_eq!(attempts.take(&legacy()), Some(second));
    }

    #[test]
    fn legacy_and_oauth_attempts_in_two_tabs_do_not_mix() {
        let mut attempts = PendingSignInAttempts::default();
        let oauth_tab = tab("add:1");
        let sso_tab = tab("add:2");
        attempts.register(&oauth_tab, "state-one".to_owned());
        attempts.register(&sso_tab, String::new());

        // A stateless callback completes the single legacy tab, not the OAuth one.
        assert_eq!(attempts.take(&legacy()), Some(sso_tab));
        assert_eq!(attempts.take(&oauth("state-one")), Some(oauth_tab));
        assert_eq!(attempts.take(&legacy()), None);
    }

    #[test]
    fn a_consumed_callback_cannot_be_replayed() {
        let mut attempts = PendingSignInAttempts::default();
        let pending = tab("add:1");
        attempts.register(&pending, "state-one".to_owned());

        assert_eq!(attempts.take(&oauth("state-one")), Some(pending.clone()));
        assert_eq!(attempts.tab(&oauth("state-one")), None);
        assert_eq!(attempts.take(&oauth("state-one")), None);

        attempts.register(&pending, String::new());
        assert_eq!(attempts.take(&legacy()), Some(pending));
        assert_eq!(attempts.take(&legacy()), None);
    }

    #[test]
    fn registering_a_second_attempt_retires_the_first_for_that_tab() {
        let mut attempts = PendingSignInAttempts::default();
        let pending = tab("add:1");
        attempts.register(&pending, "state-old".to_owned());
        attempts.register(&pending, "state-new".to_owned());

        assert_eq!(attempts.take(&oauth("state-old")), None);
        assert_eq!(attempts.take(&oauth("state-new")), Some(pending.clone()));

        // A restarted legacy attempt also retires the tab's OAuth attempt.
        attempts.register(&pending, "state-old".to_owned());
        attempts.register(&pending, String::new());
        assert_eq!(attempts.take(&oauth("state-old")), None);
        assert_eq!(attempts.take(&legacy()), Some(pending));
    }

    #[test]
    fn unsolicited_callback_matches_nothing() {
        let mut attempts = PendingSignInAttempts::default();

        assert_eq!(attempts.tab(&legacy()), None);
        assert_eq!(attempts.tab(&oauth("state-one")), None);
        assert_eq!(attempts.take(&legacy()), None);
        assert_eq!(attempts.take(&oauth("state-one")), None);
    }

    #[test]
    fn forgetting_a_tab_drops_both_attempt_kinds() {
        let mut attempts = PendingSignInAttempts::default();
        let pending = tab("add:1");
        attempts.register(&pending, "state-one".to_owned());
        attempts.forget(&pending);
        assert_eq!(attempts.take(&oauth("state-one")), None);

        attempts.register(&pending, String::new());
        attempts.forget(&pending);
        assert_eq!(attempts.take(&legacy()), None);
    }

    #[test]
    fn oauth_state_never_appears_in_debug_output() {
        let debug = format!("{:?}", oauth("synthetic-csrf-value"));

        assert!(debug.contains("OAuthState"));
        assert!(!debug.contains("synthetic-csrf-value"));
        assert!(format!("{:?}", legacy()).contains("LegacySso"));
    }
}
