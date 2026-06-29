//! Scale reproduction harness for issue #116 (room selection times out on
//! large real accounts). These tests drive the REAL headless `CoreRuntime`
//! (AppActor + AccountActor + RoomActor channel topology, no Matrix SDK) and
//! flood it with a real-account-shaped room list (5 spaces / ~110 rooms /
//! 57 DMs), then exercise the click path.
//!
//! They are private-data-free: all ids are synthetic `example.test` ids.
//!
//! Goal: convert the three competing #116 hypotheses into deterministic
//! pass/fail at the runtime/reducer layer, BELOW the Matrix SDK:
//!   - H-A: `handle_select_room` no-ops because the clicked room is missing
//!     from `state.rooms` at reduce time (room-list projection divergence).
//!   - H-B: navigation-restore overwrites the click within the action batch.
//!   - H-storm: a background `RoomListUpdated` storm starves/reverts the click.
//!
//! Harness note: `support::wait_for_state` PANICS after ~1s if its predicate is
//! never satisfied, so wrapping it in `executor::timeout` does NOT prevent the
//! inner panic — the panic fires first and fails the test. Therefore:
//!   - success-expected scenarios call `wait_for_state` DIRECTLY (a panic is a
//!     legitimate reproduction of the #116 timeout at the runtime layer);
//!   - non-satisfaction scenarios use the local non-panicking `poll_for_state`.

use std::time::Duration;

use koushi_core::executor;
use koushi_core::runtime::{CoreConnection, CoreRuntime};
use koushi_state::{AppAction, AppState, RoomSummary, SessionState, SpaceSummary};

mod support;
use support::*;

const SPACES: usize = 5;
const NON_DM_ROOMS: usize = 110;
const DMS: usize = 57;

fn space_id(i: usize) -> String {
    format!("!space-{i}:example.test")
}
fn room_id(i: usize) -> String {
    format!("!room-{i}:example.test")
}
fn dm_id(i: usize) -> String {
    format!("!dm-{i}:example.test")
}

/// A non-DM room, assigned round-robin into one of the spaces.
fn scale_room(i: usize) -> RoomSummary {
    RoomSummary {
        parent_space_ids: vec![space_id(i % SPACES)],
        ..room_summary(&room_id(i))
    }
}

fn scale_dm(i: usize) -> RoomSummary {
    RoomSummary {
        is_dm: true,
        dm_user_ids: vec![format!("@dm-user-{i}:example.test")],
        ..room_summary(&dm_id(i))
    }
}

fn scale_spaces() -> Vec<SpaceSummary> {
    (0..SPACES)
        .map(|s| {
            let children: Vec<String> = (0..NON_DM_ROOMS)
                .filter(|i| i % SPACES == s)
                .map(room_id)
                .collect();
            SpaceSummary {
                space_id: space_id(s),
                display_name: format!("Space {s}"),
                avatar: None,
                child_room_ids: children,
            }
        })
        .collect()
}

/// Real-account-shaped room list: spaces + non-DM rooms + DMs.
fn scale_rooms() -> Vec<RoomSummary> {
    let mut rooms: Vec<RoomSummary> = (0..NON_DM_ROOMS).map(scale_room).collect();
    rooms.extend((0..DMS).map(scale_dm));
    rooms
}

fn room_list_updated() -> AppAction {
    AppAction::RoomListUpdated {
        spaces: scale_spaces(),
        rooms: scale_rooms(),
    }
}

/// Non-panicking counterpart to `support::wait_for_state`. Polls the latest
/// snapshot up to `attempts` times (5ms apart) and returns `Some(state)` on the
/// first satisfying snapshot, or `None` on timeout. Used by scenarios that
/// expect the predicate to STAY unsatisfied, so a timeout is the expected
/// (asserted) outcome rather than a panic.
async fn poll_for_state<F>(
    connection: &mut CoreConnection,
    predicate: F,
    attempts: usize,
) -> Option<AppState>
where
    F: Fn(&AppState) -> bool,
{
    for _ in 0..attempts {
        let snapshot = connection.snapshot();
        if predicate(&snapshot) {
            return Some(snapshot);
        }
        executor::sleep(Duration::from_millis(5)).await;
    }
    None
}

/// H-baseline (THE decisive test): on a real-account-shaped list, clicking a
/// room that IS present in `state.rooms`, deep in the list and different from
/// the auto-selected startup room, MUST land the active room on the clicked
/// room. `wait_for_state` is called directly: if it panics ("state predicate
/// was not satisfied"), the #116 timeout reproduces purely in the
/// runtime/reducer and the Matrix SDK is exonerated.
#[tokio::test]
async fn select_room_deep_in_large_account_lands_on_clicked_room() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();

    runtime
        .inject_actions(vec![
            AppAction::RestoreSessionSucceeded(session_info()),
            room_list_updated(),
        ])
        .await;

    // The reducer auto-selects a startup room when none is active.
    let startup = wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_)) && state.navigation.active_room_id.is_some()
    })
    .await;
    let startup_room = startup.navigation.active_room_id.clone().unwrap();

    // Click a non-DM room deep in the list, guaranteed present and different
    // from the startup room.
    let target = room_id(NON_DM_ROOMS - 7);
    assert_ne!(target, startup_room, "target must differ from startup room");
    assert!(
        startup.rooms.iter().any(|r| r.room_id == target),
        "precondition: clicked room is present in state.rooms"
    );

    // Separate batch, mirroring a reducer-side selection projection.
    runtime
        .inject_actions(vec![AppAction::SelectRoom {
            room_id: target.clone(),
        }])
        .await;

    // Direct call: a panic here IS the #116 reproduction.
    let landed = wait_for_state(&mut conn, |state| {
        state.navigation.active_room_id.as_deref() == Some(target.as_str())
    })
    .await;
    assert_eq!(
        landed.navigation.active_room_id.as_deref(),
        Some(target.as_str()),
        "clicking a present room deep in a {NON_DM_ROOMS}-room/{DMS}-DM account must land it"
    );
}

/// H-A: model the SyncService transient-drop hypothesis — the user sees room X
/// (it was in an earlier snapshot), but a later `RoomListUpdated` replaced
/// `state.rooms` with a vec MISSING room X. The click then no-ops.
///
/// This test documents the CURRENT (buggy, information-swallowing) behavior:
/// the click for a room absent from the LATEST `state.rooms` produces NO
/// observable outcome — `active_room_id` is unchanged and nothing signals the
/// no-op. After the #116 fix this should become a correlated, observable
/// failure rather than a silent timeout. Uses the non-panicking poll because
/// non-satisfaction is the asserted outcome.
#[tokio::test]
async fn select_room_missing_from_state_rooms_is_a_silent_noop_today() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();

    // Snapshot N: full list including the room the user will click.
    runtime
        .inject_actions(vec![
            AppAction::RestoreSessionSucceeded(session_info()),
            room_list_updated(),
        ])
        .await;
    let startup = wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_)) && state.navigation.active_room_id.is_some()
    })
    .await;
    let startup_room = startup.navigation.active_room_id.clone().unwrap();

    let clicked = room_id(NON_DM_ROOMS - 3);
    assert_ne!(clicked, startup_room);

    // Snapshot N+1: a transiently incomplete re-projection that drops `clicked`
    // from state.rooms (everything else present).
    let incomplete_rooms: Vec<RoomSummary> = scale_rooms()
        .into_iter()
        .filter(|r| r.room_id != clicked)
        .collect();
    runtime
        .inject_actions(vec![AppAction::RoomListUpdated {
            spaces: scale_spaces(),
            rooms: incomplete_rooms,
        }])
        .await;

    // The click for a room missing from the LATEST state.rooms.
    runtime
        .inject_actions(vec![AppAction::SelectRoom {
            room_id: clicked.clone(),
        }])
        .await;

    // Today: the click silently no-ops. active_room_id never reaches `clicked`.
    let landed = poll_for_state(
        &mut conn,
        |state| state.navigation.active_room_id.as_deref() == Some(clicked.as_str()),
        80,
    )
    .await;
    assert!(
        landed.is_none(),
        "regression sentinel: if this now lands, the missing-room no-op path changed — \
         re-evaluate the #116 observable-failure contract"
    );
}

/// H-storm: a background `RoomListUpdated` re-projection storm interleaved with
/// the click MUST NOT starve or revert it. Confirms the storm alone is not the
/// cause and guards the reliable-delivery / coalescing contract. Success is
/// expected, so `wait_for_state` is called directly (panic = reproduction).
#[tokio::test]
async fn select_room_survives_background_room_list_storm() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();

    runtime
        .inject_actions(vec![
            AppAction::RestoreSessionSucceeded(session_info()),
            room_list_updated(),
        ])
        .await;
    wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_)) && state.navigation.active_room_id.is_some()
    })
    .await;

    let target = dm_id(DMS - 1);

    // Storm of full re-projections, with the click in the middle.
    for n in 0..40 {
        if n == 20 {
            runtime
                .inject_actions(vec![AppAction::SelectRoom {
                    room_id: target.clone(),
                }])
                .await;
        }
        runtime.inject_actions(vec![room_list_updated()]).await;
    }

    // Direct call: a panic here IS a reproduction (click lost under storm).
    let landed = wait_for_state(&mut conn, |state| {
        state.navigation.active_room_id.as_deref() == Some(target.as_str())
    })
    .await;
    assert_eq!(
        landed.navigation.active_room_id.as_deref(),
        Some(target.as_str()),
        "click was lost/reverted under a background room-list storm"
    );
}
