use matrix_desktop_backend::{DEFAULT_HOMESERVER, FakeDesktopBackend, FakeDesktopBackendConfig};
use matrix_desktop_state::{
    AppAction, AuthSecret, LoginRequest, SearchMatchField, SearchScope, SearchState, SessionState,
    SyncState, ThreadPaneState,
};

#[test]
fn fake_backend_boots_into_ready_session_with_rooms_and_thread() {
    let backend = FakeDesktopBackend::booted();

    let snapshot = backend.snapshot();

    let SessionState::Ready(session_info) = &snapshot.state.session else {
        panic!("expected ready session");
    };
    assert_eq!(session_info.homeserver, DEFAULT_HOMESERVER);
    assert_eq!(snapshot.state.sync, SyncState::Running);
    assert_eq!(
        snapshot.state.navigation.active_space_id.as_deref(),
        Some("!space-alpha:example.invalid")
    );
    assert_eq!(
        snapshot.state.navigation.active_room_id.as_deref(),
        Some("!room-alpha:example.invalid")
    );
    assert!(snapshot.state.timeline.is_subscribed);
    assert!(matches!(
        snapshot.state.thread,
        ThreadPaneState::Open { .. }
    ));
    assert!(
        snapshot
            .timeline
            .iter()
            .any(|message| message.event_id == "$alpha-update")
    );
    assert_eq!(
        snapshot
            .thread
            .as_ref()
            .map(|thread| thread.root_event_id.as_str()),
        Some("$alpha-update")
    );
    assert!(
        snapshot
            .sidebar
            .global_dms
            .iter()
            .any(|room| room.display_name == "Member 1")
    );
}

#[test]
fn fake_backend_keeps_homeserver_configurable() {
    let backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        homeserver: "https://matrix.example.org".into(),
        user_id: "@user-a:example.invalid".into(),
        device_id: "DEVICE_A".into(),
        ..FakeDesktopBackendConfig::default()
    });

    let SessionState::Ready(session_info) = &backend.snapshot().state.session else {
        panic!("expected ready session");
    };
    assert_eq!(session_info.homeserver, "https://matrix.example.org");
    assert_eq!(
        backend.session_key_id().homeserver,
        "https://matrix.example.org"
    );
}

#[test]
fn fake_backend_can_boot_without_saved_session() {
    let backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        restore_session: false,
        ..FakeDesktopBackendConfig::default()
    });

    let snapshot = backend.snapshot();

    assert_eq!(snapshot.state.session, SessionState::SignedOut);
    assert_eq!(snapshot.state.sync, SyncState::Stopped);
    assert!(snapshot.state.rooms.is_empty());
    assert!(snapshot.state.spaces.is_empty());
    assert!(snapshot.state.errors.is_empty());
}

#[test]
fn fake_backend_login_boundary_fails_explicitly_before_real_sdk_wiring() {
    let mut backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        restore_session: false,
        ..FakeDesktopBackendConfig::default()
    });

    backend.dispatch(AppAction::LoginSubmitted(LoginRequest {
        homeserver: "https://matrix.example.org".to_owned(),
        username: "demo-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    }));

    let snapshot = backend.snapshot();

    assert_eq!(snapshot.state.session, SessionState::SignedOut);
    assert!(snapshot.state.rooms.is_empty());
    assert_eq!(snapshot.state.errors.len(), 1);
    assert_eq!(snapshot.state.errors[0].code, "login_failed");
    assert!(
        snapshot.state.errors[0]
            .message
            .contains("real Matrix login is not wired")
    );
    assert!(!format!("{snapshot:?}").contains("synthetic-password"));
}

#[test]
fn fake_backend_keeps_dms_global_when_switching_spaces() {
    let mut backend = FakeDesktopBackend::booted();

    backend.dispatch(AppAction::SelectSpace {
        space_id: Some("!space-beta:example.invalid".into()),
    });
    let snapshot = backend.snapshot();

    assert_eq!(
        snapshot
            .sidebar
            .space_rooms
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["!room-search:example.invalid"]
    );
    assert!(snapshot.sidebar.global_dms.iter().any(|room| {
        room.room_id == "!dm-member-1:example.invalid" && room.display_name == "Member 1"
    }));
    assert!(
        snapshot
            .sidebar
            .space_rooms
            .iter()
            .all(|room| !room.room_id.starts_with("!dm-"))
    );
}

#[test]
fn fake_backend_search_drops_ngram_false_positive() {
    let mut backend = FakeDesktopBackend::booted();

    let results = backend.submit_search("Alpha", SearchScope::AllRooms);

    assert_eq!(
        results
            .iter()
            .map(|result| result.event_id.as_str())
            .collect::<Vec<_>>(),
        vec!["$alpha-update"]
    );
    assert_eq!(results[0].match_field, SearchMatchField::MessageBody);
    assert_eq!(results[0].highlights[0].start_utf16, 0);
    assert!(matches!(
        backend.snapshot().state.search,
        SearchState::Results { .. }
    ));
}

#[test]
fn fake_backend_searches_attachment_filenames() {
    let mut backend = FakeDesktopBackend::booted();

    let results = backend.submit_search("fixture_budget.xlsx", SearchScope::AllRooms);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].event_id, "$budget-file");
    assert_eq!(results[0].match_field, SearchMatchField::AttachmentFileName);
    assert_eq!(results[0].snippet, "fixture_budget.xlsx");
}

#[test]
fn fake_backend_search_uses_visible_edited_message_body() {
    let mut backend = FakeDesktopBackend::booted();

    let results = backend.submit_search("checklist", SearchScope::AllRooms);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].event_id, "$late-original");
    assert_eq!(results[0].snippet, "Final synthetic checklist");
}
