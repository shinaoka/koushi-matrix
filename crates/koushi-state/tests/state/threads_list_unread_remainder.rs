use koushi_state::{
    AppAction, AppState, RoomSummary, RoomTags, SessionInfo, SessionState, ThreadsListItem, reduce,
};

fn room() -> RoomSummary {
    RoomSummary {
        room_id: "!room:example.invalid".into(),
        display_name: "Room".into(),
        display_name_placeholder: None,
        display_label: "Room".into(),
        original_display_label: "Room".into(),
        display_label_placeholder: None,
        avatar: None,
        is_dm: false,
        dm_user_ids: vec![],
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
        parent_space_ids: vec![],
        dm_space_ids: vec![],
        is_encrypted: false,
        joined_members: 2,
    }
}

fn item(root: &str) -> ThreadsListItem {
    ThreadsListItem {
        room_id: "!room:example.invalid".into(),
        root_event_id: root.into(),
        root_sender: "@sender:example.invalid".into(),
        root_sender_label: None,
        root_body_preview: None,
        root_timestamp_ms: None,
        latest_event_id: None,
        latest_sender: None,
        latest_sender_label: None,
        latest_body_preview: None,
        latest_timestamp_ms: None,
        reply_count: 0,
        unread_count: 0,
    }
}

#[test]
fn every_counted_thread_root_is_selectable_in_the_threads_list() {
    let mut state = AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".into(),
            user_id: "@viewer:example.invalid".into(),
            device_id: "VIEWER".into(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        rooms: vec![room()],
        ..AppState::default()
    };
    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "!room:example.invalid".into(),
        },
    );
    for (root, unread) in [
        ("$listed:example.invalid", 2),
        ("$older:example.invalid", 3),
    ] {
        reduce(
            &mut state,
            AppAction::ThreadUnreadObserved {
                room_id: "!room:example.invalid".into(),
                root_event_id: root.into(),
                unread,
                highlight: 0,
            },
        );
    }
    reduce(
        &mut state,
        AppAction::OpenThreadsList {
            request_id: 7,
            room_id: "!room:example.invalid".into(),
        },
    );
    reduce(
        &mut state,
        AppAction::ThreadsListOpened {
            request_id: 7,
            room_id: "!room:example.invalid".into(),
            items: vec![item("$listed:example.invalid")],
            end_reached: false,
        },
    );
    let list = serde_json::to_value(&state.threads_list).unwrap();
    assert_eq!(state.rooms[0].thread_unread_count, 5);
    let items = list["items"].as_array().unwrap();
    assert_eq!(
        items
            .iter()
            .map(|item| item["unread_count"].as_u64().unwrap())
            .sum::<u64>(),
        5
    );
    assert!(
        items
            .iter()
            .any(|item| item["root_event_id"] == "$listed:example.invalid")
    );
    assert!(
        items
            .iter()
            .any(|item| item["root_event_id"] == "$older:example.invalid")
    );
}

#[test]
fn observed_unread_root_remains_selectable_when_thread_service_fails() {
    let mut state = AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".into(),
            user_id: "@viewer:example.invalid".into(),
            device_id: "VIEWER".into(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        rooms: vec![room()],
        ..AppState::default()
    };
    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "!room:example.invalid".into(),
        },
    );
    reduce(
        &mut state,
        AppAction::ThreadUnreadObserved {
            room_id: "!room:example.invalid".into(),
            root_event_id: "$known:example.invalid".into(),
            unread: 1,
            highlight: 0,
        },
    );
    reduce(
        &mut state,
        AppAction::OpenThreadsList {
            request_id: 9,
            room_id: "!room:example.invalid".into(),
        },
    );
    reduce(
        &mut state,
        AppAction::ThreadsListFailed {
            request_id: 9,
            room_id: "!room:example.invalid".into(),
            failure_kind: koushi_state::OperationFailureKind::Network,
        },
    );
    let list = serde_json::to_value(&state.threads_list).unwrap();
    assert_eq!(list["kind"], "failed");
    assert_eq!(list["items"][0]["root_event_id"], "$known:example.invalid");
    assert_eq!(list["items"][0]["unread_count"], 1);
}
