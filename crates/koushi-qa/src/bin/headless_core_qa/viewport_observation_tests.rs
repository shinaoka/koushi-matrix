use super::*;
use crate::contracts::{ScriptedQaEventSource, withheld_projection_test_item};
use crate::{AccountKey, Duration, TimelineReadStateSync, TimelineUnreadPosition};
use koushi_protocol::event::{TimelineDisplayWindow, TimelineNavigationSnapshot};
use koushi_protocol::ids::{RuntimeConnectionId, TimelineGeneration};
fn cause() -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(1),
        sequence: 2,
    }
}
fn key() -> TimelineKey {
    TimelineKey::room(
        AccountKey("@member:example.invalid".into()),
        "!room:example.invalid",
    )
}
fn initial(cause_request_id: RequestId) -> CoreEvent {
    CoreEvent::Timeline(TimelineEvent::InitialItems {
        request_id: Some(cause()),
        cause_request_id: Some(cause_request_id),
        key: key(),
        actor_generation: 1,
        generation: TimelineGeneration(0),
        items: vec![withheld_projection_test_item(
            "$visible:example.invalid",
            "Synthetic",
        )],
    })
}
fn navigation(window: Option<TimelineDisplayWindow>) -> CoreEvent {
    CoreEvent::Timeline(TimelineEvent::NavigationUpdated {
        key: key(),
        snapshot: Box::new(TimelineNavigationSnapshot {
            display_window: window,
            read_marker_event_id: None,
            read_marker_display_event_id: None,
            first_unread_event_id: None,
            first_unread_display_event_id: None,
            unread_event_count: 0,
            unread_position: TimelineUnreadPosition::None,
            newer_event_count: 0,
            can_jump_to_bottom: false,
            local_viewed_event_id: None,
            server_confirmed_read_event_id: None,
            read_state_sync: TimelineReadStateSync::NotRequested,
        }),
    })
}
fn window(revision: u64) -> TimelineDisplayWindow {
    TimelineDisplayWindow {
        revision,
        first_event_id: Some("$visible:example.invalid".into()),
        last_event_id: Some("$visible:example.invalid".into()),
        reaches_live_end: true,
    }
}
fn observation() -> TimelineViewportObservation {
    TimelineViewportObservation {
        first_visible_event_id: Some("$visible:example.invalid".into()),
        last_visible_event_id: Some("$visible:example.invalid".into()),
        ..Default::default()
    }
}
#[tokio::test]
async fn replay_attaches_current_revision_after_causal_initial_and_metadata() {
    let mut source = ScriptedQaEventSource {
        events: [
            initial(cause()),
            navigation(None),
            navigation(Some(window(7))),
        ]
        .into(),
    };
    let observed = observation_from_replay(
        &mut source,
        &key(),
        cause(),
        observation(),
        QaEventDeadline::after(Duration::from_secs(1)),
    )
    .await
    .unwrap();
    assert_eq!(
        observed.display_window_revision,
        Some(7),
        "None is rejected by the production Room guard"
    );
}

#[tokio::test]
async fn replay_ignores_wrong_cause_then_uses_current_boundaries_after_diffs() {
    let mut wrong = cause();
    wrong.sequence += 1;
    let latest = withheld_projection_test_item("$latest:example.invalid", "Synthetic latest");
    let mut current = window(9);
    current.last_event_id = Some("$latest:example.invalid".into());
    current.reaches_live_end = false;
    let mut source = ScriptedQaEventSource {
        events: [
            initial(wrong),
            navigation(Some(window(1))),
            initial(cause()),
            crate::contracts::withheld_projection_items_updated(key(), latest),
            navigation(Some(window(7))),
            navigation(Some(current)),
        ]
        .into(),
    };
    let mut desired = observation();
    desired.at_bottom = true;
    let observed = observation_from_replay(
        &mut source,
        &key(),
        cause(),
        desired,
        QaEventDeadline::after(Duration::from_secs(1)),
    )
    .await
    .unwrap();
    assert_eq!(observed.display_window_revision, Some(9));
    assert!(!observed.at_bottom, "historical window bottom is not live");
}

#[tokio::test(start_paused = true)]
async fn replay_without_window_metadata_has_one_bounded_deadline() {
    let mut source = ScriptedQaEventSource {
        events: [initial(cause()), navigation(None)].into(),
    };
    let began = tokio::time::Instant::now();
    let error = observation_from_replay(
        &mut source,
        &key(),
        cause(),
        observation(),
        QaEventDeadline::after(Duration::from_secs(1)),
    )
    .await
    .unwrap_err();
    assert_eq!(tokio::time::Instant::now() - began, Duration::from_secs(1));
    assert!(error.contains("timed out"));
    assert!(!error.contains("example.invalid"));
}

#[tokio::test]
async fn replay_rejects_observation_outside_the_committed_display() {
    let mut source = ScriptedQaEventSource {
        events: [initial(cause()), navigation(Some(window(7)))].into(),
    };
    let mut desired = observation();
    desired.first_visible_event_id = Some("$outside:example.invalid".into());
    let error = observation_from_replay(
        &mut source,
        &key(),
        cause(),
        desired,
        QaEventDeadline::after(Duration::from_secs(1)),
    )
    .await
    .unwrap_err();
    assert!(error.contains("outside"));
    assert!(!error.contains("example.invalid"));
}

#[tokio::test]
async fn replay_boundary_skips_metadata_row_without_activity_identity() {
    let mut pending =
        withheld_projection_test_item("$pending-root:example.invalid", "Synthetic pending root");
    pending.display_metadata = Some(koushi_protocol::event::TimelineDisplayMetadata {
        row_id: "synthetic-pending-root".into(),
        kind: koushi_protocol::event::TimelineDisplayKind::ThreadRootPending,
        content_event_id: Some("$pending-root:example.invalid".into()),
        activity_event_id: None,
        display_timestamp_ms: None,
    });
    let mut initial = initial(cause());
    if let CoreEvent::Timeline(TimelineEvent::InitialItems { items, .. }) = &mut initial {
        items.insert(0, pending);
    }
    let mut source = ScriptedQaEventSource {
        events: [initial, navigation(Some(window(7)))].into(),
    };
    let observed = observation_from_replay(
        &mut source,
        &key(),
        cause(),
        observation(),
        QaEventDeadline::after(Duration::from_secs(1)),
    )
    .await
    .unwrap();
    assert_eq!(observed.display_window_revision, Some(7));
}
