use super::fixture::{Fixture, TARGET};
use crate::{
    link_preview::LinkPreviewContext,
    timeline::{
        item_projection::apply_link_previews_to_item,
        manager::TimelineMessage,
        test_support::{fake_rid, timeline_item},
    },
};
use koushi_protocol::{
    command::TimelineCommand,
    event::{CoreEvent, TimelineDiff, TimelineEvent},
};
use koushi_state::{ComposerDocument, ComposerFormattingOptions, SessionInfo};
use matrix_sdk::{
    ruma::{event_id, room_id},
    test_utils::mocks::MatrixMockServer,
};
use matrix_sdk_test::{User, event_factory::EventFactory};
use std::sync::Arc;

#[tokio::test]
async fn edit_uses_command_formatting_options_in_the_sdk_local_echo() {
    let mut fixture = Fixture::new(true).await;
    fixture
        .context
        .timeline
        .room()
        .send_queue()
        .set_enabled(false);
    let factory = EventFactory::new()
        .room(room_id!("!mutation:example.invalid"))
        .sender(User::Alice);
    fixture
        .server
        .mock_room_event()
        .expect_access_token("synthetic")
        .ok(factory
            .text_msg("Synthetic original")
            .event_id(event_id!("$mutation:example.invalid"))
            .into_event())
        .mount()
        .await;
    fixture
        .manager
        .send(TimelineMessage::CommandWithComposerFormatting {
            command: TimelineCommand::EditText {
                request_id: fake_rid(600),
                key: fixture.key.clone(),
                event_id: TARGET.into(),
                document: ComposerDocument::from_plain_text("Energy $E=mc^2$"),
            },
            formatting_options: ComposerFormattingOptions { math_mode: false },
        })
        .await
        .unwrap();
    fixture
        .wait_for(|event| {
            if let CoreEvent::Timeline(TimelineEvent::ItemsUpdated { diffs, .. }) = event {
                diffs.iter().any(|diff| match diff {
                    TimelineDiff::Set { item, .. }
                        if item.body.as_deref() == Some("Energy $E=mc^2$") =>
                    {
                        assert!(
                            item.formatted
                                .as_ref()
                                .is_none_or(|body| !body.html.contains("data-mx-maths")),
                            "math-mode false must reach the edit worker"
                        );
                        true
                    }
                    _ => false,
                })
            } else {
                false
            }
        })
        .await;
    fixture.stop().await;
}

#[tokio::test]
async fn unknown_or_missing_room_preview_projection_never_fetches_state() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!unknown-encryption:example.invalid");
    let room = server.sync_joined_room(&client, room_id).await;
    assert!(room.encryption_state().is_unknown());
    let session = Arc::new(koushi_sdk::MatrixClientSession::from_client_for_testing(
        client,
        SessionInfo {
            homeserver: server.uri(),
            user_id: "@example:localhost".into(),
            device_id: "SYNTHETIC".into(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    ));
    for target in [room_id.as_str(), "!missing:example.invalid"] {
        for unencrypted in [false, true] {
            for encrypted in [false, true] {
                for override_ in [None, Some(false), Some(true)] {
                    let mut item = timeline_item(
                        "$preview:example.invalid",
                        None,
                        "@member:example.invalid",
                        false,
                    );
                    item.body = Some("https://example.invalid/synthetic".into());
                    let context = LinkPreviewContext {
                        unencrypted_global_enabled: unencrypted,
                        encrypted_global_enabled: encrypted,
                        room_enabled: override_,
                        ..Default::default()
                    };
                    apply_link_previews_to_item(&mut item, target, &context, &session).await;
                    assert_eq!(
                        item.link_previews.is_some(),
                        override_.unwrap_or(unencrypted && encrypted)
                    );
                }
            }
        }
    }
    let requests = server.server().received_requests().await.unwrap();
    assert!(
        !requests
            .iter()
            .any(|request| request.url.path().contains("m.room.encryption")),
        "projection cannot issue an encryption-state HTTP request"
    );
}

#[tokio::test]
async fn adding_again_does_not_toggle_off_an_own_pending_local_reaction() {
    use koushi_protocol::failure::{CoreFailure, TimelineFailureKind};
    let mut fixture = Fixture::new(true).await;
    fixture
        .context
        .timeline
        .room()
        .send_queue()
        .set_enabled(false);
    fixture
        .command(TimelineCommand::SendReaction {
            request_id: fake_rid(610),
            key: fixture.key.clone(),
            event_id: TARGET.into(),
            reaction_key: "👍".into(),
        })
        .await;
    fixture.wait_for(|event| matches!(event, CoreEvent::Timeline(TimelineEvent::ItemsUpdated { diffs, .. }) if diffs.iter().any(|diff| matches!(diff, TimelineDiff::Set { item, .. } if item.reactions.iter().any(|reaction| reaction.reacted_by_me && reaction.my_reaction_event_id.is_none()))))).await;
    fixture
        .command(TimelineCommand::SendReaction {
            request_id: fake_rid(611),
            key: fixture.key.clone(),
            event_id: TARGET.into(),
            reaction_key: "👍".into(),
        })
        .await;
    fixture.wait_for(|event| matches!(event, CoreEvent::OperationFailed { request_id, failure: CoreFailure::TimelineOperationFailed { kind: TimelineFailureKind::InvalidReactionState } } if *request_id == fake_rid(611))).await;
    let items = fixture.context.timeline.items().await;
    let item = items
        .iter()
        .find(|item| {
            item.as_event()
                .is_some_and(|item| item.event_id().is_some_and(|id| id.as_str() == TARGET))
        })
        .unwrap();
    let projected = crate::timeline::item_projection::sdk_item_to_timeline_item(
        &fixture.key,
        item,
        Some(fixture.context.timeline.room().own_user_id()),
    );
    assert!(
        projected
            .reactions
            .iter()
            .any(|reaction| reaction.reacted_by_me),
        "invalid second add must not abort the existing SDK local reaction"
    );
    fixture.stop().await;
}
