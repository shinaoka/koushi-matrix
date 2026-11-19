use std::{collections::BTreeSet, sync::Arc};

use koushi_protocol::SessionKeyId;
use koushi_sdk::MatrixClientSession;
use koushi_state::{AppAction, ScheduledSendHandle, ScheduledSendItem, SessionInfo};

use tokio::sync::{mpsc, oneshot};

use super::{
    AuthoritativeRoomEncryption, build_scheduled_message_content,
    scheduled_dispatch_targets_active_session, send_scheduled_acceptance_actions,
    server_delayed_events_are_safe, user_content_is_admitted,
};
use crate::account::actor::{AccountActorHandle, AccountMessage};
use crate::account::test_support::{
    recv_account_action_with_sliding_sync_effects, spawn_actor_with_dirs_and_registry,
};
use crate::composer_draft_lifecycle::ComposerDraftLeaseRegistry;
use koushi_protocol::command::AccountCommand;

use koushi_protocol::ids::{RequestId, RuntimeConnectionId};

use crate::composer_draft_lifecycle::ForwardedComposerDraftPermit;

use matrix_sdk::test_utils::mocks::MatrixMockServer;

use tempfile::tempdir;
use wiremock::ResponseTemplate;

#[test]
fn secure_backup_room_admission_fails_closed_without_authoritative_metadata() {
    assert!(!user_content_is_admitted(
        AuthoritativeRoomEncryption::Unknown
    ));
}

#[test]
fn encrypted_room_admission_does_not_wait_for_secure_backup() {
    assert!(user_content_is_admitted(
        AuthoritativeRoomEncryption::Unencrypted
    ));
    assert!(user_content_is_admitted(
        AuthoritativeRoomEncryption::Encrypted
    ));
}

#[test]
fn server_delayed_events_are_used_only_for_authoritatively_unencrypted_rooms() {
    assert!(server_delayed_events_are_safe(
        AuthoritativeRoomEncryption::Unencrypted
    ));
    assert!(!server_delayed_events_are_safe(
        AuthoritativeRoomEncryption::Encrypted
    ));
    assert!(!server_delayed_events_are_safe(
        AuthoritativeRoomEncryption::Unknown
    ));
}

#[tokio::test]
async fn scheduled_acceptance_retains_exact_permit_until_reducer_delivery() {
    let account = SessionKeyId {
        homeserver: "https://schedule.example.test".to_owned(),
        user_id: "@schedule:example.test".to_owned(),
        device_id: "SCHEDULE".to_owned(),
    };
    let target = koushi_state::ComposerTarget::Main {
        room_id: "!scheduled:example.test".to_owned(),
    };
    let scope = crate::composer_draft_lifecycle::ComposerDraftScope {
        account: account.clone(),
        target: target.clone(),
    };
    let registry = Arc::new(crate::composer_draft_lifecycle::ComposerDraftLeaseRegistry::new());
    let renderer_generation = registry
        .begin_renderer_generation()
        .expect("begin renderer generation");
    let lease_id = registry
        .acquire(renderer_generation, scope.clone())
        .expect("acquire scheduled-send lease");
    let command_permit = registry
        .try_command_permit(renderer_generation, lease_id, &scope)
        .expect("admit scheduled-send command");
    let app_pending_permit = command_permit.clone();
    registry
        .release(renderer_generation, lease_id)
        .expect("release activation after command admission");

    let request_id = RequestId {
        connection_id: RuntimeConnectionId(44),
        sequence: 9,
    };
    let (rejected_tx, mut rejected_rx) = mpsc::unbounded_channel();
    let (acceptance_probe_tx, acceptance_probe_rx) = oneshot::channel();
    let forwarded_permit = ForwardedComposerDraftPermit::new_with_acceptance_probe(
        request_id,
        command_permit,
        rejected_tx,
        acceptance_probe_tx,
    );
    let (action_tx, mut action_rx) = mpsc::channel(1);
    action_tx
        .try_send(Vec::new())
        .expect("fill reducer action lane");
    let send = tokio::spawn(async move {
        send_scheduled_acceptance_actions(
            &action_tx,
            vec![AppAction::ScheduledSendCreatedAtRevision {
                item: ScheduledSendItem {
                    scheduled_id: "scheduled-permit".to_owned(),
                    room_id: "!scheduled:example.test".to_owned(),
                    thread_root_event_id: None,
                    body: "synthetic body".to_owned(),
                    send_at_ms: 1,
                    handle: ScheduledSendHandle::Local,
                    is_dispatching: false,
                },
                draft_revision: 4.into(),
            }],
            forwarded_permit,
        )
        .await;
    });

    acceptance_probe_rx
        .await
        .expect("account schedule reached acceptance projection");
    assert_eq!(
        registry.protected_targets(&account),
        BTreeSet::from([target.clone()]),
        "the AccountActor schedule permit must protect the exact blocked target"
    );
    assert!(
        action_rx
            .recv()
            .await
            .expect("reducer lane marker")
            .is_empty(),
        "the first action only opens the deterministic reducer barrier"
    );
    let acceptance = action_rx.recv().await.expect("scheduled acceptance action");
    assert!(matches!(
        acceptance.as_slice(),
        [AppAction::ScheduledSendCreatedAtRevision { item, .. }]
            if item.scheduled_id == "scheduled-permit"
    ));
    send.await.expect("scheduled acceptance sender");
    assert_eq!(
        registry.protected_targets(&account),
        BTreeSet::from([target.clone()]),
        "the AppActor pending clone must outlive schedule acceptance enqueue"
    );

    let mut changes = registry.subscribe();
    changes.borrow_and_update();
    drop(app_pending_permit);
    changes
        .changed()
        .await
        .expect("matching reducer release notification");
    assert!(
        registry.protected_targets(&account).is_empty(),
        "the scheduled target becomes eligible only after reducer delivery"
    );
    assert!(
        rejected_rx.try_recv().is_err(),
        "successful scheduled acceptance must disarm rejection cleanup"
    );
}

#[tokio::test]
async fn stale_server_delayed_schedule_is_rejected_after_account_replacement() {
    let server = MatrixMockServer::new().await;
    server
        .mock_versions()
        .with_feature(crate::scheduled_send::MSC4140_FEATURE, true)
        .with_feature("org.matrix.simplified_msc3575", true)
        .ok()
        .mount()
        .await;
    server
        .mock_room_send()
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "delay_id": "synthetic-delay" })),
        )
        .mount()
        .await;

    let old_account = SessionKeyId {
        homeserver: server.uri(),
        user_id: "@old-schedule:localhost".to_owned(),
        device_id: "OLD-SCHEDULE".to_owned(),
    };
    let replacement_account = SessionKeyId {
        homeserver: server.uri(),
        user_id: "@replacement-schedule:localhost".to_owned(),
        device_id: "REPLACEMENT-SCHEDULE".to_owned(),
    };
    let old_session = MatrixClientSession::from_client_for_testing(
        server.client_builder().build().await,
        SessionInfo {
            homeserver: old_account.homeserver.clone(),
            user_id: old_account.user_id.clone(),
            device_id: old_account.device_id.clone(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    );
    let replacement_session = MatrixClientSession::from_client_for_testing(
        server.client_builder().build().await,
        SessionInfo {
            homeserver: replacement_account.homeserver.clone(),
            user_id: replacement_account.user_id.clone(),
            device_id: replacement_account.device_id.clone(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    );
    let credential_dir = tempdir().expect("credential tempdir");
    let data_dir = tempdir().expect("data tempdir");
    let registry = Arc::new(ComposerDraftLeaseRegistry::new());
    let (handle, mut action_rx, _event_rx) = spawn_actor_with_dirs_and_registry(
        credential_dir.path(),
        data_dir.path(),
        registry.clone(),
    );

    configure_synthetic_oidc_session(
        &handle,
        old_session,
        RequestId {
            connection_id: RuntimeConnectionId(51),
            sequence: 1,
        },
    )
    .await;
    loop {
        let actions = recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx).await;
        if matches!(
            actions.as_slice(),
            [AppAction::LoginSucceeded { info, .. }]
                if info.user_id == old_account.user_id
                    && info.device_id == old_account.device_id
        ) {
            break;
        }
    }

    let target = koushi_state::ComposerTarget::Main {
        room_id: "!scheduled-owner:localhost".to_owned(),
    };
    let scope = crate::composer_draft_lifecycle::ComposerDraftScope {
        account: old_account.clone(),
        target: target.clone(),
    };
    let renderer_generation = registry
        .begin_renderer_generation()
        .expect("begin old-account renderer generation");
    let lease_id = registry
        .acquire(renderer_generation, scope.clone())
        .expect("acquire old-account schedule lease");
    let command_permit = registry
        .try_command_permit(renderer_generation, lease_id, &scope)
        .expect("admit old-account schedule command");
    let app_pending_permit = command_permit.clone();
    registry
        .release(renderer_generation, lease_id)
        .expect("release activation after schedule admission");
    let request_id = RequestId {
        connection_id: RuntimeConnectionId(51),
        sequence: 3,
    };
    let (rejected_tx, mut rejected_rx) = mpsc::unbounded_channel();
    let forwarded_permit =
        ForwardedComposerDraftPermit::new(request_id, command_permit, rejected_tx);

    configure_synthetic_oidc_session(
        &handle,
        replacement_session,
        RequestId {
            connection_id: RuntimeConnectionId(51),
            sequence: 2,
        },
    )
    .await;
    loop {
        let actions = recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx).await;
        if matches!(
            actions.as_slice(),
            [AppAction::LoginSucceeded { info, .. }]
                if info.user_id == replacement_account.user_id
                    && info.device_id == replacement_account.device_id
        ) {
            break;
        }
    }
    assert!(
        handle
            .send(AccountMessage::ScheduleServerDelayedSend {
                request_id,
                expected_account: old_account.clone(),
                scheduled_id: "stale-server-schedule".to_owned(),
                room_id: "!scheduled-owner:localhost".to_owned(),
                thread_root_event_id: None,
                body: "synthetic stale scheduled body".to_owned(),
                send_at_ms: crate::time::current_epoch_ms().saturating_add(60_000),
                draft_revision: 4.into(),
                composer_permit: forwarded_permit,
                formatting_options: koushi_state::ComposerFormattingOptions::default(),
            })
            .await
    );
    let (barrier_tx, barrier_rx) = oneshot::channel();
    assert!(
        handle
            .send(AccountMessage::InspectSessionRuntime {
                response: barrier_tx,
            })
            .await
    );
    barrier_rx
        .await
        .expect("account actor mailbox barrier after stale schedule");

    let delayed_request_count = server
        .received_requests()
        .await
        .expect("recorded Matrix requests")
        .into_iter()
        .filter(|request| {
            request
                .url
                .query_pairs()
                .any(|(key, _)| key == "org.matrix.msc4140.delay")
        })
        .count();
    assert_eq!(
        delayed_request_count, 0,
        "an old-account schedule must not reach the replacement account SDK"
    );
    let queued_actions = std::iter::from_fn(|| action_rx.try_recv().ok())
        .flatten()
        .collect::<Vec<_>>();
    assert!(
        !queued_actions.iter().any(|action| matches!(
            action,
            AppAction::ScheduledSendCapabilityChanged { .. }
                | AppAction::ScheduledSendCreatedAtRevision { .. }
        )),
        "an old-account schedule must not enqueue an acceptance for the replacement account"
    );
    assert_eq!(
        rejected_rx.try_recv(),
        Ok(request_id),
        "the rejected schedule must settle AppActor's pending permit"
    );
    assert_eq!(
        registry.protected_targets(&old_account),
        BTreeSet::from([target]),
        "the AppActor pending clone remains protected until rejection cleanup"
    );
    drop(app_pending_permit);
    assert!(
        registry.protected_targets(&old_account).is_empty(),
        "the rejected old-account permit must not leak target protection"
    );

    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[test]
fn scheduled_thread_message_content_preserves_the_thread_relation() {
    let content = build_scheduled_message_content(
        "scheduled thread body",
        Some("$thread-root:example.invalid"),
        koushi_state::ComposerFormattingOptions::default(),
    )
    .expect("thread content should build");
    let value = serde_json::to_value(content).expect("content should serialize");

    assert_eq!(value["m.relates_to"]["rel_type"], "m.thread");
    assert_eq!(
        value["m.relates_to"]["event_id"],
        "$thread-root:example.invalid"
    );
}

/// #1348: a scheduled send is admitted under the composer's own formatting
/// options, so `math_mode` off must not produce math-formatted content.
#[test]
fn scheduled_message_content_follows_the_composer_math_mode() {
    use matrix_sdk::ruma::events::room::message::MessageType;

    let math_on = build_scheduled_message_content(
        "Energy $E=mc^2$",
        None,
        koushi_state::ComposerFormattingOptions { math_mode: true },
    )
    .expect("math-enabled content should build");
    let MessageType::Text(text) = &math_on.msgtype else {
        panic!("expected text content");
    };
    assert!(
        text.formatted
            .as_ref()
            .is_some_and(|formatted| formatted.body.contains("data-mx-maths")),
        "math mode on must keep the math span: {:?}",
        text.formatted
    );

    let math_off = build_scheduled_message_content(
        "Energy $E=mc^2$",
        None,
        koushi_state::ComposerFormattingOptions { math_mode: false },
    )
    .expect("math-disabled content should build");
    let MessageType::Text(text) = &math_off.msgtype else {
        panic!("expected text content");
    };
    assert_eq!(text.body, "Energy $E=mc^2$");
    assert!(
        text.formatted.is_none(),
        "math mode off must not format the formula: {:?}",
        text.formatted
    );
}

/// #1348: the delayed event the homeserver stores must carry the formatting the
/// scheduling command was admitted under, not the builder's default.
#[tokio::test]
async fn server_delayed_schedule_uses_the_command_formatting_options() {
    use matrix_sdk::ruma::room_id;

    let server = MatrixMockServer::new().await;
    server
        .mock_versions()
        .with_feature(crate::scheduled_send::MSC4140_FEATURE, true)
        .with_feature("org.matrix.simplified_msc3575", true)
        .ok()
        .mount()
        .await;
    server
        .mock_room_send()
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "delay_id": "synthetic-delay" })),
        )
        .mount()
        .await;

    let account = SessionKeyId {
        homeserver: server.uri(),
        user_id: "@schedule-math:localhost".to_owned(),
        device_id: "SCHEDULE-MATH".to_owned(),
    };
    let client = server.client_builder().build().await;
    let room = room_id!("!scheduled-math:localhost");
    // The server-delayed path is only admitted for an authoritatively
    // unencrypted room, so the encryption state must be reachable.
    server.mock_room_state_encryption().plain().mount().await;
    server.sync_joined_room(&client, room).await;
    // The SDK caches server versions; make the cache reflect the capabilities
    // the mock advertises so the server-delayed path is reachable.
    client
        .reset_supported_versions()
        .await
        .expect("reset versions cache");
    let session = MatrixClientSession::from_client_for_testing(
        client,
        SessionInfo {
            homeserver: account.homeserver.clone(),
            user_id: account.user_id.clone(),
            device_id: account.device_id.clone(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    );
    let credential_dir = tempdir().expect("credential tempdir");
    let data_dir = tempdir().expect("data tempdir");
    let registry = Arc::new(ComposerDraftLeaseRegistry::new());
    let (handle, mut action_rx, _event_rx) = spawn_actor_with_dirs_and_registry(
        credential_dir.path(),
        data_dir.path(),
        registry.clone(),
    );

    configure_synthetic_oidc_session(
        &handle,
        session,
        RequestId {
            connection_id: RuntimeConnectionId(61),
            sequence: 1,
        },
    )
    .await;
    loop {
        let actions = recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx).await;
        if matches!(
            actions.as_slice(),
            [AppAction::LoginSucceeded { info, .. }]
                if info.user_id == account.user_id && info.device_id == account.device_id
        ) {
            break;
        }
    }

    let request_id = RequestId {
        connection_id: RuntimeConnectionId(61),
        sequence: 2,
    };
    let scope = crate::composer_draft_lifecycle::ComposerDraftScope {
        account: account.clone(),
        target: koushi_state::ComposerTarget::Main {
            room_id: room.to_string(),
        },
    };
    let renderer_generation = registry
        .begin_renderer_generation()
        .expect("begin renderer generation");
    let lease_id = registry
        .acquire(renderer_generation, scope.clone())
        .expect("acquire schedule lease");
    let command_permit = registry
        .try_command_permit(renderer_generation, lease_id, &scope)
        .expect("admit schedule command");
    registry
        .release(renderer_generation, lease_id)
        .expect("release activation after schedule admission");
    let (rejected_tx, _rejected_rx) = mpsc::unbounded_channel();
    let forwarded_permit =
        ForwardedComposerDraftPermit::new(request_id, command_permit, rejected_tx);
    assert!(
        handle
            .send(AccountMessage::ScheduleServerDelayedSend {
                request_id,
                expected_account: account.clone(),
                scheduled_id: "scheduled-math-off".to_owned(),
                room_id: room.to_string(),
                thread_root_event_id: None,
                body: "Energy $E=mc^2$".to_owned(),
                send_at_ms: crate::time::current_epoch_ms().saturating_add(60_000),
                draft_revision: 1.into(),
                composer_permit: forwarded_permit,
                formatting_options: koushi_state::ComposerFormattingOptions { math_mode: false },
            })
            .await
    );

    // Wait for the accepted server handle before reading the wire: the account
    // actor performs the delayed request asynchronously.
    let created = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let Some(actions) = action_rx.recv().await else {
                panic!("account actor stopped before answering the schedule command");
            };
            for action in &actions {
                if let AppAction::ScheduledSendCreatedAtRevision { item, .. } = action {
                    return item.clone();
                }
            }
        }
    })
    .await
    .expect("the schedule command must be answered");
    assert!(
        matches!(created.handle, ScheduledSendHandle::Server { .. }),
        "the unencrypted delayed path must accept the schedule: {:?}",
        created.handle
    );

    let requests = server
        .server()
        .received_requests()
        .await
        .unwrap_or_default();
    let delayed = requests
        .iter()
        .find(|request| request.url.path().contains("/send/m.room.message/"))
        .expect("the delayed message request must reach the homeserver");
    let body: serde_json::Value =
        serde_json::from_slice(&delayed.body).expect("delayed request body is JSON");
    assert_eq!(body["body"], "Energy $E=mc^2$");
    assert!(
        body.get("formatted_body").is_none(),
        "math mode off must not store a formatted delayed body: {body}"
    );

    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn server_item_check_defers_while_scheduled_and_retires_once_sent() {
    use wiremock::matchers::{method, path};

    const LISTING: &str = "/_matrix/client/unstable/org.matrix.msc4140/delayed_events";
    let server = MatrixMockServer::new().await;
    server
        .mock_versions()
        .with_feature(crate::scheduled_send::MSC4140_FEATURE, true)
        .with_feature("org.matrix.simplified_msc3575", true)
        .ok()
        .mount()
        .await;
    let account = SessionKeyId {
        homeserver: server.uri(),
        user_id: "@schedule-check:localhost".to_owned(),
        device_id: "SCHEDULE-CHECK".to_owned(),
    };
    let session = MatrixClientSession::from_client_for_testing(
        server.client_builder().build().await,
        SessionInfo {
            homeserver: account.homeserver.clone(),
            user_id: account.user_id.clone(),
            device_id: account.device_id.clone(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    );
    let credential_dir = tempdir().expect("credential tempdir");
    let data_dir = tempdir().expect("data tempdir");
    let (handle, mut action_rx, _event_rx) = spawn_actor_with_dirs_and_registry(
        credential_dir.path(),
        data_dir.path(),
        Arc::new(ComposerDraftLeaseRegistry::new()),
    );
    configure_synthetic_oidc_session(
        &handle,
        session,
        RequestId {
            connection_id: RuntimeConnectionId(62),
            sequence: 1,
        },
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let actions =
                recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx).await;
            if matches!(actions.as_slice(), [AppAction::LoginSucceeded { .. }]) {
                break;
            }
        }
    })
    .await
    .expect("the synthetic session must log in");

    let check = |delay_id: &str| AccountMessage::CheckServerScheduledSend {
        origin_session_key: account.clone(),
        scheduled_id: "scheduled-check".to_owned(),
        delay_id: delay_id.to_owned(),
    };
    // The homeserver still holds the delayed event: check again later.
    let pending = wiremock::Mock::given(method("GET"))
        .and(path(LISTING))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "delayed_events": [{
                "delay_id": "delay-check",
                "room_id": "!scheduled-check:localhost",
                "type": "m.room.message",
                "content": { "msgtype": "m.text", "body": "synthetic" },
                "delay": 60000,
                "running_since": 1_900_000_000_000_u64
            }]
        })))
        .mount_as_scoped(server.server())
        .await;
    assert!(handle.send(check("delay-check")).await);
    let deferred = next_server_check_result(&mut action_rx)
        .await
        .expect("the pending check must be answered");
    assert!(
        matches!(
            &deferred,
            AppAction::ScheduledSendServerCheckDeferred { scheduled_id, delay_id, recheck_at_ms }
                if scheduled_id == "scheduled-check"
                    && delay_id == "delay-check"
                    && *recheck_at_ms > crate::time::current_epoch_ms()
        ),
        "a still-scheduled delayed event must defer: {deferred:?}"
    );
    drop(pending);

    // The homeserver sent it: the listing no longer holds the delay id.
    let _sent = wiremock::Mock::given(method("GET"))
        .and(path(LISTING))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "delayed_events": [] })),
        )
        .mount_as_scoped(server.server())
        .await;
    assert!(handle.send(check("delay-check")).await);
    let delivered = next_server_check_result(&mut action_rx)
        .await
        .expect("the delivered check must be answered");
    assert!(
        matches!(
            &delivered,
            AppAction::ScheduledSendServerDelivered { scheduled_id, delay_id }
                if scheduled_id == "scheduled-check" && delay_id == "delay-check"
        ),
        "a delayed event the server no longer lists must retire: {delivered:?}"
    );

    // A check for another session never retires anything.
    let other_session = AccountMessage::CheckServerScheduledSend {
        origin_session_key: SessionKeyId {
            device_id: "OTHER-DEVICE".to_owned(),
            ..account.clone()
        },
        scheduled_id: "scheduled-check".to_owned(),
        delay_id: "delay-check".to_owned(),
    };
    assert!(handle.send(other_session).await);
    let foreign = next_server_check_result(&mut action_rx)
        .await
        .expect("the foreign-session check must be answered");
    assert!(
        matches!(foreign, AppAction::ScheduledSendServerCheckDeferred { .. }),
        "a check from a replaced session must not retire the item: {foreign:?}"
    );

    let _ = handle.send(AccountMessage::Shutdown).await;
}

async fn next_server_check_result(
    action_rx: &mut mpsc::Receiver<Vec<AppAction>>,
) -> Result<AppAction, tokio::time::error::Elapsed> {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let Some(actions) = action_rx.recv().await else {
                panic!("account actor stopped before answering the check");
            };
            for action in actions {
                if matches!(
                    action,
                    AppAction::ScheduledSendServerCheckDeferred { .. }
                        | AppAction::ScheduledSendServerDelivered { .. }
                ) {
                    return action;
                }
            }
        }
    })
    .await
}

#[test]
fn scheduled_dispatch_targets_its_origin_session() {
    let origin = SessionKeyId {
        homeserver: "https://example.test".to_owned(),
        user_id: "@alice:example.test".to_owned(),
        device_id: "ALICE".to_owned(),
    };
    let switched = SessionKeyId {
        homeserver: "https://example.test".to_owned(),
        user_id: "@bob:example.test".to_owned(),
        device_id: "BOB".to_owned(),
    };

    assert!(scheduled_dispatch_targets_active_session(
        Some(&origin),
        &origin
    ));
    assert!(!scheduled_dispatch_targets_active_session(
        Some(&switched),
        &origin
    ));
    assert!(!scheduled_dispatch_targets_active_session(None, &origin));
}

async fn configure_synthetic_oidc_session(
    handle: &AccountActorHandle,
    session: MatrixClientSession,
    request_id: RequestId,
) {
    let homeserver = session.info.homeserver.clone();
    assert!(
        handle
            .send(AccountMessage::ConfigureTrustObservation {
                observation: koushi_sdk::CurrentDeviceTrustObservation {
                    current: koushi_state::CurrentDeviceTrustState::Unknown,
                    updates: Box::pin(futures_util::stream::pending()),
                },
            })
            .await
    );
    assert!(
        handle
            .send(AccountMessage::ConfigureOidcCompletion {
                start_request_id: request_id,
                homeserver,
                session,
            })
            .await
    );
    assert!(
        handle
            .send(AccountMessage::Command(AccountCommand::CompleteOidcLogin {
                request_id,
                callback_url: "http://127.0.0.1/callback?code=fixture&state=fixture".to_owned(),
                platform: koushi_state::DisplayPlatform::Linux,
            }))
            .await
    );
}
