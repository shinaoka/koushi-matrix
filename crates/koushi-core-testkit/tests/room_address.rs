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
