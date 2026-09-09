use super::{
    QaMessagesProxyDecision, QaMessagesProxyExpectation, QaMessagesProxyState,
    QaRoomMessagesRequestMetadata, invite_observer_diagnostic_summary,
    qa_room_messages_request_metadata, rewrite_http_request_connection_close,
    send_lifecycle_diagnostic_summary, trust_admission_diagnostic_summary,
};

#[test]
fn held_media_responses_detect_peer_close_and_release_without_holding_sync() {
    use std::{
        io::{Read, Write},
        net::{Shutdown, TcpListener, TcpStream},
        time::{Duration, Instant},
    };
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    server.set_nonblocking(true).unwrap();
    let proxy =
        super::QaTcpProxy::start(&format!("http://{}", server.local_addr().unwrap())).unwrap();
    let serving = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(4);
        for _ in 0..3 {
            let mut stream = loop {
                match server.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline);
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("fixture accept failed: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                headers.push(byte[0]);
                assert!(headers.len() < 4096);
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .unwrap();
        }
    });
    let request = |path: &str| {
        let mut client = TcpStream::connect(proxy.listen_addr).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        write!(
            client,
            "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        client
    };
    let wait = |condition: &dyn Fn() -> bool| {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !condition() {
            assert!(Instant::now() < deadline, "media gate did not settle");
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    proxy.hold_media_responses();
    let cancelled = request("/_matrix/client/v1/media/download/example.invalid/one");
    wait(&|| proxy.media_responses_held_count() == 1);
    cancelled
        .set_read_timeout(Some(Duration::from_millis(30)))
        .unwrap();
    assert!(matches!(
        cancelled.peek(&mut [0]).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    ));
    cancelled.shutdown(Shutdown::Both).unwrap();
    drop(cancelled);
    wait(&|| proxy.media_peer_closed_count() == 1 && proxy.media_responses_held_count() == 0);
    let mut sync = request("/_matrix/client/v3/sync");
    let mut response = String::new();
    sync.read_to_string(&mut response).unwrap();
    assert!(response.ends_with("ok"));
    let mut released = request("/_matrix/client/v1/media/download/example.invalid/two");
    wait(&|| proxy.media_responses_held_count() == 1);
    proxy.release_media_responses();
    response.clear();
    released.read_to_string(&mut response).unwrap();
    assert!(response.ends_with("ok"));
    wait(&|| proxy.media_responses_held_count() == 0);
    assert_eq!(proxy.media_read_forwarded_count(), 2);
    assert_eq!(proxy.media_peer_closed_count(), 1);
    serving.join().unwrap();
}

#[test]
fn proxy_counts_actual_media_reads_without_counting_uploads_or_sync() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        time::{Duration, Instant},
    };
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    server.set_nonblocking(true).unwrap();
    let proxy =
        super::QaTcpProxy::start(&format!("http://{}", server.local_addr().unwrap())).unwrap();
    let requests = [
        (
            "GET",
            "/_matrix/client/v1/media/download/example.invalid/one",
            1,
        ),
        (
            "GET",
            "/_matrix/client/v1/media/thumbnail/example.invalid/two?width=32",
            2,
        ),
        ("GET", "/_matrix/media/v3/download/example.invalid/three", 3),
        ("GET", "/_matrix/media/r0/thumbnail/example.invalid/four", 4),
        ("POST", "/_matrix/media/v3/upload", 4),
        ("GET", "/_matrix/client/v3/sync", 4),
        ("GET", "/_matrix/media/v3/config", 4),
        (
            "GET",
            "/_matrix/client/v1/media/download/example.invalid/",
            4,
        ),
        (
            "POST",
            "/_matrix/client/v1/media/download/example.invalid/five",
            4,
        ),
    ];
    let worker = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut forwarded = 0;
        while forwarded < requests.len() {
            assert!(
                Instant::now() < deadline,
                "proxy did not forward all requests"
            );
            let (mut stream, _) = match server.accept() {
                Ok(pair) => pair,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("accept failed: {error}"),
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                head.push(byte[0]);
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .unwrap();
            forwarded += 1;
        }
    });
    for (method, path, expected) in requests {
        let mut client =
            std::net::TcpStream::connect(proxy.homeserver_url().trim_start_matches("http://"))
                .unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        write!(client, "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).unwrap();
        assert!(response.ends_with(b"ok"));
        assert_eq!(proxy.media_read_forwarded_count(), expected);
    }
    worker.join().unwrap();
}

#[test]
fn trust_admission_timeout_summary_is_allowlisted_and_private_safe() {
    use koushi_diagnostics::{
        DiagnosticEvent, DiagnosticField, DiagnosticLevel, DiagnosticRecord, DiagnosticSnapshot,
    };

    let record = |event| DiagnosticRecord {
        timestamp_ms: 0,
        event,
    };
    let snapshot = DiagnosticSnapshot {
        records: vec![
            record(
                DiagnosticEvent::new(
                    DiagnosticLevel::Info,
                    "core.verification_admission",
                    "trust_recheck_requested",
                )
                .field(DiagnosticField::token(
                    "ignored_private_field",
                    "@private:example.invalid",
                )),
            ),
            record(DiagnosticEvent::new(
                DiagnosticLevel::Info,
                "other.source",
                "trust_recheck_started",
            )),
            record(DiagnosticEvent::new(
                DiagnosticLevel::Info,
                "core.verification_admission",
                "unallowlisted-private-stage",
            )),
            record(DiagnosticEvent::new(
                DiagnosticLevel::Info,
                "core.verification_admission",
                "trust_recheck_started",
            )),
            record(DiagnosticEvent::new(
                DiagnosticLevel::Info,
                "core.verification_admission",
                "trust_recheck_finished_verified",
            )),
        ],
        dropped_records: 0,
    };

    let summary = trust_admission_diagnostic_summary(&snapshot);
    assert_eq!(
        summary,
        "trust_recheck_requested>trust_recheck_started>trust_recheck_finished_verified"
    );
    assert!(!summary.contains("private"));
}

#[test]
fn invite_timeout_diagnostic_summary_is_allowlisted_and_private_safe() {
    use koushi_diagnostics::{
        DiagnosticEvent, DiagnosticField, DiagnosticLevel, DiagnosticRecord, DiagnosticSnapshot,
    };

    let record = |event| DiagnosticRecord {
        timestamp_ms: 0,
        event,
    };
    let snapshot = DiagnosticSnapshot {
        records: vec![
            record(DiagnosticEvent::new(
                DiagnosticLevel::Debug,
                "core.room",
                "live_observer_started",
            )),
            record(
                DiagnosticEvent::new(
                    DiagnosticLevel::Debug,
                    "core.room",
                    "live_observer_wake_milestone",
                )
                .field(DiagnosticField::token("source", "rls_diff"))
                .field(DiagnosticField::count("wake_count", 4))
                .field(DiagnosticField::token(
                    "ignored_private_field",
                    "!private-room:example.invalid",
                )),
            ),
            record(
                DiagnosticEvent::new(
                    DiagnosticLevel::Debug,
                    "core.room",
                    "live_observer_wake_milestone",
                )
                .field(DiagnosticField::token("source", "base_room_updates"))
                .field(DiagnosticField::count("wake_count", 8))
                .field(DiagnosticField::boolean("invite_update_observed", true))
                .field(DiagnosticField::boolean("invite_membership_changed", false))
                .field(DiagnosticField::boolean("projection_required", true)),
            ),
            record(DiagnosticEvent::new(
                DiagnosticLevel::Debug,
                "core.room",
                "live_observer_invite_projection",
            )),
            record(
                DiagnosticEvent::new(
                    DiagnosticLevel::Debug,
                    "core.room",
                    "live_observer_invite_projection_completed",
                )
                .field(DiagnosticField::boolean("action_delivered", true)),
            ),
            record(DiagnosticEvent::new(
                DiagnosticLevel::Warn,
                "core.room",
                "live_observer_base_lagged",
            )),
            record(DiagnosticEvent::new(
                DiagnosticLevel::Warn,
                "core.room",
                "live_observer_auxiliary_closed",
            )),
            record(DiagnosticEvent::new(
                DiagnosticLevel::Error,
                "core.room",
                "live_observer_exit",
            )),
        ],
        dropped_records: 2,
    };

    let summary = invite_observer_diagnostic_summary(&snapshot);
    assert_eq!(
        summary,
        "observer_diag_started=1 observer_diag_rls_wake_max=4 \
         observer_diag_base_wake_max=8 observer_diag_base_invite_update_seen=true \
         observer_diag_base_membership_change_seen=false \
         observer_diag_base_projection_required_seen=true \
         observer_diag_invite_projection=1 observer_diag_invite_projection_delivered=1 \
         observer_diag_invite_projection_undelivered=0 observer_diag_last_projection_rooms=0 \
         observer_diag_last_projection_spaces=0 observer_diag_last_projection_invites=0 \
         observer_diag_last_refresh_entries=0 observer_diag_last_refresh_invites=0 \
         observer_diag_last_refresh_authoritative=false \
         observer_diag_last_refresh_room_present=false \
         observer_diag_lagged=1 \
         observer_diag_closed=1 observer_diag_exit=1 observer_diag_last_exit_reason=unknown \
         observer_diag_dropped=2"
    );
    assert!(!summary.contains("private-room"));
    assert!(!summary.contains("room_id"));
}

#[test]
fn send_lifecycle_summary_is_per_send_and_private_safe() {
    use koushi_diagnostics::{
        DiagnosticEvent, DiagnosticField, DiagnosticLevel, DiagnosticRecord, DiagnosticSnapshot,
    };

    let record = |event| DiagnosticRecord {
        timestamp_ms: 0,
        event,
    };
    let snapshot = DiagnosticSnapshot {
        records: vec![
            record(
                DiagnosticEvent::new(DiagnosticLevel::Info, "core.send", "accepted")
                    .field(DiagnosticField::correlation("correlation", 7))
                    .field(DiagnosticField::token("send_kind", "text"))
                    .field(DiagnosticField::milliseconds(
                        "elapsed_since_submission_ms",
                        0,
                    ))
                    .field(DiagnosticField::milliseconds(
                        "elapsed_since_previous_ms",
                        0,
                    ))
                    .field(DiagnosticField::token("private_body", "do-not-print")),
            ),
            record(
                DiagnosticEvent::new(DiagnosticLevel::Info, "core.send", "sdk_enqueue_started")
                    .field(DiagnosticField::correlation("correlation", 7))
                    .field(DiagnosticField::token("send_kind", "text"))
                    .field(DiagnosticField::milliseconds(
                        "elapsed_since_submission_ms",
                        3,
                    ))
                    .field(DiagnosticField::milliseconds(
                        "elapsed_since_previous_ms",
                        3,
                    )),
            ),
            record(
                DiagnosticEvent::new(DiagnosticLevel::Info, "core.send", "terminal_applied")
                    .field(DiagnosticField::correlation("correlation", 7))
                    .field(DiagnosticField::token("send_kind", "text"))
                    .field(DiagnosticField::token("outcome", "succeeded"))
                    .field(DiagnosticField::token("delivery_mode", "immediate"))
                    .field(DiagnosticField::milliseconds(
                        "elapsed_since_submission_ms",
                        8,
                    ))
                    .field(DiagnosticField::milliseconds(
                        "elapsed_since_previous_ms",
                        5,
                    )),
            ),
            record(DiagnosticEvent::new(
                DiagnosticLevel::Info,
                "other.source",
                "not-a-send",
            )),
        ],
        dropped_records: 0,
    };

    assert_eq!(
        send_lifecycle_diagnostic_summary(&snapshot),
        "corr=7 stage=accepted kind=text outcome=none mode=none elapsed_ms=0 delta_ms=0;corr=7 stage=sdk_enqueue_started kind=text outcome=none mode=none elapsed_ms=3 delta_ms=3;corr=7 stage=terminal_applied kind=text outcome=succeeded mode=immediate elapsed_ms=8 delta_ms=5"
    );
    assert!(!send_lifecycle_diagnostic_summary(&snapshot).contains("do-not-print"));
}

#[test]
fn send_queue_proxy_forces_connection_close_per_request() {
    let request = b"POST /_matrix/client/v3/login HTTP/1.1\r\nHost: example.test\r\nConnection: keep-alive\r\nProxy-Connection: keep-alive\r\nContent-Length: 2\r\n\r\n{}";
    let rewritten = rewrite_http_request_connection_close(request).unwrap();
    let rewritten = String::from_utf8(rewritten).unwrap();
    let (head, body) = rewritten.split_once("\r\n\r\n").unwrap();

    assert!(
        head.contains("\r\nConnection: close"),
        "send queue proxy must force one HTTP request per connection so response copying can read to EOF"
    );
    assert!(
        !head.to_ascii_lowercase().contains("proxy-connection"),
        "send queue proxy must drop proxy keep-alive headers before forwarding"
    );
    assert_eq!(body, "{}");
}

#[test]
fn live_tail_proxy_enforces_tokenless_refresh_and_exact_continuation_requests() {
    let metadata = qa_room_messages_request_metadata(
        b"GET /_matrix/client/v3/rooms/%21room%3Aexample.invalid/messages?dir=b&limit=128 HTTP/1.1\r\nHost: example.invalid\r\n\r\n",
    )
    .expect("valid request")
    .expect("room messages metadata");
    assert_eq!(
        metadata,
        QaRoomMessagesRequestMetadata {
            query_is_exact_tokenless_limit: true,
            has_from: false,
            direction_is_backward: true,
            from_token: None,
        }
    );

    let mut state = QaMessagesProxyState::default();
    state.arm_page(QaMessagesProxyExpectation::TokenlessLiveTail, None);
    assert_eq!(
        state.observe_room_messages_request(&metadata),
        QaMessagesProxyDecision::ServeCannedPage
    );

    let continuation = qa_room_messages_request_metadata(
        b"GET /_matrix/client/v3/rooms/%21room%3Aexample.invalid/messages?dir=b&from=continuation&limit=128 HTTP/1.1\r\nHost: example.invalid\r\n\r\n",
    )
    .expect("valid continuation request")
    .expect("room messages continuation metadata");
    state.arm_page(
        QaMessagesProxyExpectation::BackwardFrom {
            token: "continuation".to_owned(),
        },
        Some("continuation".to_owned()),
    );
    assert_eq!(
        state.observe_room_messages_request(&continuation),
        QaMessagesProxyDecision::ServeCannedPage
    );
    assert!(state.observation.expected_end_token_was_used);
    assert_eq!(state.observation.expected_end_token_request_count, 1);
}
