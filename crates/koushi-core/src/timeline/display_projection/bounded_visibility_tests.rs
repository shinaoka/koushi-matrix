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
