use koushi_state::{
    AppAction, AppEffect, AppState, LocalUserAliasUpdateState, SessionAuthenticationMethod,
    SessionInfo, SessionState, UiEvent, reduce,
};
use std::collections::BTreeMap;

#[test]
fn confirmed_alias_failure_remains_recoverable_and_authoritative_reload_restores_labels() {
    let mut state = AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".into(),
            user_id: "@self:example.invalid".into(),
            device_id: "TESTDEVICE".into(),
            authentication_method: SessionAuthenticationMethod::Unknown,
        }),
        ..AppState::default()
    };
    let user = "@reader:example.invalid";
    let previous = BTreeMap::from([(user.to_owned(), "Previous alias".to_owned())]);
    reduce(
        &mut state,
        AppAction::LocalUserAliasesLoaded {
            aliases: previous.clone(),
        },
    );
    reduce(
        &mut state,
        AppAction::LocalUserAliasUpdateRequested {
            request_id: 1,
            user_id: user.into(),
            alias: Some("New alias".into()),
        },
    );
    let effects = reduce(
        &mut state,
        AppAction::LocalUserAliasUpdateFailed {
            request_id: 1,
            message: "synthetic failure".into(),
        },
    );
    assert_eq!(
        state.profile.local_alias_update,
        LocalUserAliasUpdateState::Idle
    );
    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::ErrorChanged)));
    assert!(
        state
            .errors
            .iter()
            .any(|error| error.code == "local_user_alias_update_failed" && error.recoverable)
    );
    // AccountActor supplies the SDK's authoritative aliases after a failed save.
    reduce(
        &mut state,
        AppAction::LocalUserAliasesLoaded {
            aliases: previous.clone(),
        },
    );
    assert_eq!(state.profile.local_aliases, previous);
    reduce(
        &mut state,
        AppAction::LocalUserAliasUpdateRequested {
            request_id: 2,
            user_id: user.into(),
            alias: None,
        },
    );
    assert_eq!(
        state.profile.local_alias_update,
        LocalUserAliasUpdateState::Saving { request_id: 2 }
    );
    assert!(!state.profile.local_aliases.contains_key(user));
}
