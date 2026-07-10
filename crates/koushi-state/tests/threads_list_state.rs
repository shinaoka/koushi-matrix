use koushi_state::{
    AppAction, AppEffect, AppState, OperationFailureKind, RoomSummary, RoomTags, SessionInfo,
    SessionState, ThreadRootProjectionStatus, ThreadsListItem, ThreadsListState, UiEvent, reduce,
};

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.invalid".to_owned(),
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
        last_activity_ms: 0,
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 0,
    }
}

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(session_info()),
        rooms: vec![room("room-a"), room("room-b")],
        ..AppState::default()
    }
}

fn selected_room_state(room_id: &str) -> AppState {
    let mut state = ready_state();

    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: room_id.to_owned(),
        },
    );

    state
}

fn thread_item(root_event_id: &str) -> ThreadsListItem {
    ThreadsListItem {
        root_event_id: root_event_id.to_owned(),
        root_sender: "@user-a:example.invalid".to_owned(),
        root_sender_label: Some("User A".to_owned()),
        root_body_preview: Some("Root preview".to_owned()),
        root_timestamp_ms: Some(1_700_000_000_000),
        latest_event_id: Some("$latest:example.invalid".to_owned()),
        latest_sender: Some("@user-b:example.invalid".to_owned()),
        latest_sender_label: Some("User B".to_owned()),
        latest_body_preview: Some("Latest preview".to_owned()),
        latest_timestamp_ms: Some(1_700_000_100_000),
        reply_count: 3,
    }
}

#[test]
fn default_threads_list_is_closed() {
    let state = AppState::default();
    assert_eq!(state.threads_list, ThreadsListState::Closed);
}

#[test]
fn thread_root_projection_is_keyed_by_room_and_root_and_terminal_failures_do_not_retry() {
    let mut state = selected_room_state("room-a");

    let first_effects = reduce(
        &mut state,
        AppAction::ThreadRootProjectionObserved {
            room_id: "room-a".to_owned(),
            root_event_id: "$old-root:example.invalid".to_owned(),
            activity_event_id: "$latest-reply:example.invalid".to_owned(),
            activity_timestamp_ms: Some(1_700_000_100_000),
        },
    );
    assert_eq!(
        first_effects,
        vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
    );
    assert_eq!(
        state
            .thread_root_projections
            .get("room-a", "$old-root:example.invalid"),
        Some(&ThreadRootProjectionStatus::Pending {
            activity_event_id: "$latest-reply:example.invalid".to_owned(),
            activity_timestamp_ms: Some(1_700_000_100_000),
        })
    );

    reduce(
        &mut state,
        AppAction::ThreadRootProjectionFailed {
            room_id: "room-a".to_owned(),
            root_event_id: "$old-root:example.invalid".to_owned(),
            activity_event_id: "$latest-reply:example.invalid".to_owned(),
            activity_timestamp_ms: Some(1_700_000_100_000),
            failure_kind: OperationFailureKind::NotFound,
        },
    );
    let duplicate_effects = reduce(
        &mut state,
        AppAction::ThreadRootProjectionObserved {
            room_id: "room-a".to_owned(),
            root_event_id: "$old-root:example.invalid".to_owned(),
            activity_event_id: "$latest-reply:example.invalid".to_owned(),
            activity_timestamp_ms: Some(1_700_000_100_000),
        },
    );
    assert!(
        duplicate_effects.is_empty(),
        "terminal projection must not loop"
    );
    assert!(matches!(
        state
            .thread_root_projections
            .get("room-a", "$old-root:example.invalid"),
        Some(ThreadRootProjectionStatus::Failed {
            failure_kind: OperationFailureKind::NotFound,
            ..
        })
    ));
}

#[test]
fn open_threads_list_transitions_to_loading_and_emits_subscribe_effect() {
    let mut state = selected_room_state("room-a");

    let effects = reduce(
        &mut state,
        AppAction::OpenThreadsList {
            request_id: 7,
            room_id: "room-a".to_owned(),
        },
    );

    assert_eq!(
        state.threads_list,
        ThreadsListState::Loading {
            room_id: "room-a".to_owned(),
            request_id: 7,
        }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::SubscribeThreadsList {
                request_id: 7,
                room_id: "room-a".to_owned(),
            },
            AppEffect::EmitUiEvent(UiEvent::ThreadsListChanged),
        ]
    );
}

#[test]
fn threads_list_opened_transitions_to_open_with_items() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::OpenThreadsList {
            request_id: 8,
            room_id: "room-a".to_owned(),
        },
    );

    let items = vec![thread_item("$root-a:example.invalid")];
    let effects = reduce(
        &mut state,
        AppAction::ThreadsListOpened {
            request_id: 8,
            room_id: "room-a".to_owned(),
            items: items.clone(),
            end_reached: false,
        },
    );

    assert_eq!(
        state.threads_list,
        ThreadsListState::Open {
            room_id: "room-a".to_owned(),
            request_id: 8,
            items,
            is_paginating: false,
            end_reached: false,
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::ThreadsListChanged)]
    );
}

#[test]
fn threads_list_updated_replaces_items_and_pagination_state() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::OpenThreadsList {
            request_id: 9,
            room_id: "room-a".to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::ThreadsListOpened {
            request_id: 9,
            room_id: "room-a".to_owned(),
            items: vec![thread_item("$root-a:example.invalid")],
            end_reached: false,
        },
    );

    let items = vec![
        thread_item("$root-a:example.invalid"),
        thread_item("$root-b:example.invalid"),
    ];
    let effects = reduce(
        &mut state,
        AppAction::ThreadsListUpdated {
            request_id: 9,
            room_id: "room-a".to_owned(),
            items: items.clone(),
            is_paginating: true,
            end_reached: false,
        },
    );

    assert_eq!(
        state.threads_list,
        ThreadsListState::Open {
            room_id: "room-a".to_owned(),
            request_id: 9,
            items,
            is_paginating: true,
            end_reached: false,
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::ThreadsListChanged)]
    );
}

#[test]
fn threads_list_pagination_completed_clears_paginating_flag() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::OpenThreadsList {
            request_id: 10,
            room_id: "room-a".to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::ThreadsListOpened {
            request_id: 10,
            room_id: "room-a".to_owned(),
            items: vec![thread_item("$root-a:example.invalid")],
            end_reached: false,
        },
    );

    let items = vec![
        thread_item("$root-a:example.invalid"),
        thread_item("$root-b:example.invalid"),
    ];
    let effects = reduce(
        &mut state,
        AppAction::ThreadsListPaginationCompleted {
            request_id: 10,
            room_id: "room-a".to_owned(),
            items: items.clone(),
            end_reached: true,
        },
    );

    assert_eq!(
        state.threads_list,
        ThreadsListState::Open {
            room_id: "room-a".to_owned(),
            request_id: 10,
            items,
            is_paginating: false,
            end_reached: true,
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::ThreadsListChanged)]
    );
}

#[test]
fn threads_list_failed_transitions_to_failed() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::OpenThreadsList {
            request_id: 11,
            room_id: "room-a".to_owned(),
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::ThreadsListFailed {
            request_id: 11,
            room_id: "room-a".to_owned(),
            failure_kind: OperationFailureKind::Network,
        },
    );

    assert_eq!(
        state.threads_list,
        ThreadsListState::Failed {
            room_id: "room-a".to_owned(),
            request_id: 11,
            failure_kind: OperationFailureKind::Network,
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::ThreadsListChanged)]
    );
}

#[test]
fn close_threads_list_resets_to_closed() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::OpenThreadsList {
            request_id: 12,
            room_id: "room-a".to_owned(),
        },
    );

    let effects = reduce(&mut state, AppAction::CloseThreadsList);

    assert_eq!(state.threads_list, ThreadsListState::Closed);
    assert_eq!(
        effects,
        vec![
            AppEffect::UnsubscribeThreadsList,
            AppEffect::EmitUiEvent(UiEvent::ThreadsListChanged),
        ]
    );
}

#[test]
fn close_threads_list_while_closed_is_no_op() {
    let mut state = selected_room_state("room-a");

    let effects = reduce(&mut state, AppAction::CloseThreadsList);

    assert_eq!(state.threads_list, ThreadsListState::Closed);
    assert_eq!(effects, Vec::<AppEffect>::new());
}

#[test]
fn stale_threads_list_success_is_ignored() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::OpenThreadsList {
            request_id: 13,
            room_id: "room-a".to_owned(),
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::ThreadsListOpened {
            request_id: 99,
            room_id: "room-a".to_owned(),
            items: vec![thread_item("$stale:example.invalid")],
            end_reached: true,
        },
    );

    assert_eq!(
        state.threads_list,
        ThreadsListState::Loading {
            room_id: "room-a".to_owned(),
            request_id: 13,
        }
    );
    assert_eq!(effects, Vec::<AppEffect>::new());
}

#[test]
fn stale_threads_list_failure_is_ignored() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::OpenThreadsList {
            request_id: 14,
            room_id: "room-a".to_owned(),
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::ThreadsListFailed {
            request_id: 98,
            room_id: "room-a".to_owned(),
            failure_kind: OperationFailureKind::Sdk,
        },
    );

    assert_eq!(
        state.threads_list,
        ThreadsListState::Loading {
            room_id: "room-a".to_owned(),
            request_id: 14,
        }
    );
    assert_eq!(effects, Vec::<AppEffect>::new());
}

#[test]
fn open_threads_list_requires_ready_session() {
    let mut state = AppState::default();

    assert_eq!(
        reduce(
            &mut state,
            AppAction::OpenThreadsList {
                request_id: 1,
                room_id: "room-a".to_owned(),
            },
        ),
        Vec::<AppEffect>::new()
    );
    assert_eq!(state.threads_list, ThreadsListState::Closed);
}

#[test]
fn open_threads_list_requires_active_room() {
    let mut state = ready_state();

    assert_eq!(
        reduce(
            &mut state,
            AppAction::OpenThreadsList {
                request_id: 1,
                room_id: "room-a".to_owned(),
            },
        ),
        Vec::<AppEffect>::new()
    );
    assert_eq!(state.threads_list, ThreadsListState::Closed);
}

#[test]
fn duplicate_open_while_loading_is_ignored() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::OpenThreadsList {
            request_id: 15,
            room_id: "room-a".to_owned(),
        },
    );

    assert_eq!(
        reduce(
            &mut state,
            AppAction::OpenThreadsList {
                request_id: 16,
                room_id: "room-a".to_owned(),
            },
        ),
        Vec::<AppEffect>::new()
    );
    assert_eq!(
        state.threads_list,
        ThreadsListState::Loading {
            room_id: "room-a".to_owned(),
            request_id: 15,
        }
    );
}

#[test]
fn paginate_threads_list_requests_pagination_only_when_open_and_idle() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::OpenThreadsList {
            request_id: 17,
            room_id: "room-a".to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::ThreadsListOpened {
            request_id: 17,
            room_id: "room-a".to_owned(),
            items: vec![thread_item("$root-a:example.invalid")],
            end_reached: false,
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::PaginateThreadsList {
            request_id: 117,
            room_id: "room-a".to_owned(),
        },
    );

    assert_eq!(
        state.threads_list,
        ThreadsListState::Open {
            room_id: "room-a".to_owned(),
            request_id: 117,
            items: vec![thread_item("$root-a:example.invalid")],
            is_paginating: true,
            end_reached: false,
        }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::PaginateThreadsList {
                request_id: 117,
                room_id: "room-a".to_owned(),
            },
            AppEffect::EmitUiEvent(UiEvent::ThreadsListChanged),
        ]
    );
}

#[test]
fn threads_list_pagination_completed_accepts_paginate_request_id() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::OpenThreadsList {
            request_id: 21,
            room_id: "room-a".to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::ThreadsListOpened {
            request_id: 21,
            room_id: "room-a".to_owned(),
            items: vec![thread_item("$root-a:example.invalid")],
            end_reached: false,
        },
    );
    reduce(
        &mut state,
        AppAction::PaginateThreadsList {
            request_id: 121,
            room_id: "room-a".to_owned(),
        },
    );

    let items = vec![
        thread_item("$root-a:example.invalid"),
        thread_item("$root-b:example.invalid"),
    ];
    let effects = reduce(
        &mut state,
        AppAction::ThreadsListPaginationCompleted {
            request_id: 121,
            room_id: "room-a".to_owned(),
            items: items.clone(),
            end_reached: true,
        },
    );

    assert_eq!(
        state.threads_list,
        ThreadsListState::Open {
            room_id: "room-a".to_owned(),
            request_id: 121,
            items,
            is_paginating: false,
            end_reached: true,
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::ThreadsListChanged)]
    );
}

#[test]
fn paginate_threads_list_is_ignored_when_already_paginating() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::OpenThreadsList {
            request_id: 18,
            room_id: "room-a".to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::ThreadsListUpdated {
            request_id: 18,
            room_id: "room-a".to_owned(),
            items: vec![thread_item("$root-a:example.invalid")],
            is_paginating: true,
            end_reached: false,
        },
    );

    assert_eq!(
        reduce(
            &mut state,
            AppAction::PaginateThreadsList {
                request_id: 18,
                room_id: "room-a".to_owned(),
            },
        ),
        Vec::<AppEffect>::new()
    );
}

#[test]
fn paginate_threads_list_is_ignored_when_end_reached() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::OpenThreadsList {
            request_id: 19,
            room_id: "room-a".to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::ThreadsListOpened {
            request_id: 19,
            room_id: "room-a".to_owned(),
            items: vec![thread_item("$root-a:example.invalid")],
            end_reached: true,
        },
    );

    assert_eq!(
        reduce(
            &mut state,
            AppAction::PaginateThreadsList {
                request_id: 19,
                room_id: "room-a".to_owned(),
            },
        ),
        Vec::<AppEffect>::new()
    );
}

#[test]
fn logout_clears_threads_list_and_emits_ui_event() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::OpenThreadsList {
            request_id: 20,
            room_id: "room-a".to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::ThreadsListOpened {
            request_id: 20,
            room_id: "room-a".to_owned(),
            items: vec![thread_item("$root-a:example.invalid")],
            end_reached: false,
        },
    );

    let effects = reduce(&mut state, AppAction::LogoutRequested);

    assert_eq!(state.threads_list, ThreadsListState::Closed);
    assert!(
        effects.contains(&AppEffect::EmitUiEvent(UiEvent::ThreadsListChanged)),
        "expected ThreadsListChanged after logout, got {:?}",
        effects
    );
}
