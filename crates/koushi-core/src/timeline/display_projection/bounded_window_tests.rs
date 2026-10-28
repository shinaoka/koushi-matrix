use super::super::test_support::{room_key, timeline_item};
use super::*;
use koushi_protocol::event::PaginationDirection;

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
fn observe(
    state: &DisplayProjectionState,
    first: usize,
    last: usize,
) -> TimelineViewportObservation {
    TimelineViewportObservation {
        display_window_revision: Some(state.window_revision),
        first_visible_event_id: Some(format!("$event{first}:example.invalid")),
        last_visible_event_id: Some(format!("$event{last}:example.invalid")),
        ..Default::default()
    }
}
fn context(observation: &TimelineViewportObservation) -> DisplayProjectionContext {
    DisplayProjectionContext::for_timeline(&room_key().kind, observation)
}
fn move_window(
    state: &mut DisplayProjectionState,
    canonical: &[TimelineItem],
    first: usize,
    last: usize,
    direction: Option<PaginationDirection>,
) {
    let observation = observe(state, first, last);
    let before = state.display_items.clone();
    let diffs =
        state.reconcile_bounded_window(canonical, &observation, &context(&observation), direction);
    let mut replay = before;
    apply_timeline_diffs_to_items(&mut replay, &diffs);
    assert_eq!(replay, state.display_items);
    assert!(state.slots.len() <= 240);
    for index in [first, last] {
        assert!(state.slots.iter().any(|slot| slot.canonical_index == index));
    }
}

#[test]
fn cached_window_moves_both_ways_without_unbounded_membership() {
    let canonical = items(10_000);
    let mut state = DisplayProjectionState::from_canonical_window(&canonical, 9880..10_000);
    for _ in 0..30 {
        let first = state.slots.first().unwrap().canonical_index;
        move_window(
            &mut state,
            &canonical,
            first,
            first + 10,
            Some(PaginationDirection::Backward),
        );
    }
    assert!(!state.window_reaches_live_end(canonical.len()));
    let oldest = state.slots.first().unwrap().canonical_index;
    assert!(oldest < 9000);
    for _ in 0..30 {
        let last = state.slots.last().unwrap().canonical_index;
        move_window(&mut state, &canonical, last - 10, last, None);
    }
    assert!(state.window_reaches_live_end(canonical.len()));
}

#[test]
fn stale_and_duplicate_evidence_cannot_relocate_a_window_twice() {
    let canonical = items(1000);
    let mut state = DisplayProjectionState::from_canonical_window(&canonical, 880..1000);
    let observation = observe(&state, 880, 890);
    let context = context(&observation);
    state.reconcile_bounded_window(
        &canonical,
        &observation,
        &context,
        Some(PaginationDirection::Backward),
    );
    let after = state.clone();
    assert!(
        state
            .reconcile_bounded_window(
                &canonical,
                &observation,
                &context,
                Some(PaginationDirection::Backward)
            )
            .is_empty()
    );
    assert_eq!(after, state);
    let current_observation = observe(&state, 880, 890);
    assert!(
        state
            .reconcile_bounded_window(
                &canonical,
                &current_observation,
                &context,
                Some(PaginationDirection::Backward)
            )
            .is_empty()
    );
    assert_eq!(after, state);
}

#[test]
fn hidden_only_spans_do_not_stall_or_allocate_the_whole_span() {
    let mut canonical = items(10_000);
    for item in &mut canonical[10..9880] {
        item.is_hidden = true;
    }
    let mut state = DisplayProjectionState::from_canonical_window(&canonical, 9880..10_000);
    move_window(
        &mut state,
        &canonical,
        9880,
        9890,
        Some(PaginationDirection::Backward),
    );
    assert_eq!(state.first_visible_canonical_index(), Some(0));
    assert_eq!(state.slots.len(), 130);
    assert!(state.display_items.iter().any(|item| matches!(&item.id, TimelineItemId::Event { event_id } if event_id == "$event0:example.invalid")));
}

#[test]
fn hidden_suffix_can_reach_the_real_live_end() {
    let mut canonical = items(1000);
    for item in &mut canonical[800..] {
        item.is_hidden = true;
    }
    let mut state = DisplayProjectionState::from_canonical_window(&canonical, 680..800);
    move_window(&mut state, &canonical, 790, 799, None);
    assert!(state.window_reaches_live_end(canonical.len()));
    let metadata = state.window_metadata(canonical.len());
    assert_eq!(
        metadata.last_event_id.as_deref(),
        Some("$event799:example.invalid")
    );
}

#[test]
fn sdk_growth_is_trimmed_around_the_observed_anchor() {
    let canonical = items(1000);
    let mut state = DisplayProjectionState::from_canonical_window(&canonical, 0..1000);
    let observation = observe(&state, 400, 410);
    state.bound_after_batch(&context(&observation));
    assert_eq!(state.slots.len(), 240);
    assert!(state.slots.iter().any(|slot| slot.canonical_index == 400));
    assert!(state.slots.iter().any(|slot| slot.canonical_index == 410));
}

#[test]
fn window_observation_requires_revision_and_represented_ordered_endpoints() {
    let canonical = items(1000);
    let state = DisplayProjectionState::from_canonical_window(&canonical, 880..1000);
    let valid = observe(&state, 900, 910);
    assert!(state.observation_is_current(&valid));
    let mut invalid = valid.clone();
    invalid.display_window_revision = None;
    assert!(!state.observation_is_current(&invalid));
    invalid = observe(&state, 100, 910);
    assert!(!state.observation_is_current(&invalid));
    invalid = observe(&state, 910, 900);
    assert!(!state.observation_is_current(&invalid));
}

#[test]
fn cached_window_movement_preserves_manager_owned_pending_sends() {
    let canonical = items(1000);
    let mut state = DisplayProjectionState::from_canonical_window(&canonical, 880..1000);
    let mut pending = timeline_item(
        "$pending:example.invalid",
        Some("synthetic"),
        "@member:example.invalid",
        false,
    );
    pending.id = TimelineItemId::Transaction {
        transaction_id: "synthetic-transaction".to_owned(),
    };
    pending.send_state = Some(TimelineSendState::Sending);
    let observation = observe(&state, 880, 890);
    state.replace_pending(
        vec![pending.clone()],
        HashSet::new(),
        &context(&observation),
    );
    move_window(
        &mut state,
        &canonical,
        880,
        890,
        Some(PaginationDirection::Backward),
    );
    assert!(state.display_items.iter().any(|item| item.id == pending.id));
}

#[test]
fn projected_activity_identity_resolves_back_to_the_content_slot() {
    let canonical = items(1000);
    let mut state = DisplayProjectionState::from_canonical_window(&canonical, 880..1000);
    let metadata = state.display_items[0].display_metadata.as_mut().unwrap();
    metadata.activity_event_id = Some("$reply:example.invalid".to_owned());
    let observation = TimelineViewportObservation {
        display_window_revision: Some(state.window_revision),
        first_visible_event_id: Some("$reply:example.invalid".to_owned()),
        last_visible_event_id: Some("$reply:example.invalid".to_owned()),
        ..Default::default()
    };
    assert!(state.observation_is_current(&observation));
    let diffs = state.reconcile_bounded_window(
        &canonical,
        &observation,
        &context(&observation),
        Some(PaginationDirection::Backward),
    );
    assert!(!diffs.is_empty());
    assert_eq!(state.slots.first().unwrap().canonical_index, 820);
}

#[test]
fn display_only_pending_event_is_valid_read_evidence_but_cannot_move_window() {
    let canonical = items(1000);
    let mut state = DisplayProjectionState::from_canonical_window(&canonical, 880..1000);
    let pending = timeline_item(
        "$pending:example.invalid",
        Some("synthetic"),
        "@member:example.invalid",
        false,
    );
    let observation = observe(&state, 990, 999);
    state.replace_pending(vec![pending], HashSet::new(), &context(&observation));
    let observation = TimelineViewportObservation {
        display_window_revision: Some(state.window_revision),
        first_visible_event_id: Some("$pending:example.invalid".to_owned()),
        last_visible_event_id: Some("$pending:example.invalid".to_owned()),
        ..Default::default()
    };
    assert!(state.observation_is_current(&observation));
    assert!(
        state
            .reconcile_bounded_window(
                &canonical,
                &observation,
                &context(&observation),
                Some(PaginationDirection::Backward)
            )
            .is_empty()
    );
}

#[test]
fn return_to_live_replaces_historical_window_with_bounded_latest_history() {
    let mut canonical = items(10_000);
    for item in &mut canonical[9000..] {
        item.is_hidden = true;
    }
    let mut state = DisplayProjectionState::from_canonical_window(&canonical, 500..740);
    let mut observation = observe(&state, 500, 510);
    observation.at_bottom = true;
    let before = state.display_items.clone();
    let old_revision = state.window_revision;
    let diffs = state.return_to_live_window(&canonical, &context(&observation));
    assert!(state.window_reaches_live_end(canonical.len()));
    assert!(state.window_revision > old_revision);
    assert!(state.slots.len() <= 240);
    assert!(state.display_items.iter().any(|item| matches!(&item.id, TimelineItemId::Event { event_id } if event_id == "$event8999:example.invalid")));
    let mut replay = before;
    apply_timeline_diffs_to_items(&mut replay, &diffs);
    assert_eq!(replay, state.display_items);
}
