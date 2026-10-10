use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

use super::{AccountWorkClass, AccountWorkKind, AccountWorkScheduler};
use koushi_state::SearchCrawlerSpeed;

const TEST_TIMEOUT: Duration = Duration::from_secs(1);

/// The published QA in-flight bound must stay tied to the admission policy it
/// models (#1171): media prefetch shares the cross-account background budget,
/// so its effective network concurrency is the shared slot, not the avatar
/// downloader's own per-account ceiling. An in-flight QA expectation sized from
/// this hook must never exceed it.
// Compile-time half of the guard: the published bound must never be zero, or a
// lane deriving an in-flight expectation from it would wait for nothing.
const _: () = assert!(super::MEDIA_PREFETCH_INFLIGHT_LIMIT >= 1);

#[test]
fn media_prefetch_inflight_limit_matches_shared_admission_policy() {
    assert!(
        AccountWorkKind::MediaPrefetch.uses_shared_background_budget(),
        "media prefetch must keep sharing the cross-account budget the QA bound models"
    );
    assert_eq!(
        super::MEDIA_PREFETCH_INFLIGHT_LIMIT,
        super::SHARED_WORK_CONCURRENCY
            .min(AccountWorkKind::MediaPrefetch.policy().max_concurrency as usize)
    );
}

#[test]
fn policy_bands_are_ordered_from_interactive_to_maintenance() {
    let ordered = [
        AccountWorkKind::MessageSend,
        AccountWorkKind::UserRoomOperation,
        AccountWorkKind::VisibleGapRepair,
        AccountWorkKind::ExplicitPagination,
        AccountWorkKind::OffscreenGapRepair,
        AccountWorkKind::MediaPrefetch,
        AccountWorkKind::SearchCrawl,
        AccountWorkKind::Maintenance,
    ];
    for pair in ordered.windows(2) {
        assert!(
            pair[0].policy().priority < pair[1].policy().priority,
            "{} must outrank {}",
            pair[0].token(),
            pair[1].token()
        );
    }
    // Interactive work never queues, so it is never preempted.
    assert!(!AccountWorkKind::MessageSend.policy().preemptible);
    assert!(!AccountWorkKind::UserRoomOperation.policy().preemptible);
    assert!(AccountWorkKind::MessageSend.is_interactive());
    assert!(AccountWorkKind::UserRoomOperation.is_interactive());
    // Foreground work is user-visible; background work waits for an
    // interactive enqueue instead of re-contending.
    for kind in [
        AccountWorkKind::VisibleGapRepair,
        AccountWorkKind::ExplicitPagination,
    ] {
        assert_eq!(kind.policy().class, AccountWorkClass::Foreground);
    }
    for kind in [
        AccountWorkKind::OffscreenGapRepair,
        AccountWorkKind::MediaPrefetch,
        AccountWorkKind::SearchCrawl,
        AccountWorkKind::Maintenance,
    ] {
        assert_eq!(kind.policy().class, AccountWorkClass::Background);
    }
    // Every scheduled kind yields and reports a bounded batch.
    for kind in [
        AccountWorkKind::VisibleGapRepair,
        AccountWorkKind::ExplicitPagination,
        AccountWorkKind::OffscreenGapRepair,
        AccountWorkKind::SearchCrawl,
        AccountWorkKind::Maintenance,
    ] {
        assert!(kind.policy().preemptible, "{} must yield", kind.token());
        assert!(
            kind.policy().batch_limit > 0,
            "{} needs a batch bound",
            kind.token()
        );
        assert_eq!(kind.policy().max_concurrency, 1);
    }
    assert!(!AccountWorkKind::MediaPrefetch.policy().preemptible);
    assert_eq!(AccountWorkKind::MediaPrefetch.policy().batch_limit, 1);
}

#[tokio::test]
async fn better_priority_waiter_is_admitted_before_a_queued_background_waiter() {
    let scheduler = AccountWorkScheduler::default();
    let initial = scheduler.acquire(AccountWorkKind::SearchCrawl).await;
    let (tx, mut rx) = mpsc::unbounded_channel();

    let crawl = {
        let scheduler = scheduler.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let _permit = scheduler.acquire(AccountWorkKind::SearchCrawl).await;
            tx.send("crawl").expect("receiver alive");
        })
    };
    tokio::task::yield_now().await;

    let pagination = {
        let scheduler = scheduler.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let _permit = scheduler.acquire(AccountWorkKind::ExplicitPagination).await;
            tx.send("pagination").expect("receiver alive");
        })
    };
    tokio::task::yield_now().await;

    drop(initial);

    let first = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .expect("a waiter must be admitted")
        .expect("sender alive");
    assert_eq!(first, "pagination");
    pagination.await.expect("pagination task finished");

    let second = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .expect("the background waiter must follow")
        .expect("sender alive");
    assert_eq!(second, "crawl");
    crawl.await.expect("crawl task finished");
}

#[tokio::test]
async fn equal_priority_waiters_are_admitted_first_in_first_out() {
    let scheduler = AccountWorkScheduler::default();
    let initial = scheduler.acquire(AccountWorkKind::SearchCrawl).await;
    let (tx, mut rx) = mpsc::unbounded_channel();

    for label in ["first", "second"] {
        let scheduler = scheduler.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let _permit = scheduler.acquire(AccountWorkKind::SearchCrawl).await;
            tx.send(label).expect("receiver alive");
        });
        tokio::task::yield_now().await;
    }

    drop(initial);
    for expected in ["first", "second"] {
        let observed = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
            .await
            .expect("waiter must be admitted")
            .expect("sender alive");
        assert_eq!(observed, expected);
    }
}

#[tokio::test]
async fn better_priority_waiter_asks_active_background_work_to_yield() {
    let scheduler = AccountWorkScheduler::default();
    let crawl = scheduler.acquire(AccountWorkKind::SearchCrawl).await;

    let pagination = {
        let scheduler = scheduler.clone();
        tokio::spawn(async move {
            let _permit = scheduler.acquire(AccountWorkKind::ExplicitPagination).await;
        })
    };
    tokio::task::yield_now().await;

    tokio::time::timeout(TEST_TIMEOUT, crawl.cancelled())
        .await
        .expect("active crawl must be asked to yield");

    drop(crawl);
    tokio::time::timeout(TEST_TIMEOUT, pagination)
        .await
        .expect("pagination must run once the crawl yields")
        .expect("pagination task finished");
}

#[tokio::test]
async fn interactive_work_never_queues_and_preempts_active_background_work() {
    let scheduler = AccountWorkScheduler::default();
    let crawl = scheduler.acquire(AccountWorkKind::SearchCrawl).await;

    // The guard is taken without waiting even though the slot is busy.
    let send = tokio::time::timeout(
        TEST_TIMEOUT,
        std::future::ready(scheduler.begin_interactive(AccountWorkKind::MessageSend)),
    )
    .await
    .expect("interactive work must not queue");

    tokio::time::timeout(TEST_TIMEOUT, crawl.cancelled())
        .await
        .expect("interactive work must preempt active background work");
    assert_eq!(
        scheduler.active_kinds(),
        vec![AccountWorkKind::SearchCrawl],
        "the interactive guard must not consume the history slot"
    );

    // While the send is enqueuing, a yielding crawl must not re-enter.
    drop(crawl);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let requeued = {
        let scheduler = scheduler.clone();
        tokio::spawn(async move {
            let _permit = scheduler.acquire(AccountWorkKind::SearchCrawl).await;
            tx.send(()).expect("receiver alive");
        })
    };
    tokio::task::yield_now().await;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), rx.recv())
            .await
            .is_err(),
        "background work must wait for the interactive guard"
    );

    drop(send);
    tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .expect("background work must resume after the interactive guard")
        .expect("sender alive");
    requeued.await.expect("requeued crawl finished");
}

#[tokio::test]
async fn interactive_work_does_not_block_foreground_pagination() {
    let scheduler = AccountWorkScheduler::default();
    // A send outranks pagination numerically, but pagination is foreground:
    // it must still be admitted while an interactive enqueue is in flight.
    let _send = scheduler.begin_interactive(AccountWorkKind::MessageSend);
    let permit = tokio::time::timeout(
        TEST_TIMEOUT,
        scheduler.acquire(AccountWorkKind::ExplicitPagination),
    )
    .await;
    assert!(
        permit.is_ok(),
        "pagination must not be deferred behind an interactive enqueue"
    );
}

#[tokio::test]
async fn a_dropped_waiter_does_not_starve_background_work() {
    let scheduler = AccountWorkScheduler::default();
    let initial = scheduler.acquire(AccountWorkKind::ExplicitPagination).await;

    let abandoned = {
        let scheduler = scheduler.clone();
        tokio::spawn(async move {
            let _permit = scheduler.acquire(AccountWorkKind::ExplicitPagination).await;
        })
    };
    tokio::task::yield_now().await;
    abandoned.abort();
    let _ = abandoned.await;

    let (tx, mut rx) = mpsc::unbounded_channel();
    let crawl = {
        let scheduler = scheduler.clone();
        tokio::spawn(async move {
            let _permit = scheduler.acquire(AccountWorkKind::SearchCrawl).await;
            tx.send(()).expect("receiver alive");
        })
    };
    tokio::task::yield_now().await;
    drop(initial);

    tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .expect("crawl must be admitted after the abandoned waiter left");
    crawl.await.expect("crawl task finished");
}

#[tokio::test]
async fn a_panicking_holder_releases_its_slot() {
    let scheduler = AccountWorkScheduler::default();
    let panicked = {
        let scheduler = scheduler.clone();
        tokio::spawn(async move {
            let _permit = scheduler.acquire(AccountWorkKind::SearchCrawl).await;
            panic!("synthetic holder panic");
        })
    };
    assert!(panicked.await.is_err(), "the holder must have panicked");

    let permit = tokio::time::timeout(
        TEST_TIMEOUT,
        scheduler.acquire(AccountWorkKind::SearchCrawl),
    )
    .await
    .expect("a panicking holder must not leak its slot");
    assert_eq!(scheduler.active_kinds().len(), 1);
    drop(permit);
    assert!(scheduler.active_kinds().is_empty());
}

#[tokio::test]
async fn background_work_progresses_on_an_idle_account() {
    let scheduler = AccountWorkScheduler::default();
    for _ in 0..3 {
        let permit = tokio::time::timeout(
            TEST_TIMEOUT,
            scheduler.acquire(AccountWorkKind::Maintenance),
        )
        .await
        .expect("idle accounts must admit maintenance work");
        assert_eq!(
            AccountWorkKind::Maintenance.policy().batch_limit,
            32,
            "maintenance batches stay bounded"
        );
        drop(permit);
    }
    assert!(scheduler.active_kinds().is_empty());
}

#[tokio::test(start_paused = true)]
async fn search_crawler_speed_sets_one_rate_limit_for_search_and_media_work() {
    let scheduler = AccountWorkScheduler::default();
    scheduler.set_search_crawler_speed(SearchCrawlerSpeed::Slow);
    drop(scheduler.acquire(AccountWorkKind::SearchCrawl).await);

    let media = scheduler.for_account("second");
    let (entered_tx, mut entered_rx) = mpsc::unbounded_channel();
    let media_task = tokio::spawn(async move {
        let _permit = media.acquire(AccountWorkKind::MediaPrefetch).await;
        entered_tx.send(()).expect("receiver alive");
    });
    tokio::task::yield_now().await;
    assert!(entered_rx.try_recv().is_err());

    tokio::time::advance(Duration::from_millis(499)).await;
    tokio::task::yield_now().await;
    assert!(entered_rx.try_recv().is_err());

    tokio::time::advance(Duration::from_millis(1)).await;
    tokio::time::timeout(TEST_TIMEOUT, entered_rx.recv())
        .await
        .expect("slow budget admits the next account after its interval")
        .expect("sender alive");
    media_task.await.expect("media task finished");
}

#[tokio::test(start_paused = true)]
async fn search_crawler_speed_does_not_gate_other_background_work() {
    let scheduler = AccountWorkScheduler::default();
    scheduler.set_search_crawler_speed(SearchCrawlerSpeed::Slow);
    drop(scheduler.acquire(AccountWorkKind::SearchCrawl).await);

    for kind in [
        AccountWorkKind::OffscreenGapRepair,
        AccountWorkKind::Maintenance,
    ] {
        let work = tokio::spawn({
            let scheduler = scheduler.clone();
            async move { scheduler.acquire(kind).await }
        });
        tokio::task::yield_now().await;
        assert!(
            work.is_finished(),
            "{kind:?} is not part of the crawler budget"
        );
        drop(work.await.expect("unbudgeted work must be admitted"));
    }

    scheduler.set_search_crawler_speed(SearchCrawlerSpeed::Paused);
    for kind in [
        AccountWorkKind::OffscreenGapRepair,
        AccountWorkKind::Maintenance,
    ] {
        let permit = scheduler.acquire(kind).await;
        drop(permit);
    }
}

#[tokio::test]
async fn paused_speed_blocks_search_and_media_until_resumed() {
    for kind in [AccountWorkKind::SearchCrawl, AccountWorkKind::MediaPrefetch] {
        let scheduler = AccountWorkScheduler::default();
        scheduler.set_search_crawler_speed(SearchCrawlerSpeed::Paused);
        let task = tokio::spawn({
            let scheduler = scheduler.clone();
            async move { scheduler.acquire(kind).await }
        });
        tokio::task::yield_now().await;
        assert!(!task.is_finished(), "{kind:?} must wait while paused");

        scheduler.set_search_crawler_speed(SearchCrawlerSpeed::Fast);
        let permit = tokio::time::timeout(TEST_TIMEOUT, task)
            .await
            .expect("resuming the budget wakes its waiters")
            .expect("work task finished");
        drop(permit);
    }
}

#[tokio::test]
async fn paused_speed_does_not_let_queued_budget_waiters_block_maintenance() {
    let scheduler = AccountWorkScheduler::default();
    scheduler.set_search_crawler_speed(SearchCrawlerSpeed::Paused);

    // A budget waiter that queued while paused stays in the queue without
    // being admissible. It must not outrank work the budget does not gate.
    let paused_crawl = tokio::spawn({
        let scheduler = scheduler.clone();
        async move { scheduler.acquire(AccountWorkKind::SearchCrawl).await }
    });
    tokio::time::timeout(TEST_TIMEOUT, async {
        while scheduler.waiting_count() != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the paused crawl must be queued");
    assert!(!paused_crawl.is_finished());

    let maintenance = tokio::time::timeout(
        TEST_TIMEOUT,
        scheduler.acquire(AccountWorkKind::Maintenance),
    )
    .await
    .expect("a waiting paused budget request must not block unrelated maintenance");
    drop(maintenance);

    paused_crawl.abort();
    let _ = paused_crawl.await;
}

#[tokio::test]
async fn shared_account_work_limits_background_load_and_prioritizes_selected_tab() {
    let scheduler = AccountWorkScheduler::default();
    scheduler.set_selected_account(Some("bob"));
    let alice = scheduler.for_account("alice");
    let bob = scheduler.for_account("bob");
    let active = bob.acquire(AccountWorkKind::OffscreenGapRepair).await;
    let (entered_tx, mut entered_rx) = mpsc::unbounded_channel();

    let (alice_release_tx, alice_release_rx) = oneshot::channel();
    let alice_task = tokio::spawn({
        let entered_tx = entered_tx.clone();
        async move {
            let _permit = alice.acquire(AccountWorkKind::SearchCrawl).await;
            entered_tx.send("alice").expect("receiver alive");
            let _ = alice_release_rx.await;
        }
    });
    tokio::time::timeout(TEST_TIMEOUT, async {
        while scheduler.waiting_count() != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("Alice must queue behind Bob's active work");

    scheduler.set_selected_account(Some("alice"));
    let (bob_release_tx, bob_release_rx) = oneshot::channel();
    let bob_task = tokio::spawn({
        let entered_tx = entered_tx.clone();
        async move {
            let _permit = bob.acquire(AccountWorkKind::SearchCrawl).await;
            entered_tx.send("bob").expect("receiver alive");
            let _ = bob_release_rx.await;
        }
    });
    tokio::time::timeout(TEST_TIMEOUT, async {
        while scheduler.waiting_count() != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("both account requests must share the queue");

    drop(active);
    assert_eq!(
        tokio::time::timeout(TEST_TIMEOUT, entered_rx.recv())
            .await
            .expect("selected account must be admitted")
            .expect("sender alive"),
        "alice"
    );
    tokio::task::yield_now().await;
    assert!(
        entered_rx.try_recv().is_err(),
        "only one account may run at once"
    );

    alice_release_tx.send(()).expect("Alice task alive");
    assert_eq!(
        tokio::time::timeout(TEST_TIMEOUT, entered_rx.recv())
            .await
            .expect("the other account must make progress")
            .expect("sender alive"),
        "bob"
    );
    bob_release_tx.send(()).expect("Bob task alive");
    alice_task.await.expect("Alice task finished");
    bob_task.await.expect("Bob task finished");
}
