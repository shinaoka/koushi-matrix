//! Runtime core tests: request-id correlation, local rejection, batching, and
//! slow-consumer recovery.

use std::time::Duration;

use koushi_core::executor;
use koushi_core::runtime::{CommandSubmitError, CoreRuntime};
use koushi_protocol::command::{AppCommand, CoreCommand, RoomCommand};
use koushi_protocol::event::CoreEvent;
use koushi_state::{
    AppAction, AuthDiscoveryState, ComposerDraftStore, SessionState, SettingsPatch, ThreadListOrder,
};

mod support;

#[tokio::test]
async fn frontend_neutral_consumer_converges_and_shuts_down_without_tauri() {
    let data_dir = tempfile::tempdir().expect("runtime data dir");
    let runtime = CoreRuntime::start_with_data_dir(data_dir.path().to_owned());
    let mut connection = runtime.attach();
    let initial = connection.versioned_snapshot();
    let request_id = connection.next_request_id();

    connection
        .command(CoreCommand::App(AppCommand::UpdateSettings {
            request_id,
            patch: SettingsPatch {
                thread_list_order: Some(ThreadListOrder::RootChronology),
                ..SettingsPatch::default()
            },
        }))
        .await
        .expect("submit typed Core command");

    executor::timeout(Duration::from_secs(1), async {
        loop {
            match connection.recv_event().await.expect("Core event") {
                CoreEvent::StateDelta(delta)
                    if delta.changed.settings.as_ref().is_some_and(|settings| {
                        settings.values.thread_list_order == ThreadListOrder::RootChronology
                    }) =>
                {
                    break;
                }
                _ => continue,
            }
        }
    })
    .await
    .expect("typed command should converge through Core event");

    let current = connection.versioned_snapshot();
    assert!(current.generation > initial.generation);
    assert_eq!(
        current.state.settings.values.thread_list_order,
        ThreadListOrder::RootChronology
    );
    let independent_consumer = runtime.attach();
    assert_eq!(independent_consumer.versioned_snapshot(), current);

    drop(independent_consumer);
    drop(connection);
    executor::timeout(Duration::from_secs(1), runtime.shutdown())
        .await
        .expect("Core runtime should complete awaited shutdown");
}

#[tokio::test]
async fn mismatched_request_id_fails_locally_without_publishing() {
    let runtime = CoreRuntime::start();
    let intruder = runtime.attach();
    let mut observer = runtime.attach();

    let foreign_id = observer.next_request_id();
    let result = intruder
        .command(CoreCommand::Room(RoomCommand::JoinRoom {
            request_id: foreign_id,
            room_id: "!room:example.test".to_owned(),
        }))
        .await;
    assert_eq!(result, Err(CommandSubmitError::InvalidRequestId));

    // No CoreEvent may be published with the forged RequestId.
    let outcome = executor::timeout(Duration::from_millis(100), observer.recv_event()).await;
    assert!(
        outcome.is_err(),
        "no event should be published for a rejected submission"
    );
}

#[tokio::test]
async fn result_events_correlate_in_submission_order() {
    let runtime = CoreRuntime::start();
    let mut connection = runtime.attach();

    let first = connection.next_request_id();
    let second = connection.next_request_id();
    assert_ne!(first, second);

    for request_id in [first, second] {
        connection
            .command(CoreCommand::Room(RoomCommand::JoinRoom {
                request_id,
                room_id: "!room:example.test".to_owned(),
            }))
            .await
            .expect("submit");
    }

    let mut seen = Vec::new();
    while seen.len() < 2 {
        if let CoreEvent::OperationFailed { request_id, .. } =
            connection.recv_event().await.expect("event")
        {
            seen.push(request_id);
        }
    }
    assert_eq!(seen, vec![first, second], "events must be ordered");
}

#[tokio::test]
async fn reducer_actions_coalesce_into_one_contiguous_delta_without_full_snapshot_events() {
    let runtime = CoreRuntime::start();
    let mut connection = runtime.attach();

    runtime
        .inject_actions(vec![
            AppAction::AppStarted,
            AppAction::RestoreSessionFailed {
                message: "synthetic".to_owned(),
            },
            AppAction::LoginDiscoveryRequested {
                homeserver: "https://example.test".to_owned(),
            },
        ])
        .await;

    let mut state_delta_generations = Vec::new();
    let mut non_delta_events = 0;
    // Drain everything emitted within a quiet period.
    while let Ok(Ok(event)) =
        executor::timeout(Duration::from_millis(200), connection.recv_event()).await
    {
        if let CoreEvent::StateDelta(delta) = event {
            state_delta_generations.push(delta.generation);
        } else {
            non_delta_events += 1;
        }
    }

    assert_eq!(state_delta_generations, vec![1]);
    assert_eq!(
        non_delta_events, 0,
        "state publication must use only StateDelta"
    );
    let last = connection.snapshot();
    // The final state reflects the LAST action in the batch.
    assert!(matches!(last.auth, AuthDiscoveryState::Discovering { .. }));
    assert_eq!(last, connection.snapshot());
}

#[tokio::test]
async fn snapshot_only_refresh_wakes_watch_without_advancing_generation_or_emitting_delta() {
    let runtime = CoreRuntime::start();
    let mut connection = runtime.attach();
    runtime
        .inject_actions(support::restore_ready_actions())
        .await;

    loop {
        let snapshot = connection.versioned_snapshot();
        if matches!(snapshot.state.session, SessionState::Ready(_)) {
            break;
        }
        connection
            .next_versioned_snapshot()
            .await
            .expect("runtime snapshot stream should remain open");
    }
    while executor::timeout(Duration::from_millis(20), connection.recv_event())
        .await
        .is_ok()
    {}
    let before = connection.versioned_snapshot();
    let mut drafts = ComposerDraftStore::default();
    drafts.set_room_draft("!snapshot-only:example.invalid".to_owned(), "draft");

    runtime
        .inject_actions(vec![AppAction::ComposerDraftsLoaded { drafts }])
        .await;
    let after = connection
        .next_versioned_snapshot()
        .await
        .expect("snapshot-only refresh should wake the watch");

    assert_eq!(after.generation, before.generation);
    assert_eq!(
        after
            .state
            .composer_drafts
            .room_revision("!snapshot-only:example.invalid"),
        koushi_state::ComposerDraftRevision::from_u64(1)
    );
    assert!(
        executor::timeout(Duration::from_millis(100), connection.recv_event())
            .await
            .is_err(),
        "snapshot-only state must wake only the snapshot watch"
    );
}

#[tokio::test]
async fn slow_consumer_observes_lag_and_recovers_via_snapshot() {
    let runtime = CoreRuntime::start_with_event_capacity(4);
    let pump = runtime.attach();
    let mut slow = runtime.attach();

    // Overflow the slow consumer's bounded queue.
    for _ in 0..32 {
        let request_id = pump.next_request_id();
        pump.command(CoreCommand::Room(RoomCommand::JoinRoom {
            request_id,
            room_id: "!room:example.test".to_owned(),
        }))
        .await
        .expect("submit");
    }
    runtime.inject_actions(vec![AppAction::AppStarted]).await;
    executor::sleep(Duration::from_millis(100)).await;

    let first = slow.recv_event().await;
    assert!(first.is_err(), "slow consumer must observe the lag marker");

    // Recovery path: latest-wins snapshot is intact and current.
    assert!(matches!(
        slow.snapshot().session,
        SessionState::Restoring | SessionState::SignedOut
    ));

    drop(slow);
    drop(pump);
    executor::timeout(Duration::from_secs(1), runtime.shutdown())
        .await
        .expect("lag recovery consumers should release runtime shutdown");
}
