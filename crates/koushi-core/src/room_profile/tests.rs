use super::*;
use koushi_state::{
    ConversationActivity, ConversationActivitySource, RoomNotificationSettings, RoomTagInfo,
    RoomTags,
};

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

/// Every input `RoomProfile::matches` compares, mutated one at a time on room 1
/// of three. A dropped comparison makes the matching case below pass with an
/// empty change set, which is the silent "Activity stops updating" failure this
/// change can cause.
#[test]
fn every_compared_input_marks_exactly_its_room_changed() {
    fn latest(event_id: &str) -> RoomLatestEventSummary {
        RoomLatestEventSummary {
            event_id: event_id.to_owned(),
            relation_type: None,
            relation_event_id: None,
            thread_root_event_id: None,
            sender_id: Some("@synthetic:example.invalid".to_owned()),
            sender_label: Some("Synthetic Sender".to_owned()),
            sender_avatar: None,
            preview: Some("synthetic preview".to_owned()),
            message_text: None,
            timestamp_ms: 7,
            is_redacted: false,
        }
    }
    fn activity(timestamp_ms: u64) -> ConversationActivity {
        ConversationActivity {
            timestamp_ms,
            source: ConversationActivitySource::Message,
        }
    }

    type Mutation = fn(&mut RoomSummary);
    let cases: [(&str, Mutation); 12] = [
        ("latest event id", |room| {
            room.latest_event = Some(latest("$synthetic-latest-1:example.invalid"));
        }),
        ("latest sender", |room| {
            room.latest_event = Some(latest("$synthetic-latest:example.invalid"));
            room.latest_event.as_mut().expect("latest").sender_id =
                Some("@synthetic-other:example.invalid".to_owned());
        }),
        ("latest preview", |room| {
            room.latest_event = Some(latest("$synthetic-latest:example.invalid"));
            room.latest_event.as_mut().expect("latest").preview =
                Some("another synthetic preview".to_owned());
        }),
        ("counts", |room| {
            room.unread_count += 1;
        }),
        ("thread counts", |room| {
            room.thread_unread_count += 1;
        }),
        ("marked unread", |room| {
            room.marked_unread = true;
        }),
        ("conversation activity", |room| {
            room.conversation_activity = Some(activity(99));
        }),
        ("low priority", |room| {
            room.tags.low_priority = Some(RoomTagInfo { order: None });
        }),
        ("display label", |room| {
            room.display_label = "Another synthetic label".to_owned();
        }),
        ("dm flag", |room| {
            room.is_dm = true;
        }),
        ("parent spaces", |room| {
            room.parent_space_ids = vec!["!space:example.invalid".to_owned()];
        }),
        // Control: not an input. If this one ever changes what the table
        // reports, `matches` grew a field Activity does not need.
        ("recency stamp", |room| {
            room.recency_stamp = Some(42);
        }),
    ];

    for (label, mutate) in cases {
        let mut state = AppState {
            rooms: (0..3).map(room).collect(),
            ..AppState::default()
        };
        let mut table = RoomProfileTable::default();
        assert_eq!(table.refresh(&state).updated, [0, 1, 2], "{label}: initial");

        mutate(&mut state.rooms[1]);
        let changes = table.refresh(&state);
        if label == "recency stamp" {
            assert!(changes.is_empty(), "{label} must not be an input");
        } else {
            assert_eq!(changes.updated, [1], "{label} must refresh only room 1");
        }
    }

    // The notification mode is read from the settings map, not the room row.
    let mut state = AppState {
        rooms: (0..3).map(room).collect(),
        ..AppState::default()
    };
    let mut table = RoomProfileTable::default();
    table.refresh(&state);
    state.room_notification_settings.insert(
        state.rooms[1].room_id.clone(),
        RoomNotificationSettings {
            mode: RoomNotificationMode::Mentions,
            ..RoomNotificationSettings::default()
        },
    );
    assert_eq!(table.refresh(&state).updated, [1]);
}
