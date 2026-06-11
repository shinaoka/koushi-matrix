use matrix_desktop_search::{SearchDocumentStore, SearchableEvent, SensitiveString};

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
