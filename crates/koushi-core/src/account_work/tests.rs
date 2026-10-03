use std::time::Duration;

use tokio::sync::mpsc;

use super::{AccountWorkClass, AccountWorkKind, AccountWorkScheduler};

const TEST_TIMEOUT: Duration = Duration::from_secs(1);

#[test]
fn policy_bands_are_ordered_from_interactive_to_maintenance() {
    let ordered = [
        AccountWorkKind::MessageSend,
        AccountWorkKind::UserRoomOperation,
        AccountWorkKind::VisibleGapRepair,
        AccountWorkKind::ExplicitPagination,
        AccountWorkKind::OffscreenGapRepair,
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
