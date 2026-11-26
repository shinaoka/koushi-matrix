use super::intake::{MessageFacts, classify_message};
use super::*;
use crate::store::test_support::{file_store_actor, make_key_id};
use std::time::Duration;

fn candidate(event: &str, sender: &str) -> MentionCandidate {
    MentionCandidate {
        room_id: "!r:example.invalid".to_owned(),
        event_id: Some(event.to_owned()),
        sender: sender.to_owned(),
        timestamp_ms: now_ms(),
        thread_root: None,
        kind: MentionKind::Mention,
    }
}

fn memory_feed() -> MentionFeed {
    MentionFeed::open(None, 0, |_| true).expect("open")
}

async fn poll(feed: &MentionFeed, cursor: Option<&str>) -> Result<MentionPage, CursorRejection> {
    feed.wait(cursor, Duration::ZERO, 100).await
}

#[tokio::test]
async fn three_mentions_in_one_room_are_three_entries_in_order() {
    let feed = memory_feed();
    for event in ["$a", "$b", "$c"] {
        assert_eq!(
            feed.record(candidate(event, "@m:x")).await,
            RecordOutcome::Recorded
        );
    }
    let page = poll(&feed, None).await.expect("page");
    let events: Vec<_> = page
        .entries
        .iter()
        .map(|e| e.event_id.clone().unwrap())
        .collect();
    assert_eq!(events, ["$a", "$b", "$c"]);
    assert_eq!(
        page.entries.iter().map(|e| e.seq).collect::<Vec<_>>(),
        [1, 2, 3]
    );
}

#[tokio::test]
async fn a_cursor_resumes_after_what_was_read() {
    let feed = memory_feed();
    for event in ["$a", "$b", "$c"] {
        feed.record(candidate(event, "@m:x")).await;
    }
    let first = feed.wait(None, Duration::ZERO, 2).await.expect("first two");
    assert_eq!(first.entries.len(), 2);
    let rest = poll(&feed, Some(&first.cursor)).await.expect("third");
    assert_eq!(rest.entries.len(), 1);
    assert_eq!(rest.entries[0].event_id.as_deref(), Some("$c"));
    // Nothing newer: the cursor still comes back.
    let quiet = poll(&feed, Some(&rest.cursor)).await.expect("quiet");
    assert!(quiet.entries.is_empty());
    assert_eq!(quiet.cursor, rest.cursor);
}

#[tokio::test]
async fn a_replayed_event_is_one_row() {
    let feed = memory_feed();
    assert_eq!(
        feed.record(candidate("$a", "@m:x")).await,
        RecordOutcome::Recorded
    );
    assert_eq!(
        feed.record(candidate("$a", "@m:x")).await,
        RecordOutcome::Duplicate
    );
}

#[tokio::test]
async fn a_thread_mention_carries_its_root() {
    let feed = memory_feed();
    let mut threaded = candidate("$a", "@m:x");
    threaded.thread_root = Some("$root".to_owned());
    feed.record(threaded).await;
    let page = poll(&feed, None).await.expect("page");
    assert_eq!(page.entries[0].thread_root.as_deref(), Some("$root"));
}

#[tokio::test]
async fn a_long_poll_wakes_on_an_append_without_sampling() {
    let feed = memory_feed();
    let waiter = {
        let feed = feed.clone();
        tokio::spawn(async move { feed.wait(None, Duration::from_secs(30), 10).await })
    };
    tokio::task::yield_now().await;
    feed.record(candidate("$a", "@m:x")).await;
    let page = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .expect("woken by the append")
        .expect("join")
        .expect("page");
    assert_eq!(page.entries.len(), 1);
}

#[tokio::test]
async fn unusable_cursors_are_rejected() {
    let feed = memory_feed();
    feed.record(candidate("$a", "@m:x")).await;
    let other = memory_feed();
    other.record(candidate("$a", "@m:x")).await;
    let foreign = other.cursor_for(1);
    assert_eq!(
        poll(&feed, Some(&foreign)).await.unwrap_err(),
        CursorRejection::Foreign
    );
    let future = feed.cursor_for(9);
    assert_eq!(
        poll(&feed, Some(&future)).await.unwrap_err(),
        CursorRejection::Future
    );
    for bad in ["", "nonsense", "mf1.zz.1", "mf1.0123456789abcdef.x"] {
        assert_eq!(
            poll(&feed, Some(bad)).await.unwrap_err(),
            CursorRejection::Malformed,
            "{bad}"
        );
    }
}

#[tokio::test]
async fn evicted_rows_expire_an_old_cursor() {
    let feed = memory_feed();
    let start = feed.cursor_for(0);
    for index in 0..=MAX_ENTRIES {
        feed.record(candidate(&format!("$e{index}"), "@m:x")).await;
    }
    assert_eq!(
        poll(&feed, Some(&start)).await.unwrap_err(),
        CursorRejection::Expired
    );
    // A client without a cursor still gets what is retained.
    let page = feed
        .wait(None, Duration::ZERO, usize::MAX)
        .await
        .expect("page");
    assert_eq!(page.entries.len(), MAX_ENTRIES);
}

fn store_backed(actor: &crate::store::StoreActor, loaded: Option<MentionLogData>) -> MentionFeed {
    let save = actor.clone();
    let key_id = make_key_id();
    MentionFeed::open(loaded, 0, move |log| {
        save.save_mention_log(&key_id, log).is_ok()
    })
    .expect("open")
}

#[tokio::test]
async fn a_mention_survives_a_restart() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let cred_dir = tempfile::tempdir().expect("tempdir");
    let actor = file_store_actor(&data_dir, &cred_dir);
    let key_id = make_key_id();

    // A mention arrives while no client is connected.
    let first_run = store_backed(&actor, None);
    first_run.record(candidate("$a", "@m:x")).await;
    first_run.record(candidate("$b", "@m:x")).await;
    let seen = poll(&first_run, None).await.expect("page");
    let cursor_after_a = first_run.cursor_for(1);
    drop(first_run);

    // The process restarts: the log is read back from the encrypted store.
    let loaded = actor.load_mention_log(&key_id).expect("load").expect("log");
    let second_run = store_backed(&actor, Some(loaded));
    let page = poll(&second_run, None).await.expect("page");
    assert_eq!(page.entries, seen.entries);
    // A cursor issued before the restart still continues the same log.
    let resumed = poll(&second_run, Some(&cursor_after_a))
        .await
        .expect("resume");
    assert_eq!(resumed.entries.len(), 1);
    assert_eq!(resumed.entries[0].event_id.as_deref(), Some("$b"));
    // New mentions continue the sequence rather than reusing it.
    second_run.record(candidate("$c", "@m:x")).await;
    let all = poll(&second_run, None).await.expect("all");
    assert_eq!(
        all.entries.iter().map(|e| e.seq).collect::<Vec<_>>(),
        [1, 2, 3]
    );
}

#[tokio::test]
async fn the_log_is_encrypted_at_rest() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let cred_dir = tempfile::tempdir().expect("tempdir");
    let actor = file_store_actor(&data_dir, &cred_dir);
    let feed = store_backed(&actor, None);
    feed.record(candidate("$visible-event-id", "@sender-name:x"))
        .await;
    let path = actor.account_mention_log_file(&make_key_id());
    let bytes = std::fs::read(path).expect("read");
    for needle in ["$visible-event-id", "@sender-name:x"] {
        assert!(!bytes.windows(needle.len()).any(|w| w == needle.as_bytes()));
    }
}

fn facts<'a>() -> MessageFacts<'a> {
    MessageFacts {
        own_user_id: "@agent:x",
        sender: "@owner:x",
        is_replacement: false,
        push_highlight: false,
        names_account: false,
        is_direct_room: false,
    }
}

#[test]
fn classification_covers_mention_dm_and_the_muted_room() {
    assert_eq!(classify_message(&facts()), None);
    let highlighted = MessageFacts {
        push_highlight: true,
        ..facts()
    };
    assert_eq!(classify_message(&highlighted), Some(MentionKind::Mention));
    // A muted room hides the highlight from the push rules; `m.mentions`
    // still names the account.
    let muted = MessageFacts {
        names_account: true,
        ..facts()
    };
    assert_eq!(classify_message(&muted), Some(MentionKind::Mention));
    let dm = MessageFacts {
        is_direct_room: true,
        ..facts()
    };
    assert_eq!(classify_message(&dm), Some(MentionKind::Dm));
    let own = MessageFacts {
        sender: "@agent:x",
        push_highlight: true,
        ..facts()
    };
    assert_eq!(classify_message(&own), None);
    let edit = MessageFacts {
        is_replacement: true,
        push_highlight: true,
        ..facts()
    };
    assert_eq!(classify_message(&edit), None);
}

use std::sync::Arc;
use std::sync::atomic::Ordering;

fn registry_with_two_accounts(
    dir: &tempfile::TempDir,
) -> (
    Arc<MentionFeedRegistry>,
    MentionIntake,
    crate::settings::SettingsStore,
) {
    let settings = crate::settings::SettingsStore::new(dir.path());
    let registry = Arc::new(MentionFeedRegistry::new(settings.clone()));
    let agent = memory_feed();
    registry.account_started("@agent:x", agent.clone());
    registry.account_started("@owner:x", memory_feed());
    registry.set_persisted_accounts(["@agent:x", "@owner:x"]);
    let intake = MentionIntake::new(
        agent,
        registry.clone(),
        "@agent:x".parse().expect("user id"),
    );
    (registry, intake, settings)
}

async fn retained(registry: &MentionFeedRegistry) -> usize {
    let feed = registry.feed_for_user("@agent:x").expect("feed");
    poll(&feed, None).await.expect("page").entries.len()
}

#[tokio::test]
async fn only_allowed_senders_enter_the_feed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (registry, intake, settings) = registry_with_two_accounts(&dir);

    // The default list is the other signed-in accounts.
    intake.offer(candidate("$owner", "@owner:x")).await;
    intake.offer(candidate("$stranger", "@stranger:x")).await;
    assert_eq!(retained(&registry).await, 1);

    // Adding the sender makes a later mention from them appear.
    settings
        .save_patch(&koushi_state::SettingsPatch {
            mcp_allowed_senders: Some(koushi_state::MentionAllowedSenders {
                added: vec!["@stranger:x".to_owned()],
                removed: Vec::new(),
            }),
            ..Default::default()
        })
        .expect("save");
    intake.offer(candidate("$stranger2", "@stranger:x")).await;
    assert_eq!(retained(&registry).await, 2);

    // The owner can stop answering one of their own accounts.
    settings
        .save_patch(&koushi_state::SettingsPatch {
            mcp_allowed_senders: Some(koushi_state::MentionAllowedSenders {
                added: Vec::new(),
                removed: vec!["@owner:x".to_owned()],
            }),
            ..Default::default()
        })
        .expect("save");
    intake.offer(candidate("$owner2", "@owner:x")).await;
    assert_eq!(retained(&registry).await, 2);

    // Flipping the bridge switch (which sends only `enabled`) keeps the list.
    settings
        .save_patch(&koushi_state::SettingsPatch {
            mcp: Some(koushi_state::McpSettings::default()),
            ..Default::default()
        })
        .expect("save");
    intake.offer(candidate("$owner3", "@owner:x")).await;
    assert_eq!(retained(&registry).await, 2);
}

#[tokio::test]
async fn an_account_signed_in_later_is_covered_without_editing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (registry, intake, _settings) = registry_with_two_accounts(&dir);
    intake.offer(candidate("$late", "@third:x")).await;
    assert_eq!(retained(&registry).await, 0);
    registry.account_started("@third:x", memory_feed());
    intake.offer(candidate("$late2", "@third:x")).await;
    assert_eq!(retained(&registry).await, 1);
}

#[tokio::test]
async fn an_empty_log_is_saved_at_open_so_the_history_line_survives_a_restart() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let cred_dir = tempfile::tempdir().expect("tempdir");
    let actor = file_store_actor(&data_dir, &cred_dir);
    let first = store_backed(&actor, None);
    let cursor = first.cursor_for(0);
    let created = first.created_at_ms();
    drop(first);

    // No mention was ever recorded, yet the log exists on disk.
    let loaded = actor
        .load_mention_log(&make_key_id())
        .expect("load")
        .expect("saved at open");
    assert_eq!(loaded.created_at_ms, created);
    let second = store_backed(&actor, Some(loaded));
    assert_eq!(second.created_at_ms(), created);
    // A cursor from before the restart is still this log's, not foreign.
    assert!(poll(&second, Some(&cursor)).await.is_ok());
}

#[test]
fn a_feed_whose_first_save_fails_is_not_created() {
    assert!(MentionFeed::open(None, 0, |_| false).is_none());
}

#[tokio::test]
async fn a_cancelled_record_does_not_reuse_its_sequence_number() {
    use std::sync::mpsc;
    let (started_tx, started_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let fresh = MentionLogData::fresh("0123456789abcdef".to_owned(), 0);
    let feed = MentionFeed::open(Some(fresh), 0, move |_| {
        if calls.fetch_add(1, Ordering::SeqCst) == 0 {
            started_tx.send(()).expect("started");
            release_rx.lock().unwrap().recv().expect("release");
        }
        true
    })
    .expect("open");

    let first = tokio::spawn({
        let feed = feed.clone();
        async move { feed.record(candidate("$a", "@m:x")).await }
    });
    tokio::task::spawn_blocking(move || started_rx.recv().expect("write began"))
        .await
        .expect("join");
    // The caller is dropped while the write is in flight.
    first.abort();
    release_tx.send(()).expect("release");

    assert_eq!(
        feed.record(candidate("$b", "@m:x")).await,
        RecordOutcome::Recorded
    );
    let page = poll(&feed, None).await.expect("page");
    assert_eq!(
        page.entries.iter().map(|e| e.seq).collect::<Vec<_>>(),
        [1, 2]
    );
}

#[tokio::test]
async fn a_transient_write_failure_is_retried() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = calls.clone();
    let feed = MentionFeed::open(
        Some(MentionLogData::fresh("0123456789abcdef".to_owned(), 0)),
        0,
        move |_| counter.fetch_add(1, Ordering::SeqCst) >= 1,
    )
    .expect("open");
    assert_eq!(
        feed.record(candidate("$a", "@m:x")).await,
        RecordOutcome::Recorded
    );
    assert!(feed.is_healthy());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_persistent_write_failure_marks_the_feed_unhealthy_and_does_not_advance() {
    let feed = MentionFeed::open(
        Some(MentionLogData::fresh("0123456789abcdef".to_owned(), 0)),
        0,
        |_| false,
    )
    .expect("open");
    assert_eq!(
        feed.record(candidate("$a", "@m:x")).await,
        RecordOutcome::Failed
    );
    assert!(!feed.is_healthy());
    // Memory did not move ahead of the disk, and nothing is recorded after.
    assert!(poll(&feed, None).await.expect("page").entries.is_empty());
    assert_eq!(
        feed.record(candidate("$b", "@m:x")).await,
        RecordOutcome::Failed
    );
}

#[tokio::test]
async fn a_sender_whose_account_has_not_started_is_still_allowed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let settings = crate::settings::SettingsStore::new(dir.path());
    let registry = Arc::new(MentionFeedRegistry::new(settings));
    // Both accounts are in the persisted list; only the agent has started.
    registry.set_persisted_accounts(["@agent:x", "@owner:x"]);
    let agent = memory_feed();
    registry.account_started("@agent:x", agent.clone());
    let intake = MentionIntake::new(agent, registry.clone(), "@agent:x".parse().expect("id"));
    intake.offer(candidate("$backlog", "@owner:x")).await;
    assert_eq!(retained(&registry).await, 1);
    // Passing through a stop does not drop the account from the list.
    registry.account_stopped("@owner:x");
    intake.offer(candidate("$again", "@owner:x")).await;
    assert_eq!(retained(&registry).await, 2);
    intake.offer(candidate("$stranger", "@stranger:x")).await;
    assert_eq!(retained(&registry).await, 2);
}

#[tokio::test]
async fn rows_older_than_the_age_limit_expire_and_so_do_their_cursors() {
    let feed = memory_feed();
    let start = feed.cursor_for(0);
    let mut old = candidate("$old", "@m:x");
    old.timestamp_ms = now_ms() - MAX_AGE_MS - 60_000;
    feed.record(old).await;
    feed.record(candidate("$new", "@m:x")).await;
    let page = poll(&feed, None).await.expect("page");
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].event_id.as_deref(), Some("$new"));
    assert_eq!(
        poll(&feed, Some(&start)).await.unwrap_err(),
        CursorRejection::Expired
    );
}
