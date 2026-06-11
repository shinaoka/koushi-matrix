use matrix_desktop_search::{
    SearchCandidate, SearchDocumentStore, SearchableEvent, SensitiveString,
};
use matrix_desktop_state::{SearchMatchField, SearchMatchKind, TextRange};

#[test]
fn search_document_store_can_be_created() {
    let store = SearchDocumentStore::default();

    assert_eq!(store.document_count(), 0);
}

#[test]
fn debug_output_redacts_decrypted_search_text() {
    let event = SearchableEvent {
        room_id: "!room:example.org".into(),
        event_id: "$event".into(),
        sender: "@alice:example.org".into(),
        timestamp_ms: 1_700_000_000_000,
        body: Some(SensitiveString::new("secret body")),
        attachment_filename: Some(SensitiveString::new("secret.pdf")),
    };

    let debug = format!("{event:?}");

    assert!(!debug.contains("secret body"));
    assert!(!debug.contains("secret.pdf"));
    assert!(debug.contains("SensitiveString(..)"));
}

#[test]
fn exact_message_body_match_returns_utf16_highlight() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(SearchableEvent {
        room_id: "!room:example.org".into(),
        event_id: "$event".into(),
        sender: "@alice:example.org".into(),
        timestamp_ms: 1_700_000_000_000,
        body: Some(SensitiveString::new("再アンケートです")),
        attachment_filename: None,
    });

    let result = store
        .verify_candidate(
            SearchCandidate {
                room_id: "!room:example.org".into(),
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
fn attachment_filename_match_uses_attachment_field() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(SearchableEvent {
        room_id: "!room:example.org".into(),
        event_id: "$file".into(),
        sender: "@alice:example.org".into(),
        timestamp_ms: 1_700_000_000_000,
        body: None,
        attachment_filename: Some(SensitiveString::new("seminar_schedule.pdf")),
    });

    let result = store
        .verify_candidate(
            SearchCandidate {
                room_id: "!room:example.org".into(),
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
        room_id: "!room:example.org".into(),
        event_id: "$event".into(),
        sender: "@alice:example.org".into(),
        timestamp_ms: 1_700_000_000_000,
        body: Some(SensitiveString::new("再アンケートです")),
        attachment_filename: None,
    });

    let result = store.verify_candidate(
        SearchCandidate {
            room_id: "!room:example.org".into(),
            event_id: "$event".into(),
            score_millis: 900,
        },
        "欠席",
    );

    assert!(result.is_none());
}
