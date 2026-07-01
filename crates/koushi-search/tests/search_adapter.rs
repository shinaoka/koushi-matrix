use koushi_search::{
    SearchCandidate, SearchDocumentStore, SearchEdit, SearchMaintenanceQueue, SearchableEvent,
    SensitiveString, cjk_search_query_variants,
};
use koushi_state::{SearchMatchField, SearchMatchKind, TextRange};

#[test]
fn search_document_store_can_be_created() {
    let store = SearchDocumentStore::default();

    assert_eq!(store.document_count(), 0);
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
    let mut store = SearchDocumentStore::default();
    store.upsert_message(SearchableEvent {
        room_id: "!room-a:example.invalid".into(),
        event_id: "$event".into(),
        sender: "@user-a:example.invalid".into(),
        timestamp_ms: 1_700_000_000_000,
        body: Some(SensitiveString::new("再アンケートです")),
        attachment_filename: None,
        attachment: None,
    });

    let result = store
        .verify_candidate(
            SearchCandidate {
                room_id: "!room-a:example.invalid".into(),
                event_id: "$event".into(),
                score_millis: 900,
            },
            "アンケート",
        )
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
fn full_width_query_matches_half_width_indexed_message_body() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(SearchableEvent {
        room_id: "!room-a:example.invalid".into(),
        event_id: "$event".into(),
        sender: "@user-a:example.invalid".into(),
        timestamp_ms: 1_700_000_000_000,
        body: Some(SensitiveString::new("会議資料 ABC123 ready")),
        attachment_filename: None,
        attachment: None,
    });

    let result = store
        .verify_candidate(
            SearchCandidate {
                room_id: "!room-a:example.invalid".into(),
                event_id: "$event".into(),
                score_millis: 900,
            },
            "ＡＢＣ１２３",
        )
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
    let mut store = SearchDocumentStore::default();
    store.upsert_message(SearchableEvent {
        room_id: "!room-a:example.invalid".into(),
        event_id: "$event".into(),
        sender: "@user-a:example.invalid".into(),
        timestamp_ms: 1_700_000_000_000,
        body: Some(SensitiveString::new("会議資料 ＡＢＣ１２３ ready")),
        attachment_filename: None,
        attachment: None,
    });

    let result = store
        .verify_candidate(
            SearchCandidate {
                room_id: "!room-a:example.invalid".into(),
                event_id: "$event".into(),
                score_millis: 900,
            },
            "ABC123",
        )
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
    let mut store = SearchDocumentStore::default();
    store.upsert_message(SearchableEvent {
        room_id: "!room-a:example.invalid".into(),
        event_id: "$event".into(),
        sender: "@user-a:example.invalid".into(),
        timestamp_ms: 1_700_000_000_000,
        body: Some(SensitiveString::new("会議資料 ﾊﾞﾅﾅ ready")),
        attachment_filename: None,
        attachment: None,
    });

    let result = store
        .verify_candidate(
            SearchCandidate {
                room_id: "!room-a:example.invalid".into(),
                event_id: "$event".into(),
                score_millis: 900,
            },
            "バナナ",
        )
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
fn attachment_filename_match_uses_attachment_field() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(SearchableEvent {
        room_id: "!room-a:example.invalid".into(),
        event_id: "$file".into(),
        sender: "@user-a:example.invalid".into(),
        timestamp_ms: 1_700_000_000_000,
        body: None,
        attachment_filename: Some(SensitiveString::new("seminar_schedule.pdf")),
        attachment: None,
    });

    let result = store
        .verify_candidate(
            SearchCandidate {
                room_id: "!room-a:example.invalid".into(),
                event_id: "$file".into(),
                score_millis: 875,
            },
            "schedule",
        )
        .expect("filename candidate should verify");

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
    let mut store = SearchDocumentStore::default();
    store.upsert_message(SearchableEvent {
        room_id: "!room-a:example.invalid".into(),
        event_id: "$event".into(),
        sender: "@user-a:example.invalid".into(),
        timestamp_ms: 1_700_000_000_000,
        body: Some(SensitiveString::new("再アンケートです")),
        attachment_filename: None,
        attachment: None,
    });

    let result = store.verify_candidate(
        SearchCandidate {
            room_id: "!room-a:example.invalid".into(),
            event_id: "$event".into(),
            score_millis: 900,
        },
        "欠席",
    );

    assert!(result.is_none());
}

#[test]
fn edit_before_target_is_pending_until_original_arrives() {
    let mut store = SearchDocumentStore::default();
    store.upsert_edit(SearchEdit {
        edit_event_id: "$edit".into(),
        target_event_id: "$original".into(),
        sender: "@user-a:example.invalid".into(),
        timestamp_ms: 1_700_000_000_100,
        body: Some(SensitiveString::new("edited agenda")),
        attachment_filename: None,
        attachment: None,
    });

    assert_eq!(store.pending_edit_count(), 1);
    assert!(
        store
            .verify_candidate(
                SearchCandidate {
                    room_id: "!room-a:example.invalid".into(),
                    event_id: "$original".into(),
                    score_millis: 900,
                },
                "edited",
            )
            .is_none()
    );

    store.upsert_message(SearchableEvent {
        room_id: "!room-a:example.invalid".into(),
        event_id: "$original".into(),
        sender: "@user-a:example.invalid".into(),
        timestamp_ms: 1_700_000_000_000,
        body: Some(SensitiveString::new("old agenda")),
        attachment_filename: None,
        attachment: None,
    });

    let result = store
        .verify_candidate(
            SearchCandidate {
                room_id: "!room-a:example.invalid".into(),
                event_id: "$original".into(),
                score_millis: 900,
            },
            "edited",
        )
        .expect("pending edit should apply after original arrives");

    assert_eq!(store.pending_edit_count(), 0);
    assert_eq!(result.event_id, "$original");
    assert_eq!(result.snippet, "edited agenda");
}

#[test]
fn redacted_event_is_not_returned() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(SearchableEvent {
        room_id: "!room-a:example.invalid".into(),
        event_id: "$event".into(),
        sender: "@user-a:example.invalid".into(),
        timestamp_ms: 1_700_000_000_000,
        body: Some(SensitiveString::new("visible before redaction")),
        attachment_filename: Some(SensitiveString::new("visible.pdf")),
        attachment: None,
    });

    store.redact("$event");

    assert_eq!(store.document_count(), 0);
    assert!(
        store
            .verify_candidate(
                SearchCandidate {
                    room_id: "!room-a:example.invalid".into(),
                    event_id: "$event".into(),
                    score_millis: 900,
                },
                "visible",
            )
            .is_none()
    );
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

// #162: the document store is a first-class candidate source. A message koushi
// has indexed (e.g. crawled history) must be findable via a direct scan even
// when the SDK ngram index would not surface it as a candidate.
#[test]
fn document_store_scan_finds_body_candidate_without_index() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(SearchableEvent {
        room_id: "!room-a:example.invalid".into(),
        event_id: "$scan-1".into(),
        sender: "@user-a:example.invalid".into(),
        timestamp_ms: 1_700_000_000_000,
        // synthetic 2-char CJK body (mirrors the reported shape without private data)
        body: Some(SensitiveString::new("検査しました")),
        attachment_filename: None,
        attachment: None,
    });

    let hits = store.scan_candidates("検査", None, 50);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].event_id, "$scan-1");
    assert_eq!(hits[0].match_field, SearchMatchField::MessageBody);
    assert_eq!(hits[0].match_kind, SearchMatchKind::Exact);

    // Absent query text yields no candidates.
    assert!(store.scan_candidates("不一致語", None, 50).is_empty());

    // Room filter restricts scope.
    assert!(
        store
            .scan_candidates("検査", Some("!other:example.invalid"), 50)
            .is_empty()
    );
    assert_eq!(
        store
            .scan_candidates("検査", Some("!room-a:example.invalid"), 50)
            .len(),
        1
    );
}

// #162: scan ordering is most-recent-first and respects the cap.
#[test]
fn document_store_scan_orders_recent_first_and_caps() {
    let mut store = SearchDocumentStore::default();
    for (idx, ts) in [("$old", 1_000u64), ("$new", 3_000u64), ("$mid", 2_000u64)] {
        store.upsert_message(SearchableEvent {
            room_id: "!room-a:example.invalid".into(),
            event_id: idx.into(),
            sender: "@user-a:example.invalid".into(),
            timestamp_ms: ts,
            body: Some(SensitiveString::new("検査")),
            attachment_filename: None,
            attachment: None,
        });
    }

    let hits = store.scan_candidates("検査", None, 50);
    assert_eq!(
        hits.iter().map(|h| h.event_id.as_str()).collect::<Vec<_>>(),
        vec!["$new", "$mid", "$old"]
    );

    let capped = store.scan_candidates("検査", None, 2);
    assert_eq!(capped.len(), 2);
    assert_eq!(capped[0].event_id, "$new");
}
