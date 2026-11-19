//! #1427, #1460: the delivery channel as the runtime actually wires it.
//!
//! These tests drive the real `CoreRuntime` over an isolated data directory and a file
//! credential store: the command path, the credential seam, the actor's own observation, and
//! the send that leaves through a stubbed relay. No test touches the OS keychain, and the only
//! socket any of them opens is a wiremock stub on the loopback interface.

use std::time::Duration;

use koushi_protocol::command::{AppCommand, CoreCommand};
use koushi_state::{
    AppAction, CurrentDeviceTrustState, NotificationDeliveryContent,
    NotificationDeliveryOutcomeState, NotificationDeliverySettings, NotificationSinkConfig,
    NotificationSinkSettings, RoomSummary, SessionInfo, SettingsPatch,
};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::tests::wait_for_runtime_snapshot;
use super::{CoreRuntime, IsolatedStores};

/// The one sink every test configures.
const SINK: &str = "qq-mail";
/// The fixture credential. It never reaches a log, a snapshot, or a file this repo keeps.
const SECRET: &str = "fixture-relay-secret";
const ROOM: &str = "!room:example.invalid";

const TEST_LIVENESS: Duration = Duration::from_secs(60);

/// The room the delivery channel observes, with the newest message text this device
/// already holds set the way the room projection sets it.
fn unread_dm_with_text(text: &str) -> RoomSummary {
    let mut room: RoomSummary = serde_json::from_value(serde_json::json!({
        "room_id": ROOM,
        "display_name": "Fixture Room",
        "display_label": "Fixture Room",
        "is_dm": true,
        "unread_count": 2,
        "notification_count": 2,
        "highlight_count": 0,
        "parent_space_ids": [],
        "latest_event": {
            "event_id": "$event:example.invalid",
            "preview": "fixture preview",
            "timestamp_ms": 42,
        },
    }))
    .expect("a room summary with a latest event");
    room.latest_event
        .as_mut()
        .expect("the fixture has a latest event")
        .message_text = Some(text.to_owned());
    room
}

fn delivery_settings(endpoint: &str) -> NotificationDeliverySettings {
    let mut sink = NotificationSinkSettings {
        id: SINK.to_owned(),
        destination: "fixture@example.invalid".to_owned(),
        from_address: "koushi@example.invalid".to_owned(),
        sender_name: "Koushi".to_owned(),
        config: NotificationSinkConfig::Resend {
            endpoint: endpoint.to_owned(),
        },
        ..NotificationSinkSettings::default()
    };
    sink.timing.dwell_secs = 60;
    sink.timing.min_interval_secs = 900;
    // The headless runtime has no window, and an unknown focus reads as focused.
    sink.timing.only_when_unfocused = false;
    NotificationDeliverySettings { sinks: vec![sink] }
}

/// The binding the snapshot holds for the fixture sink, revision included.
fn snapshot_binding(connection: &super::CoreConnection) -> Option<String> {
    connection.snapshot().settings.values.delivery.sinks[0]
        .secret_binding
        .clone()
}

/// A runtime whose settings already carry one complete, enabled sink.
async fn runtime_with_sink(endpoint: &str) -> (CoreRuntime, IsolatedStores, super::CoreConnection) {
    runtime_with_delivery(delivery_settings(endpoint)).await
}

/// Put one delivery block into the app-scoped settings through the real command path, and wait
/// for the projection the actor's own observation publishes for it.
async fn put_delivery_settings(
    connection: &mut super::CoreConnection,
    delivery: NotificationDeliverySettings,
) {
    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::App(AppCommand::UpdateSettings {
            request_id,
            patch: SettingsPatch {
                delivery: Some(delivery),
                ..SettingsPatch::default()
            },
        }))
        .await
        .expect("the settings command is admitted");
}

async fn runtime_with_delivery(
    delivery: NotificationDeliverySettings,
) -> (CoreRuntime, IsolatedStores, super::CoreConnection) {
    let (runtime, stores) = CoreRuntime::start_isolated_with_event_capacity(64);
    let mut connection = runtime.attach();
    establish_notification_account(&runtime, &mut connection).await;
    put_delivery_settings(&mut connection, delivery).await;
    wait_for_runtime_snapshot(&mut connection, |state| {
        !state.notification_delivery.sinks.is_empty()
    })
    .await;
    (runtime, stores, connection)
}

/// The binding the settings file records for the fixture sink, as a test can read it back.
fn settings_file_has_binding(stores: &IsolatedStores, binding: &str) -> bool {
    credential_store(stores)
        .load_account_settings(&notification_key())
        .is_ok_and(|settings| {
            settings.delivery.sinks.iter().any(|sink| {
                sink.secret_binding
                    .as_deref()
                    .is_some_and(|value| value.contains(binding))
            })
        })
}

/// Wait until the settings file names this binding, which the persist effect writes after the
/// state has already changed.
async fn wait_for_settings_binding(stores: &IsolatedStores, binding: &str) {
    tokio::time::timeout(TEST_LIVENESS, async {
        loop {
            if settings_file_has_binding(stores, binding) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("configuring a secret persists the binding it was entered for");
}

/// Wait for the operation failure that answers one request id.
///
/// The delivery commands settle through the ordinary operation-failure event, and their
/// expectation is not one of the curated feature outcomes the shared waiter knows.
async fn wait_for_failure(
    connection: &mut super::CoreConnection,
    request_id: koushi_protocol::RequestId,
) -> koushi_protocol::CoreFailure {
    tokio::time::timeout(TEST_LIVENESS, async {
        loop {
            match connection.recv_event().await {
                Ok(koushi_protocol::CoreEvent::OperationFailed {
                    request_id: failed,
                    failure,
                }) if failed == request_id => return failure,
                // Any other event, and a lag that dropped some, is not this answer.
                Ok(_) | Err(_) => continue,
            }
        }
    })
    .await
    .expect("the refusal settles")
}

/// Wait for the settings file the injected patch is persisted to, and return its bytes.
///
/// The patch is persisted by an effect, so the file can appear after the projection it caused.
async fn wait_for_settings_file(path: &std::path::Path) -> Vec<u8> {
    tokio::time::timeout(TEST_LIVENESS, async {
        loop {
            if let Ok(bytes) = std::fs::read(path) {
                return bytes;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the settings file is written")
}

fn sink_state(connection: &super::CoreConnection) -> koushi_state::NotificationDeliverySinkState {
    connection
        .snapshot()
        .notification_delivery
        .sinks
        .into_iter()
        .next()
        .expect("the fixture settings carry one sink")
}

async fn set_secret(connection: &super::CoreConnection) {
    set_secret_value(connection, SECRET).await;
}

async fn set_secret_value(
    connection: &super::CoreConnection,
    secret: &str,
) -> koushi_protocol::RequestId {
    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::App(
            AppCommand::SetNotificationDeliverySecret {
                request_id,
                sink_id: SINK.to_owned(),
                secret: koushi_state::AuthSecret::new(secret),
            },
        ))
        .await
        .expect("the delivery command is admitted");
    request_id
}

/// A store over the same isolated credential directory the runtime was given, so a test can
/// read back what the command wrote without going through the app.
fn credential_store(stores: &IsolatedStores) -> crate::store::StoreActor {
    crate::store::StoreActor::with_backend(
        koushi_store::CredentialStoreBackend::FileDir(koushi_store::FileCredentialStore::new(
            stores.credential_dir().to_path_buf(),
        )),
        stores.data_dir().to_path_buf(),
    )
}

#[tokio::test]
async fn the_secret_command_round_trips_and_flips_configured() {
    let (runtime, stores, mut connection) =
        runtime_with_sink("https://relay.example.invalid/emails").await;
    assert!(!sink_state(&connection).secret_configured);
    assert_eq!(sink_state(&connection).last_outcome, None);
    assert!(
        !sink_state(&connection).secret_retired,
        "a sink that never had a credential has nothing to explain"
    );

    set_secret(&connection).await;
    wait_for_runtime_snapshot(&mut connection, |state| {
        state.notification_delivery.sinks[0].secret_configured
    })
    .await;

    let store = credential_store(&stores);
    let stored = store
        .load_notification_sink_secret(&notification_key(), SINK)
        .expect("the credential store is readable")
        .expect("the command stored the secret");
    assert_eq!(stored.as_str(), SECRET);

    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::App(
            AppCommand::ClearNotificationDeliverySecret {
                request_id,
                sink_id: SINK.to_owned(),
            },
        ))
        .await
        .expect("the clear command is admitted");
    wait_for_runtime_snapshot(&mut connection, |state| {
        !state.notification_delivery.sinks[0].secret_configured
    })
    .await;
    assert!(
        store
            .load_notification_sink_secret(&notification_key(), SINK)
            .expect("the credential store is readable")
            .is_none()
    );
    assert!(
        !sink_state(&connection).secret_retired,
        "clearing a password the user chose to clear is not a retirement to explain"
    );
    drop(runtime);
}

#[tokio::test]
async fn an_unusable_sink_id_is_refused_without_touching_the_store() {
    let (runtime, stores, mut connection) =
        runtime_with_sink("https://relay.example.invalid/emails").await;
    // A sink the settings do not hold has no credential to write and no sink to configure.
    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::App(
            AppCommand::SetNotificationDeliverySecret {
                request_id,
                sink_id: "Not A Sink Id".to_owned(),
                secret: koushi_state::AuthSecret::new(SECRET),
            },
        ))
        .await
        .expect("the command is admitted and fails as an operation failure");
    let failure = wait_for_failure(&mut connection, request_id).await;
    assert!(
        matches!(
            failure,
            koushi_protocol::CoreFailure::NotificationDeliveryRejected { .. }
        ),
        "{failure:?}"
    );
    assert!(
        credential_store(&stores)
            .load_notification_sink_secret(&notification_key(), SINK)
            .expect("the credential store is readable")
            .is_none()
    );
    assert!(
        !connection
            .snapshot()
            .settings
            .values
            .delivery
            .sinks
            .is_empty()
    );
    drop(runtime);
}

#[tokio::test]
async fn the_secret_reaches_no_snapshot_no_debug_and_no_settings_file() {
    let (runtime, stores, mut connection) =
        runtime_with_sink("https://relay.example.invalid/emails").await;
    let settings_path = credential_store(&stores)
        .account_local_data_dir(&notification_key())
        .join("settings/account-settings.v1.enc");
    let without_binding = wait_for_settings_file(&settings_path).await;

    let command = AppCommand::SetNotificationDeliverySecret {
        request_id: connection.next_request_id(),
        sink_id: SINK.to_owned(),
        secret: koushi_state::AuthSecret::new(SECRET),
    };
    let debug = format!("{command:?}");
    assert!(
        !debug.contains(SECRET),
        "the command Debug prints redacted: {debug}"
    );
    assert!(debug.contains("NotificationSinkSecret(<redacted>)"));

    connection
        .command(CoreCommand::App(command))
        .await
        .expect("the delivery command is admitted");
    wait_for_runtime_snapshot(&mut connection, |state| {
        state.notification_delivery.sinks[0].secret_configured
    })
    .await;

    let app_state = serde_json::to_string(&connection.snapshot()).expect("app state serializes");
    assert!(
        !app_state.contains(SECRET),
        "the app state carries no secret"
    );
    let delivery_projection =
        serde_json::to_string(&connection.snapshot().notification_delivery).expect("projection");
    assert!(!delivery_projection.contains(SECRET));

    // What the settings file gains is the binding - the server and account the credential was
    // entered for, plus an opaque revision of this save - and never the value. The frontend
    // never supplies the binding, so a file that carries one is a file the core wrote.
    let binding = snapshot_binding(&connection).expect("the save bound the credential");
    let (identity, revision) = binding.split_once('#').expect("a revisioned binding");
    assert_eq!(identity, "resend:https://relay.example.invalid/emails");
    assert!(!revision.is_empty() && revision.bytes().all(|byte| byte.is_ascii_hexdigit()));
    wait_for_settings_binding(&stores, &binding).await;
    let configured_once = std::fs::read(&settings_path).expect("the settings file is there");
    assert!(
        !configured_once
            .windows(SECRET.len())
            .any(|window| window == SECRET.as_bytes())
    );

    // Two different credentials for one configuration are indistinguishable in the settings, and
    // clearing the secret takes the settings back to exactly what they were before it.
    let cleared = {
        let request_id = connection.next_request_id();
        connection
            .command(CoreCommand::App(
                AppCommand::ClearNotificationDeliverySecret {
                    request_id,
                    sink_id: SINK.to_owned(),
                },
            ))
            .await
            .expect("the clear command is admitted");
        wait_for_runtime_snapshot(&mut connection, |state| {
            !state.notification_delivery.sinks[0].secret_configured
        })
        .await;
        std::fs::read(&settings_path).expect("the settings file is still there")
    };
    assert!(!cleared.is_empty() && !without_binding.is_empty());
    assert!(
        credential_store(&stores)
            .load_account_settings(&notification_key())
            .expect("read cleared account settings")
            .delivery
            .sinks[0]
            .secret_binding
            .is_none()
    );

    // Saving again mints a new revision, so a content consent bound to the previous credential
    // can tell it was replaced; the value itself still never reaches the file.
    set_secret(&connection).await;
    wait_for_runtime_snapshot(&mut connection, |state| {
        state.notification_delivery.sinks[0].secret_configured
    })
    .await;
    let rebound = snapshot_binding(&connection).expect("the save bound the credential");
    assert_ne!(rebound, binding);
    assert!(rebound.starts_with("resend:https://relay.example.invalid/emails#"));
    wait_for_settings_binding(&stores, &rebound).await;
    let configured_twice = std::fs::read(&settings_path).expect("the settings file is there");
    for bytes in [&configured_once, &configured_twice] {
        assert!(
            !bytes
                .windows(SECRET.len())
                .any(|window| window == SECRET.as_bytes())
        );
    }

    // The outcome state is a verdict with no value of its own, and it survives the projection.
    let outcome = NotificationDeliveryOutcomeState::InvalidSecret { at_ms: 5 };
    let outcome_debug = format!("{outcome:?}");
    assert!(!outcome_debug.contains(SECRET));
    assert!(
        !serde_json::to_string(&outcome)
            .expect("outcome")
            .contains(SECRET)
    );
    drop(runtime);
}

#[tokio::test]
async fn changing_the_recipient_targets_or_timing_keeps_the_credential() {
    let (runtime, stores, mut connection) =
        runtime_with_sink("https://relay.example.invalid/emails").await;
    set_secret(&connection).await;
    wait_for_runtime_snapshot(&mut connection, |state| {
        state.notification_delivery.sinks[0].secret_configured
    })
    .await;

    // Who receives the mail, what the sender is called, what is sent and when are not what the
    // credential authenticates, so none of them may throw the password away. The block is the
    // one the state holds, with the binding carried back untouched, which is what the settings
    // form sends.
    let mut cosmetic = *connection.snapshot().settings.values.delivery;
    assert!(cosmetic.sinks[0].secret_binding.is_some());
    cosmetic.sinks[0].destination = "other@example.invalid".to_owned();
    cosmetic.sinks[0].from_address = "other@example.invalid".to_owned();
    cosmetic.sinks[0].sender_name = "Someone Else".to_owned();
    cosmetic.sinks[0].targets.direct_messages = false;
    put_delivery_settings(&mut connection, cosmetic).await;
    wait_for_runtime_snapshot(&mut connection, |state| {
        !state.settings.values.delivery.sinks[0]
            .targets
            .direct_messages
    })
    .await;
    assert!(sink_state(&connection).secret_configured);
    assert!(
        !sink_state(&connection).secret_retired,
        "a cosmetic edit retires nothing"
    );
    assert!(
        credential_store(&stores)
            .load_notification_sink_secret(&notification_key(), SINK)
            .expect("the credential store is readable")
            .is_some(),
        "a cosmetic edit must not delete the credential"
    );

    // Timing is the same kind of edit.
    let mut retimed = *connection.snapshot().settings.values.delivery;
    retimed.sinks[0].timing.min_interval_secs = 60;
    put_delivery_settings(&mut connection, retimed).await;
    wait_for_runtime_snapshot(&mut connection, |state| {
        state.settings.values.delivery.sinks[0]
            .timing
            .min_interval_secs
            == 60
    })
    .await;
    assert!(sink_state(&connection).secret_configured);
    drop(runtime);
}

#[tokio::test]
async fn changing_the_server_retires_the_credential_and_flips_configured() {
    // The form fills the preset, host, port and account together, so that is what a user picking
    // another provider does here.
    let mut smtp = delivery_settings("https://relay.example.invalid/emails");
    smtp.sinks[0].config = NotificationSinkConfig::Smtp {
        preset: Some("qq-mail".to_owned()),
        host: "smtp.qq.com".to_owned(),
        port: 465,
        tls: koushi_state::SmtpTls::Implicit,
        username: "fixture@qq.com".to_owned(),
    };
    let (runtime, stores, mut connection) = runtime_with_delivery(smtp.clone()).await;
    set_secret(&connection).await;
    wait_for_runtime_snapshot(&mut connection, |state| {
        state.notification_delivery.sinks[0].secret_configured
    })
    .await;
    wait_for_settings_binding(&stores, "smtp:smtp.qq.com:465:fixture@qq.com").await;

    let mut moved = smtp.clone();
    moved.sinks[0].config = NotificationSinkConfig::Smtp {
        preset: Some("gmail".to_owned()),
        host: "smtp.gmail.com".to_owned(),
        port: 465,
        tls: koushi_state::SmtpTls::Implicit,
        username: "fixture@qq.com".to_owned(),
    };
    put_delivery_settings(&mut connection, moved).await;

    // The binding no longer names this configuration, so the credential is not this sink's, the
    // sink reads as unconfigured, and the record is gone rather than waiting to be offered to the
    // wrong server.
    let state = wait_for_runtime_snapshot(&mut connection, |state| {
        !state.notification_delivery.sinks[0].secret_configured
    })
    .await;
    // The user never typed the identity, so the UI cannot work out that the password went away;
    // the projection has to say it.
    assert!(state.notification_delivery.sinks[0].secret_retired);
    assert!(
        credential_store(&stores)
            .load_notification_sink_secret(&notification_key(), SINK)
            .expect("the credential store is readable")
            .is_none(),
        "a credential entered for another server is deleted, not kept"
    );
    assert!(!settings_file_has_binding(
        &stores,
        "smtp:smtp.qq.com:465:fixture@qq.com"
    ));

    // Saving a password for the configuration the sink names now is the user acting on that, so
    // the explanation stops being shown.
    set_secret(&connection).await;
    let state = wait_for_runtime_snapshot(&mut connection, |state| {
        state.notification_delivery.sinks[0].secret_configured
    })
    .await;
    assert!(!state.notification_delivery.sinks[0].secret_retired);
    drop(runtime);
}

#[tokio::test]
async fn a_stored_credential_with_no_binding_is_deleted_and_never_used() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let (runtime, stores) = CoreRuntime::start_isolated_with_event_capacity(64);
    let mut connection = runtime.attach();
    establish_notification_account(&runtime, &mut connection).await;
    // A credential that nothing in the settings names, as a hand-edited or half-restored profile
    // leaves behind.
    credential_store(&stores)
        .save_notification_sink_secret(&notification_key(), SINK, SECRET)
        .expect("the fixture credential is stored");

    put_delivery_settings(
        &mut connection,
        delivery_settings(&format!("{}/emails", server.uri())),
    )
    .await;
    let state = wait_for_runtime_snapshot(&mut connection, |state| {
        !state.notification_delivery.sinks.is_empty()
            && !state.notification_delivery.sinks[0].secret_configured
    })
    .await;
    assert_eq!(state.notification_delivery.sinks[0].last_outcome, None);
    assert!(
        state.notification_delivery.sinks[0].secret_retired,
        "the record was there and the core removed it, so the user is owed an explanation"
    );
    assert!(
        !state.notification_delivery.sinks[0].secret_configured,
        "and the sink is not configured"
    );
    assert!(
        credential_store(&stores)
            .load_notification_sink_secret(&notification_key(), SINK)
            .expect("the credential store is readable")
            .is_none(),
        "a credential no binding names belongs to no destination this sink has"
    );

    // Nothing may be attempted with it either: a test send is refused by kind, and no request ever
    // reaches the relay.
    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::App(AppCommand::TestNotificationDelivery {
            request_id,
            sink_id: SINK.to_owned(),
        }))
        .await
        .expect("the test send is admitted");
    let failure = wait_for_failure(&mut connection, request_id).await;
    assert!(
        matches!(
            failure,
            koushi_protocol::CoreFailure::NotificationDeliveryRejected {
                kind: koushi_protocol::NotificationDeliveryFailureKind::SecretMissing
            }
        ),
        "{failure:?}"
    );
    assert_eq!(
        server
            .received_requests()
            .await
            .expect("the stub keeps requests")
            .len(),
        0
    );
    drop(runtime);
}

#[tokio::test]
async fn a_settings_file_whose_binding_moved_comes_up_unconfigured() {
    let (runtime, stores, mut connection) =
        runtime_with_sink("https://relay.example.invalid/emails").await;
    set_secret(&connection).await;
    wait_for_runtime_snapshot(&mut connection, |state| {
        state.notification_delivery.sinks[0].secret_configured
    })
    .await;
    let binding = "resend:https://relay.example.invalid/emails";
    wait_for_settings_binding(&stores, binding).await;
    drop(runtime);

    // The settings file is editable while the app is closed, so a binding that no longer names
    // this configuration must fail closed on the next start rather than be trusted.
    let store = credential_store(&stores);
    let mut values = store
        .load_account_settings(&notification_key())
        .expect("load account settings");
    values.delivery.sinks[0].secret_binding =
        Some("resend:https://elsewhere.example.invalid/emails#01".into());
    store
        .save_account_settings(&notification_key(), &values)
        .expect("write changed binding");

    let runtime = CoreRuntime::restart_isolated(&stores);
    let mut connection = runtime.attach();
    establish_notification_account(&runtime, &mut connection).await;
    let state = wait_for_runtime_snapshot(&mut connection, |state| {
        !state.notification_delivery.sinks.is_empty()
            && !state.notification_delivery.sinks[0].secret_configured
    })
    .await;
    // No edit happened in this run - the file said something that could not be trusted - so the
    // flag is the only way the frontend can tell the user their password is gone.
    assert!(state.notification_delivery.sinks[0].secret_retired);
    assert!(!state.notification_delivery.sinks[0].secret_configured);
    assert!(
        credential_store(&stores)
            .load_notification_sink_secret(&notification_key(), SINK)
            .expect("the credential store is readable")
            .is_none(),
        "the credential loaded beside a binding that moved belongs to another server"
    );
    drop(runtime);
}

#[tokio::test]
async fn a_test_send_delivers_through_the_sink_and_records_the_outcome() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": "ok"})))
        .expect(1)
        .mount(&server)
        .await;
    let endpoint = format!("{}/emails", server.uri());
    let (runtime, _stores, mut connection) = runtime_with_sink(&endpoint).await;
    set_secret(&connection).await;
    wait_for_runtime_snapshot(&mut connection, |state| {
        state.notification_delivery.sinks[0].secret_configured
    })
    .await;

    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::App(AppCommand::TestNotificationDelivery {
            request_id,
            sink_id: SINK.to_owned(),
        }))
        .await
        .expect("the test send is admitted");
    let state = wait_for_runtime_snapshot(&mut connection, |state| {
        state.notification_delivery.sinks[0].last_outcome.is_some()
    })
    .await;
    assert!(matches!(
        state.notification_delivery.sinks[0].last_outcome,
        Some(NotificationDeliveryOutcomeState::Accepted { at_ms }) if at_ms > 0
    ));

    let requests = server
        .received_requests()
        .await
        .expect("the stub keeps requests");
    assert_eq!(requests.len(), 1);
    let body: serde_json::Value =
        serde_json::from_slice(&requests[0].body).expect("the request body is JSON");
    let text = body["text"].as_str().expect("the body carries text");
    // The canned message says it is a test and carries no room, event, or message data.
    assert!(text.to_ascii_lowercase().contains("test"), "{text}");
    assert!(!text.contains(ROOM));
    // A test send consumes no coalescing window, so the real delivery is still to come.
    assert!(
        connection
            .snapshot()
            .notification_delivery
            .sinks
            .iter()
            .all(|sink| sink.secret_configured)
    );
    drop(runtime);
}

#[tokio::test]
async fn a_rejected_secret_is_typed_and_never_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;
    let endpoint = format!("{}/emails", server.uri());
    let (runtime, _stores, mut connection) = runtime_with_sink(&endpoint).await;
    set_secret(&connection).await;
    wait_for_runtime_snapshot(&mut connection, |state| {
        state.notification_delivery.sinks[0].secret_configured
    })
    .await;

    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::App(AppCommand::TestNotificationDelivery {
            request_id,
            sink_id: SINK.to_owned(),
        }))
        .await
        .expect("the test send is admitted");
    let state = wait_for_runtime_snapshot(&mut connection, |state| {
        state.notification_delivery.sinks[0].last_outcome.is_some()
    })
    .await;
    assert!(matches!(
        state.notification_delivery.sinks[0].last_outcome,
        Some(NotificationDeliveryOutcomeState::InvalidSecret { .. })
    ));
    // The sink still reads as configured, so the product can ask for a new credential; what it
    // must not do is try the same rejected value again.
    assert!(state.notification_delivery.sinks[0].secret_configured);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        server
            .received_requests()
            .await
            .expect("the stub keeps requests")
            .len(),
        1,
        "an invalid credential is a terminal verdict"
    );
    drop(runtime);
}

#[tokio::test]
async fn a_test_send_for_an_unconfigured_sink_is_refused_by_kind() {
    let (runtime, _stores, mut connection) =
        runtime_with_sink("https://relay.example.invalid/emails").await;
    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::App(AppCommand::TestNotificationDelivery {
            request_id,
            sink_id: SINK.to_owned(),
        }))
        .await
        .expect("the test send is admitted");
    let failure = wait_for_failure(&mut connection, request_id).await;
    assert!(
        matches!(
            failure,
            koushi_protocol::CoreFailure::NotificationDeliveryRejected { .. }
        ),
        "{failure:?}"
    );
    assert_eq!(sink_state(&connection).last_outcome, None);
    drop(runtime);
}

/// #1427: the newest message text this device already holds leaves through the sink whose
/// stored consent covers it, and never through the app-state snapshot the webview sees.
#[tokio::test]
async fn the_rooms_newest_text_reaches_the_wire_and_never_the_app_state_snapshot() {
    const TEXT: &str = "Alice: review the budget";
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": "ok"})))
        .mount(&server)
        .await;
    let endpoint = format!("{}/emails", server.uri());
    let mut delivery = delivery_settings(&endpoint);
    // Due on the first observation, so the fixture room does not need the clock moved.
    let sink = &mut delivery.sinks[0];
    sink.timing.dwell_secs = 0;
    sink.content = NotificationDeliveryContent::Full;
    sink.content_consent = Some(sink.consent_for_current(1));
    let (runtime, _stores, mut connection) = runtime_with_delivery(delivery).await;
    set_secret(&connection).await;
    wait_for_runtime_snapshot(&mut connection, |state| {
        state.notification_delivery.sinks[0].secret_configured
    })
    .await;

    // A room list only reaches the state under a ready session, so the fixture
    // establishes one exactly as the other runtime tests do.
    runtime
        .inject_actions(vec![
            AppAction::AppStarted,
            AppAction::RestoreSessionSucceeded(SessionInfo {
                homeserver: "https://example.invalid".into(),
                user_id: "@me:example.invalid".into(),
                device_id: "DEVICE".into(),
                authentication_method: Default::default(),
            }),
            // Verified trust is what promotes the provisional session to ready, and a
            // room list only reaches the state under a ready session.
            AppAction::CurrentDeviceTrustChanged(CurrentDeviceTrustState::Verified),
            AppAction::RoomListUpdated {
                spaces: Vec::new(),
                rooms: vec![unread_dm_with_text(TEXT)],
            },
        ])
        .await;
    wait_for_runtime_snapshot(&mut connection, |state| {
        state.rooms.iter().any(|room| room.room_id == ROOM)
    })
    .await;
    let state = wait_for_runtime_snapshot(&mut connection, |state| {
        state.notification_delivery.sinks[0].last_outcome.is_some()
    })
    .await;
    assert!(
        matches!(
            state.notification_delivery.sinks[0].last_outcome,
            Some(NotificationDeliveryOutcomeState::Accepted { .. })
        ),
        "the consented delivery was recorded as {:?}",
        state.notification_delivery.sinks[0].last_outcome
    );
    // The room really is in the state this snapshot is made of: the negative assertion
    // below is about the field, not about an empty room list.
    assert!(
        state
            .rooms
            .iter()
            .any(|room| room.latest_event.is_some() && room.display_label == "Fixture Room")
    );
    assert_eq!(
        state.settings.values.delivery.sinks[0].effective_content(),
        NotificationDeliveryContent::Full,
        "the stored consent covers the sink"
    );
    assert!(
        state
            .rooms
            .iter()
            .find(|room| room.room_id == ROOM)
            .and_then(|room| room.latest_event.as_ref())
            .is_some_and(|event| event.message_text.as_deref() == Some(TEXT)),
        "the room kept the newest text the projection handed it"
    );
    let serialized = serde_json::to_string(&state).expect("the app state serializes");
    assert!(
        !serialized.contains(TEXT),
        "the message text reached the app-state snapshot"
    );

    let requests = server
        .received_requests()
        .await
        .expect("the stub keeps requests");
    assert_eq!(requests.len(), 1, "one digest, one send");
    let body: serde_json::Value =
        serde_json::from_slice(&requests[0].body).expect("the request body is JSON");
    let text = body["text"].as_str().expect("the body carries text");
    assert!(
        text.starts_with("Direct message in Fixture Room: 2 unread"),
        "the room line comes first: {text}"
    );
    assert!(
        text.contains(&format!("> {TEXT}")),
        "the consented sink quotes the room's newest text: {text}"
    );
    // The room id does not ride out with the text.
    assert!(!text.contains(ROOM), "{text}");
    drop(runtime);
}

/// A ServerChan Turbo destination whose service is the loopback stub at `endpoint`.
fn serverchan_settings(endpoint: &str) -> NotificationDeliverySettings {
    let mut delivery = delivery_settings(endpoint);
    let sink = &mut delivery.sinks[0];
    sink.destination.clear();
    sink.from_address.clear();
    sink.config = NotificationSinkConfig::ServerChan {
        endpoint: endpoint.to_owned(),
    };
    delivery
}

/// A Turbo SendKey; the fixture is not a key any account holds.
const SENDKEY: &str = "SCTfixture0sendkey";

#[tokio::test]
async fn a_serverchan3_sendkey_is_refused_by_kind_and_never_stored() {
    let (runtime, stores, mut connection) =
        runtime_with_delivery(serverchan_settings("https://sctapi.ftqq.com")).await;
    let request_id = set_secret_value(&connection, "sctp123tfixturekey").await;
    let failure = wait_for_failure(&mut connection, request_id).await;
    assert!(
        matches!(
            failure,
            koushi_protocol::CoreFailure::NotificationDeliveryRejected {
                kind: koushi_protocol::NotificationDeliveryFailureKind::UnsupportedSendKey
            }
        ),
        "{failure:?}"
    );
    assert!(!sink_state(&connection).secret_configured);
    assert!(
        credential_store(&stores)
            .load_notification_sink_secret(&notification_key(), SINK)
            .expect("the credential store is readable")
            .is_none()
    );
    assert!(snapshot_binding(&connection).is_none());
    drop(runtime);
}

#[tokio::test]
async fn a_serverchan_test_send_posts_a_form_to_the_sendkey_path() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex(r"^/SCT[A-Za-z0-9]+\.send$"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"code": 0, "data": {}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let (runtime, _stores, mut connection) =
        runtime_with_delivery(serverchan_settings(&server.uri())).await;
    set_secret_value(&connection, SENDKEY).await;
    wait_for_runtime_snapshot(&mut connection, |state| {
        state.notification_delivery.sinks[0].secret_configured
    })
    .await;
    assert!(
        snapshot_binding(&connection)
            .is_some_and(|binding| binding.starts_with(&format!("serverchan:{}#", server.uri())))
    );

    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::App(AppCommand::TestNotificationDelivery {
            request_id,
            sink_id: SINK.to_owned(),
        }))
        .await
        .expect("the test send is admitted");
    let state = wait_for_runtime_snapshot(&mut connection, |state| {
        state.notification_delivery.sinks[0].last_outcome.is_some()
    })
    .await;
    assert!(
        matches!(
            state.notification_delivery.sinks[0].last_outcome,
            Some(NotificationDeliveryOutcomeState::Accepted { .. })
        ),
        "{:?}",
        state.notification_delivery.sinks[0].last_outcome
    );

    let requests = server
        .received_requests()
        .await
        .expect("the stub keeps requests");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.path(), format!("/{SENDKEY}.send"));
    let form: std::collections::HashMap<String, String> =
        url::form_urlencoded::parse(&requests[0].body)
            .into_owned()
            .collect();
    assert!(!form["title"].is_empty());
    assert!(
        form["desp"].to_ascii_lowercase().contains("test"),
        "{form:?}"
    );
    // The SendKey is in the URL path only: never in the form, the snapshot or its Debug.
    assert!(!form.values().any(|value| value.contains(SENDKEY)));
    let snapshot = connection.snapshot();
    // #1985: the whole-`AppState` dump is a summary now, so this pins the
    // assertion on the slices that carry delivery configuration, which is where
    // a leaked key would actually appear.
    assert!(!format!("{:?}", snapshot.settings.values.delivery).contains(SENDKEY));
    assert!(!format!("{:?}", snapshot.notification_delivery).contains(SENDKEY));
    assert!(!format!("{snapshot:?}").contains(SENDKEY));
    assert!(
        !serde_json::to_string(&snapshot)
            .expect("serializable")
            .contains(SENDKEY)
    );
    drop(runtime);
}

#[cfg(unix)]
#[tokio::test]
async fn a_new_secret_is_not_stored_while_the_old_binding_cannot_be_retired() {
    use std::os::unix::fs::PermissionsExt;

    let (runtime, stores, mut connection) =
        runtime_with_sink("https://relay.example.invalid/emails").await;
    set_secret(&connection).await;
    wait_for_runtime_snapshot(&mut connection, |state| {
        state.notification_delivery.sinks[0].secret_configured
    })
    .await;
    let old_binding = snapshot_binding(&connection).expect("the first secret is bound");
    wait_for_settings_binding(&stores, &old_binding).await;

    // The settings file can no longer be replaced, so no binding change can be persisted.
    let settings_dir = credential_store(&stores)
        .account_local_data_dir(&notification_key())
        .join("settings");
    let writable = std::fs::metadata(&settings_dir)
        .expect("the settings directory exists")
        .permissions();
    std::fs::set_permissions(&settings_dir, std::fs::Permissions::from_mode(0o555))
        .expect("the settings directory can be made read-only");

    let request_id = set_secret_value(&connection, "re_replacement_fixture_key").await;
    let failure = wait_for_failure(&mut connection, request_id).await;
    std::fs::set_permissions(&settings_dir, writable).expect("permissions are restored");

    assert_eq!(failure, koushi_protocol::CoreFailure::StoreUnavailable);
    // The new key never reached the store behind a file that still names the old revision (and,
    // for ServerChan, the consent given for the old key's owner). The unbound in-memory sink lets
    // the orphan sweep drop the old credential too, which is the safe side of a failed write.
    let stored = credential_store(&stores)
        .load_notification_sink_secret(&notification_key(), SINK)
        .expect("the credential store is readable");
    assert!(
        stored
            .as_ref()
            .is_none_or(|secret| secret.as_str() != "re_replacement_fixture_key"),
        "a replacement key is stored only after the old binding is retired on disk"
    );
    // No revision minted for the replacement is held anywhere, and with the old credential gone
    // the sink reads as unconfigured rather than sending with a binding the file never received.
    let state = wait_for_runtime_snapshot(&mut connection, |state| {
        !state.notification_delivery.sinks[0].secret_configured
    })
    .await;
    let binding = state.settings.values.delivery.sinks[0]
        .secret_binding
        .clone();
    assert!(binding.is_none() || binding == Some(old_binding));
    drop(runtime);
}

// #1468: the settings screen's single Save.

/// Send one `SaveNotificationSink` and return its request id.
async fn save_sink(
    connection: &super::CoreConnection,
    sink: NotificationSinkSettings,
    secret: Option<&str>,
    accept_content_consent: bool,
) -> koushi_protocol::RequestId {
    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::App(AppCommand::SaveNotificationSink {
            request_id,
            sink,
            secret: secret.map(koushi_state::AuthSecret::new),
            accept_content_consent,
        }))
        .await
        .expect("the save command is admitted");
    request_id
}

fn fixture_sink(endpoint: &str) -> NotificationSinkSettings {
    delivery_settings(endpoint).sinks.remove(0)
}

fn rejected(
    kind: koushi_protocol::NotificationDeliveryFailureKind,
) -> koushi_protocol::CoreFailure {
    koushi_protocol::CoreFailure::NotificationDeliveryRejected { kind }
}

#[tokio::test]
async fn one_save_adds_a_destination_with_its_secret() {
    let (runtime, stores) = CoreRuntime::start_isolated_with_event_capacity(64);
    let mut connection = runtime.attach();
    establish_notification_account(&runtime, &mut connection).await;
    save_sink(
        &connection,
        fixture_sink("https://relay.example.invalid/emails"),
        Some(SECRET),
        false,
    )
    .await;
    wait_for_runtime_snapshot(&mut connection, |state| {
        state
            .notification_delivery
            .sinks
            .first()
            .is_some_and(|sink| sink.secret_configured)
    })
    .await;
    let binding = snapshot_binding(&connection).expect("the save bound the credential");
    wait_for_settings_binding(&stores, &binding).await;
    let stored = credential_store(&stores)
        .load_notification_sink_secret(&notification_key(), SINK)
        .expect("the credential store is readable")
        .expect("the save stored the secret");
    assert_eq!(stored.as_str(), SECRET);
    drop(runtime);
}

#[tokio::test]
async fn a_save_without_a_secret_keeps_the_stored_credential() {
    let (runtime, _stores, mut connection) =
        runtime_with_sink("https://relay.example.invalid/emails").await;
    set_secret(&connection).await;
    wait_for_runtime_snapshot(&mut connection, |state| {
        state.notification_delivery.sinks[0].secret_configured
    })
    .await;
    let binding = snapshot_binding(&connection);
    let mut sink = connection.snapshot().settings.values.delivery.sinks[0].clone();
    sink.sender_name = "Renamed".to_owned();
    // The renderer echoing a binding it never owned changes nothing.
    sink.secret_binding = None;
    save_sink(&connection, sink, None, false).await;
    let state = wait_for_runtime_snapshot(&mut connection, |state| {
        state.settings.values.delivery.sinks[0].sender_name == "Renamed"
    })
    .await;
    assert!(state.notification_delivery.sinks[0].secret_configured);
    assert_eq!(snapshot_binding(&connection), binding);
    drop(runtime);
}

#[cfg(unix)]
#[tokio::test]
async fn a_failed_secret_store_removes_the_destination_the_save_added() {
    use std::os::unix::fs::PermissionsExt;

    let (runtime, stores) = CoreRuntime::start_isolated_with_event_capacity(64);
    let mut connection = runtime.attach();
    establish_notification_account(&runtime, &mut connection).await;
    let credential_dir = stores.credential_dir().to_path_buf();
    std::fs::create_dir_all(&credential_dir).expect("the credential directory exists");
    let writable = std::fs::metadata(&credential_dir)
        .expect("the credential directory exists")
        .permissions();
    std::fs::set_permissions(&credential_dir, std::fs::Permissions::from_mode(0o555))
        .expect("the credential directory can be made read-only");

    let request_id = save_sink(
        &connection,
        fixture_sink("https://relay.example.invalid/emails"),
        Some(SECRET),
        false,
    )
    .await;
    let failure = wait_for_failure(&mut connection, request_id).await;
    std::fs::set_permissions(&credential_dir, writable).expect("permissions are restored");

    assert!(
        !matches!(
            failure,
            koushi_protocol::CoreFailure::NotificationDeliveryRejected { .. }
        ),
        "the store's own failure is reported: {failure:?}"
    );
    let state = wait_for_runtime_snapshot(&mut connection, |state| {
        state.settings.values.delivery.sinks.is_empty()
    })
    .await;
    assert!(state.notification_delivery.sinks.is_empty());
    drop(runtime);
}

#[tokio::test]
async fn a_serverchan_consent_accepted_in_the_save_names_the_key_it_stored() {
    let (runtime, _stores) = CoreRuntime::start_isolated_with_event_capacity(64);
    let mut connection = runtime.attach();
    establish_notification_account(&runtime, &mut connection).await;
    let mut sink = serverchan_settings("https://sctapi.ftqq.com")
        .sinks
        .remove(0);
    sink.content = NotificationDeliveryContent::Full;
    save_sink(&connection, sink, Some(SENDKEY), true).await;
    let state = wait_for_runtime_snapshot(&mut connection, |state| {
        state
            .settings
            .values
            .delivery
            .sinks
            .first()
            .is_some_and(|sink| sink.content_consent.is_some())
    })
    .await;
    let saved = &state.settings.values.delivery.sinks[0];
    assert_eq!(saved.effective_content(), NotificationDeliveryContent::Full);
    let consent = saved.content_consent.as_ref().expect("consent minted");
    assert!(consent.recipient.starts_with("sendkey#"));
    assert!(state.notification_delivery.sinks[0].secret_configured);
    drop(runtime);
}

#[tokio::test]
async fn a_consent_the_renderer_built_is_not_taken() {
    let (runtime, _stores, mut connection) =
        runtime_with_sink("https://relay.example.invalid/emails").await;
    let mut sink = connection.snapshot().settings.values.delivery.sinks[0].clone();
    sink.content = NotificationDeliveryContent::Full;
    sink.content_consent = Some(sink.consent_for_current(1));
    sink.sender_name = "Marker".to_owned();
    save_sink(&connection, sink, None, false).await;
    let state = wait_for_runtime_snapshot(&mut connection, |state| {
        state.settings.values.delivery.sinks[0].sender_name == "Marker"
    })
    .await;
    let saved = &state.settings.values.delivery.sinks[0];
    assert!(saved.content_consent.is_none());
    assert_eq!(
        saved.effective_content(),
        NotificationDeliveryContent::NamesOnly
    );
    drop(runtime);
}

#[tokio::test]
async fn a_malformed_sendkey_save_writes_nothing() {
    let (runtime, stores) = CoreRuntime::start_isolated_with_event_capacity(64);
    let mut connection = runtime.attach();
    establish_notification_account(&runtime, &mut connection).await;
    let sink = serverchan_settings("https://sctapi.ftqq.com")
        .sinks
        .remove(0);
    let request_id = save_sink(&connection, sink, Some("sctp123tfixturekey"), false).await;
    let failure = wait_for_failure(&mut connection, request_id).await;
    assert_eq!(
        failure,
        rejected(koushi_protocol::NotificationDeliveryFailureKind::UnsupportedSendKey)
    );
    assert!(
        connection
            .snapshot()
            .settings
            .values
            .delivery
            .sinks
            .is_empty()
    );
    assert!(
        credential_store(&stores)
            .load_notification_sink_secret(&notification_key(), SINK)
            .expect("the credential store is readable")
            .is_none()
    );
    drop(runtime);
}

#[tokio::test]
async fn an_incomplete_destination_is_refused_by_kind() {
    let (runtime, _stores) = CoreRuntime::start_isolated_with_event_capacity(64);
    let mut connection = runtime.attach();
    establish_notification_account(&runtime, &mut connection).await;
    let mut sink = fixture_sink("https://relay.example.invalid/emails");
    sink.destination.clear();
    let request_id = save_sink(&connection, sink, Some(SECRET), false).await;
    let failure = wait_for_failure(&mut connection, request_id).await;
    assert_eq!(
        failure,
        rejected(koushi_protocol::NotificationDeliveryFailureKind::IncompleteSink)
    );
    assert!(
        connection
            .snapshot()
            .settings
            .values
            .delivery
            .sinks
            .is_empty()
    );
    drop(runtime);
}

#[tokio::test]
async fn a_save_beyond_the_destination_limit_is_refused() {
    let template = fixture_sink("https://relay.example.invalid/emails");
    let sinks = (0..koushi_state::MAX_NOTIFICATION_DELIVERY_SINKS)
        .map(|index| NotificationSinkSettings {
            id: format!("sink-{index}"),
            ..template.clone()
        })
        .collect();
    let (runtime, _stores, mut connection) =
        runtime_with_delivery(NotificationDeliverySettings { sinks }).await;
    wait_for_runtime_snapshot(&mut connection, |state| {
        state.settings.values.delivery.sinks.len() == koushi_state::MAX_NOTIFICATION_DELIVERY_SINKS
    })
    .await;
    let request_id = save_sink(
        &connection,
        NotificationSinkSettings {
            id: "one-too-many".to_owned(),
            ..template
        },
        None,
        false,
    )
    .await;
    let failure = wait_for_failure(&mut connection, request_id).await;
    assert_eq!(
        failure,
        rejected(koushi_protocol::NotificationDeliveryFailureKind::SinkLimitReached)
    );
    assert!(
        connection
            .snapshot()
            .settings
            .values
            .delivery
            .sink("one-too-many")
            .is_none()
    );
    drop(runtime);
}

#[tokio::test]
async fn app_settings_cannot_replace_account_routes_or_retire_their_credentials() {
    let (runtime, stores, mut connection) =
        runtime_with_sink("https://relay.example.invalid/emails").await;
    set_secret(&connection).await;
    wait_for_runtime_snapshot(&mut connection, |state| {
        state.notification_delivery.sinks[0].secret_configured
    })
    .await;
    let settings = crate::settings::SettingsStore::new(stores.data_dir());
    settings
        .save_patch(&SettingsPatch {
            scope: Some(koushi_state::SettingsPatchScope::App),
            delivery: Some(NotificationDeliverySettings::default()),
            ..SettingsPatch::default()
        })
        .expect("app patch");
    assert_eq!(
        connection.snapshot().settings.values.delivery.sinks.len(),
        1
    );
    assert!(
        credential_store(&stores)
            .load_notification_sink_secret(&notification_key(), SINK)
            .expect("account credential")
            .is_some()
    );
    drop(runtime);
}

fn notification_key() -> koushi_protocol::SessionKeyId {
    koushi_protocol::SessionKeyId {
        homeserver: "https://example.invalid".into(),
        user_id: "@me:example.invalid".into(),
        device_id: "DEVICE".into(),
    }
}

async fn establish_notification_account(
    runtime: &CoreRuntime,
    connection: &mut super::CoreConnection,
) {
    runtime
        .inject_actions(vec![
            AppAction::AppStarted,
            AppAction::RestoreSessionSucceeded(SessionInfo {
                homeserver: "https://example.invalid".into(),
                user_id: "@me:example.invalid".into(),
                device_id: "DEVICE".into(),
                authentication_method: Default::default(),
            }),
            AppAction::CurrentDeviceTrustChanged(CurrentDeviceTrustState::Verified),
        ])
        .await;
    wait_for_runtime_snapshot(connection, |state| {
        matches!(state.session, koushi_state::SessionState::Ready(_))
    })
    .await;
}
