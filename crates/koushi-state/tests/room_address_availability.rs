//! Advisory create-room address availability (#1006).

use koushi_state::{
    AppAction, AppState, RoomAddressAvailability, RoomAddressAvailabilityState,
    RoomAddressSuggestion, SessionInfo, SessionState, reduce,
    suggest_alternative_room_alias_localpart,
};

fn ready() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "http://127.0.0.1:6167".to_owned(),
            user_id: "@member:example.invalid".to_owned(),
            device_id: "DEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        ..AppState::default()
    }
}

fn request(state: &mut AppState, request_id: u64, alias: &str) {
    reduce(
        state,
        AppAction::RoomAddressAvailabilityRequested {
            request_id,
            full_alias: alias.to_owned(),
        },
    );
}

fn settle(
    state: &mut AppState,
    request_id: u64,
    alias: &str,
    availability: RoomAddressAvailability,
    suggestion: Option<RoomAddressSuggestion>,
) {
    reduce(
        state,
        AppAction::RoomAddressAvailabilitySettled {
            request_id,
            full_alias: alias.to_owned(),
            availability,
            suggestion,
        },
    );
}

fn suggestion() -> RoomAddressSuggestion {
    RoomAddressSuggestion {
        localpart: "papers-2".to_owned(),
        full_alias: "#papers-2:example.invalid".to_owned(),
    }
}

#[test]
fn a_check_settles_in_use_with_an_unchecked_suggestion() {
    let mut state = ready();
    request(&mut state, 1, "#papers:example.invalid");
    assert_eq!(
        state.room_address_availability,
        RoomAddressAvailabilityState::Checking {
            request_id: 1,
            full_alias: "#papers:example.invalid".to_owned()
        }
    );
    settle(
        &mut state,
        1,
        "#papers:example.invalid",
        RoomAddressAvailability::InUse,
        Some(suggestion()),
    );
    assert_eq!(
        state.room_address_availability,
        RoomAddressAvailabilityState::Checked {
            request_id: 1,
            full_alias: "#papers:example.invalid".to_owned(),
            availability: RoomAddressAvailability::InUse,
            suggestion: Some(suggestion()),
        }
    );
}

#[test]
fn only_an_in_use_result_carries_a_suggestion() {
    for availability in [
        RoomAddressAvailability::Available,
        RoomAddressAvailability::Unknown,
    ] {
        let mut state = ready();
        request(&mut state, 1, "#papers:example.invalid");
        settle(
            &mut state,
            1,
            "#papers:example.invalid",
            availability,
            Some(suggestion()),
        );
        assert!(matches!(
            state.room_address_availability,
            RoomAddressAvailabilityState::Checked { suggestion: None, availability: a, .. }
                if a == availability
        ));
    }
}

#[test]
fn a_stale_result_for_an_earlier_draft_is_ignored() {
    let mut state = ready();
    request(&mut state, 1, "#papers:example.invalid");
    request(&mut state, 2, "#papers-2026:example.invalid");
    // The earlier lookup completes after the draft changed.
    settle(
        &mut state,
        1,
        "#papers:example.invalid",
        RoomAddressAvailability::InUse,
        Some(suggestion()),
    );
    assert!(matches!(
        state.room_address_availability,
        RoomAddressAvailabilityState::Checking { request_id: 2, .. }
    ));
    // A result whose address does not match the pending check is ignored too.
    settle(
        &mut state,
        2,
        "#papers:example.invalid",
        RoomAddressAvailability::Available,
        None,
    );
    assert!(matches!(
        state.room_address_availability,
        RoomAddressAvailabilityState::Checking { request_id: 2, .. }
    ));
    settle(
        &mut state,
        2,
        "#papers-2026:example.invalid",
        RoomAddressAvailability::Available,
        None,
    );
    assert!(matches!(
        state.room_address_availability,
        RoomAddressAvailabilityState::Checked {
            availability: RoomAddressAvailability::Available,
            ..
        }
    ));
    // A duplicate completion does not replace the settled result.
    settle(
        &mut state,
        2,
        "#papers-2026:example.invalid",
        RoomAddressAvailability::InUse,
        None,
    );
    assert!(matches!(
        state.room_address_availability,
        RoomAddressAvailabilityState::Checked {
            availability: RoomAddressAvailability::Available,
            ..
        }
    ));
}

#[test]
fn clearing_and_signing_out_drop_the_check_and_late_results() {
    let mut state = ready();
    request(&mut state, 1, "#papers:example.invalid");
    reduce(&mut state, AppAction::RoomAddressAvailabilityCleared);
    assert_eq!(
        state.room_address_availability,
        RoomAddressAvailabilityState::Idle
    );
    settle(
        &mut state,
        1,
        "#papers:example.invalid",
        RoomAddressAvailability::Available,
        None,
    );
    assert_eq!(
        state.room_address_availability,
        RoomAddressAvailabilityState::Idle
    );

    request(&mut state, 2, "#papers:example.invalid");
    reduce(&mut state, AppAction::LogoutRequested);
    assert_eq!(
        state.room_address_availability,
        RoomAddressAvailabilityState::Idle
    );
}

#[test]
fn a_check_needs_a_ready_session() {
    let mut state = AppState::default();
    request(&mut state, 1, "#papers:example.invalid");
    assert_eq!(
        state.room_address_availability,
        RoomAddressAvailabilityState::Idle
    );
}

#[test]
fn alternatives_increment_a_trailing_number_or_append_two() {
    assert_eq!(
        suggest_alternative_room_alias_localpart("papers"),
        "papers-2"
    );
    assert_eq!(
        suggest_alternative_room_alias_localpart("papers-2"),
        "papers-3"
    );
    assert_eq!(
        suggest_alternative_room_alias_localpart("research-group-papers"),
        "research-group-papers-2"
    );
    assert_eq!(
        suggest_alternative_room_alias_localpart("papers-2026"),
        "papers-2027"
    );
    assert_eq!(suggest_alternative_room_alias_localpart("設計"), "設計-2");
    assert_eq!(suggest_alternative_room_alias_localpart("-5"), "-5-2");
}
