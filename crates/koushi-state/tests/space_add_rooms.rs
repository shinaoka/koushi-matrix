//! Space add-existing-room projection and linking settlement (#1007).

use koushi_state::{
    AppAction, AppState, BasicOperationRequest, OperationFailureKind, RoomSummary, RoomTags,
    SessionInfo, SessionState, SpaceAddRoomStatus, SpaceChildLinkOutcome, SpaceSummary,
    compose_sidebar_for_state, reduce, space_add_rooms_for_state,
};

const SPACE: &str = "!space:example.invalid";
/// A room version 12 room ID has no server component.
const DOMAINLESS: &str = "!31hneApxJ_1o-63DmFrpeqnkFfWppnzWso1JvH3ogLM";

fn room(room_id: &str, name: &str, is_dm: bool, parents: &[&str]) -> RoomSummary {
    RoomSummary {
        room_id: room_id.to_owned(),
        display_name: name.to_owned(),
        display_label: name.to_owned(),
        original_display_label: name.to_owned(),
        avatar: None,
        is_dm,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        marked_unread: false,
        recency_stamp: None,
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: parents.iter().map(|id| (*id).to_owned()).collect(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 1,
    }
}

fn state() -> AppState {
    let mut state = AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "http://127.0.0.1:6167".to_owned(),
            user_id: "@member:example.invalid".to_owned(),
            device_id: "DEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        spaces: vec![SpaceSummary {
            space_id: SPACE.to_owned(),
            raw_name: Some("Synthetic Workspace".to_owned()),
            display_name: "Synthetic Workspace".to_owned(),
            avatar: None,
            join_rule: None,
            child_room_ids: vec!["!linked:example.invalid".to_owned()],
        }],
        rooms: vec![
            room("!linked:example.invalid", "Linked", false, &[SPACE]),
            // Parent-only: the room claims the Space, the Space does not list it.
            room(DOMAINLESS, "Parent only", false, &[SPACE]),
            room("!other:example.invalid", "another room", false, &[]),
            room("!dm:example.invalid", "Member 1", true, &[]),
        ],
        ..AppState::default()
    };
    state.navigation.active_space_id = Some(SPACE.to_owned());
    state
}

fn status_of(state: &AppState, room_id: &str) -> Option<SpaceAddRoomStatus> {
    space_add_rooms_for_state(state)?
        .candidates
        .into_iter()
        .find(|candidate| candidate.room_id == room_id)
        .map(|candidate| candidate.status)
}

fn request_link(state: &mut AppState, request_id: u64, room_id: &str) {
    reduce(
        state,
        AppAction::BasicOperationRequested {
            request_id,
            request: BasicOperationRequest::LinkSpaceChild {
                space_id: SPACE.to_owned(),
                child_room_id: room_id.to_owned(),
            },
        },
    );
}

fn settle(state: &mut AppState, request_id: u64, room_id: &str, outcome: SpaceChildLinkOutcome) {
    reduce(
        state,
        AppAction::SpaceChildLinkSettled {
            request_id,
            space_id: SPACE.to_owned(),
            child_room_id: room_id.to_owned(),
            outcome,
        },
    );
}

#[test]
fn eligibility_comes_from_parent_side_children_and_excludes_dms() {
    let state = state();
    let model = space_add_rooms_for_state(&state).expect("a Space is active");
    assert_eq!(model.space_id, SPACE);
    let rows: Vec<_> = model
        .candidates
        .iter()
        .map(|candidate| (candidate.display_name.as_str(), candidate.status))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("another room", SpaceAddRoomStatus::Available),
            ("Linked", SpaceAddRoomStatus::Added),
            ("Parent only", SpaceAddRoomStatus::Available),
        ]
    );
}

#[test]
fn home_has_no_add_rooms_projection() {
    let mut state = state();
    state.navigation.active_space_id = None;
    assert!(space_add_rooms_for_state(&state).is_none());
    assert!(compose_sidebar_for_state(&state).space_add_rooms.is_none());
}

#[test]
fn sidebar_carries_the_active_space_projection() {
    let state = state();
    assert_eq!(
        compose_sidebar_for_state(&state).space_add_rooms,
        space_add_rooms_for_state(&state)
    );
}

#[test]
fn a_linked_settlement_marks_the_room_added_before_the_next_sync() {
    let mut state = state();
    request_link(&mut state, 7, DOMAINLESS);
    assert_eq!(
        status_of(&state, DOMAINLESS),
        Some(SpaceAddRoomStatus::Adding)
    );
    assert_eq!(
        status_of(&state, "!other:example.invalid"),
        Some(SpaceAddRoomStatus::Available)
    );

    settle(&mut state, 7, DOMAINLESS, SpaceChildLinkOutcome::Linked);
    reduce(
        &mut state,
        AppAction::BasicOperationSucceeded { request_id: 7 },
    );

    assert!(state.basic_operation.is_idle());
    assert_eq!(
        status_of(&state, DOMAINLESS),
        Some(SpaceAddRoomStatus::Added)
    );
    assert!(
        state.spaces[0]
            .child_room_ids
            .contains(&DOMAINLESS.to_owned())
    );

    // A room-list refresh from a lagging SDK cache must not make it addable.
    state.spaces[0].child_room_ids = vec!["!linked:example.invalid".to_owned()];
    assert_eq!(
        status_of(&state, DOMAINLESS),
        Some(SpaceAddRoomStatus::Added)
    );
}

#[test]
fn a_failed_settlement_is_retryable_and_a_retry_replaces_it() {
    let mut state = state();
    request_link(&mut state, 3, DOMAINLESS);
    let failed = SpaceChildLinkOutcome::Failed {
        reason: OperationFailureKind::Network,
    };
    settle(&mut state, 3, DOMAINLESS, failed);
    reduce(
        &mut state,
        AppAction::BasicOperationFailed {
            request_id: 3,
            message: "Linking the room to the space failed".to_owned(),
        },
    );
    assert_eq!(
        status_of(&state, DOMAINLESS),
        Some(SpaceAddRoomStatus::Failed {
            reason: OperationFailureKind::Network
        })
    );
    assert!(
        !state.spaces[0]
            .child_room_ids
            .contains(&DOMAINLESS.to_owned())
    );

    request_link(&mut state, 4, DOMAINLESS);
    assert_eq!(
        status_of(&state, DOMAINLESS),
        Some(SpaceAddRoomStatus::Adding)
    );
    settle(&mut state, 4, DOMAINLESS, SpaceChildLinkOutcome::Linked);
    reduce(
        &mut state,
        AppAction::BasicOperationSucceeded { request_id: 4 },
    );
    assert_eq!(
        status_of(&state, DOMAINLESS),
        Some(SpaceAddRoomStatus::Added)
    );
    assert_eq!(state.space_child_links.entries.len(), 1);
}

#[test]
fn stale_duplicate_and_idle_settlements_are_ignored() {
    let mut state = state();
    // Idle: nothing is in flight.
    settle(&mut state, 1, DOMAINLESS, SpaceChildLinkOutcome::Linked);
    assert!(state.space_child_links.entries.is_empty());

    request_link(&mut state, 2, DOMAINLESS);
    // A duplicate submission while one is in flight is not admitted.
    request_link(&mut state, 5, "!other:example.invalid");
    assert_eq!(
        status_of(&state, "!other:example.invalid"),
        Some(SpaceAddRoomStatus::Available)
    );
    settle(
        &mut state,
        5,
        "!other:example.invalid",
        SpaceChildLinkOutcome::Linked,
    );
    assert!(state.space_child_links.entries.is_empty());
    assert_eq!(
        status_of(&state, DOMAINLESS),
        Some(SpaceAddRoomStatus::Adding)
    );
}

#[test]
fn switching_spaces_scopes_status_to_the_selected_space() {
    let mut state = state();
    request_link(&mut state, 9, DOMAINLESS);
    settle(
        &mut state,
        9,
        DOMAINLESS,
        SpaceChildLinkOutcome::Failed {
            reason: OperationFailureKind::Forbidden,
        },
    );
    state.spaces.push(SpaceSummary {
        space_id: "!second:example.invalid".to_owned(),
        raw_name: None,
        display_name: "Second".to_owned(),
        avatar: None,
        join_rule: None,
        child_room_ids: Vec::new(),
    });
    state.navigation.active_space_id = Some("!second:example.invalid".to_owned());
    let model = space_add_rooms_for_state(&state).expect("second Space");
    assert_eq!(model.space_id, "!second:example.invalid");
    assert!(
        model
            .candidates
            .iter()
            .all(|candidate| candidate.status == SpaceAddRoomStatus::Available)
    );
}

#[test]
fn sign_out_clears_linking_settlements() {
    let mut state = state();
    request_link(&mut state, 11, DOMAINLESS);
    settle(&mut state, 11, DOMAINLESS, SpaceChildLinkOutcome::Linked);
    reduce(&mut state, AppAction::LogoutRequested);
    assert!(state.space_child_links.entries.is_empty());
}
