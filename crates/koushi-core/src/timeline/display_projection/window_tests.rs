use super::super::test_support::timeline_item;
use super::*;

fn items(count: usize) -> Vec<TimelineItem> {
    (0..count)
        .map(|i| {
            timeline_item(
                &format!("$event{i}:example.invalid"),
                Some("synthetic"),
                "@member:example.invalid",
                false,
            )
        })
        .collect()
}
fn observation(first: usize, last: usize) -> TimelineViewportObservation {
    TimelineViewportObservation {
        first_visible_event_id: Some(format!("$event{first}:example.invalid")),
        last_visible_event_id: Some(format!("$event{last}:example.invalid")),
        at_bottom: false,
        display_window_revision: Some(0),
        ..Default::default()
    }
}
#[test]
fn leaving_live_edge_interior_does_not_expand_history() {
    let canonical = items(10_000);
    let mut state = DisplayProjectionState::from_canonical_window(&canonical, 9880..10_000);
    let before = state.display_items().to_vec();
    let diffs = state.reconcile_bounded_window(
        &canonical,
        &observation(9970, 9980),
        &DisplayProjectionContext::for_timeline(
            &super::super::test_support::room_key().kind,
            &observation(9970, 9980),
        ),
        None,
    );
    assert!(
        diffs.is_empty(),
        "a small scroll must not publish the hidden canonical prefix"
    );
    assert_eq!(state.display_items(), before);
}

#[test]
fn visibility_reversal_recovers_previously_skipped_slots() {
    let mut canonical = items(1000);
    for item in &mut canonical[0..880] {
        item.is_hidden = true;
    }
    let mut state = DisplayProjectionState::from_canonical_window(&canonical, 880..1000);
    let mut obs = observation(880, 890);
    obs.display_window_revision = Some(state.window_revision);
    let context =
        DisplayProjectionContext::for_timeline(&super::super::test_support::room_key().kind, &obs);
    state.reconcile_bounded_window(
        &canonical,
        &obs,
        &context,
        Some(koushi_protocol::event::PaginationDirection::Backward),
    );
    canonical[870].is_hidden = false;
    state.rehydrate_bounded_visibility(&canonical, &context);
    assert!(
        state
            .display_items()
            .iter()
            .any(|item| item.id == canonical[870].id)
    );
    assert!(state.slots.len() <= 240);
}
#[test]
fn large_room_reset_does_not_materialize_entire_canonical_history() {
    let mut canonical = items(100);
    let mut state = DisplayProjectionState::from_canonical_window(&canonical, 0..100);
    let obs = observation(70, 80);
    let context =
        DisplayProjectionContext::for_timeline(&super::super::test_support::room_key().kind, &obs);
    let batch = project_sdk_batch(
        &mut canonical,
        &mut state,
        &[TimelineDiff::Reset {
            items: items(10_000),
        }],
        &context,
    );
    assert!(state.slots.len() <= 240);
    // The old bounded membership is visited once before the bounded reset.
    assert!(batch.display_payload_visits <= 480);
}
