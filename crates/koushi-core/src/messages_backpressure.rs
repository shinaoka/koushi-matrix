//! Account-wide backpressure for SDK calls that page room history through
//! `/rooms/{roomId}/messages`.
//!
//! Search history crawl and user-visible timeline pagination both hit the same
//! homeserver endpoint. The crawler is background work, so it must yield to
//! timeline pagination while still keeping global concurrency at one page.

use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::Notify;

#[derive(Clone, Debug, Default)]
pub(crate) struct MessagesBackpressure {
    inner: Arc<MessagesBackpressureInner>,
}

#[derive(Debug, Default)]
struct MessagesBackpressureInner {
    state: Mutex<MessagesBackpressureState>,
    notify: Notify,
}

#[derive(Debug, Default)]
struct MessagesBackpressureState {
    active: bool,
    /// True while the active permit is held by a crawler request.
    active_is_crawler: bool,
    /// Cancellation signal for the currently-active crawler permit, taken by a
    /// waiting timeline acquirer to make the crawler yield mid-page.
    active_crawler_cancel: Option<Arc<Notify>>,
    waiting_timeline: u64,
}

#[must_use]
pub(crate) struct MessagesRequestPermit {
    inner: Arc<MessagesBackpressureInner>,
    /// `Some` for crawler permits — resolves when a timeline acquirer requests
    /// preemption. `None` for timeline permits (never cancelled).
    cancel: Option<Arc<Notify>>,
}

impl MessagesRequestPermit {
    /// Resolves when a timeline acquirer has asked this crawler permit to yield.
    /// For a timeline permit this never resolves.
    pub(crate) async fn cancelled(&self) {
        match &self.cancel {
            Some(cancel) => cancel.notified().await,
            None => std::future::pending::<()>().await,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MessagesRequestPriority {
    Timeline,
    Crawler,
}

struct WaitingTimelineSlot {
    inner: Option<Arc<MessagesBackpressureInner>>,
}

impl MessagesBackpressure {
    pub(crate) async fn acquire_timeline(&self) -> MessagesRequestPermit {
        self.acquire(MessagesRequestPriority::Timeline).await
    }

    pub(crate) async fn acquire_crawler(&self) -> MessagesRequestPermit {
        self.acquire(MessagesRequestPriority::Crawler).await
    }

    async fn acquire(&self, priority: MessagesRequestPriority) -> MessagesRequestPermit {
        let mut timeline_slot = match priority {
            MessagesRequestPriority::Timeline => Some(WaitingTimelineSlot::new(self.inner.clone())),
            MessagesRequestPriority::Crawler => None,
        };

        loop {
            let notified = self.inner.notify.notified();
            {
                let mut state = lock_state(&self.inner);
                let can_acquire = !state.active
                    && (priority == MessagesRequestPriority::Timeline
                        || state.waiting_timeline == 0);
                if can_acquire {
                    if let Some(slot) = timeline_slot.take() {
                        slot.finish(&mut state);
                    }
                    state.active = true;
                    let cancel = match priority {
                        MessagesRequestPriority::Crawler => {
                            let cancel = Arc::new(Notify::new());
                            state.active_is_crawler = true;
                            state.active_crawler_cancel = Some(cancel.clone());
                            Some(cancel)
                        }
                        MessagesRequestPriority::Timeline => {
                            state.active_is_crawler = false;
                            None
                        }
                    };
                    return MessagesRequestPermit {
                        inner: self.inner.clone(),
                        cancel,
                    };
                }
                // Timeline is blocked behind an active crawler: ask it to yield.
                if priority == MessagesRequestPriority::Timeline
                    && state.active
                    && state.active_is_crawler
                {
                    if let Some(cancel) = &state.active_crawler_cancel {
                        cancel.notify_one();
                    }
                }
            }
            notified.await;
        }
    }
}

impl WaitingTimelineSlot {
    fn new(inner: Arc<MessagesBackpressureInner>) -> Self {
        {
            let mut state = lock_state(&inner);
            state.waiting_timeline = state.waiting_timeline.saturating_add(1);
        }
        inner.notify.notify_waiters();
        Self { inner: Some(inner) }
    }

    fn finish(mut self, state: &mut MessagesBackpressureState) {
        state.waiting_timeline = state.waiting_timeline.saturating_sub(1);
        self.inner = None;
    }
}

impl Drop for WaitingTimelineSlot {
    fn drop(&mut self) {
        let Some(inner) = self.inner.take() else {
            return;
        };
        {
            let mut state = lock_state(&inner);
            state.waiting_timeline = state.waiting_timeline.saturating_sub(1);
        }
        inner.notify.notify_waiters();
    }
}

impl Drop for MessagesRequestPermit {
    fn drop(&mut self) {
        {
            let mut state = lock_state(&self.inner);
            state.active = false;
            state.active_is_crawler = false;
            state.active_crawler_cancel = None;
        }
        self.inner.notify.notify_waiters();
    }
}

fn lock_state(inner: &MessagesBackpressureInner) -> MutexGuard<'_, MessagesBackpressureState> {
    inner
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::sync::mpsc;

    use super::MessagesBackpressure;

    #[tokio::test]
    async fn crawler_waiting_first_yields_to_timeline_waiter() {
        let gate = MessagesBackpressure::default();
        let initial = gate.acquire_crawler().await;
        let (tx, mut rx) = mpsc::unbounded_channel();

        let crawler_gate = gate.clone();
        let crawler_tx = tx.clone();
        let crawler = tokio::spawn(async move {
            let _permit = crawler_gate.acquire_crawler().await;
            crawler_tx.send("crawler").expect("test receiver alive");
        });
        tokio::task::yield_now().await;

        let timeline_gate = gate.clone();
        let timeline_tx = tx.clone();
        let timeline = tokio::spawn(async move {
            let _permit = timeline_gate.acquire_timeline().await;
            timeline_tx.send("timeline").expect("test receiver alive");
        });
        tokio::task::yield_now().await;

        drop(initial);

        let first = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("first waiter should acquire")
            .expect("sender alive");
        assert_eq!(first, "timeline");
        timeline.await.expect("timeline task should finish");

        let second = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("second waiter should acquire")
            .expect("sender alive");
        assert_eq!(second, "crawler");
        crawler.await.expect("crawler task should finish");
    }

    #[tokio::test]
    async fn timeline_acquire_cancels_active_crawler_permit() {
        let gate = MessagesBackpressure::default();
        let crawler = gate.acquire_crawler().await;

        // A timeline acquire while the crawler holds the permit must signal the
        // crawler to yield: its `cancelled()` future resolves.
        let timeline_gate = gate.clone();
        let timeline = tokio::spawn(async move {
            let _permit = timeline_gate.acquire_timeline().await;
        });
        tokio::task::yield_now().await;

        tokio::time::timeout(Duration::from_secs(1), crawler.cancelled())
            .await
            .expect("active crawler must be cancelled when a timeline acquire is waiting");

        // Releasing the crawler lets the timeline acquire proceed.
        drop(crawler);
        tokio::time::timeout(Duration::from_secs(1), timeline)
            .await
            .expect("timeline must acquire after the crawler yields")
            .expect("timeline task should finish");
    }

    #[tokio::test]
    async fn waiting_timeline_drop_does_not_starve_crawler() {
        let gate = MessagesBackpressure::default();
        let initial = gate.acquire_timeline().await;
        let timeline_gate = gate.clone();
        let timeline = tokio::spawn(async move {
            let _permit = timeline_gate.acquire_timeline().await;
        });
        tokio::task::yield_now().await;
        timeline.abort();
        let _ = timeline.await;

        let crawler_gate = gate.clone();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let crawler = tokio::spawn(async move {
            let _permit = crawler_gate.acquire_crawler().await;
            tx.send(()).expect("test receiver alive");
        });
        tokio::task::yield_now().await;
        drop(initial);

        tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("crawler must acquire after cancelled timeline waiter");
        crawler.await.expect("crawler task should finish");
    }
}
