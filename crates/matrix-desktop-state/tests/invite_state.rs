use matrix_desktop_state::{
    AppAction, AppState, InvitePreview, SessionInfo, SessionState, UiEvent, reduce,
};

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "http://127.0.0.1:6167".to_owned(),
            user_id: "@qa:localhost".to_owned(),
            device_id: "LOCALDEVICE".to_owned(),
        }),
        ..AppState::default()
    }
}

fn invite_preview(room_id: &str, is_dm: bool) -> InvitePreview {
    InvitePreview {
        room_id: room_id.to_owned(),
        display_name: "Invite preview".to_owned(),
        topic: Some("Project room".to_owned()),
        inviter_display_name: Some("Inviter".to_owned()),
        is_dm,
    }
}

#[test]
fn invite_list_is_rust_owned_and_replaces_by_snapshot() {
    let mut state = ready_state();
    let effects = reduce(
        &mut state,
        AppAction::InviteListUpdated {
            invites: vec![invite_preview("!room:localhost", false)],
        },
    );

    assert_eq!(
        state.invites,
        vec![invite_preview("!room:localhost", false)]
    );
    assert_eq!(
        effects,
        vec![matrix_desktop_state::AppEffect::EmitUiEvent(
            UiEvent::RoomListChanged
        )]
    );

    reduce(
        &mut state,
        AppAction::InviteListUpdated {
            invites: vec![invite_preview("!dm:localhost", true)],
        },
    );
    assert_eq!(state.invites, vec![invite_preview("!dm:localhost", true)]);
}

#[test]
fn invite_list_is_ignored_without_ready_session_and_cleared_on_logout() {
    let mut signed_out = AppState::default();
    reduce(
        &mut signed_out,
        AppAction::InviteListUpdated {
            invites: vec![invite_preview("!room:localhost", false)],
        },
    );
    assert!(signed_out.invites.is_empty());

    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::InviteListUpdated {
            invites: vec![invite_preview("!room:localhost", false)],
        },
    );
    assert!(!state.invites.is_empty());

    reduce(&mut state, AppAction::LogoutFinished);
    assert!(state.invites.is_empty());
}
