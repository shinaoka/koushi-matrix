use std::time::Duration;

use super::*;
use koushi_protocol::command::SearchScope;
use koushi_protocol::ids::{RequestId, RuntimeConnectionId};
use koushi_search::{
    SearchDocumentStore, SearchEdit, SearchEditKey, SearchableEvent, SensitiveString,
};

#[test]
fn visible_content_applies_the_account_content_policy() {
    let both = SearchCrawlerSettings {
        include_media_captions: true,
        include_filenames: true,
        ..SearchCrawlerSettings::default()
    };
    let captions_only = SearchCrawlerSettings {
        include_media_captions: true,
        include_filenames: false,
        ..both.clone()
    };
    let filenames_only = SearchCrawlerSettings {
        include_media_captions: false,
        include_filenames: true,
        ..both.clone()
    };
    let neither = SearchCrawlerSettings {
        include_media_captions: false,
        include_filenames: false,
        ..both.clone()
    };

    // A text message always keeps its body; the policy governs media only.
    assert_eq!(
        visible_content(&neither, Some("hello"), None),
        Some((Some("hello".to_owned()), None))
    );

    // A media message is marked by the resolver's filename field.
    assert_eq!(
        visible_content(&both, Some("holiday"), Some("beach.jpg")),
        Some((Some("holiday".to_owned()), Some("beach.jpg".to_owned())))
    );
    assert_eq!(
        visible_content(&captions_only, Some("holiday"), Some("beach.jpg")),
        Some((Some("holiday".to_owned()), None))
    );
    assert_eq!(
        visible_content(&filenames_only, Some("holiday"), Some("beach.jpg")),
        Some((None, Some("beach.jpg".to_owned())))
    );
    assert_eq!(
        visible_content(&neither, Some("holiday"), Some("beach.jpg")),
        None
    );
    // A filename-only media message is dropped when filenames are opted out.
    assert_eq!(
        visible_content(&captions_only, None, Some("beach.jpg")),
        None
    );
}

#[test]
fn the_content_policy_starts_restricted_until_the_account_settings_arrive() {
    let seeded = restricted_crawler_settings();

    assert!(!seeded.include_media_captions);
    assert!(!seeded.include_filenames);
    assert_eq!(
        visible_content(&seeded, Some("holiday"), Some("beach.jpg")),
        None
    );
}

#[test]
fn select_newest_caps_by_the_key_the_candidate_scan_used() {
    let mut candidates = Vec::new();
    // 50 results the index holds newest-first, all displaying an old timestamp.
    for index in 0..SEARCH_CANDIDATE_LIMIT {
        candidates.push(verified_candidate(
            1_000 + index as i64,
            &format!("$filler-{index}"),
            500,
        ));
    }
    // An out-of-order older edit: a newer displayed timestamp at an older index
    // position. Selection must not let it displace a newer result.
    candidates.push(verified_candidate(100, "$stale", 10_000));
    // A newer index position with an older displayed timestamp.
    candidates.push(verified_candidate(200, "$fresh", 10));

    let ordered = select_newest(candidates);

    assert_eq!(ordered.len(), SEARCH_CANDIDATE_LIMIT);
    assert!(
        !ordered.iter().any(|result| result.event_id == "$stale"),
        "a candidate the scan saw as older must not displace a newer one just because it displays a newer timestamp"
    );
    assert!(!ordered.iter().any(|result| result.event_id == "$fresh"));
}

#[test]
fn select_newest_presents_by_displayed_timestamp_with_an_index_tiebreak() {
    let ordered = select_newest(vec![
        verified_candidate(300, "$older-index", 900),
        verified_candidate(100, "$newest-index", 900),
        verified_candidate(200, "$oldest-display", 100),
    ]);

    // Same displayed timestamp: the newer index position comes first, matching
    // the index's `(timestamp, event_id)` order.
    assert_eq!(ordered[0].event_id, "$older-index");
    assert_eq!(ordered[1].event_id, "$newest-index");
    assert_eq!(ordered[2].event_id, "$oldest-display");
}

fn verified_candidate(
    index_timestamp_millis: i64,
    index_event_id: &str,
    displayed_timestamp_ms: u64,
) -> VerifiedCandidate {
    VerifiedCandidate {
        index_key: IndexOrderKey {
            timestamp_millis: index_timestamp_millis,
            event_id: index_event_id.to_owned(),
        },
        result: koushi_state::SearchResult {
            room_id: "!room-a:test".to_owned(),
            event_id: index_event_id.to_owned(),
            context_label: None,
            sender: "@alice:test".to_owned(),
            timestamp_ms: displayed_timestamp_ms,
            score_millis: 0,
            snippet: "body".to_owned(),
            match_field: koushi_state::SearchMatchField::MessageBody,
            highlights: Vec::new(),
            match_kind: koushi_state::SearchMatchKind::Exact,
        },
    }
}

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
        report_task: None,
    };

    handle
        .shutdown_with_timeouts(Duration::from_millis(100), Duration::from_millis(10))
        .await;

    // Liveness bound: an actor task that is never settled stays pending.
    let _ = executor::timeout(Duration::from_secs(60), settled_rx)
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
        13,
        17,
        &IndexCandidateVerification {
            in_scope: 3,
            rooms: ["!room-a:test".to_owned()].into_iter().collect(),
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
            (
                "candidates_in_scope",
                koushi_diagnostics::DiagnosticValue::Count(3)
            ),
            ("rooms", koushi_diagnostics::DiagnosticValue::Count(1)),
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

#[test]
fn only_a_content_policy_change_invalidates_an_in_flight_query() {
    let base = SearchCrawlerSettings::default();
    let slower = SearchCrawlerSettings {
        speed: koushi_state::SearchCrawlerSpeed::Slow,
        ..base.clone()
    };
    let captions_off = SearchCrawlerSettings {
        include_media_captions: false,
        ..base.clone()
    };
    let filenames_off = SearchCrawlerSettings {
        include_filenames: false,
        ..base.clone()
    };

    assert!(
        !content_policy_changed(&base, &slower),
        "a crawler speed change is not a content-policy change"
    );
    assert!(content_policy_changed(&base, &captions_off));
    assert!(content_policy_changed(&base, &filenames_off));
}

#[test]
fn a_history_replay_of_the_original_cannot_replace_an_applied_rename() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(
        make_attachment_event("!r:test", "$e1", "original.pdf"),
        true,
        None,
    );
    store.upsert_edit(make_edit_at("$e1", "$edit1", 2_000, "renamed.pdf"), true);

    // A history crawl replays the message's original attachment.
    store.upsert_message(
        make_attachment_event("!r:test", "$e1", "original.pdf"),
        false,
        None,
    );

    assert_eq!(first_filename(&store).as_deref(), Some("renamed.pdf"));
}

#[test]
fn an_older_history_edit_cannot_outrank_a_newer_one() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(
        make_attachment_event("!r:test", "$e1", "original.pdf"),
        false,
        None,
    );

    // The crawler pages newest first: the older edit arrives last.
    store.upsert_edit(make_edit_at("$e1", "$newer", 2_000, "newer.pdf"), false);
    store.upsert_edit(make_edit_at("$e1", "$older", 1_000, "older.pdf"), false);

    assert_eq!(first_filename(&store).as_deref(), Some("newer.pdf"));
}

#[test]
fn redacting_an_applied_edit_lets_the_current_content_set_the_row() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(
        make_attachment_event("!r:test", "$e1", "original.pdf"),
        false,
        None,
    );
    // A history crawl saw a later edit that the canonical state no longer shows.
    store.upsert_edit(make_edit_at("$e1", "$redacted", 5_000, "stale.pdf"), false);

    // The redaction retires that edit and drops the metadata it produced.
    store.redact("$redacted");
    assert!(
        attachment_rows(&store).is_empty(),
        "a redacted rename must not stay visible"
    );

    // The canonical projection then sends the content it still shows, as one
    // guarded upsert + edit pair.
    store.upsert_message(
        make_attachment_event("!r:test", "$e1", "current.pdf"),
        true,
        Some(SearchEditKey::new("$current", 1_000)),
    );
    store.upsert_edit(make_edit_at("$e1", "$current", 1_000, "current.pdf"), true);

    assert_eq!(first_filename(&store).as_deref(), Some("current.pdf"));
}

#[test]
fn an_older_canonical_upsert_cannot_undo_a_newer_applied_edit() {
    let mut store = SearchDocumentStore::default();
    // The timeline observed edit A.
    store.upsert_message(
        make_attachment_event("!r:test", "$e1", "a.pdf"),
        true,
        Some(SearchEditKey::new("$a", 2_000)),
    );
    store.upsert_edit(make_edit_at("$e1", "$a", 2_000, "a.pdf"), true);

    // A catch-up crawl then supplied a genuinely newer edit B.
    store.upsert_edit(make_edit_at("$e1", "$b", 3_000, "b.pdf"), false);
    assert_eq!(first_filename(&store).as_deref(), Some("b.pdf"));

    // A delayed canonical observation of A -- both halves of its pair -- must not
    // overwrite B.
    store.upsert_message(
        make_attachment_event("!r:test", "$e1", "a.pdf"),
        true,
        Some(SearchEditKey::new("$a", 2_000)),
    );
    store.upsert_edit(make_edit_at("$e1", "$a", 2_000, "a.pdf"), true);
    assert_eq!(first_filename(&store).as_deref(), Some("b.pdf"));
}

#[test]
fn a_producer_retirement_lets_a_promoted_edit_apply_and_refuses_a_replay() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(
        make_attachment_event("!r:test", "$e1", "original.pdf"),
        false,
        None,
    );
    // Normal advancement: A then a newer B. A stays valid (the SDK can promote
    // it again), so the producer retires nothing here.
    store.upsert_edit(make_edit_at("$e1", "$a", 1_000, "a.pdf"), false);
    store.upsert_edit(make_edit_at("$e1", "$b", 3_000, "b.pdf"), false);
    assert_eq!(first_filename(&store).as_deref(), Some("b.pdf"));

    // B is removed and the SDK promotes the older surviving edit A: the producer
    // retires B, then sends the promoted content as one guarded pair. A's own
    // edit time is older, and it must still apply -- and a later replay of the
    // removed B must not resurrect it.
    store.redact("$b");
    store.upsert_message(
        make_attachment_event("!r:test", "$e1", "a.pdf"),
        true,
        Some(SearchEditKey::new("$a", 1_000)),
    );
    store.upsert_edit(make_edit_at("$e1", "$a", 1_000, "a.pdf"), true);
    assert_eq!(first_filename(&store).as_deref(), Some("a.pdf"));

    // A later replay of the removed edit cannot resurrect it.
    store.upsert_edit(make_edit_at("$e1", "$b", 3_000, "b.pdf"), false);
    assert_eq!(first_filename(&store).as_deref(), Some("a.pdf"));
}

#[test]
fn a_redacted_edit_cannot_come_back_through_the_content_half_of_its_pair() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(
        make_attachment_event("!r:test", "$e1", "original.pdf"),
        false,
        None,
    );
    store.upsert_edit(make_edit_at("$e1", "$applied", 2_000, "renamed.pdf"), false);
    store.redact("$applied");
    assert!(attachment_rows(&store).is_empty());

    // A delayed canonical observation arrives as an upsert carrying the same
    // edit identity; it must not recreate the row the redaction dropped.
    store.upsert_message(
        make_attachment_event("!r:test", "$e1", "renamed.pdf"),
        true,
        Some(SearchEditKey::new("$applied", 2_000)),
    );
    assert!(attachment_rows(&store).is_empty());
    assert_eq!(store.pending_edit_count(), 0);
}

#[test]
fn retirement_refuses_every_redacted_edit_of_a_row() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(
        make_attachment_event("!r:test", "$e1", "original.pdf"),
        false,
        None,
    );
    store.upsert_edit(make_edit_at("$e1", "$older", 2_000, "older.pdf"), false);
    store.redact("$older");
    // The row comes back (a replay of the original), then a second edit is
    // redacted too.
    store.upsert_message(
        make_attachment_event("!r:test", "$e1", "original.pdf"),
        false,
        None,
    );
    store.upsert_edit(make_edit_at("$e1", "$newer", 3_000, "newer.pdf"), false);
    store.redact("$newer");

    // Retiring the newer edit must not forget the older one.
    store.upsert_edit(make_edit_at("$e1", "$older", 2_000, "older.pdf"), false);
    assert_eq!(store.pending_edit_count(), 0);
    assert!(attachment_rows(&store).is_empty());
}

#[test]
fn a_keyless_canonical_upsert_does_not_erase_a_newer_edit() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(
        make_attachment_event("!r:test", "$e1", "original.pdf"),
        true,
        None,
    );
    // A catch-up crawl applied a rename the timeline had not observed.
    store.upsert_edit(make_edit_at("$e1", "$newer", 3_000, "newer.pdf"), false);

    // A queued observation from before that edit must not replace the row.
    store.upsert_message(
        make_attachment_event("!r:test", "$e1", "original.pdf"),
        true,
        None,
    );
    assert_eq!(first_filename(&store).as_deref(), Some("newer.pdf"));
}

#[test]
fn a_redacted_edit_cannot_be_replayed() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(
        make_attachment_event("!r:test", "$e1", "original.pdf"),
        false,
        None,
    );
    store.upsert_edit(make_edit_at("$e1", "$applied", 2_000, "renamed.pdf"), false);
    store.redact("$applied");

    // The redacted edit must not come back through a replay, not even as a
    // pending edit for a row that is gone.
    store.upsert_edit(make_edit_at("$e1", "$applied", 2_000, "renamed.pdf"), false);
    assert_eq!(store.pending_edit_count(), 0);
    assert!(attachment_rows(&store).is_empty());

    // A replay of the original restores the row with its original metadata.
    store.upsert_message(
        make_attachment_event("!r:test", "$e1", "original.pdf"),
        false,
        None,
    );
    assert_eq!(first_filename(&store).as_deref(), Some("original.pdf"));
}

#[test]
fn a_newer_history_edit_beats_an_older_canonical_one() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(
        make_attachment_event("!r:test", "$e1", "original.pdf"),
        true,
        None,
    );
    // The timeline was open and showed an earlier rename.
    store.upsert_edit(make_edit_at("$e1", "$earlier", 2_000, "earlier.pdf"), true);

    // A later catch-up crawl supplies a genuinely newer rename; a canonical
    // observation must not pin the row against it.
    store.upsert_edit(make_edit_at("$e1", "$later", 3_000, "later.pdf"), false);

    assert_eq!(first_filename(&store).as_deref(), Some("later.pdf"));

    // Replaying the older canonical edit changes nothing.
    store.upsert_edit(make_edit_at("$e1", "$earlier", 2_000, "earlier.pdf"), true);
    assert_eq!(first_filename(&store).as_deref(), Some("later.pdf"));
}

#[test]
fn redacting_an_edit_retires_it_from_an_applied_row_and_from_pending_edits() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(
        make_attachment_event("!r:test", "$e1", "original.pdf"),
        false,
        None,
    );
    store.upsert_edit(make_edit_at("$e1", "$applied", 2_000, "renamed.pdf"), false);
    store.upsert_edit(
        make_edit_at("$missing", "$pending", 2_000, "pending.pdf"),
        false,
    );
    assert_eq!(store.pending_edit_count(), 1);

    store.redact("$applied");
    store.redact("$pending");

    assert_eq!(store.pending_edit_count(), 0);
    // The redacted rename no longer pins the row: the original can come back.
    store.upsert_message(
        make_attachment_event("!r:test", "$e1", "original.pdf"),
        false,
        None,
    );
    assert_eq!(first_filename(&store).as_deref(), Some("original.pdf"));
}

#[test]
fn an_edit_before_its_message_keeps_the_newest_of_the_pending_edits() {
    let mut store = SearchDocumentStore::default();
    store.upsert_edit(make_edit_at("$e1", "$older", 1_000, "older.pdf"), false);
    store.upsert_edit(make_edit_at("$e1", "$newer", 2_000, "newer.pdf"), false);
    assert_eq!(store.pending_edit_count(), 2);

    store.upsert_message(
        make_attachment_event("!r:test", "$e1", "original.pdf"),
        false,
        None,
    );

    assert_eq!(store.pending_edit_count(), 0);
    assert_eq!(first_filename(&store).as_deref(), Some("newer.pdf"));
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
        room_id: "!r:test".into(),
        edit_event_id: format!("{target}_edit"),
        target_event_id: target.to_owned(),
        sender: "@alice:test".to_owned(),
        timestamp_ms: 2000,
        body: None,
        attachment_filename: Some(SensitiveString::new(filename.to_owned())),
        attachment: None,
    }
}

/// An edit as a producer would report it: the edit event's own time and id.
fn make_edit_at(
    target: &str,
    edit_event_id: &str,
    timestamp_ms: u64,
    filename: &str,
) -> SearchEdit {
    SearchEdit {
        room_id: "!r:test".into(),
        edit_event_id: edit_event_id.to_owned(),
        target_event_id: target.to_owned(),
        sender: "@alice:test".to_owned(),
        timestamp_ms,
        body: None,
        attachment_filename: Some(SensitiveString::new(filename.to_owned())),
        attachment: None,
    }
}

fn first_filename(store: &SearchDocumentStore) -> Option<String> {
    attachment_rows(store)
        .first()
        .map(|row| row.filename.clone())
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
        room_id: "!r:test".into(),
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
    store.upsert_message(make_event("!r:test", "$e1", "hello world"), true, None);

    assert_eq!(
        store.document_count(),
        0,
        "message bodies must not be retained in RAM"
    );
}

#[test]
fn attachment_rows_are_retained_for_the_files_view() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(
        make_attachment_event("!r:test", "$e1", "agenda.pdf"),
        true,
        None,
    );

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
    store.upsert_edit(make_attachment_edit("$original", "renamed.pdf"), true);

    assert_eq!(store.document_count(), 0);
    assert_eq!(store.pending_edit_count(), 1);

    store.upsert_message(
        make_attachment_event("!r:test", "$original", "original.pdf"),
        true,
        None,
    );

    assert_eq!(store.pending_edit_count(), 0, "pending edit must resolve");
    let rows = attachment_rows(&store);
    assert_eq!(rows[0].filename, "renamed.pdf");
    assert!(rows[0].is_edited);
}

#[test]
fn caption_only_edit_still_marks_the_row_edited() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(
        make_attachment_event("!r:test", "$e1", "agenda.pdf"),
        true,
        None,
    );
    let mut caption_edit = make_edit("$e1", "a new caption");
    // A caption replacement retains its media msgtype; both SDK producers
    // carry that attachment, unlike a replacement with a text msgtype.
    caption_edit.attachment = make_attachment_event("!r:test", "$e1", "agenda.pdf").attachment;
    store.upsert_edit(caption_edit, true);

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
    store.upsert_message(
        make_attachment_event("!r:test", "$e1", "secret.pdf"),
        true,
        None,
    );

    store.redact("$e1");

    assert_eq!(store.document_count(), 0, "redacted row must drop out");
    assert!(attachment_rows(&store).is_empty());
}

#[test]
fn clear_removes_documents_and_pending_edits() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(
        make_attachment_event("!r:test", "$e1", "original.pdf"),
        true,
        None,
    );
    store.upsert_edit(make_attachment_edit("$e1", "edited.pdf"), true);
    store.upsert_edit(make_attachment_edit("$missing", "pending.pdf"), true);

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
        canonical: true,
        edit: None,
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
        room_id: "!r:test".to_owned(),
        edit_event_id: "$edit:test".to_owned(),
        target_event_id: "$orig:test".to_owned(),
        sender: "@a:test".to_owned(),
        timestamp_ms: 2000,
        body: Some("private-edited-content".to_owned()),
        attachment_filename: None,
        attachment: None,
        canonical: true,
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
        sender_id: "@s:test".to_owned(),
        timestamp_ms: 1_700_000_000_000,
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
            content_policy: None,
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

/// A fixture with a real search actor and observable reducer actions.
async fn progress_fixture() -> (SearchActor, mpsc::Receiver<Vec<AppAction>>) {
    let server = matrix_sdk::test_utils::mocks::MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let session = MatrixClientSession::from_client_for_testing(
        client.clone(),
        koushi_state::SessionInfo {
            homeserver: server.server().uri(),
            user_id: client.user_id().expect("user id").to_string(),
            device_id: client.device_id().expect("device id").to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    );
    let (action_tx, action_rx) = mpsc::channel(32);
    let (event_tx, _) = broadcast::channel(32);
    let (_, msg_rx) = mpsc::channel(32);
    let actor = SearchActor::new(
        Arc::new(session),
        action_tx,
        event_tx,
        msg_rx,
        AccountWorkScheduler::default(),
    );
    (actor, action_rx)
}

fn progress_record(index_instance: &str, settings: &SearchCrawlerSettings) -> SearchProgress {
    SearchProgress::new(SearchProgressIdentity {
        index_contract_version: crate::store::SEARCH_INDEX_CONTRACT_VERSION,
        index_instance: index_instance.to_owned(),
        include_media_captions: settings.include_media_captions,
        include_filenames: settings.include_filenames,
    })
}

fn rooms_available(
    room_ids: &[&str],
    latest: &[(&str, &str)],
    settings: &SearchCrawlerSettings,
) -> CrawlerRoomsNotification {
    CrawlerRoomsNotification {
        room_ids: room_ids
            .iter()
            .map(|room_id| (*room_id).to_owned())
            .collect(),
        latest_event_ids: latest
            .iter()
            .map(|(room_id, event_id)| ((*room_id).to_owned(), (*event_id).to_owned()))
            .collect(),
        settings: settings.clone(),
    }
}

/// A restart must not re-crawl a room whose backfill is committed, and it must
/// re-crawl a room that still owes late-decryption work.
#[tokio::test]
async fn a_settled_room_is_not_crawled_again_but_queued_work_is() {
    let (mut actor, mut action_rx) = progress_fixture().await;
    let settings = SearchCrawlerSettings::default();
    const SETTLED: &str = "!settled:example.invalid";
    const PENDING: &str = "!pending:example.invalid";

    let mut progress = progress_record("instance", &settings);
    let settled = progress.room_mut(SETTLED);
    settled.backfill_complete = true;
    settled.latest_event_id = Some("$settled-latest".to_owned());
    settled.processed = 7;
    settled.indexed = 6;
    let pending = progress.room_mut(PENDING);
    pending.backfill_complete = true;
    pending.latest_event_id = Some("$pending-latest".to_owned());
    pending.needs_reread = true;

    actor.search_progress = Some(progress);
    actor.search_progress_index_instance = "instance".to_owned();
    actor
        .handle_rooms_available(rooms_available(
            &[SETTLED, PENDING],
            &[(SETTLED, "$settled-latest"), (PENDING, "$pending-latest")],
            &settings,
        ))
        .await;

    assert!(
        actor.completed_rooms.contains_key(SETTLED),
        "a committed room is seeded from the record"
    );
    assert!(
        !actor.completed_rooms.contains_key(PENDING),
        "a room that owes late-decryption work is not treated as done"
    );
    assert!(
        actor.queued_crawl_rooms.contains(PENDING),
        "the owed work is queued"
    );
    assert!(!actor.queued_crawl_rooms.contains(SETTLED));

    let mut started = Vec::new();
    while let Ok(actions) = action_rx.try_recv() {
        for action in actions {
            if let AppAction::HistoryCrawlStarted { room_id, .. } = action {
                started.push(room_id);
            }
        }
    }
    assert_eq!(
        started,
        [PENDING.to_owned()],
        "only the room with owed work starts a crawl"
    );
}

/// #1634: each queued crawl records why it was queued, so a caller can tell a
/// completed room's catch-up from a crawl that starts over.
#[tokio::test]
async fn a_queued_crawl_records_whether_it_catches_up_or_starts_over() {
    let _diagnostics = koushi_diagnostics::test_support::lock_async().await;
    let (mut actor, _action_rx) = progress_fixture().await;
    let settings = SearchCrawlerSettings::default();
    const SETTLED: &str = "!settled:example.invalid";
    const NEW: &str = "!new:example.invalid";

    let mut progress = progress_record("instance", &settings);
    let settled = progress.room_mut(SETTLED);
    settled.backfill_complete = true;
    settled.latest_event_id = Some("$settled-latest".to_owned());
    actor.search_progress = Some(progress);
    actor.search_progress_index_instance = "instance".to_owned();

    let cursor = koushi_diagnostics::test_support::detail_cursor();
    actor
        .handle_rooms_available(rooms_available(
            &[SETTLED, NEW],
            &[(SETTLED, "$newer-than-completion"), (NEW, "$new-latest")],
            &settings,
        ))
        .await;

    // Other search tests may record in parallel; these two must be present.
    let causes = koushi_diagnostics::test_support::detail_records_since(cursor)
        .into_iter()
        .filter(|record| {
            record.event.source == "core.search" && record.event.stage == "crawl_enqueued"
        })
        .filter_map(|record| {
            record
                .event
                .fields
                .iter()
                .find_map(|field| match field.value {
                    koushi_diagnostics::DiagnosticValue::Token(cause) if field.key == "cause" => {
                        Some(cause)
                    }
                    _ => None,
                })
        })
        .collect::<Vec<_>>();
    assert!(causes.contains(&"catch_up"), "the settled room catches up");
    assert!(causes.contains(&"initial"), "the new room starts its crawl");
}

/// The record is only trusted for the index and content policy it was written
/// under.
#[tokio::test]
async fn a_record_from_another_index_or_policy_is_not_seeded() {
    let (mut actor, _action_rx) = progress_fixture().await;
    let settings = SearchCrawlerSettings::default();
    const ROOM: &str = "!recorded:example.invalid";

    let mut progress = progress_record("another-instance", &settings);
    progress.room_mut(ROOM).backfill_complete = true;
    actor.search_progress = Some(progress);
    actor.search_progress_index_instance = "instance".to_owned();
    actor
        .handle_rooms_available(rooms_available(&[ROOM], &[(ROOM, "$latest")], &settings))
        .await;
    assert!(
        !actor.completed_rooms.contains_key(ROOM),
        "a record from another index instance must not suppress the crawl"
    );

    // Same index, different content policy: also not trusted.
    let (mut actor, _action_rx) = progress_fixture().await;
    let recorded = SearchCrawlerSettings {
        include_media_captions: true,
        include_filenames: true,
        ..settings.clone()
    };
    let running = SearchCrawlerSettings {
        include_media_captions: false,
        include_filenames: true,
        ..settings.clone()
    };
    let mut progress = progress_record("instance", &recorded);
    progress.room_mut(ROOM).backfill_complete = true;
    actor.search_progress = Some(progress);
    actor.search_progress_index_instance = "instance".to_owned();
    actor
        .handle_rooms_available(rooms_available(&[ROOM], &[(ROOM, "$latest")], &running))
        .await;
    assert!(
        !actor.completed_rooms.contains_key(ROOM),
        "a policy change must re-crawl instead of trusting the record"
    );
}

/// A late decryption rewrites a stored event in place, so a settled room must be
/// reopened for indexing rather than skipped: nothing newer appears to compare.
#[tokio::test]
async fn a_late_decryption_report_reopens_a_settled_room() {
    let (mut actor, _action_rx) = progress_fixture().await;
    let settings = SearchCrawlerSettings::default();
    const ROOM: &str = "!late:example.invalid";

    let mut progress = progress_record("instance", &settings);
    let settled = progress.room_mut(ROOM);
    settled.backfill_complete = true;
    settled.latest_event_id = Some("$latest".to_owned());
    actor.search_progress = Some(progress);
    actor.search_progress_index_instance = "instance".to_owned();

    actor
        .handle_rooms_available(rooms_available(&[ROOM], &[(ROOM, "$latest")], &settings))
        .await;
    assert!(
        actor.completed_rooms.contains_key(ROOM),
        "the committed room starts settled"
    );
    assert!(!actor.queued_crawl_rooms.contains(ROOM));

    actor
        .handle_actor_message(SearchActorMessage::LateDecryptionResolved {
            room_id: ROOM.to_owned(),
        })
        .await;

    assert!(
        !actor.completed_rooms.contains_key(ROOM),
        "a late decryption must reopen the room"
    );
    assert!(
        actor.queued_crawl_rooms.contains(ROOM),
        "the reopened room is queued for a re-read"
    );
    assert!(
        actor
            .search_progress
            .as_ref()
            .and_then(|progress| progress.room(ROOM))
            .is_some_and(|room| room.needs_reread),
        "the reopen is durable"
    );
}

/// A room the restart skips has no attachment rows, so a Files query rebuilds
/// them from persisted store — without replaying the room's whole history.
#[tokio::test]
async fn files_query_rebuilds_rows_for_a_room_the_restart_skipped() {
    use matrix_sdk::ruma::{mxc_uri, room_id, user_id};
    use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};

    let server = matrix_sdk::test_utils::mocks::MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!files-rebuild:example.invalid");
    let attachment_event_id = matrix_sdk::ruma::event_id!("$files-rebuild:example.invalid");
    let f = EventFactory::new()
        .room(room_id)
        .sender(user_id!("@crawler:test.invalid"));
    server
        .mock_sync()
        .ok_and_run(&client, |builder| {
            builder.add_joined_room(
                JoinedRoomBuilder::new(room_id).add_timeline_event(
                    f.image(
                        "report.png".to_owned(),
                        mxc_uri!("mxc://example/files-rebuild").to_owned(),
                    )
                    .event_id(attachment_event_id),
                ),
            );
        })
        .await;

    let session = MatrixClientSession::from_client_for_testing(
        client.clone(),
        koushi_state::SessionInfo {
            homeserver: server.server().uri(),
            user_id: client.user_id().expect("user id").to_string(),
            device_id: client.device_id().expect("device id").to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    );
    let (action_tx, _) = mpsc::channel(32);
    let (event_tx, _) = broadcast::channel(32);
    let (_, msg_rx) = mpsc::channel(32);
    let mut actor = SearchActor::new(
        Arc::new(session),
        action_tx,
        event_tx,
        msg_rx,
        AccountWorkScheduler::default(),
    );
    let settings = SearchCrawlerSettings::default();
    let mut progress = progress_record("instance", &settings);
    let settled = progress.room_mut(room_id.as_str());
    settled.backfill_complete = true;
    settled.latest_event_id = Some(attachment_event_id.to_string());
    actor.search_progress = Some(progress);
    actor.search_progress_index_instance = "instance".to_owned();
    actor
        .handle_rooms_available(rooms_available(
            &[room_id.as_str()],
            &[(room_id.as_str(), attachment_event_id.as_str())],
            &settings,
        ))
        .await;
    assert!(
        actor.files_rebuild_pending.contains_key(room_id.as_str()),
        "a skipped room owes an on-demand rebuild"
    );
    assert!(
        actor
            .document_store
            .attachments(
                &AttachmentScope::Account,
                &AttachmentFilter::default(),
                AttachmentSort::NewestFirst
            )
            .is_empty(),
        "the skipped room starts with no rows"
    );

    actor
        .handle_attachments(
            RequestId {
                connection_id: RuntimeConnectionId(1),
                sequence: 1,
            },
            AttachmentScope::Room {
                room_id: room_id.to_string(),
            },
            AttachmentFilter::default(),
            AttachmentSort::NewestFirst,
        )
        .await;

    let rows = actor.document_store.attachments(
        &AttachmentScope::Account,
        &AttachmentFilter::default(),
        AttachmentSort::NewestFirst,
    );
    assert_eq!(
        rows.iter()
            .map(|row| row.event_id.as_str())
            .collect::<Vec<_>>(),
        [attachment_event_id.as_str()],
        "the rebuilt room exposes its stored attachment"
    );
    assert!(
        !actor.files_rebuild_pending.contains_key(room_id.as_str()),
        "one bounded rebuild finishes a small room"
    );

    // The rebuilt row survives a later query that finds nothing new: rows are
    // refreshed, never blanket-replaced.
    actor
        .handle_attachments(
            RequestId {
                connection_id: RuntimeConnectionId(1),
                sequence: 2,
            },
            AttachmentScope::Room {
                room_id: room_id.to_string(),
            },
            AttachmentFilter::default(),
            AttachmentSort::NewestFirst,
        )
        .await;
    assert_eq!(
        actor
            .document_store
            .attachments(
                &AttachmentScope::Account,
                &AttachmentFilter::default(),
                AttachmentSort::NewestFirst
            )
            .len(),
        1
    );
}

/// A page whose index commit failed must not advance the durable record: the
/// room has to be crawled again rather than marked done.
#[tokio::test]
async fn a_failed_page_leaves_the_durable_record_untouched() {
    let (mut actor, _action_rx) = progress_fixture().await;
    let settings = SearchCrawlerSettings::default();
    const ROOM: &str = "!failed-page:example.invalid";

    let mut progress = progress_record("instance", &settings);
    let room = progress.room_mut(ROOM);
    room.processed = 10;
    room.indexed = 9;
    actor.search_progress = Some(progress);
    actor.search_progress_index_instance = "instance".to_owned();
    actor.crawler_settings = settings.clone();
    actor.available_crawl_rooms.insert(ROOM.to_owned());
    let recorded_before = actor.search_progress.clone();

    let mut checkpoint = HistoryCrawlCheckpoint::new(
        ROOM.to_owned(),
        settings,
        actor.crawl_settings_generation,
        false,
    );
    checkpoint.processed = 20;
    checkpoint.indexed = 18;
    actor
        .handle_history_crawl_page_result(HistoryCrawlPageResult::Failed {
            checkpoint,
            kind: koushi_state::SearchCrawlerFailureKind::IndexUnavailable,
        })
        .await;

    assert_eq!(
        actor.search_progress, recorded_before,
        "a failed page must not advance the record"
    );
    let after = actor
        .search_progress
        .as_ref()
        .and_then(|progress| progress.room(ROOM))
        .cloned();
    let before = recorded_before
        .as_ref()
        .and_then(|progress| progress.room(ROOM))
        .cloned();
    assert_eq!(after, before, "the room keeps its pre-failure state");
}

/// The record written by one session is what the next session loads: a saved,
/// unchanged account crawls nothing.
#[tokio::test]
async fn a_saved_record_is_loaded_and_skips_the_crawl() {
    use crate::store::test_support::{file_store_actor, make_key_id};

    let data_dir = tempfile::tempdir().expect("tempdir");
    let cred_dir = tempfile::tempdir().expect("tempdir");
    let key_id = make_key_id();
    let store = file_store_actor(&data_dir, &cred_dir);
    let settings = SearchCrawlerSettings::default();
    const ROOM: &str = "!restart:example.invalid";
    const LATEST: &str = "$restart-latest";

    let instance = store.search_index_instance(&key_id).expect("instance");
    let mut progress = progress_record(&instance, &settings);
    let room = progress.room_mut(ROOM);
    room.backfill_complete = true;
    room.latest_event_id = Some(LATEST.to_owned());
    room.processed = 12;
    room.indexed = 11;
    store
        .save_search_progress(&key_id, &progress)
        .expect("save");

    // A new session loads what the previous one committed.
    let loaded = store
        .load_search_progress(&key_id)
        .expect("load")
        .expect("a saved record");
    let (mut actor, mut action_rx) = progress_fixture().await;
    actor.adopt_loaded_progress(loaded, instance);
    actor
        .handle_rooms_available(rooms_available(&[ROOM], &[(ROOM, LATEST)], &settings))
        .await;

    assert!(
        actor.completed_rooms.contains_key(ROOM),
        "the loaded record seeds the completed set"
    );
    assert!(actor.queued_crawl_rooms.is_empty());
    assert!(
        actor.crawl_queue.is_empty(),
        "an unchanged committed room is not crawled again"
    );
    let mut started = Vec::new();
    while let Ok(actions) = action_rx.try_recv() {
        for action in actions {
            if let AppAction::HistoryCrawlStarted { room_id, .. } = action {
                started.push(room_id);
            }
        }
    }
    assert!(started.is_empty(), "no crawl starts: {started:?}");
}

/// #1524: a direct search fails closed on a room the account has not joined,
/// rather than answering from the index or from another account's rooms.
#[tokio::test]
async fn a_direct_search_rejects_a_room_that_is_not_joined() {
    let server = matrix_sdk::test_utils::mocks::MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client.clone(),
        koushi_state::SessionInfo {
            homeserver: server.server().uri(),
            user_id: client.user_id().expect("user id").to_string(),
            device_id: client.device_id().expect("device id").to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    ));
    let failure = run_direct_search(
        Arc::clone(&session),
        "needle".to_owned(),
        vec!["!not-joined:example.invalid".to_owned()],
        SearchCrawlerSettings::default(),
    )
    .await
    .expect_err("an unjoined room must fail closed");
    assert_eq!(failure, DirectSearchFailure::ForbiddenRoom);
}

/// #1524: a blank query answers empty without touching the SDK index.
#[tokio::test]
async fn a_direct_search_with_a_blank_query_answers_empty() {
    let server = matrix_sdk::test_utils::mocks::MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client.clone(),
        koushi_state::SessionInfo {
            homeserver: server.server().uri(),
            user_id: client.user_id().expect("user id").to_string(),
            device_id: client.device_id().expect("device id").to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    ));
    let results = run_direct_search(
        Arc::clone(&session),
        "   ".to_owned(),
        Vec::new(),
        SearchCrawlerSettings::default(),
    )
    .await
    .expect("a blank query is not a failure");
    assert!(results.is_empty());
}
