//! Executor abstraction (Platform Portability rule 2).
//!
//! All task spawning and timing in core goes through this module. Today it
//! wraps tokio; a wasm backend swaps these implementations without touching
//! actor logic. Direct `tokio::spawn`/`tokio::time` calls elsewhere in this
//! crate are a portability violation.

use std::future::Future;
use std::time::Duration;

pub use tokio::task::JoinHandle;

pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::task::spawn(future)
}

pub async fn sleep(duration: Duration) {
    tokio::time::sleep(duration).await;
}

pub async fn timeout<F: Future>(
    duration: Duration,
    future: F,
) -> Result<F::Output, TimeoutElapsed> {
    tokio::time::timeout(duration, future)
        .await
        .map_err(|_| TimeoutElapsed)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimeoutElapsed;
