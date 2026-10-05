//! #1037 / #1046: Core-level ownership of the focused (anchored) timeline when
//! the reducer leaves anchored history implicitly (accepted main send, room
//! switch), and fencing of in-flight main-pane navigation against an accepted
//! send. All identifiers are synthetic.

use super::*;

const ROOM: &str = "!anchored-send-room:example.invalid";
const OTHER_ROOM: &str = "!anchored-send-other:example.invalid";
const EVENT: &str = "$anchored-send-event:example.invalid";
const USER: &str = "@synthetic:example.invalid";

fn focused_key(room_id: &str, event_id: &str) -> TimelineKey {
    TimelineKey {
        account_key: AccountKey(USER.to_owned()),
        kind: TimelineKind::Focused {
            room_id: room_id.to_owned(),
            event_id: event_id.to_owned(),
        },
    }
}

fn request(sequence: u64) -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(1037),
        sequence,
    }
}

/// Ready session with `ROOM` selected and live.
fn selected_room_state() -> AppState {
    let mut state = AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: USER.to_owned(),
            device_id: "SYNTHETIC".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        rooms: vec![
            unread_diagnostic_room(ROOM),
            unread_diagnostic_room(OTHER_ROOM),
        ],
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

/// Main pane anchored to `EVENT` through the focused-context lifecycle.
fn anchored_state() -> AppState {
    let mut state = selected_room_state();
    reduce(
        &mut state,
        AppAction::OpenFocusedContext {
            room_id: ROOM.to_owned(),
            event_id: EVENT.to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::EnterAnchoredTimeline {
            room_id: ROOM.to_owned(),
            event_id: EVENT.to_owned(),
        },
    );
    assert!(state.navigation.main_timeline_anchor.is_some());
    state
}

fn accepted_send(submission: &str) -> AppAction {
    AppAction::ComposerSubmissionAccepted {
        submission_id: koushi_state::SubmissionId::new(submission),
        room_id: ROOM.to_owned(),
        transaction_id: format!("txn-{submission}"),
        body: "hello".to_owned(),
    }
}

/// #1060: a released focused timeline is retired through the retained
/// desired-foreground ingress rather than a mailbox `Unsubscribe`, so wait
/// until the admitted demand no longer desires `key`.
async fn expect_focused_released(
    navigation_projection_rx: &mut watch::Receiver<crate::timeline::NavigationProjectionDemand>,
    key: &TimelineKey,
) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            navigation_projection_rx
                .changed()
                .await
                .expect("navigation projection channel");
            if navigation_projection_rx
                .borrow_and_update()
                .focused
                .as_ref()
                != Some(key)
            {
                break;
            }
        }
    })
    .await
    .expect("the focused timeline must be released");
}

async fn wait_for_snapshot(
    snapshot_rx: &mut watch::Receiver<VersionedAppStateSnapshot>,
    predicate: impl Fn(&AppState) -> bool,
) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if predicate(&snapshot_rx.borrow_and_update().state) {
                break;
            }
            snapshot_rx.changed().await.expect("snapshot channel");
        }
    })
    .await
    .expect("expected snapshot state");
}

/// Bounded negative check: the main pane must not become anchored again.
async fn assert_never_reanchors(snapshot_rx: &mut watch::Receiver<VersionedAppStateSnapshot>) {
    let reanchored = tokio::time::timeout(Duration::from_millis(300), async {
        loop {
            if snapshot_rx
                .borrow_and_update()
                .state
                .navigation
                .main_timeline_anchor
                .is_some()
            {
                break;
            }
            if snapshot_rx.changed().await.is_err() {
                future::pending::<()>().await;
            }
        }
    })
    .await;
    assert!(
        reanchored.is_err(),
        "a late navigation completion re-anchored the main pane after an accepted send"
    );
}

fn committed(projection_request_id: RequestId, key: TimelineKey) -> FocusedProjectionCommitted {
    FocusedProjectionCommitted {
        projection_request_id,
        key,
        actor_generation: 1,
        timeline_generation: TimelineGeneration(1),
        item_count: 3,
        target_present: true,
    }
}

#[tokio::test]
async fn accepted_send_from_anchored_history_releases_the_focused_timeline() {
    let data_dir = tempfile::tempdir().expect("runtime data directory");
    let (
        actor,
        _command_tx,
        action_tx,
        _account_rx,
        _event_rx,
        mut snapshot_rx,
        mut navigation_projection_rx,
        _event_navigation_prepared_tx,
        _focused_projection_tx,
    ) = app_actor_event_navigation_fixture(data_dir.path(), anchored_state());
    let actor_task = tokio::spawn(actor.run());

    action_tx
        .send(vec![accepted_send("anchored")])
        .await
        .expect("accepted send action");

    wait_for_snapshot(&mut snapshot_rx, |state| {
        state.navigation.main_timeline_anchor.is_none()
            && state.focused_context == koushi_state::FocusedContextState::Closed
    })
    .await;
    expect_focused_released(&mut navigation_projection_rx, &focused_key(ROOM, EVENT)).await;
    actor_task.abort();
}

#[tokio::test]
async fn room_switch_releases_the_focused_timeline() {
    // #1046: SelectRoom closes the focused context in the reducer; Core must
    // release the focused timeline actor and its room lease too.
    let data_dir = tempfile::tempdir().expect("runtime data directory");
    let (
        mut actor,
        _command_tx,
        action_tx,
        _account_rx,
        _event_rx,
        mut snapshot_rx,
        mut navigation_projection_rx,
        _event_navigation_prepared_tx,
        _focused_projection_tx,
    ) = app_actor_event_navigation_fixture(data_dir.path(), anchored_state());
    actor.pending_select.insert(
        OTHER_ROOM.to_owned(),
        std::collections::VecDeque::from([request(1)]),
    );
    let actor_task = tokio::spawn(actor.run());

    action_tx
        .send(vec![AppAction::SelectRoom {
            room_id: OTHER_ROOM.to_owned(),
        }])
        .await
        .expect("room switch action");

    wait_for_snapshot(&mut snapshot_rx, |state| {
        state.navigation.active_room_id.as_deref() == Some(OTHER_ROOM)
            && state.focused_context == koushi_state::FocusedContextState::Closed
    })
    .await;
    expect_focused_released(&mut navigation_projection_rx, &focused_key(ROOM, EVENT)).await;
    actor_task.abort();
}

#[tokio::test]
async fn room_switch_during_a_loading_date_jump_settles_the_jump_superseded() {
    // Releasing the focused timeline drops the jump's pending navigation, so
    // Core must publish its terminal instead of leaving the waiter to time out.
    let data_dir = tempfile::tempdir().expect("runtime data directory");
    let mut state = selected_room_state();
    reduce(
        &mut state,
        AppAction::OpenFocusedContext {
            room_id: ROOM.to_owned(),
            event_id: EVENT.to_owned(),
        },
    );
    let (
        mut actor,
        _command_tx,
        action_tx,
        _account_rx,
        mut event_rx,
        mut snapshot_rx,
        mut navigation_projection_rx,
        _event_navigation_prepared_tx,
        _focused_projection_tx,
    ) = app_actor_event_navigation_fixture(data_dir.path(), state);
    let jump_request_id = request(7);
    let key = focused_key(ROOM, EVENT);
    actor.pending_focused_navigation = Some(PendingFocusedNavigation {
        projection_request_id: jump_request_id,
        key: key.clone(),
        room_id: ROOM.to_owned(),
        event_id: EVENT.to_owned(),
        allow_live_fallback: true,
        generation: None,
    });
    actor.pending_select.insert(
        OTHER_ROOM.to_owned(),
        std::collections::VecDeque::from([request(8)]),
    );
    let actor_task = tokio::spawn(actor.run());

    action_tx
        .send(vec![AppAction::SelectRoom {
            room_id: OTHER_ROOM.to_owned(),
        }])
        .await
        .expect("room switch action");

    wait_for_snapshot(&mut snapshot_rx, |state| {
        state.navigation.active_room_id.as_deref() == Some(OTHER_ROOM)
    })
    .await;
    expect_focused_released(&mut navigation_projection_rx, &key).await;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if matches!(
                event_rx.recv().await.expect("core event"),
                CoreEvent::IntentLifecycle {
                    request_id,
                    outcome: IntentOutcome::BenignNoOp(IntentNoOpReason::Superseded),
                    ..
                } if request_id == jump_request_id
            ) {
                break;
            }
        }
    })
    .await
    .expect("the dropped date jump must settle Superseded");
    actor_task.abort();
}

#[tokio::test]
async fn accepted_send_cancels_a_loading_date_jump_before_its_projection_lands() {
    // Jump-to-date with a cached target: Core opened the focused timeline and
    // waits for its projection ACK before EnterAnchoredTimeline. The main pane
    // is still live, so the reducer alone cannot see this in-flight intent.
    let data_dir = tempfile::tempdir().expect("runtime data directory");
    let mut state = selected_room_state();
    reduce(
        &mut state,
        AppAction::OpenFocusedContext {
            room_id: ROOM.to_owned(),
            event_id: EVENT.to_owned(),
        },
    );
    let (
        mut actor,
        _command_tx,
        action_tx,
        _account_rx,
        _event_rx,
        mut snapshot_rx,
        mut navigation_projection_rx,
        _event_navigation_prepared_tx,
        focused_projection_tx,
    ) = app_actor_event_navigation_fixture(data_dir.path(), state);
    let projection_request_id = request(2);
    let key = focused_key(ROOM, EVENT);
    actor.pending_focused_navigation = Some(PendingFocusedNavigation {
        projection_request_id,
        key: key.clone(),
        room_id: ROOM.to_owned(),
        event_id: EVENT.to_owned(),
        allow_live_fallback: true,
        generation: None,
    });
    let actor_task = tokio::spawn(actor.run());

    action_tx
        .send(vec![accepted_send("date-jump")])
        .await
        .expect("accepted send action");
    wait_for_snapshot(&mut snapshot_rx, |state| {
        state.timeline.composer.pending_submission_id.is_some()
            && state.focused_context == koushi_state::FocusedContextState::Closed
    })
    .await;
    expect_focused_released(&mut navigation_projection_rx, &key).await;

    focused_projection_tx
        .send(committed(projection_request_id, key))
        .expect("late projection commit");
    assert_never_reanchors(&mut snapshot_rx).await;
    actor_task.abort();
}

#[tokio::test]
async fn accepted_send_fences_a_date_jump_still_awaiting_the_server() {
    // Jump-to-date without a cached target: the account actor later replies
    // with the OpenFocusedContext + EnterAnchoredTimeline pair, which AppActor
    // subscribes only if it reduces it. A send accepted in between must win.
    let data_dir = tempfile::tempdir().expect("runtime data directory");
    let (
        mut actor,
        _command_tx,
        action_tx,
        mut account_rx,
        _event_rx,
        mut snapshot_rx,
        _navigation_projection_rx,
        _event_navigation_prepared_tx,
        _focused_projection_tx,
    ) = app_actor_event_navigation_fixture(data_dir.path(), selected_room_state());
    actor.pending_date_navigation_request_id = Some(request(3));
    let actor_task = tokio::spawn(actor.run());

    action_tx
        .send(vec![accepted_send("date-server")])
        .await
        .expect("accepted send action");
    wait_for_snapshot(&mut snapshot_rx, |state| {
        state.timeline.composer.pending_submission_id.is_some()
    })
    .await;

    action_tx
        .send(vec![
            AppAction::OpenFocusedContext {
                room_id: ROOM.to_owned(),
                event_id: EVENT.to_owned(),
            },
            AppAction::EnterAnchoredTimeline {
                room_id: ROOM.to_owned(),
                event_id: EVENT.to_owned(),
            },
        ])
        .await
        .expect("late date-jump reply");
    assert_never_reanchors(&mut snapshot_rx).await;
    // The fenced reply is never subscribed, so nothing needs releasing.
    while let Ok(message) = account_rx.try_recv() {
        assert!(
            !matches!(
                message,
                AccountMessage::TimelineCommand(
                    koushi_protocol::command::TimelineCommand::Subscribe { .. }
                        | koushi_protocol::command::TimelineCommand::Unsubscribe { .. }
                )
            ),
            "a fenced date-jump reply must not touch timeline subscriptions"
        );
    }
    assert_eq!(
        snapshot_rx.borrow().state.focused_context,
        koushi_state::FocusedContextState::Closed
    );
    actor_task.abort();
}

#[tokio::test]
async fn accepted_send_cancels_an_in_flight_event_navigation() {
    // Search/Activity/Pinned navigation located its target and waits for the
    // focused projection ACK while the main pane is still live.
    let data_dir = tempfile::tempdir().expect("runtime data directory");
    let generation = 1;
    let mut state = selected_room_state();
    reduce(
        &mut state,
        AppAction::EventNavigationStarted {
            source: koushi_state::EventNavigationSource::Pinned,
        },
    );
    reduce(
        &mut state,
        AppAction::OpenFocusedContext {
            room_id: ROOM.to_owned(),
            event_id: EVENT.to_owned(),
        },
    );
    assert!(matches!(
        state.navigation.event_navigation,
        koushi_state::EventNavigationState::Opening { generation: current, .. } if current == generation
    ));
    let (
        mut actor,
        _command_tx,
        action_tx,
        _account_rx,
        _event_rx,
        mut snapshot_rx,
        mut navigation_projection_rx,
        _event_navigation_prepared_tx,
        focused_projection_tx,
    ) = app_actor_event_navigation_fixture(data_dir.path(), state);
    let navigation_request_id = request(4);
    let key = focused_key(ROOM, EVENT);
    actor.pending_event_navigation = Some(PendingEventNavigation {
        request_id: navigation_request_id,
        select_request_id: request(5),
        room_id: ROOM.to_owned(),
        event_id: EVENT.to_owned(),
        source: koushi_state::EventNavigationSource::Pinned,
        generation,
    });
    actor.pending_focused_navigation = Some(PendingFocusedNavigation {
        projection_request_id: navigation_request_id,
        key: key.clone(),
        room_id: ROOM.to_owned(),
        event_id: EVENT.to_owned(),
        allow_live_fallback: false,
        generation: Some(TimelineGeneration(generation)),
    });
    let actor_task = tokio::spawn(actor.run());

    action_tx
        .send(vec![accepted_send("event-navigation")])
        .await
        .expect("accepted send action");
    wait_for_snapshot(&mut snapshot_rx, |state| {
        state.timeline.composer.pending_submission_id.is_some()
            && state.focused_context == koushi_state::FocusedContextState::Closed
            && state.navigation.event_navigation == koushi_state::EventNavigationState::Idle
    })
    .await;
    expect_focused_released(&mut navigation_projection_rx, &key).await;

    focused_projection_tx
        .send(committed(navigation_request_id, key))
        .expect("late projection commit");
    assert_never_reanchors(&mut snapshot_rx).await;
    actor_task.abort();
}

#[tokio::test]
async fn accepted_attachment_send_returns_anchored_pane_to_live_and_releases_focus() {
    // Attachment sends settle through AppCommand::AcceptComposerDraft, whose
    // arm reduces ComposerDraftAccepted and handles effects on the command
    // path (the leased command itself needs a renderer permit, so this drives
    // the same two calls the arm makes).
    let data_dir = tempfile::tempdir().expect("runtime data directory");
    let state = anchored_state();
    let submitted_revision = state.composer_drafts.room_revision(ROOM);
    let (
        mut actor,
        _command_tx,
        _action_tx,
        _account_rx,
        _event_rx,
        _snapshot_rx,
        mut navigation_projection_rx,
        _event_navigation_prepared_tx,
        _focused_projection_tx,
    ) = app_actor_event_navigation_fixture(data_dir.path(), state);

    let effects = actor
        .reduce_app_action(AppAction::ComposerDraftAccepted {
            target: koushi_state::ComposerTarget::Main {
                room_id: ROOM.to_owned(),
            },
            submitted_revision,
            consumes_draft: false,
        })
        .await;
    actor.handle_app_effects(request(6), effects).await;

    assert_eq!(actor.state.navigation.main_timeline_anchor, None);
    assert_eq!(
        actor.state.focused_context,
        koushi_state::FocusedContextState::Closed
    );
    expect_focused_released(&mut navigation_projection_rx, &focused_key(ROOM, EVENT)).await;
}

#[tokio::test]
async fn date_jump_reply_admits_the_focused_owner_before_subscribing_it() {
    // #1060 review: the server-resolved date-jump target must be subscribed by
    // AppActor only after it is the retained desired focused foreground, so no
    // concurrent focused admission can retire it before the pair is reduced.
    let data_dir = tempfile::tempdir().expect("runtime data directory");
    let (
        mut actor,
        _command_tx,
        action_tx,
        mut account_rx,
        _event_rx,
        _snapshot_rx,
        navigation_projection_rx,
        _event_navigation_prepared_tx,
        _focused_projection_tx,
    ) = app_actor_event_navigation_fixture(data_dir.path(), selected_room_state());
    actor.pending_date_navigation_request_id = Some(request(3));
    let actor_task = tokio::spawn(actor.run());

    action_tx
        .send(vec![
            AppAction::OpenFocusedContext {
                room_id: ROOM.to_owned(),
                event_id: EVENT.to_owned(),
            },
            AppAction::EnterAnchoredTimeline {
                room_id: ROOM.to_owned(),
                event_id: EVENT.to_owned(),
            },
        ])
        .await
        .expect("date-jump reply");
    let key = focused_key(ROOM, EVENT);
    let subscribe_request_id = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let AccountMessage::TimelineCommand(
                koushi_protocol::command::TimelineCommand::Subscribe {
                    request_id,
                    key: subscribed,
                    ..
                },
            ) = account_rx.recv().await.expect("account actor channel")
                && subscribed == key
            {
                break request_id;
            }
        }
    })
    .await
    .expect("AppActor must subscribe the date-jump focused timeline");
    assert_eq!(
        subscribe_request_id,
        request(3),
        "the subscription keeps the date jump's projection correlation"
    );
    assert_eq!(
        navigation_projection_rx.borrow().focused.as_ref(),
        Some(&key),
        "the focused owner is admitted as desired before its Subscribe"
    );
    actor_task.abort();
}
