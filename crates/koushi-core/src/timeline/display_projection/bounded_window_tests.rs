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
    state.bound_after_batch(&canonical, &context(&observation));
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

#[test]
fn bounded_reset_live_tail_keeps_messages_before_a_long_hidden_suffix() {
    let mut canonical = items(1120);
    for item in &mut canonical[120..] {
        item.is_hidden = true;
    }
    let observation = TimelineViewportObservation {
        at_bottom: true,
        ..Default::default()
    };
    let context = context(&observation);
    let mut state = DisplayProjectionState::default();
    state.return_to_live_window(&canonical, &context);
    let replacement = canonical.clone();
    project_sdk_batch(
        &mut canonical,
        &mut state,
        &[TimelineDiff::Reset { items: replacement }],
        &context,
    );
    assert_eq!(
        state
            .display_items()
            .iter()
            .filter(|item| !item.is_hidden)
            .count(),
        120
    );
    assert!(state.slots.len() <= 240);
    assert!(state.window_reaches_live_end(canonical.len()));
}

#[test]
fn bounded_reset_historical_window_preserves_both_sparse_viewport_endpoints() {
    let mut canonical = items(700);
    for item in &mut canonical[100..600] {
        item.is_hidden = true;
    }
    let mut state = DisplayProjectionState::default();
    let live = TimelineViewportObservation {
        at_bottom: true,
        ..Default::default()
    };
    state.return_to_live_window(&canonical, &context(&live));
    let observation = observe(&state, 95, 605);
    let context = context(&observation);
    let replacement = canonical.clone();
    project_sdk_batch(
        &mut canonical,
        &mut state,
        &[TimelineDiff::Reset { items: replacement }],
        &context,
    );
    for index in [95, 605] {
        assert!(
            state.slots.iter().any(|slot| slot.canonical_index == index),
            "Reset must retain both observed sparse endpoints"
        );
    }
    assert!(state.slots.len() <= 240);
}

/// A root whose root event and latest reply both lie outside the canonical
/// slots, as when the reply reaches the room only through the thread list.
/// #1657: it is displayed only while `activity_timestamp_ms` falls inside the
/// window's time span, so callers place its activity there.
const STANDALONE_REPLY: &str = "$reply:example.invalid";

fn standalone_root_context(
    canonical: &[TimelineItem],
    root_index: usize,
    activity_timestamp_ms: u64,
    observation: &TimelineViewportObservation,
) -> DisplayProjectionContext {
    use crate::threads_list::{AuthoritativeThreadAggregate, ThreadRootDisplayData};
    let root_id = format!("$event{root_index}:example.invalid");
    context(observation).with_thread_roots(
        TimelineThreadRootOrder::LatestReply,
        vec![ThreadRootDisplayData {
            root_event_id: root_id,
            unread: 0,
            activity_event_id: STANDALONE_REPLY.to_owned(),
            activity_timestamp_ms: Some(activity_timestamp_ms),
            item: Some(canonical[root_index].clone()),
            aggregate: AuthoritativeThreadAggregate {
                reply_count: 1,
                latest_event_id: Some(STANDALONE_REPLY.to_owned()),
                latest_sender: None,
                latest_sender_label: None,
                latest_body_preview: None,
                latest_timestamp_ms: Some(activity_timestamp_ms),
            },
            pending: false,
            failure_kind: None,
        }],
    )
}

fn observe_root(
    state: &DisplayProjectionState,
    first: Option<usize>,
    last: Option<usize>,
) -> TimelineViewportObservation {
    let id = |index: Option<usize>| {
        index.map_or_else(
            || STANDALONE_REPLY.to_owned(),
            |index| format!("$event{index}:example.invalid"),
        )
    };
    TimelineViewportObservation {
        display_window_revision: Some(state.window_revision),
        first_visible_event_id: Some(id(first)),
        last_visible_event_id: Some(id(last)),
        ..Default::default()
    }
}

#[test]
fn standalone_root_at_top_allows_cached_backward_paging_and_rejects_duplicate_evidence() {
    let mut canonical = items(1000);
    for (index, item) in canonical.iter_mut().enumerate() {
        item.timestamp_ms = Some(index as u64);
    }
    let mut state = DisplayProjectionState::from_canonical_window(&canonical, 880..1000);
    let observation = observe_root(&state, None, Some(890));
    let context = standalone_root_context(&canonical, 0, 880, &observation);
    state.reproject(&context);
    assert!(state.observation_is_current(&observation));
    let diffs = state.reconcile_bounded_window(
        &canonical,
        &observation,
        &context,
        Some(PaginationDirection::Backward),
    );
    assert!(
        !diffs.is_empty(),
        "a genuinely displayed root without canonical slots must not block cached pagination"
    );
    assert_eq!(state.first_visible_canonical_index(), Some(820));
    assert!(
        state
            .display_items()
            .iter()
            .any(|item| item.id == canonical[0].id)
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
}

#[test]
fn standalone_root_at_bottom_allows_cached_forward_paging() {
    let mut canonical = items(1000);
    for (index, item) in canonical.iter_mut().enumerate() {
        item.timestamp_ms = Some(index as u64);
    }
    let mut state = DisplayProjectionState::from_canonical_window(&canonical, 0..120);
    let observation = observe_root(&state, Some(110), None);
    let context = standalone_root_context(&canonical, 999, 119, &observation);
    state.reproject(&context);
    assert!(state.observation_is_current(&observation));
    let diffs = state.reconcile_bounded_window(&canonical, &observation, &context, None);
    assert!(
        !diffs.is_empty(),
        "a standalone root at the rendered bottom must not block forward movement"
    );
    assert_eq!(state.slots.last().unwrap().canonical_index, 179);
    assert!(
        state
            .display_items()
            .iter()
            .any(|item| item.id == canonical[999].id)
    );
    assert!(state.slots.len() <= 240);
}

#[test]
fn standalone_root_repeated_fresh_demand_always_advances_or_reaches_cached_start() {
    let mut canonical = items(1000);
    for (index, item) in canonical.iter_mut().enumerate() {
        item.timestamp_ms = Some(index as u64);
    }
    let mut state = DisplayProjectionState::from_canonical_window(&canonical, 880..1000);
    let seed = observe_root(&state, None, Some(890));
    state.reproject(&standalone_root_context(&canonical, 0, 880, &seed));
    for _ in 0..20 {
        let oldest = state.slots.first().unwrap().canonical_index;
        if oldest == 0 {
            break;
        }
        // Keep the root at the top of each window: its activity is the
        // window's oldest timed row, so it stays inside the time span.
        let observation = observe_root(&state, None, Some(oldest + 10));
        let context = standalone_root_context(&canonical, 0, oldest as u64, &observation);
        let before = state.display_items().to_vec();
        let diffs = state.reconcile_bounded_window(
            &canonical,
            &observation,
            &context,
            Some(PaginationDirection::Backward),
        );
        assert!(
            state.slots.first().unwrap().canonical_index < oldest,
            "cached demand must make strict progress, not leave a retry loop at the same boundary"
        );
        assert!(state.slots.len() <= 240);
        let mut replay = before;
        apply_timeline_diffs_to_items(&mut replay, &diffs);
        assert_eq!(replay, state.display_items());
        assert!(
            state
                .display_items()
                .iter()
                .any(|item| item.id == canonical[0].id)
        );
    }
    assert_eq!(state.first_visible_canonical_index(), Some(0));
    let observation = observe_root(&state, None, Some(20));
    let context = standalone_root_context(&canonical, 0, 0, &observation);
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
    let mut unknown = observation;
    unknown.first_visible_event_id = Some("$not-projected:example.invalid".to_owned());
    assert!(
        state
            .reconcile_bounded_window(
                &canonical,
                &unknown,
                &context,
                Some(PaginationDirection::Backward)
            )
            .is_empty()
    );
}

#[test]
fn standalone_root_only_viewport_forward_demand_makes_strict_progress() {
    let mut canonical = items(1000);
    for (index, item) in canonical.iter_mut().enumerate() {
        item.timestamp_ms = Some(index as u64);
        if index < 240 {
            item.is_hidden = true;
        }
    }
    let mut state = DisplayProjectionState::from_canonical_window(&canonical, 0..240);
    let observation = observe_root(&state, None, None);
    let context = standalone_root_context(&canonical, 999, 239, &observation);
    state.reproject(&context);
    assert_eq!(
        state
            .display_items()
            .iter()
            .filter(|item| !item.is_hidden)
            .count(),
        1
    );
    let diffs = state.reconcile_bounded_window(&canonical, &observation, &context, None);
    assert!(!diffs.is_empty());
    assert_eq!(state.slots.last().unwrap().canonical_index, 299);
    assert_eq!(state.slots.len(), 240);
    assert!(
        state
            .display_items()
            .iter()
            .any(|item| !item.is_hidden && item.id == canonical[240].id)
    );
}
