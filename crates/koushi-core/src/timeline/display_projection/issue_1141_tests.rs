//! #1141: synthetic thread-root slots are display rows, not content.
//!
//! A withheld (`Forbidden` / `NotFound`) or still-loading thread root is
//! projected as a bodyless `Synthetic` row that carries the reply chip. The
//! consumer export applied the #1110 content-suppression reason to it and hid
//! the only entry point to the visible replies. These probes run the production
//! display projection and then the production export on its output.

use koushi_protocol::event::{TimelineDisplayKind, TimelineItem, TimelineViewportObservation};
use koushi_state::{AppState, OperationFailureKind, TimelineThreadRootOrder};

use super::super::test_support::{room_key, timeline_item};
use super::{DisplayProjectionContext, DisplayProjectionState};
use crate::event_projection::project_timeline_item_display_labels;

fn project_root_slot(pending: bool, failure_kind: Option<OperationFailureKind>) -> TimelineItem {
    let key = room_key();
    let mut reply = timeline_item(
        "$reply:example.invalid",
        Some("Synthetic reply"),
        "@member:example.invalid",
        false,
    );
    reply.thread_root = Some("$root:example.invalid".to_owned());
    let canonical = vec![reply];
    let mut state = DisplayProjectionState::from_canonical_window(&canonical, 0..1);
    let context = DisplayProjectionContext::for_timeline(
        &key.kind,
        &TimelineViewportObservation::default(),
        false,
    )
    .with_thread_roots(
        TimelineThreadRootOrder::LatestReply,
        vec![crate::threads_list::ThreadRootDisplayData {
            root_event_id: "$root:example.invalid".to_owned(),
            activity_event_id: "$reply:example.invalid".to_owned(),
            activity_timestamp_ms: Some(1),
            item: None,
            aggregate: crate::threads_list::AuthoritativeThreadAggregate {
                reply_count: 1,
                latest_event_id: Some("$reply:example.invalid".to_owned()),
                latest_sender: Some("@member:example.invalid".to_owned()),
                latest_sender_label: None,
                latest_body_preview: Some("Synthetic reply".to_owned()),
                latest_timestamp_ms: Some(1),
            },
            pending,
            failure_kind,
        }],
    );
    state.reproject(&context);
    let [root_slot] = state.display_items() else {
        panic!("the reply is folded into exactly one root slot");
    };
    root_slot.clone()
}

fn assert_exported_visible(mut root_slot: TimelineItem, expected_kind: TimelineDisplayKind) {
    assert_eq!(
        root_slot
            .display_metadata
            .as_ref()
            .map(|metadata| metadata.kind),
        Some(expected_kind)
    );
    assert!(
        root_slot.body.is_none(),
        "the placeholder carries no content"
    );
    assert!(
        !root_slot.is_hidden,
        "the display projection keeps it visible"
    );

    project_timeline_item_display_labels(&mut root_slot, &AppState::default());

    assert!(
        !root_slot.is_hidden,
        "the export must not content-suppress a synthetic root slot"
    );
    assert_eq!(
        root_slot.thread_summary.map(|summary| summary.reply_count),
        Some(1),
        "the reply chip stays attached"
    );
}

#[test]
fn withheld_thread_root_placeholder_stays_visible_on_export() {
    for failure_kind in [
        OperationFailureKind::Forbidden,
        OperationFailureKind::NotFound,
    ] {
        assert_exported_visible(
            project_root_slot(false, Some(failure_kind)),
            TimelineDisplayKind::ThreadRootFailed { failure_kind },
        );
    }
}

#[test]
fn pending_thread_root_placeholder_stays_visible_on_export() {
    assert_exported_visible(
        project_root_slot(true, None),
        TimelineDisplayKind::ThreadRootPending,
    );
}
