use futures_util::StreamExt;
use koushi_sdk::{
    MatrixRoomListRoom, MatrixRoomListSnapshot, MatrixRoomListSpace, MatrixRoomTags,
    MatrixSearchCandidate, MatrixTimelineItem,
};
use koushi_state::{AuthSecret, LoginRequest, RecoveryRequest};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
};

const MATRIX_VERSIONS_RESPONSE: &str = r#"{"versions":["r0.6.0","v1.1","v1.2","v1.3","v1.4","v1.5","v1.6","v1.7"],"unstable_features":{}}"#;

#[test]
fn room_list_smoke_report_counts_without_private_names() {
    let snapshot = MatrixRoomListSnapshot {
        spaces: vec![MatrixRoomListSpace {
            space_id: "!space:example.invalid".into(),
            display_name: "Private Space Name".into(),
            avatar_mxc_uri: None,
            child_room_ids: Vec::new(),
            member_user_ids: Vec::new(),
        }],
        rooms: vec![
            MatrixRoomListRoom {
                room_id: "!room-a:example.invalid".into(),
                display_name: "Private Room Name".into(),
                avatar_mxc_uri: None,
                is_dm: false,
                dm_user_ids: Vec::new(),
                tags: MatrixRoomTags::default(),
                unread_count: 2,
                notification_count: 2,
                highlight_count: 0,
                marked_unread: false,
                recency_stamp: None,
                conversation_activity: None,
                latest_event: None,
                parent_space_ids: Vec::new(),
                is_encrypted: false,
                joined_members: 0,
            },
            MatrixRoomListRoom {
                room_id: "!room-b:example.invalid".into(),
                display_name: "Private DM Name".into(),
                avatar_mxc_uri: None,
                is_dm: true,
                dm_user_ids: vec!["@private-dm:example.invalid".into()],
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
            },
        ],
        ..MatrixRoomListSnapshot::default()
    };

    let report = koushi_sdk::room_list_smoke_report(&snapshot);

    assert_eq!(report.rooms, 2);
    assert_eq!(report.spaces, 1);
    assert_eq!(report.dms, 1);
    assert_eq!(report.unread_rooms, 1);
    assert_eq!(report.to_string(), "rooms=2 spaces=1 dms=1 unread_rooms=1");
    assert!(!report.to_string().contains("Private"));
}

#[test]
fn real_account_qa_report_counts_without_private_timeline_data() {
    let snapshot = MatrixRoomListSnapshot {
        spaces: vec![MatrixRoomListSpace {
            space_id: "!space:example.invalid".into(),
            display_name: "Private Space Name".into(),
            avatar_mxc_uri: None,
            child_room_ids: Vec::new(),
            member_user_ids: Vec::new(),
        }],
        rooms: vec![MatrixRoomListRoom {
            room_id: "!room:example.invalid".into(),
            display_name: "Private Room Name".into(),
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
    let timeline_items = vec![MatrixTimelineItem {
        room_id: "!room:example.invalid".into(),
        event_id: "$event:example.invalid".into(),
        sender: "@private:example.invalid".into(),
        timestamp_ms: 1_820_000_000_000,
        body: "Private visible message body".into(),
    }];

    let report = koushi_sdk::real_account_qa_report(&snapshot, true, &timeline_items);
    let rendered = report.to_string();

    assert_eq!(report.room_list.rooms, 1);
    assert_eq!(report.timeline.timeline_items, 1);
    assert!(report.timeline.selected_room_present);
    assert_eq!(
        rendered,
        "rooms=1 spaces=1 dms=0 unread_rooms=0 selected_room_present=true timeline_items=1 session_restored=false search_invoked=false search_candidates=0"
    );
    assert!(!rendered.contains("Private"));
    assert!(!rendered.contains("example.invalid"));
}

#[test]
fn restored_real_account_qa_report_records_restore_without_private_data() {
    let snapshot = MatrixRoomListSnapshot {
        spaces: Vec::new(),
        rooms: vec![MatrixRoomListRoom {
            room_id: "!room:example.invalid".into(),
            display_name: "Private Room Name".into(),
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
    let timeline_items = vec![MatrixTimelineItem {
        room_id: "!room:example.invalid".into(),
        event_id: "$event:example.invalid".into(),
        sender: "@private:example.invalid".into(),
        timestamp_ms: 1_820_000_000_000,
        body: "Private visible message body".into(),
    }];

    let report = koushi_sdk::restored_real_account_qa_report(&snapshot, true, &timeline_items);
    let rendered = report.to_string();

    assert!(report.session_restored);
    assert_eq!(
        rendered,
        "rooms=1 spaces=0 dms=0 unread_rooms=0 selected_room_present=true timeline_items=1 session_restored=true search_invoked=false search_candidates=0"
    );
    assert!(!rendered.contains("Private"));
    assert!(!rendered.contains("example.invalid"));
}

#[test]
fn real_account_qa_report_records_search_without_private_candidate_ids() {
    let snapshot = MatrixRoomListSnapshot {
        spaces: Vec::new(),
        rooms: vec![MatrixRoomListRoom {
            room_id: "!room:example.invalid".into(),
            display_name: "Private Room Name".into(),
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
    let timeline_items = vec![MatrixTimelineItem {
        room_id: "!room:example.invalid".into(),
        event_id: "$timeline-event:example.invalid".into(),
        sender: "@private:example.invalid".into(),
        timestamp_ms: 1_820_000_000_000,
        body: "Private visible message body".into(),
    }];
    let search_candidates = vec![MatrixSearchCandidate {
        room_id: "!private-search-room:example.invalid".into(),
        event_id: "$private-search-event:example.invalid".into(),
        score_millis: 900,
    }];

    let report = koushi_sdk::real_account_qa_report_with_search(
        &snapshot,
        true,
        &timeline_items,
        true,
        &search_candidates,
    );
    let rendered = report.to_string();

    assert!(report.search.invoked);
    assert_eq!(report.search.candidates, 1);
    assert_eq!(
        rendered,
        "rooms=1 spaces=0 dms=0 unread_rooms=0 selected_room_present=true timeline_items=1 session_restored=true search_invoked=true search_candidates=1"
    );
    assert!(!rendered.contains("Private"));
    assert!(!rendered.contains("private-search"));
    assert!(!rendered.contains("example.invalid"));
}

#[test]
fn sdk_password_login_returns_session_info_without_exposing_secret() {
    let homeserver = spawn_password_login_server(200);
    let request = LoginRequest {
        homeserver: homeserver.clone(),
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };

    let session =
        koushi_sdk::login_with_password_blocking(&request).expect("password login should succeed");

    assert_eq!(session.info.homeserver, homeserver);
    assert_eq!(session.info.user_id, "@fixture-user:example.invalid");
    assert_eq!(session.info.device_id, "FIXTUREDEVICE");
    assert!(!format!("{session:?}").contains("synthetic-password"));
}

#[test]
fn sdk_password_login_failure_does_not_include_secret() {
    let homeserver = spawn_password_login_server(403);
    let request = LoginRequest {
        homeserver,
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };

    let error =
        koushi_sdk::login_with_password_blocking(&request).expect_err("password login should fail");

    assert!(!error.to_string().contains("synthetic-password"));
}

#[test]
fn encrypted_store_config_debug_redacts_raw_key() {
    let store_config = koushi_sdk::MatrixClientStoreConfig::new(
        "/tmp/matrix-desktop-test-store",
        koushi_sdk::MatrixClientStoreKey::new([7; 32]),
    )
    .with_cache_path("/tmp/matrix-desktop-test-cache")
    .with_search_index_store(koushi_sdk::MatrixSearchIndexStoreConfig::new(
        "/tmp/matrix-desktop-test-search",
        koushi_sdk::MatrixSearchIndexKey::new("synthetic-search-index-secret"),
    ));

    let debug = format!("{store_config:?}");

    assert!(debug.contains("MatrixClientStoreKey(..)"));
    assert!(debug.contains("MatrixSearchIndexKey(..)"));
    assert!(!debug.contains("7, 7"));
    assert!(!debug.contains("synthetic-search-index-secret"));
}

#[test]
fn sdk_password_login_can_use_encrypted_sqlite_store_without_exposing_store_key() {
    let homeserver = spawn_password_login_server(200);
    let store_dir = tempfile::tempdir().expect("store tempdir should be created");
    let cache_dir = tempfile::tempdir().expect("cache tempdir should be created");
    let search_dir = tempfile::tempdir().expect("search tempdir should be created");
    let store_config = koushi_sdk::MatrixClientStoreConfig::new(
        store_dir.path(),
        koushi_sdk::MatrixClientStoreKey::new([11; 32]),
    )
    .with_cache_path(cache_dir.path())
    .with_search_index_store(koushi_sdk::MatrixSearchIndexStoreConfig::new(
        search_dir.path(),
        koushi_sdk::MatrixSearchIndexKey::new("synthetic-search-index-secret"),
    ));
    let request = LoginRequest {
        homeserver: homeserver.clone(),
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let session = koushi_sdk::login_with_password_with_store(&request, Some(&store_config))
            .await
            .expect("password login with encrypted store should succeed");

        assert_eq!(session.info.homeserver, homeserver);
        assert_eq!(session.info.user_id, "@fixture-user:example.invalid");
        assert_eq!(session.info.device_id, "FIXTUREDEVICE");
        assert!(store_dir.path().read_dir().unwrap().next().is_some());
        assert!(!format!("{session:?}").contains("synthetic-password"));
    });
}

#[test]
fn encrypted_sqlite_store_rejects_wrong_key_without_exposing_key_material() {
    let homeserver = spawn_password_login_server(200);
    let store_dir = tempfile::tempdir().expect("store tempdir should be created");
    let cache_dir = tempfile::tempdir().expect("cache tempdir should be created");
    let search_dir = tempfile::tempdir().expect("search tempdir should be created");
    let correct_store_config = koushi_sdk::MatrixClientStoreConfig::new(
        store_dir.path(),
        koushi_sdk::MatrixClientStoreKey::new([11; 32]),
    )
    .with_cache_path(cache_dir.path())
    .with_search_index_store(koushi_sdk::MatrixSearchIndexStoreConfig::new(
        search_dir.path(),
        koushi_sdk::MatrixSearchIndexKey::new("synthetic-search-index-secret"),
    ));
    let wrong_store_config = koushi_sdk::MatrixClientStoreConfig::new(
        store_dir.path(),
        koushi_sdk::MatrixClientStoreKey::new([12; 32]),
    )
    .with_cache_path(cache_dir.path())
    .with_search_index_store(koushi_sdk::MatrixSearchIndexStoreConfig::new(
        search_dir.path(),
        koushi_sdk::MatrixSearchIndexKey::new("synthetic-search-index-secret"),
    ));
    let request = LoginRequest {
        homeserver,
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let session =
            koushi_sdk::login_with_password_with_store(&request, Some(&correct_store_config))
                .await
                .expect("password login with encrypted store should succeed");
        let persistable = session
            .persistable_session()
            .expect("SDK should expose session data");
        drop(session);

        let error = koushi_sdk::restore_session_with_store(&persistable, Some(&wrong_store_config))
            .await
            .expect_err("encrypted store should reject the wrong key");

        assert!(!error.to_string().contains("12, 12"));
        assert!(!format!("{error:?}").contains("12, 12"));
        assert!(!error.to_string().contains("synthetic-password"));
        assert!(!format!("{error:?}").contains("synthetic-password"));
    });
}

#[test]
fn sdk_password_login_session_can_logout_without_exposing_secret() {
    let logout_seen = Arc::new(AtomicBool::new(false));
    let homeserver = spawn_password_login_server_with_logout(200, Arc::clone(&logout_seen));
    let request = LoginRequest {
        homeserver,
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };
    let session =
        koushi_sdk::login_with_password_blocking(&request).expect("password login should succeed");

    koushi_sdk::logout_blocking(&session).expect("logout should succeed");

    assert!(logout_seen.load(Ordering::SeqCst));
    assert!(!format!("{session:?}").contains("synthetic-password"));
}

#[test]
fn sdk_password_login_exports_redacted_persistable_session() {
    let homeserver = spawn_password_login_server(200);
    let request = LoginRequest {
        homeserver: homeserver.clone(),
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };
    let session =
        koushi_sdk::login_with_password_blocking(&request).expect("password login should succeed");

    let persisted = session
        .persistable_session()
        .expect("SDK should expose session data");
    let json = persisted
        .to_json()
        .expect("persistable session should serialize");
    let restored = koushi_sdk::PersistableMatrixSession::from_json(&json)
        .expect("persistable session should deserialize");

    assert_eq!(persisted.info, session.info);
    assert_eq!(restored.info, session.info);
    assert!(json.contains("fixture-access-token"));
    assert!(!format!("{persisted:?}").contains("fixture-access-token"));
    assert!(!format!("{restored:?}").contains("fixture-access-token"));
    assert!(!format!("{persisted:?}").contains("synthetic-password"));
}

#[test]
fn persisted_session_restores_sdk_client_without_exposing_token() {
    let logout_seen = Arc::new(AtomicBool::new(false));
    let homeserver = spawn_password_login_server_with_logout(200, Arc::clone(&logout_seen));
    let request = LoginRequest {
        homeserver: homeserver.clone(),
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };
    let session =
        koushi_sdk::login_with_password_blocking(&request).expect("password login should succeed");
    let persisted = session
        .persistable_session()
        .expect("SDK should expose session data");

    let restored =
        koushi_sdk::restore_session_blocking(&persisted).expect("persisted session should restore");
    koushi_sdk::logout_blocking(&restored).expect("restored session should logout");

    assert_eq!(restored.info.homeserver, homeserver);
    assert_eq!(restored.info.user_id, "@fixture-user:example.invalid");
    assert_eq!(restored.info.device_id, "FIXTUREDEVICE");
    assert!(logout_seen.load(Ordering::SeqCst));
    assert!(!format!("{restored:?}").contains("fixture-access-token"));
}

#[test]
fn sdk_sync_once_failure_does_not_include_token_or_password() {
    let homeserver = spawn_password_login_server(200);
    let request = LoginRequest {
        homeserver,
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let session = koushi_sdk::login_with_password(&request)
            .await
            .expect("password login should succeed");
        let error = koushi_sdk::sync_once(&session)
            .await
            .expect_err("fixture server does not provide sync");

        assert!(!error.to_string().contains("fixture-access-token"));
        assert!(!format!("{error:?}").contains("fixture-access-token"));
        assert!(!error.to_string().contains("synthetic-password"));
        assert!(!format!("{error:?}").contains("synthetic-password"));
        assert!(!error.to_string().contains("Unexpected test request"));
        assert!(!format!("{error:?}").contains("Unexpected test request"));
    });
}

#[test]
fn sdk_room_operation_failures_do_not_include_body_ids_or_token() {
    let homeserver = spawn_password_login_server(200);
    let request = LoginRequest {
        homeserver,
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let session = koushi_sdk::login_with_password(&request)
            .await
            .expect("password login should succeed");
        let room_id = "!missing-room:example.invalid";
        let event_id = "$missing-event:example.invalid";
        let transaction_id = "desktop-sensitive-transaction";
        let body = "synthetic-body-secret";
        let forbidden = [
            room_id,
            event_id,
            transaction_id,
            body,
            "fixture-access-token",
            "synthetic-password",
        ];

        let send_error = koushi_sdk::send_text_message(&session, room_id, body, transaction_id)
            .await
            .expect_err("missing room should make SDK send fail");
        assert_error_redacts(&send_error, &forbidden);

        let edit_error = koushi_sdk::edit_text_message(&session, room_id, event_id, body)
            .await
            .expect_err("missing room should make SDK edit fail");
        assert_error_redacts(&edit_error, &forbidden);

        let redact_error = koushi_sdk::redact_message(&session, room_id, event_id)
            .await
            .expect_err("missing room should make SDK redaction fail");
        assert_error_redacts(&redact_error, &forbidden);
    });
}

#[test]
fn sdk_send_forbidden_failure_is_classified_without_private_data() {
    let homeserver = spawn_password_login_server_with_send_forbidden();
    let request = LoginRequest {
        homeserver,
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let session = koushi_sdk::login_with_password(&request)
            .await
            .expect("password login should succeed");
        koushi_sdk::sync_once(&session)
            .await
            .expect("sync with joined room should succeed");
        let room_id = "!joined-room:example.invalid";
        let transaction_id = "desktop-sensitive-transaction";
        let body = "synthetic-body-secret";

        let error = koushi_sdk::send_text_message(&session, room_id, body, transaction_id)
            .await
            .expect_err("forbidden send should fail");

        assert_eq!(
            error.failure_kind(),
            Some(koushi_sdk::MatrixRoomOperationFailureKind::Forbidden)
        );
        assert_error_redacts(
            &error,
            &[
                room_id,
                transaction_id,
                body,
                "fixture-access-token",
                "synthetic-password",
            ],
        );
    });
}

#[test]
fn sdk_room_can_send_text_message_respects_power_levels() {
    let homeserver = spawn_password_login_server_with_sendable_room_sync();
    let request = LoginRequest {
        homeserver,
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let session = koushi_sdk::login_with_password(&request)
            .await
            .expect("password login should succeed");
        koushi_sdk::sync_once(&session)
            .await
            .expect("sync with power levels should succeed");

        let read_only =
            koushi_sdk::room_can_send_text_message(&session, "!readonly-room:example.invalid")
                .await
                .expect("read-only room sendability should be available");
        let sendable =
            koushi_sdk::room_can_send_text_message(&session, "!sendable-room:example.invalid")
                .await
                .expect("sendable room sendability should be available");

        assert!(!read_only);
        assert!(sendable);
    });
}

#[test]
fn sdk_search_candidates_return_empty_without_joined_rooms() {
    let homeserver = spawn_password_login_server(200);
    let request = LoginRequest {
        homeserver,
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let session = koushi_sdk::login_with_password(&request)
            .await
            .expect("password login should succeed");

        let candidates = koushi_sdk::search_message_candidates(&session, "synthetic query", 20)
            .await
            .expect("empty joined room set should search successfully");

        assert!(candidates.is_empty());
    });
}

#[test]
fn sdk_room_list_snapshot_returns_empty_without_synced_rooms() {
    let homeserver = spawn_password_login_server(200);
    let request = LoginRequest {
        homeserver,
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let session = koushi_sdk::login_with_password(&request)
            .await
            .expect("password login should succeed");

        let snapshot = koushi_sdk::room_list_snapshot(&session)
            .await
            .expect("empty joined room set should snapshot successfully");

        assert!(snapshot.spaces.is_empty());
        assert!(snapshot.rooms.is_empty());
        assert!(!format!("{snapshot:?}").contains("fixture-access-token"));
        assert!(!format!("{snapshot:?}").contains("synthetic-password"));
    });
}

#[test]
fn sdk_room_list_snapshot_preserves_synced_parent_spaces() {
    let homeserver = spawn_password_login_server_with_space_sync();
    let request = LoginRequest {
        homeserver,
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let session = koushi_sdk::login_with_password(&request)
            .await
            .expect("password login should succeed");
        koushi_sdk::sync_once(&session)
            .await
            .expect("sync with space relationship should succeed");

        let snapshot = koushi_sdk::room_list_snapshot(&session)
            .await
            .expect("synced rooms should snapshot successfully");

        let space = snapshot
            .spaces
            .iter()
            .find(|space| space.space_id == "!fixture-space:example.invalid")
            .expect("space should be in room list");
        assert_eq!(space.child_room_ids, vec!["!fixture-room:example.invalid"]);
        assert_eq!(
            snapshot
                .spaces
                .iter()
                .map(|space| space.space_id.as_str())
                .collect::<Vec<_>>(),
            vec!["!fixture-space:example.invalid"],
            "snapshot = {snapshot:?}"
        );
        let room = snapshot
            .rooms
            .iter()
            .find(|room| room.room_id == "!fixture-room:example.invalid")
            .expect("regular room should be in room list");
        assert_eq!(room.display_name, "Fixture Room");
        assert_eq!(
            room.parent_space_ids,
            vec!["!fixture-space:example.invalid".to_owned()]
        );
        assert!(!format!("{snapshot:?}").contains("fixture-access-token"));
        assert!(!format!("{snapshot:?}").contains("synthetic-password"));
    });
}

#[test]
fn sdk_room_list_snapshot_excludes_left_rooms() {
    let homeserver = spawn_password_login_server_with_joined_and_left_sync();
    let request = LoginRequest {
        homeserver,
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let session = koushi_sdk::login_with_password(&request)
            .await
            .expect("password login should succeed");
        koushi_sdk::sync_once(&session)
            .await
            .expect("sync with left room should succeed");

        let snapshot = koushi_sdk::room_list_snapshot(&session)
            .await
            .expect("synced room snapshot should succeed");

        assert_eq!(
            snapshot
                .rooms
                .iter()
                .map(|room| room.room_id.as_str())
                .collect::<Vec<_>>(),
            vec!["!joined-room:example.invalid"],
            "snapshot = {snapshot:?}"
        );
    });
}

#[test]
fn sdk_search_candidates_blocking_returns_empty_for_empty_query() {
    let homeserver = spawn_password_login_server(200);
    let request = LoginRequest {
        homeserver,
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };
    let session =
        koushi_sdk::login_with_password_blocking(&request).expect("password login should succeed");

    let candidates = koushi_sdk::search_message_candidates_blocking(&session, "  ", 10)
        .expect("empty search should succeed");

    assert!(candidates.is_empty());
}

#[test]
fn sdk_timeline_subscription_failure_does_not_include_room_id_or_token() {
    let homeserver = spawn_password_login_server(200);
    let request = LoginRequest {
        homeserver,
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let session = koushi_sdk::login_with_password(&request)
            .await
            .expect("password login should succeed");
        let room_id = "!timeline-missing:example.invalid";
        let error = koushi_sdk::subscribe_room_timeline(&session, room_id)
            .await
            .expect_err("missing room should make timeline subscription fail");

        assert_error_redacts(
            &error,
            &[
                room_id,
                "fixture-access-token",
                "synthetic-password",
                "synthetic query",
            ],
        );
    });
}

#[test]
fn sdk_sync_loop_reports_running_and_can_stop_after_callback() {
    let sync_seen = Arc::new(AtomicUsize::new(0));
    let homeserver = spawn_password_login_server_with_sync(Arc::clone(&sync_seen));
    let request = LoginRequest {
        homeserver,
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let session = koushi_sdk::login_with_password(&request)
            .await
            .expect("password login should succeed");
        let callback_count = Arc::new(AtomicUsize::new(0));
        let callback_count_for_loop = Arc::clone(&callback_count);

        koushi_sdk::sync_loop(&session, move || {
            let callback_count = Arc::clone(&callback_count_for_loop);
            async move {
                callback_count.fetch_add(1, Ordering::SeqCst);
                koushi_sdk::MatrixSyncLoopControl::Stop
            }
        })
        .await
        .expect("sync loop should stop cleanly when callback asks");

        assert_eq!(callback_count.load(Ordering::SeqCst), 1);
        assert_eq!(sync_seen.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn restricted_verification_sync_sends_the_restricted_filter_and_processes_top_level_data() {
    let restricted_sync_seen = Arc::new(AtomicBool::new(false));
    let homeserver =
        spawn_password_login_server_with_restricted_sync(Arc::clone(&restricted_sync_seen));
    let request = LoginRequest {
        homeserver,
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let session = koushi_sdk::login_with_password(&request)
            .await
            .expect("password login should succeed");
        koushi_sdk::restricted_verification_sync_once(&session)
            .await
            .expect("restricted sync should process top-level verification data");
        assert!(session.client().rooms().is_empty());
    });
    assert!(
        restricted_sync_seen.load(Ordering::SeqCst),
        "restricted sync request was not observed"
    );
}

#[test]
fn sdk_e2ee_recovery_failure_does_not_include_secret() {
    let homeserver = spawn_password_login_server(200);
    let request = LoginRequest {
        homeserver,
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };
    let session =
        koushi_sdk::login_with_password_blocking(&request).expect("password login should succeed");
    let recovery = RecoveryRequest {
        secret: AuthSecret::new("synthetic-recovery-secret"),
    };

    let error = koushi_sdk::recover_e2ee_blocking(&session, &recovery)
        .expect_err("fixture server does not provide secret storage");

    assert!(!error.to_string().contains("synthetic-recovery-secret"));
    assert!(!format!("{error:?}").contains("synthetic-recovery-secret"));
}

#[test]
fn sdk_e2ee_recovery_state_is_exposed_without_secret_material() {
    let homeserver = spawn_password_login_server(200);
    let request = LoginRequest {
        homeserver,
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };
    let session =
        koushi_sdk::login_with_password_blocking(&request).expect("password login should succeed");

    let state = session.e2ee_recovery_state();

    assert!(
        matches!(
            state,
            koushi_sdk::E2eeRecoveryState::Unknown | koushi_sdk::E2eeRecoveryState::Disabled
        ),
        "expected Unknown or Disabled, got {state:?}"
    );
    assert!(!format!("{state:?}").contains("synthetic-password"));
}

#[test]
fn sdk_e2ee_recovery_state_stream_emits_initial_state_without_secret_material() {
    let homeserver = spawn_password_login_server(200);
    let request = LoginRequest {
        homeserver,
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };
    let session =
        koushi_sdk::login_with_password_blocking(&request).expect("password login should succeed");

    let mut stream = session.e2ee_recovery_state_stream();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");
    let state = runtime
        .block_on(async { stream.next().await })
        .expect("recovery stream should emit the initial state");

    // The initial stream emission races the post-login Unknown -> Disabled
    // backup-state determination; both are valid secret-free initial states.
    // This test's contract is "emits an initial state without secret material",
    // so accept either rather than racing on the exact value (was a CI flake).
    assert!(
        matches!(
            state,
            koushi_sdk::E2eeRecoveryState::Unknown | koushi_sdk::E2eeRecoveryState::Disabled
        ),
        "unexpected initial recovery state: {state:?}"
    );
    assert!(!format!("{state:?}").contains("synthetic-password"));
}

#[test]
fn current_device_trust_subscribes_before_reading_current_value() {
    let homeserver = spawn_password_login_server(200);
    let request = LoginRequest {
        homeserver,
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };
    let session =
        koushi_sdk::login_with_password_blocking(&request).expect("password login should succeed");

    let observation = session.observe_current_device_trust();
    let current = observation.current;
    let mut updates = observation.updates;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");
    let first = runtime
        .block_on(async { updates.next().await })
        .expect("trust stream should emit the subscribed current value");

    assert_eq!(first, current);
    assert!(matches!(
        current,
        koushi_state::CurrentDeviceTrustState::Unknown
            | koushi_state::CurrentDeviceTrustState::Verified
            | koushi_state::CurrentDeviceTrustState::Unverified
    ));
}

fn assert_error_redacts(error: &(impl std::fmt::Display + std::fmt::Debug), forbidden: &[&str]) {
    let display = error.to_string();
    let debug = format!("{error:?}");

    for value in forbidden {
        assert!(
            !display.contains(value),
            "error Display leaked forbidden value {value:?}: {display}"
        );
        assert!(
            !debug.contains(value),
            "error Debug leaked forbidden value {value:?}: {debug}"
        );
    }
}

fn spawn_password_login_server(status: u16) -> String {
    spawn_password_login_server_with_logout(status, Arc::new(AtomicBool::new(false)))
}

fn spawn_password_login_server_with_sync(sync_seen: Arc<AtomicUsize>) -> String {
    spawn_password_login_server_with_options(200, Arc::new(AtomicBool::new(false)), Some(sync_seen))
}

fn spawn_password_login_server_with_restricted_sync(sync_seen: Arc<AtomicBool>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let addr = listener
        .local_addr()
        .expect("test server should have an address");

    thread::spawn(move || {
        for _ in 0..16 {
            let (mut stream, _) = listener
                .accept()
                .expect("test server should accept a request");
            let request = read_http_request(&mut stream);
            if request.starts_with("GET /_matrix/client/versions ") {
                write_json(&mut stream, 200, MATRIX_VERSIONS_RESPONSE);
                continue;
            }
            if write_common_sdk_bootstrap_response(&mut stream, &request) {
                continue;
            }
            if request.starts_with("GET /_matrix/client/") && request.contains("/sync") {
                assert!(
                    request.contains("timeout=3000"),
                    "restricted sync server wait was not bounded: {request}"
                );
                assert!(
                    request.contains("%22presence%22") && request.contains("%22room%22"),
                    "restricted sync omitted its inline filter: {request}"
                );
                assert!(
                    request.contains("%22types%22%3A%5B%5D")
                        && request.contains("%22rooms%22%3A%5B%5D"),
                    "restricted sync did not suppress presence and rooms: {request}"
                );
                sync_seen.store(true, Ordering::SeqCst);
                write_json(
                    &mut stream,
                    200,
                    r#"{
                        "device_one_time_keys_count": {},
                        "next_batch": "restricted-batch",
                        "device_lists": {"changed": [], "left": []},
                        "rooms": {"invite": {}, "join": {}, "leave": {}, "knock": {}},
                        "to_device": {"events": []},
                        "presence": {"events": []},
                        "account_data": {"events": [{"type":"m.direct","content":{}}]}
                    }"#,
                );
                return;
            }
            if request.starts_with("POST /_matrix/client/")
                && request.contains("fixture-user")
                && request.contains("synthetic-password")
                && request.contains("Matrix Desktop Test")
            {
                write_json(
                    &mut stream,
                    200,
                    r#"{"access_token":"fixture-access-token","device_id":"FIXTUREDEVICE","user_id":"@fixture-user:example.invalid"}"#,
                );
                continue;
            }
            write_json(
                &mut stream,
                404,
                r#"{"errcode":"M_NOT_FOUND","error":"Unexpected test request"}"#,
            );
        }
    });
    format!("http://{addr}")
}

fn spawn_password_login_server_with_space_sync() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let addr = listener
        .local_addr()
        .expect("test server should have an address");

    thread::spawn(move || {
        for _ in 0..16 {
            let (mut stream, _) = listener
                .accept()
                .expect("test server should accept a request");
            let request = read_http_request(&mut stream);

            if request.starts_with("GET /_matrix/client/versions ") {
                write_json(&mut stream, 200, MATRIX_VERSIONS_RESPONSE);
                continue;
            }

            if write_common_sdk_bootstrap_response(&mut stream, &request) {
                continue;
            }

            if request.starts_with("GET /_matrix/client/") && request.contains("/sync") {
                write_json(&mut stream, 200, space_relationship_sync_response());
                continue;
            }

            if request.starts_with("POST /_matrix/client/")
                && request.contains("fixture-user")
                && request.contains("synthetic-password")
                && request.contains("Matrix Desktop Test")
            {
                write_json(
                    &mut stream,
                    200,
                    r#"{"access_token":"fixture-access-token","device_id":"FIXTUREDEVICE","user_id":"@fixture-user:example.invalid"}"#,
                );
                continue;
            }

            write_json(
                &mut stream,
                404,
                r#"{"errcode":"M_NOT_FOUND","error":"Unexpected test request"}"#,
            );
        }
    });

    format!("http://{addr}")
}

fn spawn_password_login_server_with_joined_and_left_sync() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let addr = listener
        .local_addr()
        .expect("test server should have an address");

    thread::spawn(move || {
        for _ in 0..16 {
            let (mut stream, _) = listener
                .accept()
                .expect("test server should accept a request");
            let request = read_http_request(&mut stream);

            if request.starts_with("GET /_matrix/client/versions ") {
                write_json(&mut stream, 200, MATRIX_VERSIONS_RESPONSE);
                continue;
            }

            if write_common_sdk_bootstrap_response(&mut stream, &request) {
                continue;
            }

            if request.starts_with("GET /_matrix/client/") && request.contains("/sync") {
                write_json(&mut stream, 200, joined_and_left_sync_response());
                continue;
            }

            if request.starts_with("POST /_matrix/client/")
                && request.contains("fixture-user")
                && request.contains("synthetic-password")
                && request.contains("Matrix Desktop Test")
            {
                write_json(
                    &mut stream,
                    200,
                    r#"{"access_token":"fixture-access-token","device_id":"FIXTUREDEVICE","user_id":"@fixture-user:example.invalid"}"#,
                );
                continue;
            }

            write_json(
                &mut stream,
                404,
                r#"{"errcode":"M_NOT_FOUND","error":"Unexpected test request"}"#,
            );
        }
    });

    format!("http://{addr}")
}

fn spawn_password_login_server_with_send_forbidden() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let addr = listener
        .local_addr()
        .expect("test server should have an address");

    thread::spawn(move || {
        for _ in 0..16 {
            let (mut stream, _) = listener
                .accept()
                .expect("test server should accept a request");
            let request = read_http_request(&mut stream);

            if request.starts_with("GET /_matrix/client/versions ") {
                write_json(&mut stream, 200, MATRIX_VERSIONS_RESPONSE);
                continue;
            }

            if write_common_sdk_bootstrap_response(&mut stream, &request) {
                continue;
            }

            if request.starts_with("GET /_matrix/client/") && request.contains("/sync") {
                write_json(&mut stream, 200, joined_room_sync_response());
                continue;
            }

            if request.starts_with("GET /_matrix/client/")
                && request.contains("/state/m.room.encryption/")
            {
                write_json(
                    &mut stream,
                    404,
                    r#"{"errcode":"M_NOT_FOUND","error":"No encryption state"}"#,
                );
                continue;
            }

            if request.starts_with("PUT /_matrix/client/")
                && request.contains("/send/m.room.message/")
            {
                write_json(
                    &mut stream,
                    403,
                    r#"{"errcode":"M_FORBIDDEN","error":"Forbidden"}"#,
                );
                continue;
            }

            if request.starts_with("POST /_matrix/client/")
                && request.contains("fixture-user")
                && request.contains("synthetic-password")
                && request.contains("Matrix Desktop Test")
            {
                write_json(
                    &mut stream,
                    200,
                    r#"{"access_token":"fixture-access-token","device_id":"FIXTUREDEVICE","user_id":"@fixture-user:example.invalid"}"#,
                );
                continue;
            }

            write_json(
                &mut stream,
                404,
                r#"{"errcode":"M_NOT_FOUND","error":"Unexpected test request"}"#,
            );
        }
    });

    format!("http://{addr}")
}

fn spawn_password_login_server_with_sendable_room_sync() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let addr = listener
        .local_addr()
        .expect("test server should have an address");

    thread::spawn(move || {
        for _ in 0..10 {
            let (mut stream, _) = listener
                .accept()
                .expect("test server should accept a request");
            let request = read_http_request(&mut stream);

            if request.starts_with("GET /_matrix/client/versions ") {
                write_json(&mut stream, 200, MATRIX_VERSIONS_RESPONSE);
                continue;
            }

            if write_common_sdk_bootstrap_response(&mut stream, &request) {
                continue;
            }

            if request.starts_with("GET /_matrix/client/") && request.contains("/sync") {
                write_json(&mut stream, 200, sendable_room_sync_response());
                continue;
            }

            if request.starts_with("POST /_matrix/client/")
                && request.contains("fixture-user")
                && request.contains("synthetic-password")
                && request.contains("Matrix Desktop Test")
            {
                write_json(
                    &mut stream,
                    200,
                    r#"{"access_token":"fixture-access-token","device_id":"FIXTUREDEVICE","user_id":"@fixture-user:example.invalid"}"#,
                );
                continue;
            }

            write_json(
                &mut stream,
                404,
                r#"{"errcode":"M_NOT_FOUND","error":"Unexpected test request"}"#,
            );
        }
    });

    format!("http://{addr}")
}

fn spawn_password_login_server_with_logout(status: u16, logout_seen: Arc<AtomicBool>) -> String {
    spawn_password_login_server_with_options(status, logout_seen, None)
}

fn spawn_password_login_server_with_options(
    status: u16,
    logout_seen: Arc<AtomicBool>,
    sync_seen: Option<Arc<AtomicUsize>>,
) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let addr = listener
        .local_addr()
        .expect("test server should have an address");

    thread::spawn(move || {
        for _ in 0..16 {
            let (mut stream, _) = listener
                .accept()
                .expect("test server should accept a request");
            let request = read_http_request(&mut stream);

            if request.starts_with("GET /_matrix/client/versions ") {
                write_json(&mut stream, 200, MATRIX_VERSIONS_RESPONSE);
                continue;
            }

            if write_common_sdk_bootstrap_response(&mut stream, &request) {
                continue;
            }

            if request.starts_with("POST /_matrix/client/") && request.contains("/logout") {
                logout_seen.store(true, Ordering::SeqCst);
                write_json(&mut stream, 200, r#"{}"#);
                return;
            }

            if request.starts_with("GET /_matrix/client/") && request.contains("/sync") {
                if let Some(sync_seen) = &sync_seen {
                    let sync_count = sync_seen.fetch_add(1, Ordering::SeqCst) + 1;
                    write_json(
                        &mut stream,
                        200,
                        &format!(
                            r#"{{
                                "device_one_time_keys_count": {{}},
                                "next_batch": "sync-batch-{sync_count}",
                                "device_lists": {{"changed": [], "left": []}},
                                "rooms": {{"invite": {{}}, "join": {{}}, "leave": {{}}, "knock": {{}}}},
                                "to_device": {{"events": []}},
                                "presence": {{"events": []}},
                                "account_data": {{"events": []}}
                            }}"#
                        ),
                    );
                    continue;
                }
            }

            if request.starts_with("POST /_matrix/client/")
                && request.contains("fixture-user")
                && request.contains("synthetic-password")
                && request.contains("Matrix Desktop Test")
            {
                if status == 200 {
                    write_json(
                        &mut stream,
                        200,
                        r#"{"access_token":"fixture-access-token","device_id":"FIXTUREDEVICE","user_id":"@fixture-user:example.invalid"}"#,
                    );
                    continue;
                } else {
                    write_json(
                        &mut stream,
                        status,
                        r#"{"errcode":"M_FORBIDDEN","error":"Invalid credentials"}"#,
                    );
                    return;
                }
            }

            write_json(
                &mut stream,
                404,
                r#"{"errcode":"M_NOT_FOUND","error":"Unexpected test request"}"#,
            );
        }
    });

    format!("http://{addr}")
}

fn read_http_request(stream: &mut std::net::TcpStream) -> String {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];

    loop {
        let bytes_read = stream
            .read(&mut buffer)
            .expect("test server should read request");
        if bytes_read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..bytes_read]);

        let request_text = String::from_utf8_lossy(&request);
        let Some(header_end) = request_text.find("\r\n\r\n") else {
            continue;
        };
        let content_length = request_text
            .lines()
            .find_map(|line| {
                line.strip_prefix("Content-Length: ")
                    .and_then(|value| value.parse::<usize>().ok())
            })
            .unwrap_or(0);
        if request.len() >= header_end + 4 + content_length {
            break;
        }
    }

    String::from_utf8(request).expect("test request should be UTF-8")
}

fn write_common_sdk_bootstrap_response(stream: &mut std::net::TcpStream, request: &str) -> bool {
    if request.starts_with("GET /_matrix/client/")
        && request.contains("/account_data/m.secret_storage.default_key")
    {
        write_json(
            stream,
            404,
            r#"{"errcode":"M_NOT_FOUND","error":"No default secret storage key"}"#,
        );
        return true;
    }

    if request.starts_with("POST /_matrix/client/") && request.contains("/keys/upload") {
        write_json(stream, 200, r#"{"one_time_key_counts":{}}"#);
        return true;
    }

    if request.starts_with("POST /_matrix/client/") && request.contains("/keys/query") {
        write_json(stream, 200, r#"{"device_keys":{},"failures":{}}"#);
        return true;
    }

    false
}

fn space_relationship_sync_response() -> &'static str {
    r#"{
        "device_one_time_keys_count": {},
        "next_batch": "sync-batch-space-1",
        "device_lists": {"changed": [], "left": []},
        "rooms": {
            "invite": {},
            "join": {
                "!fixture-space:example.invalid": {
                    "summary": {},
                    "state": {
                        "events": [
                            {
                                "content": {"room_version": "10", "type": "m.space"},
                                "event_id": "$fixture-space-create",
                                "origin_server_ts": 1,
                                "sender": "@fixture-user:example.invalid",
                                "state_key": "",
                                "type": "m.room.create"
                            },
                            {
                                "content": {"name": "Fixture Space"},
                                "event_id": "$fixture-space-name",
                                "origin_server_ts": 2,
                                "sender": "@fixture-user:example.invalid",
                                "state_key": "",
                                "type": "m.room.name"
                            },
                            {
                                "content": {"via": ["example.invalid"]},
                                "event_id": "$fixture-space-child",
                                "origin_server_ts": 3,
                                "sender": "@fixture-user:example.invalid",
                                "state_key": "!fixture-room:example.invalid",
                                "type": "m.space.child"
                            }
                        ]
                    },
                    "timeline": {"events": [], "limited": false, "prev_batch": "space-prev"},
                    "ephemeral": {"events": []},
                    "account_data": {"events": []},
                    "unread_notifications": {"highlight_count": 0, "notification_count": 0}
                },
                "!fixture-room:example.invalid": {
                    "summary": {},
                    "state": {
                        "events": [
                            {
                                "content": {"room_version": "10"},
                                "event_id": "$fixture-room-create",
                                "origin_server_ts": 4,
                                "sender": "@fixture-user:example.invalid",
                                "state_key": "",
                                "type": "m.room.create"
                            },
                            {
                                "content": {"name": "Fixture Room"},
                                "event_id": "$fixture-room-name",
                                "origin_server_ts": 5,
                                "sender": "@fixture-user:example.invalid",
                                "state_key": "",
                                "type": "m.room.name"
                            },
                            {
                                "content": {"canonical": true, "via": ["example.invalid"]},
                                "event_id": "$fixture-room-parent",
                                "origin_server_ts": 6,
                                "sender": "@fixture-user:example.invalid",
                                "state_key": "!fixture-space:example.invalid",
                                "type": "m.space.parent"
                            }
                        ]
                    },
                    "timeline": {"events": [], "limited": false, "prev_batch": "room-prev"},
                    "ephemeral": {"events": []},
                    "account_data": {"events": []},
                    "unread_notifications": {"highlight_count": 0, "notification_count": 0}
                }
            },
            "leave": {},
            "knock": {}
        },
        "to_device": {"events": []},
        "presence": {"events": []},
        "account_data": {"events": []}
    }"#
}

fn joined_and_left_sync_response() -> &'static str {
    r#"{
        "device_one_time_keys_count": {},
        "next_batch": "sync-batch-left-1",
        "device_lists": {"changed": [], "left": []},
        "rooms": {
            "invite": {},
            "join": {
                "!joined-room:example.invalid": {
                    "summary": {},
                    "state": {
                        "events": [
                            {
                                "content": {"room_version": "10"},
                                "event_id": "$joined-room-create",
                                "origin_server_ts": 1,
                                "sender": "@fixture-user:example.invalid",
                                "state_key": "",
                                "type": "m.room.create"
                            },
                            {
                                "content": {"name": "Joined Fixture Room"},
                                "event_id": "$joined-room-name",
                                "origin_server_ts": 2,
                                "sender": "@fixture-user:example.invalid",
                                "state_key": "",
                                "type": "m.room.name"
                            }
                        ]
                    },
                    "timeline": {"events": [], "limited": false, "prev_batch": "joined-prev"},
                    "ephemeral": {"events": []},
                    "account_data": {"events": []},
                    "unread_notifications": {"highlight_count": 0, "notification_count": 0}
                }
            },
            "leave": {
                "!left-room:example.invalid": {
                    "state": {
                        "events": [
                            {
                                "content": {"room_version": "10"},
                                "event_id": "$left-room-create",
                                "origin_server_ts": 3,
                                "sender": "@fixture-user:example.invalid",
                                "state_key": "",
                                "type": "m.room.create"
                            },
                            {
                                "content": {"name": "Left Fixture Room"},
                                "event_id": "$left-room-name",
                                "origin_server_ts": 4,
                                "sender": "@fixture-user:example.invalid",
                                "state_key": "",
                                "type": "m.room.name"
                            }
                        ]
                    },
                    "timeline": {"events": [], "limited": false, "prev_batch": "left-prev"},
                    "account_data": {"events": []}
                }
            },
            "knock": {}
        },
        "to_device": {"events": []},
        "presence": {"events": []},
        "account_data": {"events": []}
    }"#
}

fn joined_room_sync_response() -> &'static str {
    r#"{
        "device_one_time_keys_count": {},
        "next_batch": "sync-batch-joined-1",
        "device_lists": {"changed": [], "left": []},
        "rooms": {
            "invite": {},
            "join": {
                "!joined-room:example.invalid": {
                    "summary": {},
                    "state": {
                        "events": [
                            {
                                "content": {"room_version": "10"},
                                "event_id": "$joined-room-create",
                                "origin_server_ts": 1,
                                "sender": "@fixture-user:example.invalid",
                                "state_key": "",
                                "type": "m.room.create"
                            },
                            {
                                "content": {"name": "Joined Fixture Room"},
                                "event_id": "$joined-room-name",
                                "origin_server_ts": 2,
                                "sender": "@fixture-user:example.invalid",
                                "state_key": "",
                                "type": "m.room.name"
                            }
                        ]
                    },
                    "timeline": {"events": [], "limited": false, "prev_batch": "joined-prev"},
                    "ephemeral": {"events": []},
                    "account_data": {"events": []},
                    "unread_notifications": {"highlight_count": 0, "notification_count": 0}
                }
            },
            "leave": {},
            "knock": {}
        },
        "to_device": {"events": []},
        "presence": {"events": []},
        "account_data": {"events": []}
    }"#
}

fn sendable_room_sync_response() -> &'static str {
    r#"{
        "device_one_time_keys_count": {},
        "next_batch": "sync-batch-sendable-1",
        "device_lists": {"changed": [], "left": []},
        "rooms": {
            "invite": {},
            "join": {
                "!readonly-room:example.invalid": {
                    "summary": {},
                    "state": {
                        "events": [
                            {
                                "content": {"creator": "@fixture-user:example.invalid", "room_version": "10"},
                                "event_id": "$readonly-create",
                                "origin_server_ts": 1,
                                "sender": "@fixture-user:example.invalid",
                                "state_key": "",
                                "type": "m.room.create"
                            },
                            {
                                "content": {"users": {"@fixture-user:example.invalid": -10}, "users_default": 0, "events_default": 0, "state_default": 50},
                                "event_id": "$readonly-power",
                                "origin_server_ts": 2,
                                "sender": "@fixture-user:example.invalid",
                                "state_key": "",
                                "type": "m.room.power_levels"
                            }
                        ]
                    },
                    "timeline": {"events": [], "limited": false, "prev_batch": "readonly-prev"},
                    "ephemeral": {"events": []},
                    "account_data": {"events": []},
                    "unread_notifications": {"highlight_count": 0, "notification_count": 0}
                },
                "!sendable-room:example.invalid": {
                    "summary": {},
                    "state": {
                        "events": [
                            {
                                "content": {"creator": "@fixture-user:example.invalid", "room_version": "10"},
                                "event_id": "$sendable-create",
                                "origin_server_ts": 3,
                                "sender": "@fixture-user:example.invalid",
                                "state_key": "",
                                "type": "m.room.create"
                            },
                            {
                                "content": {"name": "Sendable Fixture Room"},
                                "event_id": "$sendable-name",
                                "origin_server_ts": 4,
                                "sender": "@fixture-user:example.invalid",
                                "state_key": "",
                                "type": "m.room.name"
                            }
                        ]
                    },
                    "timeline": {"events": [], "limited": false, "prev_batch": "sendable-prev"},
                    "ephemeral": {"events": []},
                    "account_data": {"events": []},
                    "unread_notifications": {"highlight_count": 0, "notification_count": 0}
                }
            },
            "leave": {},
            "knock": {}
        },
        "to_device": {"events": []},
        "presence": {"events": []},
        "account_data": {"events": []}
    }"#
}

fn write_json(stream: &mut std::net::TcpStream, status: u16, body: &str) {
    let response = format!(
        "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .expect("test server should write response");
}
