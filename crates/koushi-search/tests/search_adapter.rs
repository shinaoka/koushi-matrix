use koushi_search::{
    AttachmentDocument, SearchCandidate, SearchDocumentStore, SearchEdit, SearchMaintenanceQueue,
    SearchableEvent, SensitiveString, cjk_search_query_variants,
};
use koushi_state::{
    AttachmentFilter, AttachmentKind, AttachmentScope, AttachmentSort, SearchMatchField,
    SearchMatchKind, TextRange,
};

/// Verify one event's visible content with the same pure matcher the Core search
/// path uses; the store no longer holds bodies, so matching is tested directly.
fn verify(event: &SearchableEvent, query: &str) -> Option<koushi_state::SearchResult> {
    let candidate = SearchCandidate {
        room_id: event.room_id.clone(),
        event_id: event.event_id.clone(),
        score_millis: 900,
    };
    koushi_search::verify_candidate(&candidate, event, query)
}

fn message(event_id: &str, body: &str) -> SearchableEvent {
    SearchableEvent {
        room_id: "!room-a:example.invalid".into(),
        event_id: event_id.into(),
        sender: "@user-a:example.invalid".into(),
        timestamp_ms: 1_700_000_000_000,
        body: Some(SensitiveString::new(body.to_owned())),
        attachment_filename: None,
        attachment: None,
    }
}

fn attachment(filename: &str) -> AttachmentDocument {
    AttachmentDocument {
        kind: AttachmentKind::File,
        msgtype: "m.file".to_owned(),
        mimetype: Some("application/pdf".to_owned()),
        size: Some(4096),
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

fn attachment_message(event_id: &str, filename: &str) -> SearchableEvent {
    SearchableEvent {
        room_id: "!room-a:example.invalid".into(),
        event_id: event_id.into(),
        sender: "@user-a:example.invalid".into(),
        timestamp_ms: 1_700_000_000_000,
        body: None,
        attachment_filename: Some(SensitiveString::new(filename.to_owned())),
        attachment: Some(attachment(filename)),
    }
}

#[test]
fn messages_without_attachments_are_not_retained() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(message("$plain", "history stays out of RAM"));

    assert_eq!(store.document_count(), 0);
}

#[test]
fn attachment_metadata_is_retained_without_the_message_body() {
    let mut store = SearchDocumentStore::default();
    let mut event = attachment_message("$file", "agenda.pdf");
    event.body = Some(SensitiveString::new("caption text"));

    store.upsert_message(event);

    let rows = store.attachments(
        &AttachmentScope::Account,
        &AttachmentFilter {
            kinds: Vec::new(),
            filename_query: None,
        },
        AttachmentSort::NewestFirst,
    );

    assert_eq!(store.document_count(), 1);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].filename, "agenda.pdf");
    assert!(!rows[0].is_edited);
}

#[test]
fn filename_edit_updates_the_files_row_name() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(attachment_message("$file", "draft.pdf"));
    store.upsert_edit(SearchEdit {
        edit_event_id: "$edit".into(),
        target_event_id: "$file".into(),
        sender: "@user-a:example.invalid".into(),
        timestamp_ms: 1_700_000_000_100,
        body: None,
        attachment_filename: Some(SensitiveString::new("final.pdf")),
        attachment: None,
    });

    let rows = store.attachments(
        &AttachmentScope::Account,
        &AttachmentFilter {
            kinds: Vec::new(),
            filename_query: None,
        },
        AttachmentSort::NewestFirst,
    );

    // The Files view renders `attachment.filename`, so a rename must land there.
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].filename, "final.pdf");
    assert!(rows[0].is_edited);
}

#[test]
fn rename_before_the_attachment_arrives_is_applied_when_it_does() {
    let mut store = SearchDocumentStore::default();
    store.upsert_edit(SearchEdit {
        edit_event_id: "$edit".into(),
        target_event_id: "$file".into(),
        sender: "@user-a:example.invalid".into(),
        timestamp_ms: 1_700_000_000_100,
        body: None,
        attachment_filename: Some(SensitiveString::new("final.pdf")),
        attachment: None,
    });

    assert_eq!(store.pending_edit_count(), 1);

    store.upsert_message(attachment_message("$file", "draft.pdf"));

    assert_eq!(store.pending_edit_count(), 0);
    let rows = store.attachments(
        &AttachmentScope::Account,
        &AttachmentFilter {
            kinds: Vec::new(),
            filename_query: None,
        },
        AttachmentSort::NewestFirst,
    );
    assert_eq!(rows[0].filename, "final.pdf");
}

#[test]
fn redacted_attachment_is_not_listed() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(attachment_message("$file", "agenda.pdf"));

    store.redact("$file");

    assert_eq!(store.document_count(), 0);
    assert!(
        store
            .attachments(
                &AttachmentScope::Account,
                &AttachmentFilter {
                    kinds: Vec::new(),
                    filename_query: None,
                },
                AttachmentSort::NewestFirst,
            )
            .is_empty()
    );
}

#[test]
fn debug_output_redacts_decrypted_search_text() {
    let event = SearchableEvent {
        room_id: "!room-a:example.invalid".into(),
        event_id: "$event".into(),
        sender: "@user-a:example.invalid".into(),
        timestamp_ms: 1_700_000_000_000,
        body: Some(SensitiveString::new("secret body")),
        attachment_filename: Some(SensitiveString::new("secret.pdf")),
        attachment: None,
    };

    let debug = format!("{event:?}");

    assert!(!debug.contains("secret body"));
    assert!(!debug.contains("secret.pdf"));
    assert!(debug.contains("MessageBody(..)"));
}

#[test]
fn exact_message_body_match_returns_utf16_highlight() {
    let result = verify(&message("$event", "再アンケートです"), "アンケート")
        .expect("candidate should verify");

    assert_eq!(result.event_id, "$event");
    assert_eq!(result.snippet, "再アンケートです");
    assert_eq!(result.match_field, SearchMatchField::MessageBody);
    assert_eq!(result.match_kind, SearchMatchKind::Exact);
    assert_eq!(
        result.highlights,
        vec![TextRange {
            start_utf16: 1,
            end_utf16: 6,
        }]
    );
}

#[test]
fn six_character_ascii_exact_match_highlights_full_word() {
    let result = verify(&message("$ascii-event", "prefix Signal suffix"), "Signal")
        .expect("candidate should verify");

    assert_eq!(
        result.highlights,
        vec![TextRange {
            start_utf16: 7,
            end_utf16: 13,
        }]
    );
}

#[test]
fn full_width_query_matches_half_width_indexed_message_body() {
    let result = verify(&message("$event", "会議資料 ABC123 ready"), "ＡＢＣ１２３")
        .expect("width-folded query should verify against canonical body text");

    assert_eq!(result.snippet, "会議資料 ABC123 ready");
    assert_eq!(result.match_field, SearchMatchField::MessageBody);
    assert_eq!(
        result.highlights,
        vec![TextRange {
            start_utf16: 5,
            end_utf16: 11,
        }]
    );
}

#[test]
fn half_width_query_matches_full_width_indexed_message_body() {
    let result = verify(&message("$event", "会議資料 ＡＢＣ１２３ ready"), "ABC123")
        .expect("canonical query should verify against width-folded body text");

    assert_eq!(result.snippet, "会議資料 ＡＢＣ１２３ ready");
    assert_eq!(result.match_field, SearchMatchField::MessageBody);
    assert_eq!(
        result.highlights,
        vec![TextRange {
            start_utf16: 5,
            end_utf16: 11,
        }]
    );
}

#[test]
fn voiced_half_width_kana_query_matches_canonical_kana_and_highlights_source_cluster() {
    let result = verify(&message("$event", "会議資料 ﾊﾞﾅﾅ ready"), "バナナ")
        .expect("voiced half-width kana should verify against canonical query text");

    assert_eq!(result.snippet, "会議資料 ﾊﾞﾅﾅ ready");
    assert_eq!(result.match_field, SearchMatchField::MessageBody);
    assert_eq!(
        result.highlights,
        vec![TextRange {
            start_utf16: 5,
            end_utf16: 9,
        }]
    );
}

#[test]
fn cjk_search_query_variants_include_raw_and_nfkc_width_folded_terms() {
    assert_eq!(
        cjk_search_query_variants(" ＡＢＣ１２３ "),
        vec!["ＡＢＣ１２３".to_owned(), "abc123".to_owned()]
    );
    assert_eq!(
        cjk_search_query_variants("ABC123"),
        vec!["ABC123".to_owned(), "abc123".to_owned()]
    );
    assert_eq!(
        cjk_search_query_variants("ﾊﾞﾅﾅ"),
        vec!["ﾊﾞﾅﾅ".to_owned(), "バナナ".to_owned()]
    );
}

#[test]
fn ascii_case_variant_generation_matches_displayed_query_case() {
    assert_eq!(
        cjk_search_query_variants("Gpt"),
        vec!["Gpt".to_owned(), "gpt".to_owned()]
    );
    assert_eq!(cjk_search_query_variants("gpt"), vec!["gpt".to_owned()]);
}

#[test]
fn uppercase_ascii_query_verifies_lowercase_message_body() {
    let result = verify(
        &message("$event", "open https://chatgpt.example/share"),
        "Gpt",
    )
    .expect("normalized verification should match lowercase body text");

    assert_eq!(result.event_id, "$event");
    assert_eq!(
        result.highlights,
        vec![TextRange {
            start_utf16: 17,
            end_utf16: 20,
        }]
    );
}

#[test]
fn attachment_filename_match_uses_attachment_field() {
    let event = SearchableEvent {
        room_id: "!room-a:example.invalid".into(),
        event_id: "$file".into(),
        sender: "@user-a:example.invalid".into(),
        timestamp_ms: 1_700_000_000_000,
        body: None,
        attachment_filename: Some(SensitiveString::new("seminar_schedule.pdf")),
        attachment: None,
    };

    let result = verify(&event, "schedule").expect("filename candidate should verify");

    assert_eq!(result.event_id, "$file");
    assert_eq!(result.snippet, "seminar_schedule.pdf");
    assert_eq!(result.match_field, SearchMatchField::AttachmentFileName);
    assert_eq!(
        result.highlights,
        vec![TextRange {
            start_utf16: 8,
            end_utf16: 16,
        }]
    );
}

#[test]
fn ngram_false_positive_without_exact_span_is_dropped() {
    assert!(verify(&message("$event", "再アンケートです"), "欠席").is_none());
}

#[test]
fn late_decryption_queue_drains_events_per_room_without_duplicates() {
    let mut queue = SearchMaintenanceQueue::default();

    queue.enqueue_late_decryption("!room-a:example.invalid", "$event-a");
    queue.enqueue_late_decryption("!room-a:example.invalid", "$event-a");
    queue.enqueue_late_decryption("!room-b:example.invalid", "$event-b");

    let room_a = queue.drain_late_decryption("!room-a:example.invalid");

    assert_eq!(room_a.len(), 1);
    assert_eq!(room_a[0].room_id, "!room-a:example.invalid");
    assert_eq!(room_a[0].event_id, "$event-a");
    assert!(
        queue
            .drain_late_decryption("!room-a:example.invalid")
            .is_empty()
    );
    assert_eq!(queue.pending_late_decryption_count(), 1);
}

#[test]
fn event_cache_lag_marks_room_for_reindex_once() {
    let mut queue = SearchMaintenanceQueue::default();

    queue.mark_room_reindex_needed("!room-a:example.invalid");
    queue.mark_room_reindex_needed("!room-a:example.invalid");
    queue.mark_room_reindex_needed("!room-b:example.invalid");

    assert_eq!(
        queue.drain_reindex_rooms(),
        vec!["!room-a:example.invalid", "!room-b:example.invalid"]
    );
    assert!(queue.drain_reindex_rooms().is_empty());
}
