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
use koushi_state::{AppAction, AvatarThumbnailState, RoomSummary, SessionState, UserProfile};

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
        match tokio::time::timeout(Duration::from_millis(5), conn.recv_event()).await {
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

async fn recv_intent_lifecycle_within(
    conn: &mut koushi_core::runtime::CoreConnection,
    request_id: koushi_core::ids::RequestId,
    timeout: Duration,
) -> Option<koushi_core::event::IntentOutcome> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match tokio::time::timeout(remaining.min(Duration::from_millis(20)), conn.recv_event())
            .await
        {
            Ok(Ok(CoreEvent::IntentLifecycle {
                request_id: rid,
                outcome,
            })) if rid == request_id => {
                return Some(outcome);
            }
            Ok(Ok(_)) | Ok(Err(_)) => {}
            Err(_) => return None,
        }
    }
}

fn background_flood_batch(batch_index: usize, kept_room_ids: &[&str]) -> Vec<AppAction> {
    let room_id = format!("!background-room-{batch_index}:example.test");
    let user_id = format!("@background-user-{batch_index}:example.test");
    let mxc_uri = format!("mxc://example.test/avatar/{batch_index}");
    let timestamp_ms = 1_720_000_000_000 + batch_index as u64;
    let mut rooms = kept_room_ids
        .iter()
        .map(|room_id| RoomSummary {
            room_id: (*room_id).to_owned(),
            display_name: (*room_id).to_owned(),
            display_label: (*room_id).to_owned(),
            original_display_label: (*room_id).to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: Default::default(),
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            marked_unread: false,
            last_activity_ms: timestamp_ms,
            parent_space_ids: vec![],
            dm_space_ids: vec![],
            is_encrypted: false,
            joined_members: 1,
        })
        .collect::<Vec<_>>();
    rooms.push(RoomSummary {
        room_id: room_id.clone(),
        display_name: format!("Background Room {batch_index}"),
        display_label: format!("Background Room {batch_index}"),
        original_display_label: format!("Background Room {batch_index}"),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: Default::default(),
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        marked_unread: false,
        last_activity_ms: timestamp_ms,
        parent_space_ids: vec![],
        dm_space_ids: vec![],
        is_encrypted: false,
        joined_members: 1000 + batch_index as u64,
    });
    vec![
        AppAction::RoomListUpdated {
            spaces: vec![],
            rooms,
        },
        AppAction::HistoryCrawlProgress {
            room_id: room_id.clone(),
            processed: 10 + batch_index as u64,
            indexed: 5 + batch_index as u64,
            timestamp_ms,
        },
        AppAction::UserProfilesUpdated {
            profiles: vec![UserProfile {
                user_id,
                display_name: Some(format!("Background User {batch_index}")),
                display_label: format!("Background User {batch_index}"),
                original_display_label: format!("Background User {batch_index}"),
                mention_search_terms: vec![format!("background-user-{batch_index}")],
                avatar: None,
            }],
        },
        AppAction::AvatarThumbnailUpdated {
            mxc_uri,
            thumbnail: AvatarThumbnailState::Ready {
                source_url: format!("koushi-thumbnail://localhost/avatar/{batch_index}"),
                width: Some(32),
                height: Some(32),
                mime_type: Some("image/png".to_owned()),
            },
        },
    ]
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
        matches!(state.session, SessionState::Ready(_)) && state.navigation.active_room_id.is_some()
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

#[test]
fn select_room_routing_is_reliable_and_correlated() {
    let runtime_source = include_str!("../src/runtime.rs");
    let command_source = include_str!("../src/command.rs");

    assert!(
        runtime_source.contains("User-intent lane: for SelectRoom, record the request_id→room_id")
            && runtime_source.contains("terminal IntentLifecycle outcome"),
        "runtime must keep the SelectRoom correlation comment next to the reliable command path"
    );
    assert!(
        runtime_source.contains("AccountMessage::RoomCommand(room_command)")
            && runtime_source.contains(".await;"),
        "SelectRoom must continue to route through the awaited command path"
    );
    assert!(
        !runtime_source.contains("try_send(crate::account::AccountMessage::RoomCommand"),
        "SelectRoom must not be routed through a drop-on-full command path"
    );
    assert!(
        command_source.contains("User-intent lane: room selection is request-id correlated"),
        "RoomCommand::SelectRoom should carry an explicit user-intent lane comment"
    );
}

/// A real SelectRoom command must still commit under a flood of reducer-side
/// background work. This exercises the live command path while background
/// room-list, crawl-progress, profile, and avatar updates are already queued.
#[tokio::test]
async fn select_room_commits_within_one_second_during_background_action_flood() {
    use koushi_core::event::IntentOutcome;

    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();

    let primary_room = "!active-room:example.test";
    let target_room = "!target-room:example.test";
    let spare_room = "!spare-room:example.test";

    runtime
        .inject_actions(vec![
            AppAction::RestoreSessionSucceeded(session_info()),
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![
                    room_summary(primary_room),
                    room_summary(target_room),
                    room_summary(spare_room),
                ],
            },
        ])
        .await;

    let ready = wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_)) && state.navigation.active_room_id.is_some()
    })
    .await;
    let target = [primary_room, target_room, spare_room]
        .into_iter()
        .find(|room_id| Some(*room_id) != ready.navigation.active_room_id.as_deref())
        .expect("a non-active room should be available");

    let flood_runtime = runtime;
    let flood = tokio::spawn(async move {
        for batch_index in 0..256 {
            flood_runtime
                .inject_actions(background_flood_batch(
                    batch_index,
                    &[primary_room, target_room, spare_room],
                ))
                .await;
            if batch_index % 8 == 0 {
                tokio::task::yield_now().await;
            }
        }
    });

    tokio::time::sleep(Duration::from_millis(10)).await;

    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::SelectRoom {
        request_id,
        room_id: target.to_owned(),
    }))
    .await
    .expect("command should submit");

    let outcome = tokio::time::timeout(
        Duration::from_secs(1),
        recv_intent_lifecycle_within(&mut conn, request_id, Duration::from_secs(1)),
    )
    .await
    .expect("SelectRoom should emit a lifecycle event within one second")
    .expect("SelectRoom should emit a correlated lifecycle outcome");

    assert_eq!(outcome, IntentOutcome::Committed, "SelectRoom must commit");
    assert!(
        conn.snapshot().navigation.active_room_id.is_some(),
        "room selection should leave an active room in state"
    );

    flood.abort();
    let _ = flood.await;
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
            Ok(Ok(CoreEvent::IntentLifecycle {
                request_id,
                outcome,
            })) => {
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
