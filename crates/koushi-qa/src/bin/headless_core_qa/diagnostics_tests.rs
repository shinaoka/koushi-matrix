use super::{
    QaMessagesProxyDecision, QaMessagesProxyExpectation, QaMessagesProxyState,
    QaRoomMessagesRequestMetadata, invite_observer_diagnostic_summary,
    qa_room_messages_request_metadata, rewrite_http_request_connection_close,
    send_lifecycle_diagnostic_summary, trust_admission_diagnostic_summary,
};

#[test]
fn read_state_observation_classifies_only_bounded_marker_bodies() {
    let body = br#"{"m.read.private":"synthetic","m.fully_read":"synthetic"}"#;
    let headers = format!(
        "POST /_matrix/client/v3/rooms/synthetic/read_markers HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    assert!(super::valid_read_state_headers(
        headers.as_bytes(),
        body.len()
    ));
    let mut request = headers.into_bytes();
    request.extend_from_slice(body);
    assert_eq!(
        super::read_state_body_keys(&request),
        Some([false, true, true])
    );
    assert!(!super::valid_read_state_headers(
        b"POST /read_markers HTTP/1.1\r\n\r\n",
        4
    ));
    assert!(!super::valid_read_state_headers(
        b"POST /read_markers HTTP/1.1\r\nContent-Length: 4\r\nTransfer-Encoding: chunked\r\n\r\n",
        4
    ));
    assert!(!super::valid_read_state_headers(
        b"POST /read_markers HTTP/1.1\r\nContent-Length: 65537\r\n\r\n",
        65537
    ));
    assert!(!super::valid_read_state_headers(
        b"POST /read_markers HTTP/1.1\r\nContent-Length: 4\r\nContent-Length: 4\r\n\r\n",
        4
    ));
    assert!(!super::valid_read_state_headers(
        b"POST /receipt/m.read/synthetic HTTP/1.1\r\nContent-Length: 4\r\n\r\n",
        4
    ));
    assert_eq!(
        super::read_state_body_keys(b"POST /read_markers HTTP/1.1\r\n\r\n{}"),
        None
    );
    assert_eq!(
        super::read_state_body_keys(
            b"POST /read_markers HTTP/1.1\r\n\r\n{\"m.read\":\"synthetic\"}"
        ),
        Some([true, false, false])
    );
    assert_eq!(
        super::read_state_body_keys(b"POST /read_markers HTTP/1.1\r\n\r\n{"),
        None
    );
}

#[test]
fn failed_marker_attempts_are_recorded_by_body_key() {
    use std::sync::{Arc, Condvar, Mutex};
    let control = Arc::new((
        Mutex::new(super::QaReadStateProxyControl::default()),
        Condvar::new(),
    ));
    control.0.lock().unwrap().mode = super::QaReadStateProxyMode::FailClosed;
    let request = b"POST /read_markers HTTP/1.1\r\n\r\n{\"m.read.private\":\"synthetic\",\"m.fully_read\":\"synthetic\"}";
    let action =
        super::qa_read_state_proxy_action(&control, super::QaProxyRequestKind::ReadState, request)
            .unwrap();
    assert!(matches!(
        action,
        Some(super::QaProxyRequestAction::FailClosed)
    ));
    let state = control.0.lock().unwrap();
    assert_eq!(state.observation.request_count, 1);
    assert_eq!(state.attempts[0].len(), 0);
    assert_eq!(state.attempts[1].len(), 1);
    assert_eq!(state.attempts[1], state.attempts[2]);
    assert!(!state.attempt_error);
    drop(state);
    super::qa_read_state_proxy_action(
        &control,
        super::QaProxyRequestKind::ReadState,
        b"POST /read_markers HTTP/1.1\r\n\r\n{}",
    )
    .unwrap();
    assert!(control.0.lock().unwrap().attempt_error);
}

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
        navigation: Default::default(),
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

    let summary = trust_admission_diagnostic_summary(&snapshot, None);
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
        navigation: Default::default(),
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
        navigation: Default::default(),
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

#[test]
fn held_sliding_sync_responses_include_a_long_poll_forwarded_before_the_hold() {
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        sync::mpsc,
        time::{Duration, Instant},
    };
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy =
        super::QaTcpProxy::start(&format!("http://{}", server.local_addr().unwrap())).unwrap();
    let (forwarded_tx, forwarded_rx) = mpsc::channel();
    let (respond_tx, respond_rx) = mpsc::channel::<()>();
    let serving = std::thread::spawn(move || {
        let (mut stream, _) = server.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            head.push(byte[0]);
        }
        forwarded_tx.send(()).unwrap();
        // The long poll answers only after the test has started the hold.
        respond_rx.recv().unwrap();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .unwrap();
    });
    let mut client = TcpStream::connect(proxy.listen_addr).unwrap();
    write!(
        client,
        "POST /_matrix/client/unstable/org.matrix.simplified_msc3575/sync HTTP/1.1\r\n\
         Host: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    forwarded_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("sync request was not forwarded");
    proxy.hold_sliding_sync_responses();
    respond_tx.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while proxy.sliding_sync_responses_held_count() != 1 {
        assert!(Instant::now() < deadline, "sync response was not held");
        std::thread::sleep(Duration::from_millis(5));
    }
    client
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    assert!(matches!(
        client.peek(&mut [0]).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    ));
    proxy.release_sliding_sync_responses();
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut response = String::new();
    client.read_to_string(&mut response).unwrap();
    assert!(response.ends_with("ok"));
    assert_eq!(proxy.sliding_sync_responses_held_count(), 0);
    serving.join().unwrap();
}

/// A distinct, process-unique owner per call: two calls model two account
/// runtimes in one process, which is the aggregate lane's shape.
fn test_diagnostics_owner() -> koushi_core::SyncDiagnosticsOwner {
    koushi_core::SlidingSyncDiagnostics::default().owner()
}

fn owner_field(owner: koushi_core::SyncDiagnosticsOwner) -> koushi_diagnostics::DiagnosticField {
    koushi_diagnostics::DiagnosticField::count("sync_owner", owner.get())
}

fn sync_snapshot(
    records: Vec<(&'static str, Vec<koushi_diagnostics::DiagnosticField>)>,
) -> koushi_diagnostics::DiagnosticSnapshot {
    koushi_diagnostics::DiagnosticSnapshot {
        records: records
            .into_iter()
            .enumerate()
            .map(
                |(index, (stage, fields))| koushi_diagnostics::DiagnosticRecord {
                    timestamp_ms: index as u64,
                    event: fields.into_iter().fold(
                        koushi_diagnostics::DiagnosticEvent::new(
                            koushi_diagnostics::DiagnosticLevel::Debug,
                            "core.sync",
                            stage,
                        ),
                        koushi_diagnostics::DiagnosticEvent::field,
                    ),
                },
            )
            .collect(),
        dropped_records: 0,
        navigation: Default::default(),
    }
}

#[test]
fn sync_diagnostic_summary_names_the_first_response_handoff_step() {
    use koushi_diagnostics::DiagnosticField;

    let snapshot = sync_snapshot(vec![
        (
            "status_projected",
            vec![
                DiagnosticField::count("generation", 4),
                DiagnosticField::token("lifecycle", "starting"),
            ],
        ),
        (
            "committed_response",
            vec![
                DiagnosticField::count("sequence", 226),
                DiagnosticField::boolean("pos_present", true),
                DiagnosticField::count("rooms_from_response", 11),
            ],
        ),
        (
            "room_list_reconcile_wait",
            vec![
                DiagnosticField::token("outcome", "timeout"),
                DiagnosticField::milliseconds("elapsed_ms", 10_000),
            ],
        ),
        (
            "first_response_stalled",
            vec![DiagnosticField::token("phase", "sync_running")],
        ),
    ]);

    // Records without `sync_owner` cannot be attributed to a runtime, so the
    // summary falls back to the whole ring and says so.
    let summary = super::sync_diagnostic_summary(&snapshot, test_diagnostics_owner());

    for expected in [
        "sync_diag_scope=process",
        "sync_diag_status_projected=1",
        "sync_diag_last_lifecycle=starting",
        "sync_diag_last_status_generation=4",
        "sync_diag_last_committed_sequence=226",
        "sync_diag_last_committed_pos_present=true",
        "sync_diag_last_rooms_from_response=11",
        "sync_diag_reconcile_wait=1",
        "sync_diag_last_reconcile_outcome=timeout",
        "sync_diag_last_reconcile_elapsed_ms=10000",
        "sync_diag_first_response_stalled=1",
        "sync_diag_last_stall_phase=sync_running",
    ] {
        assert!(
            summary.contains(expected),
            "expected `{expected}` in `{summary}`"
        );
    }
}

#[test]
fn sync_diagnostic_summary_uses_fixed_placeholders_without_sync_records() {
    let summary = super::sync_diagnostic_summary(&sync_snapshot(vec![]), test_diagnostics_owner());

    for expected in [
        "sync_diag_scope=process",
        "sync_diag_status_projected=0",
        "sync_diag_last_status_generation=0",
        "sync_diag_last_committed_sequence=0",
        "sync_diag_last_committed_pos_present=false",
        "sync_diag_reconcile_wait=0",
        "sync_diag_last_reconcile_outcome=none",
        "sync_diag_last_reconcile_elapsed_ms=0",
        "sync_diag_first_response_stalled=0",
        "sync_diag_last_stall_phase=none",
    ] {
        assert!(
            summary.contains(expected),
            "expected `{expected}` in `{summary}`"
        );
    }
}

/// #1634 step 1: the aggregate lane runs one `CoreRuntime` per participant in a
/// single process, so a summary built from the whole ring mixed the stuck run's
/// records with every other run's. `sync_owner` names the runtime and
/// `backend_generation` the run inside it; both runs below use generation 1,
/// which is exactly why the owner is needed.
#[test]
fn sync_diagnostic_summary_reports_the_stuck_run_not_the_process() {
    use koushi_diagnostics::DiagnosticField;

    let stuck = test_diagnostics_owner();
    let healthy = test_diagnostics_owner();
    assert_ne!(stuck.get(), healthy.get());

    let handoff =
        |owner: koushi_core::SyncDiagnosticsOwner, step: &'static str, phase: &'static str| {
            (
                "handoff_step",
                vec![
                    owner_field(owner),
                    DiagnosticField::token("step", step),
                    DiagnosticField::token("phase", phase),
                    DiagnosticField::count("backend_generation", 1),
                ],
            )
        };

    let snapshot = sync_snapshot(vec![
        // A run that finished its handoff. It must not contribute a count.
        (
            "status_projected",
            vec![
                owner_field(healthy),
                DiagnosticField::count("generation", 2),
                DiagnosticField::token("lifecycle", "running"),
            ],
        ),
        (
            "committed_response",
            vec![
                owner_field(healthy),
                DiagnosticField::count("sequence", 899),
                DiagnosticField::count("backend_generation", 1),
                DiagnosticField::boolean("pos_present", true),
                DiagnosticField::count("rooms_from_response", 31),
            ],
        ),
        handoff(healthy, "reconcile", "entered"),
        handoff(healthy, "reconcile", "completed"),
        handoff(healthy, "handoff", "entered"),
        handoff(healthy, "handoff", "completed"),
        // The run that stalled: one committed response, and its reconcile round
        // trip was entered and never came back.
        (
            "status_projected",
            vec![
                owner_field(stuck),
                DiagnosticField::count("generation", 1),
                DiagnosticField::token("lifecycle", "starting"),
            ],
        ),
        (
            "committed_response",
            vec![
                owner_field(stuck),
                DiagnosticField::count("sequence", 3),
                DiagnosticField::count("backend_generation", 1),
                DiagnosticField::boolean("pos_present", true),
                DiagnosticField::count("rooms_from_response", 2),
            ],
        ),
        handoff(stuck, "refresh", "entered"),
        handoff(stuck, "refresh", "completed"),
        handoff(stuck, "timeline", "entered"),
        handoff(stuck, "timeline", "completed"),
        handoff(stuck, "reconcile", "entered"),
    ]);

    let summary = super::sync_diagnostic_summary(&snapshot, stuck);

    for expected in [
        "sync_diag_scope=run",
        "sync_diag_run_generation=1",
        "sync_diag_status_projected=1",
        "sync_diag_last_lifecycle=starting",
        "sync_diag_committed_response=1",
        "sync_diag_last_committed_sequence=3",
        "sync_diag_last_rooms_from_response=2",
        "sync_diag_await_refresh=completed",
        "sync_diag_await_timeline=completed",
        "sync_diag_await_reconcile=pending",
        "sync_diag_await_handoff=none",
    ] {
        assert!(
            summary.contains(expected),
            "expected `{expected}` in `{summary}`"
        );
    }
    for excluded in [
        "sync_diag_last_committed_sequence=899",
        "sync_diag_last_rooms_from_response=31",
        "sync_diag_last_lifecycle=running",
        "sync_diag_await_reconcile=completed",
    ] {
        assert!(
            !summary.contains(excluded),
            "did not expect `{excluded}` in `{summary}`"
        );
    }
    assert!(summary.contains(&format!("sync_diag_owner={}", stuck.get())));
}

/// The stuck run is the newest run of its runtime, so an older run's counts must
/// not leak into the summary through the shared owner.
#[test]
fn sync_diagnostic_summary_scopes_to_the_newest_run_of_the_owner() {
    use koushi_diagnostics::DiagnosticField;

    let owner = test_diagnostics_owner();
    let snapshot = sync_snapshot(vec![
        (
            "committed_response",
            vec![
                owner_field(owner),
                DiagnosticField::count("sequence", 412),
                DiagnosticField::count("backend_generation", 1),
                DiagnosticField::boolean("pos_present", true),
                DiagnosticField::count("rooms_from_response", 31),
            ],
        ),
        (
            "handoff_step",
            vec![
                owner_field(owner),
                DiagnosticField::token("step", "reconcile"),
                DiagnosticField::token("phase", "completed"),
                DiagnosticField::count("backend_generation", 1),
            ],
        ),
        // A later start (the stuck one) with a single commit and no handoff step.
        (
            "committed_response",
            vec![
                owner_field(owner),
                DiagnosticField::count("sequence", 1),
                DiagnosticField::count("backend_generation", 2),
                DiagnosticField::boolean("pos_present", true),
                DiagnosticField::count("rooms_from_response", 0),
            ],
        ),
    ]);

    let summary = super::sync_diagnostic_summary(&snapshot, owner);

    for expected in [
        "sync_diag_scope=run",
        "sync_diag_run_generation=2",
        "sync_diag_committed_response=1",
        "sync_diag_last_committed_sequence=1",
        "sync_diag_await_reconcile=none",
    ] {
        assert!(
            summary.contains(expected),
            "expected `{expected}` in `{summary}`"
        );
    }
    assert!(!summary.contains("sync_diag_last_committed_sequence=412"));
}
