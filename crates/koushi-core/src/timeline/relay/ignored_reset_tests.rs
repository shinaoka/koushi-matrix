use std::{collections::BTreeSet, sync::Arc, time::Duration};

use koushi_protocol::{
    event::{CoreEvent, PaginationDirection, PaginationState, TimelineEvent, TimelineItem},
    ids::{AccountKey, TimelineKey},
};
use koushi_sdk::MatrixClientSession;
use koushi_state::SessionInfo;
use matrix_sdk::{
    ruma::{event_id, room_id, user_id},
    test_utils::mocks::{MatrixMockServer, RoomMessagesResponseTemplate},
};
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};
use matrix_sdk_ui::timeline::TimelineFocus;
use tokio::sync::{broadcast, mpsc};

use super::super::{
    actor::{TimelineActor, TimelineActorMessage},
    display_projection::apply_timeline_diffs_to_items,
    test_support::{fake_rid, live_tail_test_manager},
};
use super::koushi_timeline_builder;
use crate::{executor, link_preview::LinkPreviewContext};

async fn wait_for(
    events: &mut broadcast::Receiver<CoreEvent>,
    items: &mut Vec<TimelineItem>,
    predicate: impl Fn(&CoreEvent, &[TimelineItem]) -> bool,
) -> Result<(), tokio::time::error::Elapsed> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.expect("live actor event stream");
            match &event {
                CoreEvent::Timeline(TimelineEvent::InitialItems { items: initial, .. }) => {
                    *items = initial.clone()
                }
                CoreEvent::Timeline(TimelineEvent::ItemsUpdated { diffs, .. }) => {
                    apply_timeline_diffs_to_items(items, diffs)
                }
                _ => {}
            }
            if predicate(&event, items) {
                break;
            }
        }
    })
    .await
}

#[tokio::test]
async fn ignored_user_cache_reset_refills_without_new_events_or_viewport_requests() {
    exercise_ignored_reset(ResetCase::Recover).await;
}

#[tokio::test]
async fn explicit_cancel_stops_cache_reset_refill_without_rescheduling() {
    exercise_ignored_reset(ResetCase::Cancel).await;
}

#[tokio::test]
async fn failed_cache_reset_refill_reports_failure_without_automatic_retry() {
    exercise_ignored_reset(ResetCase::Fail).await;
}

#[derive(Clone, Copy)]
enum ResetCase {
    Recover,
    Cancel,
    Fail,
}

async fn message_request_count(server: &MatrixMockServer) -> usize {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|request| request.url.path().ends_with("/messages"))
        .count()
}

async fn exercise_ignored_reset(case: ResetCase) {
    let cancel_refill = matches!(case, ResetCase::Cancel);
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    client.event_cache().subscribe().unwrap();
    let room_id = room_id!("!ignore-reset:example.invalid");
    let alice = user_id!("@alice:example.invalid");
    let bob = user_id!("@bob:example.invalid");
    let factory = EventFactory::new().room(room_id);
    let first = || {
        factory
            .text_msg("Synthetic first")
            .sender(alice)
            .event_id(event_id!("$first:example.invalid"))
    };
    let ignored = || {
        factory
            .text_msg("Synthetic ignored")
            .sender(bob)
            .event_id(event_id!("$ignored:example.invalid"))
    };
    let last = || {
        factory
            .text_msg("Synthetic last")
            .sender(alice)
            .event_id(event_id!("$last:example.invalid"))
    };
    let room = server.sync_joined_room(&client, room_id).await;
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(room_id)
                .set_timeline_prev_batch("synthetic-before")
                .add_timeline_event(first())
                .add_timeline_event(ignored())
                .add_timeline_event(last()),
        )
        .await;
    server
        .mock_room_messages()
        .ok(RoomMessagesResponseTemplate::default().events(vec![last(), ignored(), first()]))
        .mount()
        .await;
    let timeline = Arc::new(
        koushi_timeline_builder(
            &room,
            TimelineFocus::Live {
                hide_threaded_events: true,
            },
        )
        .build()
        .await
        .unwrap(),
    );
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client.clone(),
        SessionInfo {
            homeserver: "https://example.invalid".into(),
            user_id: alice.to_string(),
            device_id: "SYNTHETIC".into(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    ));
    let key = TimelineKey::room(AccountKey(alice.to_string()), room_id.to_string());
    let mut manager = live_tail_test_manager(Default::default());
    let (action_tx, mut action_rx) = mpsc::channel(64);
    manager.action_tx = action_tx;
    let drain = executor::spawn(async move { while action_rx.recv().await.is_some() {} });
    manager.event_tx = broadcast::channel(128).0;
    let mut events = manager.event_tx.subscribe();
    let generation = manager
        .timeline_actor_generations
        .activate_after_quiescence(&key)
        .await
        .generation;
    let actor = TimelineActor::spawn(
        key,
        timeline,
        session,
        fake_rid(1),
        true,
        manager.action_tx.clone(),
        manager.event_tx.clone(),
        None,
        Default::default(),
        None,
        LinkPreviewContext::default(),
        manager.account_work.clone(),
        manager.thread_root_projection_service.clone(),
        manager.thread_root_order,
        manager.timeline_actor_generations.clone(),
        generation,
        None,
        Default::default(),
        manager.terminal_ingress.clone(),
        manager.msg_tx.clone(),
    )
    .await;
    let mut items = Vec::new();
    let mut stage = "initial";
    let outcome = async {
        wait_for(&mut events, &mut items, |event, _| {
            matches!(
                event,
                CoreEvent::Timeline(TimelineEvent::InitialItems { .. })
            )
        })
        .await?;
        stage = "initial-pagination";
        actor
            .send(TimelineActorMessage::Paginate {
                request_id: fake_rid(2),
                direction: PaginationDirection::Backward,
                event_count: 20,
            })
            .await;
        wait_for(&mut events, &mut items, |event, _| {
            matches!(
                event,
                CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                    state: PaginationState::EndReached,
                    ..
                })
            )
        })
        .await?;
        let initial_requests = message_request_count(&server).await;
        if cancel_refill {
            wiremock::Mock::given(wiremock::matchers::path_regex(r"/messages$"))
                .respond_with(
                    wiremock::ResponseTemplate::new(200)
                        .set_delay(Duration::from_secs(1))
                        .set_body_json(
                            serde_json::json!({"chunk": [], "start": "synthetic-start"}),
                        ),
                )
                .with_priority(1)
                .mount(server.server())
                .await;
        }
        if matches!(case, ResetCase::Fail) {
            wiremock::Mock::given(wiremock::matchers::path_regex(r"/messages$"))
                .respond_with(wiremock::ResponseTemplate::new(403).set_body_json(
                    serde_json::json!({"errcode": "M_FORBIDDEN", "error": "synthetic denial"}),
                ))
                .with_priority(1)
                .mount(server.server())
                .await;
        }
        for ignore in [true, false] {
            let users = if ignore { vec![bob.to_owned()] } else { vec![] };
            actor
                .send(TimelineActorMessage::IgnoredUsersUpdated(
                    users
                        .iter()
                        .map(ToString::to_string)
                        .collect::<BTreeSet<_>>(),
                ))
                .await;
            server
                .mock_sync()
                .ok_and_run(&client, |builder| {
                    builder.add_global_account_data(factory.ignored_user_list(users));
                })
                .await;
            stage = if ignore {
                "ignore-clear"
            } else {
                "unignore-clear"
            };
            wait_for(&mut events, &mut items, |_, items| {
                !items.iter().any(|item| item.sender.is_some())
            })
            .await?;
            if matches!(case, ResetCase::Fail) {
                stage = "refill-failed";
                wait_for(&mut events, &mut items, |event, _| {
                    matches!(
                        event,
                        CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                            state: PaginationState::Failed { .. },
                            ..
                        })
                    )
                })
                .await?;
                // This non-retryable response has completed; observe a bounded
                // quiet interval for an accidental actor-driven retry loop.
                tokio::time::sleep(Duration::from_millis(300)).await;
                assert_eq!(message_request_count(&server).await, initial_requests + 1);
                break;
            }
            if cancel_refill {
                stage = "refill-request-started";
                tokio::time::timeout(Duration::from_secs(2), async {
                    while message_request_count(&server).await == initial_requests {
                        tokio::task::yield_now().await;
                    }
                })
                .await?;
                actor
                    .send(TimelineActorMessage::CancelPagination {
                        request_id: fake_rid(3),
                    })
                    .await;
                stage = "refill-cancelled";
                wait_for(&mut events, &mut items, |event, _| {
                    matches!(event,
                        CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                            request_id: Some(id), state: PaginationState::Idle, ..
                        }) if *id == fake_rid(3)
                    )
                })
                .await?;
                // Wait past the controlled server response: cancellation must not
                // schedule a replacement request when the old request finishes.
                tokio::time::sleep(Duration::from_millis(1200)).await;
                assert_eq!(message_request_count(&server).await, initial_requests + 1);
                break;
            }
            stage = if ignore {
                "ignore-refill"
            } else {
                "unignore-refill"
            };
            wait_for(&mut events, &mut items, |_, items| {
                items
                    .iter()
                    .filter(|item| item.sender.is_some() && !item.is_hidden)
                    .count()
                    == if ignore { 2 } else { 3 }
            })
            .await?;
        }
        Ok::<_, tokio::time::error::Elapsed>(())
    }
    .await;
    actor.stop().await;
    drain.abort();
    let _ = drain.await;
    assert!(
        outcome.is_ok(),
        "stage={stage}: ignore/unignore must refill after the SDK clear without a new event, restart, or viewport request"
    );
}
