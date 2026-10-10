//! #1050: the SDK's calculated empty-room name is projected as a structured
//! placeholder so the GUI renders catalog text instead of English.

use koushi_state::{
    AppState, ProfileState, RoomNamePlaceholder, RoomSummary, RoomTags, UserProfile,
    compose_sidebar_for_state, refresh_room_summary_display_projection,
};

const OWN_USER_ID: &str = "@own:example.invalid";

fn room(room_id: &str, placeholder: Option<RoomNamePlaceholder>) -> RoomSummary {
    let display_name = match &placeholder {
        Some(RoomNamePlaceholder::Empty) => "Empty Room".to_owned(),
        Some(RoomNamePlaceholder::EmptyWas { previous_names }) => {
            format!("Empty Room (was {previous_names})")
        }
        None => "Named Room".to_owned(),
    };
    RoomSummary {
        room_id: room_id.to_owned(),
        display_name: display_name.clone(),
        display_name_placeholder: placeholder.clone(),
        display_label: display_name.clone(),
        original_display_label: display_name,
        display_label_placeholder: placeholder,
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        thread_unread_count: 0,
        thread_highlight_count: 0,
        marked_unread: false,
        recency_stamp: None,
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 1,
    }
}

fn former_dm(user_id: &str) -> RoomSummary {
    RoomSummary {
        is_dm: true,
        dm_user_ids: vec![user_id.to_owned()],
        ..room(
            "!dm:example.invalid",
            Some(RoomNamePlaceholder::EmptyWas {
                previous_names: "Bob".to_owned(),
            }),
        )
    }
}

fn profile_with(user_id: &str, display_name: &str) -> ProfileState {
    let mut profiles = ProfileState::default();
    profiles.users.insert(
        user_id.to_owned(),
        UserProfile {
            user_id: user_id.to_owned(),
            display_name: Some(display_name.to_owned()),
            display_label: display_name.to_owned(),
            original_display_label: display_name.to_owned(),
            mention_search_terms: Vec::new(),
            avatar: None,
        },
    );
    profiles
}

#[test]
fn an_unnamed_room_keeps_its_placeholder_through_label_projection() {
    let mut rooms = vec![room(
        "!empty:example.invalid",
        Some(RoomNamePlaceholder::Empty),
    )];

    refresh_room_summary_display_projection(
        &mut rooms,
        &ProfileState::default(),
        Some(OWN_USER_ID),
    );

    assert_eq!(rooms[0].display_label, "Empty Room");
    assert_eq!(
        rooms[0].display_label_placeholder,
        Some(RoomNamePlaceholder::Empty)
    );
}

#[test]
fn a_former_dm_resolves_to_its_member_instead_of_the_placeholder() {
    let mut rooms = vec![former_dm("@bob:example.invalid")];
    let profiles = profile_with("@bob:example.invalid", "Bob Builder");

    refresh_room_summary_display_projection(&mut rooms, &profiles, Some(OWN_USER_ID));

    assert_eq!(rooms[0].display_label, "Bob Builder");
    assert_eq!(rooms[0].display_label_placeholder, None);
}

#[test]
fn a_former_dm_without_a_cached_profile_uses_the_identity_fallback() {
    let mut rooms = vec![former_dm("@bob:example.invalid")];

    // Repeated refreshes must keep the same projection: the SDK fact lives in
    // `display_name_placeholder`, not in the derived label placeholder.
    for _ in 0..2 {
        refresh_room_summary_display_projection(
            &mut rooms,
            &ProfileState::default(),
            Some(OWN_USER_ID),
        );
        assert_eq!(rooms[0].display_label, "@bob:example.invalid");
        assert!(!rooms[0].display_label.contains("Empty Room"));
        assert_eq!(rooms[0].display_label_placeholder, None);
    }
}

#[test]
fn a_named_room_has_no_placeholder() {
    let mut rooms = vec![room("!named:example.invalid", None)];

    refresh_room_summary_display_projection(
        &mut rooms,
        &ProfileState::default(),
        Some(OWN_USER_ID),
    );

    assert_eq!(rooms[0].display_label, "Named Room");
    assert_eq!(rooms[0].display_label_placeholder, None);
}

#[test]
fn sidebar_rows_mirror_the_label_placeholder() {
    let state = AppState {
        rooms: vec![room(
            "!empty:example.invalid",
            Some(RoomNamePlaceholder::EmptyWas {
                previous_names: "Alice".to_owned(),
            }),
        )],
        ..AppState::default()
    };

    let sidebar = compose_sidebar_for_state(&state);
    let row = sidebar
        .sections
        .rooms
        .iter()
        .find(|row| row.room_id == "!empty:example.invalid")
        .expect("sidebar row");

    assert_eq!(
        row.display_name_placeholder,
        Some(RoomNamePlaceholder::EmptyWas {
            previous_names: "Alice".to_owned()
        })
    );
}

#[test]
fn placeholder_debug_output_redacts_member_names() {
    let placeholder = RoomNamePlaceholder::EmptyWas {
        previous_names: "Alice".to_owned(),
    };

    assert!(!format!("{placeholder:?}").contains("Alice"));
    assert!(!format!("{:?}", room("!r:example.invalid", Some(placeholder))).contains("Alice"));
}

#[test]
fn placeholder_serializes_as_a_tagged_value() {
    assert_eq!(
        serde_json::to_value(RoomNamePlaceholder::Empty).unwrap(),
        serde_json::json!({ "kind": "empty" })
    );
    assert_eq!(
        serde_json::to_value(RoomNamePlaceholder::EmptyWas {
            previous_names: "Alice".to_owned()
        })
        .unwrap(),
        serde_json::json!({ "kind": "emptyWas", "previous_names": "Alice" })
    );
}
