//! #1981: whether the desktop's current display window already shows an event
//! as a row. A reply quote whose original passes this check settles in place
//! instead of opening a focused timeline. Everything here is synthetic.

use koushi_protocol::event::{TimelineDisplayKind, TimelineDisplayMetadata, TimelineItemId};

use super::super::test_support::timeline_item;
use super::display_items_show_event;

#[test]
fn an_ordinary_row_is_matched_by_its_event_id() {
    let items = vec![
        timeline_item(
            "$earlier:example.invalid",
            Some("a"),
            "@a:example.invalid",
            false,
        ),
        timeline_item(
            "$original:example.invalid",
            Some("b"),
            "@b:example.invalid",
            false,
        ),
    ];
    assert!(display_items_show_event(
        &items,
        "$original:example.invalid"
    ));
    assert!(!display_items_show_event(&items, "$absent:example.invalid"));
}

#[test]
fn hidden_items_and_thread_replies_are_not_rows() {
    let hidden = timeline_item("$hidden:example.invalid", None, "@a:example.invalid", true);
    let mut reply = timeline_item(
        "$reply:example.invalid",
        Some("r"),
        "@a:example.invalid",
        false,
    );
    reply.thread_root = Some("$root:example.invalid".to_owned());
    let items = vec![hidden, reply];
    assert!(!display_items_show_event(&items, "$hidden:example.invalid"));
    assert!(
        !display_items_show_event(&items, "$reply:example.invalid"),
        "a thread reply belongs to the thread pane, not the room timeline"
    );
}

#[test]
fn a_moved_thread_root_is_matched_by_its_content_event_id() {
    let mut root = timeline_item(
        "$root:example.invalid",
        Some("root"),
        "@a:example.invalid",
        false,
    );
    root.id = TimelineItemId::Synthetic {
        synthetic_id: "thread-root-row".to_owned(),
    };
    root.display_metadata = Some(TimelineDisplayMetadata {
        row_id: "thread-root-row".to_owned(),
        kind: TimelineDisplayKind::ThreadRoot,
        content_event_id: Some("$root:example.invalid".to_owned()),
        activity_event_id: Some("$latest-reply:example.invalid".to_owned()),
        display_timestamp_ms: None,
        group_count: 1,
        group_position: 0,
        group_id: None,
    });
    let items = vec![root];
    assert!(display_items_show_event(&items, "$root:example.invalid"));
    assert!(
        !display_items_show_event(&items, "$latest-reply:example.invalid"),
        "a quote names the content event, never the row's activity event"
    );
}
