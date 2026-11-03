//! The shared state fence (#1759) must fail on silence, not on elapsed time, and
//! its failure must say enough to tell a wedged Core from a slow machine.

#[path = "../support/mod.rs"]
#[expect(
    clippy::duplicate_mod,
    reason = "suite aggregation preserves feature-local fixture scopes and registered helper tests"
)]
mod support;

use std::panic::AssertUnwindSafe;
use std::time::{Duration, Instant};

use futures_util::FutureExt;
use koushi_core_testkit::StateWaitWatchdog;
use koushi_state::AppAction;

/// A quiet Core that never reaches the predicate must still fail the wait, and
/// the failure must name silence as the cause together with the state it was
/// stuck on. The budget is shortened here so the check does not wait out the
/// shipped 30 s window.
#[tokio::test]
async fn a_state_predicate_that_never_holds_fails_the_wait_with_a_wedge_diagnostic() {
    let (runtime, _stores) = koushi_core::CoreRuntime::start_isolated();
    let mut connection = runtime.attach();
    let watchdog = StateWaitWatchdog::start_at(
        Instant::now(),
        Duration::from_millis(50),
        Duration::from_millis(200),
    );

    let panic = AssertUnwindSafe(support::wait_for_state_event_within(
        &mut connection,
        watchdog,
        |_| false,
    ))
    .catch_unwind()
    .await
    .expect_err("an unreachable predicate must never satisfy the wait");
    let message = panic_message(&panic);

    assert!(
        message.contains("state predicate was not satisfied: silence for"),
        "the failure must be silence, not elapsed time: {message}"
    );
    assert!(
        message.contains("state events in"),
        "diagnostic must report whether any event arrived and how long the wait lasted: {message}"
    );
    assert!(
        message.contains("window 50ms, backstop 200ms"),
        "diagnostic must name the budgets it was given: {message}"
    );
    assert!(
        message.contains("generation") && message.contains("session"),
        "diagnostic must report the last observed state: {message}"
    );

    drop(connection);
    runtime.shutdown().await;
}

fn panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|text| (*text).to_owned()))
        .expect("panic payload is a message")
}

/// Arriving state events re-arm the window, so a wait outlives the elapsed time
/// its window allows and only the asserted state ends it. Without that re-arm
/// this waits the 500 ms window out and fails.
#[tokio::test]
async fn arriving_state_events_keep_a_wait_alive_past_its_window() {
    const ROOM_ID: &str = "!fence-window:example.invalid";
    const STEPS: usize = 40;

    let (runtime, mut connection, _ready, _data_dir, _credential_dir) =
        support::ready_room_conn(ROOM_ID).await;
    let watchdog = StateWaitWatchdog::start_at(
        Instant::now(),
        Duration::from_millis(500),
        Duration::from_secs(30),
    );

    let (state, ()) = tokio::join!(
        support::wait_for_state_event_within(&mut connection, watchdog, |state| {
            state.rooms.len() == STEPS
        }),
        async {
            for step in 2..=STEPS {
                tokio::time::sleep(Duration::from_millis(25)).await;
                runtime
                    .inject_actions(vec![AppAction::RoomListUpdated {
                        spaces: Vec::new(),
                        rooms: (0..step)
                            .map(|index| {
                                support::room_summary(&format!("!fence-{index}:example.invalid"))
                            })
                            .collect(),
                    }])
                    .await;
            }
        },
    );

    assert_eq!(state.rooms.len(), STEPS);
    drop(connection);
    runtime.shutdown().await;
}
