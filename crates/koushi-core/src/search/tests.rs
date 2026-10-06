use std::time::Duration;

use super::*;
use koushi_protocol::command::SearchScope;
use koushi_protocol::ids::{RequestId, RuntimeConnectionId};
use koushi_search::{SearchDocumentStore, SearchEdit, SearchableEvent, SensitiveString};

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
        13,
        17,
        &IndexCandidateVerification {
            in_scope: 3,
            resolved: 2,
            verified: 1,
            results: Vec::new(),
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
                "candidates_in_scope",
                koushi_diagnostics::DiagnosticValue::Count(3)
            ),
            (
                "cache_resolved",
                koushi_diagnostics::DiagnosticValue::Count(2)
            ),
            ("verified", koushi_diagnostics::DiagnosticValue::Count(1)),
            (
                "sdk_total_ms",
                koushi_diagnostics::DiagnosticValue::Milliseconds(13),
            ),
            (
                "project_ms",
                koushi_diagnostics::DiagnosticValue::Milliseconds(17),
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

fn make_attachment_event(room_id: &str, event_id: &str, filename: &str) -> SearchableEvent {
    SearchableEvent {
        room_id: room_id.to_owned(),
        event_id: event_id.to_owned(),
        sender: "@alice:test".to_owned(),
        timestamp_ms: 1000,
        body: None,
        attachment_filename: Some(SensitiveString::new(filename.to_owned())),
        attachment: Some(attachment_document(filename)),
    }
}

fn attachment_document(filename: &str) -> koushi_search::AttachmentDocument {
    koushi_search::AttachmentDocument {
        kind: koushi_state::AttachmentKind::File,
        msgtype: "m.file".to_owned(),
        mimetype: Some("application/pdf".to_owned()),
        size: Some(1024),
        source_mxc: "mxc://example.invalid/source".to_owned(),
        thumbnail_mxc: None,
        filename: SensitiveString::new(filename.to_owned()),
        thread_root: None,
        encrypted: false,
        encryption_version: None,
        width: None,
        height: None,
        is_edited: false,
    }
}

fn make_attachment_edit(target: &str, filename: &str) -> SearchEdit {
    SearchEdit {
        edit_event_id: format!("{target}_edit"),
        target_event_id: target.to_owned(),
        sender: "@alice:test".to_owned(),
        timestamp_ms: 2000,
        body: None,
        attachment_filename: Some(SensitiveString::new(filename.to_owned())),
        attachment: None,
    }
}

fn attachment_rows(store: &SearchDocumentStore) -> Vec<koushi_state::AttachmentResult> {
    store.attachments(
        &koushi_state::AttachmentScope::Account,
        &koushi_state::AttachmentFilter {
            kinds: Vec::new(),
            filename_query: None,
        },
        koushi_state::AttachmentSort::NewestFirst,
    )
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

// --- Store maintenance retains attachment metadata only ---

#[test]
fn plain_messages_are_not_retained() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(make_event("!r:test", "$e1", "hello world"));

    assert_eq!(
        store.document_count(),
        0,
        "message bodies must not be retained in RAM"
    );
}

#[test]
fn attachment_rows_are_retained_for_the_files_view() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(make_attachment_event("!r:test", "$e1", "agenda.pdf"));

    let rows = attachment_rows(&store);

    assert_eq!(store.document_count(), 1);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].filename, "agenda.pdf");
    assert!(!rows[0].is_edited);
}

// --- Edits only touch attachment metadata ---

#[test]
fn edit_before_attachment_is_pending_until_it_arrives() {
    let mut store = SearchDocumentStore::default();
    // Arrive edit BEFORE the attachment — it must not become a message row.
    store.upsert_edit(make_attachment_edit("$original", "renamed.pdf"));

    assert_eq!(store.document_count(), 0);
    assert_eq!(store.pending_edit_count(), 1);

    store.upsert_message(make_attachment_event(
        "!r:test",
        "$original",
        "original.pdf",
    ));

    assert_eq!(store.pending_edit_count(), 0, "pending edit must resolve");
    let rows = attachment_rows(&store);
    assert_eq!(rows[0].filename, "renamed.pdf");
    assert!(rows[0].is_edited);
}

#[test]
fn caption_only_edit_still_marks_the_row_edited() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(make_attachment_event("!r:test", "$e1", "agenda.pdf"));
    store.upsert_edit(make_edit("$e1", "a new caption"));

    let rows = attachment_rows(&store);

    assert_eq!(
        rows[0].filename, "agenda.pdf",
        "a caption edit keeps the name"
    );
    assert!(rows[0].is_edited);
}

// --- Redaction removes the row ---

#[test]
fn redaction_removes_the_attachment_row() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(make_attachment_event("!r:test", "$e1", "secret.pdf"));

    store.redact("$e1");

    assert_eq!(store.document_count(), 0, "redacted row must drop out");
    assert!(attachment_rows(&store).is_empty());
}

#[test]
fn clear_removes_documents_and_pending_edits() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(make_attachment_event("!r:test", "$e1", "original.pdf"));
    store.upsert_edit(make_attachment_edit("$e1", "edited.pdf"));
    store.upsert_edit(make_attachment_edit("$missing", "pending.pdf"));

    assert_eq!(store.document_count(), 1);
    assert_eq!(store.pending_edit_count(), 1);
    assert_eq!(attachment_rows(&store)[0].filename, "edited.pdf");

    store.clear();

    assert_eq!(store.document_count(), 0);
    assert_eq!(store.pending_edit_count(), 0);
    assert!(attachment_rows(&store).is_empty());
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
