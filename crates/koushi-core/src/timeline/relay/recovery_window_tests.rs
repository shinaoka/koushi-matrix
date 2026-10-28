use super::*;
use crate::timeline::test_support::{room_key, timeline_item};

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
