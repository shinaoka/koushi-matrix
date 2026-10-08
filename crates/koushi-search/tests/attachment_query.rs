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

fn replacement(id: &str, time: u64, media: bool) -> SearchEdit {
    SearchEdit {
        room_id: "!room-a:example.invalid".into(),
        edit_event_id: id.into(),
        target_event_id: "$original".into(),
        sender: "@user-a:example.invalid".into(),
        timestamp_ms: time,
        body: None,
        attachment_filename: None,
        attachment: media.then(|| attachment(AttachmentKind::File, id)),
    }
}

fn original() -> SearchableEvent {
    event(
        "!room-a:example.invalid",
        "$original",
        "@user-a:example.invalid",
        1,
        attachment(AttachmentKind::File, "original.pdf"),
    )
}

fn files(store: &SearchDocumentStore) -> Vec<koushi_state::AttachmentResult> {
    store.attachments(
        &AttachmentScope::Account,
        &AttachmentFilter::default(),
        AttachmentSort::NewestFirst,
    )
}

#[test]
fn foreign_replacements_cannot_change_attachment_metadata() {
    for pending_first in [false, true] {
        for canonical in [false, true] {
            for wrong_room in [false, true] {
                let mut store = SearchDocumentStore::default();
                let mut edit = replacement("$foreign", 10, true);
                if wrong_room {
                    edit.room_id = "!other:example.invalid".into();
                } else {
                    edit.sender = "@other:example.invalid".into();
                }
                if !pending_first {
                    store.upsert_message(original(), false, None);
                }
                store.upsert_edit(edit, canonical);
                if pending_first {
                    store.upsert_message(original(), false, None);
                }
                assert_eq!(files(&store)[0].filename, "original.pdf");
                store.upsert_edit(replacement("$valid", 2, true), false);
                assert_eq!(files(&store)[0].filename, "$valid");
            }
        }
    }
}

#[test]
fn room_message_edit_does_not_replace_sticker_root() {
    let mut store = SearchDocumentStore::default();
    let mut root = original();
    root.attachment = Some(attachment(AttachmentKind::Sticker, "original.pdf"));
    store.upsert_message(root, false, None);
    store.upsert_edit(replacement("$message-edit", 2, true), false);
    assert_eq!(files(&store)[0].filename, "original.pdf");
}

#[test]
fn text_provenance_survives_retirement_of_the_media_version() {
    for canonical_upsert in [false, true] {
        let mut store = SearchDocumentStore::default();
        store.upsert_message(original(), false, None);
        store.upsert_edit(replacement("$media", 3, true), false);
        store.retire_edit("$original", "$media");
        if canonical_upsert {
            let mut text = original();
            text.attachment = None;
            store.upsert_message(
                text,
                true,
                Some(koushi_search::SearchEditKey::new("$text", 4)),
            );
        } else {
            store.upsert_edit(replacement("$text", 4, false), true);
        }
        store.upsert_message(original(), false, None);
        assert!(
            files(&store).is_empty(),
            "old media must not resurrect after text"
        );
        assert_eq!(store.resident_body_bytes(), 0);
    }
}

#[test]
fn retiring_one_pending_edit_preserves_the_survivor() {
    let mut store = SearchDocumentStore::default();
    store.upsert_edit(replacement("$a", 2, true), false);
    store.upsert_edit(replacement("$b", 3, true), false);
    store.redact("$b");
    store.upsert_message(original(), false, None);
    assert_eq!(files(&store)[0].filename, "$a");
}

#[test]
fn equal_time_stale_canonical_edit_does_not_erase_newer_id() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(original(), false, None);
    store.upsert_edit(replacement("$z", 3, true), false);
    store.upsert_edit(replacement("$a", 3, true), true);
    assert_eq!(files(&store)[0].filename, "$z");
}

#[test]
fn text_replacement_hides_a_file_and_blocks_older_media() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(original(), false, None);
    store.upsert_edit(replacement("$text", 4, false), false);
    store.upsert_edit(replacement("$media", 3, true), true);
    assert!(files(&store).is_empty());
    assert_eq!(store.resident_body_bytes(), 0);
}

#[test]
fn pending_text_replacement_hides_older_pending_media() {
    let mut store = SearchDocumentStore::default();
    store.upsert_edit(replacement("$media", 3, true), false);
    store.upsert_edit(replacement("$text", 4, false), false);
    store.upsert_message(original(), false, None);
    assert!(files(&store).is_empty());
    assert_eq!(store.resident_body_bytes(), 0);
}

#[test]
fn room_scope_filters_attachments_to_single_room() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(
        event(
            "!room-a:example.invalid",
            "$event-a1",
            "@user-a:example.invalid",
            1_700_000_000_000,
            attachment(AttachmentKind::Image, "a.png"),
        ),
        true,
        None,
    );
    store.upsert_message(
        event(
            "!room-b:example.invalid",
            "$event-b1",
            "@user-b:example.invalid",
            1_700_000_000_001,
            attachment(AttachmentKind::File, "b.pdf"),
        ),
        true,
        None,
    );

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
    store.upsert_message(
        event(
            "!room-alpha:example.invalid",
            "$event-alpha",
            "@user-a:example.invalid",
            1_700_000_000_000,
            attachment(AttachmentKind::Image, "alpha.png"),
        ),
        true,
        None,
    );
    store.upsert_message(
        event(
            "!room-beta:example.invalid",
            "$event-beta",
            "@user-b:example.invalid",
            1_700_000_000_001,
            attachment(AttachmentKind::Audio, "beta.mp3"),
        ),
        true,
        None,
    );
    store.upsert_message(
        event(
            "!room-gamma:example.invalid",
            "$event-gamma",
            "@user-c:example.invalid",
            1_700_000_000_002,
            attachment(AttachmentKind::Video, "gamma.mp4"),
        ),
        true,
        None,
    );

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
    store.upsert_message(
        event(
            "!room-a:example.invalid",
            "$event-a",
            "@user-a:example.invalid",
            1_700_000_000_000,
            attachment(AttachmentKind::Image, "a.png"),
        ),
        true,
        None,
    );
    store.upsert_message(
        event(
            "!room-b:example.invalid",
            "$event-b",
            "@user-b:example.invalid",
            1_700_000_000_001,
            attachment(AttachmentKind::File, "b.pdf"),
        ),
        true,
        None,
    );

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
    store.upsert_message(
        event(
            "!room-a:example.invalid",
            "$img",
            "@user-a:example.invalid",
            1,
            attachment(AttachmentKind::Image, "img.png"),
        ),
        true,
        None,
    );
    store.upsert_message(
        event(
            "!room-a:example.invalid",
            "$vid",
            "@user-a:example.invalid",
            2,
            attachment(AttachmentKind::Video, "vid.mp4"),
        ),
        true,
        None,
    );
    store.upsert_message(
        event(
            "!room-a:example.invalid",
            "$aud",
            "@user-a:example.invalid",
            3,
            attachment(AttachmentKind::Audio, "aud.mp3"),
        ),
        true,
        None,
    );
    store.upsert_message(
        event(
            "!room-a:example.invalid",
            "$file",
            "@user-a:example.invalid",
            4,
            attachment(AttachmentKind::File, "file.pdf"),
        ),
        true,
        None,
    );
    store.upsert_message(
        event(
            "!room-a:example.invalid",
            "$sticker",
            "@user-a:example.invalid",
            5,
            attachment(AttachmentKind::Sticker, "sticker.png"),
        ),
        true,
        None,
    );

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
    store.upsert_message(
        event(
            "!room-a:example.invalid",
            "$event-1",
            "@user-a:example.invalid",
            1,
            attachment(AttachmentKind::File, "Quarterly_REPORT.pdf"),
        ),
        true,
        None,
    );
    store.upsert_message(
        event(
            "!room-a:example.invalid",
            "$event-2",
            "@user-a:example.invalid",
            2,
            attachment(AttachmentKind::File, "notes.txt"),
        ),
        true,
        None,
    );

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
    store.upsert_message(
        event(
            "!room-a:example.invalid",
            "$event-cjk",
            "@user-a:example.invalid",
            1,
            attachment(AttachmentKind::File, "会議資料.pdf"),
        ),
        true,
        None,
    );

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
    store.upsert_message(
        event(
            "!room-a:example.invalid",
            "$oldest",
            "@user-a:example.invalid",
            1_700_000_000_000,
            attachment(AttachmentKind::Image, "oldest.png"),
        ),
        true,
        None,
    );
    store.upsert_message(
        event(
            "!room-a:example.invalid",
            "$middle",
            "@user-a:example.invalid",
            1_700_000_000_001,
            attachment(AttachmentKind::Image, "middle.png"),
        ),
        true,
        None,
    );
    store.upsert_message(
        event(
            "!room-a:example.invalid",
            "$newest",
            "@user-a:example.invalid",
            1_700_000_000_002,
            attachment(AttachmentKind::Image, "newest.png"),
        ),
        true,
        None,
    );

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
    store.upsert_message(
        event(
            "!room-a:example.invalid",
            "$event-c",
            "@user-a:example.invalid",
            1,
            attachment(AttachmentKind::File, "charlie.txt"),
        ),
        true,
        None,
    );
    store.upsert_message(
        event(
            "!room-a:example.invalid",
            "$event-a",
            "@user-a:example.invalid",
            2,
            attachment(AttachmentKind::File, "alpha.txt"),
        ),
        true,
        None,
    );
    store.upsert_message(
        event(
            "!room-a:example.invalid",
            "$event-b",
            "@user-a:example.invalid",
            3,
            attachment(AttachmentKind::File, "bravo.txt"),
        ),
        true,
        None,
    );

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
    store.upsert_message(
        event(
            "!room-a:example.invalid",
            "$original",
            "@user-a:example.invalid",
            1,
            attachment(AttachmentKind::Image, "draft.png"),
        ),
        true,
        None,
    );

    store.upsert_edit(
        SearchEdit {
            room_id: "!room-a:example.invalid".into(),
            edit_event_id: "$edit".into(),
            target_event_id: "$original".into(),
            sender: "@user-a:example.invalid".into(),
            timestamp_ms: 2,
            body: None,
            attachment_filename: None,
            attachment: Some(attachment(AttachmentKind::File, "final_report.pdf")),
        },
        true,
    );

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
    store.upsert_message(
        event(
            "!room-a:example.invalid",
            "$redacted",
            "@user-a:example.invalid",
            1,
            attachment(AttachmentKind::File, "secret.pdf"),
        ),
        true,
        None,
    );

    store.redact("$redacted");

    let results = store.attachments(
        &AttachmentScope::Account,
        &AttachmentFilter::default(),
        AttachmentSort::NewestFirst,
    );

    assert!(results.is_empty());
}
