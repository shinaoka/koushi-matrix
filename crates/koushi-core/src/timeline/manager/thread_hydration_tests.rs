use futures_util::StreamExt;

#[tokio::test]
async fn initial_thread_hydrates_across_hidden_cached_chunks() {
    use matrix_sdk::{
        ruma::{event_id, room_id, user_id},
        test_utils::mocks::MatrixMockServer,
    };
    use matrix_sdk_base::{
        ThreadingSupport,
        linked_chunk::{ChunkIdentifier, LinkedChunkId, Position, Update},
    };
    use matrix_sdk_test::event_factory::EventFactory;
    use matrix_sdk_ui::timeline::TimelineFocus;
    let server = MatrixMockServer::new().await;
    let client = server
        .client_builder()
        .on_builder(|b| {
            b.with_threading_support(ThreadingSupport::Enabled {
                with_subscriptions: true,
            })
        })
        .build()
        .await;
    let room_id = room_id!("!thread:example.test");
    let room = server.sync_joined_room(&client, room_id).await;
    let root = event_id!("$root:example.test");
    let reply = event_id!("$reply:example.test");
    let factory = EventFactory::new()
        .room(room_id)
        .sender(user_id!("@alice:example.test"));
    let chunks = vec![
        vec![
            factory.text_msg("root").event_id(root).into_event(),
            factory
                .text_msg("reply")
                .event_id(reply)
                .in_thread(root, root)
                .into_event(),
        ],
        vec![factory.reaction(reply, "x").into_event()],
        vec![
            factory
                .text_msg("* edited reply")
                .edit(
                    reply,
                    matrix_sdk::ruma::events::room::message::MessageType::text_plain(
                        "edited reply",
                    )
                    .into(),
                )
                .into_event(),
        ],
    ];
    let mut updates = Vec::new();
    for (i, items) in chunks.into_iter().enumerate() {
        let id = ChunkIdentifier::new(i as u64);
        updates.push(Update::NewItemsChunk {
            previous: i.checked_sub(1).map(|n| ChunkIdentifier::new(n as u64)),
            new: id,
            next: None,
        });
        updates.push(Update::PushItems {
            at: Position::new(id, 0),
            items,
        });
    }
    client
        .event_cache_store()
        .lock()
        .await
        .unwrap()
        .as_clean()
        .unwrap()
        .handle_linked_chunk_updates(LinkedChunkId::Thread(room_id, root), updates)
        .await
        .unwrap();
    let timeline = super::koushi_timeline_builder(
        &room,
        TimelineFocus::Thread {
            thread_id: root.to_owned(),
        },
    )
    .build()
    .await
    .unwrap();
    assert!(timeline.subscribe().await.0.is_empty());
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            super::hydrate_initial_thread(&timeline, &client, room_id, root)
        )
        .await
        .expect("cached visible history must not wait for the opening deadline")
        .unwrap()
    );
    let (mut items, mut stream) = timeline.subscribe().await;
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !items
            .iter()
            .any(|i| i.as_event().is_some_and(|e| e.event_id() == Some(reply)))
        {
            for diff in stream.next().await.unwrap() {
                diff.apply(&mut items);
            }
        }
    })
    .await
    .expect("reply is projected");
    let requests = server.server().received_requests().await.unwrap();
    assert!(
        !requests
            .iter()
            .any(|r| r.url.path().contains("/relations/") || r.url.path().ends_with("/messages"))
    );
}

/// #1473: a server answers the thread `/relations` request for a root the
/// account may not see with 403/404 (Synapse: 404 `M_UNKNOWN`). Hydration
/// must classify that as a withheld root, not a generic SDK failure.
#[tokio::test]
async fn initial_thread_classifies_withheld_root_relations_errors() {
    use matrix_sdk::{
        ruma::{event_id, room_id},
        test_utils::mocks::MatrixMockServer,
    };
    use matrix_sdk_base::ThreadingSupport;
    use matrix_sdk_ui::timeline::TimelineFocus;

    for (status, errcode, expected) in [
        (403, "M_FORBIDDEN", super::TimelineFailureKind::Forbidden),
        (404, "M_NOT_FOUND", super::TimelineFailureKind::NotFound),
        (404, "M_UNKNOWN", super::TimelineFailureKind::NotFound),
        (500, "M_UNKNOWN", super::TimelineFailureKind::Sdk),
    ] {
        let server = MatrixMockServer::new().await;
        let client = server
            .client_builder()
            .on_builder(|b| {
                b.with_threading_support(ThreadingSupport::Enabled {
                    with_subscriptions: true,
                })
            })
            .build()
            .await;
        let room_id = room_id!("!withheld:example.test");
        let room = server.sync_joined_room(&client, room_id).await;
        let root = event_id!("$withheld-root:example.test");
        server
            .mock_room_relations()
            .respond_with(
                wiremock::ResponseTemplate::new(status)
                    .set_body_json(serde_json::json!({ "errcode": errcode, "error": "withheld" })),
            )
            .mount()
            .await;
        server
            .mock_room_event()
            .respond_with(
                wiremock::ResponseTemplate::new(status)
                    .set_body_json(serde_json::json!({ "errcode": errcode, "error": "withheld" })),
            )
            .mount()
            .await;
        let timeline = super::koushi_timeline_builder(
            &room,
            TimelineFocus::Thread {
                thread_id: root.to_owned(),
            },
        )
        .build()
        .await
        .unwrap();

        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            super::hydrate_initial_thread(&timeline, &client, room_id, root),
        )
        .await
        .expect("a withheld root must fail before the opening deadline");
        assert_eq!(outcome, Err(expected), "status {status}");
    }
}

#[tokio::test]
async fn initial_thread_open_waits_for_delayed_projection() {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel::<usize>();
    let receiver = futures_util::stream::unfold(receiver, |mut rx| async move {
        rx.recv().await.map(|count| (count, rx))
    });
    let mut ready = Box::pin(super::await_initial_thread_projection(false, receiver));
    assert!(
        futures_util::poll!(ready.as_mut()).is_pending(),
        "successful pagination must not fail before its projection arrives"
    );
    sender.send(0).unwrap();
    assert!(futures_util::poll!(ready.as_mut()).is_pending());
    sender.send(2).unwrap();
    assert!(ready.await);
}

#[tokio::test]
async fn initial_thread_projection_end_and_closed_stream() {
    assert!(super::await_initial_thread_projection(true, futures_util::stream::pending()).await);
    assert!(!super::await_initial_thread_projection(false, futures_util::stream::empty()).await);
}

#[tokio::test(start_paused = true)]
async fn initial_thread_projection_deadline_is_not_extended_by_empty_updates() {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel::<usize>();
    let receiver = futures_util::stream::unfold(receiver, |mut rx| async move {
        rx.recv().await.map(|count| (count, rx))
    });
    let mut ready = Box::pin(super::await_initial_thread_projection(false, receiver));
    assert!(futures_util::poll!(ready.as_mut()).is_pending());
    tokio::time::advance(std::time::Duration::from_secs(9)).await;
    sender.send(0).unwrap();
    assert!(futures_util::poll!(ready.as_mut()).is_pending());
    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    assert!(!ready.await);
    assert!(sender.is_closed());
}

#[tokio::test]
async fn initial_thread_projection_cancellation_releases_subscription() {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel::<usize>();
    let receiver = futures_util::stream::unfold(receiver, |mut rx| async move {
        rx.recv().await.map(|count| (count, rx))
    });
    let mut ready = Box::pin(super::await_initial_thread_projection(false, receiver));
    assert!(futures_util::poll!(ready.as_mut()).is_pending());
    drop(ready);
    assert!(sender.is_closed());
}

#[test]
fn existing_thread_initial_backfill_requires_items_or_authoritative_end() {
    assert!(super::initial_thread_backfill_is_authoritative(true, 0));
    assert!(super::initial_thread_backfill_is_authoritative(false, 1));
    assert!(!super::initial_thread_backfill_is_authoritative(false, 0));
}
