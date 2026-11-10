use super::*;
use crate::timeline::test_support::{room_key, send_row, timeline_item};

#[tokio::test]
async fn authoritative_recovery_preserves_historical_window_anchor() {
    let key = room_key();
    let gate = Arc::new(TimelineActorGenerationGate::default());
    let actor_generation = gate.activate_after_quiescence(&key).await.generation;
    let (events, mut receiver) = tokio::sync::broadcast::channel(16);
    let service = Arc::new(Mutex::new(ThreadRootProjectionService::default()));
    let mut canonical = (0..1000)
        .map(|index| {
            timeline_item(
                &format!("$recovery{index}:example.invalid"),
                Some("synthetic"),
                "@member:example.invalid",
                false,
            )
        })
        .collect::<Vec<_>>();
    let mut display = DisplayProjectionState::from_canonical_window(&canonical, 100..340);
    let expected = canonical[300].id.clone();
    let snapshot = canonical.clone();
    assert!(commit_authoritative_recovery_window(
        &mut canonical,
        &mut display,
        &HashMap::new(),
        &events,
        &service,
        koushi_state::TimelineThreadRootOrder::RootEvent,
        &TimelineViewportObservation {
            first_visible_event_id: Some("$recovery300:example.invalid".into()),
            last_visible_event_id: Some("$recovery310:example.invalid".into()),
            ..Default::default()
        },
        &gate,
        &key,
        actor_generation,
        TimelineGeneration(2),
        TimelineResyncReason::QueueOverflow,
        snapshot,
        || {},
    ));
    assert!(
        display
            .display_items()
            .iter()
            .any(|item| item.id == expected),
        "recovery must retain the historical reader anchor when its identity survives"
    );
    assert!(display.display_items().len() <= 240);
    assert_eq!(
        display.display_items().first().unwrap().id,
        canonical[100].id
    );
    assert_eq!(
        display.display_items().last().unwrap().id,
        canonical[339].id
    );
    let mut initial_seen = false;
    while let Ok(event) = receiver.try_recv() {
        if let CoreEvent::Timeline(TimelineEvent::InitialItems { items, .. }) = event {
            assert!(items.iter().any(|item| item.id == expected));
            initial_seen = true;
        }
    }
    assert!(initial_seen);
}

/// #1846: a replacement snapshot (overflow recovery, SDK or Core reset) rebuilds
/// the window from rows that carry only the SDK's report. The rendered send
/// state is derived from the room's table at publish time, so the resolved
/// `not_sent` head survives and its successor joins as blocked.
#[tokio::test]
async fn reset_recovery_rederives_not_sent_instead_of_copying_the_snapshot() {
    use koushi_protocol::event::TimelineSendFailureReason;
    let recoverable = koushi_protocol::event::TimelineSendState::NotSent {
        reason: TimelineSendFailureReason::Recoverable,
    };
    let key = room_key();
    let gate = Arc::new(TimelineActorGenerationGate::default());
    let actor_generation = gate.activate_after_quiescence(&key).await.generation;
    let (events, mut receiver) = tokio::sync::broadcast::channel(16);
    let service = Arc::new(Mutex::new(ThreadRootProjectionService::default()));
    let snapshot = vec![
        send_row("t1", Some(recoverable.clone())),
        send_row(
            "t2",
            Some(koushi_protocol::event::TimelineSendState::Sending),
        ),
    ];
    let send_statuses = HashMap::from([
        ("t1".to_owned(), recoverable.clone()),
        (
            "t2".to_owned(),
            koushi_protocol::event::TimelineSendState::Sending,
        ),
    ]);
    let mut canonical = Vec::new();
    let mut display = DisplayProjectionState::default();

    assert!(commit_authoritative_recovery_window(
        &mut canonical,
        &mut display,
        &send_statuses,
        &events,
        &service,
        koushi_state::TimelineThreadRootOrder::RootEvent,
        &TimelineViewportObservation {
            at_bottom: true,
            ..Default::default()
        },
        &gate,
        &key,
        actor_generation,
        TimelineGeneration(2),
        TimelineResyncReason::QueueOverflow,
        snapshot,
        || {},
    ));

    use koushi_protocol::event::TimelineItemId;
    let send_state = |items: &[TimelineItem], transaction_id: &str| {
        items
            .iter()
            .find(|item| {
                matches!(
                    &item.id,
                    TimelineItemId::Transaction { transaction_id: id } if id == transaction_id
                )
            })
            .and_then(|item| item.send_state.clone())
    };
    assert_eq!(
        send_state(display.display_items(), "t1"),
        Some(recoverable.clone())
    );
    assert_eq!(
        send_state(display.display_items(), "t2"),
        Some(recoverable.clone())
    );

    let mut initial_items = None;
    while let Ok(event) = receiver.try_recv() {
        if let CoreEvent::Timeline(TimelineEvent::InitialItems { items, .. }) = event {
            initial_items = Some(items);
        }
    }
    let initial_items = initial_items.expect("recovery publishes InitialItems");
    assert_eq!(send_state(&initial_items, "t1"), Some(recoverable.clone()));
    assert_eq!(
        send_state(&initial_items, "t2"),
        Some(recoverable),
        "the published window carries the derived state, not the snapshot copy"
    );
}
