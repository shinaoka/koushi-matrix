use super::*;
use crate::ids::{AccountKey, TimelineBatchId, TimelineGeneration, TimelineKey};

const PRIVATE_THREAD_ROOT: &str = "$private-thread:example.invalid";
const PRIVATE_PARENT: &str = "$private-parent:example.invalid";
const PRIVATE_EVENT_ID: &str = "$private-event:example.invalid";
const PRIVATE_SENDER: &str = "@private-sender:example.invalid";
const PRIVATE_SENDER_LABEL: &str = "Private Alias";
const PRIVATE_REACTOR: &str = "@private-reactor:example.invalid";
const PRIVATE_REACTOR_LABEL: &str = "Private Reactor";
const PRIVATE_REACTION_EVENT_ID: &str = "$private-reaction:example.invalid";
const PRIVATE_REACTION_KEY: &str = "private-reaction-key";
const PRIVATE_ROOM_ID: &str = "!private-room:example.invalid";
const PRIVATE_ACCOUNT_ID: &str = "@private-account:example.invalid";

fn private_item() -> TimelineItem {
    TimelineItem {
        id: TimelineItemId::Event {
            event_id: PRIVATE_EVENT_ID.to_owned(),
        },
        sender: Some(PRIVATE_SENDER.to_owned()),
        sender_label: Some(PRIVATE_SENDER_LABEL.to_owned()),
        sender_avatar: None,
        body: Some("private body".to_owned()),
        notice_i18n: None,
        message_kind: Default::default(),
        spoiler_spans: Vec::new(),
        timestamp_ms: Some(1_700_000_000_000),
        in_reply_to_event_id: Some(PRIVATE_PARENT.to_owned()),
        formatted: None,
        reply_quote: None,
        thread_root: Some(PRIVATE_THREAD_ROOT.to_owned()),
        thread_summary: None,
        media: None,
        link_previews: None,
        link_ranges: Vec::new(),
        mentioned_user_ids: Vec::new(),
        mentions_room: false,
        reactions: vec![ReactionGroup {
            key: PRIVATE_REACTION_KEY.to_owned(),
            count: 3,
            reacted_by_me: true,
            my_reaction_event_id: Some(PRIVATE_REACTION_EVENT_ID.to_owned()),
            sender_preview: vec![ReactionSender {
                user_id: PRIVATE_REACTOR.to_owned(),
                display_label: Some(PRIVATE_REACTOR_LABEL.to_owned()),
            }],
        }],
        can_react: true,
        is_redacted: false,
        is_hidden: false,
        can_redact: false,
        is_edited: false,
        can_edit: false,
        unable_to_decrypt: None,
        request_state: None,
        actions: TimelineMessageActions::default(),
        send_state: None,
        display_metadata: None,
    }
}

/// Every raw identity the fixture carries, so one list covers each debug path.
fn private_values() -> [&'static str; 10] {
    [
        PRIVATE_THREAD_ROOT,
        PRIVATE_PARENT,
        PRIVATE_EVENT_ID,
        PRIVATE_SENDER,
        PRIVATE_SENDER_LABEL,
        PRIVATE_REACTOR,
        PRIVATE_REACTOR_LABEL,
        PRIVATE_REACTION_EVENT_ID,
        PRIVATE_REACTION_KEY,
        PRIVATE_ROOM_ID,
    ]
}

fn assert_redacted(debug: &str) {
    for raw in private_values() {
        assert!(!debug.contains(raw), "raw {raw} leaked: {debug}");
    }
    assert!(!debug.contains(":example.invalid"), "{debug}");
    assert!(!debug.contains("private body"), "{debug}");
    for placeholder in [
        "EventId(..)",
        "UserId(..)",
        "SenderLabel(..)",
        "ReactionKey(..)",
    ] {
        assert!(
            debug.contains(placeholder),
            "missing {placeholder}: {debug}"
        );
    }
    // Counts, booleans and kinds stay, because they carry no identity.
    assert!(debug.contains("count: 3"), "{debug}");
    assert!(debug.contains("reacted_by_me: true"), "{debug}");
}

#[test]
fn timeline_item_debug_redacts_the_thread_root_and_reaction_identities() {
    assert_redacted(&format!("{:?}", private_item()));
}

#[test]
fn timeline_event_debug_redacts_the_nested_item() {
    let key = TimelineKey::room(AccountKey(PRIVATE_ACCOUNT_ID.to_owned()), PRIVATE_ROOM_ID);
    let event = TimelineEvent::ItemsUpdated {
        key,
        generation: TimelineGeneration(4),
        batch_id: TimelineBatchId(9),
        diffs: vec![TimelineDiff::Set {
            index: 0,
            item: private_item(),
        }],
    };

    let debug = format!("{event:?}");
    assert_redacted(&debug);
    // The event's own key is redacted too, not only the nested item's fields.
    assert!(!debug.contains(PRIVATE_ACCOUNT_ID), "{debug}");
    assert!(debug.contains("TimelineKey(..)"), "{debug}");
}
