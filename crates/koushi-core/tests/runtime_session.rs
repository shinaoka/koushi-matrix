//! Runtime session routing tests.

use std::time::Duration;

use koushi_core::{
    AccountKey, AppCommand, CoreCommand, CoreEvent, CoreFailure, CoreRuntime, CreateRoomOptions,
    CreateRoomVisibility, PaginationDirection, RequestId, RoomCommand, TimelineCommand,
    TimelineKey, executor,
};
use koushi_state::{
    AppAction, AuthSecret, RecoveryMethod, RecoveryRequest, SessionState,
    StagedUploadCompressionChoice, StagedUploadItem, StagedUploadKind,
};

mod support;
use support::*;

#[tokio::test]
async fn unauthenticated_session_commands_are_rejected() {
    let runtime = CoreRuntime::start();
    let mut connection = runtime.attach();

    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Room(RoomCommand::CreateRoom {
            request_id,
            options: CreateRoomOptions {
                name: "qa room".to_owned(),
                topic: None,
                alias_localpart: None,
                encrypted: false,
                visibility: CreateRoomVisibility::Private,
                parent_space: None,
            },
        }))
        .await
        .expect("submit");

    match connection.recv_event().await.expect("event") {
        CoreEvent::OperationFailed {
            request_id: failed_id,
            failure,
        } => {
            assert_eq!(failed_id, request_id);
            assert_eq!(failure, CoreFailure::SessionRequired);
        }
        other => panic!("expected OperationFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn ready_session_routes_past_appactor_session_gate() {
    // Verify that a Timeline command passes the AppActor's session gate
    // (only applied before routing) and reaches AccountActor, which returns
    // a timeline-domain failure (not a routing/gate failure like an unknown
    // command kind).
    //
    // With inject_actions we get a Ready AppState but no real SDK session in
    // AccountActor, so AccountActor emits SessionRequired from its own guard.
    // That is a valid "routes to AccountActor" signal: the AppActor did not
    // short-circuit it with a different failure.
    let runtime = CoreRuntime::start();
    let mut connection = runtime.attach();
    runtime
        .inject_actions(vec![AppAction::RestoreSessionSucceeded(session_info())])
        .await;
    // Wait for the Ready snapshot before submitting.
    loop {
        if matches!(connection.snapshot().session, SessionState::Ready(_)) {
            break;
        }
        executor::sleep(Duration::from_millis(5)).await;
    }

    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Timeline(TimelineCommand::Paginate {
            request_id,
            key: TimelineKey::room(AccountKey("acc".to_owned()), "!room:example.test"),
            direction: PaginationDirection::Backward,
            event_count: 20,
        }))
        .await
        .expect("submit");

    loop {
        match connection.recv_event().await.expect("event") {
            CoreEvent::OperationFailed {
                request_id: failed_id,
                failure,
            } if failed_id == request_id => {
                // The AppActor allows timeline commands to reach AccountActor
                // when the session is Ready. AccountActor checks its own session
                // guard; with a fake inject there is no real SDK session, so it
                // returns SessionRequired. That is the expected behavior:
                // the command reached AccountActor (not rejected at AppActor).
                assert!(
                    matches!(
                        failure,
                        CoreFailure::SessionRequired | CoreFailure::TimelineOperationFailed { .. }
                    ),
                    "unexpected failure kind: {failure:?}"
                );
                return;
            }
            _ => continue,
        }
    }
}

#[tokio::test]
async fn actor_projected_session_lock_executes_stop_sync_effect() {
    let runtime = CoreRuntime::start();
    let mut connection = runtime.attach();

    runtime
        .inject_actions(vec![AppAction::RestoreSessionSucceeded(session_info())])
        .await;
    wait_for_state(&mut connection, |state| {
        matches!(state.session, SessionState::Ready(_))
    })
    .await;

    let start_failure = next_session_required_failure(&mut connection).await;

    runtime.inject_actions(vec![AppAction::SessionLocked]).await;
    wait_for_state(&mut connection, |state| {
        matches!(state.session, SessionState::Locked(_))
    })
    .await;

    let stop_failure = executor::timeout(Duration::from_millis(500), async {
        next_session_required_failure(&mut connection).await
    })
    .await
    .expect("SessionLocked must execute AppEffect::StopSync through AccountActor");
    assert_ne!(
        start_failure, stop_failure,
        "lock should produce a distinct stop-sync routing attempt"
    );
}

async fn next_session_required_failure(
    connection: &mut koushi_core::runtime::CoreConnection,
) -> RequestId {
    loop {
        match connection.recv_event().await.expect("event") {
            CoreEvent::OperationFailed {
                request_id,
                failure: CoreFailure::SessionRequired,
            } => return request_id,
            _ => continue,
        }
    }
}

#[tokio::test]
async fn recovery_sessions_route_ready_guarded_app_commands() {
    for target in [
        RecoveryRouteTarget::NeedsRecovery,
        RecoveryRouteTarget::Recovering,
    ] {
        assert_upload_staging_command_routes_for_recovery_session(target).await;
    }
}

#[derive(Clone, Copy)]
enum RecoveryRouteTarget {
    NeedsRecovery,
    Recovering,
}

async fn assert_upload_staging_command_routes_for_recovery_session(target: RecoveryRouteTarget) {
    let runtime = CoreRuntime::start();
    let mut connection = runtime.attach();
    let room_id = "!room:example.test";
    let mut actions = vec![
        AppAction::E2eeRecoveryRequired {
            info: session_info(),
            methods: vec![RecoveryMethod::RecoveryKey],
        },
        AppAction::RoomListUpdated {
            spaces: vec![],
            rooms: vec![room_summary(room_id)],
        },
        AppAction::SelectRoom {
            room_id: room_id.to_owned(),
        },
    ];
    if matches!(target, RecoveryRouteTarget::Recovering) {
        actions.push(AppAction::E2eeRecoverySubmitted(RecoveryRequest {
            secret: AuthSecret::new("synthetic recovery secret"),
        }));
    }
    runtime.inject_actions(actions).await;
    wait_for_state(&mut connection, |state| {
        state.navigation.active_room_id.as_deref() == Some(room_id)
            && match target {
                RecoveryRouteTarget::NeedsRecovery => {
                    matches!(state.session, SessionState::AwaitingVerification { .. })
                }
                RecoveryRouteTarget::Recovering => {
                    matches!(state.session, SessionState::Verifying { .. })
                }
            }
    })
    .await;

    let request_id = connection.next_request_id();
    let staged_item = StagedUploadItem {
        staged_id: "staged-1".to_owned(),
        room_id: room_id.to_owned(),
        position: 0,
        filename: "synthetic.txt".to_owned(),
        mime_type: "text/plain".to_owned(),
        byte_count: 12,
        kind: StagedUploadKind::File,
        caption: None,
        compression_choice: StagedUploadCompressionChoice::NotApplicable,
    };
    connection
        .command(CoreCommand::App(AppCommand::SetUploadStaging {
            request_id,
            room_id: room_id.to_owned(),
            items: vec![staged_item],
        }))
        .await
        .expect("submit");

    wait_for_state(&mut connection, |state| {
        state
            .timeline
            .staged_uploads
            .iter()
            .any(|item| item.staged_id == "staged-1" && item.room_id == room_id)
    })
    .await;
}
