//! Issue #1410: suggested Space rooms lead the Not joined lane, admins can
//! toggle the flag, and joining a Space offers its suggested rooms once.

use koushi_state::{
    AppAction, AppEffect, AppState, OperationFailureKind, RoomSummary, RoomTags, SessionInfo,
    SessionState, SpaceChildMembership, SpaceChildSuggestedEditState, SpaceChildSummary,
    SpaceChildrenState, SpaceSummary, SuggestedRoomJoinOutcome, SuggestedRoomsOffer, UiEvent,
    compose_sidebar_for_state, reduce,
};

const SPACE_ID: &str = "!space:example.invalid";
const SUGGESTED: &str = "!suggested:example.invalid";
const PLAIN: &str = "!plain:example.invalid";
const PRIVATE: &str = "!private:example.invalid";

fn joined_room(room_id: &str, label: &str) -> RoomSummary {
    RoomSummary {
        display_name_placeholder: None,
        display_label_placeholder: None,
        room_id: room_id.to_owned(),
        display_name: label.to_owned(),
        display_label: label.to_owned(),
        original_display_label: label.to_owned(),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: 3,
        notification_count: 3,
        highlight_count: 0,
        thread_unread_count: 0,
        thread_highlight_count: 0,
        marked_unread: false,
        recency_stamp: Some(10),
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: vec![SPACE_ID.to_owned()],
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 2,
    }
}

fn child(room_id: &str, label: &str, suggested: bool, can_join: bool) -> SpaceChildSummary {
    SpaceChildSummary {
        room_id: room_id.to_owned(),
        display_name: label.to_owned(),
        display_name_placeholder: None,
        avatar: None,
        topic: None,
        membership: SpaceChildMembership::NotJoined,
        can_join,
        suggested,
        is_space: false,
        joined_members: 3,
    }
}

fn children() -> Vec<SpaceChildSummary> {
    vec![
        child(PLAIN, "Alpha", false, true),
        child(SUGGESTED, "Zulu", true, true),
        // Invite-only: suggested, but never offered for a plain join.
        child(PRIVATE, "Private", true, false),
    ]
}

fn ready_state() -> AppState {
    let mut state = AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@alice:example.invalid".to_owned(),
            device_id: "DEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        spaces: vec![SpaceSummary {
            space_id: SPACE_ID.to_owned(),
            raw_name: Some("Space".to_owned()),
            display_name: "Space".to_owned(),
            avatar: None,
            join_rule: None,
            topic: None,
            child_room_ids: Vec::new(),
            parent_side_child_room_ids: Vec::new(),
        }],
        space_children: SpaceChildrenState {
            selected_space_id: Some(SPACE_ID.to_owned()),
            generation: 1,
            ..Default::default()
        },
        ..Default::default()
    };
    state.navigation.active_space_id = Some(SPACE_ID.to_owned());
    state
}

fn load(state: &mut AppState, can_edit_suggested: bool) -> Vec<AppEffect> {
    let generation = state.space_children.generation;
    reduce(
        state,
        AppAction::SpaceChildrenLoaded {
            space_id: SPACE_ID.to_owned(),
            generation,
            children: children(),
            can_edit_suggested,
        },
    )
}

fn emits_children_changed(effects: &[AppEffect]) -> bool {
    effects.iter().any(|effect| {
        matches!(
            effect,
            AppEffect::EmitUiEvent(UiEvent::SpaceChildrenChanged)
        )
    })
}

fn offered_room_ids(state: &AppState) -> Vec<String> {
    match &state.space_children.suggested_offer {
        SuggestedRoomsOffer::Offered { rooms, .. } | SuggestedRoomsOffer::Joining { rooms, .. } => {
            rooms.iter().map(|room| room.room_id.clone()).collect()
        }
        _ => Vec::new(),
    }
}

#[test]
fn suggested_rooms_lead_the_not_joined_lane() {
    let mut state = ready_state();
    load(&mut state, false);

    let sidebar = compose_sidebar_for_state(&state);
    let lane: Vec<(&str, bool)> = sidebar
        .sections
        .not_joined
        .iter()
        .map(|item| (item.display_name.as_str(), item.suggested))
        .collect();
    assert_eq!(lane, [("Private", true), ("Zulu", true), ("Alpha", false)]);
}

#[test]
fn joining_a_space_offers_its_joinable_suggested_rooms_once() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomJoinedLocally {
            room_id: SPACE_ID.to_owned(),
        },
    );

    let effects = load(&mut state, false);

    assert!(emits_children_changed(&effects));
    assert_eq!(offered_room_ids(&state), [SUGGESTED]);
    assert_eq!(
        state.space_children.suggested_offer.space_id(),
        Some(SPACE_ID)
    );

    // Skip, then any later visit or reload: never offered again.
    reduce(
        &mut state,
        AppAction::SuggestedRoomsOfferDismissed {
            space_id: SPACE_ID.to_owned(),
        },
    );
    assert_eq!(
        state.space_children.suggested_offer,
        SuggestedRoomsOffer::None
    );
    state.space_children.generation += 1;
    load(&mut state, false);
    assert_eq!(
        state.space_children.suggested_offer,
        SuggestedRoomsOffer::None
    );
}

#[test]
fn a_space_that_was_not_just_joined_offers_nothing() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomJoinedLocally {
            room_id: "!unrelated:example.invalid".to_owned(),
        },
    );
    load(&mut state, false);
    assert_eq!(
        state.space_children.suggested_offer,
        SuggestedRoomsOffer::None
    );
}

#[test]
fn the_offer_survives_a_selection_change() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomJoinedLocally {
            room_id: SPACE_ID.to_owned(),
        },
    );
    load(&mut state, false);
    reduce(&mut state, AppAction::SelectSpace { space_id: None });
    assert_eq!(offered_room_ids(&state), [SUGGESTED]);
}

#[test]
fn bulk_join_reports_each_failure_and_clears_on_full_success() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomJoinedLocally {
            room_id: SPACE_ID.to_owned(),
        },
    );
    load(&mut state, false);

    let effects = reduce(
        &mut state,
        AppAction::SuggestedRoomsJoinRequested {
            request_id: 7,
            space_id: SPACE_ID.to_owned(),
        },
    );
    assert!(emits_children_changed(&effects));
    assert!(matches!(
        state.space_children.suggested_offer,
        SuggestedRoomsOffer::Joining { request_id: 7, .. }
    ));
    // Joining cannot be dismissed or requested again.
    assert!(
        reduce(
            &mut state,
            AppAction::SuggestedRoomsOfferDismissed {
                space_id: SPACE_ID.to_owned(),
            },
        )
        .is_empty()
    );
    assert!(
        reduce(
            &mut state,
            AppAction::SuggestedRoomsJoinRequested {
                request_id: 8,
                space_id: SPACE_ID.to_owned(),
            },
        )
        .is_empty()
    );
    // A stale settle is fenced.
    assert!(
        reduce(
            &mut state,
            AppAction::SuggestedRoomsJoinSettled {
                request_id: 6,
                space_id: SPACE_ID.to_owned(),
                outcomes: Vec::new(),
            },
        )
        .is_empty()
    );

    reduce(
        &mut state,
        AppAction::SuggestedRoomsJoinSettled {
            request_id: 7,
            space_id: SPACE_ID.to_owned(),
            outcomes: vec![SuggestedRoomJoinOutcome {
                room_id: SUGGESTED.to_owned(),
                failure: Some(OperationFailureKind::Forbidden),
            }],
        },
    );
    let SuggestedRoomsOffer::Completed { results, .. } = &state.space_children.suggested_offer
    else {
        panic!(
            "a failure is reported: {:?}",
            state.space_children.suggested_offer
        );
    };
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].display_name, "Zulu");
    assert_eq!(results[0].failure, Some(OperationFailureKind::Forbidden));

    reduce(
        &mut state,
        AppAction::SuggestedRoomsOfferDismissed {
            space_id: SPACE_ID.to_owned(),
        },
    );
    assert_eq!(
        state.space_children.suggested_offer,
        SuggestedRoomsOffer::None
    );
}

#[test]
fn a_fully_successful_bulk_join_closes_the_offer() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomJoinedLocally {
            room_id: SPACE_ID.to_owned(),
        },
    );
    load(&mut state, false);
    reduce(
        &mut state,
        AppAction::SuggestedRoomsJoinRequested {
            request_id: 1,
            space_id: SPACE_ID.to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::SuggestedRoomsJoinSettled {
            request_id: 1,
            space_id: SPACE_ID.to_owned(),
            outcomes: vec![SuggestedRoomJoinOutcome {
                room_id: SUGGESTED.to_owned(),
                failure: None,
            }],
        },
    );
    assert_eq!(
        state.space_children.suggested_offer,
        SuggestedRoomsOffer::None
    );
}

#[test]
fn leaving_the_space_withdraws_the_offer() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomJoinedLocally {
            room_id: SPACE_ID.to_owned(),
        },
    );
    load(&mut state, false);
    let effects = reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: Vec::new(),
        },
    );
    assert!(emits_children_changed(&effects));
    assert_eq!(
        state.space_children.suggested_offer,
        SuggestedRoomsOffer::None
    );
}

#[test]
fn suggested_toggle_is_admitted_for_admins_one_at_a_time() {
    let mut state = ready_state();
    load(&mut state, false);
    let request =
        |request_id: u64, suggested: bool| AppAction::SpaceChildSuggestedUpdateRequested {
            request_id,
            space_id: SPACE_ID.to_owned(),
            child_room_id: PLAIN.to_owned(),
            suggested,
        };
    // Not an admin: rejected.
    assert!(reduce(&mut state, request(1, true)).is_empty());

    state.space_children.generation += 1;
    load(&mut state, true);
    assert!(!reduce(&mut state, request(2, true)).is_empty());
    // A second toggle while saving is ignored.
    assert!(reduce(&mut state, request(3, false)).is_empty());

    reduce(
        &mut state,
        AppAction::SpaceChildSuggestedUpdateSettled {
            request_id: 2,
            space_id: SPACE_ID.to_owned(),
            child_room_id: PLAIN.to_owned(),
            suggested: true,
            failure: None,
        },
    );
    assert_eq!(
        state.space_children.suggested_edit,
        SpaceChildSuggestedEditState::Idle
    );
    assert!(
        state
            .space_children
            .children
            .iter()
            .find(|child| child.room_id == PLAIN)
            .unwrap()
            .suggested
    );

    reduce(&mut state, request(4, false));
    reduce(
        &mut state,
        AppAction::SpaceChildSuggestedUpdateSettled {
            request_id: 4,
            space_id: SPACE_ID.to_owned(),
            child_room_id: PLAIN.to_owned(),
            suggested: false,
            failure: Some(OperationFailureKind::Forbidden),
        },
    );
    assert!(matches!(
        state.space_children.suggested_edit,
        SpaceChildSuggestedEditState::Failed {
            failure: OperationFailureKind::Forbidden,
            ..
        }
    ));
    // The flag is unchanged by a failure.
    assert!(
        state
            .space_children
            .children
            .iter()
            .find(|child| child.room_id == PLAIN)
            .unwrap()
            .suggested
    );
}

#[test]
fn a_suggested_room_already_joined_is_not_offered() {
    let mut state = ready_state();
    state.rooms = vec![joined_room(SUGGESTED, "Zulu")];
    reduce(
        &mut state,
        AppAction::RoomJoinedLocally {
            room_id: SPACE_ID.to_owned(),
        },
    );
    load(&mut state, false);
    assert_eq!(
        state.space_children.suggested_offer,
        SuggestedRoomsOffer::None
    );
}
