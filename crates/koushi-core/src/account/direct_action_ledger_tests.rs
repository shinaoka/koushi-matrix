//! Retry ledger rules and the actor admission around them (#1660, #1681).

use super::*;
use crate::direct_operations::{DirectEditOutcome, DirectRetryKey};

const ACTION_DEADLINE: Duration = Duration::from_secs(45);

fn edit_key(client: &str, client_request_id: &str) -> DirectActionKey {
    DirectActionKey::for_request(&DirectMessageRequest::Edit {
        room_id: "!room:example.invalid".to_owned(),
        event_id: "$target".to_owned(),
        body: "edited".to_owned(),
        formatting: koushi_state::ComposerFormattingOptions::default(),
        retry: Some(DirectRetryKey {
            client: client.to_owned(),
            client_request_id: client_request_id.to_owned(),
        }),
    })
    .expect("an edit with a retry key has a ledger key")
}

fn edited() -> Result<DirectMessageResponse, DirectMessageFailure> {
    Ok(DirectMessageResponse::Edit(DirectEditOutcome {
        edit_event_id: Some("$edit".to_owned()),
    }))
}

fn claim(ledger: &mut DirectActionLedger, key: &DirectActionKey, now: Instant) -> ClaimToken {
    match ledger.begin(key.clone(), now) {
        Begin::Fresh(token) => token,
        other => panic!("expected a fresh claim, got {other:?}"),
    }
}

#[tokio::test(start_paused = true)]
async fn a_claim_nobody_settles_ages_out_after_its_deadline_and_the_ttl() {
    let mut ledger = DirectActionLedger::new(ACTION_DEADLINE);
    let key = edit_key("pi", "uuid-1");
    let start = Instant::now();
    claim(&mut ledger, &key, start);

    assert!(matches!(ledger.begin(key.clone(), start), Begin::Pending));
    tokio::time::advance(ACTION_DEADLINE + LEDGER_TTL - Duration::from_secs(1)).await;
    assert!(
        matches!(ledger.begin(key.clone(), Instant::now()), Begin::Pending),
        "an unsettled claim is ambiguous until the TTL after its deadline"
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(
        matches!(ledger.begin(key, Instant::now()), Begin::Fresh(_)),
        "a claim nobody settled must not block its retry key forever"
    );
}

#[tokio::test(start_paused = true)]
async fn a_full_ledger_answers_at_once_and_admits_again_once_entries_age_out() {
    let mut ledger = DirectActionLedger::new(ACTION_DEADLINE);
    for index in 0..LEDGER_LIMIT {
        claim(
            &mut ledger,
            &edit_key("pi", &index.to_string()),
            Instant::now(),
        );
    }
    // No entry may be forgotten early, so the insert is refused, not
    // waited on.
    assert!(matches!(
        ledger.begin(edit_key("pi", "one-more"), Instant::now()),
        Begin::Full
    ));
    tokio::time::advance(ACTION_DEADLINE + LEDGER_TTL).await;
    claim(&mut ledger, &edit_key("pi", "one-more"), Instant::now());
    assert_eq!(ledger.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_full_ledger_forgets_the_oldest_settled_entry_but_no_ambiguous_one() {
    let mut ledger = DirectActionLedger::new(ACTION_DEADLINE);
    let first_ambiguous = edit_key("pi", "ambiguous");
    let token = claim(&mut ledger, &first_ambiguous, Instant::now());
    ledger.finish(
        &first_ambiguous,
        token,
        &Err(DirectMessageFailure::Unavailable),
        Instant::now(),
    );
    for index in 1..LEDGER_LIMIT {
        let key = edit_key("pi", &index.to_string());
        let token = claim(&mut ledger, &key, Instant::now());
        ledger.finish(&key, token, &edited(), Instant::now());
    }
    claim(&mut ledger, &edit_key("pi", "one-more"), Instant::now());
    assert_eq!(ledger.len(), LEDGER_LIMIT);
    assert!(matches!(
        ledger.begin(first_ambiguous, Instant::now()),
        Begin::Pending
    ));
    assert!(
        matches!(
            ledger.begin(edit_key("pi", "1"), Instant::now()),
            Begin::Fresh(_)
        ),
        "the oldest settled entry made room"
    );
}

#[tokio::test(start_paused = true)]
async fn a_settled_request_replays_until_the_ttl() {
    let mut ledger = DirectActionLedger::new(ACTION_DEADLINE);
    let key = edit_key("pi", "uuid-1");
    let token = claim(&mut ledger, &key, Instant::now());
    ledger.finish(&key, token, &edited(), Instant::now());

    match ledger.begin(key.clone(), Instant::now()) {
        Begin::Replay(response) => {
            let DirectMessageResponse::Edit(outcome) = *response else {
                panic!("an edit replays an edit");
            };
            assert_eq!(outcome.edit_event_id.as_deref(), Some("$edit"));
        }
        other => panic!("a settled request replays its result, got {other:?}"),
    }
    tokio::time::advance(LEDGER_TTL).await;
    assert!(matches!(ledger.begin(key, Instant::now()), Begin::Fresh(_)));
}

#[tokio::test(start_paused = true)]
async fn an_ambiguous_failure_is_pending_until_the_ttl_and_a_definite_one_frees_the_key() {
    let mut ledger = DirectActionLedger::new(ACTION_DEADLINE);
    let ambiguous = edit_key("pi", "uuid-1");
    let token = claim(&mut ledger, &ambiguous, Instant::now());
    ledger.finish(
        &ambiguous,
        token,
        &Err(DirectMessageFailure::Unavailable),
        Instant::now(),
    );
    tokio::time::advance(LEDGER_TTL - Duration::from_secs(1)).await;
    assert!(matches!(
        ledger.begin(ambiguous.clone(), Instant::now()),
        Begin::Pending
    ));
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(matches!(
        ledger.begin(ambiguous, Instant::now()),
        Begin::Fresh(_)
    ));

    let definite = edit_key("pi", "uuid-2");
    let token = claim(&mut ledger, &definite, Instant::now());
    ledger.finish(
        &definite,
        token,
        &Err(DirectMessageFailure::NotAuthor),
        Instant::now(),
    );
    assert!(matches!(
        ledger.begin(definite, Instant::now()),
        Begin::Fresh(_)
    ));
}

#[tokio::test(start_paused = true)]
async fn a_late_completion_of_an_aged_out_claim_does_not_touch_a_newer_claim() {
    let mut ledger = DirectActionLedger::new(ACTION_DEADLINE);
    let key = edit_key("pi", "uuid-1");
    let stale = claim(&mut ledger, &key, Instant::now());
    tokio::time::advance(ACTION_DEADLINE + LEDGER_TTL).await;
    let current = claim(&mut ledger, &key, Instant::now());

    ledger.finish(&key, stale, &edited(), Instant::now());
    assert!(
        matches!(ledger.begin(key.clone(), Instant::now()), Begin::Pending),
        "the stale completion must not settle the newer claim"
    );
    ledger.finish(&key, current, &edited(), Instant::now());
    assert!(matches!(
        ledger.begin(key, Instant::now()),
        Begin::Replay(_)
    ));
}

#[test]
fn keys_are_scoped_to_the_client_and_requests_without_a_retry_key_are_not_kept() {
    assert_ne!(edit_key("pi", "uuid-9"), edit_key("codex", "uuid-9"));
    let reply = DirectMessageRequest::ThreadReply {
        room_id: "!room:example.invalid".to_owned(),
        root_event_id: "$target".to_owned(),
        body: "reply".to_owned(),
        formatting: koushi_state::ComposerFormattingOptions::default(),
        retry: Some(DirectRetryKey {
            client: "pi".to_owned(),
            client_request_id: "uuid-9".to_owned(),
        }),
    };
    assert_ne!(
        DirectActionKey::for_request(&reply),
        Some(edit_key("pi", "uuid-9")),
        "an edit and a thread reply on the same target do not share a key"
    );
    let delete = DirectMessageRequest::Delete {
        room_id: "!room:example.invalid".to_owned(),
        event_id: "$target".to_owned(),
    };
    assert_eq!(DirectActionKey::for_request(&delete), None);
}

fn slots(count: usize) -> Arc<tokio::sync::Semaphore> {
    Arc::new(tokio::sync::Semaphore::new(count))
}

fn edit_request(client_request_id: &str) -> DirectMessageRequest {
    DirectMessageRequest::Edit {
        room_id: "!room:example.invalid".to_owned(),
        event_id: "$target".to_owned(),
        body: "edited".to_owned(),
        formatting: koushi_state::ComposerFormattingOptions::default(),
        retry: Some(DirectRetryKey {
            client: "pi".to_owned(),
            client_request_id: client_request_id.to_owned(),
        }),
    }
}

#[tokio::test(start_paused = true)]
async fn an_action_that_hits_its_deadline_leaves_the_key_ambiguous_until_the_ttl() {
    let ledger = Mutex::new(DirectActionLedger::new(ACTION_DEADLINE));
    let slots = slots(1);
    let request = edit_request("uuid-1");
    let admitted = admit(&ledger, &slots, &request, Instant::now()).expect("admitted");
    let outcome = run_claimed(
        &ledger,
        admitted.claim,
        ACTION_DEADLINE,
        std::future::pending(),
    )
    .await;
    assert_eq!(outcome.err(), Some(DirectMessageFailure::Unavailable));
    drop(admitted.permit);

    assert!(matches!(
        admit(&ledger, &slots, &request, Instant::now()).map_err(|answer| *answer),
        Err(Err(DirectMessageFailure::DuplicatePending))
    ));
    tokio::time::advance(LEDGER_TTL).await;
    assert!(
        admit(&ledger, &slots, &request, Instant::now()).is_ok(),
        "the retry key is free once the ambiguous window ends"
    );
}

#[tokio::test(start_paused = true)]
async fn an_aborted_action_releases_its_key_after_its_deadline_and_the_ttl() {
    let ledger = Arc::new(Mutex::new(DirectActionLedger::new(ACTION_DEADLINE)));
    let slots = slots(1);
    let request = edit_request("uuid-1");
    let admitted = admit(&ledger, &slots, &request, Instant::now()).expect("admitted");
    let task = tokio::spawn({
        let ledger = Arc::clone(&ledger);
        async move {
            let _permit = admitted.permit;
            run_claimed(
                &ledger,
                admitted.claim,
                ACTION_DEADLINE,
                std::future::pending(),
            )
            .await
        }
    });
    tokio::task::yield_now().await;
    task.abort();
    assert!(task.await.is_err());

    assert!(matches!(
        admit(&ledger, &slots, &request, Instant::now()).map_err(|answer| *answer),
        Err(Err(DirectMessageFailure::DuplicatePending))
    ));
    tokio::time::advance(ACTION_DEADLINE + LEDGER_TTL).await;
    assert!(admit(&ledger, &slots, &request, Instant::now()).is_ok());
}

#[tokio::test(start_paused = true)]
async fn a_settled_retry_replays_while_the_slots_are_busy_and_a_slotless_claim_is_released() {
    let ledger = Mutex::new(DirectActionLedger::new(ACTION_DEADLINE));
    let slots = slots(1);
    let request = edit_request("uuid-1");
    let admitted = admit(&ledger, &slots, &request, Instant::now()).expect("admitted");
    run_claimed(&ledger, admitted.claim, ACTION_DEADLINE, async { edited() })
        .await
        .expect("edited");
    drop(admitted.permit);

    let busy =
        admit(&ledger, &slots, &edit_request("other"), Instant::now()).expect("the one slot");
    assert!(
        matches!(
            admit(&ledger, &slots, &request, Instant::now()).map_err(|answer| *answer),
            Err(Ok(DirectMessageResponse::Edit(_)))
        ),
        "a replay needs no slot"
    );
    let slotless = edit_request("uuid-2");
    assert!(matches!(
        admit(&ledger, &slots, &slotless, Instant::now()).map_err(|answer| *answer),
        Err(Err(DirectMessageFailure::Capacity))
    ));
    drop(busy);
    assert!(
        admit(&ledger, &slots, &slotless, Instant::now()).is_ok(),
        "a claim refused for capacity was released"
    );
}

#[tokio::test(start_paused = true)]
async fn a_definite_failure_frees_the_key_and_requests_without_a_key_are_not_kept() {
    let ledger = Mutex::new(DirectActionLedger::new(ACTION_DEADLINE));
    let slots = slots(1);
    let request = edit_request("uuid-1");
    let admitted = admit(&ledger, &slots, &request, Instant::now()).expect("admitted");
    let outcome = run_claimed(&ledger, admitted.claim, ACTION_DEADLINE, async {
        Err(DirectMessageFailure::NotAuthor)
    })
    .await;
    assert_eq!(outcome.err(), Some(DirectMessageFailure::NotAuthor));
    drop(admitted.permit);
    assert!(admit(&ledger, &slots, &request, Instant::now()).is_ok());

    let unkeyed = DirectMessageRequest::Delete {
        room_id: "!room:example.invalid".to_owned(),
        event_id: "$target".to_owned(),
    };
    let before = ledger.lock().expect("ledger").len();
    let admitted = admit(&ledger, &slots, &unkeyed, Instant::now()).expect("admitted");
    assert!(admitted.claim.is_none());
    assert_eq!(ledger.lock().expect("ledger").len(), before);
}
