//! #1110: suppressed rows must not consume display capacity.
//!
//! The live-edge window and the bounded membership are sized in *displayed
//! rows*. Sizing them from `thread_root.is_none()` alone let a run of hidden
//! technical state updates evict a real conversation message from the window.
//! These probes exercise the production projection functions directly.

use koushi_protocol::event::TimelineItem;

use super::super::item_projection::timeline_item_event_id;
use super::super::navigation::ROOM_REPLAY_INITIAL_ITEMS_MAX;
use super::super::test_support::timeline_item;
use super::{DisplayProjectionState, item_occupies_display_row, live_edge_window_start};

fn hidden_technical_item(index: usize) -> TimelineItem {
    timeline_item(
        &format!("$acl-{index}:example.invalid"),
        None,
        "@moderator:example.invalid",
        true,
    )
}

/// Probe 4: a hidden technical run at the live edge must not evict the visible
/// message that precedes it.
#[test]
fn hidden_tail_does_not_evict_the_visible_message_from_the_live_edge() {
    let mut canonical = vec![timeline_item(
        "$message:example.invalid",
        Some("Synthetic message"),
        "@member:example.invalid",
        false,
    )];
    for index in 0..ROOM_REPLAY_INITIAL_ITEMS_MAX {
        canonical.push(hidden_technical_item(index));
    }

    let start = live_edge_window_start(&canonical, ROOM_REPLAY_INITIAL_ITEMS_MAX);
    let projection =
        DisplayProjectionState::from_canonical_window(&canonical, start..canonical.len());

    assert!(
        projection
            .display_items()
            .iter()
            .any(|item| timeline_item_event_id(item) == Some("$message:example.invalid")),
        "hidden events consumed the entire live-edge capacity"
    );
}

/// The capacity predicate itself: only a non-reply row the visibility policy
/// actually renders counts.
#[test]
fn only_rendered_event_rows_occupy_display_capacity() {
    let visible = timeline_item(
        "$message:example.invalid",
        Some("Synthetic message"),
        "@member:example.invalid",
        false,
    );
    let hidden = hidden_technical_item(0);
    let mut reply = timeline_item(
        "$reply:example.invalid",
        Some("Synthetic reply"),
        "@member:example.invalid",
        false,
    );
    reply.thread_root = Some("$root:example.invalid".to_owned());

    assert!(item_occupies_display_row(&visible));
    assert!(!item_occupies_display_row(&hidden));
    assert!(!item_occupies_display_row(&reply));
}

/// A visible tail still bounds the window, so the change never widens it.
#[test]
fn visible_tail_still_bounds_the_live_edge_window() {
    let mut canonical: Vec<TimelineItem> = (0..ROOM_REPLAY_INITIAL_ITEMS_MAX + 5)
        .map(|index| {
            timeline_item(
                &format!("$message-{index}:example.invalid"),
                Some("Synthetic message"),
                "@member:example.invalid",
                false,
            )
        })
        .collect();
    canonical.push(hidden_technical_item(0));

    let start = live_edge_window_start(&canonical, ROOM_REPLAY_INITIAL_ITEMS_MAX);
    let window = &canonical[start..];

    assert_eq!(
        window
            .iter()
            .filter(|item| item_occupies_display_row(item))
            .count(),
        ROOM_REPLAY_INITIAL_ITEMS_MAX,
        "the window holds exactly the displayed-row budget"
    );
    assert!(
        !window
            .iter()
            .any(|item| { timeline_item_event_id(item) == Some("$message-0:example.invalid") }),
        "the oldest visible message stays outside the bounded window"
    );
}
