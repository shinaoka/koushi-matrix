//! Event handlers that feed one account's [`MentionFeed`].

use std::sync::Arc;

use matrix_sdk::event_handler::EventHandlerHandle;
use matrix_sdk::ruma::OwnedUserId;
use matrix_sdk::ruma::events::room::member::{MembershipState, StrippedRoomMemberEvent};
use matrix_sdk::ruma::events::room::message::{OriginalSyncRoomMessageEvent, Relation};
use matrix_sdk::ruma::push::Action;
use matrix_sdk::{Client, Room, RoomState};

use super::{MentionCandidate, MentionFeed, MentionFeedRegistry, MentionKind};

/// The decision for one message, kept free of SDK types so it is testable.
pub(crate) struct MessageFacts<'a> {
    pub(crate) own_user_id: &'a str,
    pub(crate) sender: &'a str,
    pub(crate) is_replacement: bool,
    /// The push rules highlighted the event (mention or keyword).
    pub(crate) push_highlight: bool,
    /// `m.mentions.user_ids` names the account.
    pub(crate) names_account: bool,
    pub(crate) is_direct_room: bool,
}

/// `None` when the event is not for the account's feed. A mute, a dropped
/// notification or the owner's do-not-disturb never reaches this decision:
/// `names_account` reads the event itself, so a muted room still counts.
pub(crate) fn classify_message(facts: &MessageFacts<'_>) -> Option<MentionKind> {
    if facts.sender == facts.own_user_id || facts.is_replacement {
        return None;
    }
    if facts.push_highlight || facts.names_account {
        Some(MentionKind::Mention)
    } else if facts.is_direct_room {
        Some(MentionKind::Dm)
    } else {
        None
    }
}

#[derive(Clone)]
pub struct MentionIntake {
    feed: MentionFeed,
    registry: Arc<MentionFeedRegistry>,
    own_user_id: OwnedUserId,
}

impl MentionIntake {
    pub fn new(
        feed: MentionFeed,
        registry: Arc<MentionFeedRegistry>,
        own_user_id: OwnedUserId,
    ) -> Self {
        Self {
            feed,
            registry,
            own_user_id,
        }
    }

    /// The allow-list is enforced here, before anything is stored.
    pub(crate) async fn offer(&self, candidate: MentionCandidate) {
        if self.registry.sender_allowed(&candidate.sender) {
            self.feed.record(candidate).await;
        }
    }

    /// Registers the handlers on `client`. They run inside the SDK's sync
    /// processing and are awaited there, so a row is committed before the sync
    /// response is acknowledged.
    pub fn register(&self, client: &Client) -> Vec<EventHandlerHandle> {
        let message = self.clone();
        let messages = client.add_event_handler(
            move |event: OriginalSyncRoomMessageEvent, room: Room, actions: Vec<Action>| {
                let intake = message.clone();
                async move { intake.on_message(event, room, actions).await }
            },
        );
        let invite = self.clone();
        let invites =
            client.add_event_handler(move |event: StrippedRoomMemberEvent, room: Room| {
                let intake = invite.clone();
                async move { intake.on_invite(event, room).await }
            });
        vec![messages, invites]
    }

    async fn on_message(
        &self,
        event: OriginalSyncRoomMessageEvent,
        room: Room,
        actions: Vec<Action>,
    ) {
        let timestamp_ms: u64 = event.origin_server_ts.0.into();
        // History replayed by a first sync is not news.
        if timestamp_ms < self.feed.created_at_ms() {
            return;
        }
        let names_account = event
            .content
            .mentions
            .as_ref()
            .is_some_and(|mentions| mentions.user_ids.contains(&self.own_user_id));
        let push_highlight = actions.iter().any(Action::is_highlight);
        let is_direct_room = if push_highlight || names_account {
            false
        } else {
            room.is_direct().await.unwrap_or(false)
        };
        let Some(kind) = classify_message(&MessageFacts {
            own_user_id: self.own_user_id.as_str(),
            sender: event.sender.as_str(),
            is_replacement: matches!(event.content.relates_to, Some(Relation::Replacement(_))),
            push_highlight,
            names_account,
            is_direct_room,
        }) else {
            return;
        };
        let thread_root = match &event.content.relates_to {
            Some(Relation::Thread(thread)) => Some(thread.event_id.to_string()),
            _ => None,
        };
        self.offer(MentionCandidate {
            room_id: room.room_id().to_string(),
            event_id: Some(event.event_id.to_string()),
            sender: event.sender.to_string(),
            timestamp_ms,
            thread_root,
            kind,
        })
        .await;
    }

    async fn on_invite(&self, event: StrippedRoomMemberEvent, room: Room) {
        if room.state() != RoomState::Invited
            || event.state_key != self.own_user_id
            || event.content.membership != MembershipState::Invite
            || event.sender == self.own_user_id
        {
            return;
        }
        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as u64)
            .unwrap_or(0);
        self.offer(MentionCandidate {
            room_id: room.room_id().to_string(),
            event_id: None,
            sender: event.sender.to_string(),
            timestamp_ms,
            thread_root: None,
            kind: MentionKind::Invite,
        })
        .await;
    }
}
