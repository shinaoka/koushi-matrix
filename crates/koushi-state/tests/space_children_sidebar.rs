//! Issue #961: a Space shows every child room it advertises, with the
//! account's membership beside it.
//!
//! The joined room list stays authoritative for everything it contains: these
//! tests pin that the not-joined lane never duplicates, replaces or reorders a
//! joined room, and that the account's own invite list decides what counts as
//! an invitation.

use koushi_state::{
    AppAction, AppState, InvitePreview, RoomListSource, RoomNamePlaceholder, RoomSummary, RoomTags,
    SessionInfo, SessionState, SpaceChildMembership, SpaceChildSummary, SpaceChildrenState,
    SpaceSummary, compose_sidebar_for_state, reduce,
};

const SPACE_ID: &str = "!space:example.invalid";

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

fn child(room_id: &str, label: &str, membership: SpaceChildMembership) -> SpaceChildSummary {
    SpaceChildSummary {
        room_id: room_id.to_owned(),
        display_name: label.to_owned(),
        display_name_placeholder: None,
        avatar: None,
        membership,
        can_join: matches!(
            membership,
            SpaceChildMembership::NotJoined | SpaceChildMembership::Invited
        ),
        is_space: false,
        joined_members: 4,
    }
}

fn state_with_children(children: Vec<SpaceChildSummary>) -> AppState {
    let mut state = AppState::default();
    state.navigation.active_space_id = Some(SPACE_ID.to_owned());
    state.spaces = vec![SpaceSummary {
        space_id: SPACE_ID.to_owned(),
        raw_name: Some("Space".to_owned()),
        display_name: "Space".to_owned(),
        avatar: None,
        join_rule: None,
        child_room_ids: vec!["!joined:example.invalid".to_owned()],
    }];
    state.rooms = vec![joined_room("!joined:example.invalid", "Joined Room")];
    state.space_children = SpaceChildrenState {
        selected_space_id: Some(SPACE_ID.to_owned()),
        generation: 1,
        children,
        load: Default::default(),
    };
    state
}

fn names(items: &[koushi_state::RoomListItem]) -> Vec<&str> {
    items
        .iter()
        .map(|item| item.display_name.as_str())
        .collect()
}

#[test]
fn not_joined_children_get_their_own_lane_without_touching_the_joined_rooms() {
    let state = state_with_children(vec![
        child(
            "!joined:example.invalid",
            "Joined Room",
            SpaceChildMembership::Joined,
        ),
        child(
            "!open:example.invalid",
            "Open Room",
            SpaceChildMembership::NotJoined,
        ),
        child(
            "!private:example.invalid",
            "!private:example.invalid",
            SpaceChildMembership::Unknown,
        ),
    ]);

    let sidebar = compose_sidebar_for_state(&state);

    assert_eq!(names(&sidebar.sections.rooms), ["Joined Room"]);
    // Encrypted/private children can be undescribed by `/hierarchy`, but they
    // still belong in the Space's room list so they do not disappear.
    assert_eq!(
        names(&sidebar.sections.not_joined),
        ["!private:example.invalid", "Open Room"]
    );
    assert_eq!(
        sidebar.sections.rooms[0].membership,
        SpaceChildMembership::Joined
    );
    assert_eq!(sidebar.sections.rooms[0].unread_count, 3);
    // A room the account is not in has no read state to report.
    assert!(
        sidebar
            .sections
            .not_joined
            .iter()
            .all(|item| item.unread_count == 0 && item.highlight_count == 0)
    );
}

#[test]
fn a_joined_room_is_never_repeated_in_the_not_joined_lane() {
    // A hierarchy response that crossed a join still calls the room not joined.
    // The account's own room list is the authority.
    let state = state_with_children(vec![child(
        "!joined:example.invalid",
        "Joined Room",
        SpaceChildMembership::NotJoined,
    )]);

    let sidebar = compose_sidebar_for_state(&state);

    assert_eq!(names(&sidebar.sections.rooms), ["Joined Room"]);
    assert!(sidebar.sections.not_joined.is_empty());
    assert!(sidebar.not_joined_space_rooms.is_empty());
}

#[test]
fn a_pending_invitation_is_reported_as_invited_inside_its_space() {
    let mut state = state_with_children(vec![child(
        "!invited:example.invalid",
        "Invited Room",
        SpaceChildMembership::NotJoined,
    )]);
    state.invites = vec![InvitePreview {
        room_id: "!invited:example.invalid".to_owned(),
        display_name: "Invited Room".to_owned(),
        display_name_placeholder: None,
        avatar: None,
        topic: None,
        inviter_display_name: None,
        inviter_user_id: None,
        is_dm: false,
        is_space: false,
    }];

    let sidebar = compose_sidebar_for_state(&state);

    assert_eq!(names(&sidebar.sections.not_joined), ["Invited Room"]);
    assert_eq!(
        sidebar.sections.not_joined[0].membership,
        SpaceChildMembership::Invited
    );
}

#[test]
fn another_spaces_children_never_leak_into_the_selected_space() {
    let mut state = state_with_children(vec![child(
        "!elsewhere:example.invalid",
        "Elsewhere",
        SpaceChildMembership::NotJoined,
    )]);
    state.space_children.selected_space_id = Some("!other-space:example.invalid".to_owned());

    let sidebar = compose_sidebar_for_state(&state);

    assert!(sidebar.sections.not_joined.is_empty());
}

/// Issue #961 acceptance: "Roomの参加・退出・招待の状態変化が一覧へ反映される".
/// The hierarchy projection is a cached server summary; the account's own room
/// and invite lists move first, and a left room must reappear in the Space
/// rather than disappear from it.
#[test]
fn leaving_a_room_returns_it_to_the_not_joined_lane_before_the_next_hierarchy_fetch() {
    let mut state = state_with_children(vec![child(
        "!joined:example.invalid",
        "Joined Room",
        SpaceChildMembership::Joined,
    )]);
    // The user left; the joined room list no longer carries it, while the
    // cached projection still says Joined.
    state.rooms.clear();

    let sidebar = compose_sidebar_for_state(&state);

    assert!(sidebar.sections.rooms.is_empty());
    assert_eq!(names(&sidebar.sections.not_joined), ["Joined Room"]);
    assert_eq!(
        sidebar.sections.not_joined[0].membership,
        SpaceChildMembership::NotJoined
    );
}

#[test]
fn a_declined_invitation_stops_being_reported_as_invited() {
    let state = state_with_children(vec![child(
        "!invited:example.invalid",
        "Invited Room",
        SpaceChildMembership::Invited,
    )]);
    // `state.invites` is empty: the invitation is gone.

    let sidebar = compose_sidebar_for_state(&state);

    assert!(sidebar.sections.not_joined.is_empty());
}

#[test]
fn a_successful_leave_removes_the_room_from_the_joined_projection_immediately() {
    let mut state = state_with_children(vec![child(
        "!joined:example.invalid",
        "Joined Room",
        SpaceChildMembership::Joined,
    )]);
    state.session = SessionState::Ready(SessionInfo {
        homeserver: "https://example.invalid".to_owned(),
        user_id: "@alice:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    });

    let effects = reduce(
        &mut state,
        AppAction::RoomLeftLocally {
            room_id: "!joined:example.invalid".to_owned(),
        },
    );

    assert!(state.rooms.is_empty());
    assert_eq!(
        names(&compose_sidebar_for_state(&state).sections.not_joined),
        ["Joined Room"]
    );
    assert!(effects.iter().any(|effect| matches!(
        effect,
        koushi_state::AppEffect::EmitUiEvent(koushi_state::UiEvent::RoomListChanged)
    )));

    // A stale provisional snapshot from the live room-list service must not
    // resurrect the room while the server processes the leave.
    reduce(
        &mut state,
        AppAction::RoomListSnapshotProvisional {
            generation: 0,
            source: RoomListSource::Cache,
            spaces: Vec::new(),
            rooms: vec![joined_room("!joined:example.invalid", "Joined Room")],
            invites: Vec::new(),
        },
    );
    assert!(state.rooms.is_empty());
}

#[test]
fn an_undescribed_child_stays_visible_in_the_room_list() {
    let state = state_with_children(vec![child(
        "!private:example.invalid",
        "!private:example.invalid",
        SpaceChildMembership::Unknown,
    )]);

    let sidebar = compose_sidebar_for_state(&state);

    assert_eq!(
        names(&sidebar.sections.not_joined),
        ["!private:example.invalid"]
    );
    assert_eq!(
        sidebar.sections.not_joined[0].membership,
        SpaceChildMembership::Unknown
    );
}

/// Only rows that can actually be joined carry the affordance: everything else
/// would fire a request the server rejects.
#[test]
fn only_joinable_rows_carry_a_join_affordance() {
    let mut state = state_with_children(vec![
        child(
            "!open:example.invalid",
            "Open Room",
            SpaceChildMembership::NotJoined,
        ),
        SpaceChildSummary {
            can_join: false,
            ..child(
                "!banned:example.invalid",
                "Banned Room",
                SpaceChildMembership::Banned,
            )
        },
        SpaceChildSummary {
            can_join: false,
            ..child(
                "!invite-only:example.invalid",
                "Invite Only",
                SpaceChildMembership::NotJoined,
            )
        },
    ]);
    state.invites = vec![InvitePreview {
        room_id: "!pending:example.invalid".to_owned(),
        display_name: "Pending Room".to_owned(),
        display_name_placeholder: None,
        avatar: None,
        topic: None,
        inviter_display_name: None,
        inviter_user_id: None,
        is_dm: false,
        is_space: false,
    }];
    state.space_children.children.push(SpaceChildSummary {
        can_join: false,
        ..child(
            "!pending:example.invalid",
            "Pending Room",
            SpaceChildMembership::NotJoined,
        )
    });

    let sidebar = compose_sidebar_for_state(&state);

    let joinable: Vec<(&str, bool)> = sidebar
        .sections
        .not_joined
        .iter()
        .map(|item| (item.display_name.as_str(), item.can_join))
        .collect();
    assert_eq!(
        joinable,
        [
            ("Banned Room", false),
            ("Invite Only", false),
            ("Open Room", true),
            // An invitation this account holds can always be accepted.
            ("Pending Room", true),
        ]
    );
    assert!(sidebar.sections.rooms.iter().all(|item| !item.can_join));
}

fn ready(state: &mut AppState) {
    state.session = SessionState::Ready(SessionInfo {
        homeserver: "https://example.invalid".to_owned(),
        user_id: "@alice:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    });
}

fn refresh_requests(effects: &[koushi_state::AppEffect]) -> Vec<(String, u64)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            koushi_state::AppEffect::LoadSpaceChildren {
                space_id,
                generation,
            } => Some((space_id.clone(), *generation)),
            _ => None,
        })
        .collect()
}

/// Issue #1062: when the account was the child's last joined member, leaving
/// empties the room. It has nobody to rejoin and no invitation, so it must not
/// linger in the Not joined lane as an inert row until the Space is reselected.
#[test]
fn the_last_member_leaving_removes_the_child_from_the_not_joined_lane() {
    let mut state = state_with_children(vec![child(
        "!joined:example.invalid",
        "Joined Room",
        SpaceChildMembership::Joined,
    )]);
    state.rooms[0].joined_members = 1;
    ready(&mut state);

    reduce(
        &mut state,
        AppAction::RoomLeftLocally {
            room_id: "!joined:example.invalid".to_owned(),
        },
    );

    let sidebar = compose_sidebar_for_state(&state);
    assert!(sidebar.sections.rooms.is_empty());
    assert!(
        sidebar.sections.not_joined.is_empty(),
        "an emptied child is not offered: {:?}",
        names(&sidebar.sections.not_joined)
    );
}

/// Issue #1062: a leave makes the cached `/hierarchy` projection stale (its
/// join rule, membership and count all predate the leave), so Rust asks for a
/// fresh one under a new generation while the Space stays selected. A response
/// that was already in flight for the old generation cannot overwrite it.
#[test]
fn leaving_a_child_refreshes_the_selected_spaces_children() {
    let mut state = state_with_children(vec![child(
        "!joined:example.invalid",
        "Joined Room",
        SpaceChildMembership::Joined,
    )]);
    ready(&mut state);

    let effects = reduce(
        &mut state,
        AppAction::RoomLeftLocally {
            room_id: "!joined:example.invalid".to_owned(),
        },
    );

    assert_eq!(refresh_requests(&effects), [(SPACE_ID.to_owned(), 2)]);
    assert_eq!(state.space_children.generation, 2);
    assert_eq!(
        state.space_children.load,
        koushi_state::SpaceChildrenLoadState::Loading
    );
    // Still a member-bearing room, so it stays in the lane meanwhile, but the
    // cached summary described a joined room: it grants no Join affordance.
    let sidebar = compose_sidebar_for_state(&state);
    assert_eq!(names(&sidebar.sections.not_joined), ["Joined Room"]);
    assert!(!sidebar.sections.not_joined[0].can_join);

    // A pre-leave response for the old generation is fenced.
    reduce(
        &mut state,
        AppAction::SpaceChildrenLoaded {
            space_id: SPACE_ID.to_owned(),
            generation: 1,
            children: vec![child(
                "!joined:example.invalid",
                "Joined Room",
                SpaceChildMembership::Joined,
            )],
        },
    );
    assert_eq!(
        state.space_children.children[0].membership,
        SpaceChildMembership::Left
    );

    // The fresh projection decides.
    reduce(
        &mut state,
        AppAction::SpaceChildrenLoaded {
            space_id: SPACE_ID.to_owned(),
            generation: 2,
            children: vec![child(
                "!joined:example.invalid",
                "Joined Room",
                SpaceChildMembership::Left,
            )],
        },
    );
    let sidebar = compose_sidebar_for_state(&state);
    assert_eq!(names(&sidebar.sections.not_joined), ["Joined Room"]);
    assert_eq!(
        state.space_children.load,
        koushi_state::SpaceChildrenLoadState::Idle
    );
}

/// Issue #1062: leaving a child while the Space's first children load is still
/// in flight fences that pre-leave answer and asks again.
#[test]
fn leaving_a_child_during_the_first_load_fences_it_and_reloads() {
    let mut state = state_with_children(Vec::new());
    state.space_children.load = koushi_state::SpaceChildrenLoadState::Loading;
    ready(&mut state);

    let effects = reduce(
        &mut state,
        AppAction::RoomLeftLocally {
            room_id: "!joined:example.invalid".to_owned(),
        },
    );

    assert_eq!(refresh_requests(&effects), [(SPACE_ID.to_owned(), 2)]);
    reduce(
        &mut state,
        AppAction::SpaceChildrenLoaded {
            space_id: SPACE_ID.to_owned(),
            generation: 1,
            children: vec![child(
                "!joined:example.invalid",
                "Joined Room",
                SpaceChildMembership::Joined,
            )],
        },
    );
    assert!(state.space_children.children.is_empty());
}

#[test]
fn leaving_a_room_outside_the_selected_space_requests_no_refresh() {
    let mut state = state_with_children(vec![child(
        "!other:example.invalid",
        "Other Room",
        SpaceChildMembership::NotJoined,
    )]);
    // The joined room belongs to no Space.
    state.spaces[0].child_room_ids.clear();
    ready(&mut state);

    let effects = reduce(
        &mut state,
        AppAction::RoomLeftLocally {
            room_id: "!joined:example.invalid".to_owned(),
        },
    );

    assert!(refresh_requests(&effects).is_empty());
    assert_eq!(state.space_children.generation, 1);
}

/// #1070: the Not-joined lane carries a child's structured empty-room name,
/// so the GUI renders catalog text instead of the SDK's English "Empty Room".
#[test]
fn not_joined_lane_keeps_the_empty_room_placeholder() {
    let state = state_with_children(vec![
        SpaceChildSummary {
            display_name_placeholder: Some(RoomNamePlaceholder::Empty),
            ..child(
                "!unnamed:example.invalid",
                "Empty Room",
                SpaceChildMembership::NotJoined,
            )
        },
        child(
            "!literal:example.invalid",
            "Empty Room",
            SpaceChildMembership::NotJoined,
        ),
    ]);

    let sidebar = compose_sidebar_for_state(&state);

    let placeholders: Vec<(&str, Option<&RoomNamePlaceholder>)> = sidebar
        .sections
        .not_joined
        .iter()
        .map(|item| {
            (
                item.room_id.as_str(),
                item.display_name_placeholder.as_ref(),
            )
        })
        .collect();
    assert_eq!(
        placeholders,
        [
            ("!literal:example.invalid", None),
            (
                "!unnamed:example.invalid",
                Some(&RoomNamePlaceholder::Empty)
            ),
        ]
    );
}
