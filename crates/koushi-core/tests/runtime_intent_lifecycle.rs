//! Invariant test for SelectRoom intent lifecycle (issue #116, §4.7 Slice 1).
//!
//! Drives the REAL command path (`conn.command(CoreCommand::Room(...))`, not
//! `inject_actions`) and asserts that every submitted SelectRoom reaches a
//! correlated, observable terminal `IntentLifecycle` outcome — no silent
//! vanishing.
//!
//! Private-data-free: all ids are synthetic `example.test` ids. No room ids,
//! user ids, event ids, or message bodies appear in test output.

use std::time::Duration;

use koushi_core::{
    command::{CoreCommand, RoomCommand},
    event::CoreEvent,
    runtime::CoreRuntime,
};
use koushi_state::{AppAction, SessionState};

mod support;
use support::*;

/// Drain events from `conn`, returning the first `IntentLifecycle` whose
/// `request_id` matches the one we submitted. Times out after `attempts *
/// 5ms`.
async fn recv_intent_lifecycle_for(
    conn: &mut koushi_core::runtime::CoreConnection,
    request_id: koushi_core::ids::RequestId,
    attempts: usize,
) -> Option<koushi_core::event::IntentOutcome> {
    for _ in 0..attempts {
        // Poll for a pending event with a short timeout.
        match tokio::time::timeout(
            Duration::from_millis(5),
            conn.recv_event(),
        )
        .await
        {
            Ok(Ok(CoreEvent::IntentLifecycle {
                request_id: rid,
                outcome,
            })) if rid == request_id => {
                return Some(outcome);
            }
            Ok(Ok(_)) | Err(_) => {
                // Keep draining or polling.
                koushi_core::executor::sleep(Duration::from_millis(1)).await;
            }
            Ok(Err(_)) => break, // stream lagged / closed
        }
    }
    None
}

/// A SelectRoom command for a room that IS present in `state.rooms` must
/// emit `IntentLifecycle { outcome: Committed }` for the matching request_id.
#[tokio::test]
async fn select_room_present_emits_committed() {
    use koushi_core::event::IntentOutcome;

    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();

    let room_a = "!room-a:example.test";
    let room_b = "!room-b:example.test";

    runtime
        .inject_actions(vec![
            AppAction::RestoreSessionSucceeded(session_info()),
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![room_summary(room_a), room_summary(room_b)],
            },
        ])
        .await;

    // Wait for Ready and an auto-selected room (so room_b is NOT already active).
    wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_))
            && state.navigation.active_room_id.is_some()
    })
    .await;

    // Submit SelectRoom for room_b through the REAL command path.
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::SelectRoom {
        request_id,
        room_id: room_b.to_owned(),
    }))
    .await
    .expect("command should submit");

    let outcome = recv_intent_lifecycle_for(&mut conn, request_id, 200).await;
    assert!(
        outcome.is_some(),
        "IntentLifecycle event must be emitted for a SelectRoom targeting a present room"
    );
    assert_eq!(
        outcome.unwrap(),
        IntentOutcome::Committed,
        "present room must emit Committed"
    );
}

/// A SelectRoom command for a room that is permanently absent from `state.rooms`
/// must immediately emit `IntentLifecycle { outcome: FailedNoOp(RoomNotInState) }`
/// for the matching request_id. This is the primary Slice 1 invariant.
#[tokio::test]
async fn select_room_missing_from_state_emits_failed_noop_room_not_in_state() {
    use koushi_core::event::{IntentNoOpReason, IntentOutcome};

    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();

    // Seed a Ready session with a known room list that does NOT contain the
    // target room. Use inject_actions so we control the exact room set.
    let known_room = "!known-room:example.test";
    let absent_room = "!absent-room:example.test";

    runtime
        .inject_actions(vec![
            AppAction::RestoreSessionSucceeded(session_info()),
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![room_summary(known_room)],
            },
        ])
        .await;

    // Wait for a Ready session so `session_ready` will be true.
    wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_))
    })
    .await;

    // Submit SelectRoom for the absent room through the REAL command path.
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::SelectRoom {
        request_id,
        room_id: absent_room.to_owned(),
    }))
    .await
    .expect("command should submit");

    // Expect FailedNoOp(RoomNotInState) immediately for this request_id.
    let outcome = recv_intent_lifecycle_for(&mut conn, request_id, 200).await;
    assert!(
        outcome.is_some(),
        "IntentLifecycle event must be emitted for a SelectRoom targeting a permanently absent room"
    );
    assert_eq!(
        outcome.unwrap(),
        IntentOutcome::FailedNoOp(IntentNoOpReason::RoomNotInState),
        "permanently absent room must emit FailedNoOp(RoomNotInState) immediately"
    );
}

/// Two concurrent SelectRoom commands for the SAME absent room must both
/// receive a terminal `IntentLifecycle` outcome immediately. Regresses the P1
/// bug where a second `insert` would overwrite the first `request_id`, causing
/// the first command to never get a correlated outcome and silently time out.
#[tokio::test]
async fn two_concurrent_select_room_for_same_room_both_receive_terminal_outcome() {
    use koushi_core::event::{IntentNoOpReason, IntentOutcome};
    use std::collections::HashMap;

    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();

    let known_room = "!known:example.test";
    let absent_room = "!absent:example.test";

    runtime
        .inject_actions(vec![
            AppAction::RestoreSessionSucceeded(session_info()),
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![room_summary(known_room)],
            },
        ])
        .await;

    wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_))
    })
    .await;

    // Submit TWO SelectRoom commands for the same absent room.
    let request_id_a = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::SelectRoom {
        request_id: request_id_a,
        room_id: absent_room.to_owned(),
    }))
    .await
    .expect("first command should submit");

    let request_id_b = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::SelectRoom {
        request_id: request_id_b,
        room_id: absent_room.to_owned(),
    }))
    .await
    .expect("second command should submit");

    // Collect IntentLifecycle events until both request_ids are observed or
    // attempts are exhausted. Both must arrive immediately — no rounds needed.
    let mut outcomes: HashMap<koushi_core::ids::RequestId, IntentOutcome> = HashMap::new();
    for _ in 0..400 {
        match tokio::time::timeout(Duration::from_millis(5), conn.recv_event()).await {
            Ok(Ok(CoreEvent::IntentLifecycle { request_id, outcome })) => {
                outcomes.insert(request_id, outcome);
                if outcomes.contains_key(&request_id_a) && outcomes.contains_key(&request_id_b) {
                    break;
                }
            }
            Ok(Ok(_)) | Err(_) => {
                koushi_core::executor::sleep(Duration::from_millis(1)).await;
            }
            Ok(Err(_)) => break,
        }
    }

    assert!(
        outcomes.contains_key(&request_id_a),
        "first SelectRoom request must receive a terminal IntentLifecycle outcome"
    );
    assert!(
        outcomes.contains_key(&request_id_b),
        "second SelectRoom request must receive a terminal IntentLifecycle outcome"
    );
    assert_eq!(
        outcomes[&request_id_a],
        IntentOutcome::FailedNoOp(IntentNoOpReason::RoomNotInState),
        "first request: absent room must emit FailedNoOp(RoomNotInState) immediately"
    );
    assert_eq!(
        outcomes[&request_id_b],
        IntentOutcome::FailedNoOp(IntentNoOpReason::RoomNotInState),
        "second request: absent room must emit FailedNoOp(RoomNotInState) immediately"
    );
}
