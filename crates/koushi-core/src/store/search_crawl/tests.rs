use super::super::test_support::{file_store_actor, make_key_id};
use super::{
    CommittedRoomCrawl, CoreFailure, CrawlContentPolicy, SEARCH_CRAWL_BACKEND_VERSION,
    SearchCrawlProgress,
};
use tempfile::tempdir;

/// The restrictive policy most tests commit under.
fn policy() -> CrawlContentPolicy {
    CrawlContentPolicy::new(false, false)
}

fn committed(latest_event_id: &str, processed: u64, indexed: u64) -> CommittedRoomCrawl {
    CommittedRoomCrawl {
        latest_event_id: Some(latest_event_id.to_owned()),
        processed,
        indexed,
    }
}

#[test]
fn committed_crawls_survive_a_restart_without_plaintext_on_disk() {
    let data_dir = tempdir().expect("tempdir");
    let cred_dir = tempdir().expect("tempdir");
    let key_id = make_key_id();
    let actor = file_store_actor(&data_dir, &cred_dir);

    let mut progress = SearchCrawlProgress::new();
    progress.commit(
        "!room:test.example.com".to_owned(),
        committed("$e9", 12, 7),
        policy(),
    );
    actor
        .save_search_crawl_progress(&key_id, &progress)
        .expect("save");

    let path = actor.account_search_crawl_path(&key_id);
    let bytes = std::fs::read(&path).expect("persisted file");
    assert!(
        !bytes
            .windows(b"!room:test.example.com".len())
            .any(|window| window == b"!room:test.example.com"),
        "the stored crawl file must not contain a plaintext room id"
    );

    let reloaded = file_store_actor(&data_dir, &cred_dir)
        .load_search_crawl_progress(&key_id)
        .expect("load");
    assert_eq!(reloaded, progress);
    assert_eq!(reloaded.backend_version, SEARCH_CRAWL_BACKEND_VERSION);
}

#[test]
fn a_missing_file_is_an_empty_commit_set() {
    let data_dir = tempdir().expect("tempdir");
    let cred_dir = tempdir().expect("tempdir");
    let actor = file_store_actor(&data_dir, &cred_dir);

    let progress = actor
        .load_search_crawl_progress(&make_key_id())
        .expect("missing file is not a failure");

    assert!(progress.is_empty());
    assert!(progress.committed_rooms(policy()).is_empty());
}

#[test]
fn corruption_is_a_typed_store_failure() {
    let data_dir = tempdir().expect("tempdir");
    let cred_dir = tempdir().expect("tempdir");
    let key_id = make_key_id();
    let actor = file_store_actor(&data_dir, &cred_dir);
    let mut progress = SearchCrawlProgress::new();
    progress.commit(
        "!room:test.example.com".to_owned(),
        committed("$e9", 12, 7),
        policy(),
    );
    actor
        .save_search_crawl_progress(&key_id, &progress)
        .expect("save");

    let path = actor.account_search_crawl_path(&key_id);
    let mut bytes = std::fs::read(&path).expect("persisted file");
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    std::fs::write(&path, &bytes).expect("corrupt");

    assert_eq!(
        actor.load_search_crawl_progress(&key_id).err(),
        Some(CoreFailure::StoreUnavailable)
    );
}

#[test]
fn a_crawl_committed_under_another_version_is_ignored() {
    let mut stale = SearchCrawlProgress::new();
    stale.commit(
        "!room:test.example.com".to_owned(),
        committed("$e9", 12, 7),
        policy(),
    );
    stale.backend_version = SEARCH_CRAWL_BACKEND_VERSION - 1;

    assert!(
        stale.committed_rooms(policy()).is_empty(),
        "a commitment from an older contract must not be trusted"
    );

    // Committing again adopts the current version and drops the stale rooms.
    stale.commit(
        "!other:test.example.com".to_owned(),
        committed("$e1", 1, 1),
        policy(),
    );
    assert_eq!(stale.backend_version, SEARCH_CRAWL_BACKEND_VERSION);
    assert_eq!(stale.committed_rooms(policy()).len(), 1);
    assert!(
        stale
            .committed_rooms(policy())
            .contains_key("!other:test.example.com")
    );
}

#[test]
fn clearing_the_last_commit_removes_the_file() {
    let data_dir = tempdir().expect("tempdir");
    let cred_dir = tempdir().expect("tempdir");
    let key_id = make_key_id();
    let actor = file_store_actor(&data_dir, &cred_dir);
    let mut progress = SearchCrawlProgress::new();
    progress.commit(
        "!room:test.example.com".to_owned(),
        committed("$e9", 12, 7),
        policy(),
    );
    actor
        .save_search_crawl_progress(&key_id, &progress)
        .expect("save");

    assert!(progress.forget("!room:test.example.com"));
    actor
        .save_search_crawl_progress(&key_id, &progress)
        .expect("save empty");

    assert!(!actor.account_search_crawl_path(&key_id).exists());
    assert!(
        actor
            .load_search_crawl_progress(&key_id)
            .expect("load")
            .is_empty()
    );
}

#[test]
fn a_commitment_under_another_content_policy_is_ignored() {
    let captions = CrawlContentPolicy::new(true, true);
    let mut progress = SearchCrawlProgress::new();
    progress.commit(
        "!room:test.example.com".to_owned(),
        committed("$e9", 12, 7),
        captions,
    );

    assert_eq!(progress.content_policy, captions);
    assert_eq!(progress.committed_rooms(captions).len(), 1);
    assert!(
        progress
            .committed_rooms(CrawlContentPolicy::new(false, true))
            .is_empty(),
        "a commitment made while captions were searchable must not be trusted after they are turned off"
    );

    // Committing under the new policy drops the commitments made under the old one.
    progress.commit(
        "!other:test.example.com".to_owned(),
        committed("$e1", 1, 1),
        CrawlContentPolicy::new(false, true),
    );
    assert_eq!(progress.committed_rooms(captions).len(), 0);
    assert_eq!(
        progress
            .committed_rooms(CrawlContentPolicy::new(false, true))
            .len(),
        1
    );
}

#[test]
fn clearing_reports_whether_the_record_changed() {
    let mut progress = SearchCrawlProgress::new();
    assert!(!progress.clear(), "an empty record has nothing to clear");

    progress.commit(
        "!room:test.example.com".to_owned(),
        committed("$e9", 12, 7),
        policy(),
    );
    assert!(progress.clear());
    assert!(progress.is_empty());
    assert!(!progress.clear());
}

#[test]
fn forgetting_an_unknown_room_reports_no_change() {
    let mut progress = SearchCrawlProgress::new();
    progress.commit(
        "!room:test.example.com".to_owned(),
        committed("$e9", 12, 7),
        policy(),
    );

    assert!(!progress.forget("!absent:test.example.com"));
    assert!(progress.forget("!room:test.example.com"));
    assert!(progress.is_empty());
}
