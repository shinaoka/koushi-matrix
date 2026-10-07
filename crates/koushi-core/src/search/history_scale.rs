//! Synthetic disk/index evidence, separate from the pure store's byte probe.
use super::*;
use koushi_state::{SessionAuthenticationMethod, SessionInfo};
use matrix_sdk::{
    deserialized_responses::TimelineEvent,
    ruma::{events::AnySyncTimelineEvent, serde::Raw},
    test_utils::mocks::MatrixMockServer,
};
use matrix_sdk_test::JoinedRoomBuilder;

fn disk_bytes(path: &std::path::Path) -> u64 {
    std::fs::read_dir(path)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                disk_bytes(&entry.path())
            } else {
                entry.metadata().unwrap().len()
            }
        })
        .sum()
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "explicit history-scale encrypted-index evidence; not a latency benchmark"]
async fn encrypted_history_index_grows_without_resident_first_party_bodies() {
    assert_eq!(
        tokio::runtime::Handle::current().runtime_flavor(),
        tokio::runtime::RuntimeFlavor::CurrentThread
    );
    let directory = tempfile::tempdir().unwrap();
    let sdk_store = tempfile::tempdir().unwrap();
    let sdk_cache = tempfile::tempdir().unwrap();
    let server = MatrixMockServer::new().await;
    let kind = matrix_sdk::search_index::SearchIndexStoreKind::encrypted_directory_ngram(
        directory.path().to_owned(),
        "synthetic-index-secret".to_owned(),
        1,
        2,
    )
    .unwrap();
    let client = server
        .client_builder()
        .on_builder(|builder| {
            builder
                .sqlite_store_with_cache_path(
                    sdk_store.path(),
                    sdk_cache.path(),
                    Some("synthetic-sdk-secret"),
                )
                .search_index_store(kind)
        })
        .build()
        .await;
    client.event_cache().subscribe().unwrap();
    let room_id = matrix_sdk::ruma::room_id!("!scale:example.invalid");
    server
        .mock_sync()
        .ok_and_run(&client, |b| {
            b.add_joined_room(JoinedRoomBuilder::new(room_id));
        })
        .await;
    let session = MatrixClientSession::from_client_for_testing(
        client.clone(),
        SessionInfo {
            homeserver: server.server().uri(),
            user_id: client.user_id().unwrap().to_string(),
            device_id: client.device_id().unwrap().to_string(),
            authentication_method: SessionAuthenticationMethod::Unknown,
        },
    );
    let mut store = SearchDocumentStore::default();
    let body = "synthetic normalized history payload ".repeat(6);
    let mut count = 0;
    let mut previous_disk = 0;
    for stage in [1_000, 10_000, 50_000] {
        while count < stage {
            let end = (count + 1_000).min(stage);
            let mut events = Vec::new();
            for index in count..end {
                let id = format!("$scale{index}");
                let raw: Raw<AnySyncTimelineEvent> = serde_json::from_value(serde_json::json!({
                    "type": "m.room.message", "event_id": id, "sender": "@member:example.invalid", "origin_server_ts": index + 1,
                    "content": {"msgtype": "m.text", "body": body}
                })).unwrap();
                events.push(TimelineEvent::from_plaintext(raw));
                store.upsert_message(
                    SearchableEvent {
                        room_id: room_id.to_string(),
                        event_id: id,
                        sender: "@member:example.invalid".into(),
                        timestamp_ms: index + 1,
                        body: Some(SensitiveString::new(body.clone())),
                        attachment_filename: None,
                        attachment: None,
                    },
                    false,
                    None,
                );
            }
            {
                let cache_store = client.event_cache_store().lock().await.unwrap();
                for event in &events {
                    cache_store
                        .as_clean()
                        .unwrap()
                        .save_event(room_id, event.clone())
                        .await
                        .unwrap();
                }
            }
            koushi_sdk::index_room_events_now(&session, room_id.as_str(), events)
                .await
                .unwrap();
            count = end;
        }
        let bytes = disk_bytes(directory.path());
        assert!(bytes > previous_disk);
        previous_disk = bytes;
        assert_eq!(store.document_count(), 0);
        assert_eq!(store.pending_edit_count(), 0);
        assert_eq!(store.resident_body_bytes(), 0);
        let room = client.get_room(room_id).unwrap();
        let page = room.search_literal_page("history", 50, None).await.unwrap();
        assert_eq!(page.len(), 50);
        assert_eq!(page[0].event_id.as_str(), format!("$scale{}", stage - 1));
        let sdk_disk = disk_bytes(sdk_cache.path());
        println!(
            "history_scale indexed={stage} index_disk_bytes={bytes} sdk_cache_disk_bytes={sdk_disk} first_party_rows=0 first_party_body_bytes=0 async_runtime_workers=1; SDK residency and process RSS not measured; no latency claim"
        );
    }
}
