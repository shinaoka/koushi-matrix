//! Runtime integration tests for E2EE trust command projection.

use std::time::Duration;

use matrix_desktop_core::command::{AccountCommand, CoreCommand};
use matrix_desktop_core::event::CoreEvent;
use matrix_desktop_core::executor;
use matrix_desktop_core::runtime::CoreRuntime;
use matrix_desktop_state::{AppAction, CrossSigningStatus, SessionState};

mod support;
use support::*;

#[tokio::test]
async fn e2ee_trust_account_command_projects_pending_state_before_routing() {
    let runtime = CoreRuntime::start();
    let mut connection = runtime.attach();

    runtime
        .inject_actions(vec![AppAction::RestoreSessionSucceeded(session_info())])
        .await;

    loop {
        if matches!(connection.snapshot().session, SessionState::Ready(_)) {
            break;
        }
        executor::sleep(Duration::from_millis(5)).await;
    }

    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Account(
            AccountCommand::BootstrapCrossSigning {
                request_id,
                auth: None,
            },
        ))
        .await
        .expect("submit bootstrap cross-signing");

    let snapshot = executor::timeout(Duration::from_secs(1), async {
        loop {
            match connection.recv_event().await.expect("event") {
                CoreEvent::StateChanged(snapshot)
                    if matches!(
                        snapshot.e2ee_trust.cross_signing,
                        CrossSigningStatus::Bootstrapping { .. }
                    ) =>
                {
                    return snapshot;
                }
                _ => continue,
            }
        }
    })
    .await
    .expect("E2EE trust command should project Rust-owned pending state before actor route");

    assert_eq!(
        snapshot.e2ee_trust.cross_signing,
        CrossSigningStatus::Bootstrapping {
            request_id: request_id.sequence,
        }
    );
}
