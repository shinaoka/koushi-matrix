use std::time::Duration;

use super::*;
use koushi_protocol::command::SearchScope;
use koushi_protocol::ids::{RequestId, RuntimeConnectionId};
use koushi_search::{
    SearchCandidate, SearchDocumentStore, SearchEdit, SearchableEvent, SensitiveString,
};

#[tokio::test]
async fn search_actor_shutdown_waits_for_actor_task_settlement() {
    let (tx, mut rx) = mpsc::channel(1);
    let (index_tx, _index_rx) = mpsc::channel(1);
    let (settled_tx, settled_rx) = tokio::sync::oneshot::channel::<()>();
    let task = executor::spawn(async move {
        let _settled = settled_tx;
        let _ = rx.recv().await;
        std::future::pending::<()>().await;
    });
    let handle = SearchActorHandle {
        tx,
        index_tx,
        task: Some(task),
    };

    handle
        .shutdown_with_timeouts(Duration::from_millis(100), Duration::from_millis(10))
        .await;

    let _ = executor::timeout(Duration::from_millis(100), settled_rx)
        .await
        .expect("shutdown must await actor task settlement");
}

#[test]
fn search_producer_records_typed_start_fields_without_environment_switch() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    trace_search_start(
        RequestId {
            connection_id: RuntimeConnectionId(8),
            sequence: 13,
        },
        &SearchScope::AllRooms,
        5,
        9,
        3,
        2,
        true,
    );
    let record = koushi_diagnostics::snapshot()
        .records
        .into_iter()
        .rev()
        .find(|record| record.event.source == "core.search" && record.event.stage == "start")
        .expect("search producer should record");
    assert_eq!(
        record
            .event
            .fields
            .iter()
            .map(|field| (field.key, field.value.clone()))
            .collect::<Vec<_>>(),
        vec![
            (
                "request_id",
                koushi_diagnostics::DiagnosticValue::RequestId {
                    connection_id: 8,
                    sequence: 13,
                },
            ),
            (
                "scope",
                koushi_diagnostics::DiagnosticValue::Token("all_rooms"),
            ),
            (
                "queued",
                koushi_diagnostics::DiagnosticValue::Milliseconds(5),
            ),
            ("query_bytes", koushi_diagnostics::DiagnosticValue::Count(9)),
            ("query_chars", koushi_diagnostics::DiagnosticValue::Count(3)),
            ("variants", koushi_diagnostics::DiagnosticValue::Count(2)),
            (
                "normalized_diff",
                koushi_diagnostics::DiagnosticValue::Boolean(true),
            ),
        ]
    );
}

#[test]
fn search_verify_event_preserves_private_data_free_scan_and_duration_fields() {
    let event = search_verify_diagnostic_event(
        RequestId {
            connection_id: RuntimeConnectionId(21),
            sequence: 34,
        },
        5,
        2,
        89,
        13,
        17,
        &koushi_search::SearchWithCandidatesStats {
            sdk_candidates_in_scope: 3,
            verified_sdk_count: 2,
            scan_elapsed_ms: 19,
            scan: koushi_search::SearchScanStats {
                documents_visited: 55,
                documents_in_scope: 44,
                matches_before_limit: 8,
                returned: 7,
            },
            results_before_limit: 9,
            returned: 7,
        },
    );

    assert_eq!(
        event
            .fields
            .iter()
            .map(|field| (field.key, field.value.clone()))
            .collect::<Vec<_>>(),
        vec![
            (
                "request_id",
                koushi_diagnostics::DiagnosticValue::RequestId {
                    connection_id: 21,
                    sequence: 34,
                },
            ),
            ("sdk_unique", koushi_diagnostics::DiagnosticValue::Count(5)),
            ("sdk_rooms", koushi_diagnostics::DiagnosticValue::Count(2)),
            (
                "sdk_in_scope",
                koushi_diagnostics::DiagnosticValue::Count(3)
            ),
            (
                "verified_sdk",
                koushi_diagnostics::DiagnosticValue::Count(2)
            ),
            ("store_docs", koushi_diagnostics::DiagnosticValue::Count(89)),
            (
                "scan_visited",
                koushi_diagnostics::DiagnosticValue::Count(55)
            ),
            (
                "scan_in_scope",
                koushi_diagnostics::DiagnosticValue::Count(44)
            ),
            (
                "scan_matches",
                koushi_diagnostics::DiagnosticValue::Count(8)
            ),
            (
                "scan_returned",
                koushi_diagnostics::DiagnosticValue::Count(7)
            ),
            (
                "sdk_total_ms",
                koushi_diagnostics::DiagnosticValue::Milliseconds(13),
            ),
            (
                "project_ms",
                koushi_diagnostics::DiagnosticValue::Milliseconds(17),
            ),
            (
                "scan_ms",
                koushi_diagnostics::DiagnosticValue::Milliseconds(19),
            ),
        ]
    );
}

// Helper constructors
fn make_event(room_id: &str, event_id: &str, body: &str) -> SearchableEvent {
    SearchableEvent {
        room_id: room_id.to_owned(),
        event_id: event_id.to_owned(),
        sender: "@alice:test".to_owned(),
        timestamp_ms: 1000,
        body: Some(SensitiveString::new(body.to_owned())),
        attachment_filename: None,
        attachment: None,
    }
}

fn make_candidate(room_id: &str, event_id: &str) -> SearchCandidate {
    SearchCandidate {
        room_id: room_id.to_owned(),
        event_id: event_id.to_owned(),
        score_millis: 900,
    }
}

fn make_edit(target: &str, new_body: &str) -> SearchEdit {
    SearchEdit {
        edit_event_id: format!("{target}_edit"),
        target_event_id: target.to_owned(),
        sender: "@alice:test".to_owned(),
        timestamp_ms: 2000,
        body: Some(SensitiveString::new(new_body.to_owned())),
        attachment_filename: None,
        attachment: None,
    }
}

// --- Candidate verification rejects index false positives ---

#[test]
fn verify_candidate_rejects_false_positive() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(make_event("!r:test", "$e1", "hello world"));
    // Candidate for a different event not in the store — must reject.
    let candidate = make_candidate("!r:test", "$not_indexed");
    assert!(
        store.verify_candidate(candidate, "hello").is_none(),
        "candidate for unindexed event must not verify"
    );
}

#[test]
fn verify_candidate_rejects_stale_query() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(make_event("!r:test", "$e1", "hello world"));
    let candidate = make_candidate("!r:test", "$e1");
    // Query doesn't appear in the body — false positive.
    assert!(
        store
            .verify_candidate(candidate, "foobar_not_present")
            .is_none(),
        "candidate must not verify against a query not in the body"
    );
}

#[test]
fn verify_candidate_accepts_exact_match() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(make_event("!r:test", "$e1", "検索対象メッセージ test body"));
    let candidate = make_candidate("!r:test", "$e1");
    assert!(
        store.verify_candidate(candidate, "検索対象").is_some(),
        "CJK substring must verify"
    );
}

// --- Edit mutation removes old terms and finds new ---

#[test]
fn edit_removes_old_body_and_indexes_new() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(make_event("!r:test", "$e1", "original text"));

    // Verify old text matches before edit.
    let candidate_before = make_candidate("!r:test", "$e1");
    assert!(
        store
            .verify_candidate(candidate_before, "original")
            .is_some(),
        "original body must verify before edit"
    );

    // Apply edit.
    store.upsert_edit(make_edit("$e1", "replacement text"));

    // Old query must no longer verify.
    let candidate_after_old = make_candidate("!r:test", "$e1");
    assert!(
        store
            .verify_candidate(candidate_after_old, "original")
            .is_none(),
        "old body must not verify after edit"
    );

    // New query must verify.
    let candidate_after_new = make_candidate("!r:test", "$e1");
    assert!(
        store
            .verify_candidate(candidate_after_new, "replacement")
            .is_some(),
        "new body must verify after edit"
    );
}

// --- Redaction removes document ---

#[test]
fn redaction_removes_document_from_store() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(make_event("!r:test", "$e1", "secret content"));

    let candidate_before = make_candidate("!r:test", "$e1");
    assert!(
        store.verify_candidate(candidate_before, "secret").is_some(),
        "must verify before redaction"
    );

    store.redact("$e1");

    let candidate_after = make_candidate("!r:test", "$e1");
    assert!(
        store.verify_candidate(candidate_after, "secret").is_none(),
        "must not verify after redaction"
    );
    assert_eq!(store.document_count(), 0, "document count must drop to 0");
}

#[test]
fn clear_removes_documents_edits_pending_edits_and_aliases() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(make_event("!r:test", "$e1", "original content"));
    store.upsert_edit(make_edit("$e1", "edited content"));
    store.upsert_edit(make_edit("$missing", "pending edit"));

    assert_eq!(store.document_count(), 1);
    assert_eq!(store.pending_edit_count(), 1);
    assert!(
        store
            .verify_candidate(make_candidate("!r:test", "$e1_edit"), "edited")
            .is_some(),
        "edit alias must verify before clear"
    );

    store.clear();

    assert_eq!(store.document_count(), 0);
    assert_eq!(store.pending_edit_count(), 0);
    assert!(
        store
            .verify_candidate(make_candidate("!r:test", "$e1"), "edited")
            .is_none(),
        "cleared document must not verify"
    );
    assert!(
        store
            .verify_candidate(make_candidate("!r:test", "$e1_edit"), "edited")
            .is_none(),
        "cleared edit alias must not verify"
    );
    assert!(
        store
            .verify_candidate(make_candidate("!r:test", "$missing"), "pending")
            .is_none(),
        "cleared pending edit must not verify"
    );
}

// --- Unresolved replacement not indexed as standalone ---

#[test]
fn unresolved_replacement_not_indexed_as_standalone() {
    let mut store = SearchDocumentStore::default();
    // Arrive edit BEFORE original — should be a pending edit, not a standalone message.
    store.upsert_edit(make_edit("$original", "edited content"));

    // The pending edit must NOT be reachable as a standalone document.
    assert_eq!(
        store.document_count(),
        0,
        "edit before original must not appear as a document"
    );
    assert_eq!(
        store.pending_edit_count(),
        1,
        "edit before original must be pending"
    );

    // Querying for edited content must return nothing (no candidate to verify).
    let candidate = make_candidate("!r:test", "$original");
    assert!(
        store.verify_candidate(candidate, "edited").is_none(),
        "unresolved replacement must not be searchable"
    );
}

#[test]
fn unresolved_replacement_resolves_when_original_arrives() {
    let mut store = SearchDocumentStore::default();
    store.upsert_edit(make_edit("$original", "edited content"));

    // Now original arrives — document_store should apply the pending edit.
    store.upsert_message(make_event("!r:test", "$original", "original content"));

    // Pending edit must have resolved.
    assert_eq!(store.pending_edit_count(), 0, "pending edit must resolve");

    // "edited content" must verify; "original content" must not.
    let c1 = make_candidate("!r:test", "$original");
    assert!(
        store.verify_candidate(c1, "edited").is_some(),
        "resolved edit body must be searchable"
    );
    let c2 = make_candidate("!r:test", "$original");
    assert!(
        store.verify_candidate(c2, "original content").is_none(),
        "superseded original body must not verify"
    );
}

// --- Failure kinds ---

#[test]
fn search_failure_kind_is_copy_eq() {
    use koushi_protocol::failure::SearchFailureKind;
    let k1 = SearchFailureKind::IndexUnavailable;
    let k2 = k1;
    assert_eq!(k1, k2);
    let _ = SearchFailureKind::Query;
    let _ = SearchFailureKind::Internal;
}

#[test]
fn matrix_sdk_search_scope_respects_actor_resolved_room_filter() {
    assert_eq!(
        matrix_sdk_search_scope(
            &SearchScope::CurrentRoom {
                room_id: "!room:example.invalid".to_owned(),
            },
            &SearchRoomFilter::AllRooms,
        ),
        koushi_sdk::MatrixSearchScope::CurrentRoom {
            room_id: "!room:example.invalid".to_owned(),
        }
    );
    assert_eq!(
        matrix_sdk_search_scope(
            &SearchScope::CurrentSpace {
                space_id: "!space:example.invalid".to_owned(),
            },
            &SearchRoomFilter::OnlyRooms(vec![
                "!room-a:example.invalid".to_owned(),
                "!room-b:example.invalid".to_owned(),
            ]),
        ),
        koushi_sdk::MatrixSearchScope::RoomSet {
            room_ids: vec![
                "!room-a:example.invalid".to_owned(),
                "!room-b:example.invalid".to_owned(),
            ],
        }
    );
}

// --- Debug redaction ---

#[test]
fn search_command_query_redacts_query_in_debug() {
    use koushi_protocol::command::{SearchCommand, SearchScope};
    use koushi_protocol::ids::{RequestId, RuntimeConnectionId};
    let cmd = SearchCommand::Query {
        request_id: RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: 1,
        },
        query: "super-secret-search-query".to_owned(),
        scope: SearchScope::AllRooms,
        room_filter: SearchRoomFilter::AllRooms,
    };
    let debug = format!("{cmd:?}");
    assert!(
        !debug.contains("super-secret-search-query"),
        "query must not appear in Debug: {debug}"
    );
    assert!(
        debug.contains("SearchQuery(..)"),
        "redacted placeholder must appear in Debug: {debug}"
    );
}

#[test]
fn search_index_message_upsert_redacts_body_in_debug() {
    let msg = super::SearchIndexMessage::Upsert {
        room_id: "!r:test".to_owned(),
        event_id: "$e:test".to_owned(),
        sender: "@a:test".to_owned(),
        timestamp_ms: 1000,
        body: Some("very-private-message-body".to_owned()),
        attachment_filename: None,
        attachment: None,
    };
    let debug = format!("{msg:?}");
    assert!(
        !debug.contains("very-private-message-body"),
        "body must not appear in Debug: {debug}"
    );
}

#[test]
fn search_index_message_edit_redacts_body_in_debug() {
    let msg = super::SearchIndexMessage::Edit {
        edit_event_id: "$edit:test".to_owned(),
        target_event_id: "$orig:test".to_owned(),
        sender: "@a:test".to_owned(),
        timestamp_ms: 2000,
        body: Some("private-edited-content".to_owned()),
        attachment_filename: None,
        attachment: None,
    };
    let debug = format!("{msg:?}");
    assert!(
        !debug.contains("private-edited-content"),
        "body must not appear in Debug: {debug}"
    );
}

// --- SearchResultItem in SearchEvent redacts snippets from Debug ---

#[test]
fn search_result_item_snippet_is_redacted_from_debug() {
    use koushi_protocol::event::{SearchEvent, SearchResultItem};
    use koushi_protocol::ids::{RequestId, RuntimeConnectionId};
    let result = SearchResultItem {
        room_id: "!r:test".to_owned(),
        event_id: "$e:test".to_owned(),
        snippet: "检索目标消息 found here".to_owned(),
    };
    let event = SearchEvent::Results {
        request_id: RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: 2,
        },
        results: vec![result],
    };
    let debug = format!("{event:?}");
    assert!(
        !debug.contains("检索目标消息"),
        "snippet must not appear in SearchEvent Debug: {debug}"
    );
    assert!(
        !debug.contains("!r:test") && !debug.contains("$e:test"),
        "Matrix identifiers must not appear in SearchEvent Debug: {debug}"
    );
    assert!(
        debug.contains("result_count"),
        "redacted Debug should keep structural counts: {debug}"
    );
}

#[test]
fn contiguous_pending_queries_coalesce_to_latest_without_crossing_non_query_messages() {
    use std::collections::VecDeque;
    use std::time::Instant;

    use koushi_protocol::command::SearchScope;
    use koushi_protocol::ids::{RequestId, RuntimeConnectionId};

    fn query(sequence: u64) -> super::SearchActorMessage {
        super::SearchActorMessage::Query {
            request_id: RequestId {
                connection_id: RuntimeConnectionId(1),
                sequence,
            },
            query: format!("q{sequence}"),
            scope: SearchScope::AllRooms,
            room_filter: SearchRoomFilter::AllRooms,
            enqueued_at: Instant::now(),
        }
    }

    fn query_sequence(message: &super::SearchActorMessage) -> u64 {
        match message {
            super::SearchActorMessage::Query { request_id, .. } => request_id.sequence,
            other => panic!("expected query message, got {other:?}"),
        }
    }

    let mut pending = VecDeque::from([
        query(2),
        query(3),
        super::SearchActorMessage::RebuildIndex,
        query(5),
    ]);

    let (latest, dropped) = super::coalesce_contiguous_pending_queries(query(1), &mut pending);

    assert_eq!(query_sequence(&latest), 3);
    assert_eq!(dropped, 2);
    assert!(matches!(
        pending.front(),
        Some(super::SearchActorMessage::RebuildIndex)
    ));
    assert_eq!(
        query_sequence(pending.get(1).expect("query after rebuild")),
        5
    );
}
