use koushi_protocol::SessionKeyId;

use std::time::Duration;

use matrix_sdk::ruma::api::error::ErrorKind;
use tokio::sync::oneshot;

use super::{classify_room_event_lookup_error, composer_timeline_command_targets_active_session};
use crate::account::RoomEventLookupResult;
use crate::account::actor::AccountMessage;
use crate::account::test_support::spawn_actor_with_dirs;
use crate::executor;
use koushi_protocol::command::TimelineCommand;

use koushi_protocol::ids::{AccountKey, RequestId, RuntimeConnectionId, TimelineKey};

use tempfile::tempdir;

#[test]
fn composer_timeline_command_rechecks_full_session_owner_before_account_routing() {
    let active = SessionKeyId {
        homeserver: "https://active.example.test".to_owned(),
        user_id: "@same-user:example.test".to_owned(),
        device_id: "ACTIVE".to_owned(),
    };
    let stale = SessionKeyId {
        homeserver: "https://stale.example.test".to_owned(),
        user_id: active.user_id.clone(),
        device_id: "STALE".to_owned(),
    };
    let command = TimelineCommand::SubmitText {
        request_id: RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: 1,
        },
        expected_account: stale.clone(),
        submission_id: koushi_state::SubmissionId::new("submission-owner-fence"),
        key: TimelineKey::room(AccountKey(active.user_id.clone()), "!room:example.test"),
        transaction_id: "transaction-owner-fence".to_owned(),
        document: koushi_state::ComposerDocument::from_plain_text("synthetic body"),
        draft_revision: 1.into(),
    };

    assert!(!composer_timeline_command_targets_active_session(
        Some(&active),
        &command
    ));
    assert!(composer_timeline_command_targets_active_session(
        Some(&stale),
        &command
    ));
}

#[tokio::test]
async fn pending_event_cache_fetch_times_out_and_releases_account_actor() {
    let cred_dir = tempdir().expect("credential tempdir");
    let data_dir = tempdir().expect("data tempdir");
    let (handle, _action_rx, _event_rx) =
        crate::account::test_support::spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    let (_fetch_tx, fetch_rx) = oneshot::channel::<RoomEventLookupResult>();
    assert!(
        handle
            .send(AccountMessage::ConfigureEventCacheFetchForTesting { fetch: fetch_rx })
            .await
    );

    let request_id = RequestId {
        connection_id: RuntimeConnectionId(7),
        sequence: 23,
    };
    let (response_tx, response_rx) = oneshot::channel();
    assert!(
        handle
            .send(AccountMessage::EnsureRoomEventCached {
                request_id,
                room_id: "!synthetic-room:example.invalid".to_owned(),
                event_id: "$synthetic-event:example.invalid".to_owned(),
                response_tx,
            })
            .await
    );
    let (acknowledged, completion) = oneshot::channel();
    assert!(
        handle
            .send(AccountMessage::ShutdownWithAck { acknowledged })
            .await
    );

    assert_eq!(
        executor::timeout(Duration::from_secs(1), response_rx)
            .await
            .expect("bounded event-cache response")
            .expect("event-cache response channel"),
        RoomEventLookupResult::Failed
    );
    executor::timeout(Duration::from_secs(1), completion)
        .await
        .expect("account actor should process the following shutdown")
        .expect("account shutdown acknowledgement");
}

#[test]
fn event_cache_repair_diagnostic_runs_without_trace_environment() {
    let child = std::process::Command::new(
        std::env::current_exe().expect("current test executable should be available"),
    )
    .arg("--exact")
    .arg(concat!(
        "account::routing::tests::",
        "event_cache_repair_diagnostic_records_without_trace_environment"
    ))
    .arg("--ignored")
    .arg("--nocapture")
    .env_remove("KOUSHI_TIMELINE_ITEM_TRACE")
    .env_remove("KOUSHI_SUBSCRIBE_TRACE")
    .status()
    .expect("env-unset event-cache-repair child should start");
    assert!(
        child.success(),
        "env-unset event-cache-repair child failed: {child}"
    );
}

#[tokio::test]
#[ignore]
async fn event_cache_repair_diagnostic_records_without_trace_environment() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    assert!(std::env::var_os("KOUSHI_TIMELINE_ITEM_TRACE").is_none());
    assert!(std::env::var_os("KOUSHI_SUBSCRIBE_TRACE").is_none());

    let cred_dir = tempdir().expect("credential tempdir");
    let data_dir = tempdir().expect("data tempdir");
    let (handle, _action_rx, _event_rx) = spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    let synthetic_room_id = "!synthetic-room:example.invalid";
    let synthetic_event_id = "$synthetic-event:example.invalid";
    let request_id = RequestId {
        connection_id: RuntimeConnectionId(17),
        sequence: 23,
    };
    let (response_tx, response_rx) = oneshot::channel();
    assert!(
        handle
            .send(AccountMessage::EnsureRoomEventCached {
                request_id,
                room_id: synthetic_room_id.to_owned(),
                event_id: synthetic_event_id.to_owned(),
                response_tx,
            })
            .await
    );
    assert_eq!(
        response_rx.await.expect("cache-repair response"),
        RoomEventLookupResult::Failed,
        "a cache miss must not be reported as successful"
    );

    let records = koushi_diagnostics::test_support::detail_snapshot().records;
    let repair = records
        .iter()
        .rev()
        .find(|record| {
            record.event.source == "core.event_cache_repair"
                && record.event.stage == "failed"
                && record.event.fields.iter().any(|field| {
                    field.key == "reason"
                        && field.value == koushi_diagnostics::DiagnosticValue::Token("no_session")
                })
        })
        .expect("event-cache repair should be collected without trace environment");
    assert_eq!(repair.event.source, "core.event_cache_repair");
    assert_eq!(repair.event.stage, "failed");
    assert_eq!(
        repair.event.fields,
        vec![
            koushi_diagnostics::DiagnosticField::request_id("request_id", 17, 23),
            koushi_diagnostics::DiagnosticField::token("outcome", "failed"),
            koushi_diagnostics::DiagnosticField::token("reason", "no_session"),
        ]
    );

    let serialized = serde_json::to_string(&repair.event)
        .expect("event-cache repair event should serialize for privacy assertions");
    for forbidden in [
        synthetic_room_id,
        synthetic_event_id,
        "synthetic-body-value",
        "https://example.invalid/synthetic",
        "/tmp/synthetic-path",
        "raw sdk error: synthetic",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "serialized event must not contain forbidden diagnostic data: {forbidden}"
        );
    }
}

#[test]
fn classify_room_event_lookup_error_only_treats_not_found_as_missing() {
    assert_eq!(
        classify_room_event_lookup_error(Some(&ErrorKind::NotFound)),
        RoomEventLookupResult::Missing
    );
    assert_eq!(
        classify_room_event_lookup_error(Some(&ErrorKind::Forbidden)),
        RoomEventLookupResult::Failed
    );
    assert_eq!(
        classify_room_event_lookup_error(None),
        RoomEventLookupResult::Failed
    );
}
