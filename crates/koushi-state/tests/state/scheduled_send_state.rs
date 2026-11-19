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
            .map(koushi_state::ComposerDocument::plain_body),
        Some("room draft".to_owned())
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

/// #1159: an accepted thread reservation must appear in the selected room's
/// scheduled-send projection immediately, without leaving the thread or
/// reselecting the room, exactly like a plain room reservation does.
#[test]
fn scheduled_thread_reply_projects_into_its_open_room_list() {
    let mut state = selected_room_state("room-a");
    let mut item = scheduled_item("sched-thread", "room-a", 1_900_000_000_000);
    item.thread_root_event_id = Some("$root-a".to_owned());

    let effects = reduce(&mut state, AppAction::ScheduledSendCreated { item });

    assert_eq!(state.timeline.scheduled_sends.len(), 1);
    assert_eq!(
        state.timeline.scheduled_sends[0].scheduled_id,
        "sched-thread"
    );
    assert_eq!(
        state.timeline.scheduled_sends[0]
            .thread_root_event_id
            .as_deref(),
        Some("$root-a")
    );
    assert!(
        effects.contains(&koushi_state::AppEffect::EmitUiEvent(
            UiEvent::TimelineChanged {
                room_id: "room-a".to_owned(),
            }
        )),
        "the open room's projection must be refreshed: {effects:?}"
    );
}

/// A thread reservation for another room must not touch the selected room's
/// projection.
#[test]
fn scheduled_thread_reply_for_another_room_leaves_the_open_room_alone() {
    let mut state = selected_room_state("room-a");
    let mut item = scheduled_item("sched-other", "room-b", 1_900_000_000_000);
    item.thread_root_event_id = Some("$root-b".to_owned());

    let effects = reduce(&mut state, AppAction::ScheduledSendCreated { item });

    assert!(state.timeline.scheduled_sends.is_empty());
    assert!(
        !effects.contains(&koushi_state::AppEffect::EmitUiEvent(
            UiEvent::TimelineChanged {
                room_id: "room-b".to_owned(),
            }
        )),
        "another room's thread reservation must not refresh the open room: {effects:?}"
    );
}

#[test]
fn scheduled_send_acceptance_fences_delayed_draft_persistence() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::ComposerDraftChangedAtRevision {
            room_id: "room-a".to_owned(),
            document: "scheduled body".into(),
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
            document: "scheduled body".into(),
            revision: 4.into(),
        },
    );

    assert!(state.timeline.composer.draft.is_empty());
    assert_eq!(state.timeline.composer.draft_revision, 5.into());
    assert!(!state.composer_drafts.rooms.contains_key("room-a"));
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
            document: "scheduled body".into(),
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
            body: "edited scheduled body".to_owned(),
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
    assert_eq!(
        state.timeline.scheduled_sends[0].body,
        "edited scheduled body"
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
        media_gallery_pagination: Default::default(),
        media_downloads: Default::default(),
        reply_quote_thumbnails: Default::default(),
        continuity: Default::default(),
    };
    assert_eq!(timeline.scheduled_sends.len(), 1);
}

/// #1159: acceptance is not durability. A failed local save must be visible as
/// its own error, reported once, and cleared by a later successful save.
#[test]
fn scheduled_send_persistence_failure_is_reported_once_and_cleared_on_success() {
    let mut state = selected_room_state("room-a");

    let effects = reduce(
        &mut state,
        AppAction::ScheduledSendPersistenceFailed {
            message: "scheduled sends could not be saved on this device".to_owned(),
        },
    );
    assert_eq!(
        effects,
        vec![koushi_state::AppEffect::EmitUiEvent(UiEvent::ErrorChanged)]
    );
    assert_eq!(state.errors.len(), 1);
    assert_eq!(state.errors[0].code, "scheduled_send_persistence_failed");
    assert!(state.errors[0].recoverable);

    // Repeated failures must not grow the notice list.
    let repeated = reduce(
        &mut state,
        AppAction::ScheduledSendPersistenceFailed {
            message: "again".to_owned(),
        },
    );
    assert!(repeated.is_empty());
    assert_eq!(state.errors.len(), 1);

    // Unrelated failures are untouched by the recovery.
    state.errors.push(koushi_state::AppError {
        code: "other".to_owned(),
        message: "other".to_owned(),
        recoverable: true,
        reason: None,
    });

    let cleared = reduce(&mut state, AppAction::ScheduledSendPersisted);
    assert_eq!(
        cleared,
        vec![koushi_state::AppEffect::EmitUiEvent(UiEvent::ErrorChanged)]
    );
    assert_eq!(state.errors.len(), 1);
    assert_eq!(state.errors[0].code, "other");

    // Nothing to clear is not an event.
    assert!(reduce(&mut state, AppAction::ScheduledSendPersisted).is_empty());
}

#[test]
fn scheduled_send_persistence_outcome_is_ignored_without_a_ready_session() {
    let mut state = AppState {
        session: SessionState::SignedOut,
        ..AppState::default()
    };

    let failed = reduce(
        &mut state,
        AppAction::ScheduledSendPersistenceFailed {
            message: "x".to_owned(),
        },
    );
    assert!(failed.is_empty());
    assert!(state.errors.is_empty());
    assert!(reduce(&mut state, AppAction::ScheduledSendPersisted).is_empty());
}

/// #1159: the notice belongs to the session that produced it, so retiring that
/// session (sign-out) must withdraw it rather than leak it into the next one.
#[test]
fn signing_out_withdraws_a_scheduled_send_persistence_notice() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::ScheduledSendPersistenceFailed {
            message: "not saved".to_owned(),
        },
    );
    assert!(
        state
            .errors
            .iter()
            .any(|error| error.code == "scheduled_send_persistence_failed")
    );

    let effects = reduce(&mut state, AppAction::LogoutRequested);

    assert!(
        !state
            .errors
            .iter()
            .any(|error| error.code == "scheduled_send_persistence_failed"),
        "a retired session must not keep its persistence notice"
    );
    assert!(effects.contains(&koushi_state::AppEffect::EmitUiEvent(UiEvent::ErrorChanged)));
}

/// #1159: the auth-failure transition retires the ready session without going
/// through `clear_session_views`, so it must withdraw the notice as well.
#[test]
fn an_auth_sync_failure_withdraws_the_scheduled_send_persistence_notice() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::ScheduledSendPersistenceFailed {
            message: "not saved".to_owned(),
        },
    );
    assert!(
        state
            .errors
            .iter()
            .any(|error| error.code == "scheduled_send_persistence_failed")
    );

    state.sync = koushi_state::SyncState::Running;
    let effects = reduce(
        &mut state,
        AppAction::SyncFailed {
            reason: "sync_failed_auth".to_owned(),
        },
    );

    assert_eq!(state.session, SessionState::Locked(session_info()));
    assert!(
        !state
            .errors
            .iter()
            .any(|error| error.code == "scheduled_send_persistence_failed"),
        "the retired session must not keep its persistence notice"
    );
    // The auth failure keeps its own explanation.
    assert!(
        state
            .errors
            .iter()
            .any(|error| error.code == "sync_auth_required")
    );
    assert!(effects.contains(&koushi_state::AppEffect::EmitUiEvent(UiEvent::ErrorChanged)));
}

fn server_item(id: &str, room_id: &str, send_at_ms: u64, delay_id: &str) -> ScheduledSendItem {
    ScheduledSendItem {
        handle: ScheduledSendHandle::Server {
            delay_id: delay_id.to_owned(),
        },
        ..scheduled_item(id, room_id, send_at_ms)
    }
}

#[test]
fn delivered_server_item_leaves_the_queue_and_the_projection() {
    // #1973: the homeserver fires a server-handle item itself; once Core sees
    // the delayed event is no longer scheduled the item must leave the queue.
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::ScheduledSendCreated {
            item: server_item("sched-1", "room-a", 1_900_000_000_000, "delay-1"),
        },
    );
    assert_eq!(
        state
            .scheduled_sends
            .next_server_check_due(1_900_000_000_000)
            .map(|item| item.scheduled_id),
        Some("sched-1".to_owned())
    );
    assert!(state.scheduled_sends.next_local_due(u64::MAX).is_none());

    reduce(
        &mut state,
        AppAction::ScheduledSendServerCheckStarted {
            scheduled_id: "sched-1".to_owned(),
            delay_id: "delay-1".to_owned(),
        },
    );
    assert!(
        state
            .scheduled_sends
            .next_server_check_due(u64::MAX)
            .is_none(),
        "a check in flight is not started twice"
    );

    let effects = reduce(
        &mut state,
        AppAction::ScheduledSendServerDelivered {
            scheduled_id: "sched-1".to_owned(),
            delay_id: "delay-1".to_owned(),
        },
    );
    assert!(state.scheduled_sends.items.is_empty());
    assert!(state.timeline.scheduled_sends.is_empty());
    assert!(effects.iter().any(|effect| matches!(
        effect,
        koushi_state::AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })
            if room_id == "room-a"
    )));
}

#[test]
fn deferred_server_check_waits_until_the_recheck_time() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::ScheduledSendCreated {
            item: server_item("sched-1", "room-a", 1_900_000_000_000, "delay-1"),
        },
    );
    reduce(
        &mut state,
        AppAction::ScheduledSendServerCheckStarted {
            scheduled_id: "sched-1".to_owned(),
            delay_id: "delay-1".to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::ScheduledSendServerCheckDeferred {
            scheduled_id: "sched-1".to_owned(),
            delay_id: "delay-1".to_owned(),
            recheck_at_ms: 1_900_000_010_000,
        },
    );

    assert_eq!(state.scheduled_sends.items.len(), 1);
    assert!(
        state
            .scheduled_sends
            .next_server_check_due(1_900_000_009_999)
            .is_none()
    );
    assert_eq!(
        state.scheduled_sends.next_server_send_at_ms(),
        Some(1_900_000_010_000)
    );
}

#[test]
fn server_check_result_for_a_replaced_handle_is_ignored() {
    // A reschedule cancels the old delayed event and creates a new one, so a
    // late "no longer scheduled" answer for the old handle must not retire the
    // rescheduled item.
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::ScheduledSendCreated {
            item: server_item("sched-1", "room-a", 1_900_000_000_000, "delay-old"),
        },
    );
    reduce(
        &mut state,
        AppAction::ScheduledSendRescheduled {
            scheduled_id: "sched-1".to_owned(),
            body: "rescheduled".to_owned(),
            send_at_ms: 1_900_000_060_000,
            handle: ScheduledSendHandle::Server {
                delay_id: "delay-new".to_owned(),
            },
        },
    );

    for action in [
        AppAction::ScheduledSendServerCheckStarted {
            scheduled_id: "sched-1".to_owned(),
            delay_id: "delay-old".to_owned(),
        },
        AppAction::ScheduledSendServerCheckDeferred {
            scheduled_id: "sched-1".to_owned(),
            delay_id: "delay-old".to_owned(),
            recheck_at_ms: 1,
        },
        AppAction::ScheduledSendServerDelivered {
            scheduled_id: "sched-1".to_owned(),
            delay_id: "delay-old".to_owned(),
        },
    ] {
        assert!(reduce(&mut state, action).is_empty());
    }
    let item = &state.scheduled_sends.items["sched-1"];
    assert_eq!(item.send_at_ms, 1_900_000_060_000);
    assert!(!item.is_dispatching);
}

#[test]
fn a_check_result_that_arrives_without_a_ready_session_does_not_wedge_the_item() {
    // #1973 review: the reply is dropped when the session stops being Ready, so
    // the in-flight mark has to be released too, or nothing re-checks the item
    // until the app restarts.
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::ScheduledSendCreated {
            item: server_item("sched-1", "room-a", 1_900_000_000_000, "delay-1"),
        },
    );
    reduce(
        &mut state,
        AppAction::ScheduledSendServerCheckStarted {
            scheduled_id: "sched-1".to_owned(),
            delay_id: "delay-1".to_owned(),
        },
    );
    assert!(
        state
            .scheduled_sends
            .next_server_check_due(u64::MAX)
            .is_none(),
        "a check in flight is not started twice"
    );

    state.session = SessionState::Restoring;
    for action in [
        AppAction::ScheduledSendServerCheckDeferred {
            scheduled_id: "sched-1".to_owned(),
            delay_id: "delay-1".to_owned(),
            recheck_at_ms: 1_900_000_010_000,
        },
        AppAction::ScheduledSendServerDelivered {
            scheduled_id: "sched-1".to_owned(),
            delay_id: "delay-1".to_owned(),
        },
    ] {
        assert!(reduce(&mut state, action).is_empty());
        assert_eq!(
            state
                .scheduled_sends
                .next_server_check_due(u64::MAX)
                .map(|item| item.scheduled_id),
            Some("sched-1".to_owned()),
            "a dropped result must leave the item due for the next wake"
        );
    }
    // The dropped deferred result must not move the user's time either.
    assert_eq!(
        state.scheduled_sends.items["sched-1"].send_at_ms,
        1_900_000_000_000
    );
}

#[test]
fn server_check_actions_never_touch_local_items() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::ScheduledSendCreated {
            item: scheduled_item("sched-local", "room-a", 1_900_000_000_000),
        },
    );
    assert!(
        state
            .scheduled_sends
            .next_server_check_due(u64::MAX)
            .is_none()
    );
    reduce(
        &mut state,
        AppAction::ScheduledSendServerDelivered {
            scheduled_id: "sched-local".to_owned(),
            delay_id: "anything".to_owned(),
        },
    );
    assert_eq!(state.scheduled_sends.items.len(), 1);
}
