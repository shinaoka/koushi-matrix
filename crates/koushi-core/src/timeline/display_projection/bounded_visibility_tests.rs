use super::super::test_support::{room_key, timeline_item};
use super::*;
use crate::threads_list::{AuthoritativeThreadAggregate, ThreadRootDisplayData};

#[test]
fn visibility_toggle_preserves_off_window_root_and_latest_reply_cursor() {
    let mut canonical = (0..300)
        .map(|i| {
            timeline_item(
                &format!("$event{i}:example.invalid"),
                Some("synthetic"),
                "@member:example.invalid",
                false,
            )
        })
        .collect::<Vec<_>>();
    let root = canonical[0].clone();
    let root_id = "$event0:example.invalid";
    for item in &mut canonical[280..289] {
        item.thread_root = Some(root_id.to_owned());
    }
    canonical[205].is_hidden = true;
    canonical[299].is_hidden = true;
    let observation = TimelineViewportObservation {
        first_visible_event_id: Some("$event190:example.invalid".to_owned()),
        last_visible_event_id: Some("$event200:example.invalid".to_owned()),
        display_window_revision: Some(0),
        ..Default::default()
    };
    let context = DisplayProjectionContext::for_timeline(&room_key().kind, &observation)
        .with_thread_roots(
            TimelineThreadRootOrder::LatestReply,
            vec![ThreadRootDisplayData {
                root_event_id: root_id.to_owned(),
                unread: 0,
                activity_event_id: "$event288:example.invalid".to_owned(),
                activity_timestamp_ms: Some(1),
                item: Some(root),
                aggregate: AuthoritativeThreadAggregate {
                    reply_count: 9,
                    latest_event_id: Some("$event288:example.invalid".to_owned()),
                    latest_sender: None,
                    latest_sender_label: None,
                    latest_body_preview: None,
                    latest_timestamp_ms: Some(1),
                },
                pending: false,
                failure_kind: None,
            }],
        );
    let mut state = DisplayProjectionState::from_canonical_window(&canonical, 180..300);
    state.reproject(&context);
    assert!(
        state.display_items().iter().any(
            |item| matches!(&item.id, TimelineItemId::Event { event_id } if event_id == root_id)
        )
    );
    let before = state.display_items().to_vec();
    canonical[205].is_hidden = false;
    let diffs = state.rehydrate_bounded_visibility(&canonical, &context);
    let mut replay = before;
    apply_timeline_diffs_to_items(&mut replay, &diffs);
    assert_eq!(replay, state.display_items());
    let projected_root = state
        .display_items()
        .iter()
        .find(|item| matches!(&item.id, TimelineItemId::Event { event_id } if event_id == root_id))
        .expect("a visibility toggle must preserve the off-window root's row");
    assert_eq!(
        projected_root
            .display_metadata
            .as_ref()
            .unwrap()
            .activity_event_id
            .as_deref(),
        Some("$event288:example.invalid")
    );
    assert!(
        state
            .display_items()
            .iter()
            .any(|item| item.id == canonical[205].id && !item.is_hidden)
    );
    assert_eq!(
        state
            .slots
            .iter()
            .filter(|slot| slot.item.thread_root.is_some())
            .count(),
        1
    );
    assert!(state.slots.iter().any(|slot| slot.canonical_index == 288));
    assert!(state.window_reaches_live_end(canonical.len()));
    assert!(state.slots.len() <= 240);
}

#[test]
fn visibility_rehydrate_preserves_more_than_sixty_distinct_reply_roots_within_budget() {
    let mut canonical = (0..120)
        .map(|i| {
            timeline_item(
                &format!("$ordinary{i}:example.invalid"),
                Some("synthetic"),
                "@member:example.invalid",
                false,
            )
        })
        .collect::<Vec<_>>();
    for index in 0..90 {
        let mut reply = timeline_item(
            &format!("$reply{index}:example.invalid"),
            Some("synthetic"),
            "@member:example.invalid",
            false,
        );
        reply.thread_root = Some(format!("$root{index}:example.invalid"));
        canonical.push(reply);
    }
    let observation = TimelineViewportObservation {
        first_visible_event_id: Some("$ordinary0:example.invalid".to_owned()),
        last_visible_event_id: Some("$ordinary10:example.invalid".to_owned()),
        display_window_revision: Some(0),
        ..Default::default()
    };
    let context = DisplayProjectionContext::for_timeline(&room_key().kind, &observation);
    let mut state = DisplayProjectionState::from_canonical_window(&canonical, 0..canonical.len());
    canonical[110].is_hidden = true;
    state.rehydrate_bounded_visibility(&canonical, &context);
    assert_eq!(
        state
            .slots
            .iter()
            .filter(|slot| slot.item.thread_root.is_some())
            .count(),
        90,
        "reply ownership must use available window capacity rather than a separate sixty-root cap"
    );
    for reply in &canonical[120..] {
        assert!(
            state.slots.iter().any(|slot| slot.item.id == reply.id),
            "each represented latest reply must survive an unrelated policy change"
        );
    }
    assert!(state.slots.len() <= 240);
    assert!(state.window_reaches_live_end(canonical.len()));
}

#[test]
fn visibility_toggle_does_not_admit_unrepresented_reply_roots_or_lose_reading_rows() {
    let mut canonical = (0..420)
        .map(|i| {
            timeline_item(
                &format!("$row{i}:example.invalid"),
                Some("synthetic"),
                "@member:example.invalid",
                false,
            )
        })
        .collect::<Vec<_>>();
    for (i, item) in canonical.iter_mut().enumerate().skip(120) {
        item.thread_root = Some(format!("$root{i}:example.invalid"));
    }
    let observation = TimelineViewportObservation {
        first_visible_event_id: Some("$row80:example.invalid".to_owned()),
        last_visible_event_id: Some("$row90:example.invalid".to_owned()),
        ..Default::default()
    };
    let context = DisplayProjectionContext::for_timeline(&room_key().kind, &observation);
    let mut state = DisplayProjectionState::default();
    state.return_to_live_window(&canonical, &context);
    let represented_roots = state
        .slots
        .iter()
        .filter_map(|s| s.item.thread_root.clone())
        .collect::<HashSet<_>>();
    canonical[110].is_hidden = true;
    state.rehydrate_bounded_visibility(&canonical, &context);
    assert!(
        state.slots.len() <= 240,
        "visibility updates must not exceed the shared slot budget"
    );
    for i in [80, 90] {
        assert!(
            state.slots.iter().any(|slot| slot.canonical_index == i),
            "reading endpoints must survive an unrelated toggle"
        );
    }
    for root in represented_roots {
        assert!(
            state
                .slots
                .iter()
                .any(|slot| slot.item.thread_root.as_ref() == Some(&root)),
            "already represented roots must keep their budget"
        );
    }
    assert!(state.window_reaches_live_end(canonical.len()));
}

/// #1657: a room of 300 rows whose untimed hidden cursors (`TimelineStart`
/// at 0, `ReadMarker` at 60) carry no timestamp. The thread root and its
/// latest reply both lie outside every window used below.
fn untimed_cursor_fixture(
    activity_timestamp_ms: u64,
) -> (Vec<TimelineItem>, ThreadRootDisplayData) {
    let mut canonical = (0..300)
        .map(|i| {
            let mut item = timeline_item(
                &format!("$event{i}:example.invalid"),
                Some("synthetic"),
                "@member:example.invalid",
                false,
            );
            item.timestamp_ms = Some(1_000 + i as u64);
            item
        })
        .collect::<Vec<_>>();
    for (index, synthetic_id) in [(0, "timeline-start"), (60, "read-marker")] {
        canonical[index].id = TimelineItemId::Synthetic {
            synthetic_id: synthetic_id.to_owned(),
        };
        canonical[index].timestamp_ms = None;
        canonical[index].is_hidden = true;
    }
    let mut root_item = timeline_item(
        "$root:example.invalid",
        Some("synthetic"),
        "@member:example.invalid",
        false,
    );
    root_item.timestamp_ms = Some(500);
    let root = ThreadRootDisplayData {
        root_event_id: "$root:example.invalid".to_owned(),
        unread: 0,
        activity_event_id: "$reply:example.invalid".to_owned(),
        activity_timestamp_ms: Some(activity_timestamp_ms),
        item: Some(root_item),
        aggregate: AuthoritativeThreadAggregate {
            reply_count: 1,
            latest_event_id: Some("$reply:example.invalid".to_owned()),
            latest_sender: None,
            latest_sender_label: None,
            latest_body_preview: None,
            latest_timestamp_ms: Some(activity_timestamp_ms),
        },
        pending: false,
        failure_kind: None,
    };
    (canonical, root)
}

fn untimed_cursor_context(root: ThreadRootDisplayData) -> DisplayProjectionContext {
    let observation = TimelineViewportObservation {
        first_visible_event_id: Some("$event1:example.invalid".to_owned()),
        last_visible_event_id: Some("$event20:example.invalid".to_owned()),
        display_window_revision: Some(0),
        ..Default::default()
    };
    DisplayProjectionContext::for_timeline(&room_key().kind, &observation)
        .with_thread_roots(TimelineThreadRootOrder::LatestReply, vec![root])
}

fn root_row(rows: &[TimelineItem]) -> Option<usize> {
    rows.iter().position(
        |item| matches!(&item.id, TimelineItemId::Event { event_id } if event_id == "$root:example.invalid"),
    )
}

#[test]
fn off_window_root_is_ordered_among_timed_rows_not_before_untimed_cursors() {
    for (activity_timestamp_ms, after_event) in [(1_150, 150), (2_000, 299)] {
        let (canonical, root) = untimed_cursor_fixture(activity_timestamp_ms);
        let context = untimed_cursor_context(root);
        let mut state = DisplayProjectionState::from_canonical_window(&canonical, 0..300);
        state.reproject(&context);
        let rows = state.display_items();
        let root_position = root_row(rows).expect("the retained root keeps a row");
        assert_eq!(rows[0].id, canonical[0].id, "TimelineStart stays first");
        assert_eq!(
            rows[root_position - 1].id,
            canonical[after_event].id,
            "activity {activity_timestamp_ms} follows its newest older row"
        );
        let timestamps = rows
            .iter()
            .filter_map(|item| item.display_metadata.as_ref()?.display_timestamp_ms)
            .collect::<Vec<_>>();
        assert!(timestamps.windows(2).all(|pair| pair[0] <= pair[1]));
    }
}

#[test]
fn historical_window_omits_root_whose_activity_lies_past_its_end() {
    let (canonical, root) = untimed_cursor_fixture(2_000);
    let context = untimed_cursor_context(root);
    let mut state = DisplayProjectionState::from_canonical_window(&canonical, 0..120);
    state.reproject(&context);
    assert!(!state.window_reaches_live_end(canonical.len()));
    assert_eq!(root_row(state.display_items()), None);
    assert_eq!(state.display_items()[0].id, canonical[0].id);
}
