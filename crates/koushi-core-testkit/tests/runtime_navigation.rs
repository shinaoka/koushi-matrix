//! Runtime navigation persistence integration tests.

use std::time::Duration;

use koushi_core::{CoreCommand, CoreRuntime, executor};
use koushi_protocol::command::AppCommand;
use koushi_state::{
    AppAction, HomeSelection, NavigationPreferenceUpdate, RoomSummary, SessionState,
    SpaceLocalPresentation, SpaceLocalPresentations, SpaceSummary,
};

mod support;
use support::*;

#[tokio::test]
async fn navigation_selection_persists_when_runtime_restarts() {
    let data_dir = tempfile::tempdir().expect("data dir");
    let credential_dir = tempfile::tempdir().expect("credential dir");
    {
        let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
            data_dir.path().to_path_buf(),
            credential_dir.path().to_path_buf(),
        );
        let mut connection = runtime.attach();
        runtime
            .inject_actions(restore_ready_actions![
                AppAction::RoomListUpdated {
                    spaces: vec![space_summary(
                        "!space-a:example.test",
                        &["!room-a:example.test"],
                    )],
                    rooms: vec![
                        room_in_space("!room-a:example.test", "!space-a:example.test"),
                        room_summary("!room-home:example.test"),
                    ],
                },
                AppAction::SelectSpace {
                    space_id: Some("!space-a:example.test".to_owned()),
                },
                AppAction::SelectRoom {
                    room_id: "!room-a:example.test".to_owned(),
                },
            ])
            .await;

        wait_for_state(&mut connection, |state| {
            state.navigation.active_space_id.as_deref() == Some("!space-a:example.test")
                && state.navigation.active_room_id.as_deref() == Some("!room-a:example.test")
        })
        .await;
        // Causally wait for the post-commit navigation persist before exercising
        // a memory-clearing verification-gate transition.
        runtime
            .inject_composer_drafts_and_wait_for_testing(
                connection.snapshot().composer_drafts.clone(),
            )
            .await;
        runtime
            .inject_actions(vec![
                AppAction::SessionLocked,
                AppAction::AuthoritativeDeviceTrustChanged {
                    generation: 1,
                    transition_id: 1,
                    trust: koushi_state::CurrentDeviceTrustState::Verified,
                },
            ])
            .await;
        wait_for_state(&mut connection, |state| {
            matches!(state.session, SessionState::Ready(_))
                && state.navigation.active_space_id.as_deref() == Some("!space-a:example.test")
                && state.navigation.active_room_id.as_deref() == Some("!room-a:example.test")
        })
        .await;
        // Selection state is published before post-commit persistence. Ordered
        // shutdown is the causal barrier that proves persistence completed.
        drop(connection);
        runtime.shutdown().await;
    }

    let restarted = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    let mut connection = restarted.attach();
    restarted
        .inject_actions(restore_ready_actions![AppAction::RoomListUpdated {
            spaces: vec![space_summary(
                "!space-a:example.test",
                &["!room-a:example.test"],
            )],
            rooms: vec![
                room_in_space("!room-a:example.test", "!space-a:example.test"),
                room_summary("!room-home:example.test"),
            ],
        },])
        .await;

    let snapshot = executor::timeout(Duration::from_secs(1), async {
        wait_for_state(&mut connection, |state| {
            matches!(state.session, SessionState::Ready(_))
                && state.navigation.active_space_id.as_deref() == Some("!space-a:example.test")
                && state.navigation.active_room_id.as_deref() == Some("!room-a:example.test")
        })
        .await
    })
    .await
    .expect("persisted navigation should be restored after room list reload");

    assert_eq!(
        snapshot
            .navigation
            .last_room_by_space_id
            .get("!space-a:example.test"),
        Some(&"!room-a:example.test".to_owned())
    );
}

#[tokio::test]
async fn legacy_navigation_import_persists_once_in_the_encrypted_store() {
    let data_dir = tempfile::tempdir().expect("data dir");
    let credential_dir = tempfile::tempdir().expect("credential dir");
    let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    let mut connection = runtime.attach();
    runtime.inject_actions(restore_ready_actions()).await;
    wait_for_state(&mut connection, |state| {
        matches!(state.session, SessionState::Ready(_))
    })
    .await;

    let imported = NavigationPreferenceUpdate::ImportLegacy {
        home_selection: Some(HomeSelection::DirectMessage {
            room_id: "!dm:example.test".to_owned(),
        }),
        space_local_presentations: SpaceLocalPresentations(std::collections::BTreeMap::from([(
            "!space:example.test".to_owned(),
            SpaceLocalPresentation {
                name: Some("Private local label".to_owned()),
                icon: Some("🧪".to_owned()),
            },
        )])),
    };
    connection
        .command_with_admission(CoreCommand::App(AppCommand::UpdateNavigationPreference {
            request_id: connection.next_request_id(),
            update: imported,
        }))
        .await
        .expect("import navigation preferences");

    let snapshot = connection.snapshot();
    assert!(snapshot.navigation.legacy_frontend_preferences_imported);
    assert!(matches!(
        snapshot.navigation.home_selection,
        HomeSelection::DirectMessage { ref room_id } if room_id == "!dm:example.test"
    ));

    connection
        .command_with_admission(CoreCommand::App(AppCommand::UpdateNavigationPreference {
            request_id: connection.next_request_id(),
            update: NavigationPreferenceUpdate::ImportLegacy {
                home_selection: Some(HomeSelection::Activity),
                space_local_presentations: SpaceLocalPresentations::default(),
            },
        }))
        .await
        .expect("admit ignored replay");
    assert!(matches!(
        connection.snapshot().navigation.home_selection,
        HomeSelection::DirectMessage { ref room_id } if room_id == "!dm:example.test"
    ));

    drop(connection);
    runtime.shutdown().await;

    let restarted = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    let mut connection = restarted.attach();
    restarted.inject_actions(restore_ready_actions()).await;
    let restored = wait_for_state(&mut connection, |state| {
        state.navigation.legacy_frontend_preferences_imported
    })
    .await;
    assert_eq!(
        restored
            .navigation
            .space_local_presentations
            .0
            .get("!space:example.test")
            .and_then(|presentation| presentation.name.as_deref()),
        Some("Private local label")
    );
}

fn space_summary(space_id: &str, child_room_ids: &[&str]) -> SpaceSummary {
    SpaceSummary {
        space_id: space_id.to_owned(),
        raw_name: None,
        display_name: "QA Space".to_owned(),
        avatar: None,
        join_rule: None,
        child_room_ids: child_room_ids
            .iter()
            .map(|room_id| (*room_id).to_owned())
            .collect(),
        parent_side_child_room_ids: child_room_ids
            .iter()
            .map(|room_id| (*room_id).to_owned())
            .collect(),
    }
}

fn room_in_space(room_id: &str, space_id: &str) -> RoomSummary {
    RoomSummary {
        parent_space_ids: vec![space_id.to_owned()],
        ..room_summary(room_id)
    }
}

/// #971: scrolling emitted one encrypted write + `sync_all()` + rename per
/// scroll-anchor update — about ten durable writes per second while the wheel
/// was moving. Anchor-only changes must coalesce into a single write; explicit
/// preference mutations must still persist immediately.
#[tokio::test]
async fn scroll_anchor_updates_coalesce_into_one_navigation_persist() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock_async().await;
    let data_dir = tempfile::tempdir().expect("data dir");
    let credential_dir = tempfile::tempdir().expect("credential dir");
    let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    let mut connection = runtime.attach();
    runtime
        .inject_actions(restore_ready_actions![
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![room_summary("!room-a:example.test")],
            },
            AppAction::SelectRoom {
                room_id: "!room-a:example.test".to_owned(),
            },
        ])
        .await;
    wait_for_state(&mut connection, |state| {
        matches!(state.session, SessionState::Ready(_))
            && state.navigation.active_room_id.as_deref() == Some("!room-a:example.test")
    })
    .await;

    // Let the selection write settle so the baseline covers only the scroll.
    executor::sleep(Duration::from_millis(1_200)).await;
    let baseline = navigation_persist_count();
    const SCROLL_UPDATES: usize = 20;
    for step in 0..SCROLL_UPDATES {
        connection
            .command(CoreCommand::App(AppCommand::TimelineScrollAnchorUpdated {
                request_id: connection.next_request_id(),
                room_id: "!room-a:example.test".to_owned(),
                anchor: koushi_state::TimelineScrollAnchor {
                    event_id: "$anchor:example.test".to_owned(),
                    edge: Default::default(),
                    offset_px: step as i32 * 100,
                    updated_at_ms: 1_800_000_000_000 + step as u64,
                },
            }))
            .await
            .expect("scroll anchor command");
    }
    wait_for_state(&mut connection, |state| {
        state
            .navigation
            .room_scroll_anchors
            .get("!room-a:example.test")
            .is_some_and(|anchor| anchor.offset_px == (SCROLL_UPDATES as i32 - 1) * 100)
    })
    .await;

    // Let the debounce elapse and settle.
    executor::sleep(Duration::from_millis(1_200)).await;
    let scroll_persists = navigation_persist_count() - baseline;
    assert_eq!(
        scroll_persists, 1,
        "{SCROLL_UPDATES} scroll-anchor updates must coalesce into exactly one durable \
         navigation write, not {scroll_persists}"
    );

    // An explicit preference mutation is not debounced.
    let before_preference = navigation_persist_count();
    connection
        .command_with_admission(CoreCommand::App(AppCommand::UpdateNavigationPreference {
            request_id: connection.next_request_id(),
            update: NavigationPreferenceUpdate::SetHomeSelection {
                selection: HomeSelection::Explore,
            },
        }))
        .await
        .expect("home selection preference");
    // Immediate means well inside the debounce window, not merely eventually.
    let mut waited = Duration::ZERO;
    while navigation_persist_count() == before_preference && waited < Duration::from_millis(200) {
        executor::sleep(Duration::from_millis(10)).await;
        waited += Duration::from_millis(10);
    }
    assert_eq!(
        navigation_persist_count() - before_preference,
        1,
        "an explicit preference mutation must persist immediately, not behind the \
         {NAVIGATION_PERSIST_DEBOUNCE_MS}ms scroll debounce"
    );

    drop(connection);
    runtime.shutdown().await;
}

const NAVIGATION_PERSIST_DEBOUNCE_MS: u64 = 500;

fn navigation_persist_count() -> usize {
    koushi_diagnostics::test_support::detail_snapshot()
        .records
        .iter()
        .filter(|record| {
            record.event.source == "core.space_order" && record.event.stage == "persisted"
        })
        .count()
}
