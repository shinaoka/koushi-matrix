//! Executor abstraction (Platform Portability rule 2).
//!
//! All task spawning and timing in core goes through this module. Today it
//! wraps tokio; a wasm backend swaps these implementations without touching
//! actor logic. Direct `tokio::spawn`/`tokio::time` calls elsewhere in this
//! crate are a portability violation.

use std::future::Future;
use std::time::Duration;

pub use tokio::task::JoinHandle;
pub use tokio::time::Instant;

/// A task handle that is aborted if its owner is dropped without an orderly
/// shutdown. Explicit shutdown takes the handle and awaits it; error paths in
/// headless QA and embedding callers therefore cannot leave detached runtime
/// tasks keeping the process alive indefinitely.
pub(crate) struct AbortOnDrop<T> {
    handle: Option<JoinHandle<T>>,
}

impl<T> AbortOnDrop<T> {
    pub(crate) fn new(handle: JoinHandle<T>) -> Self {
        Self {
            handle: Some(handle),
        }
    }

    pub(crate) fn get(&self) -> &JoinHandle<T> {
        self.handle
            .as_ref()
            .expect("abort-on-drop task handle must remain present")
    }

    pub(crate) fn take(&mut self) -> JoinHandle<T> {
        self.handle
            .take()
            .expect("abort-on-drop task handle must be taken once")
    }

    pub(crate) fn abort(&self) {
        if let Some(handle) = &self.handle {
            handle.abort();
        }
    }

    pub(crate) async fn settle(&mut self) {
        if let Some(handle) = &mut self.handle {
            let _ = handle.await;
        }
        self.handle.take();
    }
}

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.abort();
    }
}

pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::task::spawn(future)
}

pub fn spawn_blocking<F, R>(function: F) -> JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    tokio::task::spawn_blocking(function)
}

pub async fn sleep(duration: Duration) {
    tokio::time::sleep(duration).await;
}

pub async fn sleep_until(deadline: Instant) {
    tokio::time::sleep_until(deadline).await;
}

pub async fn timeout<F: Future>(
    duration: Duration,
    future: F,
) -> Result<F::Output, TimeoutElapsed> {
    tokio::time::timeout(duration, future)
        .await
        .map_err(|_| TimeoutElapsed)
}

pub async fn timeout_at<F: Future>(
    deadline: Instant,
    future: F,
) -> Result<F::Output, TimeoutElapsed> {
    tokio::time::timeout_at(deadline, future)
        .await
        .map_err(|_| TimeoutElapsed)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimeoutElapsed;
