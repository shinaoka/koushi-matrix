use matrix_desktop_backend::{DEFAULT_HOMESERVER, FakeDesktopBackend, FakeDesktopBackendConfig};
use matrix_desktop_state::{
    AppAction, AuthDiscoveryState, AuthSecret, LoginFlowKind, LoginRequest, SearchMatchField,
    SearchScope, SearchState, SessionState, SyncState, ThreadPaneState,
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
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
fn fake_backend_discovers_password_and_sso_login_methods() {
    let mut backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        restore_session: false,
        ..FakeDesktopBackendConfig::default()
    });

    backend.dispatch(AppAction::LoginDiscoveryRequested {
        homeserver: "https://matrix.example.org".to_owned(),
    });

    let AuthDiscoveryState::Ready { homeserver, flows } = &backend.snapshot().state.auth else {
        panic!("expected discovered login flows");
    };

    assert_eq!(homeserver, "https://matrix.example.org");
    assert!(
        flows
            .iter()
            .any(|flow| flow.kind == LoginFlowKind::Password)
    );
    assert!(
        flows
            .iter()
            .any(|flow| { flow.kind == LoginFlowKind::Sso && flow.delegated_oidc_compatibility })
    );
}

#[test]
fn http_backend_discovers_login_methods_from_homeserver() {
    let homeserver = spawn_login_discovery_server(
        200,
        r#"{"flows":[{"type":"m.login.password"},{"type":"m.login.token"}]}"#,
    );
    let mut backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        restore_session: false,
        login_discovery: matrix_desktop_backend::LoginDiscoveryMode::Http,
        ..FakeDesktopBackendConfig::default()
    });

    backend.dispatch(AppAction::LoginDiscoveryRequested {
        homeserver: homeserver.clone(),
    });

    let AuthDiscoveryState::Ready {
        homeserver: discovered_homeserver,
        flows,
    } = &backend.snapshot().state.auth
    else {
        panic!("expected discovered login flows");
    };

    assert_eq!(discovered_homeserver, &homeserver);
    assert_eq!(flows[0].kind, LoginFlowKind::Password);
    assert_eq!(flows[1].kind, LoginFlowKind::Token);
}

#[test]
fn http_backend_records_login_discovery_failure() {
    let homeserver = spawn_login_discovery_server(
        404,
        r#"{"errcode":"M_UNRECOGNIZED","error":"OAuth 2.0 authentication is in use on this homeserver."}"#,
    );
    let mut backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        restore_session: false,
        login_discovery: matrix_desktop_backend::LoginDiscoveryMode::Http,
        ..FakeDesktopBackendConfig::default()
    });

    backend.dispatch(AppAction::LoginDiscoveryRequested {
        homeserver: homeserver.clone(),
    });

    let AuthDiscoveryState::Failed {
        homeserver: failed_homeserver,
        message,
    } = &backend.snapshot().state.auth
    else {
        panic!("expected login discovery failure");
    };

    assert_eq!(failed_homeserver, &homeserver);
    assert!(message.contains("HTTP 404"));
    assert!(message.contains("OAuth 2.0 authentication is in use"));
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

fn spawn_login_discovery_server(status: u16, body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let addr = listener
        .local_addr()
        .expect("test server should have an address");

    thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("test server should accept a request");
        let mut request = [0_u8; 2048];
        let bytes_read = stream
            .read(&mut request)
            .expect("test server should read request");
        let request = String::from_utf8_lossy(&request[..bytes_read]);
        assert!(request.starts_with("GET /_matrix/client/v3/login HTTP/1.1"));

        let response = format!(
            "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .expect("test server should write response");
    });

    format!("http://{addr}")
}
