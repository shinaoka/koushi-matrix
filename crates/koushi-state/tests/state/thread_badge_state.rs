//! #1238: the room badge composes main-timeline and thread counters.
//!
//! The decomposition is non-overlapping by construction (`RoomReadReceiptEventFilter`
//! excludes thread replies from `unread_count`; each `ThreadReadReceiptEventFilter`
//! counts one thread), so the badge sums the two client counters while a
//! server `notification_count` cannot add an unlocatable number. `unread_count`
//! stays the main-only navigation value.

use koushi_state::{
    AppAction, AppState, RoomNotificationMode, RoomSummary, RoomTags, SessionInfo, SessionState,
    compose_sidebar_for_state, reduce, room_activity_unread_count, room_attention_projection,
};

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://matrix.example.invalid".to_owned(),
            user_id: "@badge:example.invalid".to_owned(),
            device_id: "BADGE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        ..AppState::default()
    }
}

fn room(unread_count: u64, notification_count: u64, highlight_count: u64) -> RoomSummary {
    RoomSummary {
        room_id: "!room:example.invalid".to_owned(),
        display_name: "Synthetic Room".to_owned(),
        display_name_placeholder: None,
        display_label: "Synthetic Room".to_owned(),
        original_display_label: "Synthetic Room".to_owned(),
        display_label_placeholder: None,
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count,
        notification_count,
        highlight_count,
        thread_unread_count: 0,
        thread_highlight_count: 0,
        marked_unread: false,
        recency_stamp: None,
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 2,
    }
}

#[test]
fn main_and_thread_unread_compose_without_overlap() {
    let mut summary = room(1, 0, 0);
    summary.thread_unread_count = 3;

    // The badge is main plus threads: 1 + 3, not 3 (threads alone) and not a
    // doubled count of an overlapping server counter.
    assert_eq!(room_activity_unread_count(&summary), 4);
    // The navigation value stays main-only.
    assert_eq!(summary.unread_count, 1);
}

#[test]
fn server_notification_count_without_locatable_messages_does_not_badge() {
    let mut summary = room(0, 5, 0);
    summary.thread_unread_count = 3;

    // The server supplies only a count, so it cannot identify five unread
    // replies. Only the three observed replies may appear in the red badge.
    assert_eq!(room_activity_unread_count(&summary), 3);
}

#[test]
fn thread_only_unread_marks_the_room_as_having_unread_content() {
    let mut summary = room(0, 0, 0);
    summary.thread_unread_count = 2;

    assert_eq!(room_activity_unread_count(&summary), 2);
    let projection = room_attention_projection(&summary, None);
    assert!(projection.has_unread_content);
    assert_eq!(projection.unread_count, 2);
    assert_eq!(projection.display_count, 2);
}

#[test]
fn thread_mention_renders_mention_styling_instead_of_a_plain_count() {
    let mut summary = room(0, 0, 0);
    summary.thread_unread_count = 1;
    summary.thread_highlight_count = 1;

    let projection = room_attention_projection(&summary, None);
    // One mention is one unread message that also highlights; the max structure
    // counts it once.
    assert_eq!(room_activity_unread_count(&summary), 1);
    assert!(projection.has_unread_mention);
    assert!(projection.is_attention_highlighted);
    assert_eq!(projection.highlight_count, 1);
    // Mentions-mode filtering keeps the room because the thread mention is a
    // highlight.
    let mentions = room_attention_projection(&summary, Some(RoomNotificationMode::Mentions));
    assert_eq!(mentions.notification_count, 0);
    assert!(mentions.has_unread_mention);
}

#[test]
fn manually_marked_unread_does_not_fabricate_a_numeric_count() {
    let summary = room(0, 0, 0);
    assert_eq!(room_activity_unread_count(&summary), 0);

    let mut marked = room(0, 0, 0);
    marked.marked_unread = true;
    assert_eq!(room_activity_unread_count(&marked), 0);
    assert!(room_attention_projection(&marked, None).has_unread_content);
}

#[test]
fn muted_room_display_count_includes_thread_unread() {
    let mut summary = room(0, 0, 0);
    summary.thread_unread_count = 2;

    let projection = room_attention_projection(&summary, Some(RoomNotificationMode::Mute));
    assert_eq!(projection.display_count, 2);
}

#[test]
fn thread_observation_fills_the_room_badge_and_survives_a_room_list_snapshot() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![room(1, 0, 0)],
        },
    );
    assert_eq!(
        room_activity_unread_count(&state.rooms[0]),
        1,
        "the room starts with only its main-timeline unread"
    );

    reduce(
        &mut state,
        AppAction::ThreadUnreadObserved {
            room_id: "!room:example.invalid".to_owned(),
            root_event_id: "$root:example.invalid".to_owned(),
            unread: 3,
            highlight: 0,
        },
    );
    assert_eq!(state.rooms[0].thread_unread_count, 3);
    assert_eq!(state.rooms[0].unread_count, 1, "navigation stays main-only");
    assert_eq!(room_activity_unread_count(&state.rooms[0]), 4);
    assert_eq!(state.native_attention.summary.badge_count, 4);
    assert_eq!(
        compose_sidebar_for_state(&state).account_home.unread_count,
        4
    );

    // A later room-list snapshot does not carry the thread counters; the reducer
    // re-derives them instead of dropping the badge.
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![room(1, 0, 0)],
        },
    );
    assert_eq!(state.rooms[0].thread_unread_count, 3);
    assert_eq!(room_activity_unread_count(&state.rooms[0]), 4);

    // A threaded read clears only the thread term.
    reduce(
        &mut state,
        AppAction::ThreadUnreadObserved {
            room_id: "!room:example.invalid".to_owned(),
            root_event_id: "$root:example.invalid".to_owned(),
            unread: 0,
            highlight: 0,
        },
    );
    assert_eq!(state.rooms[0].thread_unread_count, 0);
    assert_eq!(room_activity_unread_count(&state.rooms[0]), 1);
    assert_eq!(state.native_attention.summary.badge_count, 1);
}

#[test]
fn leaving_and_rejoining_does_not_reuse_old_thread_unread() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![room(0, 0, 0)],
        },
    );
    let observation = || AppAction::ThreadUnreadObserved {
        room_id: "!room:example.invalid".to_owned(),
        root_event_id: "$root:example.invalid".to_owned(),
        unread: 3,
        highlight: 0,
    };
    reduce(&mut state, observation());
    assert_eq!(state.rooms[0].thread_unread_count, 3);

    reduce(
        &mut state,
        AppAction::RoomLeftLocally {
            room_id: "!room:example.invalid".to_owned(),
        },
    );
    assert!(state.rooms.is_empty());
    assert!(state.thread_unread.is_empty());
    // A queued observation from the old actor must not survive the leave.
    reduce(&mut state, observation());
    assert!(state.thread_unread.is_empty());

    reduce(
        &mut state,
        AppAction::RoomJoinedLocally {
            room_id: "!room:example.invalid".to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![room(0, 0, 0)],
        },
    );
    assert_eq!(state.rooms[0].thread_unread_count, 0);
    assert_eq!(room_activity_unread_count(&state.rooms[0]), 0);
}

#[test]
fn an_authoritative_room_removal_forgets_thread_unread() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![room(0, 0, 0)],
        },
    );
    reduce(
        &mut state,
        AppAction::ThreadUnreadObserved {
            room_id: "!room:example.invalid".to_owned(),
            root_event_id: "$root:example.invalid".to_owned(),
            unread: 2,
            highlight: 0,
        },
    );
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: Vec::new(),
        },
    );
    assert!(state.thread_unread.is_empty());
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![room(0, 0, 0)],
        },
    );
    assert_eq!(state.rooms[0].thread_unread_count, 0);
}

#[test]
fn a_main_timeline_read_never_clears_thread_unread() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![room(1, 1, 0)],
        },
    );
    reduce(
        &mut state,
        AppAction::ThreadUnreadObserved {
            room_id: "!room:example.invalid".to_owned(),
            root_event_id: "$root:example.invalid".to_owned(),
            unread: 3,
            highlight: 0,
        },
    );
    assert_eq!(room_activity_unread_count(&state.rooms[0]), 4);

    reduce(
        &mut state,
        AppAction::RoomMarkedAsReadSucceeded {
            request_id: 1,
            room_id: "!room:example.invalid".to_owned(),
        },
    );

    // Reading the main timeline clears exactly the main counters.
    assert_eq!(state.rooms[0].unread_count, 0);
    assert_eq!(state.rooms[0].notification_count, 0);
    assert_eq!(state.rooms[0].highlight_count, 0);
    // The unopened thread stays unread, so the room still badges.
    assert_eq!(state.rooms[0].thread_unread_count, 3);
    assert_eq!(room_activity_unread_count(&state.rooms[0]), 3);
    assert!(room_attention_projection(&state.rooms[0], None).has_unread_content);
}
