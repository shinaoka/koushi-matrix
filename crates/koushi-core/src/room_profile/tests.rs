use super::*;
use koushi_state::{RoomNotificationSettings, RoomTags};

fn room(index: usize) -> RoomSummary {
    RoomSummary {
        display_name_placeholder: None,
        display_label_placeholder: None,
        room_id: format!("!profile-{index}:example.invalid"),
        display_name: format!("Synthetic room {index}"),
        display_label: format!("Synthetic room {index}"),
        original_display_label: format!("Synthetic room {index}"),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: 1,
        notification_count: 1,
        highlight_count: 0,
        thread_unread_count: 0,
        thread_highlight_count: 0,
        marked_unread: false,
        recency_stamp: Some(1),
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 2,
    }
}

fn space(child_room_ids: Vec<String>) -> SpaceSummary {
    SpaceSummary {
        space_id: "!space:example.invalid".to_owned(),
        raw_name: None,
        display_name: "Synthetic Workspace".to_owned(),
        avatar: None,
        join_rule: None,
        topic: None,
        parent_side_child_room_ids: child_room_ids.clone(),
        child_room_ids,
    }
}

fn revisions(table: &RoomProfileTable, state: &AppState) -> Vec<u64> {
    state
        .rooms
        .iter()
        .map(|room| table.get(&room.room_id).expect("profile").revision())
        .collect()
}

#[test]
fn identical_refresh_keeps_every_revision() {
    let state = AppState {
        rooms: (0..3).map(room).collect(),
        ..AppState::default()
    };
    let mut table = RoomProfileTable::default();
    assert_eq!(table.refresh(&state).updated, [0, 1, 2]);
    let before = revisions(&table, &state);
    assert!(table.refresh(&state).is_empty());
    assert_eq!(revisions(&table, &state), before);
}

#[test]
fn only_changed_inputs_advance_a_revision() {
    let mut state = AppState {
        rooms: (0..3).map(room).collect(),
        ..AppState::default()
    };
    let mut table = RoomProfileTable::default();
    table.refresh(&state);
    let before = revisions(&table, &state);

    state.rooms[1].unread_count = 4;
    assert_eq!(table.refresh(&state).updated, [1]);
    let after = revisions(&table, &state);
    assert_eq!(after[0], before[0]);
    assert_ne!(after[1], before[1]);
    assert_eq!(after[2], before[2]);

    // Fields no profile reads (recency, member count) are not inputs.
    state.rooms[2].recency_stamp = Some(99);
    state.rooms[2].joined_members = 9;
    assert!(table.refresh(&state).is_empty());

    state.room_notification_settings.insert(
        state.rooms[2].room_id.clone(),
        RoomNotificationSettings {
            mode: RoomNotificationMode::Mute,
            ..RoomNotificationSettings::default()
        },
    );
    assert_eq!(table.refresh(&state).updated, [2]);
    assert!(
        table
            .get(&state.rooms[2].room_id)
            .expect("profile")
            .is_muted()
    );
}

#[test]
fn a_space_change_relabels_only_its_rooms() {
    let mut state = AppState {
        rooms: (0..2).map(room).collect(),
        ..AppState::default()
    };
    let mut table = RoomProfileTable::default();
    table.refresh(&state);

    state.spaces = vec![space(vec![state.rooms[0].room_id.clone()])];
    assert_eq!(table.refresh(&state).updated, [0]);
    assert_eq!(
        table
            .get(&state.rooms[0].room_id)
            .expect("profile")
            .context_label,
        "Synthetic Workspace / Synthetic room 0"
    );
}

#[test]
fn rooms_that_leave_the_list_are_dropped() {
    let mut state = AppState {
        rooms: (0..3).map(room).collect(),
        ..AppState::default()
    };
    let mut table = RoomProfileTable::default();
    table.refresh(&state);

    let removed = state.rooms.remove(1);
    let changes = table.refresh(&state);
    assert!(changes.updated.is_empty());
    assert_eq!(changes.removed, std::slice::from_ref(&removed.room_id));
    assert!(table.get(&removed.room_id).is_none());
}
