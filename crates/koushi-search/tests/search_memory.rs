//! #1150 memory budget: indexed history must not be retained as RAM-resident
//! message bodies.
//!
//! Search pages the persistent ngram index and reads each candidate's current
//! content from the encrypted event cache on demand, so the first-party store
//! retains no message text no matter how deep the history is.
//!
//! Scope: this measures the koushi-owned store. The encrypted Tantivy index and
//! the SDK event cache keep their own residency on disk and are reported
//! separately (index/disk size) by the QA lanes; allocator ownership is measured
//! here as retained body bytes rather than process RSS, which the QA lanes
//! report.

use koushi_search::{AttachmentDocument, SearchDocumentStore, SearchableEvent, SensitiveString};
use koushi_state::AttachmentKind;

const HISTORY_EVENTS: usize = 50_000;
const ATTACHMENT_EVENTS: usize = 5_000;
const BODY: &str = "a long message body that would dominate a RAM-resident copy ";

fn message(index: usize) -> SearchableEvent {
    SearchableEvent {
        room_id: format!("!room-{}:test", index % 32),
        event_id: format!("$event-{index}:test"),
        sender: "@user:test".into(),
        timestamp_ms: index as u64,
        body: Some(SensitiveString::new(BODY.repeat(4))),
        attachment_filename: None,
        attachment: None,
    }
}

fn attachment(index: usize) -> SearchableEvent {
    let filename = format!("report-{index}.pdf");
    SearchableEvent {
        room_id: format!("!room-{}:test", index % 32),
        event_id: format!("$file-{index}:test"),
        sender: "@user:test".into(),
        timestamp_ms: index as u64,
        body: Some(SensitiveString::new("caption text")),
        attachment_filename: Some(SensitiveString::new(filename.clone())),
        attachment: Some(AttachmentDocument {
            kind: AttachmentKind::File,
            msgtype: "m.file".to_owned(),
            mimetype: Some("application/pdf".to_owned()),
            size: Some(1024),
            source_mxc: "mxc://test/source".to_owned(),
            thumbnail_mxc: None,
            filename: SensitiveString::new(filename),
            thread_root: None,
            encrypted: false,
            encryption_version: None,
            width: None,
            height: None,
            is_edited: false,
        }),
    }
}

#[test]
fn indexed_history_retains_no_message_bodies() {
    let mut store = SearchDocumentStore::default();
    for index in 0..HISTORY_EVENTS {
        store.upsert_message(message(index), true);
    }

    assert_eq!(
        store.resident_body_bytes(),
        0,
        "{HISTORY_EVENTS} indexed messages must not be retained as RAM bodies"
    );
    assert_eq!(store.document_count(), 0);
    assert_eq!(store.pending_edit_count(), 0);
}

#[test]
fn attachment_history_retains_metadata_only() {
    let mut store = SearchDocumentStore::default();
    for index in 0..ATTACHMENT_EVENTS {
        store.upsert_message(attachment(index), true);
    }

    assert_eq!(store.document_count(), ATTACHMENT_EVENTS);
    assert_eq!(
        store.resident_body_bytes(),
        0,
        "Files metadata must be retained without the message caption"
    );

    let rows = store.attachments(
        &koushi_state::AttachmentScope::Account,
        &koushi_state::AttachmentFilter {
            kinds: Vec::new(),
            filename_query: None,
        },
        koushi_state::AttachmentSort::NewestFirst,
    );
    assert_eq!(rows.len(), ATTACHMENT_EVENTS);
    assert_eq!(
        rows[0].filename,
        format!("report-{}.pdf", ATTACHMENT_EVENTS - 1)
    );
}
