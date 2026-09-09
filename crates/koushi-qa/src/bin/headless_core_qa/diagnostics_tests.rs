use super::{
    QaMessagesProxyDecision, QaMessagesProxyExpectation, QaMessagesProxyState,
    QaRoomMessagesRequestMetadata, invite_observer_diagnostic_summary,
    qa_room_messages_request_metadata, rewrite_http_request_connection_close,
    send_lifecycle_diagnostic_summary, trust_admission_diagnostic_summary,
};

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
