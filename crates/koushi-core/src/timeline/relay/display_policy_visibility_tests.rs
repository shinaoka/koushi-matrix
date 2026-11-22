//! #1110/#2041: the display preferences are applied by Rust, not the renderer.
//!
//! A `DisplayPolicyChanged` delivery must recompute the authoritative row
//! visibility over the canonical window and publish the changed rows as
//! ordinary `ItemsUpdated` diffs, reversibly and without changing any other
//! row. Before this, the actor had no redaction input at all: the renderer
//! recomputed `is_hidden` from `hide_redacted` alone and lost every other
//! suppression reason.

use std::{sync::Arc, time::Duration};

use koushi_protocol::{
    event::{CoreEvent, TimelineEvent, TimelineItem, TimelineItemId},
    ids::{AccountKey, TimelineKey},
};
use koushi_sdk::MatrixClientSession;
use koushi_state::SessionInfo;
use matrix_sdk::{
    ruma::{
        event_id,
        events::room::{
            member::{MembershipState, RoomMemberEventContent},
            message::RedactedRoomMessageEventContent,
        },
        room_id, user_id,
    },
    test_utils::mocks::MatrixMockServer,
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

const MESSAGE_ID: &str = "$visibility-message:example.invalid";
const REDACTED_ID: &str = "$visibility-redacted:example.invalid";
const PROFILE_ID: &str = "$visibility-profile:example.invalid";
const JOIN_ID: &str = "$visibility-join:example.invalid";
const BAN_ID: &str = "$visibility-ban:example.invalid";
const ENCRYPTION_ID: &str = "$visibility-encryption:example.invalid";
const TOMBSTONE_ID: &str = "$visibility-tombstone:example.invalid";
const MEMBERSHIP_UPDATE_ID: &str = "$visibility-membership-update:example.invalid";

async fn wait_for_actor_events(
    events: &mut broadcast::Receiver<CoreEvent>,
    items: &mut Vec<TimelineItem>,
    predicate: impl Fn(&CoreEvent, &[TimelineItem]) -> bool,
) -> Result<(), tokio::time::error::Elapsed> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.expect("live actor event stream");
            match &event {
                CoreEvent::Timeline(TimelineEvent::InitialItems { items: initial, .. }) => {
                    *items = initial.clone();
                }
                CoreEvent::Timeline(TimelineEvent::ItemsUpdated { diffs, .. }) => {
                    apply_timeline_diffs_to_items(items, diffs);
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

fn hidden_state(items: &[TimelineItem], event_id: &str) -> Option<bool> {
    items.iter().find_map(|item| match &item.id {
        TimelineItemId::Event { event_id: id } if id == event_id => Some(item.is_hidden),
        _ => None,
    })
}

#[tokio::test]
async fn redaction_preference_toggle_is_published_as_row_diffs() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!visibility:example.invalid");
    let alice = user_id!("@alice:example.invalid");
    let factory = EventFactory::new().room(room_id);
    let room = server.sync_joined_room(&client, room_id).await;
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(room_id)
                .set_timeline_prev_batch("synthetic-before")
                .add_timeline_event(
                    factory
                        .text_msg("Synthetic visible message")
                        .sender(alice)
                        .event_id(event_id!("$visibility-message:example.invalid")),
                )
                .add_timeline_event(
                    factory
                        .redacted(alice, RedactedRoomMessageEventContent::new())
                        .sender(alice)
                        .event_id(event_id!("$visibility-redacted:example.invalid")),
                ),
        )
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
        false,
        true,
        manager.timeline_actor_generations.clone(),
        generation,
        None,
        Default::default(),
        manager.terminal_ingress.clone(),
        manager.msg_tx.clone(),
        None,
    )
    .await;

    let mut items = Vec::new();
    let outcome = async {
        wait_for_actor_events(&mut events, &mut items, |event, _| {
            matches!(
                event,
                CoreEvent::Timeline(TimelineEvent::InitialItems { .. })
            )
        })
        .await?;
        assert_eq!(
            hidden_state(&items, REDACTED_ID),
            Some(false),
            "hide_redacted=false keeps the redacted placeholder row"
        );
        assert_eq!(hidden_state(&items, MESSAGE_ID), Some(false));

        actor
            .send(TimelineActorMessage::DisplayPolicyChanged {
                thread_root_order: manager.thread_root_order,
                hide_redacted: true,
                show_room_and_member_changes: true,
            })
            .await;
        wait_for_actor_events(&mut events, &mut items, |_, items| {
            hidden_state(items, REDACTED_ID) == Some(true)
        })
        .await?;
        assert_eq!(
            hidden_state(&items, MESSAGE_ID),
            Some(false),
            "hiding redacted rows must not touch an ordinary message"
        );

        actor
            .send(TimelineActorMessage::DisplayPolicyChanged {
                thread_root_order: manager.thread_root_order,
                hide_redacted: false,
                show_room_and_member_changes: true,
            })
            .await;
        wait_for_actor_events(&mut events, &mut items, |_, items| {
            hidden_state(items, REDACTED_ID) == Some(false)
        })
        .await?;
        assert_eq!(hidden_state(&items, MESSAGE_ID), Some(false));
        Ok::<(), tokio::time::error::Elapsed>(())
    }
    .await;

    drain.abort();
    outcome.expect("#1110: the redaction preference must be published as row diffs");
}

/// #2041: the "show room and member changes" preference hides profile and
/// membership notices, and never the rows a reader needs for safety.
///
/// The four rows it must not hide are checked here as rows rather than as a
/// rule, because the reason they survive is that their projection never sets a
/// notice category at all: encryption and a room upgrade are room state, and a
/// ban is a moderation action, not a join or a leave.
#[tokio::test]
async fn room_and_member_change_preference_hides_only_profile_and_membership_rows() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!notice-visibility:example.invalid");
    let alice = user_id!("@alice:example.invalid");
    let bob = user_id!("@bob:example.invalid");
    let factory = EventFactory::new().room(room_id);
    let state_factory = EventFactory::new().room(room_id).sender(alice);
    let joined = RoomMemberEventContent::new(MembershipState::Join);
    let room = server.sync_joined_room(&client, room_id).await;
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(room_id)
                .set_timeline_prev_batch("synthetic-before")
                .add_state_event(factory.member(alice).into_raw_sync_state())
                .add_state_event(factory.member(bob).into_raw_sync_state())
                .add_timeline_event(
                    factory
                        .member(alice)
                        .prev_content(joined.clone())
                        .display_name("Alice Renamed")
                        .event_id(event_id!("$visibility-profile:example.invalid"))
                        .into_raw_sync(),
                )
                .add_timeline_event(
                    factory
                        .member(bob)
                        .event_id(event_id!("$visibility-join:example.invalid"))
                        .into_raw_sync(),
                )
                .add_timeline_event(
                    factory
                        .event(RoomMemberEventContent::new(MembershipState::Ban))
                        .sender(alice)
                        .state_key(bob)
                        .prev_content(joined.clone())
                        .event_id(event_id!("$visibility-ban:example.invalid"))
                        .into_raw_sync(),
                )
                // A member event that changes no membership: the SDK reports
                // it with no change at all. It may carry a power level, so it
                // is not a join or a leave and must never be hidden by this
                // preference.
                .add_timeline_event(
                    factory
                        .event(RoomMemberEventContent::new(MembershipState::Join))
                        .sender(alice)
                        .state_key(bob)
                        .prev_content(joined.clone())
                        .event_id(event_id!("$visibility-membership-update:example.invalid"))
                        .into_raw_sync(),
                )
                .add_timeline_event(
                    state_factory
                        .room_encryption()
                        .event_id(event_id!("$visibility-encryption:example.invalid"))
                        .into_raw_sync(),
                )
                .add_timeline_event(
                    state_factory
                        .room_tombstone("upgraded", room_id!("!next:example.invalid"))
                        .event_id(event_id!("$visibility-tombstone:example.invalid"))
                        .into_raw_sync(),
                ),
        )
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
        fake_rid(2),
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
        false,
        true,
        manager.timeline_actor_generations.clone(),
        generation,
        None,
        Default::default(),
        manager.terminal_ingress.clone(),
        manager.msg_tx.clone(),
        None,
    )
    .await;

    let mut items = Vec::new();
    let outcome = async {
        wait_for_actor_events(&mut events, &mut items, |event, _| {
            matches!(
                event,
                CoreEvent::Timeline(TimelineEvent::InitialItems { .. })
            )
        })
        .await?;
        for event_id in [
            PROFILE_ID,
            JOIN_ID,
            BAN_ID,
            ENCRYPTION_ID,
            TOMBSTONE_ID,
            MEMBERSHIP_UPDATE_ID,
        ] {
            assert_eq!(
                hidden_state(&items, event_id),
                Some(false),
                "{event_id} is visible while the preference is on"
            );
        }

        actor
            .send(TimelineActorMessage::DisplayPolicyChanged {
                thread_root_order: manager.thread_root_order,
                hide_redacted: false,
                show_room_and_member_changes: false,
            })
            .await;
        wait_for_actor_events(&mut events, &mut items, |_, items| {
            hidden_state(items, JOIN_ID) == Some(true)
        })
        .await?;
        assert_eq!(
            hidden_state(&items, PROFILE_ID),
            Some(true),
            "a profile change is hidden with the preference off"
        );
        assert_eq!(
            hidden_state(&items, JOIN_ID),
            Some(true),
            "a membership change is hidden with the preference off"
        );
        for event_id in [BAN_ID, ENCRYPTION_ID, TOMBSTONE_ID, MEMBERSHIP_UPDATE_ID] {
            assert_eq!(
                hidden_state(&items, event_id),
                Some(false),
                "{event_id} stays visible with the preference off"
            );
        }

        actor
            .send(TimelineActorMessage::DisplayPolicyChanged {
                thread_root_order: manager.thread_root_order,
                hide_redacted: false,
                show_room_and_member_changes: true,
            })
            .await;
        wait_for_actor_events(&mut events, &mut items, |_, items| {
            hidden_state(items, PROFILE_ID) == Some(false)
                && hidden_state(items, JOIN_ID) == Some(false)
        })
        .await?;
        Ok::<(), tokio::time::error::Elapsed>(())
    }
    .await;

    drain.abort();
    outcome.expect("#2041: the preference must be published as row diffs, reversibly");
}
