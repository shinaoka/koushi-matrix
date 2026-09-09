use super::*;

#[tokio::test]
async fn runtime_loop_settles_inactive_reader_without_manual_actor_pumping() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
        directory.path().join("data"),
        directory.path().join("credentials"),
    );
    let connection = runtime.attach();
    let consumer = connection.view_consumer().unwrap();
    let source = serde_json::from_value(serde_json::json!({
        "key": {"account_key": "@owner:example.org", "kind": {"Room": {"room_id": "!room:example.org"}}},
        "projection_request_id": {"connection_id": "1", "sequence": "2"},
        "generation": "3", "event_id": "$event"
    })).unwrap();
    let mut scope = consumer
        .open_reader(
            source,
            0,
            koushi_protocol::view::ReaderWindowLimit::try_from(3).unwrap(),
        )
        .unwrap();
    drop(connection);
    let delivery =
        crate::executor::timeout(std::time::Duration::from_secs(1), scope.next_delivery())
            .await
            .expect("actual actor loop must consume the reader queue");
    assert!(matches!(
        delivery,
        Some(crate::view_scope_lifecycle::ScopeDelivery::Retired(
            koushi_protocol::view::ViewRetirement::SessionRetired
        ))
    ));
    assert!(!scope.is_live());
    assert!(
        consumer.open().is_ok(),
        "a failed reader does not retire its logical consumer"
    );
    consumer.ack_retirement(scope.id()).unwrap();
    drop(runtime);
}

#[tokio::test]
async fn attached_view_contexts_survive_connection_drop_and_retire_with_runtime() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
        directory.path().join("data"),
        directory.path().join("credentials"),
    );
    let first = runtime.attach();
    let second = runtime.attach();
    let a = first.view_consumer().unwrap();
    let sibling = first.view_consumer().unwrap();
    let b = second.view_consumer().unwrap();
    let scope_a = a.open().unwrap();
    let scope_sibling = sibling.open().unwrap();
    let mut scope_b = b.open().unwrap();
    drop(first);
    assert!(scope_a.is_live());
    assert!(scope_sibling.is_live());
    a.retire();
    assert!(!scope_a.is_live());
    assert!(scope_sibling.is_live());
    assert!(scope_b.is_live());
    drop(runtime);
    let end = crate::executor::timeout(std::time::Duration::from_secs(1), scope_b.next_delivery())
        .await
        .unwrap();
    assert!(matches!(
        end,
        Some(crate::view_scope_lifecycle::ScopeDelivery::Retired(
            koushi_protocol::view::ViewRetirement::RuntimeStopped
        ))
    ));
    assert!(!scope_sibling.is_live());
    assert!(second.view_consumer().is_err());
}

use futures_util::FutureExt;
use koushi_protocol::event::{
    ThreadSummaryDto, TimelineDiff, TimelineEvent, TimelineItem, TimelineItemId,
};
use koushi_protocol::ids::{AccountKey, TimelineKey, TimelineKind};
use koushi_state::{
    AppAction, AppState, ComposerTarget, LocalUserAliasUpdateState, OwnProfile, ProfileState,
    SessionInfo, UserProfile, reduce,
};
use std::collections::{BTreeMap, BTreeSet};

fn scripted_connection(
    event_capacity: usize,
) -> (
    CoreConnection,
    mpsc::Receiver<CoreCommandEnvelope>,
    broadcast::Sender<CoreEvent>,
    watch::Sender<VersionedAppStateSnapshot>,
) {
    let connection_id = RuntimeConnectionId(41);
    let (command_tx, command_rx) = mpsc::channel(4);
    let (event_tx, event_rx) = broadcast::channel(event_capacity);
    let (snapshot_tx, snapshot_rx) = watch::channel(VersionedAppStateSnapshot {
        generation: 0,
        state: AppState::default(),
    });
    (
        CoreConnection {
            view_scopes: crate::view_scope_lifecycle::ViewScopeRegistry::default(),
            connection_id,
            command_tx,
            composer_draft_leases: Arc::new(ComposerDraftLeaseRegistry::new()),
            native_artifacts: Arc::new(crate::native_artifact::RejectingNativeArtifactPort),
            media_staging: Arc::new(MediaStagingService::new(Arc::new(
                crate::media_preparation::MediaPreparationService::default(),
            ))),
            event_rx,
            snapshot_rx,
            next_sequence: AtomicU64::new(1),
        },
        command_rx,
        event_tx,
        snapshot_tx,
    )
}

#[test]
fn reader_avatar_observation_uses_runtime_context_and_honors_close() {
    let (connection, _commands, _events, _snapshots) = scripted_connection(4);
    let source = serde_json::from_value(serde_json::json!({
        "key": {"account_key": "@owner:example.org", "kind": {"Room": {"room_id": "!room:example.org"}}},
        "projection_request_id": {"connection_id": "1", "sequence": "2"},
        "generation": "3", "event_id": "$event"
    })).unwrap();
    let reader = connection
        .subscribe_reader(
            source,
            0,
            koushi_protocol::view::ReaderWindowLimit::try_from(3).unwrap(),
        )
        .unwrap();
    let revision = koushi_protocol::view::ViewRevision(1);
    assert_eq!(
        reader.observe_avatars(revision, 1, &[], &[]),
        Err(crate::view_scope_lifecycle::ScopeError::InactiveSession)
    );
    reader.close_handle().close();
    assert_eq!(
        reader.observe_avatars(revision, 1, &[], &[]),
        Err(crate::view_scope_lifecycle::ScopeError::Closed)
    );
}

#[test]
fn resource_access_rejects_foreign_runtime_and_retired_consumers_at_connection_boundary() {
    use crate::view_scope_lifecycle::ScopeError;
    use koushi_protocol::view::ViewRevision;
    let (connection, _commands, _events, _snapshots) = scripted_connection(4);
    let (foreign_connection, _commands2, _events2, _snapshots2) = scripted_connection(4);
    assert_eq!(
        connection.connection_id(),
        foreign_connection.connection_id()
    );
    let consumer = connection.view_consumer().unwrap();
    let foreign = foreign_connection.view_consumer().unwrap();
    let scope = consumer.open().unwrap();
    assert_eq!(
        connection
            .view_resource(&foreign, scope.id(), ViewRevision(1), "avatar/unknown")
            .err(),
        Some(ScopeError::NotOwned)
    );
    assert_eq!(
        connection
            .view_resource(&consumer, scope.id(), ViewRevision(1), "avatar/unknown")
            .err(),
        Some(ScopeError::InvalidRevision)
    );
    consumer.retire();
    assert!(
        connection
            .view_resource(&consumer, scope.id(), ViewRevision(1), "avatar/unknown")
            .is_err()
    );
}

#[tokio::test]
async fn receipt_window_uses_the_existing_command_lane_without_a_snapshot_copy() {
    for retirement_phase in 0..6 {
        let retire_before_handoff = retirement_phase == 1;
        let (connection, mut commands, _events, _snapshots) = scripted_connection(4);
        let source = koushi_protocol::view::ReceiptSourceRef {
            timeline: koushi_protocol::view::TimelineViewSource {
                key: TimelineKey::room(AccountKey("account".into()), "!room:example.org"),
                projection_request_id: RequestId {
                    connection_id: RuntimeConnectionId(1),
                    sequence: 2,
                },
                generation: koushi_protocol::TimelineGeneration(3),
            },
            event_id: "$event".into(),
        };
        let limit = koushi_protocol::view::ReaderWindowLimit::try_from(3).unwrap();
        let (foreign_connection, _foreign_commands, _foreign_events, _foreign_snapshots) =
            scripted_connection(4);
        assert_eq!(
            connection.connection_id(),
            foreign_connection.connection_id()
        );
        let foreign = foreign_connection.view_consumer().unwrap();
        let retired = connection.view_consumer().unwrap();
        retired.retire();
        for denied in [&foreign, &retired] {
            assert!(matches!(
                connection
                    .read_receipt_window(denied, source.clone(), 7, limit)
                    .now_or_never(),
                Some(Err(crate::view_scope_lifecycle::ScopeError::NotOwned
                    | crate::view_scope_lifecycle::ScopeError::Closed))
            ));
            assert!(matches!(
                commands.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
            ));
        }
        let consumer = connection.view_consumer().unwrap();
        let retirement = consumer.clone();
        let runtime_views = connection.view_scopes.clone();
        let reader = tokio::spawn(async move {
            connection
                .read_receipt_window(&consumer, source, 7, limit)
                .await
        });
        let Some(CoreCommandEnvelope::ReadReceiptWindow {
            source,
            start,
            limit,
            response,
        }) = commands.recv().await
        else {
            panic!("expected bounded receipt request");
        };
        if retirement_phase == 5 {
            assert!(
                response
                    .send(Err(
                        crate::view_scope_lifecycle::ScopeError::CounterExhausted
                    ))
                    .is_ok()
            );
            assert_eq!(
                reader.await.unwrap().err(),
                Some(crate::view_scope_lifecycle::ScopeError::CounterExhausted)
            );
            continue;
        }
        if matches!(retirement_phase, 2 | 3) {
            if retirement_phase == 2 {
                retirement.retire();
            } else {
                runtime_views.shutdown();
            }
            let result = crate::executor::timeout(std::time::Duration::from_secs(1), reader)
                .await
                .expect("retirement must not wait for SDK reply")
                .unwrap();
            assert_eq!(
                result.err(),
                Some(crate::view_scope_lifecycle::ScopeError::Closed)
            );
            assert!(response.is_closed());
            continue;
        }
        assert_eq!(source.event_id, "$event");
        assert_eq!(start, 7);
        assert_eq!(limit.get(), 3);
        let mut window = crate::timeline::RawReceiptWindow {
            total_count: 1500,
            start,
            receipts: Vec::new(),
            profiles: Vec::new(),
            owner: None,
            epoch: std::sync::Weak::new(),
        };
        let epoch = window.bind_test_owner(&source).await;
        assert!(response.send(Ok(window)).is_ok());
        let Some(CoreCommandEnvelope::ResolveReceiptWindow {
            source,
            window,
            response,
        }) = commands.recv().await
        else {
            panic!("expected AppActor profile resolution");
        };
        assert_eq!(source.event_id, "$event");
        assert!(
            response
                .send(Ok(window.into_resolved(koushi_state::CatalogLocale::En)))
                .is_ok()
        );
        if retire_before_handoff {
            retirement.retire();
        }
        let _live_epoch = if retirement_phase == 4 {
            drop(epoch);
            None
        } else {
            Some(epoch)
        };
        let result = reader.await.unwrap();
        if retire_before_handoff || retirement_phase == 4 {
            assert_eq!(
                result.err(),
                Some(if retirement_phase == 4 {
                    crate::view_scope_lifecycle::ScopeError::SourceUnavailable
                } else {
                    crate::view_scope_lifecycle::ScopeError::Closed
                })
            );
        } else {
            let result = result.unwrap();
            assert_eq!(result.total_count, 1500);
            assert_eq!(result.start, 7);
        }
    }
}

fn event_navigation_snapshot(
    generation: u64,
    event_navigation: EventNavigationState,
) -> VersionedAppStateSnapshot {
    let mut state = AppState::default();
    state.navigation.event_navigation = event_navigation;
    VersionedAppStateSnapshot { generation, state }
}

fn event_navigation_request(
    connection: &mut CoreConnection,
) -> impl std::future::Future<Output = Result<VersionedAppStateSnapshot, EventNavigationError>> + '_
{
    connection.navigate_to_event_and_wait(
        "!room:example.invalid".to_owned(),
        "$event:example.invalid".to_owned(),
        EventNavigationSource::Activity,
        EventNavigationMissingTargetPolicy::Fail,
        Duration::from_secs(1),
    )
}

#[tokio::test]
async fn event_navigation_same_generation_succeeds() {
    let (mut connection, mut control) = CoreConnection::new_for_testing(4);
    let mut waiter = Box::pin(event_navigation_request(&mut connection));
    assert!(waiter.as_mut().now_or_never().is_none());
    let _ = control.recv_command().await.expect("navigation command");
    let snapshot = event_navigation_snapshot(
        1,
        EventNavigationState::Anchored {
            generation: 1,
            source: EventNavigationSource::Activity,
        },
    );
    control.send_snapshot(snapshot.clone());
    assert_eq!(waiter.await, Ok(snapshot));
}

#[tokio::test]
async fn event_navigation_same_generation_failure_is_typed() {
    let (mut connection, mut control) = CoreConnection::new_for_testing(4);
    let mut waiter = Box::pin(event_navigation_request(&mut connection));
    assert!(waiter.as_mut().now_or_never().is_none());
    let _ = control.recv_command().await.expect("navigation command");
    control.send_snapshot(event_navigation_snapshot(
        1,
        EventNavigationState::Failed {
            generation: 1,
            source: EventNavigationSource::Activity,
            failure_kind: EventNavigationFailureKind::Timeline,
        },
    ));
    assert_eq!(
        waiter.await,
        Err(EventNavigationError::Failed(
            EventNavigationFailureKind::Timeline
        ))
    );
}

#[tokio::test]
async fn event_navigation_newer_generation_is_benign_success() {
    let (mut connection, mut control) = CoreConnection::new_for_testing(4);
    let mut waiter = Box::pin(event_navigation_request(&mut connection));
    assert!(waiter.as_mut().now_or_never().is_none());
    let _ = control.recv_command().await.expect("navigation command");
    let snapshot = event_navigation_snapshot(
        2,
        EventNavigationState::Opening {
            generation: 2,
            source: EventNavigationSource::Search,
        },
    );
    control.send_snapshot(snapshot.clone());
    assert_eq!(waiter.await, Ok(snapshot));
}

#[tokio::test]
async fn event_navigation_generation_overflow_is_rejected() {
    let (mut connection, mut control) = CoreConnection::new_for_testing(4);
    control.send_snapshot(event_navigation_snapshot(
        1,
        EventNavigationState::Opening {
            generation: u64::MAX,
            source: EventNavigationSource::Activity,
        },
    ));
    assert_eq!(
        event_navigation_request(&mut connection).await,
        Err(EventNavigationError::Rejected)
    );
}

#[tokio::test]
async fn event_navigation_times_out_without_terminal_snapshot() {
    let (mut connection, mut control) = CoreConnection::new_for_testing(4);
    let mut waiter = Box::pin(connection.navigate_to_event_and_wait(
        "!room:example.invalid".to_owned(),
        "$event:example.invalid".to_owned(),
        EventNavigationSource::Activity,
        EventNavigationMissingTargetPolicy::Fail,
        Duration::from_millis(1),
    ));
    assert!(waiter.as_mut().now_or_never().is_none());
    let _ = control.recv_command().await.expect("navigation command");
    assert_eq!(waiter.await, Err(EventNavigationError::Timeout));
}

#[tokio::test]
async fn event_navigation_closed_stream_is_reported() {
    let (mut connection, mut control) = CoreConnection::new_for_testing(4);
    let mut waiter = Box::pin(event_navigation_request(&mut connection));
    assert!(waiter.as_mut().now_or_never().is_none());
    let _ = control.recv_command().await.expect("navigation command");
    drop(control);
    assert_eq!(waiter.await, Err(EventNavigationError::EventStreamClosed));
}

#[tokio::test]
async fn unrelated_command_cannot_claim_a_native_artifact_registration() {
    let (mut connection, mut command_rx, _event_tx, _snapshot_tx) = scripted_connection(1);
    let registry = Arc::new(crate::native_artifact::NativeArtifactRegistry::new());
    connection.native_artifacts = registry.clone();
    let request_id = connection.next_request_id();

    let result = connection
        .command_handle()
        .command_with_native_artifact_and_admission(
            CoreCommand::App(koushi_protocol::AppCommand::UpdateSettings {
                request_id,
                patch: koushi_state::SettingsPatch::default(),
            }),
            crate::native_artifact::NativeArtifactKind::RoomKeyExportDestination,
            std::path::PathBuf::from("synthetic-path"),
        )
        .await;

    assert!(matches!(
        result,
        Err(CommandSubmitError::NativeArtifact(
            crate::native_artifact::NativeArtifactError::Missing
        ))
    ));
    assert!(registry.is_empty());
    assert!(matches!(
        command_rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn cancelled_native_artifact_enqueue_releases_the_registered_path() {
    let (mut connection, _command_rx, _event_tx, _snapshot_tx) = scripted_connection(4);
    let registry = Arc::new(crate::native_artifact::NativeArtifactRegistry::new());
    connection.native_artifacts = registry.clone();

    for _ in 0..4 {
        let request_id = connection.next_request_id();
        connection
            .command(CoreCommand::App(
                koushi_protocol::AppCommand::UpdateSettings {
                    request_id,
                    patch: koushi_state::SettingsPatch::default(),
                },
            ))
            .await
            .expect("fill command queue");
    }

    let request_id = connection.next_request_id();
    let command_handle = connection.command_handle();
    let mut submission = Box::pin(command_handle.command_with_native_artifact_and_admission(
        CoreCommand::Account(koushi_protocol::AccountCommand::ExportRoomKeys {
            request_id,
            request: koushi_protocol::RoomKeyExportRequest {
                passphrase: koushi_state::AuthSecret::new("synthetic-passphrase"),
            },
        }),
        crate::native_artifact::NativeArtifactKind::RoomKeyExportDestination,
        std::path::PathBuf::from("synthetic-path"),
    ));
    assert!(submission.as_mut().now_or_never().is_none());
    drop(submission);
    assert!(registry.is_empty());
}

fn selected_snapshot(room_id: &str, generation: u64) -> VersionedAppStateSnapshot {
    let mut state = AppState::default();
    state.navigation.active_room_id = Some(room_id.to_owned());
    VersionedAppStateSnapshot { generation, state }
}

#[tokio::test]
async fn committed_lifecycle_waits_for_the_matching_published_snapshot() {
    let room_id = "!committed-before-watch:example.test";
    let (mut connection, mut command_rx, event_tx, snapshot_tx) = scripted_connection(4);
    let mut waiter =
        Box::pin(connection.select_room_and_wait(room_id.to_owned(), Duration::from_secs(1)));
    assert!(waiter.as_mut().now_or_never().is_none());
    let request_id = command_rx
        .recv()
        .await
        .expect("select command")
        .command()
        .request_id();

    event_tx
        .send(CoreEvent::IntentLifecycle {
            request_id,
            outcome: IntentOutcome::Committed,
            published_generation: 17,
        })
        .expect("committed lifecycle");
    assert!(
        waiter.as_mut().now_or_never().is_none(),
        "telemetry must not settle selection before watch publication"
    );

    let published = selected_snapshot(room_id, 17);
    snapshot_tx
        .send(published.clone())
        .expect("publish selected snapshot");
    assert_eq!(
        waiter.await.expect("settled selection"),
        published.generation
    );
    assert_eq!(connection.versioned_snapshot(), published);
}

#[tokio::test]
async fn select_room_waiter_recovers_lag_from_latest_watch_snapshot() {
    let room_id = "!lagged-watch:example.test";
    let (mut connection, mut command_rx, event_tx, snapshot_tx) = scripted_connection(1);
    let mut waiter =
        Box::pin(connection.select_room_and_wait(room_id.to_owned(), Duration::from_secs(1)));
    assert!(waiter.as_mut().now_or_never().is_none());
    let _command = command_rx.recv().await.expect("select command");

    event_tx
        .send(CoreEvent::OperationFailed {
            request_id: RequestId {
                connection_id: RuntimeConnectionId(99),
                sequence: 1,
            },
            failure: koushi_protocol::failure::CoreFailure::SessionRequired,
        })
        .expect("first event");
    event_tx
        .send(CoreEvent::OperationFailed {
            request_id: RequestId {
                connection_id: RuntimeConnectionId(99),
                sequence: 2,
            },
            failure: koushi_protocol::failure::CoreFailure::SessionRequired,
        })
        .expect("overflowing event");
    let published = selected_snapshot(room_id, 23);
    snapshot_tx
        .send(published.clone())
        .expect("publish selected snapshot");

    assert_eq!(
        waiter.await.expect("lag recovery settlement"),
        published.generation
    );
    assert_eq!(connection.versioned_snapshot(), published);
}

#[tokio::test]
async fn matching_operation_failure_returns_the_typed_core_failure() {
    let (mut connection, mut command_rx, event_tx, _snapshot_tx) = scripted_connection(1);
    let mut waiter = Box::pin(connection.select_room_and_wait(
        "!matching-operation-failure:example.test".to_owned(),
        Duration::from_secs(1),
    ));
    assert!(waiter.as_mut().now_or_never().is_none());
    let request_id = command_rx
        .recv()
        .await
        .expect("select command")
        .command()
        .request_id();

    event_tx
        .send(CoreEvent::OperationFailed {
            request_id,
            failure: koushi_protocol::failure::CoreFailure::SessionRequired,
        })
        .expect("matching failure");
    assert_eq!(
        waiter.await,
        Err(SelectRoomError::OperationFailed(
            koushi_protocol::failure::CoreFailure::SessionRequired
        ))
    );
}

#[tokio::test]
async fn matching_superseded_lifecycle_returns_the_typed_noop() {
    let (mut connection, mut command_rx, event_tx, _snapshot_tx) = scripted_connection(1);
    let mut waiter = Box::pin(connection.select_room_and_wait(
        "!matching-superseded:example.test".to_owned(),
        Duration::from_secs(1),
    ));
    assert!(waiter.as_mut().now_or_never().is_none());
    let request_id = command_rx
        .recv()
        .await
        .expect("select command")
        .command()
        .request_id();

    event_tx
        .send(CoreEvent::IntentLifecycle {
            request_id,
            outcome: IntentOutcome::FailedNoOp(IntentNoOpReason::Superseded),
            published_generation: 0,
        })
        .expect("matching superseded lifecycle");
    assert_eq!(
        waiter.await,
        Err(SelectRoomError::FailedNoOp(IntentNoOpReason::Superseded))
    );
}

#[tokio::test]
async fn unrelated_request_failures_do_not_settle_room_selection() {
    let room_id = "!unrelated-request:example.test";
    let (mut connection, mut command_rx, event_tx, snapshot_tx) = scripted_connection(4);
    let mut waiter =
        Box::pin(connection.select_room_and_wait(room_id.to_owned(), Duration::from_secs(1)));
    assert!(waiter.as_mut().now_or_never().is_none());
    let request_id = command_rx
        .recv()
        .await
        .expect("select command")
        .command()
        .request_id();
    let unrelated_request_id = RequestId {
        connection_id: request_id.connection_id,
        sequence: request_id.sequence + 1,
    };

    event_tx
        .send(CoreEvent::OperationFailed {
            request_id: unrelated_request_id,
            failure: koushi_protocol::failure::CoreFailure::SessionRequired,
        })
        .expect("unrelated failure");
    event_tx
        .send(CoreEvent::IntentLifecycle {
            request_id: unrelated_request_id,
            outcome: IntentOutcome::FailedNoOp(IntentNoOpReason::RoomNotInState),
            published_generation: 0,
        })
        .expect("unrelated lifecycle");
    assert!(waiter.as_mut().now_or_never().is_none());

    let published = selected_snapshot(room_id, 29);
    snapshot_tx
        .send(published.clone())
        .expect("publish selected snapshot");
    assert_eq!(
        waiter.await.expect("settled selection"),
        published.generation
    );
    assert_eq!(connection.versioned_snapshot(), published);
}

#[tokio::test]
async fn closed_event_stream_returns_a_final_matching_snapshot() {
    let room_id = "!closed-after-publish:example.test";
    let (mut connection, mut command_rx, event_tx, snapshot_tx) = scripted_connection(1);
    let mut waiter =
        Box::pin(connection.select_room_and_wait(room_id.to_owned(), Duration::from_secs(1)));
    assert!(waiter.as_mut().now_or_never().is_none());
    let _command = command_rx.recv().await.expect("select command");
    let published = selected_snapshot(room_id, 31);
    snapshot_tx
        .send(published.clone())
        .expect("publish final selected snapshot");

    drop(event_tx);
    assert_eq!(
        waiter.await.expect("final watch settlement"),
        published.generation
    );
    assert_eq!(connection.versioned_snapshot(), published);
}

#[tokio::test]
async fn closed_event_stream_returns_a_typed_selection_error() {
    let (mut connection, mut command_rx, event_tx, _snapshot_tx) = scripted_connection(1);
    let mut waiter = Box::pin(connection.select_room_and_wait(
        "!closed-stream:example.test".to_owned(),
        Duration::from_secs(1),
    ));
    assert!(waiter.as_mut().now_or_never().is_none());
    let _command = command_rx.recv().await.expect("select command");

    drop(event_tx);
    assert_eq!(waiter.await, Err(SelectRoomError::EventStreamClosed));
}

#[test]
fn standalone_composer_command_permit_outlives_activation_lease() {
    let composer_draft_leases = Arc::new(ComposerDraftLeaseRegistry::new());
    let (command_tx, _command_rx) = mpsc::channel(1);
    let handle = CoreCommandHandle {
        connection_id: RuntimeConnectionId(1),
        command_tx,
        composer_draft_leases: Arc::clone(&composer_draft_leases),
        native_artifacts: Arc::new(crate::native_artifact::RejectingNativeArtifactPort),
    };
    let account = koushi_protocol::SessionKeyId {
        homeserver: "https://example.invalid".to_owned(),
        user_id: "@permit:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
    };
    let target = ComposerTarget::Main {
        room_id: "!room:example.invalid".to_owned(),
    };
    let scope = ComposerDraftScope {
        account: account.clone(),
        target: target.clone(),
    };
    let generation = handle
        .begin_composer_draft_renderer_generation()
        .expect("renderer generation");
    let lease_id = handle
        .acquire_composer_draft_lease(generation, scope.clone())
        .expect("activation lease");
    let permit = handle
        .acquire_composer_draft_command_permit(generation, lease_id, &scope)
        .expect("standalone terminal permit");

    handle
        .release_composer_draft_lease(generation, lease_id)
        .expect("release activation lease");
    assert_eq!(
        composer_draft_leases.protected_targets(&account),
        std::collections::BTreeSet::from([target.clone()])
    );

    drop(permit);
    assert!(composer_draft_leases.protected_targets(&account).is_empty());
}

#[tokio::test]
async fn timeline_sender_label_and_reaction_sender_preview_follow_people_facing_policy() {
    let (command_tx, _command_rx) = mpsc::channel(1);
    let (event_tx, event_rx) = broadcast::channel(4);
    let mut state = AppState::default();
    reduce(&mut state, AppAction::AppStarted);
    reduce(
        &mut state,
        AppAction::RestoreSessionSucceeded(SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@me:example.invalid".to_owned(),
            device_id: "DEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
    );
    reduce(
        &mut state,
        AppAction::CurrentDeviceTrustChanged(koushi_state::CurrentDeviceTrustState::Verified),
    );
    state.profile = ProfileState {
        own: OwnProfile {
            display_name: Some("Me Upstream".to_owned()),
            avatar: None,
        },
        room_users: BTreeMap::new(),
        ignored_user_ids: BTreeSet::new(),
        ignored_user_update: koushi_state::IgnoredUserUpdateState::Idle,
        users: BTreeMap::from([
            (
                "@alice:example.invalid".to_owned(),
                UserProfile {
                    user_id: "@alice:example.invalid".to_owned(),
                    display_name: Some("Alice Upstream".to_owned()),
                    display_label: "Alice Alias".to_owned(),
                    original_display_label: "Alice Upstream".to_owned(),
                    mention_search_terms: vec![],
                    avatar: None,
                },
            ),
            (
                "@bob:example.invalid".to_owned(),
                UserProfile {
                    user_id: "@bob:example.invalid".to_owned(),
                    display_name: Some("Bob Upstream".to_owned()),
                    display_label: "Bob Alias".to_owned(),
                    original_display_label: "Bob Upstream".to_owned(),
                    mention_search_terms: vec![],
                    avatar: None,
                },
            ),
            (
                "@carol:example.invalid".to_owned(),
                UserProfile {
                    user_id: "@carol:example.invalid".to_owned(),
                    display_name: Some("Carol Upstream".to_owned()),
                    display_label: "Carol Alias".to_owned(),
                    original_display_label: "Carol Upstream".to_owned(),
                    mention_search_terms: vec![],
                    avatar: None,
                },
            ),
        ]),
        local_aliases: BTreeMap::from([
            (
                "@alice:example.invalid".to_owned(),
                "Alice Alias".to_owned(),
            ),
            ("@bob:example.invalid".to_owned(), "Bob Alias".to_owned()),
            (
                "@carol:example.invalid".to_owned(),
                "Carol Alias".to_owned(),
            ),
        ]),
        local_alias_update: LocalUserAliasUpdateState::Idle,
        update: Default::default(),
    };
    let (_snapshot_tx, snapshot_rx) = watch::channel(VersionedAppStateSnapshot {
        generation: 0,
        state,
    });
    let mut connection = CoreConnection {
        view_scopes: crate::view_scope_lifecycle::ViewScopeRegistry::default(),
        connection_id: RuntimeConnectionId(7),
        command_tx,
        composer_draft_leases: Arc::new(ComposerDraftLeaseRegistry::new()),
        native_artifacts: Arc::new(crate::native_artifact::RejectingNativeArtifactPort),
        media_staging: Arc::new(MediaStagingService::new(Arc::new(
            crate::media_preparation::MediaPreparationService::default(),
        ))),
        event_rx,
        snapshot_rx,
        next_sequence: AtomicU64::new(1),
    };
    let key = TimelineKey {
        account_key: AccountKey("@me:example.invalid".to_owned()),
        kind: TimelineKind::Room {
            room_id: "!room:example.invalid".to_owned(),
        },
    };

    let _ = event_tx.send(CoreEvent::Timeline(TimelineEvent::InitialItems {
        request_id: None,
        cause_request_id: None,
        key,
        actor_generation: 0,
        generation: koushi_protocol::ids::TimelineGeneration(0),
        items: vec![TimelineItem {
            request_state: None,
            id: TimelineItemId::Event {
                event_id: "$event:example.invalid".to_owned(),
            },
            sender: Some("@alice:example.invalid".to_owned()),
            sender_label: Some("Alice Room Name".to_owned()),
            sender_avatar: None,
            body: Some("hello".to_owned()),
            notice_i18n: None,
            message_kind: Default::default(),
            spoiler_spans: Vec::new(),
            timestamp_ms: Some(1),
            in_reply_to_event_id: Some("$root:example.invalid".to_owned()),
            formatted: None,
            reply_quote: Some(koushi_state::ReplyQuote {
                event_id: "$root:example.invalid".to_owned(),
                sender: Some("@bob:example.invalid".to_owned()),
                sender_label: None,
                body_preview: Some("quoted".to_owned()),
                formatted: None,
                state: koushi_state::ReplyQuoteState::Ready,
            }),
            thread_root: None,
            thread_summary: Some(ThreadSummaryDto {
                reply_count: 1,
                latest_event_id: Some("$latest:example.invalid".to_owned()),
                latest_sender: Some("@carol:example.invalid".to_owned()),
                latest_sender_label: None,
                latest_body_preview: Some("latest".to_owned()),
                latest_timestamp_ms: Some(2),
            }),
            media: None,
            link_previews: None,
            link_ranges: Vec::new(),
            reactions: vec![koushi_protocol::event::ReactionGroup {
                key: "👍".to_owned(),
                count: 1,
                reacted_by_me: false,
                my_reaction_event_id: None,
                sender_preview: vec![koushi_protocol::event::ReactionSender {
                    user_id: "@bob:example.invalid".to_owned(),
                    display_label: Some("Bob Room Name".to_owned()),
                }],
            }],
            can_react: false,
            is_redacted: false,
            is_hidden: false,
            can_redact: false,
            is_edited: false,
            can_edit: false,
            actions: Default::default(),
            send_state: None,
            unable_to_decrypt: None,
            display_metadata: None,
        }],
    }));

    match connection.recv_event().await.expect("timeline event") {
        CoreEvent::Timeline(TimelineEvent::InitialItems { items, .. }) => {
            let item = items.first().expect("projected item");
            assert_eq!(item.sender.as_deref(), Some("@alice:example.invalid"));
            assert_eq!(item.sender_label.as_deref(), Some("Alice Alias"));
            assert_eq!(
                item.reactions[0].sender_preview[0].display_label.as_deref(),
                Some("Bob Alias")
            );
            let quote = item.reply_quote.as_ref().expect("reply quote");
            assert_eq!(quote.sender.as_deref(), Some("@bob:example.invalid"));
            assert_eq!(quote.sender_label.as_deref(), Some("Bob Alias"));
            let thread = item.thread_summary.as_ref().expect("thread summary");
            assert_eq!(
                thread.latest_sender.as_deref(),
                Some("@carol:example.invalid")
            );
            assert_eq!(thread.latest_sender_label.as_deref(), Some("Carol Alias"));
        }
        other => panic!("expected projected timeline event, got {other:?}"),
    }

    let key = TimelineKey {
        account_key: AccountKey("@me:example.invalid".to_owned()),
        kind: TimelineKind::Room {
            room_id: "!room:example.invalid".to_owned(),
        },
    };
    let _ = event_tx.send(CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
        key,
        generation: koushi_protocol::ids::TimelineGeneration(0),
        batch_id: koushi_protocol::ids::TimelineBatchId(1),
        diffs: vec![TimelineDiff::PushBack {
            item: TimelineItem {
                request_state: None,
                id: TimelineItemId::Event {
                    event_id: "$later:example.invalid".to_owned(),
                },
                sender: Some("@room-only:example.invalid".to_owned()),
                sender_label: Some("Room-only Person".to_owned()),
                sender_avatar: None,
                body: Some("later".to_owned()),
                notice_i18n: None,
                message_kind: Default::default(),
                spoiler_spans: Vec::new(),
                timestamp_ms: Some(3),
                in_reply_to_event_id: None,
                formatted: None,
                reply_quote: None,
                thread_root: None,
                thread_summary: None,
                media: None,
                link_previews: None,
                link_ranges: Vec::new(),
                reactions: Vec::new(),
                can_react: false,
                is_redacted: false,
                is_hidden: false,
                can_redact: false,
                is_edited: false,
                can_edit: false,
                actions: Default::default(),
                send_state: None,
                unable_to_decrypt: None,
                display_metadata: None,
            },
        }],
    }));

    match connection.recv_event().await.expect("timeline diff event") {
        CoreEvent::Timeline(TimelineEvent::ItemsUpdated { diffs, .. }) => {
            let TimelineDiff::PushBack { item } = diffs.first().expect("projected diff item")
            else {
                panic!("expected push-back diff");
            };
            assert_eq!(item.sender.as_deref(), Some("@room-only:example.invalid"));
            assert_eq!(item.sender_label.as_deref(), Some("Room-only Person"));
        }
        other => panic!("expected projected timeline diff event, got {other:?}"),
    }
}
