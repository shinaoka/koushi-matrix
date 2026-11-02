//! #1482 acceptance: mutation progress without the presentation actor, and
//! refusal of a command whose target another view owns.

use super::fixture::{Fixture, LIVENESS, REACTION, TARGET};
use crate::timeline::{
    item_projection::sdk_item_to_timeline_item,
    mutation::{Mutation, MutationContext, MutationLane},
    relay::koushi_timeline_builder,
    test_support::fake_rid,
};
use koushi_protocol::{
    command::TimelineCommand,
    event::CoreEvent,
    failure::{CoreFailure, TimelineFailureKind},
    ids::{AccountKey, TimelineKey, TimelineKind},
};
use matrix_sdk::ruma::{event_id, room_id};
use matrix_sdk_test::{User, event_factory::EventFactory};
use matrix_sdk_ui::timeline::{TimelineEventFocusThreadMode, TimelineFocus};
use std::sync::Arc;
use tokio::sync::broadcast;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{method, path_regex},
};

/// A presentation actor that cannot accept mailbox messages at all cannot gate a
/// mutation. If admission still ran through that mailbox, the send would fail and
/// the operation would never reach the SDK.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mutation_progress_does_not_depend_on_actor_mailbox_capacity() {
    let mut fixture = Fixture::new(false).await;
    Mock::given(method("PUT"))
        .and(path_regex("/redact/"))
        .respond_with(ResponseTemplate::new(403).set_body_json(
            serde_json::json!({ "errcode": "M_FORBIDDEN", "error": "Synthetic refusal" }),
        ))
        .expect(1)
        .mount(fixture.server.server())
        .await;
    fixture.stop_actor();
    while !fixture.actor.is_closed() {
        tokio::task::yield_now().await;
    }
    fixture
        .command(TimelineCommand::RedactReaction {
            request_id: fake_rid(640),
            key: fixture.key.clone(),
            event_id: TARGET.into(),
            reaction_key: "👍".into(),
            reaction_event_id: REACTION.into(),
        })
        .await;
    fixture
        .wait_for(|event| {
            matches!(event, CoreEvent::OperationFailed { request_id, failure: CoreFailure::TimelineOperationFailed { kind: TimelineFailureKind::Sdk } } if *request_id == fake_rid(640))
        })
        .await;
    let requests = fixture.server.server().received_requests().await.unwrap();
    assert!(
        requests
            .iter()
            .any(|request| request.url.path().contains("/redact/")),
        "the mutation reached real SDK HTTP with no actor mailbox to admit through"
    );
    fixture.stop().await;
}

/// A command admitted from a view that cannot see the target refuses instead of
/// blindly toggling state another view owns.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cross_view_reaction_removal_refuses_instead_of_toggling() {
    let fixture = Fixture::new(false).await;
    let elsewhere = event_id!("$elsewhere:example.invalid");
    let factory = EventFactory::new()
        .room(room_id!("!mutation:example.invalid"))
        .sender(User::Alice);
    Mock::given(method("GET"))
        .and(path_regex("/context/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "event": factory
                .text_msg("Synthetic elsewhere")
                .event_id(elsewhere)
                .into_event()
                .into_raw()
                .json(),
            "events_before": [], "events_after": [], "state": []
        })))
        .mount(fixture.server.server())
        .await;
    // Removing a reaction belongs to the view that owns it; a cross-view command
    // must not reach HTTP at all.
    Mock::given(method("PUT"))
        .and(path_regex("/redact/"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "event_id": "$redacted:example.invalid" })),
        )
        .expect(0)
        .mount(fixture.server.server())
        .await;
    let room = fixture.context.timeline.room().clone();
    let other_timeline = Arc::new(
        koushi_timeline_builder(
            &room,
            TimelineFocus::Event {
                target: elsewhere.to_owned(),
                num_context_events: 0,
                thread_mode: TimelineEventFocusThreadMode::Automatic {
                    hide_threaded_events: false,
                },
            },
        )
        .build()
        .await
        .unwrap(),
    );
    assert!(
        !other_timeline.items().await.iter().any(|item| item
            .as_event()
            .and_then(|event| event.event_id())
            .is_some_and(|id| id.as_str() == TARGET)),
        "the originating view cannot see the target"
    );
    let other_key = TimelineKey {
        account_key: AccountKey(User::Alice.to_string()),
        kind: TimelineKind::Focused {
            room_id: room_id!("!mutation:example.invalid").to_string(),
            event_id: elsewhere.to_string(),
        },
    };
    let generation = fixture
        .context
        .gate
        .activate_after_quiescence(&other_key)
        .await
        .generation;
    let context = MutationContext {
        key: other_key,
        generation,
        gate: fixture.context.gate.clone(),
        timeline: other_timeline,
        account_work: fixture.context.account_work.clone(),
        sent_event_txns: fixture.context.sent_event_txns.clone(),
    };
    let mut lane = MutationLane::default();
    let (events, mut terminal) = broadcast::channel(16);
    lane.admit(
        context,
        fake_rid(641),
        Mutation::RemoveReaction {
            event_id: TARGET.into(),
            reaction_key: "👍".into(),
            reaction_event_id: REACTION.into(),
        },
        events,
    );
    let failure = tokio::time::timeout(LIVENESS, terminal.recv())
        .await
        .expect("cross-view refusal settles")
        .unwrap();
    assert!(
        matches!(
            failure,
            CoreEvent::OperationFailed {
                request_id,
                failure: CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::InvalidReactionTarget
                }
            } if request_id == fake_rid(641)
        ),
        "an unseen target is refused, not toggled: {failure:?}"
    );
    let items = fixture.context.timeline.items().await;
    let target = items
        .iter()
        .find(|item| {
            item.as_event()
                .is_some_and(|event| event.event_id().is_some_and(|id| id.as_str() == TARGET))
        })
        .expect("the owning view still holds the target");
    let projected = sdk_item_to_timeline_item(
        &fixture.key,
        target,
        Some(fixture.context.timeline.room().own_user_id()),
    );
    assert!(
        projected
            .reactions
            .iter()
            .any(|reaction| reaction.my_reaction_event_id.as_deref() == Some(REACTION)),
        "the owning view keeps its own reaction"
    );
    fixture.stop().await;
    lane.stop().await;
}
