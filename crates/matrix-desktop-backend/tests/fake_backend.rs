use matrix_desktop_backend::{DEFAULT_HOMESERVER, FakeDesktopBackend, FakeDesktopBackendConfig};
use matrix_desktop_state::{
    AppAction, SearchMatchField, SearchScope, SearchState, SessionState, SyncState, ThreadPaneState,
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
        Some("!space-seminars:example.org")
    );
    assert_eq!(
        snapshot.state.navigation.active_room_id.as_deref(),
        Some("!springschool:example.org")
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
            .any(|message| message.event_id == "$zoom-invite")
    );
    assert_eq!(
        snapshot
            .thread
            .as_ref()
            .map(|thread| thread.root_event_id.as_str()),
        Some("$zoom-invite")
    );
    assert!(
        snapshot
            .sidebar
            .global_dms
            .iter()
            .any(|room| room.display_name == "Akio")
    );
}

#[test]
fn fake_backend_keeps_homeserver_configurable() {
    let backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        homeserver: "https://matrix.example.org".into(),
        user_id: "@alice:example.org".into(),
        device_id: "ALICEDEVICE".into(),
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
fn fake_backend_keeps_dms_global_when_switching_spaces() {
    let mut backend = FakeDesktopBackend::booted();

    backend.dispatch(AppAction::SelectSpace {
        space_id: Some("!space-lab:example.org".into()),
    });
    let snapshot = backend.snapshot();

    assert_eq!(
        snapshot
            .sidebar
            .space_rooms
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["!search-dev:example.org"]
    );
    assert!(
        snapshot
            .sidebar
            .global_dms
            .iter()
            .any(|room| { room.room_id == "!dm-akio:example.org" && room.display_name == "Akio" })
    );
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

    let results = backend.submit_search("Zoom", SearchScope::AllRooms);

    assert_eq!(
        results
            .iter()
            .map(|result| result.event_id.as_str())
            .collect::<Vec<_>>(),
        vec!["$zoom-invite"]
    );
    assert_eq!(results[0].match_field, SearchMatchField::MessageBody);
    assert_eq!(results[0].highlights[0].start_utf16, 33);
    assert!(matches!(
        backend.snapshot().state.search,
        SearchState::Results { .. }
    ));
}

#[test]
fn fake_backend_searches_attachment_filenames() {
    let mut backend = FakeDesktopBackend::booted();

    let results = backend.submit_search("seminar_budget.xlsx", SearchScope::AllRooms);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].event_id, "$budget-file");
    assert_eq!(results[0].match_field, SearchMatchField::AttachmentFileName);
    assert_eq!(results[0].snippet, "seminar_budget.xlsx");
}

#[test]
fn fake_backend_search_uses_visible_edited_message_body() {
    let mut backend = FakeDesktopBackend::booted();

    let results = backend.submit_search("venue", SearchScope::AllRooms);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].event_id, "$late-original");
    assert_eq!(results[0].snippet, "Final venue checklist");
}
