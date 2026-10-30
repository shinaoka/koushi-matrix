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

/// A room at the bottom with no tracked thread roots.
fn room_context() -> DisplayProjectionContext {
    let observation = TimelineViewportObservation {
        at_bottom: true,
        ..Default::default()
    };
    let kind = TimelineKind::Room {
        room_id: "!room:test".to_owned(),
    };
    DisplayProjectionContext::for_timeline(&kind, &observation)
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

fn shown(state: &DisplayProjectionState) -> Vec<String> {
    state
        .display_items()
        .iter()
        .filter_map(|item| item.display_metadata.as_ref()?.content_event_id.clone())
        .collect()
}

fn ids(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| format!("${name}")).collect()
}

/// The reporter's shapes: older pages arrive as `PushFront`s or `Insert` at 0,
/// one diff or one page per batch, so the reply hidden at canonical 0 by one
/// batch is the neighbour of the next batch's rows.
#[test]
fn a_reply_hidden_at_canonical_zero_does_not_hide_the_next_batch() {
    let initial = || {
        vec![
            event("a", 10, None),
            event("reply-new", 11, Some("r")),
            event("b", 12, None),
        ]
    };
    let older = [
        event("reply-old", 4, Some("r")),
        event("m1", 3, None),
        event("r", 2, None),
        event("m0", 1, None),
    ];
    let expected = ids(&["m0", "r", "m1", "a", "b"]);
    let context = DisplayProjectionContext::bounded_live_edge();
    for one_batch in [true, false] {
        for push_front in [true, false] {
            let mut canonical = initial();
            let mut state = DisplayProjectionState::from_canonical_window(&canonical, 0..3);
            let diffs = older
                .iter()
                .map(|item| {
                    if push_front {
                        TimelineDiff::PushFront { item: item.clone() }
                    } else {
                        TimelineDiff::Insert {
                            index: 0,
                            item: item.clone(),
                        }
                    }
                })
                .collect::<Vec<_>>();
            if one_batch {
                project_sdk_batch(&mut canonical, &mut state, &diffs, &context);
            } else {
                for diff in &diffs {
                    project_sdk_batch(
                        &mut canonical,
                        &mut state,
                        std::slice::from_ref(diff),
                        &context,
                    );
                }
            }
            assert_eq!(
                shown(&state),
                expected,
                "one_batch={one_batch} push_front={push_front}"
            );
        }
    }
}

/// Two cached pages, the first one ending (oldest) on a superseded reply.
#[test]
fn a_page_behind_a_page_that_ends_on_a_superseded_reply_stays_visible() {
    let context = DisplayProjectionContext::bounded_live_edge();
    let mut canonical = vec![
        event("a", 10, None),
        event("reply-new", 11, Some("r")),
        event("b", 12, None),
    ];
    let mut state = DisplayProjectionState::from_canonical_window(&canonical, 0..3);
    let page_1 = [
        TimelineDiff::Insert {
            index: 0,
            item: event("reply-old", 5, Some("r")),
        },
        TimelineDiff::Insert {
            index: 1,
            item: event("p1", 6, None),
        },
    ];
    project_sdk_batch(&mut canonical, &mut state, &page_1, &context);
    let page_2 = [
        TimelineDiff::Insert {
            index: 0,
            item: event("m0", 1, None),
        },
        TimelineDiff::Insert {
            index: 1,
            item: event("r", 2, None),
        },
        TimelineDiff::Insert {
            index: 2,
            item: event("m1", 3, None),
        },
    ];
    project_sdk_batch(&mut canonical, &mut state, &page_2, &context);
    assert_eq!(shown(&state), ids(&["m0", "r", "m1", "p1", "a", "b"]));
}

/// The same diffs give the same display however they are split into batches,
/// also when a run holds both a trimmed prefix and a suppressed reply.
#[test]
fn a_mixed_trimmed_and_suppressed_run_survives_a_batch_boundary() {
    let context = DisplayProjectionContext::bounded_live_edge();
    let start = || {
        let canonical = vec![
            event("p0", 1, None),
            event("p1", 2, None),
            event("reply-old", 3, Some("r")),
            event("a", 4, None),
            event("b", 5, None),
        ];
        // p0 and p1 are a trimmed prefix; the window starts at the old reply.
        let state = DisplayProjectionState::from_canonical_window(&canonical, 2..5);
        (canonical, state)
    };
    let diffs = [
        // The newer reply suppresses the old one (#873)...
        TimelineDiff::PushBack {
            item: event("reply-new", 6, Some("r")),
        },
        // ...and a row then lands between the trimmed prefix and that reply.
        TimelineDiff::Insert {
            index: 2,
            item: event("m", 3, None),
        },
    ];
    let (mut canonical, mut together) = start();
    project_sdk_batch(&mut canonical, &mut together, &diffs, &context);
    let (mut canonical, mut split) = start();
    for diff in &diffs {
        project_sdk_batch(
            &mut canonical,
            &mut split,
            std::slice::from_ref(diff),
            &context,
        );
    }
    assert_eq!(shown(&together), ids(&["m", "a", "b"]));
    assert_eq!(shown(&split), shown(&together));
}

/// A visibility rehydration between batches keeps a suppressed-only prefix
/// transparent.
#[test]
fn rehydration_keeps_a_suppressed_prefix_transparent() {
    let context = room_context();
    let mut canonical = vec![
        event("reply-old", 2, Some("r")),
        event("a", 3, None),
        event("reply-new", 4, Some("r")),
    ];
    let mut state = DisplayProjectionState::default();
    state.return_to_live_window(&canonical, &context);
    state.rehydrate_bounded_visibility(&canonical, &context);
    let diffs = [TimelineDiff::PushFront {
        item: event("m0", 1, None),
    }];
    project_sdk_batch(&mut canonical, &mut state, &diffs, &context);
    assert_eq!(shown(&state), ids(&["m0", "a"]));
}

/// A replacement snapshot that turns the suppressed prefix into an ordinary
/// row must not leave a stale transparent run behind: a later front page may
/// not appear above a hidden ordinary row.
#[test]
fn a_reset_that_adds_a_row_to_a_transparent_prefix_makes_it_trimmed() {
    let at_bottom = room_context();
    let mut away = at_bottom.clone();
    away.viewport.at_bottom = false;
    away.max_live_edge_items = None;
    let mut canonical = vec![
        event("reply-old", 2, Some("r")),
        event("a", 3, None),
        event("reply-new", 4, Some("r")),
    ];
    let mut state = DisplayProjectionState::default();
    state.return_to_live_window(&canonical, &at_bottom);
    // The relay's replacement snapshot keeps the represented positions, so
    // `reset_bounded` takes its same-positions path.
    canonical = vec![
        event("x", 2, None),
        event("a", 3, None),
        event("reply-new", 4, Some("r")),
    ];
    state.reset_bounded(&canonical, &away);
    let page = [TimelineDiff::PushFront {
        item: event("y", 1, None),
    }];
    project_sdk_batch(&mut canonical, &mut state, &page, &away);
    let rows = shown(&state);
    assert!(
        !rows.contains(&"$y".to_owned()) || rows.contains(&"$x".to_owned()),
        "a front page above a hidden ordinary row: {rows:?}"
    );
}

/// Fix 2: a tracked root whose own event is not a visible slot keeps its own
/// time under `RootEvent`, and its latest reply's time under `LatestReply`.
#[test]
fn an_off_window_root_follows_the_thread_root_order() {
    let root_t = event("t", 8, None);
    let mut canonical = vec![
        date_divider(),
        event("m2", 7, None),
        root_t.clone(),
        event("m3", 9, None),
        event("t2", 16, Some("t")),
        event("m7", 17, None),
    ];
    let place = |order: TimelineThreadRootOrder, canonical: &mut Vec<TimelineItem>| {
        let mut context = context(&root_t);
        context.thread_root_order = order;
        let mut state = DisplayProjectionState::default();
        state.return_to_live_window(canonical, &context);
        // The root's own event leaves the represented slots.
        state.slots.retain(|slot| slot.item.id != root_t.id);
        state.reproject(&context);
        rows(&state)
    };
    assert_eq!(
        place(TimelineThreadRootOrder::RootEvent, &mut canonical),
        [
            "syn:date-divider@-",
            "$m2@7",
            "thread-root:$t@8",
            "$m3@9",
            "$m7@17"
        ]
    );
    assert_eq!(
        place(TimelineThreadRootOrder::LatestReply, &mut canonical),
        [
            "syn:date-divider@-",
            "$m2@7",
            "$m3@9",
            "thread-root:$t@16",
            "$m7@17"
        ]
    );
}

/// A trimmed row that a `Set` turns into a non-row item (for example a
/// redaction while deleted messages are hidden) no longer blocks adjacency.
#[test]
fn a_set_that_hides_a_trimmed_row_makes_it_transparent() {
    let context = DisplayProjectionContext::bounded_live_edge();
    let mut canonical = vec![event("x", 1, None), event("a", 2, None)];
    // x is outside the represented window.
    let mut state = DisplayProjectionState::from_canonical_window(&canonical, 1..2);
    let mut hidden_x = event("x", 1, None);
    hidden_x.is_hidden = true;
    let diffs = [
        TimelineDiff::Set {
            index: 0,
            item: hidden_x,
        },
        TimelineDiff::PushFront {
            item: event("y", 0, None),
        },
    ];
    project_sdk_batch(&mut canonical, &mut state, &diffs, &context);
    assert_eq!(shown(&state), ids(&["y", "a"]));
}
