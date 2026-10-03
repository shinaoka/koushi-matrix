//! Create-room dialog defaults (#1023): Rust decides the initial access
//! choice from the selected Space's join rule; React only renders it.

use koushi_state::{
    AppState, CreateRoomDefaults, CreateRoomVisibility, RoomJoinRule, SpaceSummary,
    compose_sidebar_for_state, create_room_defaults_for_state,
};

const SPACE: &str = "!space:example.invalid";

fn state_in_space(join_rule: Option<RoomJoinRule>) -> AppState {
    let mut state = AppState {
        spaces: vec![SpaceSummary {
            space_id: SPACE.to_owned(),
            raw_name: Some("Synthetic Workspace".to_owned()),
            display_name: "Synthetic Workspace".to_owned(),
            avatar: None,
            join_rule,
            child_room_ids: Vec::new(),
            parent_side_child_room_ids: Vec::new(),
        }],
        ..AppState::default()
    };
    state.navigation.active_space_id = Some(SPACE.to_owned());
    state
}

const PRIVATE_DEFAULTS: CreateRoomDefaults = CreateRoomDefaults {
    visibility: CreateRoomVisibility::Private,
    encrypted: true,
    invited_only: false,
};

#[test]
fn a_public_space_selects_a_public_room_initially() {
    let state = state_in_space(Some(RoomJoinRule::Public));
    assert_eq!(
        create_room_defaults_for_state(&state),
        CreateRoomDefaults {
            visibility: CreateRoomVisibility::Public,
            // The private choice keeps its encrypted default when the user
            // switches to it; a public room is never created encrypted.
            encrypted: true,
            invited_only: false,
        }
    );
    assert_eq!(
        compose_sidebar_for_state(&state)
            .create_room_defaults
            .visibility,
        CreateRoomVisibility::Public
    );
}

#[test]
fn private_spaces_unknown_rules_and_home_keep_the_private_default() {
    for rule in [
        Some(RoomJoinRule::Invite),
        Some(RoomJoinRule::Restricted),
        Some(RoomJoinRule::Knock),
        Some(RoomJoinRule::KnockRestricted),
        Some(RoomJoinRule::Private),
        Some(RoomJoinRule::Unknown),
        // Not yet projected: never guessed as public.
        None,
    ] {
        let state = state_in_space(rule);
        assert_eq!(
            create_room_defaults_for_state(&state),
            PRIVATE_DEFAULTS,
            "{rule:?}"
        );
    }

    let mut home = state_in_space(Some(RoomJoinRule::Public));
    home.navigation.active_space_id = None;
    assert_eq!(create_room_defaults_for_state(&home), PRIVATE_DEFAULTS);
    assert_eq!(
        compose_sidebar_for_state(&home).create_room_defaults,
        PRIVATE_DEFAULTS
    );
}

#[test]
fn a_selected_space_missing_from_the_list_keeps_the_private_default() {
    let mut state = state_in_space(Some(RoomJoinRule::Public));
    state.navigation.active_space_id = Some("!gone:example.invalid".to_owned());
    assert_eq!(create_room_defaults_for_state(&state), PRIVATE_DEFAULTS);
}

#[test]
fn visibility_serializes_as_the_protocol_wire_values() {
    assert_eq!(
        serde_json::to_value(CreateRoomDefaults {
            visibility: CreateRoomVisibility::Public,
            encrypted: true,
            invited_only: false,
        })
        .unwrap(),
        serde_json::json!({ "visibility": "public", "encrypted": true, "invited_only": false })
    );
}
