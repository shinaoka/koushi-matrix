use super::{
    BodyWaitObserver, InitialItemsWaitMatch, PairedEventWaitError, SendFlowWaiter,
    WithheldEventProjectionOrigin, find_timeline_item_with_body, match_initial_items_wait_event,
    projection_timeline_item, visit_timeline_diff_items, wait_for_initial_items_from_source,
    wait_for_logged_in, wait_for_logged_out, wait_for_operation_failed,
    wait_for_paired_event_until, wait_for_session_restored,
    wait_for_withheld_event_projection_from_source,
};
use crate::contracts::{
    IntervalQaEventSource, IntervalQaSnapshotEventSource, ScriptedQaEventSource,
    ScriptedQaSnapshotEventSource, SharedSnapshotPendingEventSource, qa_logged_out_event,
    qa_state_with_session, synthetic_timeline_item, withheld_projection_items_updated,
    withheld_projection_test_item,
};
use crate::registry::{EVENT_TIMEOUT, LOGIN_EVENT_TIMEOUT};
use crate::{
    AccountEvent, AccountKey, Arc, CoreEvent, CoreFailure, Duration, Mutex, RequestId, SessionInfo,
    SessionState, SyncEvent, TimelineDiff, TimelineEvent, TimelineKey, TimelineMessageActions,
    TimelineSendState,
};

#[test]
fn diff_item_visitor_scans_set_and_reset_items() {
    let set_item = synthetic_timeline_item("$root:test", Some("root"), None, None, None);
    let reset_item = synthetic_timeline_item(
        "$reply:test",
        Some("Phase 11 QA thread reply from B"),
        Some("$root:test"),
        Some("$root:test"),
        None,
    );
    let diffs = vec![
        TimelineDiff::Set {
            index: 0,
            item: set_item,
        },
        TimelineDiff::Reset {
            items: vec![reset_item],
        },
    ];
    let mut bodies = Vec::new();

    visit_timeline_diff_items(&diffs, |item| {
        if let Some(body) = item.body.as_deref() {
            bodies.push(body.to_owned());
        }
        Ok(())
    })
    .unwrap();

    assert_eq!(bodies, ["root", "Phase 11 QA thread reply from B"]);
}

#[test]
fn body_wait_observer_tolerates_transient_decryption_failure_before_expected_body() {
    let mut observer = BodyWaitObserver::new("delivered encrypted body");
    let utd = synthetic_timeline_item(
        "$utd:test",
        Some("Unable to decrypt message"),
        None,
        None,
        None,
    );
    let delivered = synthetic_timeline_item(
        "$delivered:test",
        Some("later delivered encrypted body"),
        None,
        None,
        None,
    );

    assert!(observer.observe_items(&[utd]).is_none());
    assert!(observer.saw_decryption_failure);
    assert!(
        observer
            .timeout_message("strict receive")
            .contains("transient undecryptable")
    );

    let found = observer
        .observe_diffs(&[TimelineDiff::Set {
            index: 0,
            item: delivered,
        }])
        .unwrap()
        .expect("expected body should still succeed after transient UTD");

    assert_eq!(
        found.body.as_deref(),
        Some("later delivered encrypted body")
    );
}

#[test]
fn find_timeline_item_with_body_finds_thread_reply_in_one_batch() {
    let items = vec![koushi_protocol::event::TimelineItem {
        request_state: None,
        id: koushi_protocol::event::TimelineItemId::Synthetic {
            synthetic_id: "thread-reply".to_owned(),
        },
        sender: Some("@b:test".to_owned()),
        sender_label: None,
        sender_avatar: None,
        body: Some("Phase 5 QA thread reply from B".to_owned()),
        notice_i18n: None,
        message_kind: Default::default(),
        spoiler_spans: Vec::new(),
        timestamp_ms: None,
        in_reply_to_event_id: Some("$root:test".to_owned()),
        formatted: None,
        reply_quote: None,
        thread_root: None,
        thread_summary: None,
        media: None,
        link_previews: None,
        link_ranges: Vec::new(),
        mentioned_user_ids: Vec::new(),
        reactions: Vec::new(),
        can_react: false,
        is_redacted: false,
        is_hidden: false,
        can_redact: false,
        is_edited: false,
        can_edit: false,
        actions: TimelineMessageActions::default(),
        send_state: None,
        display_metadata: None,
        unable_to_decrypt: None,
    }];

    assert_eq!(
        find_timeline_item_with_body(&items, "thread reply from B")
            .as_ref()
            .and_then(|item| item.body.as_deref()),
        Some("Phase 5 QA thread reply from B")
    );
}

#[test]
fn find_timeline_item_with_body_returns_none_when_missing() {
    let items = vec![koushi_protocol::event::TimelineItem {
        request_state: None,
        id: koushi_protocol::event::TimelineItemId::Synthetic {
            synthetic_id: "placeholder".to_owned(),
        },
        sender: None,
        sender_label: None,
        sender_avatar: None,
        body: Some("Phase 5 QA message 1".to_owned()),
        notice_i18n: None,
        message_kind: Default::default(),
        spoiler_spans: Vec::new(),
        timestamp_ms: None,
        in_reply_to_event_id: None,
        formatted: None,
        reply_quote: None,
        thread_root: None,
        thread_summary: None,
        media: None,
        link_previews: None,
        link_ranges: Vec::new(),
        mentioned_user_ids: Vec::new(),
        reactions: Vec::new(),
        can_react: false,
        is_redacted: false,
        is_hidden: false,
        can_redact: false,
        is_edited: false,
        can_edit: false,
        actions: TimelineMessageActions::default(),
        send_state: None,
        display_metadata: None,
        unable_to_decrypt: None,
    }];

    assert!(find_timeline_item_with_body(&items, "thread reply from B").is_none());
}

#[test]
fn send_flow_waiter_accepts_send_completed_before_local_echo() {
    let key = TimelineKey::room(
        AccountKey("@alice:test".to_owned()),
        "!room:test".to_owned(),
    );
    let request_id = koushi_protocol::ids::RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
        sequence: 1,
    };
    let mut waiter = SendFlowWaiter::new(
        request_id,
        key.clone(),
        "qa-phase5-txn-1",
        "Phase 5 QA message 1",
    );

    assert!(!waiter.is_complete());
    waiter
        .observe(CoreEvent::Timeline(TimelineEvent::SendCompleted {
            request_id,
            key: key.clone(),
            transaction_id: "qa-phase5-txn-1".to_owned(),
            event_id: "$event:test".to_owned(),
        }))
        .unwrap();
    assert!(!waiter.is_complete());

    waiter
        .observe(CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
            key: key.clone(),
            generation: koushi_protocol::ids::TimelineGeneration(1),
            batch_id: koushi_protocol::ids::TimelineBatchId(1),
            diffs: vec![koushi_protocol::event::TimelineDiff::PushBack {
                item: koushi_protocol::event::TimelineItem {
                    request_state: None,
                    id: koushi_protocol::event::TimelineItemId::Transaction {
                        transaction_id: "sdk-txn-1".to_owned(),
                    },
                    sender: Some("@alice:test".to_owned()),
                    sender_label: None,
                    sender_avatar: None,
                    body: Some("Phase 5 QA message 1".to_owned()),
                    notice_i18n: None,
                    message_kind: Default::default(),
                    spoiler_spans: Vec::new(),
                    timestamp_ms: None,
                    in_reply_to_event_id: None,
                    formatted: None,
                    reply_quote: None,
                    thread_root: None,
                    thread_summary: None,
                    media: None,
                    link_previews: None,
                    link_ranges: Vec::new(),
                    mentioned_user_ids: Vec::new(),
                    reactions: Vec::new(),
                    can_react: false,
                    is_redacted: false,
                    is_hidden: false,
                    can_redact: false,
                    is_edited: false,
                    can_edit: false,
                    actions: TimelineMessageActions::default(),
                    send_state: None,
                    display_metadata: None,
                    unable_to_decrypt: None,
                },
            }],
        }))
        .unwrap();

    let result = waiter.finish().unwrap();
    assert_eq!(result.sdk_transaction_id, "sdk-txn-1");
    assert_eq!(result.send_transaction_id, "qa-phase5-txn-1");
    assert_eq!(result.event_id, "$event:test");
}

#[test]
fn send_flow_waiter_status_reports_local_echo_send_state() {
    let key = TimelineKey::room(
        AccountKey("@alice:test".to_owned()),
        "!room:test".to_owned(),
    );
    let request_id = koushi_protocol::ids::RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
        sequence: 1,
    };
    let mut waiter = SendFlowWaiter::new(
        request_id,
        key.clone(),
        "qa-phase5-txn-1",
        "Phase 5 QA message 1",
    );

    waiter
        .observe(CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
            key,
            generation: koushi_protocol::ids::TimelineGeneration(1),
            batch_id: koushi_protocol::ids::TimelineBatchId(1),
            diffs: vec![koushi_protocol::event::TimelineDiff::PushBack {
                item: koushi_protocol::event::TimelineItem {
                    request_state: None,
                    id: koushi_protocol::event::TimelineItemId::Transaction {
                        transaction_id: "sdk-txn-1".to_owned(),
                    },
                    sender: Some("@alice:test".to_owned()),
                    sender_label: None,
                    sender_avatar: None,
                    body: Some("Phase 5 QA message 1".to_owned()),
                    notice_i18n: None,
                    message_kind: Default::default(),
                    spoiler_spans: Vec::new(),
                    timestamp_ms: None,
                    in_reply_to_event_id: None,
                    formatted: None,
                    reply_quote: None,
                    thread_root: None,
                    thread_summary: None,
                    media: None,
                    link_previews: None,
                    link_ranges: Vec::new(),
                    mentioned_user_ids: Vec::new(),
                    reactions: Vec::new(),
                    can_react: false,
                    is_redacted: false,
                    is_hidden: false,
                    can_redact: false,
                    is_edited: false,
                    can_edit: false,
                    actions: TimelineMessageActions::default(),
                    send_state: Some(TimelineSendState::Sending),
                    display_metadata: None,
                    unable_to_decrypt: None,
                },
            }],
        }))
        .unwrap();

    assert!(
        waiter
            .status_summary()
            .contains("local_echo_send_state=Sending")
    );
}

#[test]
fn send_flow_waiter_errors_when_local_echo_becomes_not_sent() {
    let key = TimelineKey::room(
        AccountKey("@alice:test".to_owned()),
        "!room:test".to_owned(),
    );
    let request_id = koushi_protocol::ids::RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
        sequence: 1,
    };
    let mut waiter = SendFlowWaiter::new(
        request_id,
        key.clone(),
        "qa-phase5-txn-1",
        "Phase 5 QA message 1",
    );

    let err = waiter
        .observe(CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
            key,
            generation: koushi_protocol::ids::TimelineGeneration(1),
            batch_id: koushi_protocol::ids::TimelineBatchId(1),
            diffs: vec![koushi_protocol::event::TimelineDiff::PushBack {
                item: koushi_protocol::event::TimelineItem {
                    request_state: None,
                    id: koushi_protocol::event::TimelineItemId::Transaction {
                        transaction_id: "sdk-txn-1".to_owned(),
                    },
                    sender: Some("@alice:test".to_owned()),
                    sender_label: None,
                    sender_avatar: None,
                    body: Some("Phase 5 QA message 1".to_owned()),
                    notice_i18n: None,
                    message_kind: Default::default(),
                    spoiler_spans: Vec::new(),
                    timestamp_ms: None,
                    in_reply_to_event_id: None,
                    formatted: None,
                    reply_quote: None,
                    thread_root: None,
                    thread_summary: None,
                    media: None,
                    link_previews: None,
                    link_ranges: Vec::new(),
                    mentioned_user_ids: Vec::new(),
                    reactions: Vec::new(),
                    can_react: false,
                    is_redacted: false,
                    is_hidden: false,
                    can_redact: false,
                    is_edited: false,
                    can_edit: false,
                    actions: TimelineMessageActions::default(),
                    send_state: Some(TimelineSendState::NotSent {
                        reason: koushi_protocol::event::TimelineSendFailureReason::Recoverable,
                    }),
                    unable_to_decrypt: None,
                    display_metadata: None,
                },
            }],
        }))
        .unwrap_err();

    assert!(err.contains("send flow failed"));
    assert!(err.contains("local_echo_send_state=NotSent(recoverable)"));
}

#[test]
fn initial_items_wait_requires_exact_subscribe_cause_even_for_same_key_replays() {
    let request_id = RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
        sequence: 2,
    };
    let old_request_id = RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
        sequence: 1,
    };
    let wrong_connection_request_id = RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(2),
        sequence: request_id.sequence,
    };
    let key = TimelineKey::room(AccountKey("@qa:example.invalid".to_owned()), "!room:test");
    let wrong_key = TimelineKey::room(AccountKey("@qa:example.invalid".to_owned()), "!other:test");
    let initial = |projection_request_id, cause_request_id, key: TimelineKey| {
        CoreEvent::Timeline(TimelineEvent::InitialItems {
            request_id: projection_request_id,
            cause_request_id,
            key,
            actor_generation: 1,
            generation: koushi_protocol::ids::TimelineGeneration(0),
            items: Vec::new(),
        })
    };
    let classify = |event| match_initial_items_wait_event(event, &key, request_id);

    assert!(matches!(
        classify(initial(Some(old_request_id), Some(request_id), key.clone())),
        InitialItemsWaitMatch::Items(_)
    ));
    assert!(matches!(
        classify(initial(None, Some(request_id), key.clone())),
        InitialItemsWaitMatch::Items(_)
    ));
    assert!(matches!(
        classify(initial(Some(request_id), Some(old_request_id), key.clone())),
        InitialItemsWaitMatch::Ignore
    ));
    assert!(matches!(
        classify(initial(Some(request_id), None, key.clone())),
        InitialItemsWaitMatch::Ignore
    ));
    assert!(matches!(
        classify(initial(Some(old_request_id), Some(request_id), wrong_key)),
        InitialItemsWaitMatch::Ignore
    ));

    assert!(matches!(
        classify(CoreEvent::OperationFailed {
            request_id,
            failure: CoreFailure::SessionRequired,
        }),
        InitialItemsWaitMatch::Failure(CoreFailure::SessionRequired)
    ));
    assert!(matches!(
        classify(CoreEvent::OperationFailed {
            request_id: old_request_id,
            failure: CoreFailure::SessionRequired,
        }),
        InitialItemsWaitMatch::Ignore
    ));
    assert!(matches!(
        classify(CoreEvent::OperationFailed {
            request_id: wrong_connection_request_id,
            failure: CoreFailure::SessionRequired,
        }),
        InitialItemsWaitMatch::Ignore
    ));
}

#[tokio::test]
async fn withheld_projection_wait_accepts_decryption_failure_from_late_items_updated() {
    let key = TimelineKey::room(AccountKey("@qa:example.invalid".to_owned()), "!room:test");
    let target_event_id = "$withheld:test";
    let mut source = ScriptedQaEventSource {
        events: [
            CoreEvent::Sync(SyncEvent::Running),
            withheld_projection_items_updated(
                key.clone(),
                withheld_projection_test_item(target_event_id, "Unable to decrypt"),
            ),
        ]
        .into(),
    };

    let origin = wait_for_withheld_event_projection_from_source(
        &mut source,
        &key,
        target_event_id,
        "blocked body",
        &[],
        "withheld projection regression",
        Duration::from_secs(1),
    )
    .await
    .expect("late decryption-failure projection should satisfy the waiter");

    assert_eq!(origin, WithheldEventProjectionOrigin::ItemsUpdated);
}

#[tokio::test(start_paused = true)]
async fn withheld_projection_wait_reports_private_safe_missing_category_at_deadline() {
    let key = TimelineKey::room(AccountKey("@qa:example.invalid".to_owned()), "!room:test");
    let target_event_id = "$withheld:test";
    let mut source = ScriptedQaEventSource {
        events: Default::default(),
    };

    let error = wait_for_withheld_event_projection_from_source(
        &mut source,
        &key,
        target_event_id,
        "blocked body",
        &[],
        "withheld projection regression",
        Duration::from_secs(1),
    )
    .await
    .expect_err("an absent canonical event should time out as missing");

    assert!(error.contains("projection_origin=missing"));
    assert!(!error.contains(target_event_id));
    assert!(!error.contains("@qa:"));
    assert!(!error.contains("!room:"));
}

#[tokio::test]
async fn withheld_projection_wait_rejects_plaintext_without_exposing_it() {
    let key = TimelineKey::room(AccountKey("@qa:example.invalid".to_owned()), "!room:test");
    let target_event_id = "$withheld:test";
    let private_body = "private withheld body";
    let initial_items = vec![withheld_projection_test_item(target_event_id, private_body)];
    let mut source = ScriptedQaEventSource {
        events: Default::default(),
    };

    let error = wait_for_withheld_event_projection_from_source(
        &mut source,
        &key,
        target_event_id,
        private_body,
        &initial_items,
        "withheld projection regression",
        Duration::from_secs(1),
    )
    .await
    .expect_err("plaintext projection must fail closed");

    assert!(error.contains("projection_outcome=non_failure"));
    assert!(error.contains("matches_expected_body=true"));
    assert!(!error.contains(target_event_id));
    assert!(!error.contains(private_body));
}

#[tokio::test]
async fn paired_verification_wait_wakes_from_either_event_source() {
    let mut primary = ScriptedQaEventSource {
        events: Default::default(),
    };
    let mut secondary = ScriptedQaEventSource {
        events: [CoreEvent::Sync(SyncEvent::Running)].into(),
    };

    assert_eq!(
        wait_for_paired_event_until(
            &mut primary,
            &mut secondary,
            tokio::time::Instant::now() + Duration::from_secs(10),
        )
        .await,
        Ok(())
    );
}

#[tokio::test(start_paused = true)]
async fn paired_verification_wait_uses_one_absolute_deadline() {
    let mut primary = ScriptedQaEventSource {
        events: Default::default(),
    };
    let mut secondary = ScriptedQaEventSource {
        events: Default::default(),
    };
    let started_at = tokio::time::Instant::now();
    let deadline = started_at + Duration::from_secs(7);

    assert_eq!(
        wait_for_paired_event_until(&mut primary, &mut secondary, deadline).await,
        Err(PairedEventWaitError::Deadline)
    );
    assert_eq!(
        tokio::time::Instant::now().duration_since(started_at),
        Duration::from_secs(7)
    );
}

#[tokio::test(start_paused = true)]
async fn login_wait_observes_ready_snapshot_once_at_deadline_without_a_broadcast() {
    let shared = Arc::new(Mutex::new(qa_state_with_session(SessionState::SignedOut)));
    let mut source = SharedSnapshotPendingEventSource {
        snapshot: shared.clone(),
    };
    let ready_shared = shared.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(1)).await;
        ready_shared
            .lock()
            .expect("shared QA snapshot lock should not be poisoned")
            .session = SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@ready:example.invalid".to_owned(),
            device_id: "READYDEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        });
    });
    let started_at = tokio::time::Instant::now();

    let account_key = wait_for_logged_in(
        &mut source,
        RequestId {
            connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
            sequence: 1,
        },
        "login final snapshot",
    )
    .await
    .expect("the final authoritative Ready snapshot should complete login");

    assert_eq!(account_key, AccountKey("@ready:example.invalid".to_owned()));
    assert_eq!(
        tokio::time::Instant::now().duration_since(started_at),
        LOGIN_EVENT_TIMEOUT
    );
}

#[tokio::test(start_paused = true)]
async fn login_wait_without_event_or_ready_snapshot_still_times_out() {
    let shared = Arc::new(Mutex::new(qa_state_with_session(SessionState::SignedOut)));
    let mut source = SharedSnapshotPendingEventSource { snapshot: shared };
    let started_at = tokio::time::Instant::now();

    let error = wait_for_logged_in(
        &mut source,
        RequestId {
            connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
            sequence: 2,
        },
        "login remains pending",
    )
    .await
    .expect_err("a non-Ready snapshot must retain the login timeout");

    // The phase token is part of the contract now (#375): it is what makes
    // one failed CI capture diagnosable.
    assert!(
        error.starts_with(
            "login remains pending: timed out waiting for LoggedIn event; phase=signed_out; trust_path="
        ),
        "unexpected timeout diagnostic: {error}"
    );
    assert_eq!(
        tokio::time::Instant::now().duration_since(started_at),
        LOGIN_EVENT_TIMEOUT
    );
}

#[tokio::test]
async fn session_restored_account_mismatch_is_private_safe() {
    let request_id = RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
        sequence: 6,
    };
    let expected = AccountKey("@expected:example.invalid".to_owned());
    let mut source = ScriptedQaSnapshotEventSource {
        events: [(
            CoreEvent::Account(AccountEvent::SessionRestored {
                request_id,
                account_key: AccountKey("@unexpected:example.invalid".to_owned()),
            }),
            SessionState::SignedOut,
        )]
        .into(),
        snapshot: qa_state_with_session(SessionState::SignedOut),
        received: 0,
    };

    let error = wait_for_session_restored(&mut source, request_id, &expected, "restore mismatch")
        .await
        .expect_err("wrong restored account must fail immediately");
    assert!(error.contains("account_key mismatch"));
    assert!(!error.contains('@'));
    assert_eq!(source.received, 1);
}

#[tokio::test(start_paused = true)]
async fn logout_and_operation_failed_deadlines_survive_unrelated_event_starvation() {
    let request_id = RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
        sequence: 9,
    };
    let account_key = AccountKey("@deadline:example.invalid".to_owned());
    let mut logout_source = IntervalQaSnapshotEventSource {
        interval: tokio::time::interval(Duration::from_secs(1)),
        snapshot: qa_state_with_session(SessionState::LoggingOut),
        first_event: Some(qa_logged_out_event(request_id, account_key.clone())),
    };
    let logout_started_at = tokio::time::Instant::now();
    wait_for_logged_out(
        &mut logout_source,
        request_id,
        &account_key,
        "logout deadline",
    )
    .await
    .expect_err("a LoggedOut event without SignedOut state must time out");
    assert_eq!(
        tokio::time::Instant::now().duration_since(logout_started_at),
        EVENT_TIMEOUT
    );

    let mut failure_source = IntervalQaEventSource {
        interval: tokio::time::interval(Duration::from_secs(1)),
    };
    let failure_started_at = tokio::time::Instant::now();
    wait_for_operation_failed(&mut failure_source, request_id, "failure deadline")
        .await
        .expect_err("unrelated events must not restart the failure deadline");
    assert_eq!(
        tokio::time::Instant::now().duration_since(failure_started_at),
        EVENT_TIMEOUT
    );
}

#[tokio::test(start_paused = true)]
async fn initial_items_wait_deadline_is_not_extended_by_continuous_unrelated_events() {
    let request_id = RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
        sequence: 2,
    };
    let key = TimelineKey::room(AccountKey("@qa:example.invalid".to_owned()), "!room:test");
    let mut source = IntervalQaEventSource {
        interval: tokio::time::interval(Duration::from_secs(1)),
    };
    let started_at = tokio::time::Instant::now();

    let result = tokio::time::timeout(
        Duration::from_secs(30),
        wait_for_initial_items_from_source(
            &mut source,
            &key,
            request_id,
            "deadline starvation regression",
            Duration::from_secs(10),
        ),
    )
    .await
    .expect("the absolute waiter must finish before the outer starvation guard");

    let error = result.expect_err("unrelated events must not satisfy the causal wait");
    assert!(error.contains("timed out waiting for TimelineEvent::InitialItems"));
    assert!(error.contains("same_key_wrong_cause=0"));
    assert!(error.contains("same_key_causeless=0"));
    assert!(error.contains("unrelated_events="));
    assert_eq!(
        tokio::time::Instant::now().duration_since(started_at),
        Duration::from_secs(10),
        "unrelated events must not restart the ten-second budget"
    );
}

#[tokio::test]
async fn initial_items_wait_skips_fresh_wrong_cause_then_accepts_exact_replay_cause() {
    let old_projection_request_id = RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
        sequence: 1,
    };
    let subscribe_request_id = RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
        sequence: 2,
    };
    let key = TimelineKey::room(AccountKey("@qa:example.invalid".to_owned()), "!room:test");
    let event = |cause_request_id, items| {
        CoreEvent::Timeline(TimelineEvent::InitialItems {
            request_id: Some(old_projection_request_id),
            cause_request_id: Some(cause_request_id),
            key: key.clone(),
            actor_generation: 1,
            generation: koushi_protocol::ids::TimelineGeneration(0),
            items,
        })
    };
    let mut source = ScriptedQaEventSource {
        events: [
            event(old_projection_request_id, Vec::new()),
            event(
                subscribe_request_id,
                vec![projection_timeline_item("$exact-replay:test", false)],
            ),
        ]
        .into(),
    };

    let items = wait_for_initial_items_from_source(
        &mut source,
        &key,
        subscribe_request_id,
        "causal replay regression",
        Duration::from_secs(1),
    )
    .await
    .expect("the exact idempotent replay cause should satisfy the waiter");

    assert_eq!(items.len(), 1, "the wrong-cause fresh event was ignored");
}

#[tokio::test(start_paused = true)]
async fn initial_items_timeout_reports_only_private_safe_causal_category_counts() {
    let old_request_id = RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
        sequence: 1,
    };
    let request_id = RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
        sequence: 2,
    };
    let key = TimelineKey::room(AccountKey("@qa:example.invalid".to_owned()), "!room:test");
    let wrong_key = TimelineKey::room(AccountKey("@qa:example.invalid".to_owned()), "!other:test");
    let initial = |event_key, cause_request_id| {
        CoreEvent::Timeline(TimelineEvent::InitialItems {
            request_id: Some(old_request_id),
            cause_request_id,
            key: event_key,
            actor_generation: 1,
            generation: koushi_protocol::ids::TimelineGeneration(0),
            items: Vec::new(),
        })
    };
    let mut source = ScriptedQaEventSource {
        events: [
            initial(key.clone(), Some(old_request_id)),
            initial(key.clone(), None),
            initial(wrong_key, Some(request_id)),
            CoreEvent::Sync(SyncEvent::Running),
        ]
        .into(),
    };

    let error = wait_for_initial_items_from_source(
        &mut source,
        &key,
        request_id,
        "causal categories regression",
        Duration::from_secs(1),
    )
    .await
    .expect_err("no exact-cause event was supplied");

    assert!(error.contains("same_key_exact_cause=0"));
    assert!(error.contains("same_key_wrong_cause=1"));
    assert!(error.contains("same_key_causeless=1"));
    assert!(error.contains("wrong_key_initial_items=1"));
    assert!(error.contains("unrelated_events=1"));
    assert!(!error.contains("@qa:"));
    assert!(!error.contains("!room:"));
}
