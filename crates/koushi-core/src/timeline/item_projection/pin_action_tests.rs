//! #1704: Pin is offered only for pinnable message-like events.
//!
//! The renderer used to offer Pin on every row with an event id, including
//! room notices such as the pinned-events notice itself. Rust now decides
//! with `actions.can_pin`, like the other row actions.

use std::collections::HashMap;

use matrix_sdk::{
    ruma::{event_id, events::room::message::RedactedRoomMessageEventContent, room_id, user_id},
    test_utils::mocks::MatrixMockServer,
};
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};
use matrix_sdk_ui::timeline::TimelineFocus;

use super::*;
use crate::timeline::relay::koushi_timeline_builder;

#[tokio::test]
async fn only_message_like_events_offer_pin() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!pin:example.invalid");
    let alice = user_id!("@alice:example.invalid");
    let message = event_id!("$pin-message:example.invalid");
    let member = event_id!("$pin-member:example.invalid");
    let pinned = event_id!("$pin-notice:example.invalid");
    let redacted = event_id!("$pin-redacted:example.invalid");
    let factory = EventFactory::new().room(room_id).sender(alice);
    let room = server.sync_joined_room(&client, room_id).await;
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(room_id)
                .add_timeline_event(factory.member(alice).event_id(member))
                .add_timeline_event(factory.text_msg("Synthetic message").event_id(message))
                .add_timeline_event(
                    factory
                        .room_pinned_events(vec![message.to_owned()])
                        .event_id(pinned),
                )
                .add_timeline_event(
                    factory
                        .redacted(alice, RedactedRoomMessageEventContent::new())
                        .event_id(redacted),
                ),
        )
        .await;
    let timeline = koushi_timeline_builder(
        &room,
        TimelineFocus::Live {
            hide_threaded_events: true,
        },
    )
    .build()
    .await
    .unwrap();
    let (items, _stream) = timeline.subscribe().await;
    let key = TimelineKey::room(
        koushi_protocol::ids::AccountKey(alice.to_string()),
        room_id.to_string(),
    );
    let can_pin = |event_id: &matrix_sdk::ruma::EventId| {
        items
            .iter()
            .find(|item| item.as_event().and_then(|event| event.event_id()) == Some(event_id))
            .map(|item| {
                sdk_item_to_timeline_item_with_send_states(
                    &key,
                    item,
                    Some(alice),
                    &HashMap::new(),
                    None,
                    None,
                    None,
                )
                .actions
                .can_pin
            })
    };

    assert_eq!(can_pin(message), Some(true), "a text message can be pinned");
    assert_eq!(
        can_pin(member),
        Some(false),
        "a membership notice cannot be pinned"
    );
    assert_eq!(
        can_pin(pinned),
        Some(false),
        "the pinned-events notice cannot be pinned"
    );
    assert_eq!(
        can_pin(redacted),
        Some(false),
        "a redacted event cannot be pinned"
    );
}
