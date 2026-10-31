use koushi_core::CoreConnection;
use koushi_protocol::state_update::VersionedAppStateSnapshot;
use koushi_state::{
    AppAction, AppState, CreateRoomAccessSeed, CreateRoomVisibility, RoomAccessDraftScope,
    RoomAddressError, SessionAuthenticationMethod, SessionInfo, SessionState, reduce,
};

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://delegated.example.invalid".into(),
            user_id: "@member:matrix.example.invalid:8448".into(),
            device_id: "SYNTHETIC".into(),
            authentication_method: SessionAuthenticationMethod::Unknown,
        }),
        spaces: vec![koushi_state::SpaceSummary {
            space_id: "!space:example.invalid".into(),
            raw_name: Some("Research Group".into()),
            display_name: "local presentation name".into(),
            avatar: None,
            join_rule: None,
            topic: None,
            child_room_ids: Vec::new(),
            parent_side_child_room_ids: Vec::new(),
        }],
        ..Default::default()
    }
}

fn open_create(state: &mut AppState, session_id: u64, parent: Option<&str>, suffix: &str) {
    reduce(
        state,
        AppAction::RoomAccessDraftOpened {
            scope: RoomAccessDraftScope::Create { session_id },
            create: Some(CreateRoomAccessSeed {
                visibility: CreateRoomVisibility::Public,
                invited_only: false,
                parent_space_id: parent.map(str::to_owned),
                address_suffix: Some(suffix.to_owned()),
            }),
        },
    );
}

/// #1416: an unedited address is the create session's random default, with
/// the attachment Space's Matrix name as the prefix; it never follows the name.
#[test]
fn an_unedited_address_is_the_create_sessions_random_default() {
    let (connection, control) = CoreConnection::new_for_testing(8);
    assert_eq!(
        connection.preview_room_address(None, 1).error,
        Some(RoomAddressError::NotReady)
    );
    let mut state = ready_state();
    open_create(&mut state, 1, Some("!space:example.invalid"), "3fa9c2");
    control.send_snapshot(VersionedAppStateSnapshot {
        generation: 1,
        state: state.clone(),
    });
    let preview = connection.preview_room_address(None, 1);
    assert_eq!(preview.localpart, "research-group-3fa9c2");
    assert_eq!(
        preview.full_alias.as_deref(),
        Some("#research-group-3fa9c2:matrix.example.invalid:8448")
    );
    // An edited address wins; an edited empty address is empty.
    assert_eq!(
        connection.preview_room_address(Some("papers"), 1).localpart,
        "papers"
    );
    assert_eq!(
        connection.preview_room_address(Some(""), 1).error,
        Some(RoomAddressError::Empty)
    );
    // Another session's default is never shown.
    assert_eq!(
        connection.preview_room_address(None, 2).error,
        Some(RoomAddressError::Empty)
    );
    assert_eq!(connection.state_generation(), 1);

    // At Home the prefix is `room`.
    let mut home = ready_state();
    open_create(&mut home, 3, None, "0b1c2d");
    control.send_snapshot(VersionedAppStateSnapshot {
        generation: 2,
        state: home,
    });
    assert_eq!(
        connection.preview_room_address(None, 3).localpart,
        "room-0b1c2d"
    );
}
