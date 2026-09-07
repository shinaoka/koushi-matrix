use std::time::Duration;

use koushi_core::runtime::CoreRuntime;
use koushi_protocol::{AccountCommand, AppCommand, CoreCommand, CoreEvent, CoreFailure};

#[test]
fn public_command_fits_a_normal_tokio_worker_stack() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_stack_size(2 * 1024 * 1024)
        .enable_all()
        .build()
        .expect("test runtime");

    runtime.block_on(async {
        let data_dir = tempfile::tempdir().expect("runtime data directory");
        let core = CoreRuntime::start_with_data_dir(data_dir.path().to_owned());
        let mut connection = core.attach();
        let close_request_id = connection.next_request_id();
        connection
            .command(CoreCommand::App(AppCommand::CloseSearch {
                request_id: close_request_id,
            }))
            .await
            .expect("app command");
        // The following correlated response proves the preceding App command
        // also passed through the actor's ordered command lane.
        let request_id = connection.next_request_id();
        connection
            .command(CoreCommand::Account(
                AccountCommand::RefreshCurrentSessionStatus {
                    request_id,
                    trigger: koushi_state::SessionStatusRefreshTrigger::Manual,
                },
            ))
            .await
            .expect("public command");

        let event = tokio::time::timeout(Duration::from_secs(5), connection.recv_event())
            .await
            .expect("command rejection");
        assert!(matches!(
            event,
            Ok(CoreEvent::OperationFailed {
                request_id: actual,
                failure: CoreFailure::SessionRequired,
            }) if actual == request_id
        ));

        drop(connection);
        core.shutdown().await;
    });
}
