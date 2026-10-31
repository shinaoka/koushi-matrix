//! Website search screenshots (#site): the synthetic mixed-language corpus runs
//! through the production index and query path, and the verified results are the
//! golden file the screenshot lane renders. A behavior change that alters what the
//! website shows fails here first.
use super::*;
use koushi_state::{SessionAuthenticationMethod, SessionInfo};
use matrix_sdk::test_utils::mocks::MatrixMockServer;
use matrix_sdk_test::JoinedRoomBuilder;
use serde::Deserialize;
use std::collections::BTreeMap;

const CORPUS: &str = include_str!("../../../../site/fixtures/search-corpus.json");
const GOLDEN: &str = include_str!("../../../../site/generated/search-scenes.json");
const GOLDEN_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../site/generated/search-scenes.json"
);

#[derive(Deserialize)]
struct Corpus {
    rooms: Vec<CorpusRoom>,
    scenes: Vec<Scene>,
}

#[derive(Deserialize)]
struct CorpusRoom {
    room_id: String,
    messages: Vec<CorpusMessage>,
}

#[derive(Deserialize)]
struct CorpusMessage {
    event_id: String,
    sender: String,
    origin_server_ts: u64,
    body: String,
}

#[derive(Deserialize)]
struct Scene {
    id: String,
    query: String,
    expect_rooms_min: usize,
    expect_event_ids: Vec<String>,
}

#[derive(serde::Serialize)]
struct SceneResults {
    query: String,
    results: Vec<koushi_state::SearchResult>,
}

async fn run_scenes() -> BTreeMap<String, SceneResults> {
    let corpus: Corpus = serde_json::from_str(CORPUS).expect("corpus parses");
    let index = tempfile::tempdir().unwrap();
    let sdk = tempfile::tempdir().unwrap();
    let cache_dir = tempfile::tempdir().unwrap();
    let server = MatrixMockServer::new().await;
    let client = server
        .client_builder()
        .on_builder(|b| {
            b.sqlite_store_with_cache_path(
                sdk.path(),
                cache_dir.path(),
                Some("synthetic-sdk-secret"),
            )
            .search_index_store(
                matrix_sdk::search_index::SearchIndexStoreKind::encrypted_directory_ngram(
                    index.path().to_owned(),
                    "synthetic-index-secret".into(),
                    1,
                    2,
                )
                .unwrap(),
            )
        })
        .build()
        .await;
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client.clone(),
        SessionInfo {
            homeserver: server.server().uri(),
            user_id: client.user_id().unwrap().to_string(),
            device_id: client.device_id().unwrap().to_string(),
            authentication_method: SessionAuthenticationMethod::Unknown,
        },
    ));

    let mut room_ids = Vec::new();
    for room in &corpus.rooms {
        let rid = matrix_sdk::ruma::RoomId::parse(&room.room_id).unwrap();
        server.sync_joined_room(&client, &rid).await;
        let mut events = Vec::new();
        let mut builder = JoinedRoomBuilder::new(&rid);
        for message in &room.messages {
            let event = matrix_sdk_test::event_factory::EventFactory::new()
                .room(&rid)
                .sender(matrix_sdk::ruma::UserId::parse(&message.sender).unwrap())
                .text_msg(message.body.as_str())
                .event_id(&matrix_sdk::ruma::EventId::parse(&message.event_id).unwrap())
                .server_ts(message.origin_server_ts)
                .into_event();
            builder = builder.add_timeline_event(event.raw().clone());
            events.push(event);
        }
        server.sync_room(&client, builder).await;
        koushi_sdk::index_room_events_now(&session, rid.as_str(), events)
            .await
            .unwrap();
        room_ids.push(room.room_id.clone());
    }

    let mut scenes = BTreeMap::new();
    for (sequence, scene) in corpus.scenes.iter().enumerate() {
        let request_id = RequestId {
            connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
            sequence: sequence as u64,
        };
        let outcome = run_sdk_query(
            session.clone(),
            1,
            request_id,
            scene.query.clone(),
            SearchScope::AllRooms,
            SearchRoomFilter::OnlyRooms(room_ids.clone()),
            koushi_sdk::MatrixSearchScope::AllRooms,
            SearchCrawlerSettings::default(),
            0,
            cjk_search_query_variants(&scene.query),
        )
        .await;
        let results = match outcome.projection {
            Ok(projection) => projection.results,
            Err(_) => panic!("scene {} query failed", scene.id),
        };
        let hit_rooms: HashSet<&str> = results.iter().map(|r| r.room_id.as_str()).collect();
        assert!(
            hit_rooms.len() >= scene.expect_rooms_min,
            "scene {} hit {} rooms, expected at least {}",
            scene.id,
            hit_rooms.len(),
            scene.expect_rooms_min
        );
        for expected in &scene.expect_event_ids {
            assert!(
                results.iter().any(|r| &r.event_id == expected),
                "scene {} is missing {expected}",
                scene.id
            );
        }
        scenes.insert(
            scene.id.clone(),
            SceneResults {
                query: scene.query.clone(),
                results,
            },
        );
    }
    scenes
}

fn render(scenes: &BTreeMap<String, SceneResults>) -> String {
    let mut json = serde_json::to_string_pretty(scenes).unwrap();
    json.push('\n');
    json
}

#[tokio::test]
async fn site_search_scenes_match_the_golden_results() {
    let scenes = run_scenes().await;
    assert_eq!(
        render(&scenes),
        GOLDEN,
        "site/generated/search-scenes.json is stale; run \
         `cargo test -p koushi-core write_site_search_golden -- --ignored`"
    );
}

#[tokio::test]
#[ignore = "writes site/generated/search-scenes.json; run explicitly after a corpus change"]
async fn write_site_search_golden() {
    let scenes = run_scenes().await;
    std::fs::write(GOLDEN_PATH, render(&scenes)).unwrap();
}
