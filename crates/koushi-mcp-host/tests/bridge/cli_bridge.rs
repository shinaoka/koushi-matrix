//! The CLI's exact IPC route through the production host and consent broker.
#[path = "../support/mod.rs"]
mod support;

use koushi_core::runtime::CoreRuntime;
use koushi_mcp::{ipc, wire::ClientInfo};
use koushi_mcp_host::{
    BridgeAccount, BridgeAccounts, Need,
    consent::{ConsentBroker, ConsentMode},
};
use koushi_state::{AppAction, CurrentDeviceTrustState};
use serde_json::{Value, json};
use std::{path::Path, sync::Arc, time::Duration};

const FIRST: &str = "account:@member1:example.invalid";
const SECOND: &str = "account:@member2:example.invalid";

struct Accounts {
    cores: Vec<Arc<CoreRuntime>>,
    media: std::path::PathBuf,
}
impl BridgeAccounts for Accounts {
    fn accounts(&self) -> Vec<BridgeAccount> {
        self.cores
            .iter()
            .enumerate()
            .map(|(index, core)| BridgeAccount {
                id: if index == 0 { FIRST } else { SECOND }.into(),
                homeserver: None,
                connection: core.attach(),
                media_cache_dir: self.media.clone(),
            })
            .collect()
    }
}
async fn connect(data: &Path, name: &str) -> ipc::Client {
    ipc::connect(
        &data.join("mcp/endpoint.json"),
        ClientInfo {
            name: name.into(),
            version: "1".into(),
        },
    )
    .await
    .expect("connect")
}
async fn rooms(client: &mut ipc::Client, account: &str) -> Value {
    tokio::time::timeout(
        Duration::from_secs(5),
        client.call(
            "tools/call",
            json!({"name":"list_rooms","arguments":{"account_id":account}}),
        ),
    )
    .await
    .expect("bounded call")
    .expect("host reply")
}
async fn cores(dir: &Path) -> Arc<Accounts> {
    let mut cores = Vec::new();
    for (index, user) in ["@member1:example.invalid", "@member2:example.invalid"]
        .iter()
        .enumerate()
    {
        let core = Arc::new(CoreRuntime::start_with_data_dir(
            dir.join(index.to_string()),
        ));
        let mut info = support::session_info();
        info.user_id = (*user).into();
        support::apply_actions(
            &core,
            vec![
                AppAction::RestoreSessionRequested,
                AppAction::RestoreSessionSucceeded(info),
                AppAction::CurrentDeviceTrustChanged(CurrentDeviceTrustState::Verified),
                AppAction::RoomListUpdated {
                    spaces: Vec::new(),
                    rooms: vec![support::room(if index == 0 {
                        "!first:example.invalid"
                    } else {
                        "!second:example.invalid"
                    })],
                },
            ],
        )
        .await;
        support::wait_for_session(&core, |state| {
            matches!(state, koushi_state::SessionState::Ready(_))
        })
        .await;
        cores.push(core);
    }
    Arc::new(Accounts {
        cores,
        media: dir.join("media"),
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_grants_do_not_inherit_from_mcp_or_cross_accounts_and_revocation_applies() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data = dir.path().join("app");
    let accounts = cores(dir.path()).await;
    let broker = ConsentBroker::new(data.clone(), ConsentMode::FailClosed);
    let bridge =
        koushi_mcp_host::start(accounts.clone(), data.clone(), broker.clone()).expect("bridge");
    broker
        .set_permission("codex", FIRST, Need::Read, true)
        .await
        .expect("other client grant");
    let mut cli = connect(&data, "koushi-cli").await;
    assert_eq!(
        rooms(&mut cli, FIRST).await["structuredContent"]["error"]["code"],
        "consent_required"
    );
    broker
        .set_permission("koushi-cli", FIRST, Need::Read, true)
        .await
        .expect("grant first");
    assert_eq!(
        rooms(&mut cli, FIRST).await["structuredContent"]["rooms"][0]["room_id"],
        "!first:example.invalid"
    );
    assert_eq!(
        rooms(&mut cli, SECOND).await["structuredContent"]["error"]["code"],
        "consent_required"
    );
    broker
        .set_permission("koushi-cli", SECOND, Need::Read, true)
        .await
        .expect("grant second");
    let mut tasks = Vec::new();
    for _ in 0..6 {
        let data = data.clone();
        tasks.push(tokio::spawn(async move {
            let mut cli = connect(&data, "koushi-cli").await;
            rooms(&mut cli, SECOND).await
        }));
    }
    for task in tasks {
        assert_eq!(
            task.await.expect("client")["structuredContent"]["rooms"][0]["room_id"],
            "!second:example.invalid"
        );
    }
    broker
        .set_permission("koushi-cli", FIRST, Need::Read, false)
        .await
        .expect("revoke");
    assert_eq!(
        rooms(&mut cli, FIRST).await["structuredContent"]["error"]["code"],
        "consent_required"
    );
    support::apply_actions(
        &accounts.cores[1],
        vec![AppAction::SessionAuthenticationInvalidated { soft_logout: true }],
    )
    .await;
    support::wait_for_session(&accounts.cores[1], |state| {
        matches!(state, koushi_state::SessionState::Locked(_))
    })
    .await;
    assert_eq!(
        rooms(&mut cli, SECOND).await["structuredContent"]["error"]["code"],
        "account_unavailable"
    );
    bridge.shutdown().await;
    assert!(cli.call("status", json!({})).await.is_err());
    let restarted = koushi_mcp_host::start(accounts, data.clone(), broker).expect("restart");
    let mut fresh = connect(&data, "koushi-cli").await;
    assert_eq!(
        rooms(&mut fresh, FIRST).await["structuredContent"]["error"]["code"],
        "consent_required"
    );
    restarted.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_cli_read_requires_an_in_app_answer_and_denial_returns_no_data() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data = dir.path().join("app");
    let accounts = cores(dir.path()).await;
    let broker = ConsentBroker::new(data.clone(), ConsentMode::Interactive);
    let bridge = koushi_mcp_host::start(accounts, data.clone(), broker.clone()).expect("bridge");
    for allow in [false, true] {
        if allow {
            broker
                .remove_client("koushi-cli")
                .await
                .expect("reset denial");
        }
        let mut cli = connect(&data, "koushi-cli").await;
        let mut state = broker.subscribe();
        let request = tokio::spawn(async move { rooms(&mut cli, FIRST).await });
        let id = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(pending) = state.borrow_and_update().pending.first() {
                    break pending.request_id;
                }
                state.changed().await.expect("approval");
            }
        })
        .await
        .expect("pending");
        assert_eq!(
            broker.answer(id, allow).await,
            koushi_mcp_host::consent::AnswerOutcome::Applied
        );
        let response = request.await.expect("request");
        assert_eq!(response["isError"], !allow);
        if !allow {
            assert!(response["structuredContent"].get("rooms").is_none());
        }
    }
    bridge.shutdown().await;
}
