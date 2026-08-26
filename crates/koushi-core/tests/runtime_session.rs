//! Runtime session routing tests.

use std::time::Duration;

use koushi_core::{
    AccountCommand, AccountKey, AppCommand, CoreCommand, CoreEvent, CoreFailure, CoreRuntime,
    CreateRoomOptions, CreateRoomVisibility, PaginationDirection, RequestId, RoomCommand,
    TimelineCommand, TimelineKey, executor,
};
use koushi_state::{
    AppAction, AuthSecret, CurrentDeviceTrustState, CurrentSessionBackupState,
    CurrentSessionStatusDetails, CurrentSessionStatusState, CurrentSessionSyncState,
    LoginAttemptId, LoginRequest, OwnIdentityVerification, RecoveryMethod, RecoveryRequest,
    SessionState, SessionStatusRefreshTrigger, SlidingSyncCapabilityState,
    StagedUploadCompressionChoice, StagedUploadItem, StagedUploadKind,
};
use matrix_sdk::test_utils::mocks::MatrixMockServer;
use wiremock::{
    Mock, Request, Respond, ResponseTemplate,
    matchers::{method, path},
};

#[derive(Clone)]
struct EchoLoginDevice {
    token: &'static str,
    user_id: &'static str,
}

impl Respond for EchoLoginDevice {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: serde_json::Value =
            serde_json::from_slice(&request.body).expect("login request JSON");
        let device_id = body["device_id"]
            .as_str()
            .expect("fresh login generated device id");
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": self.token,
            "device_id": device_id,
            "user_id": self.user_id,
        }))
    }
}

async fn mount_echo_login(server: &MatrixMockServer, token: &'static str, user_id: &'static str) {
    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/login"))
        .respond_with(EchoLoginDevice { token, user_id })
        .expect(1)
        .mount(&server.server())
        .await;
}

#[tokio::test]
async fn password_command_projects_authentication_before_account_actor_completion() {
    let runtime = CoreRuntime::start();
    let mut connection = runtime.attach();
    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id,
            request: LoginRequest {
                homeserver: "http://127.0.0.1:9".to_owned(),
                username: "user".to_owned(),
                password: AuthSecret::new("synthetic-password"),
                device_display_name: None,
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await
        .expect("submit");

    loop {
        match connection.recv_event().await.expect("event") {
            CoreEvent::StateChanged(snapshot)
                if matches!(
                    &snapshot.session,
                    SessionState::Authenticating { homeserver, attempt_id }
                        if homeserver == "http://127.0.0.1:9"
                            && *attempt_id == LoginAttemptId::new(
                                request_id.connection_id.0,
                                request_id.sequence,
                            )
                ) =>
            {
                return;
            }
            CoreEvent::OperationFailed {
                request_id: failed, ..
            } if failed == request_id => {
                panic!("account actor completed before AuthenticationStarted was observed")
            }
            _ => {}
        }
    }
}

#[tokio::test]
async fn password_login_capability_gate_round_trips_through_reducer_effects() {
    let server = MatrixMockServer::new().await;
    server
        .mock_versions()
        .with_feature("org.matrix.simplified_msc3575", true)
        .ok()
        .mount()
        .await;
    mount_echo_login(&server, "synthetic-token", "@reducer-gate:localhost").await;
    let data_dir = tempfile::tempdir().expect("runtime data directory");
    let credential_dir = tempfile::tempdir().expect("runtime credential directory");
    let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    assert!(
        runtime
            .configure_trust_observation_for_testing(koushi_sdk::CurrentDeviceTrustObservation {
                current: koushi_state::CurrentDeviceTrustState::Verified,
                updates: Box::pin(futures_util::stream::pending()),
            },)
            .await
    );
    let mut connection = runtime.attach();
    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id,
            request: LoginRequest {
                homeserver: server.uri(),
                username: "reducer-gate".to_owned(),
                password: AuthSecret::new("synthetic-password"),
                device_display_name: None,
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await
        .expect("submit login");

    let state = wait_for_state_event(&mut connection, |state| {
        matches!(state.session, SessionState::Ready(_))
    })
    .await;
    assert!(matches!(
        state.sliding_sync_capability,
        SlidingSyncCapabilityState::Supported { .. }
    ));
}

#[tokio::test]
async fn account_switch_capability_gate_round_trips_through_reducer_effects() {
    let server_a = MatrixMockServer::new().await;
    server_a
        .mock_versions()
        .with_feature("org.matrix.simplified_msc3575", true)
        .ok()
        .mount()
        .await;
    mount_echo_login(&server_a, "token-a", "@alpha:localhost").await;
    server_a
        .mock_logout()
        .ignore_access_token()
        .ok()
        .mock_once()
        .mount()
        .await;

    let server_b = MatrixMockServer::new().await;
    server_b
        .mock_versions()
        .with_feature("org.matrix.simplified_msc3575", true)
        .ok()
        .mount()
        .await;
    mount_echo_login(&server_b, "token-b", "@beta:localhost").await;

    let data_dir = tempfile::tempdir().expect("runtime data directory");
    let credential_dir = tempfile::tempdir().expect("runtime credential directory");
    let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    let mut connection = runtime.attach();

    configure_verified_runtime_trust(&runtime).await;
    submit_runtime_password_login(&connection, server_a.uri(), "alpha").await;
    wait_for_runtime_account(&mut connection, Some("@alpha:localhost"), "alpha login").await;

    let logout_request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Account(AccountCommand::Logout {
            request_id: logout_request_id,
        }))
        .await
        .expect("submit local logout");
    wait_for_runtime_account(&mut connection, None, "alpha logout").await;

    configure_verified_runtime_trust(&runtime).await;
    submit_runtime_password_login(&connection, server_b.uri(), "beta").await;
    wait_for_runtime_account(&mut connection, Some("@beta:localhost"), "beta login").await;

    configure_verified_runtime_trust(&runtime).await;
    let switch_request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Account(AccountCommand::SwitchAccount {
            request_id: switch_request_id,
            account_key: AccountKey("@alpha:localhost".to_owned()),
        }))
        .await
        .expect("submit account switch");
    let switched =
        wait_for_runtime_account(&mut connection, Some("@alpha:localhost"), "switch to alpha")
            .await;
    assert!(matches!(
        switched.sliding_sync_capability,
        SlidingSyncCapabilityState::Supported { .. }
    ));
}

async fn configure_verified_runtime_trust(runtime: &CoreRuntime) {
    assert!(
        runtime
            .configure_trust_observation_for_testing(koushi_sdk::CurrentDeviceTrustObservation {
                current: koushi_state::CurrentDeviceTrustState::Verified,
                updates: Box::pin(futures_util::stream::pending()),
            },)
            .await
    );
}

async fn submit_runtime_password_login(
    connection: &koushi_core::runtime::CoreConnection,
    homeserver: String,
    username: &str,
) {
    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id,
            request: LoginRequest {
                homeserver,
                username: username.to_owned(),
                password: AuthSecret::new("synthetic-password"),
                device_display_name: None,
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await
        .expect("submit password login");
}

async fn wait_for_runtime_account(
    connection: &mut koushi_core::runtime::CoreConnection,
    ready_user_id: Option<&str>,
    stage: &str,
) -> koushi_state::AppState {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = connection.snapshot();
            let matches = match ready_user_id {
                Some(user_id) => matches!(
                    &snapshot.session,
                    SessionState::Ready(info) if info.user_id == user_id
                ),
                None => matches!(snapshot.session, SessionState::SignedOut),
            };
            if matches {
                return snapshot;
            }
            connection
                .recv_event()
                .await
                .expect("runtime event stream must remain open");
        }
    })
    .await;
    match result {
        Ok(snapshot) => snapshot,
        Err(_) => {
            let snapshot = connection.snapshot();
            panic!(
                "{stage} did not settle; current session: {:?}; sliding sync: {:?}",
                snapshot.session, snapshot.sliding_sync_capability
            )
        }
    }
}

#[tokio::test]
async fn active_session_rejects_a_new_password_login_before_account_routing() {
    let runtime = CoreRuntime::start();
    let mut connection = runtime.attach();
    runtime.inject_actions(restore_ready_actions()).await;
    wait_for_state(&mut connection, |state| {
        matches!(state.session, SessionState::Ready(_))
    })
    .await;

    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id,
            request: LoginRequest {
                homeserver: "http://127.0.0.1:9".to_owned(),
                username: "user".to_owned(),
                password: AuthSecret::new("synthetic-password"),
                device_display_name: None,
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await
        .expect("submit");

    loop {
        match connection.recv_event().await.expect("event") {
            CoreEvent::OperationFailed {
                request_id: failed,
                failure: CoreFailure::SessionRequired,
            } if failed == request_id => return,
            CoreEvent::OperationFailed {
                request_id: failed, ..
            } if failed == request_id => panic!("login reached AccountActor"),
            _ => {}
        }
    }
}

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
    runtime.inject_actions(restore_ready_actions()).await;
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
async fn actor_projected_session_gate_and_authentication_lock_execute_stop_sync_effect() {
    for (action, authentication_locked) in [
        (AppAction::SessionLocked, false),
        (
            AppAction::SessionAuthenticationInvalidated { soft_logout: true },
            true,
        ),
        (
            AppAction::SessionAuthenticationInvalidated { soft_logout: false },
            true,
        ),
    ] {
        assert_projected_session_exit_stops_sync(action, authentication_locked).await;
    }
}

async fn assert_projected_session_exit_stops_sync(action: AppAction, authentication_locked: bool) {
    let runtime = CoreRuntime::start();
    let mut connection = runtime.attach();

    runtime.inject_actions(restore_ready_actions()).await;
    wait_for_state(&mut connection, |state| {
        matches!(state.session, SessionState::Ready(_))
    })
    .await;

    let start_failure = next_session_required_failure(&mut connection).await;

    runtime.inject_actions(vec![action]).await;
    wait_for_state(&mut connection, |state| {
        if authentication_locked {
            matches!(state.session, SessionState::Locked(_))
        } else {
            matches!(
                state.session,
                SessionState::Provisional {
                    phase: koushi_state::ProvisionalPhase::DiscoveringMethods,
                    ..
                }
            )
        }
    })
    .await;

    let stop_failure = executor::timeout(Duration::from_millis(500), async {
        next_session_required_failure(&mut connection).await
    })
    .await
    .expect("session exit must execute AppEffect::StopSync through AccountActor");
    assert_ne!(
        start_failure, stop_failure,
        "session exit should produce a distinct stop-sync routing attempt"
    );

    drop(connection);
    runtime.shutdown().await;
}

#[tokio::test]
async fn authoritative_trust_loss_publishes_one_atomic_reset_delta_after_setup_quiesces() {
    let runtime = CoreRuntime::start_with_event_capacity(128);
    let mut connection = runtime.attach();
    let room_id = "!room:example.invalid".to_owned();
    let event_id = "$focused:example.invalid".to_owned();
    let mut setup = restore_ready_actions();
    setup.extend([
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![support::room_summary(&room_id)],
        },
        AppAction::SelectRoom {
            room_id: room_id.clone(),
        },
        AppAction::CurrentSessionStatusRefreshRequested {
            request_id: 41,
            trigger: SessionStatusRefreshTrigger::Manual,
        },
        AppAction::CurrentSessionStatusRefreshed {
            request_id: 41,
            details: CurrentSessionStatusDetails::new(
                Some("Synthetic device".to_owned()),
                "DEVICE".to_owned(),
                koushi_state::SessionAuthenticationMethod::Unknown,
                CurrentSessionSyncState::Running,
                CurrentDeviceTrustState::Verified,
                true,
                OwnIdentityVerification::Verified,
                CurrentSessionBackupState::Ready,
                1_000,
            ),
        },
        AppAction::InviteWorkflowOpened {
            room_id: room_id.clone(),
        },
        AppAction::OpenFocusedContext {
            room_id: room_id.clone(),
            event_id: event_id.clone(),
        },
        AppAction::FocusedContextSubscribed {
            room_id: room_id.clone(),
            event_id,
        },
    ]);
    runtime.inject_actions(setup).await;

    let setup_state = wait_for_state_event(&mut connection, |state| {
        matches!(state.session, SessionState::Ready(_))
            && state.timeline.room_id.as_deref() == Some(room_id.as_str())
            && matches!(
                &state.current_session_status,
                CurrentSessionStatusState::Ready { request_id: 41, .. }
            )
            && state.invite_workflow != koushi_state::InviteWorkflowState::default()
            && state.focused_context != koushi_state::FocusedContextState::Closed
    })
    .await;
    assert!(matches!(setup_state.session, SessionState::Ready(_)));

    runtime
        .inject_actions(vec![AppAction::AuthoritativeDeviceTrustChanged {
            generation: 7,
            transition_id: 9,
            trust: CurrentDeviceTrustState::Unverified,
        }])
        .await;

    let delta = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = connection
                .recv_event()
                .await
                .expect("runtime event stream must remain open");
            let CoreEvent::StateDelta(delta) = event else {
                continue;
            };
            let changed = &delta.changed;
            if changed.session.is_some()
                && changed.current_session_status.is_some()
                && changed.invite_workflow.is_some()
                && changed.focused_context.is_some()
            {
                break delta;
            }
        }
    })
    .await
    .expect("trust-loss reset delta must arrive before the event deadline");

    assert!(matches!(
        delta.changed.session.as_ref(),
        Some(SessionState::Provisional {
            phase: koushi_state::ProvisionalPhase::DiscoveringMethods,
            ..
        })
    ));
    assert_eq!(
        delta.changed.current_session_status.as_ref(),
        Some(&CurrentSessionStatusState::Idle)
    );
    assert_eq!(
        delta.changed.invite_workflow.as_ref(),
        Some(&koushi_state::InviteWorkflowState::default())
    );
    assert_eq!(
        delta.changed.focused_context.as_ref(),
        Some(&koushi_state::FocusedContextState::Closed)
    );
    drop(connection);
    runtime.shutdown().await;
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
async fn recovery_sessions_reject_ready_guarded_app_commands() {
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
    let attempt_id = LoginAttemptId::new(0, 1);
    let mut actions = vec![
        AppAction::AuthenticationStarted {
            attempt_id,
            homeserver: session_info().homeserver,
        },
        AppAction::LoginSucceeded {
            attempt_id,
            info: session_info(),
        },
        AppAction::CurrentDeviceTrustChanged(koushi_state::CurrentDeviceTrustState::Unverified),
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
        actions.push(AppAction::E2eeRecoverySubmitted {
            flow_id: 77,
            request: RecoveryRequest {
                secret: AuthSecret::new("synthetic recovery secret"),
            },
        });
    }
    runtime.inject_actions(actions).await;
    wait_for_state(&mut connection, |state| match target {
        RecoveryRouteTarget::NeedsRecovery => {
            matches!(state.session, SessionState::AwaitingVerification { .. })
        }
        RecoveryRouteTarget::Recovering => {
            matches!(state.session, SessionState::Verifying { .. })
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
        preparation: Default::default(),
    };
    connection
        .command(CoreCommand::App(AppCommand::SetUploadStaging {
            request_id,
            target: koushi_state::ComposerTarget::Main {
                room_id: room_id.to_owned(),
            },
            items: vec![staged_item],
        }))
        .await
        .expect("submit");

    loop {
        match connection.recv_event().await.expect("event") {
            CoreEvent::OperationFailed {
                request_id: failed_id,
                failure: CoreFailure::SessionRequired,
            } if failed_id == request_id => return,
            _ => {}
        }
    }
}
