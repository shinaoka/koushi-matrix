//! #1160: the explicitly opened Home/Space scheduled-sends projection.

use koushi_state::{
    AppAction, AppState, RoomSummary, RoomTags, ScheduledSendCapability, ScheduledSendHandle,
    ScheduledSendItem, ScheduledSendStore, ScheduledSendsListState, ScheduledSendsScope,
    SessionInfo, SessionState, SlidingSyncAdmission, SlidingSyncCapabilityResult,
    SlidingSyncCapabilityState, SlidingSyncPositiveEvidence, SlidingSyncRevalidationState,
    SpaceSummary, SyncLifecycleStatus, reduce,
};

const ACCOUNT_EPOCH: u64 = 7;
const REQUEST_ID: u64 = 41;

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.invalid".to_owned(),
        user_id: "@user-a:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    }
}

fn room(room_id: &str) -> RoomSummary {
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
        recency_stamp: None,
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 0,
    }
}

fn dm(room_id: &str, dm_space_ids: &[&str]) -> RoomSummary {
    let mut room = room(room_id);
    room.is_dm = true;
    room.dm_space_ids = dm_space_ids.iter().map(|id| (*id).to_owned()).collect();
    room
}

fn space(space_id: &str, child_room_ids: &[&str]) -> SpaceSummary {
    SpaceSummary {
        space_id: space_id.to_owned(),
        raw_name: None,
        display_name: space_id.to_owned(),
        avatar: None,
        join_rule: None,
        child_room_ids: child_room_ids.iter().map(|id| (*id).to_owned()).collect(),
        parent_side_child_room_ids: child_room_ids.iter().map(|id| (*id).to_owned()).collect(),
    }
}

fn item(scheduled_id: &str, room_id: &str, send_at_ms: u64) -> ScheduledSendItem {
    ScheduledSendItem {
        scheduled_id: scheduled_id.to_owned(),
        room_id: room_id.to_owned(),
        thread_root_event_id: None,
        body: format!("body {scheduled_id}"),
        send_at_ms,
        handle: ScheduledSendHandle::Local,
        is_dispatching: false,
    }
}

fn ready_state(spaces: Vec<SpaceSummary>, rooms: Vec<RoomSummary>) -> AppState {
    AppState {
        session: SessionState::Ready(session_info()),
        spaces,
        rooms,
        ..AppState::default()
    }
}

fn open(state: &mut AppState, scope: ScheduledSendsScope) {
    reduce(state, AppAction::OpenScheduledSendsList { scope });
}

fn open_list(state: &AppState) -> &ScheduledSendsListState {
    &state.scheduled_sends_list
}

fn listed_ids(state: &AppState) -> Vec<String> {
    match open_list(state) {
        ScheduledSendsListState::Open { items, .. } => {
            items.iter().map(|item| item.scheduled_id.clone()).collect()
        }
        ScheduledSendsListState::Closed => Vec::new(),
    }
}

#[test]
fn home_scope_lists_every_item_ordered_by_send_at_then_id() {
    let rooms = vec![
        room("!room-a:example.invalid"),
        room("!room-b:example.invalid"),
    ];
    let mut state = ready_state(Vec::new(), rooms);
    state.scheduled_sends = ScheduledSendStore {
        capability: ScheduledSendCapability::LocalFallback,
        items: [
            item("later", "!room-a:example.invalid", 200),
            item("tie-b", "!room-b:example.invalid", 100),
            item("tie-a", "!room-a:example.invalid", 100),
        ]
        .into_iter()
        .map(|item| (item.scheduled_id.clone(), item))
        .collect(),
    };

    open(&mut state, ScheduledSendsScope::Home);

    assert_eq!(listed_ids(&state), vec!["tie-a", "tie-b", "later"]);
    assert!(matches!(
        open_list(&state),
        ScheduledSendsListState::Open {
            scope: ScheduledSendsScope::Home,
            capability: ScheduledSendCapability::LocalFallback,
            ..
        }
    ));
}

#[test]
fn space_scope_matches_the_sidebar_for_a_parent_child_space_fixture() {
    // Synthetic parent Space -> child Space -> room. The sidebar only shows a
    // Space's immediate children, so the room below the child Space must NOT
    // appear in the parent's list. A pre-flattened fixture would wrongly show it.
    let spaces = vec![
        space(
            "!parent:example.invalid",
            &[
                "!parent-room:example.invalid",
                "!child-space:example.invalid",
            ],
        ),
        space(
            "!child-space:example.invalid",
            &["!room-in-child:example.invalid"],
        ),
    ];
    let rooms = vec![
        room("!parent-room:example.invalid"),
        room("!room-in-child:example.invalid"),
        dm("!parent-dm:example.invalid", &["!parent:example.invalid"]),
        dm(
            "!other-dm:example.invalid",
            &["!child-space:example.invalid"],
        ),
    ];
    let mut state = ready_state(spaces, rooms);
    state.scheduled_sends = ScheduledSendStore {
        capability: ScheduledSendCapability::ServerDelayedEvents,
        items: [
            item("parent-room", "!parent-room:example.invalid", 100),
            item("room-in-child", "!room-in-child:example.invalid", 100),
            item("parent-dm", "!parent-dm:example.invalid", 100),
            item("other-dm", "!other-dm:example.invalid", 100),
        ]
        .into_iter()
        .map(|item| (item.scheduled_id.clone(), item))
        .collect(),
    };

    state.navigation.active_space_id = Some("!parent:example.invalid".to_owned());
    open(
        &mut state,
        ScheduledSendsScope::Space {
            space_id: "!parent:example.invalid".to_owned(),
        },
    );
    assert_eq!(listed_ids(&state), vec!["parent-dm", "parent-room"]);

    state.navigation.active_space_id = Some("!child-space:example.invalid".to_owned());
    open(
        &mut state,
        ScheduledSendsScope::Space {
            space_id: "!child-space:example.invalid".to_owned(),
        },
    );
    assert_eq!(listed_ids(&state), vec!["other-dm", "room-in-child"]);
}

#[test]
fn each_reservation_is_listed_once_for_a_duplicate_child_path() {
    // The room is named twice by the Space (and is also reachable as a DM
    // assignment lane would be), so membership must not duplicate the item.
    let spaces = vec![space(
        "!space:example.invalid",
        &["!room:example.invalid", "!room:example.invalid"],
    )];
    let rooms = vec![room("!room:example.invalid")];
    let mut state = ready_state(spaces, rooms);
    state.scheduled_sends = ScheduledSendStore {
        capability: ScheduledSendCapability::LocalFallback,
        items: [item("only-once", "!room:example.invalid", 100)]
            .into_iter()
            .map(|item| (item.scheduled_id.clone(), item))
            .collect(),
    };
    state.navigation.active_space_id = Some("!space:example.invalid".to_owned());

    open(
        &mut state,
        ScheduledSendsScope::Space {
            space_id: "!space:example.invalid".to_owned(),
        },
    );

    assert_eq!(listed_ids(&state), vec!["only-once"]);
}

#[test]
fn open_list_reflects_create_reschedule_cancel_and_dispatch_without_reopen() {
    let rooms = vec![room("!room-a:example.invalid")];
    let mut state = ready_state(Vec::new(), rooms);
    open(&mut state, ScheduledSendsScope::Home);
    assert_eq!(listed_ids(&state), Vec::<String>::new());

    reduce(
        &mut state,
        AppAction::ScheduledSendCreated {
            item: item("sched-1", "!room-a:example.invalid", 300),
        },
    );
    assert_eq!(listed_ids(&state), vec!["sched-1"]);

    reduce(
        &mut state,
        AppAction::ScheduledSendRescheduled {
            scheduled_id: "sched-1".to_owned(),
            body: "rescheduled".to_owned(),
            send_at_ms: 100,
            handle: ScheduledSendHandle::Server {
                delay_id: "delay-1".to_owned(),
            },
        },
    );
    assert_eq!(listed_ids(&state), vec!["sched-1"]);
    let ScheduledSendsListState::Open { items, .. } = open_list(&state) else {
        panic!("expected an open list");
    };
    assert_eq!(items[0].send_at_ms, 100);
    assert_eq!(items[0].body, "rescheduled");

    reduce(
        &mut state,
        AppAction::ScheduledSendCancelled {
            scheduled_id: "sched-1".to_owned(),
        },
    );
    assert_eq!(listed_ids(&state), Vec::<String>::new());

    reduce(
        &mut state,
        AppAction::ScheduledSendCreated {
            item: item("sched-2", "!room-a:example.invalid", 400),
        },
    );
    assert_eq!(listed_ids(&state), vec!["sched-2"]);
    reduce(
        &mut state,
        AppAction::ScheduledSendDispatched {
            scheduled_id: "sched-2".to_owned(),
        },
    );
    assert_eq!(listed_ids(&state), Vec::<String>::new());
}

#[test]
fn queue_load_and_a_capability_only_change_refresh_the_open_list() {
    let rooms = vec![room("!room-a:example.invalid")];
    let mut state = ready_state(Vec::new(), rooms);
    open(&mut state, ScheduledSendsScope::Home);
    assert_eq!(
        open_list(&state),
        &ScheduledSendsListState::Open {
            scope: ScheduledSendsScope::Home,
            capability: ScheduledSendCapability::Unknown,
            items: Vec::new(),
        }
    );

    reduce(
        &mut state,
        AppAction::ScheduledSendCapabilityChanged {
            capability: ScheduledSendCapability::ServerDelayedEvents,
        },
    );
    assert!(matches!(
        open_list(&state),
        ScheduledSendsListState::Open {
            capability: ScheduledSendCapability::ServerDelayedEvents,
            ..
        }
    ));

    let mut store = ScheduledSendStore {
        capability: ScheduledSendCapability::ServerDelayedEvents,
        items: Default::default(),
    };
    store.insert(item("loaded", "!room-a:example.invalid", 500));
    reduce(
        &mut state,
        AppAction::ScheduledSendsLoaded {
            scheduled_sends: store,
        },
    );
    assert_eq!(listed_ids(&state), vec!["loaded"]);
}

#[test]
fn opening_outside_a_ready_session_is_rejected() {
    let rooms = vec![room("!room-a:example.invalid")];
    for session in [
        SessionState::SignedOut,
        SessionState::Restoring,
        SessionState::Locked(session_info()),
    ] {
        let mut state = ready_state(Vec::new(), rooms.clone());
        state.session = session;
        open(&mut state, ScheduledSendsScope::Home);
        assert_eq!(
            state.scheduled_sends_list,
            ScheduledSendsListState::Closed,
            "opening must be rejected outside a Ready session"
        );
    }
}

#[test]
fn an_invalid_or_unknown_space_scope_is_rejected() {
    let spaces = vec![
        space("!space:example.invalid", &["!room:example.invalid"]),
        space("!other:example.invalid", &[]),
    ];
    let rooms = vec![room("!room:example.invalid")];
    let mut state = ready_state(spaces, rooms);
    state.navigation.active_space_id = Some("!space:example.invalid".to_owned());

    // An unknown Space id never broadens to every room.
    reduce(
        &mut state,
        AppAction::OpenScheduledSendsList {
            scope: ScheduledSendsScope::Space {
                space_id: "!missing:example.invalid".to_owned(),
            },
        },
    );
    assert_eq!(state.scheduled_sends_list, ScheduledSendsListState::Closed);

    // An inactive Space id is rejected even when it exists.
    reduce(
        &mut state,
        AppAction::OpenScheduledSendsList {
            scope: ScheduledSendsScope::Space {
                space_id: "!other:example.invalid".to_owned(),
            },
        },
    );
    assert_eq!(state.scheduled_sends_list, ScheduledSendsListState::Closed);

    // Home is not openable while a Space is the active scope.
    reduce(
        &mut state,
        AppAction::OpenScheduledSendsList {
            scope: ScheduledSendsScope::Home,
        },
    );
    assert_eq!(state.scheduled_sends_list, ScheduledSendsListState::Closed);

    // The active, existing Space opens.
    reduce(
        &mut state,
        AppAction::OpenScheduledSendsList {
            scope: ScheduledSendsScope::Space {
                space_id: "!space:example.invalid".to_owned(),
            },
        },
    );
    assert!(matches!(
        state.scheduled_sends_list,
        ScheduledSendsListState::Open {
            scope: ScheduledSendsScope::Space { .. },
            ..
        }
    ));

    // A duplicate close is a no-op.
    let closed = reduce(&mut state, AppAction::CloseScheduledSendsList);
    assert!(closed.is_empty());
    assert_eq!(state.scheduled_sends_list, ScheduledSendsListState::Closed);
    let closed_again = reduce(&mut state, AppAction::CloseScheduledSendsList);
    assert!(closed_again.is_empty());
    assert_eq!(state.scheduled_sends_list, ScheduledSendsListState::Closed);
}

#[test]
fn sync_failed_auth_closes_the_open_projection() {
    let rooms = vec![room("!room-a:example.invalid")];
    let mut state = ready_state(Vec::new(), rooms);
    state
        .scheduled_sends
        .insert(item("sched-1", "!room-a:example.invalid", 100));
    state.sync = koushi_state::SyncState::Running;
    open(&mut state, ScheduledSendsScope::Home);
    assert!(matches!(
        state.scheduled_sends_list,
        ScheduledSendsListState::Open { .. }
    ));

    reduce(
        &mut state,
        AppAction::SyncFailed {
            reason: "sync_failed_auth".to_owned(),
        },
    );

    assert!(matches!(state.session, SessionState::Locked(_)));
    assert_eq!(state.scheduled_sends_list, ScheduledSendsListState::Closed);
}

#[test]
fn unsupported_capability_revalidation_closes_the_open_projection() {
    let rooms = vec![room("!room-a:example.invalid")];
    let mut state = ready_state(Vec::new(), rooms);
    state
        .scheduled_sends
        .insert(item("sched-1", "!room-a:example.invalid", 100));
    open(&mut state, ScheduledSendsScope::Home);

    state.sliding_sync_account_epoch = ACCOUNT_EPOCH;
    state.sliding_sync_capability = SlidingSyncCapabilityState::Supported {
        account_epoch: ACCOUNT_EPOCH,
        request_id: REQUEST_ID,
        admission: SlidingSyncAdmission::StoredSessionRestore {
            info: session_info(),
        },
        evidence: SlidingSyncPositiveEvidence {
            observed_at_ms: 1_000,
        },
        revalidation: SlidingSyncRevalidationState::Checking {
            request_id: REQUEST_ID + 1,
        },
    };
    reduce(
        &mut state,
        AppAction::SlidingSyncCapabilityRevalidationCompleted {
            account_epoch: ACCOUNT_EPOCH,
            request_id: REQUEST_ID + 1,
            result: SlidingSyncCapabilityResult::Unsupported,
        },
    );

    assert!(matches!(
        state.session,
        SessionState::CapabilityBlocked { .. }
    ));
    assert_eq!(state.scheduled_sends_list, ScheduledSendsListState::Closed);
}

#[test]
fn an_automatic_space_removal_closes_before_broadening_the_scope() {
    let spaces = vec![space("!space:example.invalid", &["!room:example.invalid"])];
    let rooms = vec![
        room("!room:example.invalid"),
        room("!outside:example.invalid"),
    ];
    let mut state = ready_state(spaces, rooms);
    state.scheduled_sends = ScheduledSendStore {
        capability: ScheduledSendCapability::LocalFallback,
        items: [
            item("in-space", "!room:example.invalid", 100),
            item("outside", "!outside:example.invalid", 100),
        ]
        .into_iter()
        .map(|item| (item.scheduled_id.clone(), item))
        .collect(),
    };
    state.navigation.active_space_id = Some("!space:example.invalid".to_owned());
    open(
        &mut state,
        ScheduledSendsScope::Space {
            space_id: "!space:example.invalid".to_owned(),
        },
    );
    assert_eq!(listed_ids(&state), vec!["in-space"]);

    // A room-list update that no longer carries the captured Space clears
    // `active_space_id`; the projection must close rather than fall back to
    // every non-DM room.
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![
                room("!room:example.invalid"),
                room("!outside:example.invalid"),
            ],
        },
    );

    assert_eq!(state.navigation.active_space_id, None);
    assert_eq!(state.scheduled_sends_list, ScheduledSendsListState::Closed);
}

#[test]
fn room_clearing_and_a_same_room_selection_close_the_projection() {
    let spaces = vec![space("!space:example.invalid", &["!room:example.invalid"])];
    let rooms = vec![room("!room:example.invalid")];

    // Same-room selection: the room and timeline already point at the room, so
    // the room-selection helper returns early. The projection must still close.
    let mut state = ready_state(spaces.clone(), rooms.clone());
    state.navigation.active_room_id = Some("!room:example.invalid".to_owned());
    state.timeline.room_id = Some("!room:example.invalid".to_owned());
    open(&mut state, ScheduledSendsScope::Home);
    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "!room:example.invalid".to_owned(),
        },
    );
    assert_eq!(state.scheduled_sends_list, ScheduledSendsListState::Closed);

    // Room clearing through a Home selection with no previous room.
    let mut state = ready_state(spaces, rooms);
    open(&mut state, ScheduledSendsScope::Home);
    assert!(matches!(
        state.scheduled_sends_list,
        ScheduledSendsListState::Open { .. }
    ));
    reduce(&mut state, AppAction::SelectSpace { space_id: None });
    assert_eq!(state.scheduled_sends_list, ScheduledSendsListState::Closed);
}

#[test]
fn an_automatic_removed_active_room_closes_the_projection() {
    let spaces = vec![space(
        "!space:example.invalid",
        &["!room-a:example.invalid"],
    )];
    let rooms = vec![
        room("!room-a:example.invalid"),
        room("!room-b:example.invalid"),
    ];
    let mut state = ready_state(spaces, rooms);
    state.navigation.active_room_id = Some("!room-a:example.invalid".to_owned());
    state.timeline.room_id = Some("!room-a:example.invalid".to_owned());
    open(&mut state, ScheduledSendsScope::Home);
    assert!(matches!(
        state.scheduled_sends_list,
        ScheduledSendsListState::Open { .. }
    ));

    // The active room disappears from the room list (an automatic room clear),
    // not a user selection; the panel projection must close with it, exactly
    // like the Threads list at room.rs.
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: vec![space(
                "!space:example.invalid",
                &["!room-a:example.invalid"],
            )],
            rooms: vec![room("!room-b:example.invalid")],
        },
    );

    assert_eq!(state.navigation.active_room_id, None);
    assert_eq!(state.scheduled_sends_list, ScheduledSendsListState::Closed);
}

#[test]
fn a_removed_child_edge_without_a_replacement_room_closes_the_projection() {
    // Space S has room R as its only non-DM child, and DM D is assigned to S
    // through the DM-space relation and carries a reservation. With R active
    // and the S-scoped panel open, removing R's child edge from S leaves no
    // replacement room, so the navigation retarget clears the active room.
    // The projection must close with it instead of keeping D's body open.
    let spaces = vec![space("!space:example.invalid", &["!room:example.invalid"])];
    let rooms = vec![
        room("!room:example.invalid"),
        dm("!dm:example.invalid", &["!space:example.invalid"]),
    ];
    let mut state = ready_state(spaces, rooms);
    state.navigation.active_space_id = Some("!space:example.invalid".to_owned());
    state.navigation.active_room_id = Some("!room:example.invalid".to_owned());
    state.timeline.room_id = Some("!room:example.invalid".to_owned());
    state.scheduled_sends = ScheduledSendStore {
        capability: ScheduledSendCapability::LocalFallback,
        items: [
            item("room-reservation", "!room:example.invalid", 100),
            item("dm-reservation", "!dm:example.invalid", 100),
        ]
        .into_iter()
        .map(|item| (item.scheduled_id.clone(), item))
        .collect(),
    };
    open(
        &mut state,
        ScheduledSendsScope::Space {
            space_id: "!space:example.invalid".to_owned(),
        },
    );
    assert_eq!(
        listed_ids(&state),
        vec!["dm-reservation", "room-reservation"]
    );

    // R loses its child edge from S while R and D stay joined. S still exists
    // and is still active, so the scope guard alone keeps the projection open.
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: vec![space("!space:example.invalid", &[])],
            rooms: vec![
                room("!room:example.invalid"),
                dm("!dm:example.invalid", &["!space:example.invalid"]),
            ],
        },
    );

    assert_eq!(state.navigation.active_room_id, None);
    assert_eq!(state.scheduled_sends_list, ScheduledSendsListState::Closed);
}

#[test]
fn a_thread_root_marks_a_thread_reply() {
    let rooms = vec![room("!room-a:example.invalid")];
    let mut state = ready_state(Vec::new(), rooms);
    let mut reply = item("thread-reply", "!room-a:example.invalid", 100);
    reply.thread_root_event_id = Some("$root:example.invalid".to_owned());
    state.scheduled_sends.insert(reply);
    state
        .scheduled_sends
        .insert(item("plain-room", "!room-a:example.invalid", 200));

    open(&mut state, ScheduledSendsScope::Home);

    let ScheduledSendsListState::Open { items, .. } = open_list(&state) else {
        panic!("expected an open list");
    };
    assert_eq!(
        items[0].thread_root_event_id.as_deref(),
        Some("$root:example.invalid")
    );
    assert_eq!(items[1].thread_root_event_id, None);
}

#[test]
fn a_membership_change_while_open_updates_the_space_list() {
    let spaces = vec![space(
        "!space:example.invalid",
        &["!room-a:example.invalid"],
    )];
    let rooms = vec![
        room("!room-a:example.invalid"),
        room("!room-b:example.invalid"),
    ];
    let mut state = ready_state(spaces, rooms);
    state.navigation.active_space_id = Some("!space:example.invalid".to_owned());
    state.scheduled_sends = ScheduledSendStore {
        capability: ScheduledSendCapability::LocalFallback,
        items: [
            item("a", "!room-a:example.invalid", 100),
            item("b", "!room-b:example.invalid", 100),
        ]
        .into_iter()
        .map(|item| (item.scheduled_id.clone(), item))
        .collect(),
    };
    open(
        &mut state,
        ScheduledSendsScope::Space {
            space_id: "!space:example.invalid".to_owned(),
        },
    );
    assert_eq!(listed_ids(&state), vec!["a"]);

    // The Space gains the second room as a child; the open list must grow
    // without reopening.
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: vec![space(
                "!space:example.invalid",
                &["!room-a:example.invalid", "!room-b:example.invalid"],
            )],
            rooms: vec![
                room("!room-a:example.invalid"),
                room("!room-b:example.invalid"),
            ],
        },
    );
    assert_eq!(listed_ids(&state), vec!["a", "b"]);
}

#[test]
fn a_room_removed_from_the_queue_closes_or_updates_without_stale_bodies() {
    let rooms = vec![room("!room-a:example.invalid")];
    let mut state = ready_state(Vec::new(), rooms);
    open(&mut state, ScheduledSendsScope::Home);
    reduce(
        &mut state,
        AppAction::ScheduledSendCreated {
            item: item("gone", "!room-a:example.invalid", 100),
        },
    );
    assert_eq!(listed_ids(&state), vec!["gone"]);

    // Leaving the room drops the reservation from the backing store; the open
    // list must not keep a stale body.
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: Vec::new(),
        },
    );
    assert_eq!(listed_ids(&state), Vec::<String>::new());
}

#[test]
fn a_ready_preserving_status_change_keeps_the_projection_open() {
    let rooms = vec![room("!room-a:example.invalid")];
    let mut state = ready_state(Vec::new(), rooms);
    open(&mut state, ScheduledSendsScope::Home);
    // A plain status action that leaves the session Ready keeps the panel open.
    reduce(
        &mut state,
        AppAction::SyncStatusChanged {
            generation: 1,
            status: SyncLifecycleStatus::Running,
        },
    );
    assert!(matches!(state.session, SessionState::Ready(_)));
    assert!(matches!(
        state.scheduled_sends_list,
        ScheduledSendsListState::Open { .. }
    ));
}

#[test]
fn a_dm_reassignment_while_open_removes_it_without_closing() {
    let spaces = vec![
        space("!space:example.invalid", &["!room:example.invalid"]),
        space("!other-space:example.invalid", &[]),
    ];
    let rooms = vec![
        room("!room:example.invalid"),
        dm("!dm:example.invalid", &["!space:example.invalid"]),
    ];
    let mut state = ready_state(spaces, rooms);
    state.navigation.active_space_id = Some("!space:example.invalid".to_owned());
    state.scheduled_sends = ScheduledSendStore {
        capability: ScheduledSendCapability::LocalFallback,
        items: [
            item("dm-reservation", "!dm:example.invalid", 100),
            item("room-reservation", "!room:example.invalid", 100),
        ]
        .into_iter()
        .map(|item| (item.scheduled_id.clone(), item))
        .collect(),
    };
    open(
        &mut state,
        ScheduledSendsScope::Space {
            space_id: "!space:example.invalid".to_owned(),
        },
    );
    assert_eq!(
        listed_ids(&state),
        vec!["dm-reservation", "room-reservation"]
    );

    // The DM stays joined but is reassigned to another Space, so it leaves this
    // Space's membership. The list must drop its body while staying open.
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: vec![
                space("!space:example.invalid", &["!room:example.invalid"]),
                space("!other-space:example.invalid", &[]),
            ],
            rooms: vec![
                room("!room:example.invalid"),
                dm("!dm:example.invalid", &["!other-space:example.invalid"]),
            ],
        },
    );

    assert_eq!(listed_ids(&state), vec!["room-reservation"]);
    assert!(matches!(
        state.scheduled_sends_list,
        ScheduledSendsListState::Open { .. }
    ));
}

#[test]
fn selecting_another_space_with_an_unchanged_room_closes_the_projection() {
    let spaces = vec![
        space("!space-a:example.invalid", &["!room:example.invalid"]),
        space("!space-b:example.invalid", &["!room:example.invalid"]),
    ];
    let rooms = vec![
        room("!room:example.invalid"),
        dm("!dm:example.invalid", &["!space-a:example.invalid"]),
    ];
    let mut state = ready_state(spaces, rooms);
    state.navigation.active_space_id = Some("!space-a:example.invalid".to_owned());
    state.navigation.active_room_id = Some("!room:example.invalid".to_owned());
    state.timeline.room_id = Some("!room:example.invalid".to_owned());
    state
        .scheduled_sends
        .insert(item("dm-reservation", "!dm:example.invalid", 100));
    open(
        &mut state,
        ScheduledSendsScope::Space {
            space_id: "!space-a:example.invalid".to_owned(),
        },
    );
    assert!(matches!(
        state.scheduled_sends_list,
        ScheduledSendsListState::Open { .. }
    ));

    // Both Spaces select the same room, so the room-selection helper is skipped
    // (the active room does not change). The explicit Space-selection close must
    // still fire.
    reduce(
        &mut state,
        AppAction::SelectSpace {
            space_id: Some("!space-b:example.invalid".to_owned()),
        },
    );

    assert_eq!(
        state.navigation.active_space_id.as_deref(),
        Some("!space-b:example.invalid")
    );
    assert_eq!(
        state.navigation.active_room_id.as_deref(),
        Some("!room:example.invalid")
    );
    assert_eq!(state.scheduled_sends_list, ScheduledSendsListState::Closed);
}

#[test]
fn a_successful_directory_join_closes_the_projection() {
    let spaces = vec![space("!space:example.invalid", &["!room:example.invalid"])];
    let rooms = vec![room("!room:example.invalid")];
    let mut state = ready_state(spaces, rooms);
    state.navigation.active_space_id = Some("!space:example.invalid".to_owned());
    state
        .scheduled_sends
        .insert(item("sched", "!room:example.invalid", 100));
    open(
        &mut state,
        ScheduledSendsScope::Space {
            space_id: "!space:example.invalid".to_owned(),
        },
    );
    assert!(matches!(
        state.scheduled_sends_list,
        ScheduledSendsListState::Open { .. }
    ));

    reduce(
        &mut state,
        AppAction::DirectoryJoinRequested {
            request_id: 1,
            room_id_or_alias: "#joined:example.invalid".to_owned(),
            via_servers: Vec::new(),
        },
    );
    reduce(
        &mut state,
        AppAction::DirectoryJoinSucceeded {
            request_id: 1,
            room_id: "!joined:example.invalid".to_owned(),
        },
    );

    assert_eq!(state.scheduled_sends_list, ScheduledSendsListState::Closed);
}

#[test]
fn central_logout_closes_the_open_projection() {
    let rooms = vec![room("!room-a:example.invalid")];
    let mut state = ready_state(Vec::new(), rooms);
    state
        .scheduled_sends
        .insert(item("sched", "!room-a:example.invalid", 100));
    open(&mut state, ScheduledSendsScope::Home);
    assert!(matches!(
        state.scheduled_sends_list,
        ScheduledSendsListState::Open { .. }
    ));

    reduce(&mut state, AppAction::LogoutRequested);

    assert!(matches!(state.session, SessionState::LoggingOut));
    assert_eq!(state.scheduled_sends_list, ScheduledSendsListState::Closed);
}
