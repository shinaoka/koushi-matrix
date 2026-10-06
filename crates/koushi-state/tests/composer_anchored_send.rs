//! #1037: a main-composer send accepted while the main pane is anchored to
//! historical context must return the selected room to the live timeline on
//! local acceptance, so the pending local echo is visible without clicking
//! "Jump to latest message". Every case here dispatches only acceptance /
//! submission actions: no send completion and no remote echo.

use koushi_state::{
    AppAction, AppEffect, AppState, ComposerMode, EventNavigationSource, EventNavigationState,
    FocusedContextState, MainTimelineAnchor, PendingComposerSendKind, RoomSummary, RoomTags,
    SessionInfo, SessionState, SubmissionId, ThreadOpenIntent, TimelineScrollAnchor,
    TimelineScrollAnchorEdge, UiEvent, reduce,
};

const ROOM: &str = "!room-a:example.test";
const OTHER_ROOM: &str = "!room-b:example.test";
const ANCHOR_EVENT: &str = "$historical:example.test";

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.test".to_owned(),
        user_id: "@user-a:example.test".to_owned(),
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

fn selected_room_state() -> AppState {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        rooms: vec![room(ROOM), room(OTHER_ROOM)],
        ..AppState::default()
    };
    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: ROOM.to_owned(),
        },
    );
    assert_eq!(state.timeline.room_id.as_deref(), Some(ROOM));
    state
}

/// Seeds a persisted live scroll anchor, then anchors the main pane to
/// historical context exactly like search/pinned event navigation does.
fn anchor_main_pane(state: &mut AppState) {
    reduce(
        state,
        AppAction::TimelineScrollAnchorUpdated {
            room_id: ROOM.to_owned(),
            anchor: TimelineScrollAnchor {
                event_id: "$old-live-position:example.test".to_owned(),
                edge: TimelineScrollAnchorEdge::Bottom,
                offset_px: 0,
                updated_at_ms: 1_700_000_000_000,
            },
        },
    );
    let generation = state.navigation.event_navigation.generation() + 1;
    reduce(
        state,
        AppAction::EventNavigationStarted {
            source: EventNavigationSource::Search,
        },
    );
    reduce(
        state,
        AppAction::OpenFocusedContext {
            room_id: ROOM.to_owned(),
            event_id: ANCHOR_EVENT.to_owned(),
        },
    );
    reduce(
        state,
        AppAction::EnterAnchoredTimeline {
            room_id: ROOM.to_owned(),
            event_id: ANCHOR_EVENT.to_owned(),
        },
    );
    reduce(state, AppAction::EventNavigationAnchored { generation });
    assert_anchored(state);
}

fn assert_anchored(state: &AppState) {
    assert_eq!(
        state.navigation.main_timeline_anchor,
        Some(MainTimelineAnchor {
            event_id: ANCHOR_EVENT.to_owned(),
        })
    );
    assert!(matches!(
        state.navigation.event_navigation,
        EventNavigationState::Anchored { .. }
    ));
    assert_ne!(state.focused_context, FocusedContextState::Closed);
    assert!(state.navigation.room_scroll_anchors.contains_key(ROOM));
}

fn assert_returned_to_live(state: &AppState) {
    assert_eq!(state.navigation.main_timeline_anchor, None);
    assert_eq!(
        state.navigation.event_navigation,
        EventNavigationState::Idle
    );
    assert_eq!(state.focused_context, FocusedContextState::Closed);
    // The stale pre-jump scroll position must not be restored: the live
    // timeline pins to its live edge where the pending echo is projected.
    assert!(!state.navigation.room_scroll_anchors.contains_key(ROOM));
    assert_eq!(state.navigation.active_room_id.as_deref(), Some(ROOM));
}

fn expected_send_effects(transaction_id: &str, body: &str) -> Vec<AppEffect> {
    vec![
        AppEffect::SendText {
            room_id: ROOM.to_owned(),
            transaction_id: transaction_id.to_owned(),
            body: body.to_owned(),
        },
        AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
            room_id: ROOM.to_owned(),
        }),
        cancel_pending_navigation_effect(),
    ]
}

/// Core cancels main-pane navigation it owns but the reducer cannot see (a
/// date jump or event navigation awaiting its focused projection); the Core
/// runtime tests in `koushi-core` (`runtime::tests::anchored_send`) prove a
/// late completion then cannot re-anchor.
fn cancel_pending_navigation_effect() -> AppEffect {
    AppEffect::CancelPendingMainTimelineNavigation {
        room_id: ROOM.to_owned(),
    }
}

fn accepted(submission_id: &str, room_id: &str, transaction_id: &str, body: &str) -> AppAction {
    AppAction::ComposerSubmissionAccepted {
        submission_id: SubmissionId::new(submission_id),
        room_id: room_id.to_owned(),
        transaction_id: transaction_id.to_owned(),
        body: body.to_owned(),
    }
}

#[test]
fn accepted_plain_send_returns_anchored_main_pane_to_live() {
    let mut state = selected_room_state();
    anchor_main_pane(&mut state);

    let effects = reduce(
        &mut state,
        accepted("sub-plain", ROOM, "txn-plain", "hello"),
    );

    assert_eq!(effects, expected_send_effects("txn-plain", "hello"));
    assert_returned_to_live(&state);
    assert_eq!(
        state.timeline.composer.pending_transaction_id.as_deref(),
        Some("txn-plain")
    );
    assert_eq!(
        state.timeline.composer.pending_send_kind,
        Some(PendingComposerSendKind::Plain)
    );
}

#[test]
fn accepted_main_reply_returns_to_live_and_preserves_reply_metadata() {
    let mut state = selected_room_state();
    anchor_main_pane(&mut state);
    reduce(
        &mut state,
        AppAction::ComposerReplyTargetSelected {
            room_id: ROOM.to_owned(),
            event_id: ANCHOR_EVENT.to_owned(),
        },
    );

    let draft_revision = state.composer_drafts.room_revision(ROOM);
    let effects = reduce(
        &mut state,
        AppAction::ComposerSubmissionAcceptedAtRevision {
            submission_id: SubmissionId::new("sub-reply"),
            room_id: ROOM.to_owned(),
            transaction_id: "txn-reply".to_owned(),
            body: "reply body".to_owned(),
            draft_revision,
        },
    );

    assert_eq!(effects, expected_send_effects("txn-reply", "reply body"));
    assert_returned_to_live(&state);
    assert_eq!(
        state.timeline.composer.pending_send_kind,
        Some(PendingComposerSendKind::Reply {
            in_reply_to_event_id: ANCHOR_EVENT.to_owned(),
        })
    );
    // Reply mode is only cleared by the send terminal, not by acceptance.
    assert_eq!(
        state.timeline.composer.mode,
        ComposerMode::Reply {
            in_reply_to_event_id: ANCHOR_EVENT.to_owned(),
        }
    );
}

#[test]
fn legacy_send_text_submitted_returns_anchored_main_pane_to_live() {
    let mut state = selected_room_state();
    anchor_main_pane(&mut state);

    let effects = reduce(
        &mut state,
        AppAction::SendTextSubmitted {
            room_id: ROOM.to_owned(),
            transaction_id: "txn-legacy".to_owned(),
            body: "legacy".to_owned(),
        },
    );

    assert_eq!(effects, expected_send_effects("txn-legacy", "legacy"));
    assert_returned_to_live(&state);
}

#[test]
fn accepted_send_from_a_live_pane_still_asks_core_to_cancel_pending_navigation() {
    // A date jump in flight leaves the main pane live (Core closed the old
    // context and has not entered the anchor yet), so the reducer cannot see
    // the intent. The accepted send must still hand Core the cancel request.
    let mut state = selected_room_state();
    reduce(
        &mut state,
        AppAction::OpenFocusedContext {
            room_id: ROOM.to_owned(),
            event_id: ANCHOR_EVENT.to_owned(),
        },
    );
    assert_eq!(state.navigation.main_timeline_anchor, None);

    let effects = reduce(&mut state, accepted("sub-jump", ROOM, "txn-jump", "hello"));

    assert_eq!(effects, expected_send_effects("txn-jump", "hello"));
}

fn accepted_attachments(room_id: &str, state: &AppState) -> AppAction {
    AppAction::ComposerDraftAccepted {
        target: koushi_state::ComposerTarget::Main {
            room_id: room_id.to_owned(),
        },
        submitted_revision: state.composer_drafts.room_revision(room_id),
        // #1130: a staged-attachment send settles the draft it never dispatched.
        consumes_draft: false,
    }
}

#[test]
fn accepted_attachment_send_returns_anchored_main_pane_to_live() {
    // Prepared-upload sends accept the main draft through ComposerDraftAccepted
    // after every upload was queued (their echoes are already live).
    let mut state = selected_room_state();
    anchor_main_pane(&mut state);

    let action = accepted_attachments(ROOM, &state);
    let effects = reduce(&mut state, action);

    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: ROOM.to_owned(),
            }),
            cancel_pending_navigation_effect(),
        ]
    );
    assert_returned_to_live(&state);
}

/// #1130: the staged-attachment send dispatches only the staged items, so
/// accepting it must settle the draft revision without deleting the text the
/// user still has to send. The #1037 return-to-live behaviour is unchanged.
#[test]
fn accepted_attachment_send_settles_the_draft_without_clearing_it() {
    let mut state = selected_room_state();
    anchor_main_pane(&mut state);
    reduce(
        &mut state,
        AppAction::ComposerDraftChanged {
            room_id: ROOM.to_owned(),
            document: koushi_state::ComposerDocument::from_plain_text("Here is the error:"),
        },
    );
    let revision_before = state.composer_drafts.room_revision(ROOM);
    assert_eq!(state.timeline.composer.draft, "Here is the error:");

    let action = accepted_attachments(ROOM, &state);
    let effects = reduce(&mut state, action);

    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: ROOM.to_owned(),
            }),
            cancel_pending_navigation_effect(),
        ]
    );
    assert_returned_to_live(&state);
    assert_eq!(state.timeline.composer.draft, "Here is the error:");
    assert_eq!(state.timeline.composer.document.inlines.len(), 1);
    assert!(
        state.composer_drafts.room_revision(ROOM) > revision_before,
        "the settled revision still advances"
    );
}

/// The text-consuming path keeps clearing: a plain or reply send really did
/// dispatch the draft.
#[test]
fn accepted_text_send_still_clears_the_draft() {
    let mut state = selected_room_state();
    reduce(
        &mut state,
        AppAction::ComposerDraftChanged {
            room_id: ROOM.to_owned(),
            document: koushi_state::ComposerDocument::from_plain_text("sent body"),
        },
    );

    let submitted_revision = state.composer_drafts.room_revision(ROOM);
    reduce(
        &mut state,
        AppAction::ComposerDraftAccepted {
            target: koushi_state::ComposerTarget::Main {
                room_id: ROOM.to_owned(),
            },
            submitted_revision,
            consumes_draft: true,
        },
    );

    assert_eq!(state.timeline.composer.draft, "");
    assert!(state.timeline.composer.document.inlines.is_empty());
}

#[test]
fn rejected_or_other_target_attachment_acceptance_keeps_navigation() {
    let mut state = selected_room_state();
    anchor_main_pane(&mut state);

    // A stale (exhausted) revision is rejected.
    let stale = reduce(
        &mut state,
        AppAction::ComposerDraftAccepted {
            target: koushi_state::ComposerTarget::Main {
                room_id: ROOM.to_owned(),
            },
            submitted_revision: koushi_state::ComposerDraftRevision::MAX,
            consumes_draft: false,
        },
    );
    assert!(stale.is_empty());
    assert_anchored(&state);

    // A background room's draft acceptance does not touch the selected room.
    let action = accepted_attachments(OTHER_ROOM, &state);
    let background = reduce(&mut state, action);
    assert!(background.is_empty());
    assert_anchored(&state);

    // A thread-target attachment send leaves main navigation alone.
    let thread = reduce(
        &mut state,
        AppAction::ComposerDraftAccepted {
            target: koushi_state::ComposerTarget::Thread {
                room_id: ROOM.to_owned(),
                root_event_id: "$root:example.test".to_owned(),
            },
            submitted_revision: 0.into(),
            consumes_draft: false,
        },
    );
    assert!(
        !thread.contains(&cancel_pending_navigation_effect()),
        "thread sends must not cancel main-pane navigation"
    );
    assert_anchored(&state);
}

#[test]
fn duplicate_accepted_submission_does_not_change_navigation() {
    let mut state = selected_room_state();
    reduce(&mut state, accepted("sub-dup", ROOM, "txn-dup", "hello"));
    anchor_main_pane(&mut state);

    let effects = reduce(&mut state, accepted("sub-dup", ROOM, "txn-dup-2", "hello"));

    assert!(effects.is_empty());
    assert_anchored(&state);
}

#[test]
fn rejected_submissions_do_not_change_navigation() {
    let mut state = selected_room_state();
    // A send is already pending: a second submission is rejected.
    reduce(
        &mut state,
        accepted("sub-first", ROOM, "txn-first", "first"),
    );
    anchor_main_pane(&mut state);
    let pending_rejected = reduce(&mut state, accepted("sub-second", ROOM, "txn-second", "x"));
    assert!(pending_rejected.is_empty());
    assert_anchored(&state);

    let legacy_rejected = reduce(
        &mut state,
        AppAction::SendTextSubmitted {
            room_id: ROOM.to_owned(),
            transaction_id: "txn-legacy-second".to_owned(),
            body: "x".to_owned(),
        },
    );
    assert!(legacy_rejected.is_empty());
    assert_anchored(&state);

    // An exhausted draft revision is rejected before acceptance.
    let mut state = selected_room_state();
    anchor_main_pane(&mut state);
    let exhausted = reduce(
        &mut state,
        AppAction::ComposerSubmissionAcceptedAtRevision {
            submission_id: SubmissionId::new("sub-exhausted"),
            room_id: ROOM.to_owned(),
            transaction_id: "txn-exhausted".to_owned(),
            body: "x".to_owned(),
            draft_revision: koushi_state::ComposerDraftRevision::MAX,
        },
    );
    assert!(exhausted.is_empty());
    assert_anchored(&state);
}

#[test]
fn background_room_send_does_not_change_selected_room_navigation() {
    let mut state = selected_room_state();
    anchor_main_pane(&mut state);

    let effects = reduce(
        &mut state,
        accepted("sub-background", OTHER_ROOM, "txn-background", "hello"),
    );
    assert!(effects.is_empty());
    let legacy = reduce(
        &mut state,
        AppAction::SendTextSubmitted {
            room_id: OTHER_ROOM.to_owned(),
            transaction_id: "txn-background-legacy".to_owned(),
            body: "hello".to_owned(),
        },
    );
    assert!(legacy.is_empty());
    assert_anchored(&state);
}

#[test]
fn thread_composer_send_does_not_change_main_navigation() {
    let mut state = selected_room_state();
    reduce(
        &mut state,
        AppAction::OpenThread {
            room_id: ROOM.to_owned(),
            root_event_id: "$root:example.test".to_owned(),
            intent: ThreadOpenIntent::ExistingThread,
        },
    );
    reduce(
        &mut state,
        AppAction::ThreadSubscribed {
            room_id: ROOM.to_owned(),
            root_event_id: "$root:example.test".to_owned(),
        },
    );
    anchor_main_pane(&mut state);

    reduce(
        &mut state,
        AppAction::ThreadComposerDraftChangedAtRevision {
            room_id: ROOM.to_owned(),
            root_event_id: "$root:example.test".to_owned(),
            document: "thread reply".into(),
            revision: 1.into(),
        },
    );
    let effects = reduce(
        &mut state,
        AppAction::ThreadSubmissionAcceptedAtRevision {
            submission_id: SubmissionId::new("sub-thread"),
            room_id: ROOM.to_owned(),
            root_event_id: "$root:example.test".to_owned(),
            transaction_id: "txn-thread".to_owned(),
            body: "thread reply".to_owned(),
            draft_revision: 1.into(),
        },
    );
    assert!(!effects.is_empty(), "thread send should be accepted");
    assert_anchored(&state);
}

#[test]
fn live_main_send_preserves_independent_right_panel_focused_context() {
    let mut state = selected_room_state();
    // Right-panel focused context without main-pane anchoring.
    reduce(
        &mut state,
        AppAction::OpenFocusedContext {
            room_id: ROOM.to_owned(),
            event_id: ANCHOR_EVENT.to_owned(),
        },
    );
    let focused_before = state.focused_context.clone();
    assert_ne!(focused_before, FocusedContextState::Closed);
    assert_eq!(state.navigation.main_timeline_anchor, None);

    let effects = reduce(&mut state, accepted("sub-live", ROOM, "txn-live", "hello"));

    assert_eq!(effects, expected_send_effects("txn-live", "hello"));
    assert_eq!(state.focused_context, focused_before);
    assert_eq!(
        state.navigation.event_navigation,
        EventNavigationState::Idle
    );
}
