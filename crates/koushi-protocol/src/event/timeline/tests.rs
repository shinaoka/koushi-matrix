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

// #1979: the threads-list event printed its room id and every item's room id,
// root event id, senders, and body previews.
fn private_thread_item() -> ThreadsListItem {
    ThreadsListItem {
        room_id: PRIVATE_ROOM_ID.to_owned(),
        root_event_id: PRIVATE_THREAD_ROOT.to_owned(),
        root_sender: PRIVATE_SENDER.to_owned(),
        root_sender_label: Some(PRIVATE_SENDER_LABEL.to_owned()),
        root_body_preview: Some("private thread body".to_owned()),
        root_timestamp_ms: Some(1_700_000_000_000),
        latest_event_id: Some(PRIVATE_EVENT_ID.to_owned()),
        latest_sender: Some(PRIVATE_REACTOR.to_owned()),
        latest_sender_label: Some(PRIVATE_REACTOR_LABEL.to_owned()),
        latest_body_preview: Some("private latest body".to_owned()),
        latest_timestamp_ms: Some(1_700_000_001_000),
        reply_count: 3,
        unread_count: 0,
    }
}

#[test]
fn threads_list_event_debug_counts_items_instead_of_printing_them() {
    let items = vec![private_thread_item(), private_thread_item()];
    let events = vec![
        ThreadsListEvent::Opened {
            request_id: crate::ids::RequestId {
                connection_id: crate::ids::RuntimeConnectionId(1),
                sequence: 2,
            },
            room_id: PRIVATE_ROOM_ID.to_owned(),
            items: items.clone(),
            end_reached: false,
        },
        ThreadsListEvent::Updated {
            request_id: crate::ids::RequestId {
                connection_id: crate::ids::RuntimeConnectionId(1),
                sequence: 3,
            },
            room_id: PRIVATE_ROOM_ID.to_owned(),
            items,
            is_paginating: true,
            end_reached: false,
        },
    ];

    for event in events {
        let debug = format!("{event:?}");
        assert!(debug.contains("items_len: 2"), "{debug}");
        assert!(debug.contains("RoomId(..)"), "{debug}");
        for raw in [
            PRIVATE_ROOM_ID,
            PRIVATE_THREAD_ROOT,
            PRIVATE_EVENT_ID,
            PRIVATE_SENDER,
            PRIVATE_REACTOR,
            "private thread body",
            "private latest body",
        ] {
            assert!(!debug.contains(raw), "raw {raw} leaked: {debug}");
        }
    }
}

// #1979: the thread summary DTO printed the latest event id, sender and body
// preview, which `TimelineItem`'s own Debug already redacts.
#[test]
fn thread_summary_dto_debug_redacts_latest_event_sender_and_preview() {
    let summary = ThreadSummaryDto {
        reply_count: 4,
        latest_event_id: Some(PRIVATE_EVENT_ID.to_owned()),
        latest_sender: Some(PRIVATE_SENDER.to_owned()),
        latest_sender_label: Some(PRIVATE_SENDER_LABEL.to_owned()),
        latest_body_preview: Some("private summary preview".to_owned()),
        latest_timestamp_ms: Some(1_700_000_000_000),
        unread_count: 1,
    };

    let debug = format!("{summary:?}");
    assert!(debug.contains("reply_count: 4"), "{debug}");
    assert!(
        debug.contains(r#"latest_event_id: Some("EventId(..)")"#),
        "{debug}"
    );
    assert!(
        debug.contains(r#"latest_sender: Some("UserId(..)")"#),
        "{debug}"
    );
    assert!(
        debug.contains(r#"latest_sender_label: Some("SenderLabel(..)")"#),
        "{debug}"
    );
    assert!(
        debug.contains(r#"latest_body_preview: Some("MessageBody(..)")"#),
        "{debug}"
    );
    for raw in [
        PRIVATE_EVENT_ID,
        PRIVATE_SENDER,
        PRIVATE_SENDER_LABEL,
        "private summary preview",
    ] {
        assert!(!debug.contains(raw), "raw {raw} leaked: {debug}");
    }
}

// #1979: the viewport observation printed the first and last visible event id.
#[test]
fn timeline_viewport_observation_debug_redacts_visible_event_ids() {
    let observation = TimelineViewportObservation {
        display_window_revision: Some(7),
        first_visible_event_id: Some(PRIVATE_EVENT_ID.to_owned()),
        last_visible_event_id: Some(PRIVATE_REACTION_EVENT_ID.to_owned()),
        visible_gap_ids: Vec::new(),
        at_bottom: true,
        bottom_arrival: TimelineBottomArrival::User,
    };

    let debug = format!("{observation:?}");
    assert!(debug.contains("at_bottom: true"), "{debug}");
    assert!(debug.contains("visible_gap_count: 0"), "{debug}");
    assert!(
        debug.contains(r#"first_visible_event_id: Some("EventId(..)")"#),
        "{debug}"
    );
    assert!(
        debug.contains(r#"last_visible_event_id: Some("EventId(..)")"#),
        "{debug}"
    );
    assert!(!debug.contains(PRIVATE_EVENT_ID), "{debug}");
    assert!(!debug.contains(PRIVATE_REACTION_EVENT_ID), "{debug}");

    let empty = TimelineViewportObservation::default();
    let debug = format!("{empty:?}");
    assert!(debug.contains("first_visible_event_id: None"), "{debug}");
}

// #1979: the undecryptable-event payload printed its megolm session id, the same
// class the message source already redacts.
#[test]
fn unable_to_decrypt_debug_redacts_the_session_id() {
    let undecryptable = TimelineUnableToDecrypt {
        session_id: Some("PRIVATE-MEGOLM-SESSION".to_owned()),
        reason: TimelineUnableToDecryptReason::MissingRoomKey,
        can_request_keys: true,
        recovery_stage: None,
        recovery_guidance: None,
    };

    let debug = format!("{undecryptable:?}");
    assert!(debug.contains("SessionId(..)"), "{debug}");
    assert!(debug.contains("can_request_keys: true"), "{debug}");
    assert!(!debug.contains("PRIVATE-MEGOLM-SESSION"), "{debug}");
}
