use koushi_search::{
    AttachmentDocument, SearchDocumentStore, SearchEdit, SearchableEvent, SensitiveString,
};
use koushi_state::{AttachmentFilter, AttachmentKind, AttachmentScope, AttachmentSort};

fn attachment(kind: AttachmentKind, filename: &str) -> AttachmentDocument {
    let msgtype = match kind {
        AttachmentKind::Image => "m.image",
        AttachmentKind::Video => "m.video",
        AttachmentKind::Audio => "m.audio",
        AttachmentKind::File => "m.file",
        AttachmentKind::Sticker => "m.sticker",
    };

    AttachmentDocument {
        kind,
        msgtype: msgtype.into(),
        mimetype: Some("application/octet-stream".into()),
        size: Some(1024),
        source_mxc: "mxc://example.invalid/source".into(),
        thumbnail_mxc: None,
        filename: SensitiveString::new(filename),
        thread_root: None,
        encrypted: false,
        encryption_version: None,
        width: None,
        height: None,
        is_edited: false,
    }
}

fn event(
    room_id: &str,
    event_id: &str,
    sender: &str,
    timestamp_ms: u64,
    attachment: AttachmentDocument,
) -> SearchableEvent {
    SearchableEvent {
        room_id: room_id.into(),
        event_id: event_id.into(),
        sender: sender.into(),
        timestamp_ms,
        body: None,
        attachment_filename: None,
        attachment: Some(attachment),
    }
}

#[test]
fn room_scope_filters_attachments_to_single_room() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(event(
        "!room-a:example.invalid",
        "$event-a1",
        "@user-a:example.invalid",
        1_700_000_000_000,
        attachment(AttachmentKind::Image, "a.png"),
    ));
    store.upsert_message(event(
        "!room-b:example.invalid",
        "$event-b1",
        "@user-b:example.invalid",
        1_700_000_000_001,
        attachment(AttachmentKind::File, "b.pdf"),
    ));

    let results = store.attachments(
        &AttachmentScope::Room {
            room_id: "!room-a:example.invalid".into(),
        },
        &AttachmentFilter::default(),
        AttachmentSort::NewestFirst,
    );

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].event_id, "$event-a1");
    assert_eq!(results[0].filename, "a.png");
}

#[test]
fn space_scope_includes_only_child_room_attachments() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(event(
        "!room-alpha:example.invalid",
        "$event-alpha",
        "@user-a:example.invalid",
        1_700_000_000_000,
        attachment(AttachmentKind::Image, "alpha.png"),
    ));
    store.upsert_message(event(
        "!room-beta:example.invalid",
        "$event-beta",
        "@user-b:example.invalid",
        1_700_000_000_001,
        attachment(AttachmentKind::Audio, "beta.mp3"),
    ));
    store.upsert_message(event(
        "!room-gamma:example.invalid",
        "$event-gamma",
        "@user-c:example.invalid",
        1_700_000_000_002,
        attachment(AttachmentKind::Video, "gamma.mp4"),
    ));

    let results = store.attachments(
        &AttachmentScope::Space {
            space_id: "!space-one:example.invalid".into(),
            child_room_ids: vec![
                "!room-alpha:example.invalid".into(),
                "!room-gamma:example.invalid".into(),
            ],
        },
        &AttachmentFilter::default(),
        AttachmentSort::NewestFirst,
    );

    assert_eq!(results.len(), 2);
    let event_ids: Vec<_> = results.iter().map(|r| r.event_id.as_str()).collect();
    assert!(event_ids.contains(&"$event-alpha"));
    assert!(event_ids.contains(&"$event-gamma"));
    assert!(!event_ids.contains(&"$event-beta"));
}

#[test]
fn account_scope_returns_all_attachments() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(event(
        "!room-a:example.invalid",
        "$event-a",
        "@user-a:example.invalid",
        1_700_000_000_000,
        attachment(AttachmentKind::Image, "a.png"),
    ));
    store.upsert_message(event(
        "!room-b:example.invalid",
        "$event-b",
        "@user-b:example.invalid",
        1_700_000_000_001,
        attachment(AttachmentKind::File, "b.pdf"),
    ));

    let results = store.attachments(
        &AttachmentScope::Account,
        &AttachmentFilter::default(),
        AttachmentSort::NewestFirst,
    );

    assert_eq!(results.len(), 2);
}

#[test]
fn kind_filter_selects_requested_attachment_kinds() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(event(
        "!room-a:example.invalid",
        "$img",
        "@user-a:example.invalid",
        1,
        attachment(AttachmentKind::Image, "img.png"),
    ));
    store.upsert_message(event(
        "!room-a:example.invalid",
        "$vid",
        "@user-a:example.invalid",
        2,
        attachment(AttachmentKind::Video, "vid.mp4"),
    ));
    store.upsert_message(event(
        "!room-a:example.invalid",
        "$aud",
        "@user-a:example.invalid",
        3,
        attachment(AttachmentKind::Audio, "aud.mp3"),
    ));
    store.upsert_message(event(
        "!room-a:example.invalid",
        "$file",
        "@user-a:example.invalid",
        4,
        attachment(AttachmentKind::File, "file.pdf"),
    ));
    store.upsert_message(event(
        "!room-a:example.invalid",
        "$sticker",
        "@user-a:example.invalid",
        5,
        attachment(AttachmentKind::Sticker, "sticker.png"),
    ));

    let results = store.attachments(
        &AttachmentScope::Account,
        &AttachmentFilter {
            kinds: vec![AttachmentKind::Image, AttachmentKind::Sticker],
            filename_query: None,
        },
        AttachmentSort::NewestFirst,
    );

    assert_eq!(results.len(), 2);
    let event_ids: Vec<_> = results.iter().map(|r| r.event_id.as_str()).collect();
    assert!(event_ids.contains(&"$img"));
    assert!(event_ids.contains(&"$sticker"));
}

#[test]
fn filename_query_matches_substring_case_insensitively() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(event(
        "!room-a:example.invalid",
        "$event-1",
        "@user-a:example.invalid",
        1,
        attachment(AttachmentKind::File, "Quarterly_REPORT.pdf"),
    ));
    store.upsert_message(event(
        "!room-a:example.invalid",
        "$event-2",
        "@user-a:example.invalid",
        2,
        attachment(AttachmentKind::File, "notes.txt"),
    ));

    let results = store.attachments(
        &AttachmentScope::Account,
        &AttachmentFilter {
            kinds: vec![],
            filename_query: Some("report".into()),
        },
        AttachmentSort::NewestFirst,
    );

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].event_id, "$event-1");
}

#[test]
fn filename_query_matches_cjk_filename() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(event(
        "!room-a:example.invalid",
        "$event-cjk",
        "@user-a:example.invalid",
        1,
        attachment(AttachmentKind::File, "会議資料.pdf"),
    ));

    let results = store.attachments(
        &AttachmentScope::Account,
        &AttachmentFilter {
            kinds: vec![],
            filename_query: Some("会議".into()),
        },
        AttachmentSort::NewestFirst,
    );

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].filename, "会議資料.pdf");
}

#[test]
fn sort_by_timestamp_orders_results() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(event(
        "!room-a:example.invalid",
        "$oldest",
        "@user-a:example.invalid",
        1_700_000_000_000,
        attachment(AttachmentKind::Image, "oldest.png"),
    ));
    store.upsert_message(event(
        "!room-a:example.invalid",
        "$middle",
        "@user-a:example.invalid",
        1_700_000_000_001,
        attachment(AttachmentKind::Image, "middle.png"),
    ));
    store.upsert_message(event(
        "!room-a:example.invalid",
        "$newest",
        "@user-a:example.invalid",
        1_700_000_000_002,
        attachment(AttachmentKind::Image, "newest.png"),
    ));

    let results = store.attachments(
        &AttachmentScope::Account,
        &AttachmentFilter::default(),
        AttachmentSort::NewestFirst,
    );

    assert_eq!(
        results
            .iter()
            .map(|r| r.event_id.as_str())
            .collect::<Vec<_>>(),
        vec!["$newest", "$middle", "$oldest"]
    );

    let results = store.attachments(
        &AttachmentScope::Account,
        &AttachmentFilter::default(),
        AttachmentSort::OldestFirst,
    );

    assert_eq!(
        results
            .iter()
            .map(|r| r.event_id.as_str())
            .collect::<Vec<_>>(),
        vec!["$oldest", "$middle", "$newest"]
    );
}

#[test]
fn sort_by_filename_orders_results_alphabetically() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(event(
        "!room-a:example.invalid",
        "$event-c",
        "@user-a:example.invalid",
        1,
        attachment(AttachmentKind::File, "charlie.txt"),
    ));
    store.upsert_message(event(
        "!room-a:example.invalid",
        "$event-a",
        "@user-a:example.invalid",
        2,
        attachment(AttachmentKind::File, "alpha.txt"),
    ));
    store.upsert_message(event(
        "!room-a:example.invalid",
        "$event-b",
        "@user-a:example.invalid",
        3,
        attachment(AttachmentKind::File, "bravo.txt"),
    ));

    let results = store.attachments(
        &AttachmentScope::Account,
        &AttachmentFilter::default(),
        AttachmentSort::Filename,
    );

    assert_eq!(
        results
            .iter()
            .map(|r| r.filename.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha.txt", "bravo.txt", "charlie.txt"]
    );
}

#[test]
fn edit_updates_attachment_for_query() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(event(
        "!room-a:example.invalid",
        "$original",
        "@user-a:example.invalid",
        1,
        attachment(AttachmentKind::Image, "draft.png"),
    ));

    store.upsert_edit(SearchEdit {
        edit_event_id: "$edit".into(),
        target_event_id: "$original".into(),
        sender: "@user-a:example.invalid".into(),
        timestamp_ms: 2,
        body: None,
        attachment_filename: None,
        attachment: Some(attachment(AttachmentKind::File, "final_report.pdf")),
    });

    let results = store.attachments(
        &AttachmentScope::Account,
        &AttachmentFilter {
            kinds: vec![AttachmentKind::File],
            filename_query: Some("report".into()),
        },
        AttachmentSort::NewestFirst,
    );

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].event_id, "$original");
    assert_eq!(results[0].filename, "final_report.pdf");
    assert_eq!(results[0].kind, AttachmentKind::File);
}

#[test]
fn redacted_attachment_is_excluded_from_results() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(event(
        "!room-a:example.invalid",
        "$redacted",
        "@user-a:example.invalid",
        1,
        attachment(AttachmentKind::File, "secret.pdf"),
    ));

    store.redact("$redacted");

    let results = store.attachments(
        &AttachmentScope::Account,
        &AttachmentFilter::default(),
        AttachmentSort::NewestFirst,
    );

    assert!(results.is_empty());
}
