//! #1704: Pin is offered only for pinnable message-like events.
//!
//! The renderer used to offer Pin on every row with an event id, including
//! room notices such as the pinned-events notice itself. Rust now decides
//! with `actions.can_pin`, like the other row actions.

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

/// #2151: the Jump bar's "new messages" count (`counts_as_message`) reads
/// `actions.can_pin` deliberately, because it is the projection's existing
/// "this is a message-like event" fact. This test keeps the two in step: it
/// builds the rows through the real projection, so if either side changes
/// meaning, this fails instead of a count quietly moving.
#[tokio::test]
async fn counts_as_message_and_can_pin_agree_on_state_notices_and_bot_notices() {
    use matrix_sdk::ruma::events::room::member::MembershipState;

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!count:example.invalid");
    let alice = user_id!("@alice:example.invalid");
    let bob = user_id!("@bob:example.invalid");
    let bot = user_id!("@bot:example.invalid");
    let join = event_id!("$count-join:example.invalid");
    let kick = event_id!("$count-kick:example.invalid");
    let bot_notice = event_id!("$count-bot:example.invalid");
    let factory = || EventFactory::new().room(room_id);
    let room = server.sync_joined_room(&client, room_id).await;
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(room_id)
                .add_timeline_event(factory().sender(bob).member(bob).event_id(join))
                .add_timeline_event(
                    factory()
                        .sender(alice)
                        .member(bob)
                        .membership(MembershipState::Leave)
                        .previous(matrix_sdk_test::event_factory::PreviousMembership::new(
                            MembershipState::Join,
                        ))
                        .event_id(kick),
                )
                .add_timeline_event(
                    factory()
                        .sender(bot)
                        .notice("deploy done")
                        .event_id(bot_notice),
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
    let project = |event_id: &matrix_sdk::ruma::EventId| {
        items
            .iter()
            .find(|item| item.as_event().and_then(|event| event.event_id()) == Some(event_id))
            .map(|item| {
                sdk_item_to_timeline_item_with_send_states(
                    &key,
                    item,
                    Some(alice),
                    None,
                    None,
                    None,
                )
            })
            .expect("the synced event is in the timeline")
    };

    for (id, what) in [(join, "a join"), (kick, "a kick")] {
        let row = project(id);
        assert!(
            !counts_as_message(&row) && !row.actions.can_pin,
            "{what} is a state notice: neither a counted message nor pinnable"
        );
    }
    let row = project(bot_notice);
    assert!(
        counts_as_message(&row) && row.actions.can_pin,
        "a bot's m.notice is a message: counted and pinnable (the count reads can_pin as the \
         projection's message-like fact; this test keeps the two in step)"
    );
}
