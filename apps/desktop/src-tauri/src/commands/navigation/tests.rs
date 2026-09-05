use super::*;
use crate::commands::contracts::fake_request_id;

#[test]
fn open_timeline_at_timestamp_command_routes_through_app_command() {
    let command = build_open_timeline_at_timestamp_command(
        fake_request_id(40),
        "!room:example.org".to_owned(),
        1_718_000_000_000,
    );

    match command {
        CoreCommand::App(AppCommand::OpenTimelineAtTimestamp {
            request_id,
            room_id,
            timestamp_ms,
        }) => {
            assert_eq!(request_id, fake_request_id(40));
            assert_eq!(room_id, "!room:example.org");
            assert_eq!(timestamp_ms, 1_718_000_000_000);
            let debug = format!(
                "{:?}",
                AppCommand::OpenTimelineAtTimestamp {
                    request_id,
                    room_id,
                    timestamp_ms,
                }
            );
            assert!(!debug.contains("!room:example.org"), "{debug}");
            assert!(!debug.contains("1718000000000"), "{debug}");
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn observe_timeline_viewport_command_routes_viewport_facts_only() {
    let account_key = AccountKey("@alice:example.org".to_owned());
    let command = build_observe_timeline_viewport_command(
        fake_request_id(41),
        account_key.clone(),
        "!room:example.org".to_owned(),
        Some("$first".to_owned()),
        Some("$last".to_owned()),
        vec![koushi_core::TimelineGapId {
            topology_revision: 7,
            ordinal: 2,
        }],
        false,
        None,
    );
    let debug = format!("{command:?}");
    assert!(!debug.contains("!room:example.org"), "{debug}");
    assert!(!debug.contains("$first"), "{debug}");
    assert!(!debug.contains("$last"), "{debug}");

    match command {
        CoreCommand::Timeline(TimelineCommand::ObserveViewport {
            request_id,
            key,
            observation,
        }) => {
            assert_eq!(request_id, fake_request_id(41));
            assert_eq!(key.account_key, account_key);
            assert_eq!(
                key.kind,
                koushi_protocol::TimelineKind::Room {
                    room_id: "!room:example.org".to_owned()
                }
            );
            assert_eq!(
                observation.first_visible_event_id.as_deref(),
                Some("$first")
            );
            assert_eq!(observation.last_visible_event_id.as_deref(), Some("$last"));
            assert_eq!(
                observation.visible_gap_ids,
                vec![koushi_core::TimelineGapId {
                    topology_revision: 7,
                    ordinal: 2,
                }]
            );
            assert!(!observation.at_bottom);
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn observe_timeline_viewport_routes_thread_identity() {
    let command = build_observe_timeline_viewport_command(
        fake_request_id(43),
        AccountKey("@alice:example.org".to_owned()),
        "!room:example.org".to_owned(),
        Some("$reply".to_owned()),
        Some("$reply".to_owned()),
        Vec::new(),
        true,
        Some("$root".to_owned()),
    );
    let CoreCommand::Timeline(TimelineCommand::ObserveViewport { key, .. }) = command else {
        panic!("expected observe viewport command");
    };
    assert_eq!(
        key.kind,
        koushi_protocol::TimelineKind::Thread {
            room_id: "!room:example.org".to_owned(),
            root_event_id: "$root".to_owned(),
        }
    );
}

#[test]
fn observe_timeline_viewport_parses_full_range_topology_revision() {
    let visible_gap_ids: Vec<koushi_core::TimelineGapId> =
        serde_json::from_value(serde_json::json!([{
            "topology_revision": "14695981039346656037",
            "ordinal": 0,
        }]))
        .expect("Tauri viewport gap ids parse from their JSON wire shape");

    let command = build_observe_timeline_viewport_command(
        fake_request_id(42),
        AccountKey("@alice:example.org".to_owned()),
        "!room:example.org".to_owned(),
        None,
        None,
        visible_gap_ids,
        false,
        None,
    );

    let CoreCommand::Timeline(TimelineCommand::ObserveViewport { observation, .. }) = command
    else {
        panic!("expected observe viewport command");
    };
    assert_eq!(
        observation.visible_gap_ids,
        vec![koushi_core::TimelineGapId {
            topology_revision: 14_695_981_039_346_656_037,
            ordinal: 0,
        }]
    );
}

#[test]
fn event_navigation_source_selects_the_only_allowed_missing_target_policy() {
    assert_eq!(
        event_navigation_policy(koushi_state::EventNavigationSource::Activity),
        koushi_core::EventNavigationMissingTargetPolicy::LiveFallback
    );
    assert_eq!(
        event_navigation_policy(koushi_state::EventNavigationSource::Search),
        koushi_core::EventNavigationMissingTargetPolicy::LiveFallback
    );
    assert_eq!(
        event_navigation_policy(koushi_state::EventNavigationSource::Pinned),
        koushi_core::EventNavigationMissingTargetPolicy::Fail
    );
}

#[test]
fn event_navigation_transport_errors_are_coarse_and_private() {
    let error = invoke_error_from_event_navigation_error(EventNavigationError::Failed(
        koushi_state::EventNavigationFailureKind::Timeline,
    ));
    assert_eq!(error, "event navigation failed");
    assert!(!error.contains("example"));
}

#[tokio::test]
async fn focused_context_close_wait_uses_core_outcome_guards() {
    let (mut connection, control) = CoreConnection::new_for_testing(4);
    let request_id = fake_request_id(44);
    let account_key = AccountKey("@alice:example.org".to_owned());
    let room_id = Some("!room:example.org".to_owned());
    let mut state = koushi_state::AppState::default();
    state.session = koushi_state::SessionState::Ready(koushi_state::SessionInfo {
        homeserver: "https://example.org".to_owned(),
        user_id: account_key.0.clone(),
        device_id: "DEVICE".to_owned(),
        authentication_method: Default::default(),
    });
    state.navigation.active_room_id = room_id.clone();
    state.focused_context = koushi_state::FocusedContextState::Closed;
    control.send_event(CoreEvent::IntentLifecycle {
        request_id,
        outcome: IntentOutcome::Committed,
        published_generation: 1,
    });
    control.send_snapshot(koushi_protocol::state_update::VersionedAppStateSnapshot {
        generation: 1,
        state,
    });

    let snapshot = wait_for_focused_context_closed(
        &mut connection,
        request_id,
        account_key,
        room_id,
        0,
        tokio::time::Instant::now() + std::time::Duration::from_secs(1),
    )
    .await
    .expect("focused close should settle through Core");
    assert_eq!(snapshot.generation, 1);
}
