//! Approval lifecycle of the MCP consent broker (#1536).

use std::sync::Arc;
use std::time::Duration;

use koushi_mcp::wire::ErrorCode;
use koushi_mcp_host::consent::{
    APPROVAL_LIFETIME, APPROVAL_WAIT, AnswerOutcome, ClientIdentity, ConsentBroker, ConsentContext,
    ConsentMode, PENDING_PER_CLIENT,
};
use koushi_mcp_host::{Need, StoreStatus};

const ALICE: &str = "account:@alice:example.invalid";
const BOB: &str = "account:@bob:example.invalid";

fn client(name: &str) -> ClientIdentity {
    ClientIdentity {
        name: name.to_owned(),
        version: "1.0.0".to_owned(),
    }
}

fn broker(dir: &tempfile::TempDir) -> Arc<ConsentBroker> {
    ConsentBroker::new(dir.path().to_path_buf(), ConsentMode::Interactive)
}

/// Starts an authorization in the background and waits until it is pending.
async fn ask(
    broker: &Arc<ConsentBroker>,
    name: &str,
    account: &str,
    need: Need,
    context: ConsentContext,
) -> (
    tokio::task::JoinHandle<Result<(), koushi_mcp::wire::WireError>>,
    u64,
) {
    let mut state = broker.subscribe();
    let task = {
        let broker = Arc::clone(broker);
        let name = name.to_owned();
        let account = account.to_owned();
        tokio::spawn(async move {
            broker
                .authorize(&client(&name), &account, need, context)
                .await
        })
    };
    let request_id = loop {
        if let Some(pending) = state.borrow_and_update().pending.iter().find(|pending| {
            pending.client_name == name && pending.account_id == account && pending.need == need
        }) {
            break pending.request_id;
        }
        state.changed().await.expect("broker alive");
    };
    (task, request_id)
}

#[tokio::test]
async fn fail_closed_mode_answers_at_once_and_opens_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let broker = ConsentBroker::new(dir.path().to_path_buf(), ConsentMode::FailClosed);
    let error = broker
        .authorize(&client("pi"), ALICE, Need::Read, ConsentContext::default())
        .await
        .expect_err("no grant");
    assert_eq!(error.code, ErrorCode::ConsentRequired);
    assert!(broker.subscribe().borrow().pending.is_empty());
}

#[tokio::test]
async fn allow_grants_exactly_that_permission_and_lets_the_call_run() {
    let dir = tempfile::tempdir().expect("tempdir");
    let broker = broker(&dir);
    let (task, request_id) = ask(
        &broker,
        "claude-code",
        ALICE,
        Need::Read,
        ConsentContext::default(),
    )
    .await;

    assert_eq!(
        broker.answer(request_id, true).await,
        AnswerOutcome::Applied
    );
    task.await.expect("join").expect("allowed");

    let state = broker.refresh();
    assert!(state.pending.is_empty());
    assert_eq!(state.store, StoreStatus::Ok);
    let account = &state.assistants[0].accounts[0];
    assert_eq!(account.account_id, ALICE);
    assert!(account.read && !account.send && !account.attachments);

    // Granted now: no new approval for the same permission...
    broker
        .authorize(
            &client("claude-code"),
            ALICE,
            Need::Read,
            ConsentContext::default(),
        )
        .await
        .expect("granted");
    // ...but another account still asks.
    let (other, _) = ask(
        &broker,
        "claude-code",
        BOB,
        Need::Read,
        ConsentContext::default(),
    )
    .await;
    other.abort();
}

#[tokio::test]
async fn deny_is_remembered_until_the_permission_is_turned_on() {
    let dir = tempfile::tempdir().expect("tempdir");
    let broker = broker(&dir);
    let (task, request_id) = ask(
        &broker,
        "codex",
        ALICE,
        Need::Send,
        ConsentContext::default(),
    )
    .await;
    assert_eq!(
        broker.answer(request_id, false).await,
        AnswerOutcome::Applied
    );
    assert_eq!(
        task.await.expect("join").expect_err("denied").code,
        ErrorCode::NotPermitted
    );

    // A retry is refused without a new dialog.
    let error = broker
        .authorize(
            &client("codex"),
            ALICE,
            Need::Send,
            ConsentContext::default(),
        )
        .await
        .expect_err("still denied");
    assert_eq!(error.code, ErrorCode::NotPermitted);
    let state = broker.subscribe().borrow().clone();
    assert!(state.pending.is_empty());
    assert_eq!(state.assistants[0].accounts[0].denied, vec![Need::Send]);

    // The settings page undoes it.
    broker
        .set_permission("codex", ALICE, Need::Send, true)
        .await
        .expect("saved");
    broker
        .authorize(
            &client("codex"),
            ALICE,
            Need::Send,
            ConsentContext::default(),
        )
        .await
        .expect("granted from settings");
}

#[tokio::test(start_paused = true)]
async fn a_timed_out_call_leaves_the_approval_open_for_a_retry() {
    let dir = tempfile::tempdir().expect("tempdir");
    let broker = broker(&dir);
    let (task, request_id) = ask(&broker, "pi", ALICE, Need::Read, ConsentContext::default()).await;
    tokio::time::advance(APPROVAL_WAIT + Duration::from_secs(1)).await;
    assert_eq!(
        task.await.expect("join").expect_err("timed out").code,
        ErrorCode::ConsentRequired
    );
    let state = broker.subscribe().borrow().clone();
    assert_eq!(state.pending.len(), 1);
    assert!(!state.pending[0].waiting);

    assert_eq!(
        broker.answer(request_id, true).await,
        AnswerOutcome::Applied
    );
    broker
        .authorize(&client("pi"), ALICE, Need::Read, ConsentContext::default())
        .await
        .expect("the retry passes");
}

#[tokio::test]
async fn calls_for_the_same_permission_share_one_approval() {
    let dir = tempfile::tempdir().expect("tempdir");
    let broker = broker(&dir);
    let (first, request_id) =
        ask(&broker, "pi", ALICE, Need::Read, ConsentContext::default()).await;
    let (second, joined) = ask(&broker, "pi", ALICE, Need::Read, ConsentContext::default()).await;
    assert_eq!(request_id, joined);
    assert_eq!(broker.subscribe().borrow().pending.len(), 1);
    assert_eq!(
        broker.answer(request_id, true).await,
        AnswerOutcome::Applied
    );
    first.await.expect("join").expect("first allowed");
    second.await.expect("join").expect("second allowed");
}

#[tokio::test]
async fn a_stale_answer_changes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let broker = broker(&dir);
    let (task, request_id) = ask(&broker, "pi", ALICE, Need::Read, ConsentContext::default()).await;
    assert_eq!(
        broker.answer(request_id, false).await,
        AnswerOutcome::Applied
    );
    let _ = task.await;
    assert_eq!(broker.answer(request_id, true).await, AnswerOutcome::Stale);
    assert_eq!(broker.answer(999, true).await, AnswerOutcome::Stale);
    assert!(broker.refresh().assistants[0].accounts[0].denied == vec![Need::Read]);
    assert!(!broker.refresh().assistants[0].accounts[0].read);
}

#[tokio::test]
async fn one_client_cannot_hold_more_than_its_share_of_approvals() {
    let dir = tempfile::tempdir().expect("tempdir");
    let broker = broker(&dir);
    let mut tasks = Vec::new();
    for index in 0..PENDING_PER_CLIENT {
        let account = format!("account:@member{index}:example.invalid");
        tasks.push(
            ask(
                &broker,
                "pi",
                &account,
                Need::Read,
                ConsentContext::default(),
            )
            .await
            .0,
        );
    }
    let error = broker
        .authorize(&client("pi"), ALICE, Need::Read, ConsentContext::default())
        .await
        .expect_err("over the cap");
    assert_eq!(error.code, ErrorCode::Busy);
    // Another client still gets its own approval.
    let (other, _) = ask(
        &broker,
        "codex",
        ALICE,
        Need::Read,
        ConsentContext::default(),
    )
    .await;
    other.abort();
    for task in tasks {
        task.abort();
    }
}

#[tokio::test(start_paused = true)]
async fn an_unanswered_approval_expires() {
    let dir = tempfile::tempdir().expect("tempdir");
    let broker = broker(&dir);
    let (task, request_id) = ask(&broker, "pi", ALICE, Need::Read, ConsentContext::default()).await;
    let _ = task.await; // times out after APPROVAL_WAIT
    tokio::time::advance(APPROVAL_LIFETIME).await;
    broker.sweep();
    assert!(broker.subscribe().borrow().pending.is_empty());
    assert_eq!(broker.answer(request_id, true).await, AnswerOutcome::Stale);
}

#[tokio::test]
async fn stopping_the_bridge_releases_waiters_and_forgets_denials() {
    let dir = tempfile::tempdir().expect("tempdir");
    let broker = broker(&dir);
    let (denied, denied_id) = ask(&broker, "pi", BOB, Need::Read, ConsentContext::default()).await;
    broker.answer(denied_id, false).await;
    let _ = denied.await;
    let (task, _) = ask(&broker, "pi", ALICE, Need::Read, ConsentContext::default()).await;

    broker.reset();
    assert_eq!(
        task.await.expect("join").expect_err("stopped").code,
        ErrorCode::KoushiUnavailable
    );
    let state = broker.subscribe().borrow().clone();
    assert!(state.pending.is_empty());
    assert!(state.assistants.is_empty(), "denials are forgotten");
}

#[tokio::test]
async fn an_allow_that_cannot_be_saved_keeps_the_approval_open() {
    let dir = tempfile::tempdir().expect("tempdir");
    let broker = broker(&dir);
    let (task, request_id) = ask(&broker, "pi", ALICE, Need::Read, ConsentContext::default()).await;
    std::fs::create_dir_all(dir.path().join("mcp")).expect("mkdir");
    std::fs::write(
        dir.path().join("mcp/connections.json"),
        "{ hand edit in progress",
    )
    .expect("write");

    assert_eq!(
        broker.answer(request_id, true).await,
        AnswerOutcome::StoreFailed
    );
    assert_eq!(
        task.await.expect("join").expect_err("not saved").code,
        ErrorCode::Internal
    );
    let state = broker.subscribe().borrow().clone();
    assert_eq!(state.store, StoreStatus::Malformed);
    assert_eq!(state.pending.len(), 1);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("mcp/connections.json")).expect("read"),
        "{ hand edit in progress",
        "a file the app could not parse is never overwritten"
    );
}

#[tokio::test]
async fn a_caller_that_goes_away_leaves_no_message_behind() {
    let dir = tempfile::tempdir().expect("tempdir");
    let broker = broker(&dir);
    let context = ConsentContext {
        room_label: Some("Synthetic Workspace".to_owned()),
        message_preview: Some("I will send the file tomorrow.".to_owned()),
    };
    let (task, request_id) = ask(&broker, "pi", ALICE, Need::Send, context).await;
    assert!(
        broker.subscribe().borrow().pending[0]
            .message_preview
            .is_some()
    );

    task.abort();
    let _ = task.await;
    let state = broker.subscribe().borrow().clone();
    assert!(!state.pending[0].waiting);
    assert_eq!(state.pending[0].message_preview, None);
    assert_eq!(
        state.pending[0].room_label.as_deref(),
        Some("Synthetic Workspace")
    );

    // Allowing later records the permission for future sends only.
    assert_eq!(
        broker.answer(request_id, true).await,
        AnswerOutcome::Applied
    );
    assert!(broker.refresh().assistants[0].accounts[0].send);
}

#[tokio::test]
async fn removing_an_assistant_clears_grants_approvals_and_denials() {
    let dir = tempfile::tempdir().expect("tempdir");
    let broker = broker(&dir);
    broker
        .set_permission("pi", ALICE, Need::Read, true)
        .await
        .expect("saved");
    let (denied, denied_id) = ask(&broker, "pi", BOB, Need::Read, ConsentContext::default()).await;
    broker.answer(denied_id, false).await;
    let _ = denied.await;
    let (task, _) = ask(&broker, "pi", ALICE, Need::Send, ConsentContext::default()).await;

    broker.remove_client("pi").await.expect("removed");
    assert_eq!(
        task.await.expect("join").expect_err("removed").code,
        ErrorCode::NotPermitted
    );
    let state = broker.refresh();
    assert!(state.pending.is_empty());
    assert!(state.assistants.is_empty());
}

#[test]
fn previews_drop_formatting_characters_and_are_capped() {
    let text = format!("a\u{202E}b\u{2066}c\nd{}", "x".repeat(400));
    let clean = koushi_mcp_host::consent::sanitize(&text, 280);
    assert!(clean.starts_with("abc d"));
    assert_eq!(
        clean.chars().count(),
        281,
        "280 characters plus an ellipsis"
    );
}

#[tokio::test]
async fn a_retry_joins_with_its_own_message_and_is_counted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let broker = broker(&dir);
    let first = ConsentContext {
        room_label: Some("Synthetic Room".to_owned()),
        message_preview: Some("first draft".to_owned()),
    };
    let (task, request_id) = ask(&broker, "pi", ALICE, Need::Send, first).await;
    // The first caller gives up (a timeout or a closed connection).
    task.abort();
    let _ = task.await;
    assert_eq!(broker.subscribe().borrow().pending[0].message_preview, None);

    let retry = ConsentContext {
        room_label: Some("Synthetic Room".to_owned()),
        message_preview: Some("retried text".to_owned()),
    };
    let (retried, joined) = ask(&broker, "pi", ALICE, Need::Send, retry.clone()).await;
    let (second, joined_again) = ask(&broker, "pi", ALICE, Need::Send, retry).await;
    assert_eq!((joined, joined_again), (request_id, request_id));
    let mut state = broker.subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        while state.borrow_and_update().pending[0].waiting_calls != 2 {
            state.changed().await.expect("broker alive");
        }
    })
    .await
    .expect("both calls wait");
    let pending = broker.subscribe().borrow().pending[0].clone();
    assert_eq!(pending.message_preview.as_deref(), Some("retried text"));
    assert!(pending.waiting);

    assert_eq!(
        broker.answer(request_id, true).await,
        AnswerOutcome::Applied
    );
    retried.await.expect("join").expect("allowed");
    second.await.expect("join").expect("allowed");
}

#[tokio::test(start_paused = true)]
async fn an_expired_approval_is_not_joined_and_allow_settles_the_whole_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    let broker = broker(&dir);
    let (task, expired_id) = ask(&broker, "pi", ALICE, Need::Read, ConsentContext::default()).await;
    let _ = task.await; // times out after APPROVAL_WAIT
    tokio::time::advance(APPROVAL_LIFETIME).await;

    // Not swept yet, but expired: a new call opens a new approval.
    let mut state = broker.subscribe();
    let fresh_task = {
        let broker = Arc::clone(&broker);
        tokio::spawn(async move {
            broker
                .authorize(&client("pi"), ALICE, Need::Read, ConsentContext::default())
                .await
        })
    };
    let fresh_id = loop {
        if let Some(fresh) = state
            .borrow_and_update()
            .pending
            .iter()
            .find(|pending| pending.request_id != expired_id)
        {
            break fresh.request_id;
        }
        state.changed().await.expect("broker alive");
    };
    assert_eq!(broker.answer(expired_id, true).await, AnswerOutcome::Stale);
    assert_eq!(broker.answer(fresh_id, true).await, AnswerOutcome::Applied);
    fresh_task.await.expect("join").expect("allowed");
    assert!(
        broker.subscribe().borrow().pending.is_empty(),
        "every approval for the key is settled"
    );
}

#[test]
fn previews_drop_invisible_and_tag_characters_but_keep_joiners() {
    let text = "a\u{2028}b\u{FEFF}c\u{2060}d\u{E0041}e\u{200D}f";
    assert_eq!(
        koushi_mcp_host::consent::sanitize(text, 280),
        "abcde\u{200D}f"
    );
}

#[tokio::test]
async fn an_outdated_launcher_is_listed_until_it_connects_on_the_current_protocol() {
    let dir = tempfile::tempdir().expect("tempdir");
    let broker = broker(&dir);
    broker.note_outdated_launcher("claude-desktop");
    assert_eq!(
        broker.subscribe().borrow().outdated_launchers,
        vec!["claude-desktop".to_owned()]
    );
    // A tool call does not clear it; a current-protocol handshake does.
    broker
        .set_permission("claude-desktop", ALICE, Need::Read, true)
        .await
        .expect("saved");
    broker
        .authorize(
            &client("claude-desktop"),
            ALICE,
            Need::Read,
            ConsentContext::default(),
        )
        .await
        .expect("granted");
    assert_eq!(broker.subscribe().borrow().outdated_launchers.len(), 1);
    broker.note_current_launcher("claude-desktop");
    assert!(broker.subscribe().borrow().outdated_launchers.is_empty());
}
