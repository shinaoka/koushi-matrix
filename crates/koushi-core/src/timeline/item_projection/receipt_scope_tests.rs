//! Issue #1255 Core-level receipt-scope coverage.
//!
//! The timeline key — not the SDK's resolved receipt thread — decides which
//! receipt scope a timeline publishes. These tests pin the actor scope
//! derivation and the two action builders that carry it.
//!
//! All identifiers are synthetic (`example.invalid` / `:test`).

use koushi_protocol::ids::{AccountKey, TimelineKey, TimelineKind};
use koushi_state::{
    AppAction, LiveEventReceiptSummaryUpdate, LiveEventReceipts, LiveReadReceipt, ReceiptScope,
};

use super::{
    build_live_receipt_summary_actions, build_receipt_observation_actions,
    receipt_scope_for_timeline_kind,
};
use crate::timeline::test_support::{focused_key, room_key, thread_key};

const ROOM_ID: &str = "!r:test";
const ROOT_EVENT_ID: &str = "$root:test";
const TARGET_EVENT_ID: &str = "$evt:test";
const REPLY_EVENT_ID: &str = "$reply:test";

fn reader() -> LiveReadReceipt {
    LiveReadReceipt {
        user_id: "@reader:example.invalid".to_owned(),
        display_name: None,
        original_display_label: String::new(),
        avatar: None,
        timestamp_ms: Some(1),
    }
}

fn summary_update() -> LiveEventReceiptSummaryUpdate {
    LiveEventReceiptSummaryUpdate {
        event_id: REPLY_EVENT_ID.to_owned(),
        readers: vec![reader()],
        total_count: 1,
    }
}

fn thread_reply_events() -> Vec<LiveEventReceipts> {
    vec![LiveEventReceipts {
        event_id: REPLY_EVENT_ID.to_owned(),
        receipts: vec![reader()],
    }]
}

#[test]
fn timeline_kind_owns_its_receipt_scope() {
    assert_eq!(
        receipt_scope_for_timeline_kind(&room_key().kind),
        ReceiptScope::Main,
        "a Room timeline publishes the main scope"
    );
    assert_eq!(
        receipt_scope_for_timeline_kind(&thread_key().kind),
        ReceiptScope::Thread {
            root_event_id: ROOT_EVENT_ID.to_owned()
        },
        "a Thread timeline publishes its root's thread scope"
    );
    assert_eq!(
        receipt_scope_for_timeline_kind(&focused_key().kind),
        ReceiptScope::Focused {
            event_id: TARGET_EVENT_ID.to_owned()
        },
        "a permalink/context timeline publishes its own focused scope"
    );
}

#[test]
fn thread_actor_summary_action_carries_its_thread_scope_and_window() {
    let actions = build_live_receipt_summary_actions(
        ROOM_ID,
        receipt_scope_for_timeline_kind(&thread_key().kind),
        vec![REPLY_EVENT_ID.to_owned()],
        vec![summary_update()],
        Vec::new(),
    );

    assert!(matches!(
        actions.as_slice(),
        [AppAction::LiveRoomReceiptSummariesUpdated {
            scope,
            scoped_event_ids,
            ..
        }] if *scope
            == ReceiptScope::Thread {
                root_event_id: ROOT_EVENT_ID.to_owned(),
            }
            && scoped_event_ids == &vec![REPLY_EVENT_ID.to_owned()]
    ));
}

#[test]
fn room_actor_summary_action_carries_the_main_scope_and_empty_live_window() {
    let actions = build_live_receipt_summary_actions(
        ROOM_ID,
        receipt_scope_for_timeline_kind(&room_key().kind),
        Vec::new(),
        vec![summary_update()],
        Vec::new(),
    );

    assert!(matches!(
        actions.as_slice(),
        [AppAction::LiveRoomReceiptSummariesUpdated {
            scope,
            scoped_event_ids,
            ..
        }] if *scope == ReceiptScope::Main && scoped_event_ids.is_empty()
    ));
}

#[test]
fn authoritative_actions_carry_the_actor_scope() {
    let cases = [
        (room_key(), ReceiptScope::Main),
        (
            thread_key(),
            ReceiptScope::Thread {
                root_event_id: ROOT_EVENT_ID.to_owned(),
            },
        ),
        (
            focused_key(),
            ReceiptScope::Focused {
                event_id: TARGET_EVENT_ID.to_owned(),
            },
        ),
    ];

    for (key, expected) in cases {
        let actions = build_receipt_observation_actions(
            ROOM_ID,
            receipt_scope_for_timeline_kind(&key.kind),
            thread_reply_events(),
            Vec::new(),
            vec![REPLY_EVENT_ID.to_owned()],
        );
        assert!(matches!(
            actions.as_slice(),
            [AppAction::LiveRoomReceiptsWindowReconciled {
                scope,
                scoped_event_ids,
                ..
            }] if *scope == expected && scoped_event_ids == &vec![REPLY_EVENT_ID.to_owned()]
        ));
    }
}

#[test]
fn focused_permalink_on_a_thread_reply_stays_in_its_focused_scope() {
    let key = TimelineKey {
        account_key: AccountKey("@a:test".to_owned()),
        kind: TimelineKind::Focused {
            room_id: ROOM_ID.to_owned(),
            event_id: REPLY_EVENT_ID.to_owned(),
        },
    };

    assert_eq!(
        receipt_scope_for_timeline_kind(&key.kind),
        ReceiptScope::Focused {
            event_id: REPLY_EVENT_ID.to_owned()
        },
        "the permalink target keys its own focused scope"
    );
    assert_ne!(
        receipt_scope_for_timeline_kind(&key.kind),
        ReceiptScope::Thread {
            root_event_id: ROOT_EVENT_ID.to_owned()
        },
        "a focused permalink on a thread reply must not claim the thread scope"
    );
}
