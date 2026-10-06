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

#[test]
fn committed_rooms_seed_the_completed_map_for_the_current_backend_version() {
    let policy = crate::store::search_crawl::CrawlContentPolicy::new(false, false);
    let mut progress = crate::store::search_crawl::SearchCrawlProgress::new();
    progress.commit(
        "!room-a:test".to_owned(),
        crate::store::search_crawl::CommittedRoomCrawl {
            latest_event_id: Some("$e9".to_owned()),
            processed: 12,
            indexed: 7,
        },
        policy,
    );

    let completed = completed_rooms_from_committed(&progress, policy);

    assert_eq!(completed.len(), 1);
    let crawl = completed.get("!room-a:test").expect("seeded room");
    assert_eq!(crawl.latest_event_id.as_deref(), Some("$e9"));
    assert_eq!(crawl.processed, 12);
    assert_eq!(crawl.indexed, 7);

    // A different content policy invalidates the same record.
    assert!(
        completed_rooms_from_committed(
            &progress,
            crate::store::search_crawl::CrawlContentPolicy::new(true, false)
        )
        .is_empty(),
        "a commitment recorded under another content policy must not be trusted"
    );
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

#[tokio::test]
async fn files_rows_are_rebuilt_from_the_persisted_event_cache() {
    use matrix_sdk::ruma::{event_id, owned_mxc_uri, room_id, user_id};
    use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};

    let server = matrix_sdk::test_utils::mocks::MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!files-refresh:example.invalid");
    client
        .event_cache()
        .subscribe()
        .expect("event cache subscription");
    let factory = EventFactory::new()
        .room(room_id)
        .sender(user_id!("@alice:example.invalid"));
    server
        .mock_sync()
        .ok_and_run(&client, |builder| {
            builder.add_joined_room(
                JoinedRoomBuilder::new(room_id)
                    .add_timeline_event(
                        factory
                            .text_msg("no attachment here")
                            .event_id(event_id!("$plain")),
                    )
                    .add_timeline_event(
                        factory
                            .image(
                                "agenda.pdf".to_owned(),
                                owned_mxc_uri!("mxc://example.invalid/agenda"),
                            )
                            .event_id(event_id!("$with-attachment")),
                    ),
            );
        })
        .await;

    let session_info = koushi_state::SessionInfo {
        homeserver: server.server().uri(),
        user_id: client.user_id().expect("mock client user id").to_string(),
        device_id: client
            .device_id()
            .expect("mock client device id")
            .to_string(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    };
    let session = MatrixClientSession::from_client_for_testing(client.clone(), session_info);

    // The room's crawl is already committed, so this paged store read is the
    // only source of its Files rows after a restart.
    let mut events = Vec::new();
    let mut cursor = None;
    loop {
        let page = koushi_sdk::persisted_room_event_page(&session, room_id.as_str(), cursor, 500)
            .await
            .expect("the persisted event cache should be readable");
        events.extend(page.events);
        match page.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(events.len(), 2, "both synced events are persisted");
    assert!(
        events.iter().any(|event| event
            .event_id()
            .is_some_and(|id| id.as_str() == "$with-attachment")),
        "the paged read returns the room's events"
    );

    // A page budget stops at a chunk boundary and hands out a cursor, so a
    // caller never holds the store lock for a whole room.
    let first = koushi_sdk::persisted_room_event_page(&session, room_id.as_str(), None, 1)
        .await
        .expect("bounded page");
    assert_eq!(first.events.len(), 2, "one chunk is returned whole");
    let next = first.next.expect("a bounded page hands out a cursor");
    let last = koushi_sdk::persisted_room_event_page(&session, room_id.as_str(), Some(next), 1)
        .await
        .expect("continuation page");
    assert!(last.events.is_empty() && last.next.is_none());

    let messages = attachment_messages_from_events(
        room_id.as_str(),
        &events,
        &SearchCrawlerSettings::default(),
    );
    let attachments: Vec<_> = messages
        .iter()
        .filter_map(|message| match message {
            SearchIndexMessage::Upsert {
                event_id,
                attachment: Some(attachment),
                attachment_filename,
                ..
            } => Some((
                event_id.clone(),
                attachment.clone(),
                attachment_filename.clone(),
            )),
            _ => None,
        })
        .collect();

    assert_eq!(
        attachments.len(),
        1,
        "only the media message carries attachment metadata"
    );
    assert!(
        messages.iter().any(|message| matches!(
            message,
            SearchIndexMessage::Upsert {
                event_id,
                attachment: None,
                ..
            } if event_id == "$plain"
        )),
        "a message without an attachment projects without one and is not retained"
    );
    assert_eq!(attachments[0].0, "$with-attachment");
    assert_eq!(attachments[0].1.filename.as_str(), "agenda.pdf");
    assert_eq!(attachments[0].2.as_deref(), Some("agenda.pdf"));
}

#[test]
fn the_files_refresh_applies_the_content_policy() {
    let media = timeline_event_from_json(serde_json::json!({
        "type": "m.room.message",
        "event_id": "$image:test",
        "room_id": "!r:test",
        "sender": "@alice:test",
        "origin_server_ts": 1_000,
        "content": {
            "msgtype": "m.image",
            "body": "agenda.pdf",
            "url": "mxc://example.invalid/agenda",
        },
    }));

    let with_filenames = SearchCrawlerSettings {
        include_media_captions: true,
        include_filenames: true,
        ..SearchCrawlerSettings::default()
    };
    let without_filenames = SearchCrawlerSettings {
        include_media_captions: true,
        include_filenames: false,
        ..SearchCrawlerSettings::default()
    };

    let rows = |settings| {
        attachment_messages_from_events("!r:test", std::slice::from_ref(&media), settings)
            .into_iter()
            .filter(|message| {
                matches!(
                    message,
                    SearchIndexMessage::Upsert {
                        attachment: Some(_),
                        ..
                    }
                )
            })
            .count()
    };

    assert_eq!(
        rows(&with_filenames),
        1,
        "a filename opt-in yields a Files row"
    );
    assert_eq!(
        rows(&without_filenames),
        0,
        "an opted-out filename must not reach the Files view"
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
fn the_files_refresh_ignores_a_replacement_from_another_sender() {
    let settings = SearchCrawlerSettings::default();
    let original = timeline_event_from_json(serde_json::json!({
        "type": "m.room.message",
        "event_id": "$photo:test",
        "room_id": "!r:test",
        "sender": "@alice:test",
        "origin_server_ts": 1_000,
        "content": {
            "msgtype": "m.file",
            "body": "original.pdf",
            "url": "mxc://example.invalid/original",
        },
    }));
    let replacement = |sender: &str| {
        timeline_event_from_json(serde_json::json!({
            "type": "m.room.message",
            "event_id": "$edit:test",
            "room_id": "!r:test",
            "sender": sender,
            "origin_server_ts": 2_000,
            "content": {
                "msgtype": "m.file",
                "body": "attacker.pdf",
                "url": "mxc://example.invalid/attacker",
                "m.relates_to": {"rel_type": "m.replace", "event_id": "$photo:test"},
                "m.new_content": {
                    "msgtype": "m.file",
                    "body": "attacker.pdf",
                    "url": "mxc://example.invalid/attacker",
                },
            },
        }))
    };

    let filenames = |sender: &str| {
        attachment_messages_from_events(
            "!r:test",
            &[original.clone(), replacement(sender)],
            &settings,
        )
        .into_iter()
        .filter_map(|message| match message {
            SearchIndexMessage::Edit {
                attachment_filename,
                ..
            } => attachment_filename,
            _ => None,
        })
        .collect::<Vec<_>>()
    };

    assert_eq!(
        filenames("@mallory:test"),
        Vec::<String>::new(),
        "another sender's replacement must not reach the row"
    );
    assert_eq!(filenames("@alice:test"), vec!["attacker.pdf".to_owned()]);
}

fn timeline_event_from_json(
    json: serde_json::Value,
) -> matrix_sdk::deserialized_responses::TimelineEvent {
    matrix_sdk::deserialized_responses::TimelineEvent::from_plaintext(
        matrix_sdk::ruma::serde::Raw::from_json_string(json.to_string()).expect("raw event"),
    )
}

#[test]
fn forgetting_a_room_replaces_its_rows_on_rebuild() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(
        make_attachment_event("!a:test", "$e1", "original.pdf"),
        true,
        None,
    );
    store.upsert_message(
        make_attachment_event("!b:test", "$e2", "other.pdf"),
        true,
        None,
    );
    store.upsert_edit(make_edit_at("$e1", "$edit1", 2_000, "renamed.pdf"), true);
    let room_filename = |store: &SearchDocumentStore, room_id: &str| {
        attachment_rows(store)
            .into_iter()
            .find(|row| row.room_id == room_id)
            .map(|row| row.filename)
    };
    assert_eq!(
        room_filename(&store, "!a:test").as_deref(),
        Some("renamed.pdf")
    );

    // The Files rebuild replaces the queried room's rows from the current cache.
    store.forget_room("!a:test");
    assert_eq!(
        attachment_rows(&store).len(),
        1,
        "only the other room remains"
    );
    assert_eq!(room_filename(&store, "!a:test"), None);
    assert_eq!(
        room_filename(&store, "!b:test").as_deref(),
        Some("other.pdf")
    );

    // What the cache still produces (the original, the rename having been
    // redacted) is what the rebuilt row shows.
    store.upsert_message(
        make_attachment_event("!a:test", "$e1", "original.pdf"),
        false,
        None,
    );
    assert_eq!(
        room_filename(&store, "!a:test").as_deref(),
        Some("original.pdf")
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

/// An edit as a producer would report it: the edit event's own time and id.
fn make_edit_at(
    target: &str,
    edit_event_id: &str,
    timestamp_ms: u64,
    filename: &str,
) -> SearchEdit {
    SearchEdit {
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
    store.upsert_edit(make_edit("$e1", "a new caption"), true);

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
