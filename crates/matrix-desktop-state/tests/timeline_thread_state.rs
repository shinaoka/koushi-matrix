use matrix_desktop_state::{
    AppAction, AppEffect, AppState, ComposerMode, ComposerState, NavigationState,
    PendingComposerSendKind, RoomSummary, RoomTags, SessionInfo, SessionState,
    ThreadAttentionState, ThreadPaneState, TimelinePaneState, UiEvent, reduce,
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

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(session_info()),
        rooms: vec![room("room-a"), room("room-b")],
        ..AppState::default()
    }
}

fn selected_room_state(room_id: &str) -> AppState {
    let mut state = ready_state();

    let effects = reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: room_id.to_owned(),
        },
    );

    assert_eq!(state.navigation.active_room_id.as_deref(), Some(room_id));
    assert_eq!(state.timeline.room_id.as_deref(), Some(room_id));
    assert_eq!(
        effects,
        vec![
            AppEffect::SubscribeTimeline {
                room_id: room_id.to_owned(),
            },
            AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: room_id.to_owned(),
            }),
        ]
    );

    state
}

#[test]
fn timeline_subscription_success_marks_selected_room_subscribed() {
    let mut state = selected_room_state("room-a");

    let effects = reduce(
        &mut state,
        AppAction::TimelineSubscribed {
            room_id: "room-a".to_owned(),
        },
    );

    assert!(state.timeline.is_subscribed);
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
            room_id: "room-a".to_owned(),
        })]
    );
}

#[test]
fn timeline_subscription_failure_records_error_for_active_room() {
    let mut state = selected_room_state("room-a");

    let effects = reduce(
        &mut state,
        AppAction::TimelineSubscriptionFailed {
            room_id: "room-a".to_owned(),
            message: "fixture-access-token rejected synthetic-password".to_owned(),
        },
    );

    assert_eq!(state.errors.len(), 1);
    assert_eq!(state.errors[0].code, "timeline_subscription_failed");
    assert_eq!(
        state.errors[0].message,
        "Matrix timeline subscription failed"
    );
    let formatted_errors = format!("{:?}", state.errors);
    assert!(!formatted_errors.contains("fixture-access-token"));
    assert!(!formatted_errors.contains("synthetic-password"));
    assert!(state.errors[0].recoverable);
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: "room-a".to_owned(),
            }),
            AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
        ]
    );
}

#[test]
fn timeline_subscription_signals_for_stale_room_are_ignored() {
    let mut state = selected_room_state("room-a");
    let previous_state = state.clone();

    assert_eq!(
        reduce(
            &mut state,
            AppAction::TimelineSubscribed {
                room_id: "room-b".to_owned(),
            },
        ),
        Vec::new()
    );
    assert_eq!(
        reduce(
            &mut state,
            AppAction::TimelineSubscriptionFailed {
                room_id: "room-b".to_owned(),
                message: "late failure".to_owned(),
            },
        ),
        Vec::new()
    );
    assert_eq!(state, previous_state);
}

#[test]
fn composer_draft_change_only_affects_active_room() {
    let mut state = selected_room_state("room-a");

    let effects = reduce(
        &mut state,
        AppAction::ComposerDraftChanged {
            room_id: "room-b".to_owned(),
            draft: "ignored".to_owned(),
        },
    );
    assert_eq!(effects, Vec::new());
    assert_eq!(state.timeline.composer.draft, "");

    let effects = reduce(
        &mut state,
        AppAction::ComposerDraftChanged {
            room_id: "room-a".to_owned(),
            draft: "hello".to_owned(),
        },
    );

    assert_eq!(state.timeline.composer.draft, "hello");
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
            room_id: "room-a".to_owned(),
        })]
    );
}

#[test]
fn composer_draft_is_restored_when_switching_back_to_room() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::ComposerDraftChanged {
            room_id: "room-a".to_owned(),
            draft: "room a draft".to_owned(),
        },
    );

    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "room-b".to_owned(),
        },
    );
    assert_eq!(state.timeline.composer.draft, "");

    reduce(
        &mut state,
        AppAction::ComposerDraftChanged {
            room_id: "room-b".to_owned(),
            draft: "room b draft".to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "room-a".to_owned(),
        },
    );

    assert_eq!(state.timeline.composer.draft, "room a draft");

    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "room-b".to_owned(),
        },
    );
    assert_eq!(state.timeline.composer.draft, "room b draft");
}

#[test]
fn composer_draft_store_is_cleared_on_send_and_room_removal() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::ComposerDraftChanged {
            room_id: "room-a".to_owned(),
            draft: "send me".to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::SendTextSubmitted {
            room_id: "room-a".to_owned(),
            transaction_id: "txn1".to_owned(),
            body: "send me".to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "room-b".to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "room-a".to_owned(),
        },
    );
    assert_eq!(state.timeline.composer.draft, "");

    reduce(
        &mut state,
        AppAction::ComposerDraftChanged {
            room_id: "room-a".to_owned(),
            draft: "prune me".to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![room("room-b")],
        },
    );

    assert!(state.composer_drafts.rooms.is_empty());
}

#[test]
fn send_text_sets_pending_transaction_and_emits_send_effect() {
    let mut state = selected_room_state("room-a");
    state.timeline.composer.draft = "hello".to_owned();

    let effects = reduce(
        &mut state,
        AppAction::SendTextSubmitted {
            room_id: "room-a".to_owned(),
            transaction_id: "txn1".to_owned(),
            body: "hello".to_owned(),
        },
    );

    assert_eq!(
        state.timeline.composer.pending_transaction_id.as_deref(),
        Some("txn1")
    );
    assert_eq!(state.timeline.composer.draft, "");
    assert_eq!(
        effects,
        vec![
            AppEffect::SendText {
                room_id: "room-a".to_owned(),
                transaction_id: "txn1".to_owned(),
                body: "hello".to_owned(),
            },
            AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: "room-a".to_owned(),
            }),
        ]
    );
}

#[test]
fn send_text_submission_is_ignored_while_send_is_pending() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::SendTextSubmitted {
            room_id: "room-a".to_owned(),
            transaction_id: "txn1".to_owned(),
            body: "hello".to_owned(),
        },
    );
    state.timeline.composer.draft = "second".to_owned();

    let effects = reduce(
        &mut state,
        AppAction::SendTextSubmitted {
            room_id: "room-a".to_owned(),
            transaction_id: "txn2".to_owned(),
            body: "second".to_owned(),
        },
    );

    assert_eq!(effects, Vec::new());
    assert_eq!(
        state.timeline.composer.pending_transaction_id.as_deref(),
        Some("txn1")
    );
    assert_eq!(state.timeline.composer.draft, "second");
}

#[test]
fn send_text_submission_for_stale_room_is_ignored() {
    let mut state = selected_room_state("room-a");
    state.timeline.composer.draft = "hello".to_owned();
    let previous_state = state.clone();

    let effects = reduce(
        &mut state,
        AppAction::SendTextSubmitted {
            room_id: "room-b".to_owned(),
            transaction_id: "txn1".to_owned(),
            body: "hello".to_owned(),
        },
    );

    assert_eq!(effects, Vec::new());
    assert_eq!(state, previous_state);
}

#[test]
fn send_text_finished_clears_matching_active_transaction() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::SendTextSubmitted {
            room_id: "room-a".to_owned(),
            transaction_id: "txn1".to_owned(),
            body: "hello".to_owned(),
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::SendTextFinished {
            room_id: "room-b".to_owned(),
            transaction_id: "txn1".to_owned(),
        },
    );
    assert_eq!(effects, Vec::new());
    assert_eq!(
        state.timeline.composer.pending_transaction_id.as_deref(),
        Some("txn1")
    );

    let effects = reduce(
        &mut state,
        AppAction::SendTextFinished {
            room_id: "room-a".to_owned(),
            transaction_id: "txn2".to_owned(),
        },
    );
    assert_eq!(effects, Vec::new());
    assert_eq!(
        state.timeline.composer.pending_transaction_id.as_deref(),
        Some("txn1")
    );

    let effects = reduce(
        &mut state,
        AppAction::SendTextFinished {
            room_id: "room-a".to_owned(),
            transaction_id: "txn1".to_owned(),
        },
    );

    assert_eq!(state.timeline.composer.pending_transaction_id, None);
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
            room_id: "room-a".to_owned(),
        })]
    );
}

#[test]
fn timeline_backward_pagination_sets_pending_and_clears_on_completion() {
    let mut state = selected_room_state("room-a");

    let effects = reduce(
        &mut state,
        AppAction::TimelineBackPaginationRequested {
            room_id: "room-a".to_owned(),
        },
    );

    assert!(state.timeline.is_paginating_backwards);
    assert_eq!(
        effects,
        vec![
            AppEffect::PaginateTimelineBackwards {
                room_id: "room-a".to_owned(),
            },
            AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: "room-a".to_owned(),
            }),
        ]
    );

    assert_eq!(
        reduce(
            &mut state,
            AppAction::TimelineBackPaginationRequested {
                room_id: "room-a".to_owned(),
            },
        ),
        Vec::new()
    );

    let effects = reduce(
        &mut state,
        AppAction::TimelineBackPaginationFinished {
            room_id: "room-a".to_owned(),
        },
    );

    assert!(!state.timeline.is_paginating_backwards);
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
            room_id: "room-a".to_owned(),
        })]
    );
}

#[test]
fn stale_timeline_backward_pagination_signals_are_ignored() {
    let mut state = selected_room_state("room-a");
    let previous_state = state.clone();

    assert_eq!(
        reduce(
            &mut state,
            AppAction::TimelineBackPaginationRequested {
                room_id: "room-b".to_owned(),
            },
        ),
        Vec::new()
    );
    assert_eq!(
        reduce(
            &mut state,
            AppAction::TimelineBackPaginationFinished {
                room_id: "room-b".to_owned(),
            },
        ),
        Vec::new()
    );
    assert_eq!(state, previous_state);
}

#[test]
fn opening_thread_requests_thread_timeline_and_subscription_success_opens_pane() {
    let mut state = selected_room_state("room-a");

    let effects = reduce(
        &mut state,
        AppAction::OpenThread {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
        },
    );

    assert_eq!(
        state.thread,
        ThreadPaneState::Opening {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
        }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::OpenThreadTimeline {
                room_id: "room-a".to_owned(),
                root_event_id: "$root".to_owned(),
            },
            AppEffect::EmitUiEvent(UiEvent::ThreadChanged),
        ]
    );

    let effects = reduce(
        &mut state,
        AppAction::ThreadSubscribed {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
        },
    );

    assert_eq!(
        state.thread,
        ThreadPaneState::Open {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            is_subscribed: true,
            composer: ComposerState::default(),
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
    );
}

#[test]
fn thread_attention_updates_matching_open_thread_only() {
    let mut state = open_thread_state("room-a", "$root");

    assert_eq!(
        state.thread_attention,
        ThreadAttentionState::Tracking {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            notification_count: 0,
            highlight_count: 0,
            live_event_marker_count: 0,
        }
    );

    let effects = reduce(
        &mut state,
        AppAction::ThreadAttentionUpdated {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            notification_count: 4,
            highlight_count: 1,
            live_event_marker_count: 2,
        },
    );

    assert_eq!(
        state.thread_attention,
        ThreadAttentionState::Tracking {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            notification_count: 4,
            highlight_count: 1,
            live_event_marker_count: 2,
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
    );

    let previous = state.clone();
    let effects = reduce(
        &mut state,
        AppAction::ThreadAttentionUpdated {
            room_id: "room-a".to_owned(),
            root_event_id: "$other-root".to_owned(),
            notification_count: 9,
            highlight_count: 9,
            live_event_marker_count: 9,
        },
    );

    assert_eq!(state, previous);
    assert!(effects.is_empty());
}

fn open_thread_state(room_id: &str, root_event_id: &str) -> AppState {
    let mut state = selected_room_state(room_id);
    reduce(
        &mut state,
        AppAction::OpenThread {
            room_id: room_id.to_owned(),
            root_event_id: root_event_id.to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::ThreadSubscribed {
            room_id: room_id.to_owned(),
            root_event_id: root_event_id.to_owned(),
        },
    );
    state
}

fn open_thread_composer(state: &AppState) -> &ComposerState {
    match &state.thread {
        ThreadPaneState::Open { composer, .. } => composer,
        other => panic!("expected open thread, got {other:?}"),
    }
}

fn set_open_thread_draft(state: &mut AppState, draft: &str) {
    match &mut state.thread {
        ThreadPaneState::Open { composer, .. } => composer.draft = draft.to_owned(),
        other => panic!("expected open thread, got {other:?}"),
    }
}

#[test]
fn thread_composer_draft_change_only_affects_matching_open_thread() {
    let mut state = open_thread_state("room-a", "$root");
    state.timeline.composer.draft = "main draft".to_owned();

    let effects = reduce(
        &mut state,
        AppAction::ThreadComposerDraftChanged {
            room_id: "room-a".to_owned(),
            root_event_id: "$other".to_owned(),
            draft: "ignored".to_owned(),
        },
    );
    assert_eq!(effects, Vec::new());
    assert_eq!(open_thread_composer(&state).draft, "");

    let effects = reduce(
        &mut state,
        AppAction::ThreadComposerDraftChanged {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            draft: "thread draft".to_owned(),
        },
    );

    assert_eq!(open_thread_composer(&state).draft, "thread draft");
    assert_eq!(state.timeline.composer.draft, "main draft");
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
    );
}

#[test]
fn thread_composer_draft_is_restored_when_thread_reopens() {
    let mut state = open_thread_state("room-a", "$root");

    reduce(
        &mut state,
        AppAction::ThreadComposerDraftChanged {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            draft: "thread draft".to_owned(),
        },
    );
    reduce(&mut state, AppAction::CloseThread);
    assert_eq!(state.thread, ThreadPaneState::Closed);

    reduce(
        &mut state,
        AppAction::OpenThread {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::ThreadSubscribed {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
        },
    );

    assert_eq!(open_thread_composer(&state).draft, "thread draft");
}

#[test]
fn thread_composer_draft_store_is_cleared_on_reply_and_room_removal() {
    let mut state = open_thread_state("room-a", "$root");
    reduce(
        &mut state,
        AppAction::ThreadComposerDraftChanged {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            draft: "reply me".to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::ThreadReplySubmitted {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            transaction_id: "txn-thread".to_owned(),
            body: "reply me".to_owned(),
        },
    );
    reduce(&mut state, AppAction::CloseThread);
    reduce(
        &mut state,
        AppAction::OpenThread {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::ThreadSubscribed {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
        },
    );
    assert_eq!(open_thread_composer(&state).draft, "");

    reduce(
        &mut state,
        AppAction::ThreadComposerDraftChanged {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            draft: "prune me".to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![room("room-b")],
        },
    );

    assert!(state.composer_drafts.threads.is_empty());
}

#[test]
fn thread_reply_submit_sets_thread_pending_reply_and_leaves_main_composer_alone() {
    let mut state = open_thread_state("room-a", "$root");
    state.timeline.composer.draft = "main draft".to_owned();
    set_open_thread_draft(&mut state, "thread reply");

    let effects = reduce(
        &mut state,
        AppAction::ThreadReplySubmitted {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            transaction_id: "txn-thread".to_owned(),
            body: "thread reply".to_owned(),
        },
    );

    let composer = open_thread_composer(&state);
    assert_eq!(
        composer.pending_transaction_id.as_deref(),
        Some("txn-thread")
    );
    assert_eq!(
        composer.pending_send_kind,
        Some(PendingComposerSendKind::Reply {
            in_reply_to_event_id: "$root".to_owned(),
        })
    );
    assert_eq!(composer.draft, "");
    assert_eq!(state.timeline.composer.pending_transaction_id, None);
    assert_eq!(state.timeline.composer.draft, "main draft");
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
    );
}

#[test]
fn thread_reply_submit_is_ignored_unless_ready_matching_open_and_idle() {
    let mut state = open_thread_state("room-a", "$root");
    reduce(
        &mut state,
        AppAction::ThreadReplySubmitted {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            transaction_id: "txn-one".to_owned(),
            body: "first".to_owned(),
        },
    );
    set_open_thread_draft(&mut state, "second draft");
    let pending_state = state.clone();

    assert_eq!(
        reduce(
            &mut state,
            AppAction::ThreadReplySubmitted {
                room_id: "room-a".to_owned(),
                root_event_id: "$root".to_owned(),
                transaction_id: "txn-two".to_owned(),
                body: "second".to_owned(),
            },
        ),
        Vec::new()
    );
    assert_eq!(state, pending_state);

    let mut signed_out = AppState {
        session: SessionState::SignedOut,
        thread: ThreadPaneState::Open {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            is_subscribed: true,
            composer: ComposerState::default(),
        },
        ..AppState::default()
    };
    let signed_out_before = signed_out.clone();
    assert_eq!(
        reduce(
            &mut signed_out,
            AppAction::ThreadReplySubmitted {
                room_id: "room-a".to_owned(),
                root_event_id: "$root".to_owned(),
                transaction_id: "txn".to_owned(),
                body: "ignored".to_owned(),
            },
        ),
        Vec::new()
    );
    assert_eq!(signed_out, signed_out_before);
}

#[test]
fn thread_reply_finished_clears_only_matching_thread_transaction() {
    let mut state = open_thread_state("room-a", "$root");
    reduce(
        &mut state,
        AppAction::ThreadReplySubmitted {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            transaction_id: "txn-thread".to_owned(),
            body: "thread reply".to_owned(),
        },
    );

    for action in [
        AppAction::ThreadReplyFinished {
            room_id: "room-b".to_owned(),
            root_event_id: "$root".to_owned(),
            transaction_id: "txn-thread".to_owned(),
        },
        AppAction::ThreadReplyFinished {
            room_id: "room-a".to_owned(),
            root_event_id: "$other".to_owned(),
            transaction_id: "txn-thread".to_owned(),
        },
        AppAction::ThreadReplyFinished {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            transaction_id: "txn-other".to_owned(),
        },
    ] {
        assert_eq!(reduce(&mut state, action), Vec::new());
        assert_eq!(
            open_thread_composer(&state)
                .pending_transaction_id
                .as_deref(),
            Some("txn-thread")
        );
    }

    let effects = reduce(
        &mut state,
        AppAction::ThreadReplyFinished {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            transaction_id: "txn-thread".to_owned(),
        },
    );

    let composer = open_thread_composer(&state);
    assert_eq!(composer.pending_transaction_id, None);
    assert_eq!(composer.pending_send_kind, None);
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
    );
}

#[test]
fn thread_reply_failed_clears_matching_pending_and_records_recoverable_error() {
    let mut state = open_thread_state("room-a", "$root");
    reduce(
        &mut state,
        AppAction::ThreadReplySubmitted {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            transaction_id: "txn-thread".to_owned(),
            body: "thread reply".to_owned(),
        },
    );

    assert_eq!(
        reduce(
            &mut state,
            AppAction::ThreadReplyFailed {
                room_id: "room-a".to_owned(),
                root_event_id: "$root".to_owned(),
                transaction_id: "txn-other".to_owned(),
                message: "send failed".to_owned(),
            },
        ),
        Vec::new()
    );
    assert_eq!(
        open_thread_composer(&state)
            .pending_transaction_id
            .as_deref(),
        Some("txn-thread")
    );

    let effects = reduce(
        &mut state,
        AppAction::ThreadReplyFailed {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            transaction_id: "txn-thread".to_owned(),
            message: "send failed".to_owned(),
        },
    );

    let composer = open_thread_composer(&state);
    assert_eq!(composer.pending_transaction_id, None);
    assert_eq!(composer.pending_send_kind, None);
    assert_eq!(state.errors.len(), 1);
    assert_eq!(state.errors[0].code, "send_text_failed");
    assert_eq!(state.errors[0].message, "send failed");
    assert!(state.errors[0].recoverable);
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::ThreadChanged),
            AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
        ]
    );
}

#[test]
fn send_text_failed_clears_matching_active_transaction_and_records_error() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::SendTextSubmitted {
            room_id: "room-a".to_owned(),
            transaction_id: "txn1".to_owned(),
            body: "hello".to_owned(),
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::SendTextFailed {
            room_id: "room-b".to_owned(),
            transaction_id: "txn1".to_owned(),
            message: "Matrix send failed".to_owned(),
        },
    );
    assert_eq!(effects, Vec::new());
    assert_eq!(
        state.timeline.composer.pending_transaction_id.as_deref(),
        Some("txn1")
    );

    let effects = reduce(
        &mut state,
        AppAction::SendTextFailed {
            room_id: "room-a".to_owned(),
            transaction_id: "txn2".to_owned(),
            message: "Matrix send failed".to_owned(),
        },
    );
    assert_eq!(effects, Vec::new());
    assert_eq!(
        state.timeline.composer.pending_transaction_id.as_deref(),
        Some("txn1")
    );

    let effects = reduce(
        &mut state,
        AppAction::SendTextFailed {
            room_id: "room-a".to_owned(),
            transaction_id: "txn1".to_owned(),
            message: "Matrix send failed".to_owned(),
        },
    );

    assert_eq!(state.timeline.composer.pending_transaction_id, None);
    assert_eq!(state.errors.len(), 1);
    assert_eq!(state.errors[0].code, "send_text_failed");
    assert_eq!(state.errors[0].message, "Matrix send failed");
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: "room-a".to_owned(),
            }),
            AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
        ]
    );
}

#[test]
fn open_thread_only_affects_active_timeline_room() {
    let mut state = selected_room_state("room-a");
    let previous_state = state.clone();

    let effects = reduce(
        &mut state,
        AppAction::OpenThread {
            room_id: "room-b".to_owned(),
            root_event_id: "$root".to_owned(),
        },
    );

    assert_eq!(effects, Vec::new());
    assert_eq!(state, previous_state);
}

#[test]
fn thread_subscription_success_must_match_current_opening_thread() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::OpenThread {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
        },
    );

    let opening_state = state.clone();
    assert_eq!(
        reduce(
            &mut state,
            AppAction::ThreadSubscribed {
                room_id: "room-a".to_owned(),
                root_event_id: "$other".to_owned(),
            },
        ),
        Vec::new()
    );
    assert_eq!(
        reduce(
            &mut state,
            AppAction::ThreadSubscribed {
                room_id: "room-b".to_owned(),
                root_event_id: "$root".to_owned(),
            },
        ),
        Vec::new()
    );
    assert_eq!(state, opening_state);
}

#[test]
fn close_thread_only_notifies_when_thread_was_active() {
    let mut state = selected_room_state("room-a");

    assert_eq!(reduce(&mut state, AppAction::CloseThread), Vec::new());
    assert_eq!(state.thread, ThreadPaneState::Closed);

    reduce(
        &mut state,
        AppAction::OpenThread {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
        },
    );

    let effects = reduce(&mut state, AppAction::CloseThread);

    assert_eq!(state.thread, ThreadPaneState::Closed);
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
    );
}

#[test]
fn timeline_and_thread_actions_are_ignored_without_ready_session() {
    let state = AppState {
        session: SessionState::SignedOut,
        navigation: NavigationState {
            active_space_id: None,
            active_room_id: Some("room-a".to_owned()),
        },
        rooms: vec![room("room-a")],
        timeline: TimelinePaneState {
            room_id: Some("room-a".to_owned()),
            is_subscribed: false,
            is_paginating_backwards: false,
            composer: ComposerState {
                pending_transaction_id: Some("txn1".to_owned()),
                pending_send_kind: None,
                draft: "draft".to_owned(),
                mode: Default::default(),
            },
            scheduled_send_capability: Default::default(),
            scheduled_sends: Vec::new(),
        },
        thread: ThreadPaneState::Opening {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
        },
        ..AppState::default()
    };

    let actions = vec![
        AppAction::TimelineSubscribed {
            room_id: "room-a".to_owned(),
        },
        AppAction::TimelineSubscriptionFailed {
            room_id: "room-a".to_owned(),
            message: "failed".to_owned(),
        },
        AppAction::ComposerDraftChanged {
            room_id: "room-a".to_owned(),
            draft: "changed".to_owned(),
        },
        AppAction::SendTextSubmitted {
            room_id: "room-a".to_owned(),
            transaction_id: "txn2".to_owned(),
            body: "changed".to_owned(),
        },
        AppAction::SendTextFinished {
            room_id: "room-a".to_owned(),
            transaction_id: "txn1".to_owned(),
        },
        AppAction::SendTextFailed {
            room_id: "room-a".to_owned(),
            transaction_id: "txn1".to_owned(),
            message: "Matrix send failed".to_owned(),
        },
        AppAction::OpenThread {
            room_id: "room-a".to_owned(),
            root_event_id: "$other".to_owned(),
        },
        AppAction::ThreadSubscribed {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
        },
        AppAction::CloseThread,
    ];

    for action in actions {
        let mut state_for_action = state.clone();

        assert_eq!(reduce(&mut state_for_action, action), Vec::new());
        assert_eq!(state_for_action, state);
    }
}

#[test]
fn send_text_finished_clears_reply_mode_for_matching_reply_send() {
    let mut state = selected_room_state("room-a");
    state.timeline.composer.mode = ComposerMode::Reply {
        in_reply_to_event_id: "$root:example.invalid".to_owned(),
    };

    reduce(
        &mut state,
        AppAction::SendTextSubmitted {
            room_id: "room-a".to_owned(),
            transaction_id: "txn-reply".to_owned(),
            body: "reply body".to_owned(),
        },
    );

    reduce(
        &mut state,
        AppAction::SendTextFinished {
            room_id: "room-a".to_owned(),
            transaction_id: "txn-reply".to_owned(),
        },
    );

    assert_eq!(state.timeline.composer.pending_transaction_id, None);
    assert_eq!(state.timeline.composer.mode, ComposerMode::Plain);
}

#[test]
fn plain_send_completion_preserves_reply_selected_after_submission() {
    let mut state = selected_room_state("room-a");

    reduce(
        &mut state,
        AppAction::SendTextSubmitted {
            room_id: "room-a".to_owned(),
            transaction_id: "txn-plain".to_owned(),
            body: "plain body".to_owned(),
        },
    );

    reduce(
        &mut state,
        AppAction::ComposerReplyTargetSelected {
            room_id: "room-a".to_owned(),
            event_id: "$new-reply:example.invalid".to_owned(),
        },
    );

    reduce(
        &mut state,
        AppAction::SendTextFinished {
            room_id: "room-a".to_owned(),
            transaction_id: "txn-plain".to_owned(),
        },
    );

    assert_eq!(state.timeline.composer.pending_transaction_id, None);
    assert_eq!(
        state.timeline.composer.mode,
        ComposerMode::Reply {
            in_reply_to_event_id: "$new-reply:example.invalid".to_owned()
        }
    );
}

#[test]
fn reply_send_completion_preserves_newer_reply_target() {
    let mut state = selected_room_state("room-a");
    state.timeline.composer.mode = ComposerMode::Reply {
        in_reply_to_event_id: "$old-root:example.invalid".to_owned(),
    };

    reduce(
        &mut state,
        AppAction::SendTextSubmitted {
            room_id: "room-a".to_owned(),
            transaction_id: "txn-reply".to_owned(),
            body: "reply body".to_owned(),
        },
    );

    reduce(
        &mut state,
        AppAction::ComposerReplyTargetSelected {
            room_id: "room-a".to_owned(),
            event_id: "$new-root:example.invalid".to_owned(),
        },
    );

    reduce(
        &mut state,
        AppAction::SendTextFinished {
            room_id: "room-a".to_owned(),
            transaction_id: "txn-reply".to_owned(),
        },
    );

    assert_eq!(state.timeline.composer.pending_transaction_id, None);
    assert_eq!(
        state.timeline.composer.mode,
        ComposerMode::Reply {
            in_reply_to_event_id: "$new-root:example.invalid".to_owned()
        }
    );
}

#[test]
fn send_text_failed_preserves_reply_mode_for_retry() {
    let mut state = selected_room_state("room-a");
    state.timeline.composer.mode = ComposerMode::Reply {
        in_reply_to_event_id: "$root:example.invalid".to_owned(),
    };

    reduce(
        &mut state,
        AppAction::SendTextSubmitted {
            room_id: "room-a".to_owned(),
            transaction_id: "txn-reply".to_owned(),
            body: "reply body".to_owned(),
        },
    );

    reduce(
        &mut state,
        AppAction::SendTextFailed {
            room_id: "room-a".to_owned(),
            transaction_id: "txn-reply".to_owned(),
            message: "send failed".to_owned(),
        },
    );

    assert_eq!(state.timeline.composer.pending_transaction_id, None);
    assert_eq!(
        state.timeline.composer.mode,
        ComposerMode::Reply {
            in_reply_to_event_id: "$root:example.invalid".to_owned()
        }
    );
}
