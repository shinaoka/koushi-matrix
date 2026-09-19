use super::*;
use std::task::Poll;

#[tokio::test]
async fn failed_final_draft_flush_still_cleans_up_account_and_reports_failure() {
    let directory = tempfile::tempdir().unwrap();
    // A file where account directories belong makes the real draft save fail.
    std::fs::write(directory.path().join("accounts"), b"not a directory").unwrap();
    let runtime = CoreRuntime::start_with_data_dir(directory.path().to_owned());
    let (completion, installed) = oneshot::channel();
    runtime
        .composer_draft_test_tx
        .send(ComposerDraftTestMutation {
            drafts: Default::default(),
            completion,
            pending_shutdown_persist: Some(PendingComposerDraftPersist::for_shutdown_test(
                koushi_protocol::SessionKeyId {
                    homeserver: "https://example.invalid".into(),
                    user_id: "@shutdown:example.invalid".into(),
                    device_id: "TEST".into(),
                },
            )),
        })
        .await
        .unwrap();
    runtime.action_tx.send(Vec::new()).await.unwrap();
    installed.await.unwrap();
    let (entered, cleanup_entered) = oneshot::channel();
    let (release, cleanup_release) = oneshot::channel();
    assert!(
        runtime
            .account_actor_test_handle
            .send(AccountMessage::ConfigureShutdownGate {
                entered,
                release: cleanup_release,
            })
            .await
    );
    request_shutdown(&runtime).await;
    cleanup_entered.await.unwrap();
    let mut waiter = Box::pin(runtime.wait_for_shutdown());
    let pending =
        std::future::poll_fn(|cx| Poll::Ready(waiter.as_mut().poll(cx).is_pending())).await;
    drop(waiter);
    release.send(()).unwrap();
    let result = runtime.wait_for_shutdown().await;
    runtime.shutdown().await;
    assert!(
        pending,
        "failed draft flush must still await account cleanup"
    );
    assert_eq!(result, Err(CoreShutdownError::Incomplete));
}

#[tokio::test]
async fn failed_room_teardown_prevents_successful_core_completion() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = CoreRuntime::start_with_data_dir(directory.path().to_owned());
    let (acknowledged, stopped) = oneshot::channel();
    assert!(
        runtime
            .account_actor_test_handle
            .send(AccountMessage::StopRoomActorForTesting { acknowledged })
            .await
    );
    stopped.await.unwrap();
    request_shutdown(&runtime).await;
    let result = runtime.wait_for_shutdown().await;
    runtime.shutdown().await;
    assert_eq!(result, Err(CoreShutdownError::Incomplete));
}

async fn request_shutdown(runtime: &CoreRuntime) {
    let connection = runtime.attach();
    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::App(AppCommand::Shutdown { request_id }))
        .await
        .unwrap();
}

#[tokio::test]
async fn completion_waits_for_account_cleanup_and_cancelled_waiter_is_independent() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = CoreRuntime::start_with_data_dir(directory.path().to_owned());
    let (entered, entry) = oneshot::channel();
    let (release, gate) = oneshot::channel();
    assert!(
        runtime
            .account_actor_test_handle
            .send(AccountMessage::ConfigureShutdownGate {
                entered,
                release: gate,
            })
            .await
    );
    request_shutdown(&runtime).await;
    entry.await.unwrap();
    let mut waiter = Box::pin(runtime.wait_for_shutdown());
    // Poll once, then cancel only this observer while AccountActor is gated.
    let pending =
        std::future::poll_fn(|cx| Poll::Ready(waiter.as_mut().poll(cx).is_pending())).await;
    drop(waiter);
    release.send(()).unwrap();
    let result = runtime.wait_for_shutdown().await;
    assert!(
        pending,
        "Core reported completion before AccountActor cleanup"
    );
    assert_eq!(result, Ok(()));
    assert_eq!(runtime.wait_for_shutdown().await, Ok(()));
    runtime.shutdown().await;
}

#[tokio::test]
async fn aborted_app_actor_is_typed_failure() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = CoreRuntime::start_with_data_dir(directory.path().to_owned());
    runtime.shutdown_handle().abort();
    assert_eq!(
        runtime.wait_for_shutdown().await,
        Err(CoreShutdownError::Incomplete)
    );
    runtime
        .account_actor_test_handle
        .shutdown_for_testing()
        .await;
    runtime.shutdown().await;
}

#[tokio::test]
async fn aborted_media_lifecycle_is_typed_failure() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = CoreRuntime::start_with_data_dir(directory.path().to_owned());
    runtime.media_lifecycle.abort();
    assert_eq!(
        runtime.wait_for_shutdown().await,
        Err(CoreShutdownError::Incomplete)
    );
    request_shutdown(&runtime).await;
    runtime.shutdown().await;
}

#[tokio::test]
async fn closed_completion_without_success_is_not_success() {
    let (sender, receiver) = watch::channel(None);
    drop(sender);
    assert_eq!(
        await_shutdown_completion(receiver).await,
        Err(CoreShutdownError::Incomplete)
    );
}

#[tokio::test]
async fn completion_requires_both_actor_and_lifecycle_in_either_order() {
    for actor_first in [true, false] {
        let (actor_tx, actor_rx) = watch::channel(None);
        let (completion_tx, completion_rx) = watch::channel(None);
        let (release, lifecycle) = oneshot::channel();
        let mut publisher = Box::pin(publish_shutdown_completion(
            async {
                lifecycle.await.unwrap();
            },
            actor_rx,
            completion_tx,
        ));
        if actor_first {
            actor_tx.send_replace(Some(Ok(())));
        } else {
            release.send(()).unwrap();
            assert!(
                std::future::poll_fn(|cx| Poll::Ready(publisher.as_mut().poll(cx).is_pending()))
                    .await
            );
            assert_eq!(*completion_rx.borrow(), None);
            actor_tx.send_replace(Some(Ok(())));
            publisher.await;
            assert_eq!(await_shutdown_completion(completion_rx).await, Ok(()));
            continue;
        }
        assert!(
            std::future::poll_fn(|cx| Poll::Ready(publisher.as_mut().poll(cx).is_pending())).await
        );
        assert_eq!(*completion_rx.borrow(), None);
        release.send(()).unwrap();
        publisher.await;
        assert_eq!(await_shutdown_completion(completion_rx).await, Ok(()));
    }
}

#[tokio::test]
async fn missing_account_acknowledgment_is_typed_failure() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = CoreRuntime::start_with_data_dir(directory.path().to_owned());
    assert!(
        runtime
            .account_actor_test_handle
            .shutdown_for_testing()
            .await
    );
    request_shutdown(&runtime).await;
    assert_eq!(
        runtime.wait_for_shutdown().await,
        Err(CoreShutdownError::Incomplete)
    );
    runtime.shutdown().await;
}
