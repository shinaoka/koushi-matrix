use super::*;
use koushi_state::{AttachmentKind, SessionAuthenticationMethod, SessionInfo};
use matrix_sdk::test_utils::mocks::MatrixMockServer;
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};

const ROOM: &str = "!admission:example.invalid";
const TARGET: &str = "$original";

async fn fixture(joined: bool) -> (MatrixMockServer, SearchActor) {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    client.event_cache().subscribe().unwrap();
    if joined {
        join(&server, &client).await;
    }
    let session = MatrixClientSession::from_client_for_testing(
        client.clone(),
        SessionInfo {
            homeserver: server.server().uri(),
            user_id: client.user_id().unwrap().to_string(),
            device_id: client.device_id().unwrap().to_string(),
            authentication_method: SessionAuthenticationMethod::Unknown,
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
    actor.crawler_settings.include_filenames = true;
    (server, actor)
}

#[tokio::test]
async fn files_submission_adopts_policy_before_deferred_crawler_notification() {
    let (_, mut actor) = fixture(true).await;
    actor.handle_index(upsert(None)).await;
    assert_eq!(actor.document_store.document_count(), 1);
    let mut events = actor.event_tx.subscribe();
    let (tx, mut rx) = mpsc::channel(1);
    let (index_tx, _) = mpsc::channel(1);
    let handle = SearchActorHandle {
        tx,
        index_tx,
        task: None,
    };
    let mut policy = actor.crawler_settings.clone();
    policy.include_filenames = false;
    let request_id = RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(9),
        sequence: 1,
    };
    assert!(
        handle
            .send_query_command(
                SearchCommand::Attachments {
                    request_id,
                    scope: AttachmentScope::Account,
                    filter: AttachmentFilter::default(),
                    sort: AttachmentSort::NewestFirst,
                },
                Some(policy)
            )
            .await
    );
    actor.handle_actor_message(rx.recv().await.unwrap()).await;
    let CoreEvent::Search(SearchEvent::AttachmentsResults { results, .. }) =
        events.recv().await.unwrap()
    else {
        panic!("expected Files result")
    };
    assert!(
        results.is_empty(),
        "old-policy filenames must not be published"
    );
    assert!(!actor.crawler_settings.include_filenames);
}

async fn join(server: &MatrixMockServer, client: &matrix_sdk::Client) {
    server
        .mock_sync()
        .ok_and_run(client, |b| {
            b.add_joined_room(JoinedRoomBuilder::new(matrix_sdk::ruma::room_id!(
                "!admission:example.invalid"
            )));
        })
        .await;
}

async fn redact(server: &MatrixMockServer, actor: &SearchActor, id: &str) {
    let client = actor.session.client();
    let room = client
        .get_room(matrix_sdk::ruma::room_id!("!admission:example.invalid"))
        .unwrap();
    let (cache, _handles) = room.event_cache().await.unwrap();
    let (_, mut updates) = cache.subscribe().await.unwrap();
    let factory = EventFactory::new()
        .room(room.room_id())
        .sender(matrix_sdk::ruma::user_id!("@member:example.invalid"));
    let id = matrix_sdk::ruma::EventId::parse(id).unwrap();
    server
        .mock_sync()
        .ok_and_run(&client, |b| {
            b.add_joined_room(
                JoinedRoomBuilder::new(room.room_id())
                    .add_timeline_event(factory.redaction(&id).into_raw()),
            );
        })
        .await;
    executor::timeout(Duration::from_secs(2), async {
        loop {
            if cache
                .redacted_event_ids(std::slice::from_ref(&id))
                .await
                .unwrap()
                .contains(&id)
            {
                break;
            }
            updates.recv().await.unwrap();
        }
    })
    .await
    .unwrap();
}

fn media() -> AttachmentDocument {
    AttachmentDocument {
        kind: AttachmentKind::File,
        msgtype: "m.file".into(),
        mimetype: None,
        size: None,
        source_mxc: "mxc://example.invalid/source".into(),
        thumbnail_mxc: None,
        filename: SensitiveString::new(""),
        thread_root: None,
        encrypted: false,
        encryption_version: None,
        width: None,
        height: None,
        is_edited: false,
    }
}

fn upsert(edit: Option<(&str, u64)>) -> SearchIndexMessage {
    SearchIndexMessage::Upsert {
        room_id: ROOM.into(),
        event_id: TARGET.into(),
        sender: "@member:example.invalid".into(),
        timestamp_ms: 1,
        body: Some("synthetic caption".into()),
        attachment_filename: Some(edit.map_or("original.pdf", |(id, _)| id).into()),
        attachment: Some(media()),
        canonical: true,
        edit: edit.map(|(id, ts)| SearchEditKey::new(id, ts)),
    }
}

fn edit(id: &str, time: u64, attached: bool) -> SearchIndexMessage {
    SearchIndexMessage::Edit {
        room_id: ROOM.into(),
        edit_event_id: id.into(),
        target_event_id: TARGET.into(),
        sender: "@member:example.invalid".into(),
        timestamp_ms: time,
        body: Some("synthetic replacement text".into()),
        attachment_filename: attached.then(|| id.into()),
        attachment: attached.then(media),
        canonical: false,
    }
}

fn names(actor: &SearchActor) -> Vec<String> {
    actor
        .document_store
        .attachments(
            &AttachmentScope::Account,
            &AttachmentFilter::default(),
            AttachmentSort::NewestFirst,
        )
        .into_iter()
        .map(|r| r.filename)
        .collect()
}

#[tokio::test]
async fn absent_focused_bundled_events_and_edit_before_root_remain_admissible() {
    let (_, mut actor) = fixture(true).await;
    actor.handle_index(edit("$bundled", 3, true)).await;
    assert_eq!(actor.document_store.pending_edit_count(), 1);
    actor.handle_index(upsert(None)).await;
    assert_eq!(names(&actor), ["$bundled"]);
    assert!(actor.attachment_retries.is_empty());
    assert_eq!(actor.document_store.resident_body_bytes(), 0);
}

#[tokio::test]
async fn rollback_uses_actual_redaction_not_producer_history_or_timestamp() {
    let (server, mut actor) = fixture(true).await;
    actor.handle_index(upsert(Some(("$a", 2)))).await;
    actor.handle_index(edit("$b", 3, true)).await;
    actor.handle_index(upsert(Some(("$a", 2)))).await; // stale projection, not a redaction
    assert_eq!(names(&actor), ["$b"]);
    redact(&server, &actor, "$b").await;
    // The projection can come from a replacement/focused/thread actor: no map.
    actor.handle_index(upsert(Some(("$a", 2)))).await;
    assert_eq!(names(&actor), ["$a"]);
    actor.handle_index(edit("$b", 3, true)).await;
    assert_eq!(names(&actor), ["$a"]);
}

#[tokio::test]
async fn pending_edit_is_rechecked_at_consumption_and_other_versions_survive() {
    let (server, mut actor) = fixture(true).await;
    actor.handle_index(edit("$a", 2, true)).await;
    actor.handle_index(edit("$b", 3, true)).await;
    redact(&server, &actor, "$b").await;
    actor.handle_index(upsert(None)).await;
    assert_eq!(names(&actor), ["$a"]);
}

#[tokio::test]
async fn superseded_redactions_and_evicted_tombstones_still_refuse_replay() {
    let (server, mut actor) = fixture(true).await;
    actor.handle_index(upsert(None)).await;
    actor.handle_index(edit("$a", 2, true)).await;
    actor.handle_index(edit("$b", 3, true)).await;
    redact(&server, &actor, "$a").await; // no longer applied
    redact(&server, &actor, "$b").await;
    actor.handle_index(upsert(None)).await;
    actor.handle_index(edit("$a", 2, true)).await;
    assert_eq!(names(&actor), ["original.pdf"]);
    for i in 0..10 {
        actor
            .document_store
            .retire_edit(TARGET, &format!("$retired{i}"));
    }
    actor.handle_index(edit("$b", 3, true)).await;
    assert_eq!(names(&actor), ["original.pdf"]);
    redact(&server, &actor, TARGET).await;
    actor.handle_index(upsert(None)).await;
    assert!(names(&actor).is_empty());
}

#[tokio::test]
async fn failed_proof_retries_at_files_query_without_resubmission() {
    let (server, mut actor) = fixture(false).await;
    actor.handle_index(upsert(Some(("$b", 3)))).await;
    actor.handle_index(edit("$a", 2, true)).await; // must not replace B in retries
    assert_eq!(actor.attachment_retries.len(), 2);
    assert!(!actor.reconcile_attachment_redactions().await);
    assert_eq!(actor.attachment_retries.len(), 2);
    join(&server, &actor.session.client()).await;
    assert!(actor.reconcile_attachment_redactions().await);
    assert!(actor.attachment_retries.is_empty());
    assert_eq!(names(&actor), ["$b"]);
    assert_eq!(actor.document_store.resident_body_bytes(), 0);
}

#[tokio::test]
async fn files_read_rechecks_redaction_after_mutation_admission() {
    let (server, mut actor) = fixture(true).await;
    actor.handle_index(upsert(Some(("$b", 3)))).await;
    redact(&server, &actor, "$b").await;
    assert!(actor.reconcile_attachment_redactions().await);
    assert!(names(&actor).is_empty());
}

#[tokio::test]
async fn an_earlier_crawl_page_text_edit_cannot_expose_the_original_file() {
    let (server, mut actor) = fixture(true).await;
    let client = actor.session.client();
    let room = client
        .get_room(matrix_sdk::ruma::room_id!("!admission:example.invalid"))
        .unwrap();
    let (cache, _handles) = room.event_cache().await.unwrap();
    let (_, mut updates) = cache.subscribe().await.unwrap();
    let original: matrix_sdk::ruma::serde::Raw<matrix_sdk::ruma::events::AnySyncTimelineEvent> =
        serde_json::from_value(serde_json::json!({
            "event_id": TARGET, "sender": "@member:example.invalid", "origin_server_ts": 1,
            "type": "m.room.message", "content": {"msgtype": "m.file", "body": "original.pdf", "url": "mxc://example.invalid/file"}
        })).unwrap();
    let factory = EventFactory::new()
        .room(room.room_id())
        .sender(matrix_sdk::ruma::user_id!("@member:example.invalid"));
    server.mock_sync().ok_and_run(&client, |b| {
        b.add_joined_room(JoinedRoomBuilder::new(room.room_id()).add_timeline_event(original)
            .add_timeline_event(factory.text_msg("synthetic text")
                .edit(matrix_sdk::ruma::event_id!("$original"), matrix_sdk::ruma::events::room::message::RoomMessageEventContentWithoutRelation::text_plain("synthetic text"))
                .event_id(matrix_sdk::ruma::event_id!("$text")).into_raw()));
    }).await;
    executor::timeout(Duration::from_secs(2), async {
        loop {
            if cache
                .find_event(matrix_sdk::ruma::event_id!("$text"))
                .await
                .unwrap()
                .is_some()
            {
                break;
            }
            updates.recv().await.unwrap();
        }
    })
    .await
    .unwrap();
    // The text edit page had no known attachment target at the time.
    actor.handle_index(edit("$text", 4, false)).await;
    actor.handle_index(upsert(None)).await; // older page later carries the file
    assert!(names(&actor).is_empty());
    assert_eq!(actor.document_store.resident_body_bytes(), 0);
}

#[tokio::test]
async fn crawled_foreign_sender_edit_is_rejected_before_or_after_root_admission() {
    for pending_first in [false, true] {
        let (server, mut actor) = fixture(true).await;
        let client = actor.session.client();
        let room = client
            .get_room(matrix_sdk::ruma::room_id!("!admission:example.invalid"))
            .unwrap();
        let (cache, _handles) = room.event_cache().await.unwrap();
        let (_, mut updates) = cache.subscribe().await.unwrap();
        let root_json = serde_json::json!({
            "event_id": TARGET, "sender": "@member:example.invalid", "origin_server_ts": 1,
            "type": "m.room.message", "content": {"msgtype": "m.file", "body": "original.pdf", "url": "mxc://example.invalid/file"}
        });
        let edit_json = serde_json::json!({
            "event_id": "$foreign", "sender": "@other:example.invalid", "origin_server_ts": 10,
            "type": "m.room.message", "content": {"msgtype": "m.file", "body": "* forged.bin", "url": "mxc://example.invalid/forged",
                "m.new_content": {"msgtype": "m.file", "body": "forged.bin", "url": "mxc://example.invalid/forged"},
                "m.relates_to": {"rel_type": "m.replace", "event_id": TARGET}}
        });
        let raw = |json: serde_json::Value| -> matrix_sdk::ruma::serde::Raw<
            matrix_sdk::ruma::events::AnySyncTimelineEvent,
        > { serde_json::from_value(json).unwrap() };
        server
            .mock_sync()
            .ok_and_run(&client, |b| {
                b.add_joined_room(
                    JoinedRoomBuilder::new(room.room_id())
                        .add_timeline_event(raw(root_json.clone()))
                        .add_timeline_event(raw(edit_json.clone())),
                );
            })
            .await;
        executor::timeout(Duration::from_secs(2), async {
            while cache
                .find_event(matrix_sdk::ruma::event_id!("$foreign"))
                .await
                .unwrap()
                .is_none()
            {
                updates.recv().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert_eq!(
            koushi_sdk::resolve_cached_message(&actor.session, ROOM, TARGET)
                .await
                .unwrap()
                .unwrap()
                .attachment_filename
                .as_deref(),
            Some("original.pdf")
        );
        let mut pending_redactions = HashSet::new();
        let root = crate::search_crawler::event_json_to_index_message(
            ROOM,
            &root_json.to_string(),
            &actor.crawler_settings,
            &mut pending_redactions,
        )
        .unwrap();
        let edit = crate::search_crawler::event_json_to_index_message(
            ROOM,
            &edit_json.to_string(),
            &actor.crawler_settings,
            &mut pending_redactions,
        )
        .unwrap();
        let messages = if pending_first {
            [edit, root]
        } else {
            [root, edit]
        };
        for message in messages {
            actor.handle_index(message).await;
        }
        assert!(actor.reconcile_attachment_redactions().await);
        assert_eq!(names(&actor), ["original.pdf"]);
    }
}

#[tokio::test]
async fn queued_crawl_text_replacement_keeps_no_body_and_removes_media() {
    let (_, mut actor) = fixture(true).await;
    actor.queue_crawl_messages(vec![
        edit("$text", 4, false),
        edit("$media", 3, true),
        upsert(None),
    ]);
    assert_eq!(actor.queued_crawl_index.len(), 3);
    assert!(actor.reconcile_attachment_redactions().await);
    assert!(names(&actor).is_empty());
    assert_eq!(actor.document_store.resident_body_bytes(), 0);
}
