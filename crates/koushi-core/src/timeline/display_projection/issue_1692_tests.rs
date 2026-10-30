//! #1692: cached history pages projected incrementally at the live edge must
//! show what a one-shot rebuild of the same canonical history shows.
//!
//! The SDK delivers a back-paginated page as oldest-first `Insert`s right after
//! the date divider at canonical 0. When a page's oldest event is an older reply
//! of a thread whose newer reply is already loaded, #873 hides that reply right
//! after its `Insert`, and the #1443 adjacency rule then hides every following
//! row of the page because both of its neighbours are hidden cells.

use koushi_protocol::event::{
    TimelineDiff, TimelineItem, TimelineItemId, TimelineViewportObservation,
};
use koushi_protocol::ids::TimelineKind;
use koushi_state::TimelineThreadRootOrder;

use super::super::test_support::timeline_item;
use super::{DisplayProjectionContext, DisplayProjectionState, project_sdk_batch};
use crate::threads_list::{AuthoritativeThreadAggregate, ThreadRootDisplayData};

fn event(name: &str, ts: u64, root: Option<&str>) -> TimelineItem {
    let mut item = timeline_item(&format!("${name}"), Some(name), "@other:test", false);
    item.timestamp_ms = Some(ts);
    item.thread_root = root.map(|root| format!("${root}"));
    item
}

fn date_divider() -> TimelineItem {
    let mut item = timeline_item("$unused", None, "@other:test", false);
    item.id = TimelineItemId::Synthetic {
        synthetic_id: "date-divider".to_owned(),
    };
    item.sender = None;
    item.timestamp_ms = None;
    item
}

/// Setting off; `$t` is a tracked root posted at 8 whose latest reply is at 16.
fn context(root_t: &TimelineItem) -> DisplayProjectionContext {
    let observation = TimelineViewportObservation {
        at_bottom: true,
        ..Default::default()
    };
    let kind = TimelineKind::Room {
        room_id: "!room:test".to_owned(),
    };
    DisplayProjectionContext::for_timeline(&kind, &observation).with_thread_roots(
        TimelineThreadRootOrder::RootEvent,
        vec![ThreadRootDisplayData {
            root_event_id: "$t".to_owned(),
            unread: 0,
            activity_event_id: "$t2".to_owned(),
            activity_timestamp_ms: Some(16),
            item: Some(root_t.clone()),
            aggregate: AuthoritativeThreadAggregate {
                reply_count: 2,
                latest_event_id: Some("$t2".to_owned()),
                latest_sender: Some("@other:test".to_owned()),
                latest_sender_label: None,
                latest_body_preview: None,
                latest_timestamp_ms: Some(16),
            },
            pending: false,
            failure_kind: None,
        }],
    )
}

/// A cached page as the SDK delivers it: oldest first, after the divider.
fn page(items: &[TimelineItem]) -> Vec<TimelineDiff> {
    items
        .iter()
        .enumerate()
        .map(|(offset, item)| TimelineDiff::Insert {
            index: 1 + offset,
            item: item.clone(),
        })
        .collect()
}

fn rows(state: &DisplayProjectionState) -> Vec<String> {
    state
        .display_items()
        .iter()
        .map(|item| {
            let metadata = item.display_metadata.as_ref().expect("projected row");
            let ts = metadata
                .display_timestamp_ms
                .map_or("-".to_owned(), |ts| ts.to_string());
            format!("{}@{ts}", metadata.row_id)
        })
        .collect()
}

#[test]
fn cached_pages_behind_a_superseded_reply_match_a_one_shot_rebuild() {
    let root_t = event("t", 8, None);
    let context = context(&root_t);
    // Live window: its oldest event is an ordinary message.
    let mut canonical = vec![
        date_divider(),
        event("m6", 14, None),
        event("x4", 15, Some("x")),
        event("t2", 16, Some("t")),
        event("m7", 17, None),
    ];
    let mut incremental = DisplayProjectionState::default();
    incremental.return_to_live_window(&canonical, &context);

    // Each older page starts with a superseded reply of `$x`.
    let newer = [
        event("x3", 10, Some("x")),
        event("m4", 11, None),
        event("t1", 12, Some("t")),
        event("m5", 13, None),
    ];
    let middle = [
        event("x2", 6, Some("x")),
        event("m2", 7, None),
        root_t.clone(),
        event("m3", 9, None),
    ];
    for batch in [page(&newer), page(&middle)] {
        project_sdk_batch(&mut canonical, &mut incremental, &batch, &context);
    }

    let mut rebuilt = DisplayProjectionState::default();
    rebuilt.return_to_live_window(&canonical, &context);
    eprintln!("incremental: {:?}", rows(&incremental));
    eprintln!("rebuilt:     {:?}", rows(&rebuilt));
    assert_eq!(
        rows(&incremental),
        rows(&rebuilt),
        "#1692: incremental cached paging must show the rows a one-shot rebuild shows"
    );
}

#[test]
fn control_page_with_an_ordinary_oldest_event_matches_a_one_shot_rebuild() {
    let root_t = event("t", 8, None);
    let context = context(&root_t);
    let mut canonical = vec![
        date_divider(),
        event("m6", 14, None),
        event("x4", 15, Some("x")),
        event("t2", 16, Some("t")),
        event("m7", 17, None),
    ];
    let mut incremental = DisplayProjectionState::default();
    incremental.return_to_live_window(&canonical, &context);
    let newer = [
        event("m4", 10, None),
        event("x3", 11, Some("x")),
        event("t1", 12, Some("t")),
        event("m5", 13, None),
    ];
    let middle = [
        event("m2", 6, None),
        event("x2", 7, Some("x")),
        root_t.clone(),
        event("m3", 9, None),
    ];
    for batch in [page(&newer), page(&middle)] {
        project_sdk_batch(&mut canonical, &mut incremental, &batch, &context);
    }
    let mut rebuilt = DisplayProjectionState::default();
    rebuilt.return_to_live_window(&canonical, &context);
    assert_eq!(rows(&incremental), rows(&rebuilt));
}
