use super::super::test_support::{file_store_actor, make_key_id};
use super::CoreFailure;
use koushi_state::NavigationState;
use tempfile::tempdir;

#[test]
fn navigation_state_is_encrypted_and_rejects_corruption() {
    let data_dir = tempdir().expect("tempdir");
    let cred_dir = tempdir().expect("tempdir");
    let key_id = make_key_id();
    let actor = file_store_actor(&data_dir, &cred_dir);
    let navigation = NavigationState {
        active_space_id: Some("!space:test.example.com".to_owned()),
        active_room_id: Some("!room:test.example.com".to_owned()),
        home_selection: koushi_state::HomeSelection::DirectMessage {
            room_id: "!remembered-dm:test.example.com".to_owned(),
        },
        space_local_presentations: koushi_state::SpaceLocalPresentations(
            std::collections::BTreeMap::from([(
                "!private-space:test.example.com".to_owned(),
                koushi_state::SpaceLocalPresentation {
                    name: Some("Private local name".to_owned()),
                    icon: Some("🧪".to_owned()),
                },
            )]),
        ),
        legacy_frontend_preferences_imported: true,
        space_order: vec!["!space:test.example.com".to_owned()],
        last_room_by_space_id: std::collections::BTreeMap::from([(
            "!space:test.example.com".to_owned(),
            "!room:test.example.com".to_owned(),
        )]),
        last_selection_by_space_id: std::collections::BTreeMap::from([(
            "!space:test.example.com".to_owned(),
            koushi_state::SpaceNavigationSelection {
                surface: koushi_state::SpaceConversationSurface::Dms,
                room_id: Some("!dm:test.example.com".to_owned()),
            },
        )]),
        room_scroll_anchors: std::collections::BTreeMap::new(),
        main_timeline_anchor: None,
        event_navigation: Default::default(),
    };

    actor
        .save_navigation(&key_id, &navigation)
        .expect("save encrypted navigation");

    let path = actor.account_navigation_file(&key_id);
    let bytes = std::fs::read(&path).expect("read encrypted navigation");
    assert!(!path.with_extension("tmp").exists());
    for plaintext in [
        "!space:test.example.com",
        "!room:test.example.com",
        "!remembered-dm:test.example.com",
        "!private-space:test.example.com",
        "Private local name",
        "🧪",
    ] {
        assert!(
            !bytes
                .windows(plaintext.len())
                .any(|window| window == plaintext.as_bytes())
        );
    }

    let loaded = actor
        .load_navigation(&key_id)
        .expect("load encrypted navigation");
    assert_eq!(loaded, navigation);

    let mut corrupted = bytes;
    let last = corrupted
        .last_mut()
        .expect("non-empty encrypted navigation");
    *last ^= 0x01;
    std::fs::write(&path, corrupted).expect("write corrupted navigation");
    assert!(matches!(
        actor.load_navigation(&key_id),
        Err(CoreFailure::StoreUnavailable)
    ));
}

#[test]
fn legacy_navigation_json_loads_and_next_save_migrates_to_encrypted_file() {
    let data_dir = tempdir().expect("tempdir");
    let cred_dir = tempdir().expect("tempdir");
    let key_id = make_key_id();
    let actor = file_store_actor(&data_dir, &cred_dir);
    let navigation = NavigationState {
        active_space_id: Some("!space:test.example.com".to_owned()),
        active_room_id: Some("!room:test.example.com".to_owned()),
        home_selection: koushi_state::HomeSelection::default(),
        space_local_presentations: koushi_state::SpaceLocalPresentations::default(),
        legacy_frontend_preferences_imported: false,
        space_order: vec!["!space:test.example.com".to_owned()],
        last_room_by_space_id: std::collections::BTreeMap::from([(
            "!space:test.example.com".to_owned(),
            "!room:test.example.com".to_owned(),
        )]),
        last_selection_by_space_id: std::collections::BTreeMap::from([(
            "!space:test.example.com".to_owned(),
            koushi_state::SpaceNavigationSelection {
                surface: koushi_state::SpaceConversationSurface::Dms,
                room_id: Some("!dm:test.example.com".to_owned()),
            },
        )]),
        room_scroll_anchors: std::collections::BTreeMap::new(),
        main_timeline_anchor: None,
        event_navigation: Default::default(),
    };
    let legacy_path = actor.account_navigation_legacy_file(&key_id);
    std::fs::create_dir_all(legacy_path.parent().expect("navigation parent"))
        .expect("create navigation parent");
    std::fs::write(
        &legacy_path,
        serde_json::to_string(&navigation).expect("serialize legacy navigation"),
    )
    .expect("write legacy navigation");

    let loaded = actor
        .load_navigation(&key_id)
        .expect("load legacy navigation");
    assert_eq!(loaded, navigation);

    actor
        .save_navigation(&key_id, &navigation)
        .expect("migrate navigation");
    assert!(!legacy_path.exists());

    let encrypted_path = actor.account_navigation_file(&key_id);
    let bytes = std::fs::read(&encrypted_path).expect("read encrypted navigation");
    for plaintext in ["!space:test.example.com", "!room:test.example.com"] {
        assert!(
            !bytes
                .windows(plaintext.len())
                .any(|window| window == plaintext.as_bytes())
        );
    }
}

#[test]
fn default_navigation_removes_encrypted_and_legacy_files() {
    let data_dir = tempdir().expect("tempdir");
    let cred_dir = tempdir().expect("tempdir");
    let key_id = make_key_id();
    let actor = file_store_actor(&data_dir, &cred_dir);
    let navigation = NavigationState {
        active_space_id: None,
        active_room_id: Some("!room:test.example.com".to_owned()),
        home_selection: koushi_state::HomeSelection::default(),
        space_local_presentations: koushi_state::SpaceLocalPresentations::default(),
        legacy_frontend_preferences_imported: false,
        space_order: Vec::new(),
        last_room_by_space_id: std::collections::BTreeMap::new(),
        last_selection_by_space_id: std::collections::BTreeMap::new(),
        room_scroll_anchors: std::collections::BTreeMap::new(),
        main_timeline_anchor: None,
        event_navigation: Default::default(),
    };

    actor
        .save_navigation(&key_id, &navigation)
        .expect("save encrypted navigation");
    let encrypted_path = actor.account_navigation_file(&key_id);
    assert!(encrypted_path.exists());

    let legacy_path = actor.account_navigation_legacy_file(&key_id);
    std::fs::create_dir_all(legacy_path.parent().expect("navigation parent"))
        .expect("create navigation parent");
    std::fs::write(&legacy_path, "{}").expect("write legacy navigation");
    assert!(legacy_path.exists());

    actor
        .save_navigation(&key_id, &NavigationState::default())
        .expect("clear navigation");
    assert!(!encrypted_path.exists());
    assert!(!legacy_path.exists());
    assert_eq!(
        actor
            .load_navigation(&key_id)
            .expect("load cleared navigation"),
        NavigationState::default()
    );
}

#[test]
fn encrypted_navigation_state_preserves_room_scroll_anchor() {
    let data_dir = tempdir().expect("tempdir");
    let cred_dir = tempdir().expect("tempdir");
    let key_id = make_key_id();
    let actor = file_store_actor(&data_dir, &cred_dir);
    let navigation = NavigationState {
        active_space_id: Some("!space:test.example.com".to_owned()),
        active_room_id: Some("!room:test.example.com".to_owned()),
        home_selection: koushi_state::HomeSelection::default(),
        space_local_presentations: koushi_state::SpaceLocalPresentations::default(),
        legacy_frontend_preferences_imported: false,
        space_order: vec!["!space:test.example.com".to_owned()],
        last_room_by_space_id: std::collections::BTreeMap::from([(
            "!space:test.example.com".to_owned(),
            "!room:test.example.com".to_owned(),
        )]),
        last_selection_by_space_id: std::collections::BTreeMap::from([(
            "!space:test.example.com".to_owned(),
            koushi_state::SpaceNavigationSelection {
                surface: koushi_state::SpaceConversationSurface::Rooms,
                room_id: Some("!room:test.example.com".to_owned()),
            },
        )]),
        room_scroll_anchors: std::collections::BTreeMap::from([(
            "!room:test.example.com".to_owned(),
            koushi_state::TimelineScrollAnchor {
                event_id: "$anchor:event".to_owned(),
                edge: koushi_state::TimelineScrollAnchorEdge::Top,
                offset_px: -32,
                updated_at_ms: 1_820_000_000_000,
            },
        )]),
        main_timeline_anchor: None,
        event_navigation: Default::default(),
    };

    actor
        .save_navigation(&key_id, &navigation)
        .expect("save encrypted navigation");
    let loaded = actor
        .load_navigation(&key_id)
        .expect("load encrypted navigation");

    assert_eq!(loaded, navigation);
}

#[test]
fn legacy_navigation_json_without_scroll_anchors_loads_with_empty_map() {
    let data_dir = tempdir().expect("tempdir");
    let cred_dir = tempdir().expect("tempdir");
    let key_id = make_key_id();
    let actor = file_store_actor(&data_dir, &cred_dir);
    let legacy_path = actor.account_navigation_legacy_file(&key_id);
    std::fs::create_dir_all(legacy_path.parent().expect("navigation parent"))
        .expect("create navigation parent");
    std::fs::write(
        &legacy_path,
        r#"{
                "active_space_id":"!space:test.example.com",
                "active_room_id":"!room:test.example.com",
                "space_order":["!space:test.example.com"],
                "last_room_by_space_id":{"!space:test.example.com":"!room:test.example.com"}
            }"#,
    )
    .expect("write legacy navigation");

    let loaded = actor
        .load_navigation(&key_id)
        .expect("load legacy navigation");

    assert!(loaded.room_scroll_anchors.is_empty());
    assert_eq!(
        loaded.active_room_id.as_deref(),
        Some("!room:test.example.com")
    );
}
