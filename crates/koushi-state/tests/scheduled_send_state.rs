use koushi_state::{
    AppAction, AppState, ComposerState, RoomSummary, RoomTags, ScheduledSendCapability,
    ScheduledSendHandle, ScheduledSendItem, ScheduledSendStore, SessionInfo, SessionState,
    TimelinePaneState, UiEvent, reduce,
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
        marked_unread: false,
        recency_stamp: None,
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 0,
    }
}

#[test]
fn scheduled_thread_send_clears_only_the_captured_thread_draft() {
    let mut state = selected_room_state("room-a");
    state
        .composer_drafts
        .set_room_draft("room-a".to_owned(), "room draft".to_owned());
    state.composer_drafts.set_thread_draft(
        "room-a".to_owned(),
        "$root-a".to_owned(),
        "thread draft".to_owned(),
    );
    state.composer_drafts.set_thread_draft(
        "room-a".to_owned(),
        "$root-b".to_owned(),
        "other thread draft".to_owned(),
    );

    let mut item = scheduled_item("sched-thread", "room-a", 1_900_000_000_000);
    item.thread_root_event_id = Some("$root-a".to_owned());
    reduce(&mut state, AppAction::ScheduledSendCreated { item });

    assert_eq!(
        state
            .composer_drafts
            .rooms
            .get("room-a")
            .map(String::as_str),
        Some("room draft")
    );
    assert!(
        state
            .composer_drafts
            .composer_for_thread("room-a", "$root-a")
            .draft
            .is_empty()
    );
    assert_eq!(
        state
            .composer_drafts
            .composer_for_thread("room-a", "$root-b")
            .draft,
        "other thread draft"
    );
    assert_eq!(
        state.scheduled_sends.items["sched-thread"]
            .thread_root_event_id
            .as_deref(),
        Some("$root-a")
    );
}

#[test]
fn scheduled_send_acceptance_fences_delayed_draft_persistence() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::ComposerDraftChangedAtRevision {
            room_id: "room-a".to_owned(),
            draft: "scheduled body".to_owned(),
            revision: 4.into(),
        },
    );

    reduce(
        &mut state,
        AppAction::ScheduledSendCreatedAtRevision {
            item: scheduled_item("sched-main", "room-a", 1_900_000_000_000),
            draft_revision: 4.into(),
        },
    );
    reduce(
        &mut state,
        AppAction::ComposerDraftChangedAtRevision {
            room_id: "room-a".to_owned(),
            draft: "scheduled body".to_owned(),
            revision: 4.into(),
        },
    );

    assert!(state.timeline.composer.draft.is_empty());
    assert_eq!(state.timeline.composer.draft_revision, 5.into());
    assert!(state.composer_drafts.rooms.get("room-a").is_none());
    assert_eq!(state.composer_drafts.room_revision("room-a"), 5.into());
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
        thread_root_event_id: None,
        body: "scheduled body".to_owned(),
        send_at_ms,
        handle: ScheduledSendHandle::Local,
        is_dispatching: false,
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

    assert_eq!(state.timeline.composer.draft, "");
    assert_eq!(state.timeline.composer.draft_revision, 2.into());
    assert!(state.composer_drafts.rooms.is_empty());
    assert_eq!(state.composer_drafts.room_revision("room-a"), 2.into());
    assert_eq!(state.timeline.scheduled_sends.len(), 1);
    assert_eq!(state.timeline.scheduled_sends[0].scheduled_id, "sched-1");
    assert_eq!(state.timeline.scheduled_sends[0].body, "scheduled body");
    assert_eq!(
        effects,
        vec![koushi_state::AppEffect::EmitUiEvent(
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
        vec![koushi_state::AppEffect::EmitUiEvent(
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
fn loaded_scheduled_sends_keep_unlisted_rooms_and_project_selected_room() {
    let mut state = selected_room_state("room-a");
    let mut scheduled_sends = ScheduledSendStore {
        capability: ScheduledSendCapability::LocalFallback,
        ..ScheduledSendStore::default()
    };
    scheduled_sends.insert(scheduled_item("sched-a", "room-a", 1_900_000_000_000));
    scheduled_sends.insert(scheduled_item("sched-c", "room-c", 1_900_000_030_000));

    let effects = reduce(
        &mut state,
        AppAction::ScheduledSendsLoaded { scheduled_sends },
    );

    assert!(state.scheduled_sends.items.contains_key("sched-c"));
    assert_eq!(
        state.scheduled_sends.capability,
        ScheduledSendCapability::LocalFallback
    );
    assert_eq!(state.timeline.scheduled_sends.len(), 1);
    assert_eq!(state.timeline.scheduled_sends[0].scheduled_id, "sched-a");
    assert_eq!(
        effects,
        vec![koushi_state::AppEffect::EmitUiEvent(
            UiEvent::TimelineChanged {
                room_id: "room-a".to_owned(),
            }
        )]
    );
}

#[test]
fn scheduled_send_debug_redacts_body_room_and_server_handle() {
    let item = ScheduledSendItem {
        scheduled_id: "sched-1".to_owned(),
        room_id: "!private-room:example.test".to_owned(),
        thread_root_event_id: None,
        body: "private future message".to_owned(),
        send_at_ms: 1_900_000_000_000,
        handle: ScheduledSendHandle::Server {
            delay_id: "server-delay-secret".to_owned(),
        },
        is_dispatching: false,
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
        submission_registry: Default::default(),
        scheduled_send_capability: ScheduledSendCapability::Unknown,
        scheduled_sends: state.timeline.scheduled_sends.clone(),
        staged_uploads: Vec::new(),
        media_gallery: Vec::new(),
        media_downloads: Default::default(),
        continuity: Default::default(),
    };
    assert_eq!(timeline.scheduled_sends.len(), 1);
}
