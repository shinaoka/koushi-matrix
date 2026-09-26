use koushi_core::CoreConnection;
use koushi_protocol::state_update::VersionedAppStateSnapshot;
use koushi_state::{
    AppState, RoomAddressError, SessionAuthenticationMethod, SessionInfo, SessionState,
};

#[test]
fn room_address_preview_uses_current_ready_identity_without_mutating_state() {
    let (connection, control) = CoreConnection::new_for_testing(8);
    assert_eq!(
        connection.preview_room_address("Name", None).error,
        Some(RoomAddressError::NotReady)
    );
    let mut state = AppState::default();
    state.session = SessionState::Ready(SessionInfo {
        homeserver: "https://delegated.example.invalid".into(),
        user_id: "@member:matrix.example.invalid:8448".into(),
        device_id: "SYNTHETIC".into(),
        authentication_method: SessionAuthenticationMethod::Unknown,
    });
    control.send_snapshot(VersionedAppStateSnapshot {
        generation: 1,
        state,
    });
    assert_eq!(
        connection
            .preview_room_address("Example Room", None)
            .full_alias
            .as_deref(),
        Some("#example-room:matrix.example.invalid:8448")
    );
    assert_eq!(
        connection
            .preview_room_address("Changed Name", Some("manual"))
            .localpart,
        "manual"
    );
    assert_eq!(connection.state_generation(), 1);
    control.send_snapshot(VersionedAppStateSnapshot {
        generation: 2,
        state: AppState::default(),
    });
    assert_eq!(
        connection.preview_room_address("Name", None).error,
        Some(RoomAddressError::NotReady)
    );
}

/// #1006: a public room created from a Space suggests `<space>-<room>`; the
/// display name is unchanged and a manual address is preserved.
#[test]
fn room_address_suggestion_is_prefixed_with_the_selected_space_name() {
    let (connection, control) = CoreConnection::new_for_testing(8);
    let mut state = AppState::default();
    state.session = SessionState::Ready(SessionInfo {
        homeserver: "https://example.invalid".into(),
        user_id: "@member:example.invalid".into(),
        device_id: "SYNTHETIC".into(),
        authentication_method: SessionAuthenticationMethod::Unknown,
    });
    state.spaces = vec![koushi_state::SpaceSummary {
        space_id: "!space:example.invalid".into(),
        raw_name: Some("research-group".into()),
        display_name: "research-group".into(),
        avatar: None,
        join_rule: None,
        child_room_ids: Vec::new(),
    }];
    state.navigation.active_space_id = Some("!space:example.invalid".into());
    control.send_snapshot(VersionedAppStateSnapshot {
        generation: 1,
        state: state.clone(),
    });

    let preview = connection.preview_room_address("papers", None);
    assert_eq!(preview.localpart, "research-group-papers");
    assert_eq!(
        preview.full_alias.as_deref(),
        Some("#research-group-papers:example.invalid")
    );
    assert_eq!(
        connection
            .preview_room_address("papers", Some("papers-2026"))
            .localpart,
        "papers-2026"
    );

    // At Home there is no Space prefix.
    state.navigation.active_space_id = None;
    control.send_snapshot(VersionedAppStateSnapshot {
        generation: 2,
        state,
    });
    assert_eq!(
        connection.preview_room_address("papers", None).localpart,
        "papers"
    );
}
