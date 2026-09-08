use std::time::Duration;

use koushi_core::runtime::{CoreRuntime, ScopeError};
use koushi_protocol::view::{
    ReaderWindowLimit, ReceiptSourceRef, TimelineViewSource, ViewDelivery, ViewRetirement,
};
use koushi_protocol::{
    AccountCommand, AccountKey, CoreCommand, CoreEvent, CoreFailure, RequestId,
    RuntimeConnectionId, TimelineGeneration, TimelineKey,
};

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

#[test]
fn reader_subscription_close_handle_interrupts_pending_receive() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_stack_size(2 * 1024 * 1024)
        .enable_all()
        .build()
        .expect("test runtime");

    runtime.block_on(async {
        let data_dir = tempfile::tempdir().expect("runtime data directory");
        let core = CoreRuntime::start_with_data_dir(data_dir.path().to_owned());
        let connection = core.attach();
        let source = ReceiptSourceRef {
            timeline: TimelineViewSource {
                key: TimelineKey::room(
                    AccountKey("@reader:example.invalid".into()),
                    "!room:example.invalid",
                ),
                projection_request_id: RequestId {
                    connection_id: RuntimeConnectionId(1),
                    sequence: 1,
                },
                generation: TimelineGeneration(1),
            },
            event_id: "$event:example.invalid".into(),
        };
        let mut subscription = connection
            .subscribe_reader(source, 0, ReaderWindowLimit::try_from(1).unwrap())
            .expect("reader subscription");
        let closer = subscription.close_handle();
        closer.close();
        assert_eq!(
            subscription
                .resource_content(
                    koushi_protocol::view::ViewRevision(1),
                    "avatar/0000000000000001"
                )
                .unwrap_err(),
            ScopeError::Closed
        );
        let pending = tokio::spawn(async move { subscription.next_delivery().await });
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), pending)
                .await
                .unwrap()
                .unwrap(),
            None
        );
        drop(connection);
        core.shutdown().await;
    });
}

#[test]
fn reader_subscription_fits_a_normal_tokio_worker_stack() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_stack_size(2 * 1024 * 1024)
        .enable_all()
        .build()
        .expect("test runtime");

    runtime.block_on(async {
        let data_dir = tempfile::tempdir().expect("runtime data directory");
        let core = CoreRuntime::start_with_data_dir(data_dir.path().to_owned());
        let connection = core.attach();
        let source = ReceiptSourceRef {
            timeline: TimelineViewSource {
                key: TimelineKey::room(
                    AccountKey("@reader:example.invalid".into()),
                    "!room:example.invalid",
                ),
                projection_request_id: RequestId {
                    connection_id: RuntimeConnectionId(1),
                    sequence: 1,
                },
                generation: TimelineGeneration(1),
            },
            event_id: "$event:example.invalid".into(),
        };
        let mut subscription = connection
            .subscribe_reader(source, 0, ReaderWindowLimit::try_from(1).unwrap())
            .expect("reader subscription");

        let delivery = tokio::time::timeout(Duration::from_secs(5), subscription.next_delivery())
            .await
            .expect("reader retirement");
        assert!(matches!(
            delivery,
            Some(ViewDelivery::Retired {
                reason: ViewRetirement::SessionRetired,
                ..
            })
        ));

        drop(subscription);
        drop(connection);
        core.shutdown().await;
    });
}
