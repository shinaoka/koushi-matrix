use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    thread,
    time::Duration,
};

use koushi_sdk::{MatrixClientStoreConfig, MatrixClientStoreKey};
use koushi_state::AuthSecret;
use koushi_state::LoginRequest;
use matrix_sdk::deserialized_responses::TimelineEvent;
use matrix_sdk::ruma::{
    RoomId,
    events::{AnySyncMessageLikeEvent, AnySyncTimelineEvent},
    serde::Raw,
};
use matrix_sdk_base::event_cache::store::EventCacheStore;

const MATRIX_VERSIONS_RESPONSE: &str = r#"{"versions":["r0.6.0","v1.1","v1.2","v1.3","v1.4","v1.5","v1.6","v1.7"],"unstable_features":{}}"#;

#[test]
fn event_cache_sqlite_store_is_not_plaintext_and_rejects_wrong_key() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let cache_dir = tempfile::tempdir().expect("cache tempdir should be created");
        let correct_key = [11; 32];
        let store_config =
            matrix_sdk::SqliteStoreConfig::new(cache_dir.path()).key(Some(&correct_key));
        let store = matrix_sdk::SqliteEventCacheStore::open_with_config(&store_config)
            .await
            .expect("encrypted event cache store should open");

        let room_id = RoomId::parse("!cache-room:example.invalid").expect("room id");
        let event_id = "$cache-event:example.invalid";
        let body = "persistent cache payload";
        let event = make_message_event(room_id.as_str(), event_id, body, 1);

        store
            .save_event(&room_id, event.clone())
            .await
            .expect("event should be persisted");

        let cached_event = store
            .find_event(&room_id, &event.event_id().expect("event id"))
            .await
            .expect("lookup should succeed")
            .expect("saved event should be found");
        assert_eq!(cached_event.event_id(), event.event_id());

        assert_cache_dir_does_not_contain(cache_dir.path(), body.as_bytes());

        drop(store);

        let reopened = matrix_sdk::SqliteEventCacheStore::open_with_config(&store_config)
            .await
            .expect("same key should reopen the encrypted event cache store");
        let persisted = reopened
            .get_room_events(&room_id, None, None)
            .await
            .expect("room events should load");
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].event_id(), event.event_id());

        drop(reopened);

        let wrong_key = [12; 32];
        let wrong_store_config =
            matrix_sdk::SqliteStoreConfig::new(cache_dir.path()).key(Some(&wrong_key));
        let wrong_error = matrix_sdk::SqliteEventCacheStore::open_with_config(&wrong_store_config)
            .await
            .expect_err("wrong key should reject the encrypted event cache store");
        assert!(!wrong_error.to_string().contains(body));
        assert!(!format!("{wrong_error:?}").contains(body));
    });
}

#[test]
fn enable_event_cache_is_idempotent() {
    let homeserver = spawn_password_login_server();
    let request = LoginRequest {
        homeserver,
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };
    let store_dir = tempfile::tempdir().expect("store tempdir should be created");
    let cache_dir = tempfile::tempdir().expect("cache tempdir should be created");
    let store_config =
        MatrixClientStoreConfig::new(store_dir.path(), MatrixClientStoreKey::new([11; 32]))
            .with_cache_path(cache_dir.path());

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let session = koushi_sdk::login_with_password_with_store(&request, Some(&store_config))
            .await
            .expect("password login with encrypted store should succeed");

        assert!(!session.client().event_cache().has_subscribed());

        let first = koushi_sdk::enable_event_cache(&session)
            .await
            .expect("event cache subscription should succeed");
        let second = koushi_sdk::enable_event_cache(&session)
            .await
            .expect("event cache subscription should remain idempotent");

        assert_eq!(first, koushi_sdk::MatrixEventCacheStatus::Enabled);
        assert_eq!(second, koushi_sdk::MatrixEventCacheStatus::AlreadyEnabled);
        assert!(session.client().event_cache().has_subscribed());
    });
}

#[test]
fn event_cache_persists_across_drop_and_restore() {
    let homeserver = spawn_password_login_server_with_sync();
    let request = LoginRequest {
        homeserver,
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };
    let store_dir = tempfile::tempdir().expect("store tempdir should be created");
    let cache_dir = tempfile::tempdir().expect("cache tempdir should be created");
    let store_config =
        MatrixClientStoreConfig::new(store_dir.path(), MatrixClientStoreKey::new([11; 32]))
            .with_cache_path(cache_dir.path());
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let session = koushi_sdk::login_with_password_with_store(&request, Some(&store_config))
            .await
            .expect("password login with encrypted store should succeed");
        let _ = koushi_sdk::enable_event_cache(&session)
            .await
            .expect("event cache should enable before sync");
        koushi_sdk::sync_once(&session)
            .await
            .expect("sync should succeed");

        let persistable = session
            .persistable_session()
            .expect("session should be persistable");
        drop(session);

        let restored = koushi_sdk::restore_session_with_store(&persistable, Some(&store_config))
            .await
            .expect("store-backed session should restore");
        let status = koushi_sdk::enable_event_cache(&restored)
            .await
            .expect("restored session should enable the event cache");
        assert_eq!(status, koushi_sdk::MatrixEventCacheStatus::Enabled);

        let room_id = RoomId::parse("!persisted-room:example.invalid").expect("room id");
        let room = restored
            .client()
            .get_room(&room_id)
            .expect("room should be restored from sync state");
        let (room_event_cache, _drop_handles) = room.event_cache().await.expect("room cache");
        let expected_body = "persistent cache payload";
        let (events, _subscriber) = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let (events, subscriber) = room_event_cache
                    .subscribe()
                    .await
                    .expect("room event cache should subscribe");

                if events
                    .iter()
                    .any(|event| message_event_body(event).as_deref() == Some(expected_body))
                {
                    break (events, subscriber);
                }

                drop(subscriber);
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("timed out waiting for restored room event cache");

        assert!(
            events
                .iter()
                .any(|event| message_event_body(event).as_deref() == Some(expected_body)),
            "restored event cache should contain the expected synthetic message"
        );
    });
}

fn make_message_event(room_id: &str, event_id: &str, body: &str, timestamp: u64) -> TimelineEvent {
    let raw = Raw::from_json_string(format!(
        r#"{{
                "content": {{"body": "{body}", "msgtype": "m.text"}},
                "event_id": "{event_id}",
                "origin_server_ts": {timestamp},
                "room_id": "{room_id}",
                "sender": "@fixture-user:example.invalid",
                "type": "m.room.message"
            }}"#
    ))
    .expect("raw event should parse");

    TimelineEvent::from_plaintext(raw)
}

fn message_event_body(event: &TimelineEvent) -> Option<String> {
    match event
        .raw()
        .deserialize()
        .expect("cached event should deserialize")
    {
        AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomMessage(message)) => {
            Some(message.as_original()?.content.body().to_owned())
        }
        _ => None,
    }
}

fn assert_cache_dir_does_not_contain(dir: &Path, needle: &[u8]) {
    for entry in std::fs::read_dir(dir).expect("cache dir should be readable") {
        let entry = entry.expect("cache entry should be readable");
        let path = entry.path();
        if path.is_dir() {
            assert_cache_dir_does_not_contain(&path, needle);
            continue;
        }

        let bytes = std::fs::read(&path).expect("cache file should be readable");
        assert!(
            !bytes.windows(needle.len()).any(|window| window == needle),
            "cache file {path:?} leaked plaintext payload"
        );
    }
}

fn spawn_password_login_server() -> String {
    spawn_password_login_server_with_options(None)
}

fn spawn_password_login_server_with_sync() -> String {
    spawn_password_login_server_with_options(Some(sync_response()))
}

fn spawn_password_login_server_with_options(sync_response: Option<String>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let addr = listener
        .local_addr()
        .expect("test server should have an address");

    thread::spawn(move || {
        for _ in 0..32 {
            let (mut stream, _) = listener
                .accept()
                .expect("test server should accept a request");
            let request = read_http_request(&mut stream);

            if request.starts_with("GET /_matrix/client/versions ") {
                write_json(&mut stream, 200, MATRIX_VERSIONS_RESPONSE);
                continue;
            }

            if write_common_sdk_bootstrap_response(&mut stream, &request) {
                continue;
            }

            if request.starts_with("GET /_matrix/client/") && request.contains("/sync") {
                if let Some(sync_response) = &sync_response {
                    write_json(&mut stream, 200, sync_response);
                    continue;
                }
            }

            if request.starts_with("POST /_matrix/client/")
                && request.contains("fixture-user")
                && request.contains("synthetic-password")
                && request.contains("Matrix Desktop Test")
            {
                write_json(
                    &mut stream,
                    200,
                    r#"{"access_token":"fixture-access-token","device_id":"FIXTUREDEVICE","user_id":"@fixture-user:example.invalid"}"#,
                );
                continue;
            }

            write_json(
                &mut stream,
                404,
                r#"{"errcode":"M_NOT_FOUND","error":"Unexpected test request"}"#,
            );
        }
    });

    format!("http://{addr}")
}

fn write_common_sdk_bootstrap_response(stream: &mut std::net::TcpStream, request: &str) -> bool {
    if request.starts_with("GET /_matrix/client/")
        && request.contains("/account_data/m.secret_storage.default_key")
    {
        write_json(
            stream,
            404,
            r#"{"errcode":"M_NOT_FOUND","error":"No default secret storage key"}"#,
        );
        return true;
    }

    if request.starts_with("POST /_matrix/client/") && request.contains("/keys/upload") {
        write_json(stream, 200, r#"{"one_time_key_counts":{}}"#);
        return true;
    }

    if request.starts_with("POST /_matrix/client/") && request.contains("/keys/query") {
        write_json(stream, 200, r#"{"device_keys":{},"failures":{}}"#);
        return true;
    }

    false
}

fn sync_response() -> String {
    format!(
        r#"{{
            "device_one_time_keys_count": {{}},
            "next_batch": "sync-batch-event-cache-1",
            "device_lists": {{"changed": [], "left": []}},
            "rooms": {{
                "invite": {{}},
                "join": {{
                    "!persisted-room:example.invalid": {{
                        "summary": {{}},
                        "state": {{
                            "events": [
                                {{
                                    "content": {{"room_version": "10"}},
                                    "event_id": "$persisted-room-create",
                                    "origin_server_ts": 1,
                                    "sender": "@fixture-user:example.invalid",
                                    "state_key": "",
                                    "type": "m.room.create"
                                }}
                            ]
                        }},
                        "timeline": {{
                            "events": [
                                {{
                                    "content": {{"body": "persistent cache payload", "msgtype": "m.text"}},
                                    "event_id": "$cache-event:example.invalid",
                                    "origin_server_ts": 2,
                                    "sender": "@fixture-user:example.invalid",
                                    "type": "m.room.message"
                                }}
                            ],
                            "limited": false,
                            "prev_batch": "cache-prev"
                        }},
                        "ephemeral": {{"events": []}},
                        "account_data": {{"events": []}},
                        "unread_notifications": {{"highlight_count": 0, "notification_count": 0}}
                    }}
                }},
                "leave": {{}},
                "knock": {{}}
            }},
            "to_device": {{"events": []}},
            "presence": {{"events": []}},
            "account_data": {{"events": []}}
        }}"#
    )
}

fn read_http_request(stream: &mut std::net::TcpStream) -> String {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];

    loop {
        let bytes_read = stream
            .read(&mut buffer)
            .expect("test server should read request");
        if bytes_read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..bytes_read]);

        let request_text = String::from_utf8_lossy(&request);
        let Some(header_end) = request_text.find("\r\n\r\n") else {
            continue;
        };
        let content_length = request_text
            .lines()
            .find_map(|line| {
                line.strip_prefix("Content-Length: ")
                    .and_then(|value| value.parse::<usize>().ok())
            })
            .unwrap_or(0);
        if request.len() >= header_end + 4 + content_length {
            break;
        }
    }

    String::from_utf8(request).expect("test request should be UTF-8")
}

fn write_json(stream: &mut std::net::TcpStream, status: u16, body: &str) {
    let response = format!(
        "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .expect("test server should write response");
}
