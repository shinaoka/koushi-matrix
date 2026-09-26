//! #1009: AccountActor timer, trust-recheck backoff, and own-identity query
//! coordination, checked with a virtual clock and fixture-server call counts
//! rather than reducer effect counts.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use koushi_state::{AppAction, SESSION_STATUS_FRESHNESS_MS};
use tokio::sync::mpsc;

use super::{SessionCheckClock, SessionCheckCoordinator, sleep_until_due};
use crate::account::actor::{AccountActorHandle, AccountMessage};
use crate::account::test_support::{
    KeyQueryControl, acknowledge_next_verified_projection,
    consume_initial_unknown_trust_projection, login_gated_actor_at, shutdown_and_ack,
    spawn_actor_with_dirs, spawn_named_quarantine_password_server_with_controls,
};
use crate::executor;

const HOUR: Duration = Duration::from_secs(60 * 60);

fn virtual_clock(base_epoch_ms: u64) -> SessionCheckClock {
    SessionCheckClock::Virtual {
        base_epoch_ms,
        origin: executor::Instant::now(),
    }
}

#[tokio::test(start_paused = true)]
async fn due_timer_fires_once_at_the_virtual_due_time_and_is_replaced_by_a_new_arm() {
    let cred_dir = tempfile::tempdir().expect("tempdir");
    let data_dir = tempfile::tempdir().expect("tempdir");
    let (handle, mut action_rx, _events) = spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    let base = 1_000_000;
    assert!(
        handle
            .send(AccountMessage::ConfigureSessionCheckClock {
                base_epoch_ms: base,
            })
            .await
    );
    let due = base + SESSION_STATUS_FRESHNESS_MS;
    handle
        .send(AccountMessage::ArmCurrentSessionStatusCheck {
            token: 3,
            due_at_ms: due,
        })
        .await;
    // A newer arm replaces the first timer; only its token may fire.
    handle
        .send(AccountMessage::ArmCurrentSessionStatusCheck {
            token: 4,
            due_at_ms: due,
        })
        .await;

    tokio::time::sleep(6 * HOUR - Duration::from_secs(1)).await;
    assert!(
        action_rx.try_recv().is_err(),
        "the timer must not fire before the due time"
    );

    let actions = tokio::time::timeout(Duration::from_secs(120), action_rx.recv())
        .await
        .expect("the timer fires at the due time")
        .expect("action channel");
    let [AppAction::CurrentSessionStatusCheckDue { token, now_ms }] = actions.as_slice() else {
        panic!("unexpected actions {actions:?}");
    };
    assert_eq!(*token, 4);
    assert!(*now_ms >= due);

    tokio::time::sleep(12 * HOUR).await;
    assert!(
        action_rx.try_recv().is_err(),
        "a replaced or fired timer must not fire again"
    );
    shutdown_and_ack(&handle).await;
}

#[tokio::test(start_paused = true)]
async fn a_due_time_far_ahead_of_the_clock_fires_within_one_period() {
    // A wall clock that regressed after the last check puts the due time far
    // in the future; the monotonic bound still wakes the reducer within 6 h.
    let clock = virtual_clock(0);
    let started = executor::Instant::now();
    sleep_until_due(&clock, 30 * 60 * 60 * 1_000).await;
    let elapsed = started.elapsed();
    assert!(elapsed <= 6 * HOUR + Duration::from_secs(1), "{elapsed:?}");
    assert!(elapsed >= 6 * HOUR - Duration::from_secs(1), "{elapsed:?}");
}

#[test]
fn trust_backoff_uses_the_shared_capped_backoff_and_ignores_a_regressed_clock() {
    let mut coordinator = SessionCheckCoordinator::default();
    assert_eq!(coordinator.trust_backoff_due_at_ms(10), None);
    coordinator.trust_failures = 1;
    coordinator.trust_failed_at_ms = 1_000;
    assert_eq!(coordinator.trust_backoff_due_at_ms(1_001), Some(61_000));
    assert_eq!(coordinator.trust_backoff_due_at_ms(61_000), None);
    assert_eq!(
        coordinator.trust_backoff_due_at_ms(999),
        None,
        "a clock behind the failure must not hold rechecks"
    );
    coordinator.trust_failures = 40;
    assert_eq!(
        coordinator.trust_backoff_due_at_ms(1_001),
        Some(1_000 + 30 * 60 * 1_000),
        "no retry ceiling: the backoff caps at 30 min"
    );
}

async fn promoted_actor() -> (
    AccountActorHandle,
    mpsc::Receiver<Vec<AppAction>>,
    Arc<KeyQueryControl>,
) {
    let control = Arc::new(KeyQueryControl::default());
    let homeserver = spawn_named_quarantine_password_server_with_controls(
        "@fixture-user:example.invalid",
        "FIXTUREDEVICE",
        None,
        Some(Arc::clone(&control)),
        Arc::new(AtomicBool::new(true)),
    );
    let (handle, mut action_rx) = login_gated_actor_at(homeserver).await;
    consume_initial_unknown_trust_projection(&mut action_rx).await;
    handle
        .send(AccountMessage::CurrentDeviceTrustChanged {
            generation: 2,
            trust: koushi_state::CurrentDeviceTrustState::Verified,
        })
        .await;
    acknowledge_next_verified_projection(&handle, &mut action_rx).await;
    (handle, action_rx, control)
}

/// Wait until the fixture's own-identity query count stops moving.
async fn settled_query_count(control: &KeyQueryControl) -> usize {
    let mut last = control.count.load(Ordering::SeqCst);
    loop {
        executor::sleep(Duration::from_millis(200)).await;
        let now = control.count.load(Ordering::SeqCst);
        if now == last {
            return now;
        }
        last = now;
    }
}

async fn wait_for_query_count(control: &KeyQueryControl, expected: usize) {
    executor::timeout(Duration::from_secs(5), async {
        while control.count.load(Ordering::SeqCst) < expected {
            executor::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("own-identity query count");
}

fn drain(action_rx: &mut mpsc::Receiver<Vec<AppAction>>) {
    while action_rx.try_recv().is_ok() {}
}

#[tokio::test]
async fn trust_recheck_during_an_inspection_joins_its_own_identity_query() {
    let (handle, mut action_rx, control) = promoted_actor().await;
    let baseline = settled_query_count(&control).await;
    let devices_baseline = control.devices_count.load(Ordering::SeqCst);
    drain(&mut action_rx);

    control.hold.store(true, Ordering::SeqCst);
    handle
        .send(AccountMessage::RefreshCurrentSessionStatus {
            request_id: 77,
            trigger: koushi_state::SessionStatusRefreshTrigger::Manual,
            sync_state: koushi_state::CurrentSessionSyncState::Running,
        })
        .await;
    wait_for_query_count(&control, baseline + 1).await;

    handle.send(AccountMessage::CheckCurrentDeviceTrust).await;
    executor::sleep(Duration::from_millis(150)).await;
    control.hold.store(false, Ordering::SeqCst);
    executor::sleep(Duration::from_millis(500)).await;

    assert_eq!(
        control.devices_count.load(Ordering::SeqCst),
        devices_baseline + 1
    );
    assert_eq!(
        control.count.load(Ordering::SeqCst),
        baseline + 1,
        "the trust recheck must join the inspection's own-identity query"
    );
    shutdown_and_ack(&handle).await;
}

#[tokio::test]
async fn inspection_during_a_trust_recheck_reuses_the_fetched_identity() {
    let (handle, mut action_rx, control) = promoted_actor().await;
    let baseline = settled_query_count(&control).await;
    let devices_baseline = control.devices_count.load(Ordering::SeqCst);
    drain(&mut action_rx);

    control.hold.store(true, Ordering::SeqCst);
    handle.send(AccountMessage::CheckCurrentDeviceTrust).await;
    wait_for_query_count(&control, baseline + 1).await;

    handle
        .send(AccountMessage::RefreshCurrentSessionStatus {
            request_id: 78,
            trigger: koushi_state::SessionStatusRefreshTrigger::Manual,
            sync_state: koushi_state::CurrentSessionSyncState::Running,
        })
        .await;
    executor::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        control.devices_count.load(Ordering::SeqCst),
        devices_baseline,
        "the inspection waits for the in-flight recheck"
    );
    control.hold.store(false, Ordering::SeqCst);

    executor::timeout(Duration::from_secs(5), async {
        while control.devices_count.load(Ordering::SeqCst) == devices_baseline {
            executor::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the waiting inspection starts after the recheck settles");
    executor::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        control.count.load(Ordering::SeqCst),
        baseline + 1,
        "the inspection must read the identity the recheck just fetched"
    );
    shutdown_and_ack(&handle).await;
}

#[tokio::test]
async fn a_failed_trust_recheck_defers_the_next_request_behind_the_backoff() {
    let (handle, mut action_rx, control) = promoted_actor().await;
    let baseline = settled_query_count(&control).await;
    drain(&mut action_rx);

    control.fail.store(true, Ordering::SeqCst);
    handle.send(AccountMessage::CheckCurrentDeviceTrust).await;
    wait_for_query_count(&control, baseline + 1).await;
    executor::sleep(Duration::from_millis(300)).await;

    for _ in 0..5 {
        handle.send(AccountMessage::CheckCurrentDeviceTrust).await;
    }
    executor::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        control.count.load(Ordering::SeqCst),
        baseline + 1,
        "requests inside the failure backoff must not query again"
    );
    shutdown_and_ack(&handle).await;
}
