use super::{
    LiveDirectEventTestSource, LiveObserverTestEvent, LiveRoomListReconciliation,
    RoomListObservationCommand, additional_room_list_pages, direct_account_data_initial_reason,
    missing_space_child_links, normalize_and_project_entries, project_room_list_snapshot,
    room_list_identity_counts, room_list_projection_admits_authority, room_list_range_is_complete,
    room_stop_matches_generation, run_live_room_list_observation_with_sources,
};

use crate::direct_message_classification::DirectClassificationState;

use crate::room::actor::{MissingSpaceChildLink, RoomListReconcileAck};

use koushi_sdk::{
    MatrixClientSession, MatrixRoomListRoom, MatrixRoomListSnapshot, MatrixRoomListSpace,
    MatrixRoomTags,
};

use koushi_state::SessionInfo;
use koushi_state::{AppAction, RoomListSource, UserProfile};

use matrix_sdk::ruma::events::direct::DirectEvent;

use std::{
    collections::BTreeSet,
    sync::{Arc, RwLock, atomic::AtomicBool},
    time::Duration,
};
use tokio::sync::{broadcast, mpsc, oneshot};

#[test]
fn room_list_identity_counts_distinguish_id_duplicates_from_name_collisions() {
    let counts = room_list_identity_counts([
        ("!one:example.org", "Element Web/Desktop"),
        ("!one:example.org", "Element Web/Desktop"),
        ("!two:example.org", "Element Web/Desktop"),
        ("!three:example.org", "Different"),
    ]);

    assert_eq!(counts.input_entry_count, 4);
    assert_eq!(counts.unique_room_id_count, 3);
    assert_eq!(counts.duplicate_entry_count, 1);
    assert_eq!(counts.display_name_collision_group_count, 1);
    assert_eq!(counts.display_name_collision_entry_count, 2);
}

async fn wait_for_live_observer_test_event(
    rx: &mut mpsc::UnboundedReceiver<LiveObserverTestEvent>,
    label: &'static str,
    predicate: impl Fn(&LiveObserverTestEvent) -> bool,
) -> LiveObserverTestEvent {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let event = rx.recv().await.expect("live observer test channel");
            if predicate(&event) {
                break event;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {label}"))
}

struct LiveObserverTestHarness {
    action_rx: mpsc::Receiver<Vec<AppAction>>,
    test_event_rx: mpsc::UnboundedReceiver<LiveObserverTestEvent>,
    direct_event_tx:
        Option<mpsc::UnboundedSender<matrix_sdk::ruma::events::direct::DirectEventContent>>,
    command_tx: mpsc::Sender<RoomListObservationCommand>,
    stop_tx: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

impl LiveObserverTestHarness {
    async fn next_actions(&mut self, label: &'static str) -> Vec<AppAction> {
        tokio::time::timeout(Duration::from_secs(1), self.action_rx.recv())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {label}"))
            .expect("action channel should stay open")
    }

    async fn expect_event(&mut self, label: &'static str, expected: LiveObserverTestEvent) {
        let actual = wait_for_live_observer_test_event(&mut self.test_event_rx, label, |event| {
            event == &expected
        })
        .await;
        assert_eq!(actual, expected);
    }

    fn send_direct_event(&self, content: matrix_sdk::ruma::events::direct::DirectEventContent) {
        self.direct_event_tx
            .as_ref()
            .expect("direct event source should be open")
            .send(content)
            .expect("direct event receiver");
    }

    fn close_direct_event_source(&mut self) {
        self.direct_event_tx.take();
    }

    async fn hydrate_space_members(&self, space_id: &str) {
        self.command_tx
            .send(RoomListObservationCommand::HydrateSpaceMembers {
                space_id: space_id.to_owned(),
            })
            .await
            .expect("observer command channel");
    }

    async fn stop(self) {
        let _ = self.stop_tx.send(());
        self.task.await.expect("observer task");
    }
}

async fn spawn_live_observer_test_harness(
    client: matrix_sdk::Client,
    homeserver: String,
    entries_limit: usize,
    room_updates_rx: broadcast::Receiver<matrix_sdk_base::sync::RoomUpdates>,
    direct_event_source: LiveDirectEventTestSource,
    entries_start_rx: Option<mpsc::Receiver<()>>,
) -> LiveObserverTestHarness {
    let service = Arc::new(
        matrix_sdk_ui::room_list_service::RoomListService::new(client.clone())
            .await
            .expect("room list service"),
    );
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client,
        SessionInfo {
            homeserver,
            user_id: "@observer:example.invalid".to_owned(),
            device_id: "OBSERVER".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    ));
    let direct_observer = session.client().observe_events::<DirectEvent, ()>();
    let direct_events = direct_observer.subscribe();
    let direct_state = DirectClassificationState::default();
    let known_room_ids = Arc::new(RwLock::new(BTreeSet::new()));
    let (room_tx, _room_rx) = mpsc::channel(4);
    let (action_tx, action_rx) = mpsc::channel(8);
    let (event_tx, _event_rx) = broadcast::channel(8);
    let (command_tx, command_rx) = mpsc::channel(1);
    let (stop_tx, stop_rx) = oneshot::channel();
    let (test_event_tx, test_event_rx) = mpsc::unbounded_channel();
    let (direct_event_tx, direct_events_rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(run_live_room_list_observation_with_sources(
        session,
        service,
        known_room_ids,
        Arc::new(RwLock::new(Vec::new())),
        room_tx,
        action_tx,
        event_tx,
        command_rx,
        stop_rx,
        1,
        RoomListSource::Live,
        Arc::new(AtomicBool::new(false)),
        crate::SlidingSyncDiagnostics::default(),
        None,
        direct_observer,
        direct_events,
        direct_state,
        entries_limit,
        room_updates_rx,
        Some(test_event_tx),
        direct_events_rx,
        direct_event_source,
        entries_start_rx,
    ));
    LiveObserverTestHarness {
        action_rx,
        test_event_rx,
        direct_event_tx: Some(direct_event_tx),
        command_tx,
        stop_tx,
        task,
    }
}

#[test]
fn missing_space_child_links_detects_parent_only_relationship() {
    let snapshot = MatrixRoomListSnapshot {
        spaces: vec![MatrixRoomListSpace {
            space_id: "!space:example.test".to_owned(),
            display_name: "My Space".to_owned(),
            avatar_mxc_uri: None,
            child_room_ids: Vec::new(),
            member_user_ids: Vec::new(),
        }],
        rooms: vec![MatrixRoomListRoom {
            room_id: "!room:example.test".to_owned(),
            display_name: "Room".to_owned(),
            avatar_mxc_uri: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: MatrixRoomTags::default(),
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            marked_unread: false,
            recency_stamp: None,
            conversation_activity: None,
            latest_event: None,
            parent_space_ids: vec!["!space:example.test".to_owned()],
            is_encrypted: true,
            joined_members: 1,
        }],
        ..MatrixRoomListSnapshot::default()
    };

    assert_eq!(
        missing_space_child_links(&snapshot),
        vec![MissingSpaceChildLink {
            space_id: "!space:example.test".to_owned(),
            child_room_id: "!room:example.test".to_owned(),
            via_server: "example.test".to_owned(),
        }]
    );
}

#[test]
fn missing_space_child_links_skips_reciprocal_relationship() {
    let snapshot = MatrixRoomListSnapshot {
        spaces: vec![MatrixRoomListSpace {
            space_id: "!space:example.test".to_owned(),
            display_name: "My Space".to_owned(),
            avatar_mxc_uri: None,
            child_room_ids: vec!["!room:example.test".to_owned()],
            member_user_ids: Vec::new(),
        }],
        rooms: vec![MatrixRoomListRoom {
            room_id: "!room:example.test".to_owned(),
            display_name: "Room".to_owned(),
            avatar_mxc_uri: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: MatrixRoomTags::default(),
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            marked_unread: false,
            recency_stamp: None,
            conversation_activity: None,
            latest_event: None,
            parent_space_ids: vec!["!space:example.test".to_owned()],
            is_encrypted: true,
            joined_members: 1,
        }],
        ..MatrixRoomListSnapshot::default()
    };

    assert!(missing_space_child_links(&snapshot).is_empty());
}

#[tokio::test]
async fn project_room_list_snapshot_updates_user_profiles() {
    let (action_tx, mut action_rx) = mpsc::channel(16);
    let (event_tx, _event_rx) = broadcast::channel(16);
    let known_room_ids = Arc::new(RwLock::new(BTreeSet::new()));
    let known_dm_rooms = Arc::new(RwLock::new(Vec::new()));
    let snapshot = MatrixRoomListSnapshot {
        user_profiles: vec![koushi_sdk::MatrixUserProfile {
            user_id: "@alice:example.test".to_owned(),
            display_name: Some("Alice".to_owned()),
            avatar_mxc_uri: None,
        }],
        ..MatrixRoomListSnapshot::default()
    };

    project_room_list_snapshot(
        &snapshot,
        &known_room_ids,
        &known_dm_rooms,
        &action_tx,
        &event_tx,
        1,
        RoomListSource::Live,
        true,
    )
    .await;

    let actions = action_rx.recv().await.expect("actions");
    assert!(
        matches!(
            actions.as_slice(),
            [
                AppAction::RoomNotificationModesObserved { .. },
                AppAction::RoomListSnapshotAuthoritative { .. },
                AppAction::UserProfilesUpdated { profiles },
            ] if *profiles == vec![UserProfile {
                user_id: "@alice:example.test".to_owned(),
                display_name: Some("Alice".to_owned()),
                display_label: "Alice".to_owned(),
                original_display_label: "Alice".to_owned(),
                mention_search_terms: vec![
                        "Alice".to_owned(),
                        "@alice:example.test".to_owned(),
                ],
                avatar: None,
            }]
        ),
        "expected UserProfilesUpdated action, got {actions:?}"
    );
}

#[tokio::test]
async fn project_room_list_snapshot_holds_unproven_empty_and_preserves_known_rooms() {
    let (action_tx, mut action_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = broadcast::channel(16);
    let known_room_ids = Arc::new(RwLock::new(BTreeSet::from([
        "!cached:example.test".to_owned()
    ])));
    let snapshot = MatrixRoomListSnapshot::default();
    let known_dm_rooms = Arc::new(RwLock::new(Vec::new()));

    project_room_list_snapshot(
        &snapshot,
        &known_room_ids,
        &known_dm_rooms,
        &action_tx,
        &event_tx,
        1,
        RoomListSource::Live,
        false,
    )
    .await;

    let actions = action_rx.recv().await.expect("provisional actions");
    assert!(matches!(
        actions.as_slice(),
        [AppAction::RoomNotificationModesObserved { .. },
            AppAction::RoomListSnapshotProvisional { rooms, invites, .. },
            AppAction::UserProfilesUpdated { .. }]
            if rooms.is_empty() && invites.is_empty()
    ));
    assert_eq!(
        known_room_ids
            .read()
            .expect("known rooms")
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec!["!cached:example.test".to_owned()]
    );
    assert!(event_rx.try_recv().is_err());
}

#[tokio::test]
async fn live_room_list_observer_projects_rooms_and_invites_from_service_entries() {
    use matrix_sdk::{
        ruma::{events::AnySyncStateEvent, room_id, serde::Raw, user_id},
        test_utils::mocks::MatrixMockServer,
    };
    use matrix_sdk_test::{InvitedRoomBuilder, JoinedRoomBuilder, event_factory::EventFactory};

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let visible_room_id = room_id!("!visible-room:example.invalid");
    let visible_room_name: Raw<AnySyncStateEvent> = EventFactory::new()
        .room(visible_room_id)
        .sender(user_id!("@sender:example.invalid"))
        .room_name("AAAA visible room")
        .into();
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(visible_room_id).add_state_event(visible_room_name),
        )
        .await;
    let invited_room_id = room_id!("!service-invite:example.invalid");
    let invited_room_name = EventFactory::new()
        .room(invited_room_id)
        .sender(user_id!("@sender:example.invalid"))
        .room_name("BBBB invited room");
    server
        .sync_room(
            &client,
            InvitedRoomBuilder::new(invited_room_id).add_state_event(invited_room_name),
        )
        .await;
    let room_updates_rx = client.subscribe_to_all_room_updates();
    let mut harness = spawn_live_observer_test_harness(
        client,
        server.uri(),
        2,
        room_updates_rx,
        LiveDirectEventTestSource::SdkAndInjected,
        None,
    )
    .await;

    let projected = harness.next_actions("initial service projection").await;
    assert!(projected.iter().any(|action| {
        matches!(
            action,
            AppAction::RoomListSnapshotProvisional { rooms, invites, .. }
                | AppAction::RoomListSnapshotAuthoritative { rooms, invites, .. }
                if rooms.iter().any(|room| room.room_id == visible_room_id.as_str())
                && invites.iter().any(|invite| invite.room_id == invited_room_id.as_str())
        )
    }));
    harness
        .expect_event(
            "initial service projection",
            LiveObserverTestEvent::RlsProjected {
                wake_count: 1,
                entries_len: 2,
            },
        )
        .await;

    harness.stop().await;
}

#[tokio::test]
async fn selected_space_hydration_completes_members_before_reprojection() {
    use matrix_sdk::{
        ruma::{RoomVersionId, events::AnySyncStateEvent, room_id, serde::Raw, user_id},
        test_utils::mocks::MatrixMockServer,
    };
    use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let inspect_client = client.clone();
    let space_id = room_id!("!hydrated-space:example.invalid");
    let own_user_id = user_id!("@observer:example.invalid");
    let factory = EventFactory::new().room(space_id).sender(own_user_id);
    let create: Raw<AnySyncStateEvent> = factory
        .create(own_user_id, RoomVersionId::V10)
        .with_space_type()
        .into();
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(space_id).add_state_event(create),
        )
        .await;
    server
        .mock_get_members()
        .ok(vec![factory.member(own_user_id).into()])
        .mock_once()
        .mount()
        .await;

    let room_updates_rx = client.subscribe_to_all_room_updates();
    let mut harness = spawn_live_observer_test_harness(
        client,
        server.uri(),
        2,
        room_updates_rx,
        LiveDirectEventTestSource::SdkAndInjected,
        None,
    )
    .await;
    let _ = harness
        .next_actions("initial partial Space projection")
        .await;
    let space = inspect_client.get_room(space_id).expect("Space room");
    assert!(!space.are_members_synced());

    harness.hydrate_space_members(space_id.as_str()).await;
    let _ = harness.next_actions("hydrated Space projection").await;
    assert!(space.are_members_synced());

    harness.stop().await;
}

#[tokio::test]
async fn live_room_list_observer_reclassifies_dm_from_direct_event_without_timeline_update() {
    use matrix_sdk::{
        ruma::{
            OwnedRoomId, OwnedUserId,
            events::{
                AnySyncStateEvent,
                direct::{DirectEventContent, OwnedDirectUserIdentifier},
            },
            room_id,
            serde::Raw,
            user_id,
        },
        test_utils::mocks::MatrixMockServer,
    };
    use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let dm_room_id = room_id!("!direct-event-room:example.invalid");
    let room_name: Raw<AnySyncStateEvent> = EventFactory::new()
        .room(dm_room_id)
        .sender(user_id!("@sender:example.invalid"))
        .room_name("Direct event room")
        .into();
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(dm_room_id).add_state_event(room_name),
        )
        .await;
    let room_updates_rx = client.subscribe_to_all_room_updates();
    let mut harness = spawn_live_observer_test_harness(
        client,
        server.uri(),
        2,
        room_updates_rx,
        LiveDirectEventTestSource::SdkAndInjected,
        None,
    )
    .await;

    loop {
        let initial = harness.next_actions("initial room projection").await;
        if initial.iter().any(|action| {
            matches!(
                action,
                AppAction::RoomListSnapshotProvisional { rooms, .. }
                    | AppAction::RoomListSnapshotAuthoritative { rooms, .. }
                    if rooms.iter().any(|room| room.room_id == dm_room_id.as_str())
            )
        }) {
            break;
        }
    }
    harness
        .expect_event(
            "initial room projection",
            LiveObserverTestEvent::RlsProjected {
                wake_count: 1,
                entries_len: 1,
            },
        )
        .await;

    let user_id: OwnedUserId = user_id!("@alice:example.invalid").to_owned();
    let room_id: OwnedRoomId = dm_room_id.to_owned();
    let mut content = DirectEventContent::default();
    content.insert(OwnedDirectUserIdentifier::from(user_id), vec![room_id]);
    harness.send_direct_event(content.clone());

    let projected = harness
        .next_actions("direct account-data reprojection")
        .await;
    assert!(projected.iter().any(|action| {
        matches!(
            action,
            AppAction::RoomListSnapshotProvisional { rooms, .. }
                | AppAction::RoomListSnapshotAuthoritative { rooms, .. }
                if rooms.iter().any(|room| room.room_id == dm_room_id.as_str() && room.is_dm)
        )
    }));
    assert!(matches!(
        projected.as_slice(),
        [
            AppAction::RoomNotificationModesObserved { .. },
            AppAction::RoomListSnapshotProvisional { .. }
                | AppAction::RoomListSnapshotAuthoritative { .. },
            AppAction::UserProfilesUpdated { .. }
        ]
    ));
    harness
        .expect_event(
            "direct account-data reprojection",
            LiveObserverTestEvent::DirectClassificationProjected {
                event_wake_count: 1,
                applied_update_count: 1,
                projected_dm_count: 1,
            },
        )
        .await;

    harness.send_direct_event(content);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), harness.action_rx.recv())
            .await
            .is_err()
    );

    harness.stop().await;
}

#[tokio::test]
async fn live_room_list_observer_defers_direct_event_projection_until_first_service_entries() {
    use matrix_sdk::{
        ruma::{
            OwnedRoomId, OwnedUserId,
            events::{
                AnySyncStateEvent,
                direct::{DirectEventContent, OwnedDirectUserIdentifier},
            },
            room_id,
            serde::Raw,
            user_id,
        },
        test_utils::mocks::MatrixMockServer,
    };
    use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let dm_room_id = room_id!("!direct-before-entries:example.invalid");
    let room_name: Raw<AnySyncStateEvent> = EventFactory::new()
        .room(dm_room_id)
        .sender(user_id!("@sender:example.invalid"))
        .room_name("Direct event before entries")
        .into();
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(dm_room_id).add_state_event(room_name),
        )
        .await;

    let room_updates_rx = client.subscribe_to_all_room_updates();
    let (entries_start_tx, entries_start_rx) = mpsc::channel(1);
    let mut harness = spawn_live_observer_test_harness(
        client,
        server.uri(),
        2,
        room_updates_rx,
        LiveDirectEventTestSource::SdkAndInjected,
        Some(entries_start_rx),
    )
    .await;

    let user_id: OwnedUserId = user_id!("@alice:example.invalid").to_owned();
    let room_id: OwnedRoomId = dm_room_id.to_owned();
    let mut content = DirectEventContent::default();
    content.insert(OwnedDirectUserIdentifier::from(user_id), vec![room_id]);
    harness.send_direct_event(content);
    harness
        .expect_event(
            "direct account-data state update before first service entries",
            LiveObserverTestEvent::DirectClassificationUpdated {
                event_wake_count: 1,
                applied_update_count: 1,
            },
        )
        .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(100), harness.action_rx.recv())
            .await
            .is_err()
    );

    entries_start_tx
        .send(())
        .await
        .expect("first service entries gate");
    let projected = harness
        .next_actions("first service projection after direct event")
        .await;
    assert!(matches!(
        projected.as_slice(),
        [
            AppAction::RoomNotificationModesObserved { .. },
            AppAction::RoomListSnapshotProvisional { rooms, .. }
                | AppAction::RoomListSnapshotAuthoritative { rooms, .. },
            AppAction::UserProfilesUpdated { .. }
        ]
            if rooms.iter().any(|room| room.room_id == dm_room_id.as_str() && room.is_dm)
    ));
    harness
        .expect_event(
            "first service projection after direct event",
            LiveObserverTestEvent::RlsProjected {
                wake_count: 1,
                entries_len: 1,
            },
        )
        .await;

    harness.stop().await;
}

#[tokio::test]
async fn live_room_list_observer_continues_after_test_direct_event_source_closes() {
    use matrix_sdk::{ruma::room_id, test_utils::mocks::MatrixMockServer};
    use matrix_sdk_test::JoinedRoomBuilder;

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let initial_room_id = room_id!("!initial-direct-source-close:example.invalid");
    server
        .sync_room(&client, JoinedRoomBuilder::new(initial_room_id))
        .await;

    let room_updates_rx = client.subscribe_to_all_room_updates();
    let mut harness = spawn_live_observer_test_harness(
        client.clone(),
        server.uri(),
        2,
        room_updates_rx,
        LiveDirectEventTestSource::InjectedOnly,
        None,
    )
    .await;
    let _ = harness.next_actions("initial service projection").await;
    harness
        .expect_event(
            "initial service projection",
            LiveObserverTestEvent::RlsProjected {
                wake_count: 1,
                entries_len: 1,
            },
        )
        .await;

    harness.close_direct_event_source();
    harness
        .expect_event(
            "direct event stream closure",
            LiveObserverTestEvent::DirectEventStreamClosed,
        )
        .await;
    let later_room_id = room_id!("!later-direct-source-close:example.invalid");
    server
        .sync_room(&client, JoinedRoomBuilder::new(later_room_id))
        .await;

    let projected = harness
        .next_actions("service projection after direct event source closes")
        .await;
    assert!(projected.iter().any(|action| {
        matches!(
            action,
            AppAction::RoomListSnapshotProvisional { rooms, .. }
                | AppAction::RoomListSnapshotAuthoritative { rooms, .. }
                if rooms.iter().any(|room| room.room_id == later_room_id.as_str())
        )
    }));
    harness
        .expect_event(
            "service projection after direct event source closes",
            LiveObserverTestEvent::RlsProjected {
                wake_count: 2,
                entries_len: 2,
            },
        )
        .await;

    harness.stop().await;
}

#[tokio::test]
async fn normalize_and_project_entries_uses_cached_direct_map_before_timeline_update() {
    use matrix_sdk::{ruma::room_id, test_utils::mocks::MatrixMockServer};
    use matrix_sdk_test::JoinedRoomBuilder;

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let dm_room_id = room_id!("!cached-direct-room:example.invalid");
    let room = server
        .sync_room(&client, JoinedRoomBuilder::new(dm_room_id))
        .await;
    let session = MatrixClientSession::from_client_for_testing(
        client,
        SessionInfo {
            homeserver: server.uri(),
            user_id: "@observer:example.invalid".to_owned(),
            device_id: "OBSERVER".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    );
    let current =
        eyeball_im::Vector::from_iter([matrix_sdk_ui::room_list_service::RoomListItem::from(room)]);
    let direct_state =
        crate::direct_message_classification::DirectClassificationState::from_targets(
            koushi_sdk::MatrixDirectTargetsByRoom::from([(dm_room_id.to_string(), Vec::new())]),
            crate::direct_message_classification::DirectAccountDataSource::LocalStore,
        );
    let known_room_ids = Arc::new(RwLock::new(BTreeSet::new()));
    let (room_tx, _room_rx) = mpsc::channel(4);
    let (action_tx, mut action_rx) = mpsc::channel(4);
    let (event_tx, _event_rx) = broadcast::channel(4);

    normalize_and_project_entries(
        &session,
        &current,
        direct_state.authoritative_targets(),
        &known_room_ids,
        &Arc::new(RwLock::new(Vec::new())),
        &room_tx,
        &action_tx,
        &event_tx,
        1,
        RoomListSource::Live,
        &Arc::new(AtomicBool::new(false)),
        None,
        None,
    )
    .await;

    let actions = action_rx.recv().await.expect("room projection");
    assert!(matches!(
        actions.as_slice(),
        [AppAction::RoomNotificationModesObserved { .. },
         AppAction::RoomListSnapshotProvisional { rooms, .. },
         AppAction::UserProfilesUpdated { .. }]
            if rooms.first().is_some_and(|room| room.is_dm)
    ));
}

#[test]
fn direct_account_data_initial_reason_tokens_are_bounded() {
    use koushi_sdk::MatrixCachedDirectAccountData;

    assert_eq!(
        direct_account_data_initial_reason(&MatrixCachedDirectAccountData::Missing),
        Some("missing")
    );
    assert_eq!(
        direct_account_data_initial_reason(&MatrixCachedDirectAccountData::StoreError),
        Some("store_error")
    );
    assert_eq!(
        direct_account_data_initial_reason(&MatrixCachedDirectAccountData::Invalid),
        Some("invalid")
    );
    assert_eq!(
        direct_account_data_initial_reason(&MatrixCachedDirectAccountData::Present(
            koushi_sdk::MatrixDirectTargetsByRoom::new(),
        )),
        None
    );
}

#[tokio::test]
async fn live_projection_does_not_import_base_client_only_invites() {
    use matrix_sdk::{
        ruma::{room_id, user_id},
        test_utils::mocks::MatrixMockServer,
    };
    use matrix_sdk_test::{InvitedRoomBuilder, JoinedRoomBuilder, event_factory::EventFactory};

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let visible_room_id = room_id!("!service-entry:example.invalid");
    let visible_room = server
        .sync_room(&client, JoinedRoomBuilder::new(visible_room_id))
        .await;
    let base_only_invite_id = room_id!("!base-only-invite:example.invalid");
    let invite_name = EventFactory::new()
        .room(base_only_invite_id)
        .sender(user_id!("@sender:example.invalid"))
        .room_name("Base-only invite");
    server
        .sync_room(
            &client,
            InvitedRoomBuilder::new(base_only_invite_id).add_state_event(invite_name),
        )
        .await;

    let session = MatrixClientSession::from_client_for_testing(
        client,
        SessionInfo {
            homeserver: server.uri(),
            user_id: "@observer:example.invalid".to_owned(),
            device_id: "OBSERVER".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    );
    let current =
        eyeball_im::Vector::from_iter([matrix_sdk_ui::room_list_service::RoomListItem::from(
            visible_room,
        )]);
    let known_room_ids = Arc::new(RwLock::new(BTreeSet::new()));
    let (room_tx, _room_rx) = mpsc::channel(4);
    let (action_tx, mut action_rx) = mpsc::channel(4);
    let (event_tx, _event_rx) = broadcast::channel(4);

    normalize_and_project_entries(
        &session,
        &current,
        None,
        &known_room_ids,
        &Arc::new(RwLock::new(Vec::new())),
        &room_tx,
        &action_tx,
        &event_tx,
        1,
        RoomListSource::Live,
        &Arc::new(AtomicBool::new(true)),
        None,
        None,
    )
    .await;

    let actions = action_rx.recv().await.expect("room projection");
    assert!(actions.iter().any(|action| {
        matches!(
            action,
            AppAction::RoomListSnapshotAuthoritative { rooms, invites, .. }
                if rooms.iter().any(|room| room.room_id == visible_room_id.as_str())
                && !invites.iter().any(|invite| invite.room_id == base_only_invite_id.as_str())
        )
    }));
}

#[test]
fn complete_live_range_accepts_committed_responses_without_a_count() {
    assert!(room_list_range_is_complete(Some(0), 0));
    assert!(room_list_range_is_complete(Some(250), 250));
    assert!(room_list_range_is_complete(None, 0));
    assert!(room_list_range_is_complete(None, 1));
    assert!(!room_list_range_is_complete(Some(250), 249));
    assert!(
        !room_list_range_is_complete(Some(250), 251),
        "a stale cache-only room must keep the projection provisional"
    );
}

#[test]
fn duplicate_room_identity_is_rejected_as_non_authoritative() {
    // The 2026-08-06 incident: the accumulator kept 121 entries while one id
    // was duplicated and another displaced, and the malformed projection was
    // still admitted as authoritative because only the length was checked.
    // A joined Space disappeared from the sidebar as a result (#446).
    assert!(
        !room_list_projection_admits_authority(true, 121, 120),
        "a duplicated room identity must never be admitted as authoritative"
    );
    assert!(room_list_projection_admits_authority(true, 121, 121));
    assert!(
        !room_list_projection_admits_authority(false, 121, 121),
        "an incomplete range stays provisional even with unique identities"
    );
    assert!(room_list_projection_admits_authority(true, 0, 0));
}

#[test]
fn dynamic_entries_expand_to_the_sdk_reported_count_without_a_fixed_cap() {
    assert_eq!(additional_room_list_pages(100, Some(0)), 0);
    assert_eq!(additional_room_list_pages(100, Some(100)), 0);
    assert_eq!(additional_room_list_pages(100, Some(101)), 1);
    assert_eq!(additional_room_list_pages(100, Some(4_097)), 40);
    assert_eq!(additional_room_list_pages(100, None), 0);
}

#[test]
fn room_stop_applies_only_to_the_matching_runtime_generation() {
    assert!(room_stop_matches_generation(Some(7), 7));
    assert!(!room_stop_matches_generation(Some(8), 7));
    assert!(!room_stop_matches_generation(None, 7));
}

#[tokio::test]
async fn partial_projection_acknowledges_connectivity_without_authority() {
    let mut reconciliation = LiveRoomListReconciliation::default();
    reconciliation.report_maximum(Some(2));
    reconciliation.report_range_fully_loaded(false);
    let (ready_tx, ready_rx) = oneshot::channel();
    reconciliation.begin(7, 11, ready_tx);

    let (backend_generation, sequence, ready_tx) = reconciliation
        .take_projection_ack()
        .expect("partial projection acknowledgement");
    ready_tx
        .send(RoomListReconcileAck::Projected {
            backend_generation,
            room_generation: 3,
            response_sequence: sequence,
        })
        .map_err(|_| ())
        .expect("readiness receiver");

    assert!(matches!(
        ready_rx.await.expect("projection acknowledgement"),
        RoomListReconcileAck::Projected {
            backend_generation: 7,
            room_generation: 3,
            response_sequence: 11,
        }
    ));
    assert!(reconciliation.has_pending_reconciliation());
    assert!(!reconciliation.is_authoritative(1));

    reconciliation.report_range_fully_loaded(true);
    let (backend_generation, sequence, ready_tx) = reconciliation
        .finish_if_complete(2)
        .expect("later complete range");
    assert_eq!((backend_generation, sequence), (7, 11));
    assert!(ready_tx.is_none());
    assert!(reconciliation.is_authoritative(2));
}

#[tokio::test]
async fn committed_response_becomes_authoritative_only_after_matching_full_range() {
    let mut reconciliation = LiveRoomListReconciliation::default();
    reconciliation.report_maximum(Some(2));
    reconciliation.report_range_fully_loaded(false);
    let (ready_tx, ready_rx) = oneshot::channel();
    reconciliation.begin(7, 11, ready_tx);

    assert!(!reconciliation.is_authoritative(1));
    assert!(reconciliation.finish_if_complete(1).is_none());
    assert!(reconciliation.finish_if_complete(2).is_none());

    reconciliation.report_range_fully_loaded(true);
    let (backend_generation, sequence, ready_tx) = reconciliation
        .finish_if_complete(2)
        .expect("matching complete range");
    assert_eq!(backend_generation, 7);
    assert_eq!(sequence, 11);
    let ready_tx = ready_tx.expect("complete range acknowledgement");
    ready_tx
        .send(RoomListReconcileAck::Reconciled {
            backend_generation,
            room_generation: 3,
            response_sequence: sequence,
        })
        .map_err(|_| ())
        .expect("readiness receiver");
    let ack = ready_rx.await.expect("readiness ack");
    assert!(matches!(
        ack,
        RoomListReconcileAck::Reconciled {
            backend_generation: 7,
            room_generation: 3,
            response_sequence: 11,
        }
    ));
    assert!(reconciliation.is_authoritative(2));

    reconciliation.report_maximum(Some(3));
    assert!(
        !reconciliation.is_authoritative(2),
        "a newly growing range returns to provisional until complete"
    );
}

#[tokio::test]
async fn committed_response_without_count_stays_authoritative_as_entries_change() {
    let mut reconciliation = LiveRoomListReconciliation::default();
    reconciliation.report_maximum(None);
    reconciliation.report_range_fully_loaded(false);
    let (ready_tx, ready_rx) = oneshot::channel();
    reconciliation.begin(7, 11, ready_tx);

    let (backend_generation, sequence, ready_tx) = reconciliation
        .finish_if_complete(2)
        .expect("missing count must not block the committed projection");
    assert_eq!((backend_generation, sequence), (7, 11));
    assert!(reconciliation.is_authoritative(2));
    let ready_tx = ready_tx.expect("complete range acknowledgement");
    ready_tx
        .send(RoomListReconcileAck::Reconciled {
            backend_generation,
            room_generation: 3,
            response_sequence: sequence,
        })
        .map_err(|_| ())
        .expect("readiness receiver");
    assert!(matches!(
        ready_rx.await.expect("readiness ack"),
        RoomListReconcileAck::Reconciled {
            backend_generation: 7,
            room_generation: 3,
            response_sequence: 11,
        }
    ));
}

#[tokio::test]
async fn project_room_list_snapshot_updates_known_rooms_before_action_delivery() {
    let (action_tx, action_rx) = mpsc::channel(1);
    drop(action_rx);
    let (event_tx, _event_rx) = broadcast::channel(16);
    let known_room_ids = Arc::new(RwLock::new(BTreeSet::new()));
    let snapshot = MatrixRoomListSnapshot {
        rooms: vec![MatrixRoomListRoom {
            room_id: "!room:example.test".to_owned(),
            display_name: "Private room".to_owned(),
            avatar_mxc_uri: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: MatrixRoomTags::default(),
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            marked_unread: false,
            recency_stamp: None,
            conversation_activity: None,
            latest_event: None,
            parent_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 0,
        }],
        ..MatrixRoomListSnapshot::default()
    };

    project_room_list_snapshot(
        &snapshot,
        &known_room_ids,
        &Arc::new(RwLock::new(Vec::new())),
        &action_tx,
        &event_tx,
        1,
        RoomListSource::Live,
        true,
    )
    .await;

    assert!(
        known_room_ids
            .read()
            .expect("known rooms")
            .contains("!room:example.test"),
        "authoritative validators must fail closed before reducer delivery"
    );
}

#[tokio::test]
async fn live_push_rules_reproject_notification_modes_without_room_updates() {
    use koushi_state::RoomNotificationMode;
    use matrix_sdk::{
        ruma::{push::Ruleset, room_id},
        test_utils::mocks::MatrixMockServer,
    };
    use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!push-policy:example.invalid");
    server
        .sync_room(&client, JoinedRoomBuilder::new(room_id))
        .await;
    let updates = client.subscribe_to_all_room_updates();
    let mut harness = spawn_live_observer_test_harness(
        client.clone(),
        server.uri(),
        2,
        updates,
        LiveDirectEventTestSource::SdkAndInjected,
        None,
    )
    .await;
    harness.next_actions("initial rooms").await;
    let muted: Ruleset = serde_json::from_value(serde_json::json!({
        "override": [{"rule_id":room_id,"default":false,"enabled":true,
        "conditions":[{"kind":"event_match","key":"room_id","pattern":room_id}],"actions":[]}]
    }))
    .unwrap();
    for (rules, expected) in [
        (muted, RoomNotificationMode::Mute),
        (Ruleset::new(), RoomNotificationMode::All),
    ] {
        server
            .mock_sync()
            .ok_and_run(&client, |b| {
                b.add_global_account_data(EventFactory::new().push_rules(rules.clone()));
            })
            .await;
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let actions = harness.action_rx.recv().await.expect("observer running");
                if actions.iter().any(|a| matches!(a, AppAction::RoomNotificationModesObserved { modes, .. } if modes.get(room_id.as_str()) == Some(&expected))) {
                    break;
                }
            }
        }).await.expect("push policy update projected without a room diff");
    }
    harness.stop().await;
}
