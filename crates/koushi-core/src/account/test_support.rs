//! Shared cfg(test)-only AccountActor fixtures used by multiple owner suites.

use std::{sync::Arc, time::Duration};

use koushi_state::{
    AppAction, LoginRequest, SlidingSyncAdmissionSource, SlidingSyncCapabilityResult,
};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::account::actor::{AccountActor, AccountActorHandle, AccountMessage};
use crate::command::AccountCommand;
use crate::composer_draft_lifecycle::ComposerDraftLeaseRegistry;
use crate::event::CoreEvent;
use crate::ids::RequestId;
use crate::link_preview::LinkPreviewContext;
use crate::store::CredentialStoreBackend;
use crate::store::StoreActor;
use tempfile::tempdir;

pub(super) fn test_request_id() -> RequestId {
    RequestId {
        connection_id: crate::ids::RuntimeConnectionId(1),
        sequence: 1,
    }
}

pub(super) async fn login_gated_actor() -> (AccountActorHandle, mpsc::Receiver<Vec<AppAction>>) {
    login_gated_actor_at(spawn_quarantine_password_server()).await
}

pub(super) async fn login_gated_actor_at(
    homeserver: String,
) -> (AccountActorHandle, mpsc::Receiver<Vec<AppAction>>) {
    let cred_dir = tempdir().expect("tempdir").keep();
    let data_dir = tempdir().expect("tempdir").keep();
    let (handle, mut action_rx, _event_rx) = spawn_actor_with_dirs(&cred_dir, &data_dir);
    let updates = futures_util::stream::pending();
    handle
        .send(AccountMessage::ConfigureTrustObservation {
            observation: koushi_sdk::CurrentDeviceTrustObservation {
                current: koushi_state::CurrentDeviceTrustState::Unknown,
                updates: Box::pin(updates),
            },
        })
        .await;
    handle
        .send(AccountMessage::Command(AccountCommand::LoginPassword {
            request_id: test_request_id(),
            request: LoginRequest {
                homeserver,
                username: "fixture-user".to_owned(),
                password: koushi_state::AuthSecret::new("synthetic-password"),
                device_display_name: None,
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await;
    while !matches!(
        recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx)
            .await
            .as_slice(),
        [AppAction::LoginSucceeded { .. }]
    ) {}
    (handle, action_rx)
}

pub(super) async fn consume_initial_unknown_trust_projection(
    action_rx: &mut mpsc::Receiver<Vec<AppAction>>,
) {
    while !matches!(
        action_rx.recv().await.as_deref(),
        Some([AppAction::AuthoritativeDeviceTrustChanged {
            trust: koushi_state::CurrentDeviceTrustState::Unknown,
            ..
        }])
    ) {}
}

pub(super) async fn inspect_session_runtime(
    handle: &AccountActorHandle,
) -> (bool, bool, bool, bool) {
    let (response, result) = oneshot::channel();
    assert!(
        handle
            .send(AccountMessage::InspectSessionRuntime { response })
            .await
    );
    result.await.expect("runtime inspection")
}

pub(super) async fn inspect_sync_owners(handle: &AccountActorHandle) -> (bool, bool, bool) {
    let (response, result) = oneshot::channel();
    assert!(
        handle
            .send(AccountMessage::InspectSyncOwners { response })
            .await
    );
    result.await.expect("sync owner inspection")
}

pub(super) async fn shutdown_and_ack(handle: &AccountActorHandle) {
    let (acknowledged, ack) = oneshot::channel();
    assert!(
        handle
            .send(AccountMessage::ShutdownWithAck { acknowledged })
            .await
    );
    ack.await.expect("account shutdown acknowledgement");
}

pub(super) async fn configure_verified_trust(handle: &AccountActorHandle) {
    let updates = futures_util::stream::pending();
    assert!(
        handle
            .send(AccountMessage::ConfigureTrustObservation {
                observation: koushi_sdk::CurrentDeviceTrustObservation {
                    current: koushi_state::CurrentDeviceTrustState::Verified,
                    updates: Box::pin(updates),
                },
            })
            .await
    );
}

pub(super) async fn acknowledge_next_verified_projection(
    handle: &AccountActorHandle,
    action_rx: &mut mpsc::Receiver<Vec<AppAction>>,
) {
    let generation = loop {
        let actions = recv_account_action_with_sliding_sync_effects(handle, action_rx).await;
        if let [
            AppAction::AuthoritativeDeviceTrustChanged {
                generation,
                transition_id,
                trust: koushi_state::CurrentDeviceTrustState::Verified,
            },
        ] = actions.as_slice()
        {
            break (*generation, *transition_id);
        }
    };
    let (generation, transition_id) = generation;
    assert!(
        handle
            .send(AccountMessage::TrustProjectionApplied {
                generation,
                transition_id,
                ready: true,
                locked: false,
            })
            .await
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if inspect_session_runtime(handle).await == (true, true, true, true) {
                break;
            }
            crate::executor::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("verified runtime children should start");
}

pub(super) fn assert_no_logout_finished(action_rx: &mut mpsc::Receiver<Vec<AppAction>>) {
    while let Ok(actions) = action_rx.try_recv() {
        assert!(
            !matches!(actions.as_slice(), [AppAction::LogoutFinished]),
            "teardown acknowledged logout before close barrier"
        );
    }
}

pub(super) async fn recv_account_action_with_sliding_sync_effects(
    handle: &AccountActorHandle,
    action_rx: &mut mpsc::Receiver<Vec<AppAction>>,
) -> Vec<AppAction> {
    let actions = action_rx.recv().await.expect("account action channel");
    route_sliding_sync_effects(handle, &actions).await;
    actions
}

pub(super) async fn route_sliding_sync_effects(handle: &AccountActorHandle, actions: &[AppAction]) {
    for action in actions {
        match action {
            AppAction::SlidingSyncCapabilityCheckCompleted {
                account_epoch,
                request_id,
                result,
            } => {
                let source = match result {
                    SlidingSyncCapabilityResult::Supported { .. } => {
                        Some(SlidingSyncAdmissionSource::Network)
                    }
                    SlidingSyncCapabilityResult::Unreachable
                    | SlidingSyncCapabilityResult::InvalidResponse => None,
                    SlidingSyncCapabilityResult::Unsupported => None,
                };
                if let Some(source) = source {
                    handle
                        .send(AccountMessage::ContinueSlidingSyncAdmission {
                            account_epoch: *account_epoch,
                            request_id: *request_id,
                            source,
                        })
                        .await;
                }
            }
            AppAction::SlidingSyncCapabilityRetryAccepted {
                account_epoch,
                blocked_request_id,
                request_id,
            } => {
                handle
                    .send(AccountMessage::RetrySlidingSyncCapabilityDiscovery {
                        account_epoch: *account_epoch,
                        blocked_request_id: *blocked_request_id,
                        request_id: *request_id,
                    })
                    .await;
            }
            _ => {}
        }
    }
}

pub(super) async fn recv_probe_with_sliding_sync_effects(
    handle: &AccountActorHandle,
    action_rx: &mut mpsc::Receiver<Vec<AppAction>>,
    probe_rx: &mut mpsc::UnboundedReceiver<&'static str>,
    expected: &'static str,
) {
    loop {
        tokio::select! {
            token = probe_rx.recv() => {
                if token == Some(expected) {
                    return;
                }
            }
            actions = action_rx.recv() => {
                let actions = actions.expect("account action channel");
                route_sliding_sync_effects(handle, &actions).await;
            }
        }
    }
}

pub(super) fn spawn_quarantine_password_server() -> String {
    spawn_named_quarantine_password_server("@fixture-user:example.invalid", "FIXTUREDEVICE")
}

#[derive(Default)]
pub(super) struct KeyQueryControl {
    pub(super) count: std::sync::atomic::AtomicUsize,
    pub(super) hold: std::sync::atomic::AtomicBool,
}

pub(super) fn spawn_named_quarantine_password_server(
    user_id: &'static str,
    device_id: &'static str,
) -> String {
    spawn_named_quarantine_password_server_with_offline(user_id, device_id, None)
}

pub(super) fn spawn_named_quarantine_password_server_with_offline(
    user_id: &'static str,
    device_id: &'static str,
    offline: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> String {
    spawn_named_quarantine_password_server_with_controls(
        user_id,
        device_id,
        offline,
        None,
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
    )
}

pub(super) fn spawn_named_quarantine_password_server_with_controls(
    user_id: &'static str,
    device_id: &'static str,
    offline: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    key_query_control: Option<std::sync::Arc<KeyQueryControl>>,
    sliding_sync_supported: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("address");
    let uploaded_device_keys = std::sync::Arc::new(std::sync::Mutex::new(None));
    let uploaded_device_keys_for_server = uploaded_device_keys.clone();
    std::thread::spawn(move || {
        'accept: while let Ok((mut stream, _)) = listener.accept() {
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let count = match stream.read(&mut buffer) {
                    Ok(0) => continue 'accept,
                    Ok(count) => count,
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::ConnectionReset
                                | std::io::ErrorKind::BrokenPipe
                                | std::io::ErrorKind::UnexpectedEof
                        ) =>
                    {
                        continue 'accept;
                    }
                    Err(error) => panic!("read: {error}"),
                };
                request.extend_from_slice(&buffer[..count]);
                let text = String::from_utf8_lossy(&request);
                let Some(end) = text.find("\r\n\r\n") else {
                    continue;
                };
                let length = text
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(0);
                if request.len() >= end + 4 + length {
                    break;
                }
            }
            let text = String::from_utf8_lossy(&request);
            if offline
                .as_ref()
                .is_some_and(|offline| offline.load(std::sync::atomic::Ordering::SeqCst))
            {
                continue;
            }
            let body = if text.starts_with("GET /_matrix/client/versions ") {
                let sliding_sync_supported =
                    sliding_sync_supported.load(std::sync::atomic::Ordering::SeqCst);
                format!(
                    r#"{{"versions":["v1.7"],"unstable_features":{{"org.matrix.simplified_msc3575":{sliding_sync_supported}}}}}"#
                )
            } else if text.contains("/_matrix/client/") && text.contains("login") {
                let requested_device_id = text
                    .split_once("\r\n\r\n")
                    .and_then(|(_, body)| serde_json::from_str::<serde_json::Value>(body).ok())
                    .and_then(|body| body["device_id"].as_str().map(str::to_owned))
                    .unwrap_or_else(|| device_id.to_owned());
                format!(
                    r#"{{"access_token":"fixture-token","device_id":"{requested_device_id}","user_id":"{user_id}"}}"#
                )
            } else if text.contains("/_matrix/client/") && text.contains("/keys/upload") {
                if let Some((_, request_body)) = text.split_once("\r\n\r\n")
                    && let Ok(request) = serde_json::from_str::<serde_json::Value>(request_body)
                    && !request["device_keys"].is_null()
                {
                    *uploaded_device_keys_for_server
                        .lock()
                        .expect("uploaded device keys lock") = Some(request["device_keys"].clone());
                }
                r#"{"one_time_key_counts":{}}"#.to_owned()
            } else if text.contains("/_matrix/client/") && text.contains("/keys/query") {
                if let Some(control) = key_query_control.as_ref() {
                    control
                        .count
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    while control.hold.load(std::sync::atomic::Ordering::SeqCst) {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
                let uploaded = uploaded_device_keys_for_server
                    .lock()
                    .expect("uploaded device keys lock")
                    .clone();
                if let Some(keys) = uploaded {
                    let user = keys["user_id"].as_str().unwrap_or(user_id).to_owned();
                    let device = keys["device_id"].as_str().unwrap_or(device_id).to_owned();
                    let mut devices = serde_json::Map::new();
                    devices.insert(device, keys);
                    let mut users = serde_json::Map::new();
                    users.insert(user, serde_json::Value::Object(devices));
                    serde_json::json!({
                        "device_keys": serde_json::Value::Object(users),
                        "failures": {}
                    })
                    .to_string()
                } else {
                    r#"{"device_keys":{},"failures":{}}"#.to_owned()
                }
            } else if text.contains("/_matrix/client/") && text.contains("/sync") {
                std::thread::sleep(Duration::from_millis(20));
                r#"{"next_batch":"batch","device_lists":{"changed":[],"left":[]},"rooms":{"invite":{},"join":{},"leave":{},"knock":{}},"to_device":{"events":[]},"presence":{"events":[]},"account_data":{"events":[]},"device_one_time_keys_count":{}}"#.to_owned()
            } else {
                r#"{"errcode":"M_NOT_FOUND","error":"not found"}"#.to_owned()
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).expect("write");
        }
    });
    format!("http://{addr}")
}

pub(super) fn spawn_actor_with_dirs(
    cred_dir: &std::path::Path,
    data_dir: &std::path::Path,
) -> (
    AccountActorHandle,
    mpsc::Receiver<Vec<AppAction>>,
    broadcast::Receiver<CoreEvent>,
) {
    spawn_actor_with_dirs_and_registry(
        cred_dir,
        data_dir,
        Arc::new(ComposerDraftLeaseRegistry::new()),
    )
}

pub(super) fn spawn_actor_with_dirs_and_registry(
    cred_dir: &std::path::Path,
    data_dir: &std::path::Path,
    composer_draft_leases: Arc<ComposerDraftLeaseRegistry>,
) -> (
    AccountActorHandle,
    mpsc::Receiver<Vec<AppAction>>,
    broadcast::Receiver<CoreEvent>,
) {
    let store = StoreActor::with_backend(
        CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(cred_dir)),
        data_dir,
    );
    let (action_tx, action_rx) = mpsc::channel(16);
    let (event_tx, event_rx) = broadcast::channel(16);
    let handle = AccountActor::spawn(
        store,
        action_tx,
        event_tx,
        LinkPreviewContext::default(),
        composer_draft_leases,
    );
    (handle, action_rx, event_rx)
}
