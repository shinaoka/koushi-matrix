//! AccountActor production-path tests for account notification settings (#981)
//! against a mock homeserver.

use koushi_protocol::{
    command::{AccountCommand, AccountNotificationsRequest},
    ids::{RequestId, RuntimeConnectionId},
};
use koushi_state::{
    AccountNotificationsFailureKind, AccountNotificationsOperation, AppAction, AuthSecret,
    IdentityResetAuthRequest, NotificationCategory, NotificationCategoryState, SessionInfo,
};
use matrix_sdk::{
    ruma::{
        push::{RuleKind, Ruleset},
        user_id,
    },
    test_utils::mocks::MatrixMockServer,
};
use serde_json::json;
use tempfile::tempdir;
use tokio::time::{Duration, timeout};
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{body_partial_json, method, path, path_regex},
};

use super::{
    actor::{AccountActorHandle, AccountMessage},
    test_support::{shutdown_and_ack, spawn_actor_with_dirs},
};

async fn install_session(server: &MatrixMockServer, handle: &AccountActorHandle) {
    let client = server.client_builder().build().await;
    let session = koushi_sdk::MatrixClientSession::from_client_for_testing(
        client.clone(),
        SessionInfo {
            homeserver: server.uri(),
            user_id: client.user_id().expect("mock user id").to_string(),
            device_id: client.device_id().expect("mock device id").to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Password,
        },
    );
    assert!(
        handle
            .install_residency_test_session(std::sync::Arc::new(session))
            .await
    );
}

fn request(sequence: u64) -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(3),
        sequence,
    }
}

async fn send(
    handle: &AccountActorHandle,
    request_id: RequestId,
    request: AccountNotificationsRequest,
) {
    handle
        .send(AccountMessage::Command(
            AccountCommand::AccountNotifications {
                request_id,
                request,
            },
        ))
        .await;
}

async fn next_notifications_action(
    action_rx: &mut tokio::sync::mpsc::Receiver<Vec<AppAction>>,
) -> AppAction {
    loop {
        let actions = timeout(Duration::from_secs(5), action_rx.recv())
            .await
            .expect("notifications action in time")
            .expect("action channel open");
        for action in actions {
            if matches!(
                action,
                AppAction::AccountNotificationsLoaded { .. }
                    | AppAction::AccountNotificationsLoadFailed { .. }
                    | AppAction::AccountNotificationsEmailTokenSent { .. }
                    | AppAction::AccountNotificationsUiaRequired { .. }
                    | AppAction::AccountNotificationsOperationSucceeded { .. }
                    | AppAction::AccountNotificationsOperationFailed { .. }
            ) {
                return action;
            }
        }
    }
}

fn defaults() -> Ruleset {
    Ruleset::server_default(user_id!("@example:localhost"))
}

async fn mount_reads(
    server: &MatrixMockServer,
    rules: serde_json::Value,
    threepids: serde_json::Value,
    pushers: serde_json::Value,
) {
    Mock::given(method("GET"))
        .and(path("/_matrix/client/v3/pushrules/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"global": rules})))
        .mount(server.server())
        .await;
    Mock::given(method("GET"))
        .and(path("/_matrix/client/v3/account/3pid"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"threepids": threepids})))
        .mount(server.server())
        .await;
    Mock::given(method("GET"))
        .and(path("/_matrix/client/v3/pushers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"pushers": pushers})))
        .mount(server.server())
        .await;
    Mock::given(method("GET"))
        .and(path("/_matrix/client/v3/capabilities"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"capabilities": {}})))
        .mount(server.server())
        .await;
}

fn email_pusher(address: &str) -> serde_json::Value {
    json!({"kind": "email", "app_id": "m.email", "pushkey": address,
           "app_display_name": "Email Notifications", "device_display_name": address,
           "lang": "en", "data": {}})
}

#[tokio::test]
async fn load_command_is_read_only_and_projects_mixed_state() {
    let server = MatrixMockServer::new().await;
    let mut ruleset = defaults();
    ruleset
        .set_actions(RuleKind::Underride, ".m.rule.encrypted", Vec::new())
        .unwrap();
    for verb in ["PUT", "DELETE", "POST"] {
        Mock::given(method(verb))
            .and(path_regex(
                r"^/_matrix/client/v3/(pushrules|pushers|account/3pid).*",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(0)
            .mount(server.server())
            .await;
    }
    mount_reads(
        &server,
        serde_json::to_value(&ruleset).unwrap(),
        json!([]),
        json!([]),
    )
    .await;
    let cred_dir = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let (handle, mut action_rx, _event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    install_session(&server, &handle).await;

    send(&handle, request(1), AccountNotificationsRequest::Load).await;
    match next_notifications_action(&mut action_rx).await {
        AppAction::AccountNotificationsLoaded {
            request_id,
            snapshot,
        } => {
            assert_eq!(request_id, 1);
            assert_eq!(
                snapshot.categories.group_messages,
                NotificationCategoryState::Mixed
            );
            assert!(!snapshot.email_notifications_active());
        }
        other => panic!("unexpected {other:?}"),
    }
    server.server().verify().await;
    shutdown_and_ack(&handle).await;
}

#[tokio::test]
async fn email_verification_with_uia_moves_an_active_target_to_the_new_address() {
    let server = MatrixMockServer::new().await;
    // Before confirmation: old@ is the only validated email and has a pusher.
    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/account/3pid/email/requestToken"))
        .and(body_partial_json(
            json!({"email": "new@example.invalid", "send_attempt": 1}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"sid": "sid1"})))
        .expect(1)
        .mount(server.server())
        .await;
    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/account/3pid/email/requestToken"))
        .and(body_partial_json(
            json!({"email": "new@example.invalid", "send_attempt": 2}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"sid": "sid1"})))
        .expect(1)
        .mount(server.server())
        .await;
    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/account/3pid/add"))
        .and(body_partial_json(
            json!({"auth": {"type": "m.login.password", "session": "uia1"}}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(server.server())
        .await;
    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/account/3pid/add"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "session": "uia1", "flows": [{"stages": ["m.login.password"]}], "params": {}
        })))
        .expect(1)
        .mount(server.server())
        .await;
    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/pushers/set"))
        .and(body_partial_json(
            json!({"kind": "email", "pushkey": "new@example.invalid", "lang": "ja"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(server.server())
        .await;
    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/pushers/set"))
        .and(body_partial_json(
            json!({"kind": null, "pushkey": "old@example.invalid"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(server.server())
        .await;
    // The mock returns both addresses as validated once confirmed; before
    // confirmation the actor only reads pushers and 3PIDs during carry-over.
    mount_reads(
        &server,
        serde_json::to_value(defaults()).unwrap(),
        json!([
            {"medium": "email", "address": "old@example.invalid", "validated_at": 1, "added_at": 1},
            {"medium": "email", "address": "new@example.invalid", "validated_at": 2, "added_at": 2}
        ]),
        json!([email_pusher("old@example.invalid")]),
    )
    .await;

    let cred_dir = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let (handle, mut action_rx, _event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    install_session(&server, &handle).await;

    send(
        &handle,
        request(10),
        AccountNotificationsRequest::RequestEmailToken {
            address: " New@Example.invalid ".to_owned(),
            lang: "ja".to_owned(),
        },
    )
    .await;
    match next_notifications_action(&mut action_rx).await {
        AppAction::AccountNotificationsEmailTokenSent {
            request_id,
            address,
            resend_count,
            ..
        } => {
            assert_eq!(request_id, 10);
            assert_eq!(address, "new@example.invalid");
            assert_eq!(resend_count, 0);
        }
        other => panic!("unexpected {other:?}"),
    }

    send(
        &handle,
        request(11),
        AccountNotificationsRequest::ResendEmailToken,
    )
    .await;
    assert!(matches!(
        next_notifications_action(&mut action_rx).await,
        AppAction::AccountNotificationsEmailTokenSent {
            resend_count: 1,
            ..
        }
    ));

    send(
        &handle,
        request(12),
        AccountNotificationsRequest::ConfirmEmail,
    )
    .await;
    assert!(matches!(
        next_notifications_action(&mut action_rx).await,
        AppAction::AccountNotificationsUiaRequired {
            request_id: 12,
            flow_id: 12,
            operation: AccountNotificationsOperation::ConfirmEmail
        }
    ));

    // A stale flow id is rejected without contacting the server.
    send(
        &handle,
        request(13),
        AccountNotificationsRequest::SubmitUia {
            flow_id: 99,
            auth: IdentityResetAuthRequest::UiaaPassword {
                password: AuthSecret::new("synthetic-password".to_owned()),
            },
        },
    )
    .await;
    send(
        &handle,
        request(14),
        AccountNotificationsRequest::SubmitUia {
            flow_id: 12,
            auth: IdentityResetAuthRequest::UiaaPassword {
                password: AuthSecret::new("synthetic-password".to_owned()),
            },
        },
    )
    .await;
    match next_notifications_action(&mut action_rx).await {
        AppAction::AccountNotificationsOperationSucceeded {
            request_id,
            operation,
            snapshot,
        } => {
            assert_eq!(request_id, 12, "settles the original confirm request");
            assert_eq!(operation, AccountNotificationsOperation::ConfirmEmail);
            assert!(snapshot.is_some());
        }
        other => panic!("unexpected {other:?}"),
    }
    server.server().verify().await;
    shutdown_and_ack(&handle).await;
}

#[tokio::test]
async fn rejected_email_pusher_settles_failed_with_the_server_state() {
    let server = MatrixMockServer::new().await;
    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/pushers/set"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "errcode": "M_THREEPID_NOT_FOUND", "error": "Email not found"
        })))
        .mount(server.server())
        .await;
    mount_reads(
        &server,
        serde_json::to_value(defaults()).unwrap(),
        json!([{"medium": "email", "address": "one@example.invalid", "validated_at": 1, "added_at": 1}]),
        json!([]),
    )
    .await;
    let cred_dir = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let (handle, mut action_rx, _event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    install_session(&server, &handle).await;

    send(
        &handle,
        request(20),
        AccountNotificationsRequest::EnableEmailNotifications {
            address: "one@example.invalid".to_owned(),
            lang: "en".to_owned(),
        },
    )
    .await;
    match next_notifications_action(&mut action_rx).await {
        AppAction::AccountNotificationsOperationFailed {
            failure_kind,
            snapshot,
            ..
        } => {
            assert_eq!(
                failure_kind,
                AccountNotificationsFailureKind::EmailNotRegistered
            );
            assert!(!snapshot.expect("re-read").email_notifications_active());
        }
        other => panic!("unexpected {other:?}"),
    }

    send(
        &handle,
        request(21),
        AccountNotificationsRequest::SetCategory {
            category: NotificationCategory::Invites,
            enabled: true,
        },
    )
    .await;
    // Invites are already ON: the toggle plans no writes and succeeds.
    assert!(matches!(
        next_notifications_action(&mut action_rx).await,
        AppAction::AccountNotificationsOperationSucceeded { request_id: 21, .. }
    ));
    shutdown_and_ack(&handle).await;
}
