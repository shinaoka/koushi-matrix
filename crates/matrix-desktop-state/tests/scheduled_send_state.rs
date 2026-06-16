use matrix_desktop_state::{
    AppAction, AppState, ComposerState, RoomSummary, RoomTags, ScheduledSendCapability,
    ScheduledSendHandle, ScheduledSendItem, SessionInfo, SessionState, TimelinePaneState, UiEvent,
    reduce,
};

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.org".to_owned(),
        user_id: "@user-a:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
    }
}

fn room(room_id: &str) -> RoomSummary {
    RoomSummary {
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
        parent_space_ids: Vec::new(),
    }
}

fn selected_room_state(room_id: &str) -> AppState {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        rooms: vec![room("room-a"), room("room-b")],
        ..AppState::default()
    };

    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: room_id.to_owned(),
        },
    );
    state
}

fn scheduled_item(id: &str, room_id: &str, send_at_ms: u64) -> ScheduledSendItem {
    ScheduledSendItem {
        scheduled_id: id.to_owned(),
        room_id: room_id.to_owned(),
        body: "scheduled body".to_owned(),
        send_at_ms,
        handle: ScheduledSendHandle::Local,
    }
}

#[test]
fn scheduled_send_create_clears_room_draft_and_projects_selected_room() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::ComposerDraftChanged {
            room_id: "room-a".to_owned(),
            draft: "scheduled body".to_owned(),
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::ScheduledSendCreated {
            item: scheduled_item("sched-1", "room-a", 1_900_000_000_000),
        },
    );

    assert_eq!(state.timeline.composer, ComposerState::default());
    assert!(state.composer_drafts.rooms.is_empty());
    assert_eq!(state.timeline.scheduled_sends.len(), 1);
    assert_eq!(state.timeline.scheduled_sends[0].scheduled_id, "sched-1");
    assert_eq!(state.timeline.scheduled_sends[0].body, "scheduled body");
    assert_eq!(
        effects,
        vec![matrix_desktop_state::AppEffect::EmitUiEvent(
            UiEvent::TimelineChanged {
                room_id: "room-a".to_owned(),
            }
        )]
    );

    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "room-b".to_owned(),
        },
    );
    assert!(state.timeline.scheduled_sends.is_empty());

    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "room-a".to_owned(),
        },
    );
    assert_eq!(state.timeline.scheduled_sends.len(), 1);
    assert_eq!(state.timeline.scheduled_sends[0].scheduled_id, "sched-1");
}

#[test]
fn scheduled_send_cancel_and_reschedule_update_store_and_projection() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::ScheduledSendCreated {
            item: scheduled_item("sched-1", "room-a", 1_900_000_000_000),
        },
    );

    reduce(
        &mut state,
        AppAction::ScheduledSendRescheduled {
            scheduled_id: "sched-1".to_owned(),
            send_at_ms: 1_900_000_030_000,
            handle: ScheduledSendHandle::Server {
                delay_id: "server-delay-id".to_owned(),
            },
        },
    );
    assert_eq!(
        state.timeline.scheduled_sends[0].send_at_ms,
        1_900_000_030_000
    );
    assert_eq!(
        state.timeline.scheduled_sends[0].handle,
        ScheduledSendHandle::Server {
            delay_id: "server-delay-id".to_owned()
        }
    );

    reduce(
        &mut state,
        AppAction::ScheduledSendCancelled {
            scheduled_id: "sched-1".to_owned(),
        },
    );
    assert!(state.timeline.scheduled_sends.is_empty());
    assert!(state.scheduled_sends.items.is_empty());
}

#[test]
fn scheduled_send_dispatch_removes_item_and_returns_body_to_core_only() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::ScheduledSendCreated {
            item: scheduled_item("sched-1", "room-a", 1_900_000_000_000),
        },
    );

    let dispatched = reduce(
        &mut state,
        AppAction::ScheduledSendDispatched {
            scheduled_id: "sched-1".to_owned(),
        },
    );

    assert!(state.timeline.scheduled_sends.is_empty());
    assert!(state.scheduled_sends.items.is_empty());
    assert_eq!(
        dispatched,
        vec![matrix_desktop_state::AppEffect::EmitUiEvent(
            UiEvent::TimelineChanged {
                room_id: "room-a".to_owned(),
            }
        )]
    );
}

#[test]
fn scheduled_send_capability_is_rust_owned() {
    let mut state = selected_room_state("room-a");

    reduce(
        &mut state,
        AppAction::ScheduledSendCapabilityChanged {
            capability: ScheduledSendCapability::LocalFallback,
        },
    );

    assert_eq!(
        state.scheduled_sends.capability,
        ScheduledSendCapability::LocalFallback
    );
}

#[test]
fn scheduled_send_debug_redacts_body_room_and_server_handle() {
    let item = ScheduledSendItem {
        scheduled_id: "sched-1".to_owned(),
        room_id: "!private-room:example.test".to_owned(),
        body: "private future message".to_owned(),
        send_at_ms: 1_900_000_000_000,
        handle: ScheduledSendHandle::Server {
            delay_id: "server-delay-secret".to_owned(),
        },
    };

    let debug = format!("{item:?}");
    assert!(debug.contains("ScheduledSendItem"), "{debug}");
    assert!(debug.contains("sched-1"), "{debug}");
    assert!(!debug.contains("!private-room:example.test"), "{debug}");
    assert!(!debug.contains("private future message"), "{debug}");
    assert!(!debug.contains("server-delay-secret"), "{debug}");
}

#[test]
fn timeline_pane_snapshot_contains_only_selected_room_scheduled_sends() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::ScheduledSendCreated {
            item: scheduled_item("sched-1", "room-a", 1_900_000_000_000),
        },
    );
    reduce(
        &mut state,
        AppAction::ScheduledSendCreated {
            item: scheduled_item("sched-2", "room-b", 1_900_000_000_000),
        },
    );

    assert_eq!(state.timeline.scheduled_sends.len(), 1);
    assert_eq!(state.timeline.scheduled_sends[0].scheduled_id, "sched-1");

    let serialized = serde_json::to_value(&state).expect("serialize app state");
    assert!(serialized.get("scheduled_sends").is_none());
    assert_eq!(
        serialized["timeline"]["scheduled_sends"][0]["scheduled_id"],
        "sched-1"
    );
    assert_eq!(
        serialized["timeline"]["scheduled_sends"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let timeline = TimelinePaneState {
        room_id: Some("room-a".to_owned()),
        is_subscribed: true,
        is_paginating_backwards: false,
        composer: ComposerState::default(),
        scheduled_send_capability: ScheduledSendCapability::Unknown,
        scheduled_sends: state.timeline.scheduled_sends.clone(),
    };
    assert_eq!(timeline.scheduled_sends.len(), 1);
}
