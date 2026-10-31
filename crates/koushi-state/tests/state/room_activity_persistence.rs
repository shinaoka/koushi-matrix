//! #1364: persisted per-room conversation activity.
//!
//! Canon: `docs/architecture/state-machine.md`, "Sidebar Sections And Low
//! Priority". The reducer remembers the newest conversation activity it has
//! observed per room, merges it into every installed snapshot, and prunes rooms
//! the authoritative list no longer contains.

use koushi_state::{
    AppAction, AppState, ConversationActivity, ConversationActivitySource, RoomActivityRecord,
    RoomActivityState, RoomListSort, RoomSummary, RoomTags, SessionAuthenticationMethod,
    SessionInfo, SessionState, reduce,
};

const ROOM_A: &str = "!room-a:example.invalid";
const ROOM_B: &str = "!room-b:example.invalid";

fn room(room_id: &str, activity: Option<u64>, recency_stamp: u64) -> RoomSummary {
    RoomSummary {
        display_name_placeholder: None,
        display_label_placeholder: None,
        room_id: room_id.to_owned(),
        display_name: room_id.to_owned(),
        display_label: room_id.to_owned(),
        original_display_label: room_id.to_owned(),
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
        recency_stamp: Some(recency_stamp),
        conversation_activity: activity.map(|timestamp_ms| ConversationActivity {
            timestamp_ms,
            source: ConversationActivitySource::Message,
        }),
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 2,
    }
}

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@owner:example.invalid".to_owned(),
            device_id: "DEVICE".to_owned(),
            authentication_method: SessionAuthenticationMethod::Password,
        }),
        ..AppState::default()
    }
}

fn update_room_list(state: &mut AppState, rooms: Vec<RoomSummary>) {
    reduce(
        state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms,
        },
    );
}

fn item_order(state: &AppState) -> Vec<&str> {
    state
        .room_list
        .items
        .iter()
        .map(|item| item.room_id.as_str())
        .collect()
}

/// A restart leaves every room without an in-memory conversation event. The
/// remembered activity must restore the real order instead of falling back to
/// the server's recency stamp.
#[test]
fn persisted_activity_restores_recent_order_after_a_restart() {
    let mut state = ready_state();
    state.settings.values.room_list_sort = RoomListSort::RecentFirst;
    // Only B has server recency: without the cache, B sorts first.
    update_room_list(
        &mut state,
        vec![room(ROOM_A, None, 100), room(ROOM_B, None, 900)],
    );
    assert_eq!(item_order(&state), vec![ROOM_B, ROOM_A]);

    reduce(
        &mut state,
        AppAction::RoomActivityLoaded {
            activity: RoomActivityState {
                rooms: std::collections::BTreeMap::from([(
                    ROOM_A.to_owned(),
                    RoomActivityRecord {
                        timestamp_ms: 1_000,
                        source: ConversationActivitySource::Message,
                    },
                )]),
            },
        },
    );

    let restored = state
        .rooms
        .iter()
        .find(|room| room.room_id == ROOM_A)
        .expect("room A is installed");
    assert_eq!(
        restored.conversation_activity.map(|it| it.timestamp_ms),
        Some(1_000)
    );
    assert_eq!(item_order(&state), vec![ROOM_A, ROOM_B]);
}

/// A room whose newest event is a sticker, poll, call, or state change loses its
/// projected activity. The cache must keep the room where it was.
#[test]
fn a_snapshot_without_activity_keeps_the_remembered_position() {
    let mut state = ready_state();
    state.settings.values.room_list_sort = RoomListSort::RecentFirst;
    update_room_list(
        &mut state,
        vec![room(ROOM_A, Some(1_000), 1_000), room(ROOM_B, None, 900)],
    );
    assert_eq!(item_order(&state), vec![ROOM_A, ROOM_B]);
    assert_eq!(
        state
            .room_activity
            .activity_for(ROOM_A)
            .map(|it| it.timestamp_ms),
        Some(1_000)
    );

    // The next snapshot's newest event for A is a sticker: no activity, but a
    // newer server recency stamp.
    update_room_list(
        &mut state,
        vec![room(ROOM_A, None, 5_000), room(ROOM_B, None, 900)],
    );

    assert_eq!(item_order(&state), vec![ROOM_A, ROOM_B]);
    assert_eq!(
        state
            .rooms
            .iter()
            .find(|room| room.room_id == ROOM_A)
            .and_then(|room| room.conversation_activity)
            .map(|it| it.timestamp_ms),
        Some(1_000)
    );
}

#[test]
fn the_cache_keeps_the_newer_of_the_remembered_and_live_values() {
    // A live value newer than the cache wins.
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomActivityLoaded {
            activity: RoomActivityState {
                rooms: std::collections::BTreeMap::from([(
                    ROOM_A.to_owned(),
                    RoomActivityRecord {
                        timestamp_ms: 100,
                        source: ConversationActivitySource::Message,
                    },
                )]),
            },
        },
    );
    update_room_list(&mut state, vec![room(ROOM_A, Some(500), 500)]);
    assert_eq!(
        state
            .room_activity
            .activity_for(ROOM_A)
            .map(|it| it.timestamp_ms),
        Some(500)
    );

    // A remembered value newer than the live one wins.
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomActivityLoaded {
            activity: RoomActivityState {
                rooms: std::collections::BTreeMap::from([(
                    ROOM_A.to_owned(),
                    RoomActivityRecord {
                        timestamp_ms: 900,
                        source: ConversationActivitySource::ThreadReply,
                    },
                )]),
            },
        },
    );
    update_room_list(&mut state, vec![room(ROOM_A, Some(500), 500)]);
    assert_eq!(
        state
            .room_activity
            .activity_for(ROOM_A)
            .map(|it| it.timestamp_ms),
        Some(900)
    );
    assert_eq!(
        state
            .rooms
            .iter()
            .find(|room| room.room_id == ROOM_A)
            .and_then(|room| room.conversation_activity),
        Some(ConversationActivity {
            timestamp_ms: 900,
            source: ConversationActivitySource::ThreadReply,
        })
    );
}

#[test]
fn the_authoritative_snapshot_prunes_rooms_it_no_longer_lists() {
    let mut state = ready_state();
    update_room_list(
        &mut state,
        vec![room(ROOM_A, Some(100), 100), room(ROOM_B, Some(200), 200)],
    );
    assert_eq!(state.room_activity.len(), 2);

    update_room_list(&mut state, vec![room(ROOM_A, None, 100)]);

    assert_eq!(state.room_activity.len(), 1);
    assert!(state.room_activity.activity_for(ROOM_B).is_none());
}

#[test]
fn leaving_a_room_forgets_its_remembered_activity() {
    let mut state = ready_state();
    update_room_list(
        &mut state,
        vec![room(ROOM_A, Some(100), 100), room(ROOM_B, Some(200), 200)],
    );

    reduce(
        &mut state,
        AppAction::RoomLeftLocally {
            room_id: ROOM_A.to_owned(),
        },
    );

    assert!(state.room_activity.activity_for(ROOM_A).is_none());
    assert!(state.room_activity.activity_for(ROOM_B).is_some());
}

/// The cache describes one session's observations, so a retired session must not
/// leak its rooms into the next account's projection.
#[test]
fn retiring_the_session_forgets_every_remembered_room() {
    let mut state = ready_state();
    update_room_list(
        &mut state,
        vec![room(ROOM_A, Some(100), 100), room(ROOM_B, Some(200), 200)],
    );
    assert_eq!(state.room_activity.len(), 2);

    reduce(&mut state, AppAction::LogoutFinished);

    assert!(state.room_activity.is_empty());
}

/// The cache is Rust-only: it must not enter the frontend snapshot, and its own
/// payload serializes the two fields the store round-trips.
#[test]
fn the_cache_stays_out_of_the_frontend_snapshot() {
    let state = ready_state();
    let snapshot = serde_json::to_value(&state).expect("serialize app state");
    assert!(
        snapshot.get("room_activity").is_none(),
        "the local ordering cache is not a frontend slice"
    );

    let mut activity = RoomActivityState::default();
    assert!(activity.is_empty());
    assert!(activity.observe(
        ROOM_A,
        ConversationActivity {
            timestamp_ms: 42,
            source: ConversationActivitySource::EncryptedMessage,
        }
    ));
    assert!(!activity.observe(
        ROOM_A,
        ConversationActivity {
            timestamp_ms: 42,
            source: ConversationActivitySource::Message,
        }
    ));
    let value = serde_json::to_value(&activity).expect("serialize room activity");
    assert_eq!(
        value["rooms"][ROOM_A]["timestamp_ms"],
        serde_json::json!(42)
    );
    assert_eq!(
        value["rooms"][ROOM_A]["source"],
        serde_json::json!("encryptedMessage")
    );
}
